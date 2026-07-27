use std::io::Write as _;
use std::sync::Arc;

use ai::diff_validation::{AIRequestedCodeDiff, DiffDelta, DiffMatchFailures, ParsedDiff, V4AHunk};
use async_io::block_on;
use tempfile::NamedTempFile;
use vec1::vec1;
use warpui::App;

use super::*;
use crate::ai::agent::{AIIdentifiers, FileEdit};
use crate::ai::blocklist::SessionContext;
use crate::auth::auth_state::AuthState;

fn update_deltas(diff: &AIRequestedCodeDiff) -> &[DiffDelta] {
    match &diff.diff_type {
        DiffType::Update { deltas, .. } => deltas,
        other => panic!("Expected Update diff_type, got {other:?}"),
    }
}

/// Asserts the outcome has no errors and returns the applied diffs.
fn assert_success(outcome: ApplyEditsOutcome) -> Vec<AIRequestedCodeDiff> {
    assert!(
        outcome.errors.is_empty(),
        "Expected no errors but got: {:?}",
        outcome.errors
    );
    outcome.applied_diffs
}

/// Asserts the outcome has no applied diffs and returns the errors.
fn assert_failure(outcome: ApplyEditsOutcome) -> Vec<DiffApplicationError> {
    assert!(
        outcome.applied_diffs.is_empty(),
        "Expected no applied diffs but got {} diffs",
        outcome.applied_diffs.len()
    );
    outcome.errors
}

#[test]
fn test_apply_diffs_error_when_no_diffs_applied() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "First line\nSecond line\n").unwrap();

        let invalid_diff = ParsedDiff::StrReplaceEdit {
            file: Some(file_path.clone()),
            search: Some("1|This content doesn't exist in the file".to_string()),
            replace: Some("Replacement content".to_string()),
        };

        let outcome = apply_edits(
            vec![FileEdit::Edit(invalid_diff)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let errors = assert_failure(outcome);
        match &errors[..] {
            [DiffApplicationError::UnmatchedDiffs { file, .. }] => {
                assert_eq!(*file, file_path);
            }
            other => panic!("Expected a single UnmatchedDiffs error, got {other:?}"),
        }
    });
}

#[test]
fn test_apply_diffs_succeeds_with_valid_diff() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "First line\nSecond line\n").unwrap();

        let valid_diff = ParsedDiff::StrReplaceEdit {
            file: Some(file_path.clone()),
            search: Some("1|First line".to_string()),
            replace: Some("Modified first line".to_string()),
        };

        let outcome = apply_edits(
            vec![FileEdit::Edit(valid_diff)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let diffs = assert_success(outcome);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].file_name, file_path);

        let deltas = update_deltas(&diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].insertion, "Modified first line");
    });
}

#[test]
fn test_apply_diffs_with_partial_failures() {
    // When a matching and a non-matching hunk target the SAME file, the whole
    // file is treated as a failure — it appears in errors, not applied_diffs.
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "First line\nSecond line\n").unwrap();

        let valid_diff = ParsedDiff::StrReplaceEdit {
            file: Some(file_path.clone()),
            search: Some("1|First line".to_string()),
            replace: Some("Modified first line".to_string()),
        };
        let invalid_diff = ParsedDiff::StrReplaceEdit {
            file: Some(file_path.clone()),
            search: Some("1|This content doesn't exist".to_string()),
            replace: Some("Replacement content".to_string()),
        };

        let outcome = apply_edits(
            vec![FileEdit::Edit(valid_diff), FileEdit::Edit(invalid_diff)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let errors = assert_failure(outcome);
        match &errors[..] {
            [DiffApplicationError::UnmatchedDiffs { file, .. }] => {
                assert_eq!(*file, file_path);
            }
            other => panic!("Expected a single UnmatchedDiffs error, got {other:?}"),
        }
    });
}

#[test]
fn test_apply_diffs_with_new_file() {
    // TODO(ben): Drop support for this behavior once the file-creation tool is live.
    App::test((), |app| async move {
        let non_existent_file = "non_existent_file.txt".to_string();
        let create_file_diff = ParsedDiff::StrReplaceEdit {
            file: Some(non_existent_file.clone()),
            search: Some("".to_string()),
            replace: Some("New file content".to_string()),
        };

        let outcome = apply_edits(
            vec![FileEdit::Edit(create_file_diff)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let diffs = assert_success(outcome);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].file_name, non_existent_file);
        assert_eq!(diffs[0].failures, None);

        match &diffs[0].diff_type {
            DiffType::Create { delta } => {
                assert_eq!(delta.insertion, "New file content");
            }
            other => panic!("Expected Create diff_type, got {other:?}"),
        }
    });
}

#[test]
fn test_apply_diffs_with_missing_file() {
    App::test((), |app| async move {
        let non_existent_file = "non_existent_file.txt".to_string();
        let invalid_non_existent_diff = ParsedDiff::StrReplaceEdit {
            file: Some(non_existent_file.clone()),
            search: Some("1|Some content".to_string()),
            replace: Some("New content".to_string()),
        };

        let outcome = block_on(apply_edits(
            vec![FileEdit::Edit(invalid_non_existent_diff)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        ));

        let errors = assert_failure(outcome);
        match &errors[..] {
            [DiffApplicationError::MissingFile { file }] => {
                assert_eq!(*file, non_existent_file);
            }
            other => panic!("Expected a single MissingFile error, got {other:?}"),
        }
    });
}

/// Regression test for QUALITY-1253.
///
/// When a multi-file batch has some files that match and others that do not,
/// the matching files should be returned in `applied_diffs` and the failing
/// files should appear in `errors` — instead of discarding the whole batch.
#[test]
fn test_multi_file_batch_applies_successful_files_when_some_fail() {
    App::test((), |app| async move {
        let mut file1 = NamedTempFile::new().expect("Failed to create first temporary file");
        let file1_path = file1.path().to_string_lossy().to_string();
        writeln!(&mut file1, "File 1 content\nSecond line\n").unwrap();

        let mut file2 = NamedTempFile::new().expect("Failed to create second temporary file");
        let file2_path = file2.path().to_string_lossy().to_string();
        writeln!(&mut file2, "File 2 content\nAnother line\n").unwrap();

        let valid_diff = ParsedDiff::StrReplaceEdit {
            file: Some(file1_path.clone()),
            search: Some("1|File 1 content".to_string()),
            replace: Some("Modified file 1 content".to_string()),
        };
        let invalid_diff = ParsedDiff::StrReplaceEdit {
            file: Some(file2_path.clone()),
            search: Some("1|This doesn't match anything".to_string()),
            replace: Some("New content".to_string()),
        };

        let outcome = apply_edits(
            vec![FileEdit::Edit(valid_diff), FileEdit::Edit(invalid_diff)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        // file1 matched: should be in applied_diffs.
        assert_eq!(
            outcome.applied_diffs.len(),
            1,
            "Expected file1 diff to be in applied_diffs"
        );
        assert_eq!(outcome.applied_diffs[0].file_name, file1_path);
        let deltas = update_deltas(&outcome.applied_diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].insertion, "Modified file 1 content");

        // file2 failed: should be in errors only, not in applied_diffs.
        assert_eq!(outcome.errors.len(), 1, "Expected file2 to have one error");
        match &outcome.errors[..] {
            [DiffApplicationError::UnmatchedDiffs { file, .. }] => {
                assert_eq!(*file, file2_path);
            }
            other => panic!("Expected a single UnmatchedDiffs for file2, got {other:?}"),
        }
    });
}

#[test]
fn test_apply_diffs_noop_with_successful_change() {
    App::test((), |app| async move {
        let mut file = NamedTempFile::new().expect("Failed to create temporary file");
        writeln!(&mut file, "Line One\nLine Two\n").unwrap();
        let file_path = file.path().to_string_lossy().to_string();

        let diffs = vec![
            // This is effectively a no-op.
            ParsedDiff::StrReplaceEdit {
                file: Some(file_path.clone()),
                search: Some("1|Line one".to_string()),
                replace: Some("Line One".to_string()),
            },
            // This is a meaningful change.
            ParsedDiff::StrReplaceEdit {
                file: Some(file_path.clone()),
                search: Some("2|Line Two".to_string()),
                replace: Some("Last Line".to_string()),
            },
        ];

        let outcome = apply_edits(
            diffs.into_iter().map(FileEdit::Edit).collect(),
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let diffs = assert_success(outcome);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].file_name, file_path);
        assert_eq!(diffs[0].failures, None);

        let deltas = update_deltas(&diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(
            deltas[0],
            DiffDelta {
                insertion: "Last Line".to_string(),
                replacement_line_range: 2..3,
            }
        );
    });
}

#[test]
fn test_apply_diffs_fails_with_only_noop() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        let content = "First line\nSecond line\n";
        writeln!(temp_file, "{content}").unwrap();

        let noop_diff = ParsedDiff::StrReplaceEdit {
            file: Some(file_path.clone()),
            search: Some("1|First line".to_string()),
            replace: Some("First line".to_string()),
        };

        let outcome = apply_edits(
            vec![FileEdit::Edit(noop_diff)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let errors = assert_failure(outcome);
        match &errors[..] {
            [
                DiffApplicationError::UnmatchedDiffs {
                    file,
                    match_failures,
                },
            ] => {
                assert_eq!(*file, file_path);
                assert_eq!(match_failures.noop_deltas, 1);
                assert_eq!(match_failures.fuzzy_match_failures, 0);
            }
            other => panic!("Expected a single UnmatchedDiffs error, got {other:?}"),
        }
    });
}

#[test]
fn test_multiple_file_create_edits_for_same_path() {
    App::test((), |app| async move {
        let file_path = "new_file.txt".to_string();

        let outcome = apply_edits(
            vec![
                FileEdit::Create {
                    file: Some(file_path.clone()),
                    content: Some("First content".to_string()),
                },
                FileEdit::Create {
                    file: Some(file_path.clone()),
                    content: Some("Second content".to_string()),
                },
            ],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let errors = assert_failure(outcome);
        match &errors[..] {
            [DiffApplicationError::MultipleFileCreation { file }] => {
                assert_eq!(*file, file_path);
            }
            other => panic!("Expected a single MultipleFileCreation error, got {other:?}"),
        }
    });
}

#[test]
fn test_mixed_create_and_edit_for_same_path() {
    App::test((), |app| async move {
        let file_path = "mixed_file.txt".to_string();

        let outcome = apply_edits(
            vec![
                FileEdit::Create {
                    file: Some(file_path.clone()),
                    content: Some("New file content".to_string()),
                },
                FileEdit::Edit(ParsedDiff::StrReplaceEdit {
                    file: Some(file_path.clone()),
                    search: Some("1|Some existing content".to_string()),
                    replace: Some("Modified content".to_string()),
                }),
            ],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let errors = assert_failure(outcome);
        match &errors[..] {
            [DiffApplicationError::MultipleFileCreation { file }] => {
                assert_eq!(*file, file_path);
            }
            other => panic!("Expected a single MultipleFileCreation error, got {other:?}"),
        }
    });
}

#[test]
fn test_delete_and_create_same_path_replaces_existing_file() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "Old line one\nOld line two").unwrap();

        let outcome = apply_edits(
            vec![
                FileEdit::Delete {
                    file: Some(file_path.clone()),
                },
                FileEdit::Create {
                    file: Some(file_path.clone()),
                    content: Some("New file content".to_string()),
                },
            ],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let diffs = assert_success(outcome);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].file_name, file_path);
        assert_eq!(diffs[0].original_content, "Old line one\nOld line two\n");

        let deltas = update_deltas(&diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].replacement_line_range, 1..3);
        assert_eq!(deltas[0].insertion, "New file content");
    });
}

#[test]
fn test_create_then_delete_same_path_replaces_existing_file() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "Old line one\nOld line two").unwrap();

        let outcome = apply_edits(
            vec![
                FileEdit::Create {
                    file: Some(file_path.clone()),
                    content: Some("New file content".to_string()),
                },
                FileEdit::Delete {
                    file: Some(file_path.clone()),
                },
            ],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let diffs = assert_success(outcome);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].file_name, file_path);

        let deltas = update_deltas(&diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].replacement_line_range, 1..3);
        assert_eq!(deltas[0].insertion, "New file content");
    });
}

#[test]
fn test_delete_create_and_edit_same_path_still_fails() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "Existing content").unwrap();

        let outcome = apply_edits(
            vec![
                FileEdit::Delete {
                    file: Some(file_path.clone()),
                },
                FileEdit::Create {
                    file: Some(file_path.clone()),
                    content: Some("New file content".to_string()),
                },
                FileEdit::Edit(ParsedDiff::StrReplaceEdit {
                    file: Some(file_path.clone()),
                    search: Some("1|Existing content".to_string()),
                    replace: Some("Modified content".to_string()),
                }),
            ],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let errors = assert_failure(outcome);
        match &errors[..] {
            [DiffApplicationError::MultipleFileCreation { file }] => {
                assert_eq!(*file, file_path);
            }
            other => panic!("Expected a single MultipleFileCreation error, got {other:?}"),
        }
    });
}

#[test]
fn test_create_edit_for_existing_file() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "Existing content").unwrap();

        let outcome = apply_edits(
            vec![FileEdit::Create {
                file: Some(file_path.clone()),
                content: Some("New content".to_string()),
            }],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let errors = assert_failure(outcome);
        match &errors[..] {
            [DiffApplicationError::AlreadyExists { file }] => {
                assert_eq!(*file, file_path);
            }
            other => panic!("Expected a single AlreadyExists error, got {other:?}"),
        }
    });
}

#[test]
fn test_format_match_error() {
    // Single unmatched search block
    let err = DiffApplicationError::UnmatchedDiffs {
        file: "file.txt".to_string(),
        match_failures: DiffMatchFailures {
            fuzzy_match_failures: 1,
            noop_deltas: 0,
            missing_line_numbers: 0,
        },
    };
    let msg = err.to_conversation_message();
    assert!(
        msg.contains("1 search block"),
        "Should mention count: {msg}"
    );
    assert!(
        msg.contains("file.txt"),
        "Should mention the file name: {msg}"
    );
    assert!(
        msg.contains("Re-read") || msg.contains("re-read"),
        "Should include recovery hint: {msg}"
    );

    // Single noop delta
    let err = DiffApplicationError::UnmatchedDiffs {
        file: "file.txt".to_string(),
        match_failures: DiffMatchFailures {
            fuzzy_match_failures: 0,
            noop_deltas: 1,
            missing_line_numbers: 0,
        },
    };
    let msg = err.to_conversation_message();
    assert!(
        msg.contains("1 change") || msg.contains("already been applied"),
        "Should mention already applied: {msg}"
    );

    // Both failures and noops — the message should cover both
    let err = DiffApplicationError::UnmatchedDiffs {
        file: "file.txt".to_string(),
        match_failures: DiffMatchFailures {
            fuzzy_match_failures: 2,
            noop_deltas: 2,
            missing_line_numbers: 0,
        },
    };
    let msg = err.to_conversation_message();
    assert!(
        msg.contains("2 search block"),
        "Should mention the unmatched count: {msg}"
    );
    assert!(
        msg.contains("2 change") || msg.contains("already"),
        "Should mention the already-applied count: {msg}"
    );
}

#[test]
fn test_format_already_exists_message_includes_recovery_hint() {
    let err = DiffApplicationError::AlreadyExists {
        file: "spec.md".to_string(),
    };

    let msg = err.to_conversation_message();
    assert!(
        msg.contains("already exists"),
        "Message should mention the file exists: {msg}"
    );
    assert!(
        msg.contains("search-and-replace") || msg.contains("update"),
        "Message should hint at recovery: {msg}"
    );
}

#[test]
fn test_format_multiple_errors() {
    let errs = vec1![
        DiffApplicationError::MissingFile {
            file: "missing.rs".to_string(),
        },
        DiffApplicationError::UnmatchedDiffs {
            file: "unmatched.rs".to_string(),
            match_failures: DiffMatchFailures {
                fuzzy_match_failures: 1,
                noop_deltas: 0,
                missing_line_numbers: 0,
            },
        },
    ];

    let msg = DiffApplicationError::error_for_conversation(&errs);
    assert!(
        msg.contains("* missing.rs does not exist"),
        "Should list missing file error: {msg}"
    );
    assert!(
        msg.contains("* 1 search block"),
        "Should list unmatched block count: {msg}"
    );
    assert!(msg.contains("unmatched.rs"), "Should name the file: {msg}");
}

#[test]
fn test_format_single_errors() {
    let errs = vec1![DiffApplicationError::ReadFailed {
        file: "no_permissions.scala".to_string(),
        message: "permission denied".to_string(),
    },];

    assert_eq!(
        DiffApplicationError::error_for_conversation(&errs),
        "Could not read no_permissions.scala"
    );
}

// V4A Tests

#[test]
fn test_apply_v4a_edits_simple_match() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(
            &mut temp_file,
            "function foo() {{\n    console.log('hello');\n    return 42;\n}}"
        )
        .unwrap();

        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(file_path.clone()),
            move_to: None,
            hunks: vec![V4AHunk {
                change_context: vec![],
                pre_context: "function foo() {".to_string(),
                old: "    console.log('hello');".to_string(),
                new: "    console.log('world');".to_string(),
                post_context: "    return 42;".to_string(),
            }],
        };

        let outcome = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let diffs = assert_success(outcome);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].file_name, file_path);

        let deltas = update_deltas(&diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].insertion, "    console.log('world');");
        assert_eq!(deltas[0].replacement_line_range, 2..3);
    });
}

#[test]
fn test_apply_v4a_edits_with_jump_context() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(
            &mut temp_file,
            "class Foo {{\n    def bar():\n        pass\n    def baz():\n        return 1\n}}"
        )
        .unwrap();

        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(file_path.clone()),
            move_to: None,
            hunks: vec![V4AHunk {
                change_context: vec!["class Foo".to_string()],
                pre_context: "    def bar():".to_string(),
                old: "        pass".to_string(),
                new: "        return None".to_string(),
                post_context: "    def baz():".to_string(),
            }],
        };

        let outcome = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let diffs = assert_success(outcome);
        assert_eq!(diffs.len(), 1);

        let deltas = update_deltas(&diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].insertion, "        return None");
    });
}

#[test]
fn test_apply_v4a_edits_no_match() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "First line\nSecond line\n").unwrap();

        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(file_path.clone()),
            move_to: None,
            hunks: vec![V4AHunk {
                change_context: vec![],
                pre_context: "Non-existent pre context".to_string(),
                old: "Non-existent old content".to_string(),
                new: "New content".to_string(),
                post_context: "Non-existent post context".to_string(),
            }],
        };

        let outcome = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let errors = assert_failure(outcome);
        match &errors[..] {
            [DiffApplicationError::UnmatchedDiffs { file, .. }] => {
                assert_eq!(*file, file_path);
            }
            other => panic!("Expected a single UnmatchedDiffs error, got {other:?}"),
        }
    });
}

#[test]
fn test_apply_v4a_edits_noop() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "Line One\nLine Two\nLine Three").unwrap();

        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(file_path.clone()),
            move_to: None,
            hunks: vec![V4AHunk {
                change_context: vec![],
                pre_context: "Line One".to_string(),
                old: "Line Two".to_string(),
                new: "Line Two".to_string(),
                post_context: "Line Three".to_string(),
            }],
        };

        let outcome = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let errors = assert_failure(outcome);
        match &errors[..] {
            [
                DiffApplicationError::UnmatchedDiffs {
                    file,
                    match_failures,
                },
            ] => {
                assert_eq!(*file, file_path);
                assert_eq!(match_failures.noop_deltas, 1);
            }
            other => panic!("Expected a single UnmatchedDiffs error, got {other:?}"),
        }
    });
}

#[test]
fn test_apply_v4a_edits_multiline_change() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(
            &mut temp_file,
            "def calculate():\n    x = 1\n    y = 2\n    return x + y\n"
        )
        .unwrap();

        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(file_path.clone()),
            move_to: None,
            hunks: vec![V4AHunk {
                change_context: vec![],
                pre_context: "def calculate():".to_string(),
                old: "    x = 1\n    y = 2".to_string(),
                new: "    x = 10\n    y = 20".to_string(),
                post_context: "    return x + y".to_string(),
            }],
        };

        let outcome = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let diffs = assert_success(outcome);
        assert_eq!(diffs.len(), 1);

        let deltas = update_deltas(&diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].insertion, "    x = 10\n    y = 20");
        assert_eq!(deltas[0].replacement_line_range, 2..4);
    });
}

#[test]
fn test_apply_v4a_edits_nested_jump_context() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(
            &mut temp_file,
            "class Outer {{\n    class Inner {{\n        def method():\n            pass\n    }}\n}}"
        )
        .unwrap();

        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(file_path.clone()),
            move_to: None,
            hunks: vec![V4AHunk {
                change_context: vec!["class Outer".to_string(), "class Inner".to_string()],
                pre_context: "        def method():".to_string(),
                old: "            pass".to_string(),
                new: "            return True".to_string(),
                post_context: "    }".to_string(),
            }],
        };

        let outcome = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let diffs = assert_success(outcome);
        assert_eq!(diffs.len(), 1);

        let deltas = update_deltas(&diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].insertion, "            return True");
    });
}

#[test]
fn test_apply_v4a_edits_missing_file() {
    App::test((), |app| async move {
        let non_existent_file = "non_existent_file.txt".to_string();

        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(non_existent_file.clone()),
            move_to: None,
            hunks: vec![V4AHunk {
                change_context: vec![],
                pre_context: "pre".to_string(),
                old: "old content".to_string(),
                new: "new content".to_string(),
                post_context: "post".to_string(),
            }],
        };

        let outcome = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let errors = assert_failure(outcome);
        match &errors[..] {
            [DiffApplicationError::MissingFile { file }] => {
                assert_eq!(*file, non_existent_file);
            }
            other => panic!("Expected a single MissingFile error, got {other:?}"),
        }
    });
}

#[test]
fn test_apply_v4a_edits_empty_context() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "first\nsecond\nthird").unwrap();

        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(file_path.clone()),
            move_to: None,
            hunks: vec![V4AHunk {
                change_context: vec![],
                pre_context: "".to_string(),
                old: "second".to_string(),
                new: "SECOND".to_string(),
                post_context: "".to_string(),
            }],
        };

        let outcome = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let diffs = assert_success(outcome);
        assert_eq!(diffs.len(), 1);

        let deltas = update_deltas(&diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].insertion, "SECOND");
    });
}

// V4A Rename Tests

#[test]
fn test_apply_v4a_rename_to_nonexistent_file() {
    App::test((), |app| async move {
        let mut source_file = NamedTempFile::new().expect("Failed to create source file");
        let source_path = source_file.path().to_string_lossy().to_string();
        writeln!(&mut source_file, "line one\nline two\nline three").unwrap();

        let target_path = format!("{}_renamed.txt", source_path);

        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(source_path.clone()),
            move_to: Some(target_path.clone()),
            hunks: vec![V4AHunk {
                change_context: vec![],
                pre_context: "line one".to_string(),
                old: "line two".to_string(),
                new: "LINE TWO MODIFIED".to_string(),
                post_context: "line three".to_string(),
            }],
        };

        let outcome = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let diffs = assert_success(outcome);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].file_name, source_path);

        match &diffs[0].diff_type {
            DiffType::Update { deltas, rename } => {
                assert_eq!(*rename, Some(target_path.into()));
                assert_eq!(deltas.len(), 1);
                assert_eq!(deltas[0].insertion, "LINE TWO MODIFIED");
            }
            other => panic!("Expected Update diff_type with rename, got {other:?}"),
        }
    });
}

#[test]
fn test_apply_v4a_rename_to_existing_file() {
    App::test((), |app| async move {
        let mut source_file = NamedTempFile::new().expect("Failed to create source file");
        let source_path = source_file.path().to_string_lossy().to_string();
        writeln!(
            &mut source_file,
            "source line one\nsource line two\nsource line three"
        )
        .unwrap();

        let mut target_file = NamedTempFile::new().expect("Failed to create target file");
        let target_path = target_file.path().to_string_lossy().to_string();
        writeln!(&mut target_file, "target old content\nshould be replaced").unwrap();

        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(source_path.clone()),
            move_to: Some(target_path.clone()),
            hunks: vec![V4AHunk {
                change_context: vec![],
                pre_context: "source line one".to_string(),
                old: "source line two".to_string(),
                new: "MODIFIED LINE TWO".to_string(),
                post_context: "source line three".to_string(),
            }],
        };

        let outcome = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let diffs = assert_success(outcome);
        assert_eq!(diffs.len(), 2);

        assert_eq!(diffs[0].file_name, source_path);
        match &diffs[0].diff_type {
            DiffType::Delete { .. } => {}
            other => panic!("Expected Delete diff_type for source, got {other:?}"),
        }

        assert_eq!(diffs[1].file_name, target_path);
        match &diffs[1].diff_type {
            DiffType::Update { deltas, rename } => {
                assert!(rename.is_none(), "Target update should not have rename");
                assert_eq!(deltas.len(), 2);
                assert!(deltas[0].insertion.contains("source line one"));
                assert!(deltas[0].insertion.contains("source line two"));
                assert!(deltas[0].insertion.contains("source line three"));
                assert!(deltas[1].insertion.contains("MODIFIED LINE TWO"));
            }
            other => panic!("Expected Update diff_type for target, got {other:?}"),
        }
    });
}

#[test]
fn test_apply_v4a_rename_to_existing_file_no_deltas() {
    App::test((), |app| async move {
        let mut source_file = NamedTempFile::new().expect("Failed to create source file");
        let source_path = source_file.path().to_string_lossy().to_string();
        writeln!(&mut source_file, "source content only").unwrap();

        let mut target_file = NamedTempFile::new().expect("Failed to create target file");
        let target_path = target_file.path().to_string_lossy().to_string();
        writeln!(&mut target_file, "target old content").unwrap();

        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(source_path.clone()),
            move_to: Some(target_path.clone()),
            hunks: vec![],
        };

        let outcome = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let diffs = assert_success(outcome);
        assert_eq!(diffs.len(), 2);

        assert_eq!(diffs[0].file_name, source_path);
        match &diffs[0].diff_type {
            DiffType::Delete { .. } => {}
            other => panic!("Expected Delete diff_type for source, got {other:?}"),
        }

        assert_eq!(diffs[1].file_name, target_path);
        match &diffs[1].diff_type {
            DiffType::Update { deltas, rename } => {
                assert!(rename.is_none());
                assert_eq!(deltas.len(), 1);
                assert_eq!(deltas[0].insertion, "source content only\n");
            }
            other => panic!("Expected Update diff_type for target, got {other:?}"),
        }
    });
}
