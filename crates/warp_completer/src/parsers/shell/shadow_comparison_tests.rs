//! Phase 1 shadow comparison (`specs/APP-5430/TECH.md`): runs both the legacy `simple::` parser
//! and the new tree-sitter adapter over the checked-in corpus and compares normalized,
//! non-sensitive facts (command count, executable identity, redirect presence). Every mismatch
//! below is classified explicitly as either an expected improvement (the corpus items the
//! adapter was built to fix) or a required exact match (the known-good controls); an assertion
//! failure here means an *unexplained* mismatch, which is the Phase 1 exit condition.
//!
//! This intentionally does not implement the production per-dialect rollout-flag plumbing the
//! spec describes for a live shadow rollout -- no consumer migrates in Phase 1, so there is
//! nothing yet to roll out to. It implements the comparison itself as an executable test suite.

use string_offset::ByteOffset;
use warp_util::path::EscapeChar;

use super::{ShellDialect, ShellParseOptions, parse_shell_input};
use crate::parsers::simple;

fn legacy_cursor(source: &str, pos: usize) -> Option<String> {
    simple::command_at_cursor_position(source, EscapeChar::Backslash, ByteOffset::from(pos))
        .map(|c| c.joined_by_space())
}

fn adapter_cursor(source: &str, pos: usize) -> Option<String> {
    parse_shell_input(source, ShellDialect::Bash, ShellParseOptions::default())
        .deepest_command_at(ByteOffset::from(pos))
        .and_then(|c| c.executable.as_ref())
        .map(|e| e.item.clone())
}

fn legacy_decompose(source: &str) -> std::collections::HashSet<String> {
    simple::decompose_command(source, EscapeChar::Backslash)
        .0
        .into_iter()
        .collect()
}

fn adapter_decompose(source: &str) -> std::collections::HashSet<String> {
    parse_shell_input(source, ShellDialect::Bash, ShellParseOptions::default())
        .decompose_for_permissions(source)
        .commands
        .into_iter()
        .collect()
}

/// Corpus items the adapter was specifically built to fix: the legacy cursor selection stays on
/// the outer command (or otherwise gets the wrong answer), while the adapter must select the
/// nested command directly. A mismatch here is expected and required, not a regression.
#[test]
fn expected_improvement_cursor_selection_on_legacy_failure_corpus() {
    let cases: &[(&str, usize, &str)] = &[
        ("echo pre$(pwd)post", 11, "pwd"),
        ("echo \"pre$(pwd)post\"", 12, "pwd"),
        ("echo \"pre$(a $(b))post\"", 15, "b"),
        ("cat <(printf x)", 9, "printf"),
    ];
    for (source, pos, adapter_expected) in cases {
        let legacy = legacy_cursor(source, *pos);
        let adapter = adapter_cursor(source, *pos);
        assert_ne!(
            legacy.as_deref(),
            Some(*adapter_expected),
            "{source:?}: legacy was expected to still get this wrong (documenting the improvement); \
             if it now matches the adapter, the legacy-corpus classification is stale"
        );
        assert_eq!(
            adapter.as_deref(),
            Some(*adapter_expected),
            "{source:?}: adapter cursor selection regressed"
        );
    }
}

/// The escaped nested-backtick case (APP-5433): the legacy decomposition (even after the Phase 0
/// `until_backtick` fix) and the adapter's decomposition should agree that `rm -rf /` is exposed
/// -- this is a safety-critical case where both backends must independently reach the same
/// correct, deny-rule-visible answer.
#[test]
fn escaped_nested_backticks_agree_between_backends() {
    let source = "echo `echo \\`rm -rf /\\``";
    let legacy = legacy_decompose(source);
    let adapter = adapter_decompose(source);
    assert!(
        legacy.contains("rm -rf /"),
        "legacy regressed on APP-5433: {legacy:?}"
    );
    assert!(
        adapter.contains("rm -rf /"),
        "adapter missed APP-5433: {adapter:?}"
    );
}

/// Known-good controls: both backends must select the same nested command, since the legacy
/// parser already gets these right. A mismatch here is a real regression, not an expected
/// improvement.
#[test]
fn known_good_cursor_selection_matches_between_backends() {
    let cases: &[(&str, usize)] = &[("echo \"$(pwd)\"", 9), ("echo \"$(a $(b $(c", 17)];
    for (source, pos) in cases {
        let legacy = legacy_cursor(source, *pos);
        let adapter = adapter_cursor(source, *pos);
        assert_eq!(
            legacy, adapter,
            "{source:?}: expected both backends to agree on this known-good control"
        );
    }
}

/// Known-good decomposition controls: unescaped nesting already exposes the inner command via
/// both backends, and single-quoted text stays literal via both backends.
#[test]
fn known_good_decomposition_matches_between_backends() {
    for source in ["echo $(echo `rm -rf /`)", "echo `echo $(rm -rf /)`"] {
        let legacy = legacy_decompose(source);
        let adapter = adapter_decompose(source);
        assert!(
            legacy.contains("rm -rf /"),
            "{source:?}: legacy regressed: {legacy:?}"
        );
        assert!(
            adapter.contains("rm -rf /"),
            "{source:?}: adapter regressed: {adapter:?}"
        );
    }

    let literal = "echo '$(pwd)'";
    assert_eq!(
        legacy_decompose(literal).len(),
        1,
        "legacy should treat this as one literal command"
    );
    assert_eq!(
        adapter_decompose(literal).len(),
        1,
        "adapter should treat this as one literal command"
    );
}

/// Redirect presence must agree between backends for a plain (non-nested) redirect.
#[test]
fn redirect_presence_matches_between_backends() {
    for (source, expected) in [("ls > file.txt", true), ("ls -la", false)] {
        let (_, legacy_redirect) = simple::decompose_command(source, EscapeChar::Backslash);
        let adapter_redirect =
            parse_shell_input(source, ShellDialect::Bash, ShellParseOptions::default())
                .decompose_for_permissions(source)
                .contains_redirection;
        assert_eq!(
            legacy_redirect, expected,
            "{source:?}: legacy redirect detection"
        );
        assert_eq!(
            adapter_redirect, expected,
            "{source:?}: adapter redirect detection"
        );
    }
}
