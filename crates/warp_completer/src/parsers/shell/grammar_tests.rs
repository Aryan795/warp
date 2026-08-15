//! Grammar conformance tests for the Phase 0 "grammar viability gate" (`specs/APP-5430/TECH.md`).
//!
//! These tests exercise the raw Arborium/tree-sitter grammars directly (not the Warp-owned
//! `ParsedShellInput` model, which is built in Phase 1) to lock down the hierarchy and spans the
//! Phase 1 adapter must project. Per the gate:
//!
//! - Every valid complete input must produce stable executable spans and nested-command ownership
//!   with no `ERROR` nodes.
//! - Incomplete inputs must produce the documented recovered hierarchy (here: the grammar's raw,
//!   pre-adapter-recovery behavior, which the Phase 1 adapter is required to normalize).
//!
//! Bash, Fish, and PowerShell pass the complete-input half of the gate below using their own
//! Arborium grammar. Zsh maps to the Bash grammar instead (a requester decision on APP-5430
//! Phase 0, overriding the spec's original prohibition on that fallback): Arborium 2.18.1's own
//! Zsh grammar fails to parse even a single bare command without error, and fixing or replacing
//! it is an open-ended upstream investigation rather than a narrowly scoped patch. The Zsh
//! section below documents where the Bash grammar gets Zsh-only syntax wrong -- most constructs
//! surface as a visible `ERROR` node, but at least one (`repeat ... do ... done`) parses without
//! error into a completely wrong, misleading hierarchy. That is the dangerous case: a consumer
//! cannot tell from `has_error()` alone that the result is nonsense.

use arborium::tree_sitter::{Node, Parser, Tree};

use super::ShellDialect;

fn parse(dialect: ShellDialect, source: &str) -> Tree {
    let mut parser = Parser::new();
    parser
        .set_language(&dialect.grammar())
        .expect("grammar version should be compatible with the vendored tree-sitter runtime");
    parser
        .parse(source, None)
        .expect("parsing a string always produces a tree")
}

/// Returns the first descendant of `node` (never `node` itself) with the given kind, searched
/// depth-first, pre-order.
///
/// Deliberately excludes `node` itself: every caller here passes an ancestor (often a node of the
/// very kind being searched for, e.g. looking for a nested `command_substitution` inside another
/// `command_substitution`) and wants the *nested* occurrence. Matching on `node` itself would
/// make that search vacuous -- it would return the ancestor without ever inspecting its children.
fn find_descendant_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find_map(|child| {
        if child.kind() == kind {
            Some(child)
        } else {
            find_descendant_kind(child, kind)
        }
    })
}

/// Returns every descendant of `node` (never `node` itself) with the given kind, in depth-first,
/// pre-order. Use this instead of `find_descendant_kind` whenever a construct is expected to
/// contain more than one occurrence (e.g. two process substitutions) -- taking only the first
/// match would silently ignore a collapsed, dropped, or duplicated second one.
fn find_all_descendant_kind<'tree>(node: Node<'tree>, kind: &str) -> Vec<Node<'tree>> {
    let mut results = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            results.push(child);
        }
        results.extend(find_all_descendant_kind(child, kind));
    }
    results
}

fn kinds_present(node: Node, kinds: &[&str]) -> bool {
    kinds
        .iter()
        .all(|kind| find_descendant_kind(node, kind).is_some())
}

// ---------------------------------------------------------------------------------------------
// Bash: complete-input conformance.
// ---------------------------------------------------------------------------------------------

#[test]
fn bash_dollar_paren_nesting_is_error_free() {
    let source = "echo \"$(a $(b $(c)))\"";
    let tree = parse(ShellDialect::Bash, source);
    assert!(!tree.root_node().has_error());
    // Three levels of `command_substitution`, each strictly nested inside the previous one (not
    // flattened into siblings), with exact spans for every level.
    let outer = find_descendant_kind(tree.root_node(), "command_substitution").unwrap();
    assert_eq!(&source[outer.byte_range()], "$(a $(b $(c)))");
    let middle = find_descendant_kind(outer, "command_substitution").unwrap();
    assert_eq!(&source[middle.byte_range()], "$(b $(c))");
    let inner = find_descendant_kind(middle, "command_substitution").unwrap();
    assert_eq!(&source[inner.byte_range()], "$(c)");
    // The innermost level has no further nested substitution.
    assert!(find_descendant_kind(inner, "command_substitution").is_none());
}

#[test]
fn bash_backtick_substitution_is_error_free() {
    let tree = parse(ShellDialect::Bash, "echo `pwd`");
    assert!(!tree.root_node().has_error());
    assert!(find_descendant_kind(tree.root_node(), "command_substitution").is_some());
}

#[test]
fn bash_substitution_inside_double_quoted_concatenated_word_is_error_free() {
    let source = "echo pre\"mid$(pwd)post\"tail";
    let tree = parse(ShellDialect::Bash, source);
    assert!(!tree.root_node().has_error());
    let substitution = find_descendant_kind(tree.root_node(), "command_substitution").unwrap();
    // The nested `pwd` command's span must point at exactly "pwd", not swallow surrounding text.
    let inner_command = find_descendant_kind(substitution, "command_name").unwrap();
    assert_eq!(&source[inner_command.byte_range()], "pwd");
}

#[test]
fn bash_process_substitution_is_error_free() {
    let source = "diff <(sort a) <(sort b)";
    let tree = parse(ShellDialect::Bash, source);
    assert!(!tree.root_node().has_error());
    // Both `<(...)` process substitutions must be represented as distinct, correctly spanned
    // nodes -- not collapsed into one, dropped, or mistaken for plain redirects.
    let substitutions = find_all_descendant_kind(tree.root_node(), "process_substitution");
    let spans: Vec<&str> = substitutions
        .iter()
        .map(|node| &source[node.byte_range()])
        .collect();
    assert_eq!(spans, vec!["<(sort a)", "<(sort b)"]);
}

#[test]
fn bash_pipeline_and_statement_list_inside_substitution_is_error_free() {
    let tree = parse(ShellDialect::Bash, "echo \"$(a $(b | c) && d; e)\"");
    assert!(!tree.root_node().has_error());
    let substitution = find_descendant_kind(tree.root_node(), "command_substitution").unwrap();
    assert!(kinds_present(substitution, &["pipeline", "list"]));
}

#[test]
fn bash_assignment_and_redirect_inside_nested_command_is_error_free() {
    let source = "echo \"$(KEY=VALUE env >out)\"";
    let tree = parse(ShellDialect::Bash, source);
    assert!(!tree.root_node().has_error());
    let substitution = find_descendant_kind(tree.root_node(), "command_substitution").unwrap();
    let assignment = find_descendant_kind(substitution, "variable_assignment").unwrap();
    assert_eq!(&source[assignment.byte_range()], "KEY=VALUE");
    let redirect = find_descendant_kind(substitution, "file_redirect").unwrap();
    let destination = find_descendant_kind(redirect, "word").unwrap();
    assert_eq!(&source[destination.byte_range()], "out");
}

#[test]
fn bash_heredoc_is_error_free() {
    let tree = parse(ShellDialect::Bash, "cat <<EOF\nhello\nEOF");
    assert!(!tree.root_node().has_error());
    assert!(kinds_present(
        tree.root_node(),
        &["heredoc_redirect", "heredoc_body"]
    ));
}

// ---------------------------------------------------------------------------------------------
// Bash: documented incomplete-input grammar behavior (adapter recovery is a Phase 1 concern).
// ---------------------------------------------------------------------------------------------

/// Per the spec's empirical findings: Bash reduces an incomplete `$()` to an `ERROR` node without
/// a nested `command`. This is the grammar's raw (pre-adapter) behavior; the Phase 1 adapter must
/// supply its own EOF recovery rather than relying on the grammar's error productions here.
#[test]
fn bash_incomplete_substitution_has_no_nested_command_at_grammar_level() {
    let tree = parse(ShellDialect::Bash, "echo \"pre$(pw");
    assert!(tree.root_node().has_error());
    assert!(find_descendant_kind(tree.root_node(), "command_substitution").is_none());
}

// ---------------------------------------------------------------------------------------------
// Fish: complete-input conformance, including the synthetic-newline recovery requirement.
// ---------------------------------------------------------------------------------------------

#[test]
fn fish_command_substitution_requires_trailing_newline() {
    // Without a terminator, an otherwise-valid Fish buffer is missing a statement separator.
    let without_newline = parse(ShellDialect::Fish, "echo (pwd)");
    assert!(without_newline.root_node().has_error());

    // The adapter must append and then clip a sentinel newline (see the spec's "Incomplete-input
    // recovery" section); appending one directly here confirms the grammar accepts the result.
    let with_newline = parse(ShellDialect::Fish, "echo (pwd)\n");
    assert!(!with_newline.root_node().has_error());
    assert!(find_descendant_kind(with_newline.root_node(), "command_substitution").is_some());
}

#[test]
fn fish_nested_pipe_inside_substitution_is_error_free() {
    let tree = parse(ShellDialect::Fish, "cat (printf x | psub)\n");
    assert!(!tree.root_node().has_error());
    let substitution = find_descendant_kind(tree.root_node(), "command_substitution").unwrap();
    assert!(find_descendant_kind(substitution, "pipe").is_some());
}

// ---------------------------------------------------------------------------------------------
// PowerShell: complete-input conformance.
// ---------------------------------------------------------------------------------------------

#[test]
fn powershell_expandable_string_subexpression_is_error_free() {
    let tree = parse(ShellDialect::PowerShell, "echo \"pre$(pwd)post\"");
    assert!(!tree.root_node().has_error());
    assert!(find_descendant_kind(tree.root_node(), "sub_expression").is_some());
}

#[test]
fn powershell_three_level_nesting_is_error_free() {
    let source = "echo \"$(a $(b) $(c))\"";
    let tree = parse(ShellDialect::PowerShell, source);
    assert!(!tree.root_node().has_error());
    let outer = find_descendant_kind(tree.root_node(), "sub_expression").unwrap();
    assert_eq!(&source[outer.byte_range()], "$(a $(b) $(c))");
    // Two sibling `$(...)` substitutions, both nested inside the outer one, with exact spans.
    let inner = find_all_descendant_kind(outer, "sub_expression");
    let spans: Vec<&str> = inner
        .iter()
        .map(|node| &source[node.byte_range()])
        .collect();
    assert_eq!(spans, vec!["$(b)", "$(c)"]);
}

#[test]
fn powershell_pipeline_and_statement_list_is_error_free() {
    let tree = parse(
        ShellDialect::PowerShell,
        "Get-Process | Where-Object { $_.CPU -gt 0 }; echo done",
    );
    assert!(!tree.root_node().has_error());
    assert!(kinds_present(
        tree.root_node(),
        &["pipeline", "statement_list"]
    ));
}

// ---------------------------------------------------------------------------------------------
// Zsh (via the Bash grammar): documented conformance and divergences.
// ---------------------------------------------------------------------------------------------

/// Zsh syntax that the Bash grammar happens to also cover correctly, because POSIX-style
/// commands, pipelines, and substitutions are shared between the two dialects.
#[test]
fn zsh_posix_compatible_syntax_is_error_free() {
    let tree = parse(
        ShellDialect::Zsh,
        "echo \"$(a $(b))\" | grep x && echo done",
    );
    assert!(!tree.root_node().has_error());
    assert!(find_descendant_kind(tree.root_node(), "command_substitution").is_some());
}

/// Zsh named-pipe process substitution (`=(...)`) has no Bash equivalent (Bash only has
/// `<(...)`/`>(...)`). The Bash grammar visibly fails on it rather than silently misparsing it.
#[test]
fn zsh_named_pipe_process_substitution_is_visibly_an_error() {
    let tree = parse(ShellDialect::Zsh, "diff <(sort a) =(sort b)");
    assert!(tree.root_node().has_error());
}

/// Zsh anonymous functions (`() { ... }`, with no name before the parens) are not valid Bash
/// function syntax. The Bash grammar visibly fails on both spellings.
#[test]
fn zsh_anonymous_function_is_visibly_an_error() {
    for source in ["() { echo hi }", "function () { echo hi }"] {
        let tree = parse(ShellDialect::Zsh, source);
        assert!(tree.root_node().has_error(), "expected {source:?} to error");
    }
}

/// Zsh's short-form `for name (list) body` loop (omitting `in` and using parens instead of
/// `do`/`done`) is not valid Bash `for` syntax. The Bash grammar visibly fails on it.
#[test]
fn zsh_short_form_for_loop_is_visibly_an_error() {
    let tree = parse(ShellDialect::Zsh, "for i (1 2 3) print $i");
    assert!(tree.root_node().has_error());
}

/// Zsh glob qualifiers (e.g. `(.)` to match only plain files) are not valid Bash word syntax.
/// The Bash grammar visibly fails on them.
#[test]
fn zsh_glob_qualifier_is_visibly_an_error() {
    let tree = parse(ShellDialect::Zsh, "ls *.txt(.)");
    assert!(tree.root_node().has_error());
}

/// Zsh parameter-expansion flags (e.g. `${(f)...}` to split on newlines) are not valid Bash
/// parameter-expansion syntax. The Bash grammar visibly fails on them.
#[test]
fn zsh_parameter_expansion_flag_is_visibly_an_error() {
    let tree = parse(ShellDialect::Zsh, "echo ${(f)\"$(cmd)\"}");
    assert!(tree.root_node().has_error());
}

/// Zsh's `try`/`always` exception-handling block has no Bash equivalent. The Bash grammar
/// visibly fails on it.
#[test]
fn zsh_always_block_is_visibly_an_error() {
    let tree = parse(ShellDialect::Zsh, "{\n  risky\n} always {\n  cleanup\n}");
    assert!(tree.root_node().has_error());
}

/// Zsh's `$+name` parameter-existence check is not valid Bash parameter-expansion syntax. The
/// Bash grammar visibly fails on it.
#[test]
fn zsh_parameter_existence_check_is_visibly_an_error() {
    let tree = parse(ShellDialect::Zsh, "echo $+name");
    assert!(tree.root_node().has_error());
}

/// **Dangerous divergence, reported to the requester alongside the Phase 0 PR.** Zsh's `repeat
/// COUNT do ... done` loop (no Bash equivalent) does not error under the Bash grammar at all: it
/// parses as three unrelated top-level commands (`repeat 3 do`, `echo hi`, `done`), none of which
/// is a loop. `has_error()` returns `false`, so a consumer cannot detect the misparse from grammar
/// status alone. Any adapter code that maps Zsh through the Bash grammar must not treat an
/// error-free parse as proof of a correct hierarchy; this case is the reason why.
#[test]
fn zsh_repeat_loop_parses_without_error_into_a_wrong_hierarchy() {
    let tree = parse(ShellDialect::Zsh, "repeat 3 do\necho hi\ndone");
    assert!(
        !tree.root_node().has_error(),
        "documented as error-free; if this starts erroring, the grammar or its version changed \
         and this test (and the divergence report) needs to be revisited"
    );
    // Three sibling top-level commands, not one loop construct: `repeat 3 do` (a plain command
    // with two positional arguments), `echo hi`, and `done` (a bare, argument-less command).
    let mut cursor = tree.root_node().walk();
    let top_level: Vec<Node> = tree.root_node().children(&mut cursor).collect();
    assert_eq!(
        top_level.len(),
        3,
        "expected the repeat loop to be misparsed as three separate commands, got {}",
        tree.root_node().to_sexp()
    );
    assert!(find_descendant_kind(tree.root_node(), "do_group").is_none());
}

/// Zsh named-directory (`~name`) and Bash both leave the tilde-prefixed word as an opaque `word`
/// token: neither grammar performs tilde expansion at parse time, so this is not a real
/// divergence even though it is Zsh-flavored syntax.
#[test]
fn zsh_named_directory_reference_is_error_free_but_not_semantically_understood() {
    let tree = parse(ShellDialect::Zsh, "cd ~mydir");
    assert!(!tree.root_node().has_error());
}

// ---------------------------------------------------------------------------------------------
// Module boundary.
// ---------------------------------------------------------------------------------------------

/// No public item in `warp_completer` may contain an Arborium or tree-sitter type (see the
/// module boundary in `specs/APP-5430/TECH.md`). `shell` is a private module for exactly this
/// reason: nothing it exports is reachable from outside the crate today. This test exists so a
/// future Phase 1 change that makes `shell` (or `ShellDialect`) `pub` gets a clear signal to
/// re-examine the boundary rather than silently leaking `tree_sitter::Language` through it.
#[test]
fn shell_module_is_not_part_of_the_public_api() {
    // If this module is ever re-exported as `pub mod shell` from `crate::parsers`, the crate's
    // public API would gain a path to `arborium`/`tree_sitter` types. Grep-based rather than
    // type-based on purpose: the point is to catch the *declaration* changing, which a
    // compile-time trait check cannot observe from within the module it is guarding.
    let mod_rs = include_str!("../mod.rs");
    assert!(
        mod_rs.contains("mod shell;") && !mod_rs.contains("pub mod shell;"),
        "the shell module must stay private to warp_completer's public API"
    );
}
