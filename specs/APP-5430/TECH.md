# Tree-sitter shell parser behind a Warp adapter

## Summary

Replace the hand-written command-input parser with Arborium tree-sitter grammars for Bash, Fish, and PowerShell. Parse Zsh with the Bash grammar and a Zsh compatibility guard. Keep tree-sitter private to `warp_completer`. Expose a Warp-owned hierarchical command model to completions, Describe/X-Ray, input decorations, alias expansion, agent permissions, and other consumers. Migrate one consumer at a time, then delete `crates/warp_completer/src/parsers/simple/`.

This spec uses **nested command** to mean a command inside command substitution or process substitution. For example, `pwd` is nested in `echo "$(pwd)"`. Signature subcommands such as `git commit` are not the accuracy target of this refactor.

## Context

The source revision researched for this spec is [`e72fd7aacbbb2236d9b3be2aad7e7178fe94b4bc`](https://github.com/warpdotdev/warp/tree/e72fd7aacbbb2236d9b3be2aad7e7178fe94b4bc).

- [`crates/warp_completer/src/parsers/simple/mod.rs (17-136)`](https://github.com/warpdotdev/warp/blob/e72fd7aacbbb2236d9b3be2aad7e7178fe94b4bc/crates/warp_completer/src/parsers/simple/mod.rs#L17-L136) exposes `parse_for_completions`, `top_level_command`, `command_at_cursor_position`, `all_parsed_commands`, `command_without_leading_env_vars`, and `decompose_command`.
- [`crates/warp_completer/src/parsers/simple/mod.rs (139-284)`](https://github.com/warpdotdev/warp/blob/e72fd7aacbbb2236d9b3be2aad7e7178fe94b4bc/crates/warp_completer/src/parsers/simple/mod.rs#L139-L284) performs cursor selection, incomplete-subshell selection, and recursive command decomposition.
- [`crates/warp_completer/src/parsers/simple/parser.rs (160-359)`](https://github.com/warpdotdev/warp/blob/e72fd7aacbbb2236d9b3be2aad7e7178fe94b4bc/crates/warp_completer/src/parsers/simple/parser.rs#L160-L359) parses backticks, `$()`, quotes, and escape characters.
- [`crates/warp_completer/src/parsers/mod.rs (34-144)`](https://github.com/warpdotdev/warp/blob/e72fd7aacbbb2236d9b3be2aad7e7178fe94b4bc/crates/warp_completer/src/parsers/mod.rs#L34-L144) defines `LiteCommand` and feeds it to signature-backed `classify_command`. The signature HIR is Warp-specific and remains above the new adapter.
- [`app/src/ai/blocklist/permissions.rs (883-954)`](https://github.com/warpdotdev/warp/blob/e72fd7aacbbb2236d9b3be2aad7e7178fe94b4bc/app/src/ai/blocklist/permissions.rs#L883-L954) decomposes a command, normalizes leading assignments, and evaluates every returned string against command deny rules before any allow decision.
- [`crates/warp_terminal/src/shell/mod.rs (224-302)`](https://github.com/warpdotdev/warp/blob/e72fd7aacbbb2236d9b3be2aad7e7178fe94b4bc/crates/warp_terminal/src/shell/mod.rs#L224-L302) already distinguishes Bash, Zsh, Fish, and PowerShell as `ShellType`. `warp_terminal` depends on `warp_completer`, so `warp_completer` cannot import `ShellType`.
- [`crates/warp_util/src/path.rs (207-238)`](https://github.com/warpdotdev/warp/blob/e72fd7aacbbb2236d9b3be2aad7e7178fe94b4bc/crates/warp_util/src/path.rs#L207-L238) groups Bash, Zsh, and Fish as `ShellFamily::Posix` for escaping. This two-value type cannot express the four-dialect parser policy and remains an escaping API only.
- [`crates/languages/src/lib.rs (244-319)`](https://github.com/warpdotdev/warp/blob/e72fd7aacbbb2236d9b3be2aad7e7178fe94b4bc/crates/languages/src/lib.rs#L244-L319) maps file-editor `shell` highlighting to Arborium Bash and separately maps PowerShell.
- [`crates/syntax_tree/src/lib.rs (22-31, 63-99, 240-280)`](https://github.com/warpdotdev/warp/blob/e72fd7aacbbb2236d9b3be2aad7e7178fe94b4bc/crates/syntax_tree/src/lib.rs#L22-L31) caches editor trees, applies incremental edits, and refuses files above 2 MiB. Its buffer, highlighting, and asynchronous decoration ownership make `SyntaxTreeState` the wrong command-input API.

### What the grammar improves

A real shell grammar can identify the structure of:

- `$()` at arbitrary depth.
- Backtick command substitution.
- Command substitution inside double-quoted and concatenated words.
- Bash process substitution. The accepted Zsh compatibility subset includes the Bash forms.
- Pipelines, `&&`, `||`, and statement lists inside a substitution.
- Assignments and redirects inside a nested command.
- Heredocs, arrays, quoting, parameter expansion, and compound commands without corrupting adjacent command boundaries.

The grammar does not replace Warp's signature data or signature HIR. `classify_command` continues to assign meanings such as flags and positionals after the adapter produces a `LiteCommand`.

### Empirical grammar findings

The initial isolated probe used Arborium `2.18.1` and all four language features. The implementation follow-up tested the shipping three-feature configuration.

- Bash produced error-free hierarchical nodes for complete nested `$()`, process substitution, pipelines, `&&`, statement lists, assignment-plus-redirect, and heredoc inputs.
- Bash reduced incomplete `echo "pre$(pw` to an `ERROR` node without a nested `command`. The adapter must provide Warp recovery; the grammar alone does not preserve completion behavior.
- Fish produced the expected nested `command_substitution` hierarchy after appending a synthetic newline. Without a terminator, otherwise valid command buffers contain a missing `;`. The adapter must append and then clip a sentinel newline.
- The dedicated Zsh grammar marked every tested valid complete input as erroneous, including bare `ls`, newline-terminated `print "pre$(pwd)post"`, three-level nesting, and process substitution. The installed Zsh executable accepted the same inputs with `zsh -n`. Arborium `2.18.1` is the newest release, and its vendored Zsh `parser.c` has no external scanner. A fix requires upstream grammar source work that is outside this project.
- PowerShell produced correct nested hierarchy for complete expandable strings, three-level `$()` nesting, and pipelines plus statement lists. Incomplete strings still require adapter recovery.

The shipping configuration enables `lang-bash`, `lang-fish`, and `lang-powershell`. Zsh reuses `lang-bash`; `lang-zsh` is not enabled. Adding Fish to a Bash-plus-PowerShell baseline costs:

- Native x86-64 with release LTO and stripping: **68,328 bytes (66.7 KiB)** uncompressed and **13,887 bytes** gzipped.
- WASM with the `release-wasm` profile: **66,589 bytes (65.0 KiB)** uncompressed and **12,887 bytes** gzipped.

CORE-2284 does not record a technical cancellation reason. Its only comment reports that a PowerShell Arborium parser probe worked. Related PowerShell highlighting and underlining issues later reached Done. Treat the canceled ticket as prior exploration, not evidence that the grammar failed.

## Evidence-backed legacy failure corpus

These results came from executing the current public parser APIs. They are failures, not examples that already work.

1. **Nested command in an unquoted concatenated word**
   - Input: `echo pre$(pwd)post`, cursor on `pwd`.
   - Current: `command_at_cursor_position` returns `echo pre$(...)post`.
   - Required: cursor selection returns nested `pwd`. Completion selection at the end of this fully closed input remains on the outer command.
   - Consequence: Describe uses the outer command context.

2. **Open nested command in an unquoted concatenated word**
   - Input: `echo pre$(pw`, cursor at the end.
   - Current: cursor and completion selection return `echo pre$(...)`.
   - Required: both APIs return nested `pw` with an open `$()` group.
   - Consequence: completions and Describe use the outer command context.

3. **Nested command in a quoted concatenated word**
   - Input: `echo "pre$(pwd)post"`, cursor on `pwd`.
   - Current: cursor selection returns `echo pre$(...)post`.
   - Required: cursor selection returns `pwd`. Completion selection at the end remains on the outer command because the nested group is closed.
   - Consequence: Describe targets the wrong command.

4. **Nested depth inside adjacent text**
   - Input: `echo "pre$(a $(b))post"`, cursor on `b`.
   - Current: cursor selection returns `echo pre$(...)post`.
   - Required: cursor selection returns `b`.
   - Consequence: X-Ray cannot identify the deepest command.

5. **Process substitution**
   - Input: `cat <(printf x)`, cursor on `printf`.
   - Current: `all_parsed_commands` returns two top-level commands, `cat` and `printf x`, and reports a redirect. The parser has no process-substitution node.
   - Required: `cat` is top-level. `printf x` is a child in an input-process-substitution group. Permissions preserve conservative redirect handling until the safety migration explicitly changes it.
   - Consequence: structural consumers cannot distinguish a nested process from a separate top-level command.

6. **Redirect inside a nested command**
   - Input: `echo "$(KEY=VALUE env >out)"`, cursor on `env`.
   - Current: cursor selection returns parts equivalent to `KEY=VALUE env out`; the redirect destination is treated as a positional part.
   - Required: the nested command has assignment `KEY=VALUE`, executable `env`, and a separate `>out` redirection.
   - Consequence: Describe and signature classification see a false positional argument.

7. **Escaped nested backticks bypass a deny predicate**
   - Input: ``echo `echo \`rm -rf /\````. `bash -n` accepts this syntax, and a harmless equivalent executes the innermost substitution.
   - Current `decompose_command` returns:
     - `` `echo \`rm ``
     - `echo \`
     - `echo \`
     - `` `echo \`rm ``
     - `` /\`` ``
     - `` /\`` ``
     - the complete outer command
   - Current normalization leaves those fragments unchanged. The production-style anchored rule `rm(\s.*)?` matches none of them.
   - Required: decomposition includes `rm -rf /` as a complete nested command. The deny predicate matches.
   - Consequence: an explicit user or organization deny rule for `rm` is skipped. An `AlwaysAllow` execution profile can then allow the command. A separate risk decision can still stop it in other profiles, but it does not repair deny-rule semantics.

Known-good controls must remain separate from this failure list:

- `echo "$(pwd)"` already returns `pwd` at the cursor.
- `echo "$(pw` already selects `pw` for completion and cursor lookup.
- `echo "$(a $(b $(c` already selects `c` for completion and cursor lookup. The current `command_at_cursor` recursion handles this nested `OpenSubshell` shape.
- Plain `$()` nesting, `$()` inside backticks, and backticks inside `$()` already expose `rm -rf /` to the deny rule.
- `echo '$(pwd)'` correctly treats the substitution text as literal in POSIX shells.
- A pipeline and statement list inside a non-concatenated `$()` already decompose into the individual commands.

## Technical design

### Module and dependency boundary

Add `crates/warp_completer/src/parsers/shell/`. This module owns:

- Arborium grammar selection.
- `tree_sitter::Parser`, `Tree`, `Node`, and query details.
- Dialect-specific node mapping.
- EOF recovery.
- Conversion to Warp-owned types.

No public item in `warp_completer` may contain an Arborium or tree-sitter type. Add a compile-fail boundary test or API review test that imports only the public shell parser module without an Arborium dependency.

Pin Arborium and its language crates to one reviewed version, initially `=2.18.1`. Enable `lang-bash`, `lang-fish`, and `lang-powershell`. Map `ShellDialect::Zsh` to the Bash grammar. Do not enable `lang-zsh`. Fish uses its dedicated grammar.

Define `warp_completer::parsers::shell::ShellDialect` with `Bash`, `Zsh`, `Fish`, and `PowerShell`. Implement conversion from `warp_terminal::shell::ShellType` in `warp_terminal`, which already depends on `warp_completer`. Keep `ShellFamily` and `EscapeChar` for escaping paths and generated text. Do not use them to select a grammar.

### Warp-owned parse model

Expose these types. Exact field visibility may use constructors and accessors, but the represented data is required.

```rust
pub enum ShellDialect { Bash, Zsh, Fish, PowerShell }

pub struct ParsedShellInput {
    pub dialect: ShellDialect,
    pub source_len: usize,
    pub commands: Vec<ParsedCommand>,
    pub status: ShellParseStatus,
}

pub struct ParsedCommand {
    pub span: Span,
    pub parts: Vec<ParsedWord>,
    pub leading_assignments: Vec<Spanned<String>>,
    pub executable: Option<Spanned<String>>,
    pub post_whitespace: Option<Span>,
    pub nested_groups: Vec<NestedCommandGroup>,
    pub redirections: Vec<ShellRedirection>,
}

pub struct ParsedWord {
    pub span: Span,
    pub raw: String,
    pub completion_value: String,
}

pub struct NestedCommandGroup {
    pub span: Span,
    pub content_span: Span,
    pub kind: NestedCommandKind,
    pub closure: DelimiterState,
    pub commands: Vec<ParsedCommand>,
}

pub enum NestedCommandKind {
    DollarSubstitution,
    BacktickSubstitution,
    FishSubstitution,
    PowerShellSubexpression,
    InputProcessSubstitution,
    OutputProcessSubstitution,
}

pub enum DelimiterState { Closed, Open }

pub struct ShellRedirection {
    pub operator_span: Span,
    pub destination_span: Option<Span>,
    pub kind: ShellRedirectionKind,
}
pub enum ShellRedirectionKind {
    Input,
    Output,
    Append,
    HereDocument,
    HereString,
    FileDescriptor,
    ProcessSubstitution,
}

pub enum ShellParseStatus {
    Complete,
    Recovered { open_delimiters: Vec<OpenDelimiter> },
    Rejected(ShellParseRejection),
}

pub enum OpenDelimiter {
    SingleQuote,
    DoubleQuote,
    DollarParen,
    Backtick,
    FishParen,
    PowerShellSubexpression,
    ProcessSubstitution,
}

pub enum ShellParseRejection {
    InputTooLarge,
    GrammarUnavailable,
    UnsupportedDialectSyntax,
    Unrecoverable,
}

pub struct ShellParseOptions {
    pub quote_mode: QuoteMode,
}

pub enum QuoteMode {
    GroupQuotedText,
    PreserveQuotesAsLiterals,
}

pub struct PermissionDecomposition {
    pub commands: Vec<String>,
    pub contains_redirection: bool,
    pub status: ShellParseStatus,
}
```

All spans are zero-based UTF-8 byte ranges into the original input. Synthetic recovery bytes never appear in spans or returned strings. `ParsedCommand` nesting follows lexical containment. A substitution containing a pipeline or statement list has multiple child commands in source order.

`ParsedWord::raw` is the original source slice. `completion_value` preserves current unquoting and escape behavior and uses the current `$(...)` placeholder for a nested region. `to_lite_command` uses `completion_value` and `QuoteMode`, and it preserves leading assignments for the existing `classify_command` projection. Redirection operators and destinations are not positional parts.

Expose one parse entry point and Warp-shaped queries:

```rust
pub fn parse_shell_input(
    source: &str,
    dialect: ShellDialect,
    options: ShellParseOptions,
) -> ParsedShellInput;

impl ParsedShellInput {
    pub fn deepest_command_at(&self, cursor: ByteOffset) -> Option<&ParsedCommand>;
    pub fn completion_command_at(&self, cursor: ByteOffset) -> Option<&ParsedCommand>;
    pub fn top_level_commands(&self) -> impl Iterator<Item = &ParsedCommand>;
    pub fn commands_depth_first(&self) -> impl Iterator<Item = &ParsedCommand>;
    pub fn first_executable(&self) -> Option<&Spanned<String>>;
    pub fn decompose_for_permissions(&self, source: &str) -> PermissionDecomposition;
}

impl ParsedCommand {
    pub fn to_lite_command(&self) -> LiteCommand;
}
```

`parse_shell_input` is total. It does not panic on user input. A grammar load failure or input above 64 KiB returns `Rejected`. There is no final legacy-parser fallback. Consumers degrade as follows:

- Agent permissions deny automatic execution with an inconclusive reason.
- Completions, Describe, decorations, alias expansion, and package helpers return no parser-derived result.
- The input remains editable and executable after explicit user action.

During migration, retain compatibility functions with the current names. Add `ShellDialect` and cursor arguments where needed, implement them only as projections from `ParsedShellInput`, and move consumers to the richer methods. Keep `LiteCommand`, `classify_command`, and signature HIR independent of the parser backend.

### Incomplete-input recovery

Tree-sitter error recovery is input data, not the Warp completion contract. Implement deterministic adapter recovery:

1. Parse a virtual dialect terminator. Fish always receives a trailing sentinel newline. Other dialects receive one when their grammar requires it.
2. If an `ERROR` or missing node intersects EOF, infer only unclosed delimiters that began in the original input.
3. Append the minimum dialect-correct closing delimiters and parse a synthetic candidate.
4. Project nodes back to original spans. Clip the sentinel and synthetic closers.
5. Mark affected groups `DelimiterState::Open` and the input `Recovered`.
6. Select the deepest open group containing the cursor for completion.
7. Reject rather than guess when recovery would change non-EOF text or executable identity.

Recovery must cover incomplete quotes, `$()`, backticks, Fish `()`, PowerShell `$()`, process substitution, and arbitrary supported nesting depth. Single-quoted POSIX text does not create a nested command.

### Grammar viability gate

Before an enabled grammar or the accepted Zsh compatibility subset can enter shadow mode:

- Run its corpus through the real shell's syntax checker when available.
- Require every valid complete input in the supported corpus to produce stable executable spans and nested-command ownership.
- Permit grammar `ERROR` nodes only when the adapter proves a correct, dialect-specific projection in a golden test.
- Require incomplete inputs to produce the documented recovered hierarchy.

The dedicated Zsh grammar fails this gate and is not part of the shipping configuration. The requester accepted Bash grammar coverage for Zsh because the dedicated grammar is non-functional and cannot be repaired within this project's scope. The hand-written parser remains only as migration scaffolding until the three enabled grammars and the Zsh compatibility contract pass.

### Zsh-on-Bash compatibility contract

The Bash grammar supports the measured common subset of Zsh input. It does not implement the complete Zsh language. For Zsh, the absence of a tree-sitter error is **not** evidence that the hierarchy is correct. Phase 1 and every consumer migration phase must enforce this rule instead of gating only on `has_error()`.

Measured Zsh-only constructs have these outcomes through the Bash grammar:

- `=(...)` process substitution, both tested anonymous-function forms, short-form `for i (1 2 3)` loops, glob qualifiers such as `*.txt(.)`, parameter-expansion flags such as `${(f)...}`, `try`/`always` blocks, and `$+name` existence checks set `has_error = true`.
- `repeat 3 do; echo hi; done` is silently wrong. It parses without an error as three unrelated top-level commands: `repeat 3 do`, `echo hi`, and `done`.
- Named directories such as `~mydir` and `**` globs are not structural divergences. Both grammars leave expansion and glob interpretation to the shell.

Add a private, token-aware `ZshCompatibilityGuard` as part of the Phase 1 adapter:

1. Reject a Zsh parse with `UnsupportedDialectSyntax` when it has a non-EOF `ERROR` or missing node.
2. Permit existing EOF recovery only for syntax in the accepted Bash-compatible subset.
3. Reject command-position Zsh-only reserved words and forms that can parse cleanly under Bash. The initial detector must include `repeat`.
4. Expand the detector and its corpus for every newly identified silent divergence before that case can enter shadow mode.
5. Return no partial hierarchy on rejection. Consumers use the existing rejected-parse degradation behavior, and agent permissions fail closed.

The detector must distinguish command-position syntax from quoted text and ordinary argument text. It must not reject `echo repeat`. Zsh-specific conformance tests use real Zsh inputs; relabeling Bash fixtures as Zsh is insufficient.

[APP-5434](https://linear.app/warpdotdev/issue/APP-5434/zsh-constructs-that-parse-silently-wrong-under-the-bash-grammar) owns exhaustive coverage of silent divergences, including untested `select` and short-form `while` and `until` constructs. It must close before Phase 5 migrates agent permissions.

### Parse ownership, performance, and memory

Parse at most 64 KiB of command input. Reuse one parser per dialect per thread or active input. Keep at most one current tree and one previous tree per active input. Drop both when the input or session closes.

Start with full reparse because command inputs are small. Add a Warp-owned `ShellInputEdit` and private `InputEdit` conversion only if benchmarks show that incremental parsing is required. Do not expose `InputEdit`.

On the checked-in representative corpus:

- Adapter parse plus Warp-model projection must be p95 at most 1 ms.
- p99 must be at most 3 ms.
- Tree-sitter p95 must be no more than 25% slower than the legacy parser p95 before the legacy parser is deleted.
- Repeating 10,000 edits across malformed 64 KiB nested input must reach a stable memory plateau. Parser/tree caches must stay within their documented bounds.

Do not log input text, command names, or parse trees. Record only dialect, duration bucket, input-length bucket, parse status, recovery depth, and selected rollout backend.

### Semantic highlighting

Input decorations continue to use `SuggestionType` and signature classification. Tree-sitter provides structure and spans only. It does not choose input colors. The existing command-token error underline remains a consumer decision.

### Permissions contract

`PermissionDecomposition` contains:

- Every executable command at every nesting depth in source order.
- Recomposed command strings for each nested group and outer group when current deny semantics require them.
- A recursive `contains_redirection` value.
- `status`, so recovered or rejected parses can fail closed.

Permissions do not migrate until the new decomposition passes every legacy safety case and the new escaped-backtick case. Do not accept a mismatch because it is “more correct.” Review each mismatch as a safety policy change. Process substitution remains redirection-positive during this refactor.

## Phased migration

### Phase 0: corpus and grammar readiness

- Add parameterized legacy-observation tests for the failure corpus and known-good controls.
- Add adapter golden tests with the required hierarchy and spans.
- Pin Arborium, enable the three shipping grammar features, and add the four dialect mappings.
- Add executable Zsh divergence tests that record loud errors, the silent `repeat` misparse, and the measured non-divergences.
- Measure native and WASM artifact size on release settings.
- Land the escaped nested-backtick permissions regression test immediately. Do not wait for the parser migration to fix this pre-existing safety bug.

Exit condition: all three enabled grammars pass complete-input conformance, the Zsh divergence observations are executable tests, and the desired incomplete hierarchy is executable as adapter tests.

### Phase 1: adapter and shadow comparison

- Add the private tree-sitter mapper and public Warp model.
- Add `ZshCompatibilityGuard` and expose rejected Zsh syntax as `UnsupportedDialectSyntax`.
- Keep `simple/` as a temporary reference backend.
- Parse with both backends behind per-dialect rollout controls.
- Compare only normalized, non-sensitive facts: command count, executable spans, nesting spans, redirect presence, selected cursor command, and recovery status.
- Classify expected improvements separately from regressions.

Exit condition: no unexplained mismatch on the checked-in corpus and sampled non-sensitive fixtures. The Zsh guard rejects every measured unsupported form, including the clean `repeat` misparse, without rejecting the measured compatible controls.

### Phase 2: Describe and X-Ray

Move `command_at_cursor_position` and token-under-cursor behavior first. This consumer directly verifies nested ownership and does not execute commands or modify input.

Exit condition: nested and incomplete cursor tests pass for all four dialects.

### Phase 3: completions

Move completion selection, argument spans, trailing whitespace, leading-assignment handling, and incomplete quote/substitution behavior. Keep `classify_command` unchanged.

Exit condition: differential tests match legacy behavior except approved corpus fixes. Performance budgets pass on every supported native target.

### Phase 4: semantic and editing consumers

Move input decorations, alias and abbreviation expansion, top-level command lookup, next-command validation, Open-in-Warp, package installation, block metadata, and CLI-agent helpers. Decorations use `commands_depth_first` only where nested commands should be colored; top-level-only behavior uses `top_level_commands`.

Exit condition: semantic color and error-underline snapshots are unchanged except for approved span fixes. Alias expansion never targets a nested or non-executable word.

### Phase 5: agent permissions

Move `decompose_command` to `decompose_for_permissions`. Run denylist, allowlist, redirect, assignment, pipeline, nested-command, recovered-input, and over-limit tests. Roll out permissions per dialect after all other consumers are stable.

Exit condition: APP-5434 is closed. The permissions suite is fail-closed for every mismatch. Escaped nested backticks expose the complete inner command. Security review approves any intentional difference.

### Phase 6: delete the hand-written parser

- Remove `simple/lexer.rs`, `simple/parser.rs`, token types, converters, and legacy-only tests.
- Remove the legacy rollout path and comparison telemetry.
- Keep the Warp adapter, `LiteCommand`, signature HIR, `ShellFamily`, and `EscapeChar`.
- Keep the failure corpus as permanent adapter regression tests.

Done means every parser consumer uses the Warp adapter for Bash, Zsh, Fish, and PowerShell; the performance and WASM gates pass; and `crates/warp_completer/src/parsers/simple/` no longer exists.

## Decisions and trade-offs

- **Warp hierarchy versus flat `LiteCommand` only:** expose hierarchy. It fixes cursor ownership and supports nested consumers. `LiteCommand` remains a classification projection to avoid rewriting signature HIR.
- **Single cutover versus coexistence:** coexist per consumer during rollout. Delete the legacy backend only after all phases pass.
- **Dedicated Zsh grammar versus Bash grammar for Zsh:** use the Bash grammar with a Zsh compatibility guard. The dedicated Arborium `2.18.1` Zsh grammar fails valid input as small as `ls`, and repairing its unshipped sources is outside this project. This choice removes the larger Zsh grammar artifact cost, but it deliberately rejects detected Zsh-only syntax and requires guards for silent divergences.
- **Grammar recovery versus Warp recovery:** use both. Grammar nodes provide structure; the adapter owns EOF closure state and completion selection.
- **Incremental parse immediately versus benchmark first:** start with bounded parser reuse and full parse. Add private incremental edits only if the approved latency budget requires them.
- **Fallback above 64 KiB versus deletion:** reject parser-derived automation above the cap. Do not retain the hand-written parser as a hidden permanent fallback.
- **Grammar highlighting versus semantic highlighting:** preserve semantic highlighting. Grammar node kinds do not encode Warp signature meaning.
- **Permissions early versus last:** migrate permissions last. A parser improvement can still change an allow/deny decision, so safety parity has a higher bar than UX parity.

## Out of scope

- The natural-language tokenizer in `crates/input_classifier`.
- Replacing signature HIR, completion specs, or `classify_command`.
- Improving signature subcommands such as `git commit`.
- Changing input decoration colors.
- Parsing unsupported shells outside Bash, Zsh, Fish, and PowerShell.
- Reusing the file-editor `SyntaxTreeState` as the command-input state object.

## Testing and validation

### Unit and conformance tests

- `cargo test -p warp_completer parsers::shell::grammar_tests`
  - Runs the 24 Phase 0 grammar and Zsh divergence observations with no ignored cases.
- `cargo test -p warp_completer shell_adapter_complete_corpus`
  - Verifies exact spans, executables, nested groups, redirects, assignments, and top-level ownership for all four dialects.
- `cargo test -p warp_completer shell_adapter_incomplete_corpus`
  - Verifies open delimiters, clipped synthetic spans, and deepest cursor/completion selection.
- `cargo test -p warp_completer shell_adapter_legacy_failure_corpus`
  - Contains all seven empirical failures and asserts the required corrected result.
- `cargo test -p warp_completer shell_adapter_known_good_parity`
  - Protects the controls listed above from regression.
- `cargo test -p warp_completer shell_adapter_zsh_bash_compatibility`
  - Runs the measured Zsh-only corpus, asserts loud grammar failures reject, asserts `repeat` rejects despite a clean Bash parse, and protects accepted common syntax.
- `cargo test -p warp_completer shell_adapter_no_backend_types_in_public_api`
  - Verifies the public adapter surface is Warp-owned.
- `cargo test -p warp permissions_nested_shell_commands`
  - Verifies complete nested commands, escaped backticks, assignments, redirect policy, recovery rejection, and 64 KiB rejection against real predicates.

### Performance and memory

- Add `crates/warp_completer/benches/shell_parser.rs`.
- Run `cargo bench -p warp_completer --bench shell_parser`.
- Store machine metadata and p50, p95, p99 for legacy and adapter projection. The benchmark fails its comparison script when the approved limits are exceeded.
- Run the malformed-input stress test under the repository heap profiler. Attach the allocation graph and final plateau to the implementation PR.

### Build and artifact validation

- Run the repository native presubmit for Linux, macOS, and Windows.
- Run `cargo check -p warp_completer --target wasm32-unknown-unknown`.
- Build matched release artifacts before and after `lang-fish`. Report uncompressed and compressed native and WASM deltas in the implementation PR.
- Confirm `cargo tree` resolves one Arborium version and matching grammar-crate versions.

### Rollout validation

- Each phase lands independently behind per-consumer and per-dialect controls.
- A phase records aggregate mismatch and rejection rates without command text.
- Disable only the affected consumer and dialect when a regression appears.
- Do not delete `simple/` until all Phase 6 exit conditions pass.

## Parallelization

Parallel work is useful after Phase 0 establishes the shared model.

- **adapter-core** — Local worktree `../warp-app5430-adapter`, branch `factory/app5430-adapter-core`. Owns `warp_completer` types, Bash/Fish/PowerShell mapping, recovery, corpus tests, and benchmarks.
- **zsh-compatibility** — Remote environment with the Warp repository, branch `factory/app5430-zsh-compatibility`. Owns the Zsh compatibility detector, Zsh-only conformance corpus, and failure-mode tests. Returns a pushed branch and probe results.
- **consumer-migration** — Local worktree `../warp-app5430-consumers`, branch `factory/app5430-consumers`. Starts only after adapter-core API stabilization. Owns Describe, completions, decorations, aliases, and helpers.
- **permissions-migration** — Local worktree `../warp-app5430-permissions`, branch `factory/app5430-permissions`. Starts after adapter-core and consumer-migration. Owns permission decomposition and safety tests.
- **cross-platform-validation** — Remote native and WASM runners. Starts after integration. Owns platform builds, latency comparison, heap evidence, and artifact-size results.

The lead integrates in that order into one implementation PR. File ownership stays disjoint until the adapter API is stable. Permissions remains the final code merge before deletion.
