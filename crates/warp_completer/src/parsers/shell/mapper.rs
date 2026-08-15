//! Converts Arborium/tree-sitter parse trees into the Warp-owned model in `super`. This is the
//! only place in `warp_completer` that touches `arborium`/`tree_sitter` types for the shell
//! adapter; nothing here is `pub` outside the crate.
//!
//! `tree_sitter::Node` does not carry a reference to the text it was parsed from, so every
//! function below that needs a node's text takes `source: &str` explicitly. `source` is always
//! the *original*, unpadded input: recovery only ever appends a sentinel newline and/or synthetic
//! closing delimiters after it, and every span is clipped back to `source.len()` before use, so
//! synthetic bytes never appear in an extracted string (per the spec's recovery contract).

use arborium::tree_sitter::{Node, Parser, Tree};

use super::{
    DelimiterState, NestedCommandGroup, NestedCommandKind, OpenDelimiter, ParsedCommand,
    ParsedShellInput, ParsedWord, ShellDialect, ShellParseOptions, ShellParseRejection,
    ShellParseStatus, ShellRedirection, ShellRedirectionKind,
};
use crate::meta::{Span, SpannedItem};

pub(super) fn parse(
    source: &str,
    dialect: ShellDialect,
    options: ShellParseOptions,
) -> ParsedShellInput {
    let _ = options; // Reserved for QuoteMode; both modes currently produce the same hierarchy.

    if let ShellDialect::Zsh = dialect
        && let Some(rejection) = ZshCompatibilityGuard::check(source)
    {
        return rejected(dialect, source.len(), rejection);
    }

    let needs_sentinel = matches!(dialect, ShellDialect::Fish);
    let candidate = if needs_sentinel {
        format!("{source}\n")
    } else {
        source.to_string()
    };

    let Some(tree) = parse_tree(dialect, &candidate) else {
        return rejected(
            dialect,
            source.len(),
            ShellParseRejection::GrammarUnavailable,
        );
    };

    if !tree.root_node().has_error() {
        let commands = collect_commands(tree.root_node(), source, source.len(), dialect);
        return ParsedShellInput {
            dialect,
            source_len: source.len(),
            commands,
            status: ShellParseStatus::Complete,
        };
    }

    // A generic "unrecoverable" rejection is a Zsh compatibility rejection when the dialect is
    // Zsh: for Zsh, any residual grammar error after recovery attempts means the Bash grammar
    // could not represent the input's actual (Zsh-only) syntax, not merely a legacy-parser-style
    // incompleteness.
    let unrecoverable = if let ShellDialect::Zsh = dialect {
        ShellParseRejection::UnsupportedDialectSyntax
    } else {
        ShellParseRejection::Unrecoverable
    };

    // Per the spec's Zsh compatibility contract point 1: a non-EOF `ERROR`/missing node means the
    // input contains syntax the Bash grammar cannot represent at all, not an incomplete buffer
    // that recovery could plausibly close. Reject immediately rather than attempting recovery,
    // which is only meaningful for errors that intersect EOF.
    if let ShellDialect::Zsh = dialect
        && !error_touches_eof(&tree, candidate.len())
    {
        return rejected(
            dialect,
            source.len(),
            ShellParseRejection::UnsupportedDialectSyntax,
        );
    }

    // Recovery: infer unclosed delimiters lexically, append the minimal synthetic closers, and
    // reparse. This only ever appends bytes after `source`, so it cannot change the
    // interpretation of any non-EOF text.
    let Some(recovery) = Recovery::infer(source, dialect) else {
        return rejected(dialect, source.len(), unrecoverable);
    };
    let sentinel = if needs_sentinel { "\n" } else { "" };
    let padded = format!("{source}{sentinel}{}", recovery.closers);
    match parse_tree(dialect, &padded) {
        Some(recovered_tree) if !recovered_tree.root_node().has_error() => {
            let commands =
                collect_commands(recovered_tree.root_node(), source, source.len(), dialect);
            ParsedShellInput {
                dialect,
                source_len: source.len(),
                commands,
                status: ShellParseStatus::Recovered {
                    open_delimiters: recovery.open_delimiters,
                },
            }
        }
        _ => rejected(dialect, source.len(), unrecoverable),
    }
}

/// Returns whether `tree` has an `ERROR` or missing node whose span reaches all the way to the
/// end of the parsed text, i.e. a candidate for EOF-based recovery rather than a flat mid-input
/// syntax error.
fn error_touches_eof(tree: &Tree, text_len: usize) -> bool {
    fn visit(node: Node, text_len: usize) -> bool {
        if (node.is_error() || node.is_missing()) && node.end_byte() == text_len {
            return true;
        }
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .any(|child| visit(child, text_len))
    }
    visit(tree.root_node(), text_len)
}

fn rejected(
    dialect: ShellDialect,
    source_len: usize,
    rejection: ShellParseRejection,
) -> ParsedShellInput {
    ParsedShellInput {
        dialect,
        source_len,
        commands: Vec::new(),
        status: ShellParseStatus::Rejected(rejection),
    }
}

fn parse_tree(dialect: ShellDialect, source: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&dialect.grammar()).ok()?;
    parser.parse(source, None)
}

// -------------------------------------------------------------------------------------------
// Zsh compatibility guard.
// -------------------------------------------------------------------------------------------

/// Rejects Zsh input that the Bash grammar would either error on, or silently misparse into a
/// wrong hierarchy. See `specs/APP-5430/TECH.md`'s "Zsh-on-Bash compatibility contract": for Zsh,
/// the absence of a tree-sitter error is not evidence that the hierarchy is correct.
struct ZshCompatibilityGuard;

impl ZshCompatibilityGuard {
    /// Returns `Some(rejection)` if `source` must be rejected before even attempting to parse it
    /// with the Bash grammar (the command-position keyword check). The `ERROR`/missing-node
    /// check that covers the other measured Zsh-only constructs is applied uniformly by `parse`
    /// for every dialect, since the Bash grammar already fails loudly on those.
    fn check(source: &str) -> Option<ShellParseRejection> {
        if Self::has_command_position_repeat(source) {
            return Some(ShellParseRejection::UnsupportedDialectSyntax);
        }
        None
    }

    /// Detects `repeat` used as a command-position reserved word (i.e. it starts a `repeat COUNT
    /// do ... done` loop), which the Bash grammar parses without error into three unrelated
    /// top-level commands rather than rejecting or preserving loop semantics. Must not reject
    /// `repeat` used as a plain argument, e.g. `echo repeat`.
    fn has_command_position_repeat(source: &str) -> bool {
        // A word is in "command position" if it is the first non-whitespace word of the input,
        // or immediately follows a command separator (`;`, `\n`, `|`, `&`) or an opening group
        // (`(`, `{`). This mirrors how the Bash grammar itself recognizes the start of a new
        // `command` node, without needing a full parse. `word_is_command_position` captures
        // `at_command_start` exactly once, at the word's first character, and is not re-derived
        // from `at_command_start` while scanning the rest of that same word.
        let mut at_command_start = true;
        let mut word_start: Option<usize> = None;
        let mut word_is_command_position = false;
        for (idx, ch) in source.char_indices() {
            let is_separator = ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '(' | '{');
            if is_separator {
                if let Some(start) = word_start.take()
                    && word_is_command_position
                    && &source[start..idx] == "repeat"
                {
                    return true;
                }
                at_command_start = matches!(ch, ';' | '\n' | '|' | '&' | '(' | '{');
            } else {
                if word_start.is_none() {
                    word_start = Some(idx);
                    word_is_command_position = at_command_start;
                }
                at_command_start = false;
            }
        }
        if let Some(start) = word_start
            && word_is_command_position
            && &source[start..] == "repeat"
        {
            return true;
        }
        false
    }
}

// -------------------------------------------------------------------------------------------
// EOF recovery: a lexical bracket scanner independent of the grammar's own error productions.
// -------------------------------------------------------------------------------------------

struct Recovery {
    closers: String,
    open_delimiters: Vec<OpenDelimiter>,
}

impl Recovery {
    /// Scans `source` for delimiters left open at EOF and returns the minimal dialect-correct
    /// closing suffix, or `None` if the source contains no recognizable open delimiter (in which
    /// case the caller should reject rather than guess, per the spec's recovery algorithm).
    fn infer(source: &str, dialect: ShellDialect) -> Option<Recovery> {
        match dialect {
            ShellDialect::Bash | ShellDialect::Zsh => Self::infer_dollar_paren_family(source, '\\'),
            ShellDialect::PowerShell => Self::infer_dollar_paren_family(source, '`'),
            // Fish's only common incompleteness (a missing statement terminator) is already
            // handled by the unconditional sentinel newline; an `ERROR` surviving that requires
            // more than a simple append and is rejected as Unrecoverable.
            ShellDialect::Fish => None,
        }
    }

    /// Shared scanner for the POSIX and PowerShell families, which both use `$(...)` (and POSIX
    /// additionally backticks) for command substitution, and differ only in their escape
    /// character (`\` for POSIX, `` ` `` for PowerShell).
    fn infer_dollar_paren_family(source: &str, escape_char: char) -> Option<Recovery> {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Open {
            Single,
            Double,
            Dollar,
            Backtick,
        }

        let mut stack: Vec<Open> = Vec::new();
        let mut chars = source.chars().peekable();
        while let Some(ch) = chars.next() {
            let in_single = stack.last() == Some(&Open::Single);
            if ch == escape_char && !in_single {
                chars.next();
                continue;
            }
            match ch {
                '\'' if !in_single && stack.last() != Some(&Open::Double) => {
                    stack.push(Open::Single)
                }
                '\'' if stack.last() == Some(&Open::Single) => {
                    stack.pop();
                }
                '"' if !in_single => {
                    if stack.last() == Some(&Open::Double) {
                        stack.pop();
                    } else {
                        stack.push(Open::Double);
                    }
                }
                // Backtick command substitution only exists in POSIX shells; when PowerShell uses
                // backtick as its escape char, the branch above already consumed it and its
                // following character, so this arm is unreachable for PowerShell.
                '`' if !in_single && escape_char != '`' => {
                    if stack.last() == Some(&Open::Backtick) {
                        stack.pop();
                    } else {
                        stack.push(Open::Backtick);
                    }
                }
                '$' if !in_single && chars.peek() == Some(&'(') => {
                    chars.next();
                    stack.push(Open::Dollar);
                }
                ')' if !in_single && stack.last() == Some(&Open::Dollar) => {
                    stack.pop();
                }
                _ => {}
            }
        }

        if stack.is_empty() {
            return None;
        }

        let mut closers = String::new();
        let mut open_delimiters = Vec::new();
        let dollar_paren_delimiter = if escape_char == '`' {
            OpenDelimiter::PowerShellSubexpression
        } else {
            OpenDelimiter::DollarParen
        };
        for open in stack.into_iter().rev() {
            match open {
                Open::Single => {
                    closers.push('\'');
                    open_delimiters.push(OpenDelimiter::SingleQuote);
                }
                Open::Double => {
                    closers.push('"');
                    open_delimiters.push(OpenDelimiter::DoubleQuote);
                }
                Open::Dollar => {
                    closers.push(')');
                    open_delimiters.push(dollar_paren_delimiter);
                }
                Open::Backtick => {
                    closers.push('`');
                    open_delimiters.push(OpenDelimiter::Backtick);
                }
            }
        }
        Some(Recovery {
            closers,
            open_delimiters,
        })
    }
}

// -------------------------------------------------------------------------------------------
// Node -> Warp model mapping.
// -------------------------------------------------------------------------------------------

fn clip(span: std::ops::Range<usize>, original_len: usize) -> Span {
    Span::new(span.start.min(original_len), span.end.min(original_len))
}

fn is_open(node: Node, original_len: usize) -> DelimiterState {
    if node.end_byte() > original_len {
        DelimiterState::Open
    } else {
        DelimiterState::Closed
    }
}

/// The original source text for `node`'s span, clipped so synthetic recovery bytes never leak
/// into a returned string.
fn node_text(node: Node, source: &str, original_len: usize) -> String {
    clip(node.byte_range(), original_len)
        .slice(source)
        .to_string()
}

fn starts_with_at(source: &str, pos: usize, prefix: &str) -> bool {
    source
        .get(pos..)
        .is_some_and(|rest| rest.starts_with(prefix))
}

fn named_children(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

/// Collects every top-level (or substitution-body) command from a container node, in source
/// order, flattening pipelines/statement-lists/groupings into sibling commands.
fn collect_commands(
    node: Node,
    source: &str,
    original_len: usize,
    dialect: ShellDialect,
) -> Vec<ParsedCommand> {
    let mut commands = Vec::new();
    match dialect {
        ShellDialect::PowerShell => {
            collect_powershell_commands(node, source, original_len, &mut commands)
        }
        ShellDialect::Fish => collect_fish_commands(node, source, original_len, &mut commands),
        ShellDialect::Bash | ShellDialect::Zsh => {
            collect_posix_commands(node, source, original_len, &mut commands)
        }
    }
    commands
}

// --- POSIX (Bash, and Zsh via the Bash grammar) --------------------------------------------

fn collect_posix_commands(
    node: Node,
    source: &str,
    original_len: usize,
    out: &mut Vec<ParsedCommand>,
) {
    match node.kind() {
        "program"
        | "list"
        | "pipeline"
        | "subshell"
        | "compound_statement"
        | "do_group"
        | "command_substitution"
        | "process_substitution" => {
            // `command_substitution`/`process_substitution` only reach this branch when treated
            // as a statement container (their content is itself a list of statements); as a word
            // constituent, they are handled by `collect_nested_groups` instead.
            for child in named_children(node) {
                collect_posix_commands(child, source, original_len, out);
            }
        }
        "redirected_statement" => {
            let start = out.len();
            if let Some(body) = node.child_by_field_name("body") {
                collect_posix_commands(body, source, original_len, out);
            }
            let redirects = posix_redirections(node, source, original_len);
            for command in &mut out[start..] {
                command.redirections.extend(redirects.clone());
            }
        }
        "command" => out.push(map_posix_command(node, source, original_len)),
        _ => {
            // Unrecognized container kind (e.g. `negated_command`, `test_command`): best-effort
            // recurse into named children rather than silently dropping the command entirely.
            for child in named_children(node) {
                collect_posix_commands(child, source, original_len, out);
            }
        }
    }
}

fn posix_redirections(node: Node, source: &str, original_len: usize) -> Vec<ShellRedirection> {
    let mut cursor = node.walk();
    node.children_by_field_name("redirect", &mut cursor)
        .map(|redirect| map_posix_redirect(redirect, source, original_len))
        .collect()
}

fn map_posix_redirect(node: Node, source: &str, original_len: usize) -> ShellRedirection {
    let destination = node
        .child_by_field_name("destination")
        .map(|d| clip(d.byte_range(), original_len));
    let operator_end = destination.map(|d| d.start()).unwrap_or(node.end_byte());
    let operator_span = clip(node.start_byte()..operator_end, original_len);
    let kind = match node.kind() {
        "heredoc_redirect" => ShellRedirectionKind::HereDocument,
        "herestring_redirect" => ShellRedirectionKind::HereString,
        _ => redirect_kind_from_operator_text(operator_span.slice(source).trim()),
    };
    ShellRedirection {
        operator_span,
        destination_span: destination,
        kind,
    }
}

fn redirect_kind_from_operator_text(text: &str) -> ShellRedirectionKind {
    match text {
        "<" => ShellRedirectionKind::Input,
        ">>" => ShellRedirectionKind::Append,
        ">" | "&>" | ">|" => ShellRedirectionKind::Output,
        text if text.contains("<&") || text.contains(">&") => ShellRedirectionKind::FileDescriptor,
        _ => ShellRedirectionKind::Output,
    }
}

fn map_posix_command(node: Node, source: &str, original_len: usize) -> ParsedCommand {
    let mut parts = Vec::new();
    let mut leading_assignments = Vec::new();
    let mut executable = None;
    let mut nested_groups = Vec::new();

    for child in named_children(node) {
        match child.kind() {
            "variable_assignment" => {
                let text = node_text(child, source, original_len);
                let span = clip(child.byte_range(), original_len);
                if executable.is_none() {
                    leading_assignments.push(text.clone().spanned(span));
                }
                collect_nested_groups(child, source, original_len, &mut nested_groups);
                parts.push(ParsedWord {
                    span,
                    raw: text.clone(),
                    completion_value: text,
                });
            }
            "command_name" => {
                let word = map_posix_argument(child, source, original_len, &mut nested_groups);
                if executable.is_none() {
                    executable = Some(word.completion_value.clone().spanned(word.span));
                }
                parts.push(word);
            }
            kind if is_redirect_kind(kind) => {
                // Redirects attached directly to the command are already reflected via
                // `posix_redirections(node, ...)` below; skip them here to avoid double-counting
                // as a positional word.
            }
            _ => parts.push(map_posix_argument(
                child,
                source,
                original_len,
                &mut nested_groups,
            )),
        }
    }

    // Escaped nested backticks (e.g. `` `echo \`rm -rf /\`` ``, APP-5433) are not modeled as a
    // nested `command_substitution` by the Bash grammar at all: it tokenizes the escaped
    // backticks as literal characters split across ordinary word arguments. Detect that pattern
    // directly against this command's own source text and inject the resulting nested group, so
    // permission decomposition still exposes the innermost command to deny rules. Any region
    // already covered by a natural nested group (e.g. this command's own text includes a whole
    // outer backtick substitution as one of its arguments) is excluded, since that region's
    // *own* command mapping already runs this same detection on the way down -- without the
    // exclusion, an outer command whose argument contains an escaped-backtick pair nested two
    // levels down would detect and inject the same group a second time at the outer level.
    let command_span = clip(node.byte_range(), original_len);
    let covered: Vec<Span> = nested_groups.iter().map(|g| g.span).collect();
    nested_groups.extend(detect_escaped_backtick_groups(
        command_span,
        source,
        &covered,
    ));

    ParsedCommand {
        span: command_span,
        parts,
        leading_assignments,
        executable,
        post_whitespace: None,
        nested_groups,
        redirections: posix_redirections(node, source, original_len),
    }
}

/// Maps a command argument that may itself be a substitution node directly (rather than a `word`
/// or `string` node that merely *contains* one), which is how the Bash grammar represents e.g.
/// `<(...)` process substitution used as a whole argument.
fn map_posix_argument(
    node: Node,
    source: &str,
    original_len: usize,
    nested_groups: &mut Vec<NestedCommandGroup>,
) -> ParsedWord {
    if matches!(node.kind(), "command_substitution" | "process_substitution") {
        push_nested_group(node, source, original_len, nested_groups);
        return ParsedWord {
            span: clip(node.byte_range(), original_len),
            raw: node_text(node, source, original_len),
            completion_value: "$(...)".to_string(),
        };
    }
    map_word(node, source, original_len, nested_groups)
}

/// Finds escaped-backtick-delimited regions in `command_span`'s own text (`` \`...\` ``, ignoring
/// content inside single quotes and inside `exclude`d spans) and maps each to a
/// `NestedCommandGroup` by reparsing the unescaped inner text as its own Bash program. Only
/// single-quote state is tracked (not double quotes), since escaped backticks inside a
/// *double*-quoted string are still tokenized as ordinary word text by the grammar and reach this
/// same command-level scan either way.
fn detect_escaped_backtick_groups(
    command_span: Span,
    source: &str,
    exclude: &[Span],
) -> Vec<NestedCommandGroup> {
    let command_source = command_span.slice(source);
    let base = command_span.start();
    let mut groups = Vec::new();
    let bytes = command_source.as_bytes();
    let mut i = 0;
    let mut in_single_quote = false;
    while i < bytes.len() {
        if let Some(skip_to) = exclude
            .iter()
            .find(|span| span.start() <= base + i && base + i < span.end())
            .map(|span| span.end() - base)
        {
            i = skip_to;
            continue;
        }
        match bytes[i] {
            b'\'' => {
                in_single_quote = !in_single_quote;
                i += 1;
            }
            b'\\' if !in_single_quote && bytes.get(i + 1) == Some(&b'`') => {
                let open_start = i;
                let content_start = i + 2;
                if let Some(rel_close) = find_escaped_backtick(&command_source[content_start..]) {
                    let content_end = content_start + rel_close;
                    let close_end = content_end + 2;
                    let inner_text =
                        unescape_backticks(&command_source[content_start..content_end]);
                    let commands = parse_fragment_as_bash(&inner_text, base + content_start);
                    groups.push(NestedCommandGroup {
                        span: Span::new(base + open_start, base + close_end),
                        content_span: Span::new(base + content_start, base + content_end),
                        kind: NestedCommandKind::BacktickSubstitution,
                        closure: DelimiterState::Closed,
                        commands,
                    });
                    i = close_end;
                    continue;
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    groups
}

/// Finds the byte offset (relative to `text`) of the next escaped backtick (`` \` ``), which
/// closes an escaped-backtick-delimited region opened by `detect_escaped_backtick_groups`.
fn find_escaped_backtick(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\\' && bytes[i + 1] == b'`' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn unescape_backticks(text: &str) -> String {
    text.replace("\\`", "`")
}

/// Parses `text` as a standalone Bash fragment and offsets every resulting span by `base_offset`
/// so it lines up with the original document. Used for regions the grammar does not expose as a
/// nested node at all (currently only escaped-backtick substitutions).
fn parse_fragment_as_bash(text: &str, base_offset: usize) -> Vec<ParsedCommand> {
    let Some(tree) = parse_tree(ShellDialect::Bash, text) else {
        return Vec::new();
    };
    if tree.root_node().has_error() {
        return Vec::new();
    }
    let mut commands = collect_commands(tree.root_node(), text, text.len(), ShellDialect::Bash);
    offset_commands(&mut commands, base_offset);
    commands
}

fn offset_commands(commands: &mut [ParsedCommand], base_offset: usize) {
    for command in commands {
        command.span = offset_span(command.span, base_offset);
        command.post_whitespace = command.post_whitespace.map(|s| offset_span(s, base_offset));
        for word in &mut command.parts {
            word.span = offset_span(word.span, base_offset);
        }
        for assignment in &mut command.leading_assignments {
            assignment.span = offset_span(assignment.span, base_offset);
        }
        if let Some(executable) = &mut command.executable {
            executable.span = offset_span(executable.span, base_offset);
        }
        for redirect in &mut command.redirections {
            redirect.operator_span = offset_span(redirect.operator_span, base_offset);
            redirect.destination_span = redirect
                .destination_span
                .map(|s| offset_span(s, base_offset));
        }
        for group in &mut command.nested_groups {
            group.span = offset_span(group.span, base_offset);
            group.content_span = offset_span(group.content_span, base_offset);
            offset_commands(&mut group.commands, base_offset);
        }
    }
}

fn offset_span(span: Span, base_offset: usize) -> Span {
    Span::new(span.start() + base_offset, span.end() + base_offset)
}

fn is_redirect_kind(kind: &str) -> bool {
    matches!(
        kind,
        "file_redirect" | "heredoc_redirect" | "herestring_redirect"
    )
}

/// Maps a word-like node (`word`, `string`, `concatenation`, `raw_string`, `simple_expansion`,
/// ...) to a `ParsedWord`, extracting any nested command/process substitutions it contains along
/// the way.
fn map_word(
    node: Node,
    source: &str,
    original_len: usize,
    nested_groups: &mut Vec<NestedCommandGroup>,
) -> ParsedWord {
    let raw = node_text(node, source, original_len);
    collect_nested_groups(node, source, original_len, nested_groups);
    let has_substitution = has_descendant_kind(node, "command_substitution")
        || has_descendant_kind(node, "process_substitution");
    let completion_value = if has_substitution {
        "$(...)".to_string()
    } else {
        raw.trim_matches(|c| c == '"' || c == '\'').to_string()
    };
    ParsedWord {
        span: clip(node.byte_range(), original_len),
        raw,
        completion_value,
    }
}

/// Recurses into `node`'s children looking for `command_substitution`/`process_substitution`
/// nodes. Does not check `node` itself; callers that may be handed a substitution node directly
/// (e.g. `<(...)` used as a whole command argument) must check `node.kind()` first and call
/// `push_nested_group` themselves -- see `map_posix_argument`.
fn collect_nested_groups(
    node: Node,
    source: &str,
    original_len: usize,
    out: &mut Vec<NestedCommandGroup>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "command_substitution" | "process_substitution"
        ) {
            push_nested_group(child, source, original_len, out);
        } else {
            collect_nested_groups(child, source, original_len, out);
        }
    }
}

/// Maps a single `command_substitution`/`process_substitution` node to a `NestedCommandGroup`.
fn push_nested_group(
    node: Node,
    source: &str,
    original_len: usize,
    out: &mut Vec<NestedCommandGroup>,
) {
    let (kind, delimiter_len) = match node.kind() {
        "command_substitution" => {
            if starts_with_at(source, node.start_byte(), "`") {
                (NestedCommandKind::BacktickSubstitution, 1)
            } else {
                (NestedCommandKind::DollarSubstitution, 2)
            }
        }
        _ => {
            if starts_with_at(source, node.start_byte(), "<(") {
                (NestedCommandKind::InputProcessSubstitution, 2)
            } else {
                (NestedCommandKind::OutputProcessSubstitution, 2)
            }
        }
    };
    out.push(NestedCommandGroup {
        span: clip(node.byte_range(), original_len),
        content_span: inner_content_span(node, original_len, delimiter_len),
        kind,
        closure: is_open(node, original_len),
        commands: collect_commands(node, source, original_len, ShellDialect::Bash),
    });
}

/// The span between a group's opening and closing delimiters. `open_delimiter_len` is the number
/// of bytes making up the opening delimiter (e.g. 2 for `$(`/`<(`, 1 for a lone backtick).
fn inner_content_span(node: Node, original_len: usize, open_delimiter_len: usize) -> Span {
    let start = node.start_byte() + open_delimiter_len;
    let end = if is_open(node, original_len) == DelimiterState::Open {
        node.end_byte()
    } else {
        node.end_byte().saturating_sub(1)
    };
    clip(start..end.max(start), original_len)
}

// --- Fish -------------------------------------------------------------------------------------

fn collect_fish_commands(
    node: Node,
    source: &str,
    original_len: usize,
    out: &mut Vec<ParsedCommand>,
) {
    match node.kind() {
        "command" => out.push(map_fish_command(node, source, original_len)),
        _ => {
            for child in named_children(node) {
                collect_fish_commands(child, source, original_len, out);
            }
        }
    }
}

fn map_fish_command(node: Node, source: &str, original_len: usize) -> ParsedCommand {
    let mut parts = Vec::new();
    let mut executable = None;
    let mut nested_groups = Vec::new();

    for (i, child) in named_children(node).into_iter().enumerate() {
        let word = map_fish_argument(child, source, original_len, &mut nested_groups);
        if i == 0 {
            executable = Some(word.completion_value.clone().spanned(word.span));
        }
        parts.push(word);
    }

    ParsedCommand {
        span: clip(node.byte_range(), original_len),
        parts,
        leading_assignments: Vec::new(),
        executable,
        post_whitespace: None,
        nested_groups,
        redirections: Vec::new(),
    }
}

/// Maps a Fish command argument that may itself be a `command_substitution` node directly (rather
/// than a `word` node that merely *contains* one).
fn map_fish_argument(
    node: Node,
    source: &str,
    original_len: usize,
    nested_groups: &mut Vec<NestedCommandGroup>,
) -> ParsedWord {
    if node.kind() == "command_substitution" {
        push_fish_nested_group(node, source, original_len, nested_groups);
        return ParsedWord {
            span: clip(node.byte_range(), original_len),
            raw: node_text(node, source, original_len),
            completion_value: "$(...)".to_string(),
        };
    }
    map_fish_word(node, source, original_len, nested_groups)
}

fn map_fish_word(
    node: Node,
    source: &str,
    original_len: usize,
    nested_groups: &mut Vec<NestedCommandGroup>,
) -> ParsedWord {
    let raw = node_text(node, source, original_len);
    collect_fish_nested_groups(node, source, original_len, nested_groups);
    let has_substitution = has_descendant_kind(node, "command_substitution");
    let completion_value = if has_substitution {
        "$(...)".to_string()
    } else {
        raw.clone()
    };
    ParsedWord {
        span: clip(node.byte_range(), original_len),
        raw,
        completion_value,
    }
}

fn collect_fish_nested_groups(
    node: Node,
    source: &str,
    original_len: usize,
    out: &mut Vec<NestedCommandGroup>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "command_substitution" {
            push_fish_nested_group(child, source, original_len, out);
        } else {
            collect_fish_nested_groups(child, source, original_len, out);
        }
    }
}

fn push_fish_nested_group(
    node: Node,
    source: &str,
    original_len: usize,
    out: &mut Vec<NestedCommandGroup>,
) {
    out.push(NestedCommandGroup {
        span: clip(node.byte_range(), original_len),
        content_span: inner_content_span(node, original_len, 1),
        kind: NestedCommandKind::FishSubstitution,
        closure: is_open(node, original_len),
        commands: collect_commands(node, source, original_len, ShellDialect::Fish),
    });
}

// --- PowerShell ---------------------------------------------------------------------------

fn collect_powershell_commands(
    node: Node,
    source: &str,
    original_len: usize,
    out: &mut Vec<ParsedCommand>,
) {
    match node.kind() {
        "command" => out.push(map_powershell_command(node, source, original_len)),
        _ => {
            for child in named_children(node) {
                collect_powershell_commands(child, source, original_len, out);
            }
        }
    }
}

fn map_powershell_command(node: Node, source: &str, original_len: usize) -> ParsedCommand {
    let mut parts = Vec::new();
    let mut executable = None;
    let mut nested_groups = Vec::new();

    if let Some(name) = node.child_by_field_name("command_name") {
        let word = map_powershell_word(name, source, original_len, &mut nested_groups);
        executable = Some(word.completion_value.clone().spanned(word.span));
        parts.push(word);
    }
    if let Some(elements) = node.child_by_field_name("command_elements") {
        for child in named_children(elements) {
            parts.push(map_powershell_word(
                child,
                source,
                original_len,
                &mut nested_groups,
            ));
        }
    }

    ParsedCommand {
        span: clip(node.byte_range(), original_len),
        parts,
        leading_assignments: Vec::new(),
        executable,
        post_whitespace: None,
        nested_groups,
        redirections: Vec::new(),
    }
}

fn map_powershell_word(
    node: Node,
    source: &str,
    original_len: usize,
    nested_groups: &mut Vec<NestedCommandGroup>,
) -> ParsedWord {
    let raw = node_text(node, source, original_len);
    collect_powershell_nested_groups(node, source, original_len, nested_groups);
    let has_substitution = has_descendant_kind(node, "sub_expression");
    let completion_value = if has_substitution {
        "$(...)".to_string()
    } else {
        raw.clone()
    };
    ParsedWord {
        span: clip(node.byte_range(), original_len),
        raw,
        completion_value,
    }
}

fn has_descendant_kind(node: Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|c| c.kind() == kind || has_descendant_kind(c, kind))
}

fn collect_powershell_nested_groups(
    node: Node,
    source: &str,
    original_len: usize,
    out: &mut Vec<NestedCommandGroup>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "sub_expression" {
            let commands = child
                .child_by_field_name("statements")
                .map(|s| collect_commands(s, source, original_len, ShellDialect::PowerShell))
                .unwrap_or_default();
            out.push(NestedCommandGroup {
                span: clip(child.byte_range(), original_len),
                content_span: inner_content_span(child, original_len, 2),
                kind: NestedCommandKind::PowerShellSubexpression,
                closure: is_open(child, original_len),
                commands,
            });
        } else {
            collect_powershell_nested_groups(child, source, original_len, out);
        }
    }
}
