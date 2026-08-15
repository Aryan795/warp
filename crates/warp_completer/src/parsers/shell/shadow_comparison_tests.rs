//! Phase 1 shadow comparison (`specs/APP-5430/TECH.md`): runs both the legacy `simple::` parser
//! and the new tree-sitter adapter over the checked-in corpus and compares a canonical normalized
//! snapshot of each backend's output. The Phase 1 exit condition is "no unexplained mismatch": a
//! mismatch is fine only when it is captured explicitly as an `ApprovedDelta` in the relevant
//! test's ledger, so that a regression anywhere else fails loudly rather than being silently
//! tolerated by a spot-check.
//!
//! `simple::`'s internal `Command`/`Part` tree is private even to sibling modules in this crate,
//! so the snapshot is built entirely from each backend's public API. Rather than comparing only a
//! few hand-picked cursor positions (the failure mode a prior version of this file had -- it could
//! approve a regression in every fact it didn't happen to probe), `legacy_snapshot`/
//! `adapter_snapshot` probe *every* byte offset in the input via
//! `command_at_cursor_position`/`completion_command_at`, recovering the
//! same load-bearing facts a direct tree comparison would (command count, span, and recomposed
//! text at every nesting depth transition), plus top-level command spans and redirect presence.
//!
//! This intentionally does not implement the production per-dialect rollout-flag plumbing the
//! spec describes for a live shadow rollout -- no consumer migrates in Phase 1, so there is
//! nothing yet to roll out to. It implements the comparison itself as an executable test suite.

use string_offset::ByteOffset;
use warp_util::path::EscapeChar;

use super::{ShellDialect, ShellParseOptions, parse_shell_input};
use crate::parsers::simple;

/// One canonical, backend-agnostic snapshot of what parsing (and cursor-probing) `source`
/// produces.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Snapshot {
    top_level_spans: Vec<(usize, usize)>,
    top_level_texts: Vec<String>,
    /// `cursor_selection[i]` is the `(span, recomposed text)` of the command selected with the
    /// cursor at byte offset `i`, for every `i` in `0..=source.len()`.
    cursor_selection: Vec<Option<((usize, usize), String)>>,
    contains_redirection: bool,
}

fn legacy_snapshot(source: &str) -> Snapshot {
    use crate::meta::HasSpan;

    let top_level: Vec<_> = simple::all_parsed_commands(source, EscapeChar::Backslash).collect();
    let top_level_spans = top_level
        .iter()
        .map(|c| (c.span().start(), c.span().end()))
        .collect();
    let top_level_texts = top_level.iter().map(|c| c.joined_by_space()).collect();
    let cursor_selection = (0..=source.len())
        .map(|pos| {
            simple::command_at_cursor_position(source, EscapeChar::Backslash, ByteOffset::from(pos))
                .map(|c| ((c.span().start(), c.span().end()), c.joined_by_space()))
        })
        .collect();
    let (_, contains_redirection) = simple::decompose_command(source, EscapeChar::Backslash);
    Snapshot {
        top_level_spans,
        top_level_texts,
        cursor_selection,
        contains_redirection,
    }
}

fn adapter_snapshot(source: &str) -> Snapshot {
    use crate::meta::HasSpan;

    let result = parse_shell_input(source, ShellDialect::Bash, ShellParseOptions::default());
    let top_level_spans = result
        .top_level_commands()
        .map(|c| (c.span.start(), c.span.end()))
        .collect();
    let top_level_texts = result
        .top_level_commands()
        .map(|c| c.to_lite_command().joined_by_space())
        .collect();
    let cursor_selection = (0..=source.len())
        .map(|pos| {
            result
                .completion_command_at(ByteOffset::from(pos))
                .map(|c| {
                    let lite = c.to_lite_command();
                    (
                        (lite.span().start(), lite.span().end()),
                        lite.joined_by_space(),
                    )
                })
        })
        .collect();
    Snapshot {
        top_level_spans,
        top_level_texts,
        cursor_selection,
        contains_redirection: result
            .decompose_for_permissions(source)
            .contains_redirection,
    }
}

/// One documented, approved divergence between backends at a specific cursor offset: the legacy
/// value it replaces, and the adapter value it must produce instead.
struct ApprovedDelta {
    cursor: usize,
    legacy_selection: Option<(&'static str, &'static str)>,
    adapter_selection: Option<(&'static str, &'static str)>,
}

/// Asserts that `legacy` and `adapter` agree on every top-level fact (command count, spans,
/// texts, redirect presence) and on cursor selection everywhere except the byte offsets listed in
/// `deltas`, at which the cursor-selection values must match exactly what the ledger says they
/// must be. A cursor offset that differs between backends but is *not* in `deltas` fails the test
/// -- this is what makes an unlisted regression unable to pass.
///
/// Only usable when the two backends agree on top-level facts; a handful of inputs where they
/// legitimately do not (e.g. legacy folding a redirect's destination into the command's own
/// parts) get their own dedicated, fully-asserted test instead of using this helper -- see
/// `plain_redirect_destination_diverges_from_legacys_lossy_interpretation` and
/// `process_substitution_diverges_in_top_level_command_count_too`.
fn assert_snapshot_matches_ledger(
    source: &str,
    legacy: &Snapshot,
    adapter: &Snapshot,
    deltas: &[ApprovedDelta],
) {
    assert_eq!(
        legacy.top_level_spans, adapter.top_level_spans,
        "{source:?}: top-level command spans diverged"
    );
    assert_eq!(
        legacy.top_level_texts, adapter.top_level_texts,
        "{source:?}: top-level command texts diverged"
    );
    assert_eq!(
        legacy.contains_redirection, adapter.contains_redirection,
        "{source:?}: redirect presence diverged"
    );
    assert_cursor_sweep_matches_ledger(source, legacy, adapter, deltas);
}

/// Asserts cursor-selection agreement only (see `assert_snapshot_matches_ledger` for the combined
/// check, which most tests should use instead of calling this directly).
fn assert_cursor_sweep_matches_ledger(
    source: &str,
    legacy: &Snapshot,
    adapter: &Snapshot,
    deltas: &[ApprovedDelta],
) {
    let delta_by_cursor: std::collections::HashMap<usize, &ApprovedDelta> =
        deltas.iter().map(|d| (d.cursor, d)).collect();

    for cursor in 0..=source.len() {
        let legacy_value = &legacy.cursor_selection[cursor];
        let adapter_value = &adapter.cursor_selection[cursor];
        match delta_by_cursor.get(&cursor) {
            None => {
                assert_eq!(
                    legacy_value, adapter_value,
                    "{source:?} at cursor {cursor}: unexplained mismatch between backends \
                     (add an ApprovedDelta if this is an intentional Phase 1 improvement)"
                );
            }
            Some(delta) => {
                let expected_legacy = delta
                    .legacy_selection
                    .map(|(span_text, text)| (parse_span(span_text), text.to_string()));
                let expected_adapter = delta
                    .adapter_selection
                    .map(|(span_text, text)| (parse_span(span_text), text.to_string()));
                assert_eq!(
                    legacy_value, &expected_legacy,
                    "{source:?} at cursor {cursor}: ledger's recorded legacy value is stale"
                );
                assert_eq!(
                    adapter_value, &expected_adapter,
                    "{source:?} at cursor {cursor}: adapter no longer matches the ledger's approved value"
                );
            }
        }
    }
}

fn parse_span(text: &str) -> (usize, usize) {
    let (start, end) = text
        .split_once("..")
        .expect("span text must be \"start..end\"");
    (start.parse().unwrap(), end.parse().unwrap())
}

/// Known-good controls: the full cursor sweep, top-level spans/texts, and redirect presence must
/// match exactly between backends, with zero approved deltas.
#[test]
fn known_good_controls_match_exactly() {
    for source in [
        "echo \"$(pwd)\"",
        "echo \"$(pw",
        "echo \"$(a $(b $(c",
        "echo $(echo `rm -rf /`)",
        "echo `echo $(rm -rf /)`",
        "echo '$(pwd)'",
        "ls $(foo | echo)",
        "ls -la",
    ] {
        let legacy = legacy_snapshot(source);
        let adapter = adapter_snapshot(source);
        assert_snapshot_matches_ledger(source, &legacy, &adapter, &[]);
    }
}

/// Legacy failure corpus: every cursor-selection divergence across the *full* sweep (not a
/// hand-picked position) is captured in an explicit, per-input ledger, derived by diffing the two
/// backends' snapshots at every byte offset and confirming each one. Any divergence not listed
/// here fails the test -- this is what makes a regression at an untested position unable to pass.
#[test]
fn legacy_failure_corpus_deltas_are_fully_ledgered() {
    let legacy_pre_span = ("0..18", "echo pre$(...)post");
    let cases: &[(&str, &[ApprovedDelta])] = &[
        (
            "echo pre$(pwd)post",
            &[
                ApprovedDelta {
                    cursor: 10,
                    legacy_selection: Some(legacy_pre_span),
                    adapter_selection: Some(("10..13", "pwd")),
                },
                ApprovedDelta {
                    cursor: 11,
                    legacy_selection: Some(legacy_pre_span),
                    adapter_selection: Some(("10..13", "pwd")),
                },
                ApprovedDelta {
                    cursor: 12,
                    legacy_selection: Some(legacy_pre_span),
                    adapter_selection: Some(("10..13", "pwd")),
                },
                ApprovedDelta {
                    cursor: 13,
                    legacy_selection: Some(legacy_pre_span),
                    adapter_selection: Some(("10..13", "pwd")),
                },
            ],
        ),
        (
            "echo pre$(pw",
            &[
                ApprovedDelta {
                    cursor: 10,
                    legacy_selection: Some(("0..12", "echo pre$(...)")),
                    adapter_selection: Some(("10..12", "pw")),
                },
                ApprovedDelta {
                    cursor: 11,
                    legacy_selection: Some(("0..12", "echo pre$(...)")),
                    adapter_selection: Some(("10..12", "pw")),
                },
                ApprovedDelta {
                    cursor: 12,
                    legacy_selection: Some(("0..12", "echo pre$(...)")),
                    adapter_selection: Some(("10..12", "pw")),
                },
            ],
        ),
        (
            "echo \"pre$(pwd)post\"",
            &[
                ApprovedDelta {
                    cursor: 11,
                    legacy_selection: Some(("0..20", "echo pre$(...)post")),
                    adapter_selection: Some(("11..14", "pwd")),
                },
                ApprovedDelta {
                    cursor: 12,
                    legacy_selection: Some(("0..20", "echo pre$(...)post")),
                    adapter_selection: Some(("11..14", "pwd")),
                },
                ApprovedDelta {
                    cursor: 13,
                    legacy_selection: Some(("0..20", "echo pre$(...)post")),
                    adapter_selection: Some(("11..14", "pwd")),
                },
                ApprovedDelta {
                    cursor: 14,
                    legacy_selection: Some(("0..20", "echo pre$(...)post")),
                    adapter_selection: Some(("11..14", "pwd")),
                },
            ],
        ),
        ("echo \"pre$(a $(b))post\"", &{
            let legacy = ("0..23", "echo pre$(...)post");
            [
                ApprovedDelta {
                    cursor: 11,
                    legacy_selection: Some(legacy),
                    adapter_selection: Some(("11..17", "a $(...)")),
                },
                ApprovedDelta {
                    cursor: 12,
                    legacy_selection: Some(legacy),
                    adapter_selection: Some(("11..17", "a $(...)")),
                },
                ApprovedDelta {
                    cursor: 13,
                    legacy_selection: Some(legacy),
                    adapter_selection: Some(("11..17", "a $(...)")),
                },
                ApprovedDelta {
                    cursor: 14,
                    legacy_selection: Some(legacy),
                    adapter_selection: Some(("11..17", "a $(...)")),
                },
                ApprovedDelta {
                    cursor: 15,
                    legacy_selection: Some(legacy),
                    adapter_selection: Some(("15..16", "b")),
                },
                ApprovedDelta {
                    cursor: 16,
                    legacy_selection: Some(legacy),
                    adapter_selection: Some(("15..16", "b")),
                },
                // At cursor 17 (the byte right after `b`'s closing `)`), the inner `$(b)`
                // group is no longer open at this position, so selection falls back one
                // level to the `a $(...)` group -- correct, not a bug.
                ApprovedDelta {
                    cursor: 17,
                    legacy_selection: Some(legacy),
                    adapter_selection: Some(("11..17", "a $(...)")),
                },
            ]
        }),
    ];
    for (source, deltas) in cases {
        let legacy = legacy_snapshot(source);
        let adapter = adapter_snapshot(source);
        assert_snapshot_matches_ledger(source, &legacy, &adapter, deltas);
    }
}

/// Legacy folds a plain redirect's *destination* into the command's own `parts` (it consumes and
/// discards only the `>`/`<` operator token itself, then keeps parsing normally, so `file.txt`
/// ends up looking like a positional argument) rather than modeling it as a distinct redirect
/// clause. This makes legacy's own top-level span/text *wrong* in a way the adapter deliberately
/// does not reproduce: the adapter keeps `ls` as the command and represents `> file.txt`
/// separately via `ShellRedirection`, matching real Bash semantics (the destination is not one of
/// `ls`'s arguments). Top-level facts are asserted explicitly here rather than via
/// `assert_snapshot_matches_ledger`, since that helper requires top-level agreement.
#[test]
fn plain_redirect_destination_diverges_from_legacys_lossy_interpretation() {
    let source = "ls > file.txt";
    let legacy = legacy_snapshot(source);
    let adapter = adapter_snapshot(source);

    assert_eq!(legacy.top_level_spans, vec![(0, 13)]);
    assert_eq!(legacy.top_level_texts, vec!["ls file.txt".to_string()]);
    assert_eq!(adapter.top_level_spans, vec![(0, 3)]);
    assert_eq!(adapter.top_level_texts, vec!["ls".to_string()]);
    // Both backends agree a redirect is present, just via different reasoning (legacy sees the
    // `>` token directly; the adapter sees a real `ShellRedirection` on the command).
    assert!(legacy.contains_redirection);
    assert!(adapter.contains_redirection);
}

/// Legacy does not model `<(...)` process substitution at all: it treats `<` as an input-redirect
/// token (incorrectly setting `contains_redirection`) and then parses the following `(printf x)`
/// as an entirely separate, sibling top-level command via its generic parenthesized-group
/// handling -- producing *two* top-level commands where there is actually one. The adapter
/// correctly keeps one top-level `cat` command with `printf x` as a child of an
/// `InputProcessSubstitution` nested group, and does not report a redirect (a process
/// substitution is not one). Top-level facts are asserted explicitly here rather than via
/// `assert_snapshot_matches_ledger`, since that helper requires top-level agreement.
#[test]
fn process_substitution_diverges_in_top_level_command_count_too() {
    let source = "cat <(printf x)";
    let legacy = legacy_snapshot(source);
    let adapter = adapter_snapshot(source);

    assert_eq!(legacy.top_level_spans, vec![(0, 5), (6, 14)]);
    assert_eq!(
        legacy.top_level_texts,
        vec!["cat".to_string(), "printf x".to_string()]
    );
    assert!(
        legacy.contains_redirection,
        "legacy incorrectly treats `<(` as a redirect"
    );

    assert_eq!(adapter.top_level_spans, vec![(0, 15)]);
    assert_eq!(adapter.top_level_texts, vec!["cat $(...)".to_string()]);
    assert!(
        !adapter.contains_redirection,
        "a process substitution is not a redirect"
    );

    // Cursor selection: legacy stays on the bare `cat` command (span 0..5) at every position up
    // to and including its own end, and returns `None` past it (there is nothing registered at
    // that position in legacy's own model); the adapter recurses correctly while inside `cat`'s
    // span and falls back to the full command past EOF instead of returning `None`.
    let deltas: Vec<ApprovedDelta> = (0..=5)
        .map(|cursor| ApprovedDelta {
            cursor,
            legacy_selection: Some(("0..5", "cat")),
            adapter_selection: Some(("0..15", "cat $(...)")),
        })
        .chain(std::iter::once(ApprovedDelta {
            cursor: 15,
            legacy_selection: None,
            adapter_selection: Some(("0..15", "cat $(...)")),
        }))
        .collect();
    assert_cursor_sweep_matches_ledger(source, &legacy, &adapter, &deltas);
}

/// APP-5433: safety-critical, so both backends are required to independently expose `rm -rf /`,
/// not merely "not regress" relative to each other. Cursor-selection is not compared here (the
/// grammar-level hierarchy differs by construction between the two backends for this input), but
/// `decompose_for_permissions`/`decompose_command` -- the actual deny-rule-facing API -- must
/// agree on the safety-relevant fact.
#[test]
fn escaped_nested_backticks_app_5433_both_backends_expose_the_inner_command() {
    let source = "echo `echo \\`rm -rf /\\``";
    let (legacy_commands, _) = simple::decompose_command(source, EscapeChar::Backslash);
    let adapter_commands =
        parse_shell_input(source, ShellDialect::Bash, ShellParseOptions::default())
            .decompose_for_permissions(source)
            .commands;
    assert!(
        legacy_commands.contains(&"rm -rf /".to_string()),
        "legacy regressed on APP-5433: {legacy_commands:?}"
    );
    assert!(
        adapter_commands.contains(&"rm -rf /".to_string()),
        "adapter missed APP-5433: {adapter_commands:?}"
    );
}
