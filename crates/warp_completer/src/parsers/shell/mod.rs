//! The tree-sitter shell parser adapter (APP-5430).
//!
//! This module owns Arborium grammar selection, `tree_sitter` usage, dialect-specific node
//! mapping, EOF recovery, and conversion to the Warp-owned types defined here. Per the module
//! boundary in `specs/APP-5430/TECH.md`, no `pub` item in this module (or re-exported from it)
//! may contain an Arborium or tree-sitter type; the raw tree-sitter usage lives in the private
//! `mapper` submodule. See `no_backend_types_in_public_api` in `adapter_tests.rs`.

use arborium::tree_sitter::Language;
use string_offset::ByteOffset;

use crate::meta::{Span, Spanned, SpannedItem};
use crate::parsers::LiteCommand;

mod mapper;

/// The shell dialects the adapter supports.
///
/// Arborium 2.18.1's own Zsh grammar (`georgeharker/tree-sitter-zsh`) fails to parse even the
/// simplest complete command without error (see the grammar conformance tests), and fixing or
/// replacing it is an open-ended upstream investigation outside this project's scope. Per the
/// requester's decision, `Zsh` maps to the Bash grammar instead, guarded by
/// [`mapper::ZshCompatibilityGuard`]: Zsh input that produces a non-EOF grammar error, or that
/// matches a known silently-misparsed Zsh-only construct (e.g. `repeat`), is rejected with
/// [`ShellParseRejection::UnsupportedDialectSyntax`] rather than returned as if it were correct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShellDialect {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

impl ShellDialect {
    /// Returns the Arborium language name used by `arborium::get_language`.
    fn arborium_name(self) -> &'static str {
        match self {
            // Zsh intentionally reuses the Bash grammar; see the `ShellDialect` doc comment.
            Self::Bash | Self::Zsh => "bash",
            Self::Fish => "fish",
            Self::PowerShell => "powershell",
        }
    }

    /// Returns the tree-sitter grammar for this dialect.
    ///
    /// Panics if the corresponding Arborium feature is disabled, which would be a build
    /// configuration bug: all three `lang-bash`/`lang-fish`/`lang-powershell` features are
    /// unconditionally enabled on the `arborium` dependency.
    pub(crate) fn grammar(self) -> Language {
        arborium::get_language(self.arborium_name())
            .unwrap_or_else(|| panic!("arborium grammar for {self:?} must be enabled"))
    }
}

/// Command input above this size is rejected rather than parsed. See the spec's "Parse ownership,
/// performance, and memory" section.
const MAX_INPUT_LEN: usize = 64 * 1024;

/// The result of parsing a command-input buffer for one dialect.
///
/// This is the Warp-owned replacement for the hand-written parser in `parsers::simple`. It is
/// produced by [`parse_shell_input`] and never contains an Arborium or tree-sitter type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedShellInput {
    pub dialect: ShellDialect,
    pub source_len: usize,
    pub commands: Vec<ParsedCommand>,
    pub status: ShellParseStatus,
}

/// A single command: an executable, its arguments, any leading environment-variable assignments,
/// nested command/process substitutions, and redirections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub span: Span,
    /// Every word in the command in source order, including leading assignments and the
    /// executable itself. `leading_assignments` and `executable` are convenience views into this
    /// same sequence, not separate data.
    pub parts: Vec<ParsedWord>,
    pub leading_assignments: Vec<Spanned<String>>,
    pub executable: Option<Spanned<String>>,
    pub post_whitespace: Option<Span>,
    pub nested_groups: Vec<NestedCommandGroup>,
    pub redirections: Vec<ShellRedirection>,
}

/// A single word within a [`ParsedCommand`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedWord {
    pub span: Span,
    /// The original source slice for this word, unmodified.
    pub raw: String,
    /// The unquoted/unescaped value used for completions. Preserves the current `$(...)`
    /// placeholder for a nested region, matching the legacy parser's `Part::Display` behavior.
    pub completion_value: String,
}

/// A command or process substitution nested within a [`ParsedCommand`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NestedCommandGroup {
    pub span: Span,
    pub content_span: Span,
    pub kind: NestedCommandKind,
    pub closure: DelimiterState,
    pub commands: Vec<ParsedCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NestedCommandKind {
    DollarSubstitution,
    BacktickSubstitution,
    FishSubstitution,
    PowerShellSubexpression,
    InputProcessSubstitution,
    OutputProcessSubstitution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelimiterState {
    Closed,
    Open,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellRedirection {
    pub operator_span: Span,
    pub destination_span: Option<Span>,
    pub kind: ShellRedirectionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellRedirectionKind {
    Input,
    Output,
    Append,
    HereDocument,
    HereString,
    FileDescriptor,
    ProcessSubstitution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellParseStatus {
    Complete,
    Recovered { open_delimiters: Vec<OpenDelimiter> },
    Rejected(ShellParseRejection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenDelimiter {
    SingleQuote,
    DoubleQuote,
    DollarParen,
    Backtick,
    FishParen,
    PowerShellSubexpression,
    ProcessSubstitution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellParseRejection {
    InputTooLarge,
    GrammarUnavailable,
    UnsupportedDialectSyntax,
    Unrecoverable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellParseOptions {
    pub quote_mode: QuoteMode,
}

impl Default for ShellParseOptions {
    fn default() -> Self {
        Self {
            quote_mode: QuoteMode::GroupQuotedText,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteMode {
    GroupQuotedText,
    PreserveQuotesAsLiterals,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionDecomposition {
    pub commands: Vec<String>,
    pub contains_redirection: bool,
    pub status: ShellParseStatus,
}

/// Parses `source` as `dialect` command input. Total: never panics on user input. Input above
/// [`MAX_INPUT_LEN`] is rejected without parsing.
pub fn parse_shell_input(
    source: &str,
    dialect: ShellDialect,
    options: ShellParseOptions,
) -> ParsedShellInput {
    if source.len() > MAX_INPUT_LEN {
        return ParsedShellInput {
            dialect,
            source_len: source.len(),
            commands: Vec::new(),
            status: ShellParseStatus::Rejected(ShellParseRejection::InputTooLarge),
        };
    }
    mapper::parse(source, dialect, options)
}

impl ParsedShellInput {
    /// Returns the deepest command whose span contains `cursor`, recursing into nested groups.
    pub fn deepest_command_at(&self, cursor: ByteOffset) -> Option<&ParsedCommand> {
        deepest_command_at(&self.commands, cursor.as_usize())
    }

    /// Returns the command that completions should act on for a cursor at `cursor`. Identical to
    /// [`Self::deepest_command_at`] except when the deepest command is inside a *closed* group and
    /// the cursor sits exactly at the end of input, in which case completion stays on the
    /// outermost command (matching the legacy parser's behavior for e.g. `echo "$(pwd)"`).
    pub fn completion_command_at(&self, cursor: ByteOffset) -> Option<&ParsedCommand> {
        let cursor = cursor.as_usize();
        if cursor == self.source_len {
            if let Some(open) = deepest_open_command_at(&self.commands, cursor) {
                return Some(open);
            }
            return self.commands.last();
        }
        deepest_command_at(&self.commands, cursor)
    }

    pub fn top_level_commands(&self) -> impl Iterator<Item = &ParsedCommand> {
        self.commands.iter()
    }

    /// Iterates every command in the input, depth-first, including nested commands.
    pub fn commands_depth_first(&self) -> impl Iterator<Item = &ParsedCommand> {
        self.commands.iter().flat_map(ParsedCommand::depth_first)
    }

    pub fn first_executable(&self) -> Option<&Spanned<String>> {
        self.commands.first()?.executable.as_ref()
    }

    /// Decomposes the input into every executable command at every nesting depth, in source
    /// order, for the agent-permissions deny/allow predicates. Fails closed: a recovered or
    /// rejected parse still returns whatever commands were found, but callers must consult
    /// `status` and treat anything other than `Complete` as inconclusive.
    ///
    /// Includes both each individual nested command *and* the recomposed text of each nested
    /// group as a whole (e.g. for `ls $(foo | echo)`, both `foo`, `echo`, and `foo | echo` are
    /// returned), matching the legacy parser's `decompose_command` so an anchored deny rule can
    /// match whichever granularity the pipeline/statement-list was written at.
    pub fn decompose_for_permissions(&self, source: &str) -> PermissionDecomposition {
        let mut commands = Vec::new();
        let mut contains_redirection = false;
        for command in &self.commands {
            decompose_command_into(command, source, &mut commands, &mut contains_redirection);
        }
        PermissionDecomposition {
            commands,
            contains_redirection,
            status: self.status.clone(),
        }
    }
}

fn decompose_command_into(
    command: &ParsedCommand,
    source: &str,
    out: &mut Vec<String>,
    contains_redirection: &mut bool,
) {
    *contains_redirection |= !command.redirections.is_empty();
    for group in &command.nested_groups {
        let text = group.content_span.slice(source).trim();
        if !text.is_empty() {
            out.push(text.to_string());
        }
        for nested in &group.commands {
            decompose_command_into(nested, source, out, contains_redirection);
        }
    }
    if let Some(text) = command.span_text(source) {
        out.push(text);
    }
}

fn deepest_command_at(commands: &[ParsedCommand], cursor: usize) -> Option<&ParsedCommand> {
    let containing = commands
        .iter()
        .find(|c| c.span.start() <= cursor && cursor <= c.span.end())?;
    for group in &containing.nested_groups {
        if group.span.start() <= cursor
            && cursor <= group.span.end()
            && let Some(nested) = deepest_command_at(&group.commands, cursor)
        {
            return Some(nested);
        }
    }
    Some(containing)
}

/// Like `deepest_command_at`, but only descends into `DelimiterState::Open` groups.
fn deepest_open_command_at(commands: &[ParsedCommand], cursor: usize) -> Option<&ParsedCommand> {
    let containing = commands
        .iter()
        .find(|c| c.span.start() <= cursor && cursor <= c.span.end())?;
    for group in &containing.nested_groups {
        if group.closure == DelimiterState::Open
            && group.span.start() <= cursor
            && cursor <= group.span.end()
        {
            if let Some(nested) = deepest_open_command_at(&group.commands, cursor) {
                return Some(nested);
            }
            return group.commands.last();
        }
    }
    None
}

impl ParsedCommand {
    /// Returns the source text spanned by this command, trimmed of trailing whitespace already
    /// excluded via `post_whitespace`.
    fn span_text(&self, source: &str) -> Option<String> {
        let range: std::ops::Range<usize> = self.span.into();
        source.get(range).map(|s| s.trim().to_string())
    }

    fn depth_first(&self) -> Box<dyn Iterator<Item = &ParsedCommand> + '_> {
        Box::new(
            std::iter::once(self).chain(
                self.nested_groups
                    .iter()
                    .flat_map(|group| group.commands.iter().flat_map(ParsedCommand::depth_first)),
            ),
        )
    }

    /// Projects this command onto the legacy `LiteCommand` shape used by `classify_command`.
    pub fn to_lite_command(&self) -> LiteCommand {
        LiteCommand {
            parts: self
                .parts
                .iter()
                .map(|word| word.completion_value.clone().spanned(word.span))
                .collect(),
            post_whitespace: self.post_whitespace,
        }
    }
}

#[cfg(test)]
#[path = "adapter_tests.rs"]
mod adapter_tests;
#[cfg(test)]
#[path = "grammar_tests.rs"]
mod grammar_tests;
#[cfg(test)]
#[path = "shadow_comparison_tests.rs"]
mod shadow_comparison_tests;
