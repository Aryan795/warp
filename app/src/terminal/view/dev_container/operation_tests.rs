use std::path::PathBuf;
use std::time::Duration;

use super::{
    BUILD_SILENCE_THRESHOLD, DevContainerBuildOperation, DevContainerBuildPhase,
    DevContainerBuildStatus, silence_subtitle,
};
use crate::terminal::view::dev_container::registry::DevContainerBuildKey;

fn key() -> DevContainerBuildKey {
    DevContainerBuildKey {
        workspace_folder: PathBuf::from("/tmp/ws"),
        config_file: PathBuf::from("/tmp/ws/.devcontainer/devcontainer.json"),
    }
}

#[test]
fn new_operation_starts_in_build_running() {
    let op = DevContainerBuildOperation::new(key());
    assert_eq!(op.phase(), DevContainerBuildPhase::Build);
    assert_eq!(op.status(), DevContainerBuildStatus::Running);
    assert_eq!(op.attempt_id(), 1);
}

#[test]
fn tombstone_rejects_late_completions_for_the_same_attempt() {
    let mut op = DevContainerBuildOperation::new(key());
    let operation_id = op.operation_id();
    let attempt_id = op.attempt_id();
    op.cancel.mark_cancelled();
    op.status = DevContainerBuildStatus::Cancelling;
    assert!(!op.is_current_attempt(operation_id, attempt_id));
}

#[test]
fn retry_increments_attempt_and_resets_running() {
    let mut op = DevContainerBuildOperation::new(key());
    let first_attempt = op.attempt_id();
    let first_id = op.operation_id();
    op.phase = DevContainerBuildPhase::Preflight;
    op.status = DevContainerBuildStatus::Failed;
    op.cancel.mark_cancelled();

    op.attempt_id += 1;
    op.phase = DevContainerBuildPhase::Build;
    op.status = DevContainerBuildStatus::Running;
    op.cancel = super::DevContainerBuildCancel::new();

    assert_eq!(op.operation_id(), first_id);
    assert_eq!(op.attempt_id(), first_attempt + 1);
    assert_eq!(op.phase(), DevContainerBuildPhase::Build);
    assert_eq!(op.status(), DevContainerBuildStatus::Running);
    assert!(op.is_current_attempt(first_id, first_attempt + 1));
    assert!(!op.is_current_attempt(first_id, first_attempt));
}

#[test]
fn silence_subtitle_is_none_below_threshold() {
    assert_eq!(silence_subtitle(Duration::from_secs(0)), None);
    assert_eq!(
        silence_subtitle(BUILD_SILENCE_THRESHOLD - Duration::from_secs(1)),
        None
    );
}

#[test]
fn silence_subtitle_names_elapsed_minutes_at_threshold() {
    assert_eq!(
        silence_subtitle(BUILD_SILENCE_THRESHOLD).as_deref(),
        Some("No output for 2m")
    );
    assert_eq!(
        silence_subtitle(Duration::from_secs(180)).as_deref(),
        Some("No output for 3m")
    );
}

#[test]
fn running_build_shows_close_without_retry() {
    let op = DevContainerBuildOperation::new(key());
    assert!(op.shows_close());
    assert!(!op.shows_retry());
    assert_eq!(op.header_secondary(), "");
}

#[test]
fn failed_build_clears_silence_subtitle() {
    let mut op = DevContainerBuildOperation::new(key());
    op.status = DevContainerBuildStatus::Failed;
    assert!(op.shows_retry());
    assert!(op.shows_close());
    assert_eq!(op.header_secondary(), "");
}
