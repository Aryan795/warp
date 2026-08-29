use std::path::PathBuf;

use super::{DevContainerBuildOperation, DevContainerBuildPhase, DevContainerBuildStatus};
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
    assert!(op.failure().is_none());
}

#[test]
fn tombstone_rejects_late_completions_for_the_same_attempt() {
    let mut op = DevContainerBuildOperation::new(key());
    let operation_id = op.operation_id();
    let attempt_id = op.attempt_id();
    op.cancel
        .cancelled
        .store(true, std::sync::atomic::Ordering::SeqCst);
    op.status = DevContainerBuildStatus::Cancelling;
    assert!(!op.is_current_attempt(operation_id, attempt_id));
}

#[test]
fn retry_increments_attempt_and_clears_failure() {
    let mut op = DevContainerBuildOperation::new(key());
    let first_attempt = op.attempt_id();
    let first_id = op.operation_id();
    op.phase = DevContainerBuildPhase::Preflight;
    op.status = DevContainerBuildStatus::Failed;
    op.failure = Some(super::DevContainerBuildFailure {
        phase: DevContainerBuildPhase::Preflight,
        message: "boom".to_owned(),
    });
    op.cancel
        .cancelled
        .store(true, std::sync::atomic::Ordering::SeqCst);

    op.attempt_id += 1;
    op.phase = DevContainerBuildPhase::Build;
    op.status = DevContainerBuildStatus::Running;
    op.failure = None;
    op.cancel = super::DevContainerBuildCancel::new();

    assert_eq!(op.operation_id(), first_id);
    assert_eq!(op.attempt_id(), first_attempt + 1);
    assert_eq!(op.phase(), DevContainerBuildPhase::Build);
    assert_eq!(op.status(), DevContainerBuildStatus::Running);
    assert!(op.failure().is_none());
    assert!(op.is_current_attempt(first_id, first_attempt + 1));
    assert!(!op.is_current_attempt(first_id, first_attempt));
}
