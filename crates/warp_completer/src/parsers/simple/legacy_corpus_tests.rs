//! Legacy-observation tests for the APP-5430 evidence corpus.
//!
//! `specs/APP-5430/TECH.md` documents cases where the hand-written parser in this module
//! (`crates/warp_completer/src/parsers/simple/`) mishandles nested shell commands, plus a set of
//! known-good controls that must not regress. These tests pin down what the *current* (legacy)
//! parser actually returns for each corpus entry, so that the Phase 1 tree-sitter adapter has a
//! precise baseline to diff against and improve on. They are deliberately named
//! `observed_legacy_*` rather than asserting the "required" behavior from the spec: for the
//! cases that remain open, the assertions below capture today's incorrect output on purpose. The
//! escaped nested-backtick permissions bypass (APP-5433) has been fixed now, per the spec's
//! explicit direction not to wait on the parser migration for that one.
//!
//! One entry from the original evidence corpus ("deep incomplete input", `echo "$(a $(b $(c`) was
//! found not to reproduce against the exact commit the spec researched: `command_at_cursor_position`
//! already recurses correctly to the innermost open group for this input. It is covered below as
//! a known-good control rather than a failure case; the spec is being corrected to match.

use string_offset::ByteOffset;
use warp_util::path::EscapeChar;

use crate::parsers::simple::{
    all_parsed_commands, command_at_cursor_position, decompose_command, parse_for_completions,
};

fn cursor(source: &str, pos: usize) -> Option<String> {
    command_at_cursor_position(source, EscapeChar::Backslash, ByteOffset::from(pos))
        .map(|c| c.joined_by_space())
}

fn completion(source: &str) -> Option<String> {
    parse_for_completions(source, EscapeChar::Backslash, false).map(|c| c.joined_by_space())
}

/// Nested command in an unquoted concatenated word.
///
/// Required (Phase 1+): cursor selection returns the nested `pwd` command.
#[test]
fn observed_legacy_nested_command_in_unquoted_concatenated_word() {
    let source = "echo pre$(pwd)post";
    // Cursor lands inside "pwd".
    assert_eq!(cursor(source, 11), Some("echo pre$(...)post".to_string()));
}

/// Open nested command in an unquoted concatenated word.
///
/// Required (Phase 1+): both cursor and completion selection return the nested, still-open `pw`.
#[test]
fn observed_legacy_open_nested_command_in_unquoted_concatenated_word() {
    let source = "echo pre$(pw";
    assert_eq!(
        cursor(source, source.len()),
        Some("echo pre$(...)".to_string())
    );
    assert_eq!(completion(source), Some("echo pre$(...)".to_string()));
}

/// Nested command in a quoted concatenated word.
///
/// Required (Phase 1+): cursor selection returns `pwd`.
#[test]
fn observed_legacy_nested_command_in_quoted_concatenated_word() {
    let source = "echo \"pre$(pwd)post\"";
    // Cursor lands inside "pwd".
    assert_eq!(cursor(source, 12), Some("echo pre$(...)post".to_string()));
}

/// Nested depth inside adjacent text.
///
/// Required (Phase 1+): cursor selection returns the deepest nested command `b`.
#[test]
fn observed_legacy_nested_depth_inside_adjacent_text() {
    let source = "echo \"pre$(a $(b))post\"";
    // Cursor lands on "b".
    assert_eq!(cursor(source, 15), Some("echo pre$(...)post".to_string()));
}

/// Process substitution.
///
/// Required (Phase 1+): `cat` is top-level and `printf x` is a child in an input-process-
/// substitution group, rather than two independent top-level commands.
#[test]
fn observed_legacy_process_substitution() {
    let source = "cat <(printf x)";
    let all: Vec<String> = all_parsed_commands(source, EscapeChar::Backslash)
        .map(|c| c.joined_by_space())
        .collect();
    assert_eq!(all, vec!["cat".to_string(), "printf x".to_string()]);
}

/// Redirect inside a nested command.
///
/// Required (Phase 1+): the nested command has assignment `KEY=VALUE`, executable `env`, and a
/// separate `>out` redirection, rather than folding the redirect destination into a positional.
#[test]
fn observed_legacy_redirect_inside_nested_command() {
    let source = "echo \"$(KEY=VALUE env >out)\"";
    // Cursor lands inside "env".
    assert_eq!(cursor(source, 15), Some("KEY=VALUE env out".to_string()));
}

/// Escaped nested backticks bypass a deny predicate (APP-5433).
///
/// This is a confirmed pre-existing safety bug and, per the spec, was fixed immediately rather
/// than deferred to the parser migration (see `until_backtick` in `iter.rs`). Decomposition must
/// include `rm -rf /` as its own command so an anchored `rm(\s.*)?` deny rule matches it.
#[test]
fn observed_fixed_escaped_nested_backticks_expose_inner_command() {
    let source = "echo `echo \\`rm -rf /\\``";
    let (decomposed, contains_redirection) = decompose_command(source, EscapeChar::Backslash);
    assert!(
        decomposed.contains(&"rm -rf /".to_string()),
        "expected decomposition to include the innermost `rm -rf /` command, got {decomposed:?}"
    );
    assert!(!contains_redirection);
}

/// Known-good control: `echo "$(pwd)"` already returns `pwd` at the cursor.
#[test]
fn known_good_simple_quoted_nested_command_cursor() {
    let source = "echo \"$(pwd)\"";
    // Cursor lands inside "pwd".
    assert_eq!(cursor(source, 9), Some("pwd".to_string()));
}

/// Known-good control: `echo "$(pw` already selects `pw` for completion and cursor lookup.
#[test]
fn known_good_simple_quoted_open_nested_command() {
    let source = "echo \"$(pw";
    assert_eq!(cursor(source, source.len()), Some("pw".to_string()));
    assert_eq!(completion(source), Some("pw".to_string()));
}

/// Known-good control: plain `$()` nesting, `$()` inside backticks, and backticks inside `$()`
/// already expose `rm -rf /` to the deny rule.
#[test]
fn known_good_unescaped_nesting_exposes_inner_command() {
    for source in ["echo $(echo `rm -rf /`)", "echo `echo $(rm -rf /)`"] {
        let (decomposed, _) = decompose_command(source, EscapeChar::Backslash);
        assert!(
            decomposed.contains(&"rm -rf /".to_string()),
            "expected {source:?} to decompose to include `rm -rf /`, got {decomposed:?}"
        );
    }
}

/// Known-good control: `echo '$(pwd)'` correctly treats the substitution text as literal in
/// POSIX shells.
#[test]
fn known_good_single_quoted_substitution_is_literal() {
    let source = "echo '$(pwd)'";
    let (decomposed, _) = decompose_command(source, EscapeChar::Backslash);
    assert_eq!(decomposed, vec![source.to_string()]);
}

/// Known-good control: deep incomplete input already recurses to the innermost open group.
///
/// The original evidence corpus described `command_at_cursor_position` as returning the outer
/// `echo $(...)` for this input while `parse_for_completions` correctly returns `c`. That
/// divergence does not reproduce against the exact commit the spec researched
/// (`e72fd7aacbbb2236d9b3be2aad7e7178fe94b4bc`, which is also current master): with the cursor at
/// the end of the input, `command_at_cursor_position` already recurses all the way to the
/// innermost open group and agrees with `parse_for_completions`. The spec is being corrected to
/// list this as a known-good control instead of a failure case.
#[test]
fn known_good_deep_incomplete_input_recurses_to_innermost_group() {
    let source = "echo \"$(a $(b $(c";
    assert_eq!(cursor(source, source.len()), Some("c".to_string()));
    assert_eq!(completion(source), Some("c".to_string()));
}

/// Known-good control: a pipeline and statement list inside a non-concatenated `$()` already
/// decompose into the individual commands.
#[test]
fn known_good_pipeline_inside_substitution_decomposes() {
    use std::collections::HashSet;

    let source = "ls $(foo | echo)";
    let (decomposed, _) = decompose_command(source, EscapeChar::Backslash);
    assert_eq!(
        HashSet::<String>::from_iter(decomposed),
        HashSet::from_iter(
            ["foo", "echo", "foo | echo", "ls $(foo | echo)"]
                .into_iter()
                .map(ToString::to_string)
        ),
    );
}
