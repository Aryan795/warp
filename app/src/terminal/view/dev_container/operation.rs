use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use uuid::Uuid;
use warpui::{Entity, ModelContext};

use super::registry::DevContainerBuildKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DevContainerBuildPhase {
    Build,
    Preflight,
    Staging,
    Attach,
}

impl DevContainerBuildPhase {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Build => "Build",
            Self::Preflight => "Preflight",
            Self::Staging => "Staging",
            Self::Attach => "Attach",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DevContainerBuildStatus {
    Running,
    Failed,
    Cancelling,
    Cancelled,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevContainerBuildFailure {
    pub phase: DevContainerBuildPhase,
    pub message: String,
}

#[derive(Clone)]
pub(crate) struct DevContainerBuildCancel {
    cancelled: Arc<AtomicBool>,
    process_group_id: Arc<Mutex<Option<u32>>>,
}

impl DevContainerBuildCancel {
    fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            process_group_id: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub(crate) fn set_process_group_id(&self, id: u32) {
        *self.process_group_id.lock() = Some(id);
    }

    pub(crate) fn take_process_group_id(&self) -> Option<u32> {
        self.process_group_id.lock().take()
    }
}

pub(crate) struct DevContainerBuildOperation {
    key: DevContainerBuildKey,
    operation_id: Uuid,
    attempt_id: u64,
    workspace_folder: PathBuf,
    config_file: PathBuf,
    phase: DevContainerBuildPhase,
    status: DevContainerBuildStatus,
    failure: Option<DevContainerBuildFailure>,
    cancel: DevContainerBuildCancel,
}

impl DevContainerBuildOperation {
    pub(crate) fn new(key: DevContainerBuildKey) -> Self {
        let workspace_folder = key.workspace_folder.clone();
        let config_file = key.config_file.clone();
        Self {
            key,
            operation_id: Uuid::new_v4(),
            attempt_id: 1,
            workspace_folder,
            config_file,
            phase: DevContainerBuildPhase::Build,
            status: DevContainerBuildStatus::Running,
            failure: None,
            cancel: DevContainerBuildCancel::new(),
        }
    }

    pub(crate) fn key(&self) -> &DevContainerBuildKey {
        &self.key
    }

    pub(crate) fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    pub(crate) fn attempt_id(&self) -> u64 {
        self.attempt_id
    }

    pub(crate) fn workspace_folder(&self) -> &PathBuf {
        &self.workspace_folder
    }

    pub(crate) fn config_file(&self) -> &PathBuf {
        &self.config_file
    }

    pub(crate) fn phase(&self) -> DevContainerBuildPhase {
        self.phase
    }

    pub(crate) fn status(&self) -> DevContainerBuildStatus {
        self.status
    }

    pub(crate) fn failure(&self) -> Option<&DevContainerBuildFailure> {
        self.failure.as_ref()
    }

    pub(crate) fn header_title(&self) -> String {
        let workspace = self
            .workspace_folder
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.workspace_folder.display().to_string());
        match self.status() {
            DevContainerBuildStatus::Failed => {
                format!("{} · {} failed", workspace, self.phase().label())
            }
            DevContainerBuildStatus::Cancelling | DevContainerBuildStatus::Cancelled => {
                format!("{} · Cancelling", workspace)
            }
            DevContainerBuildStatus::Running | DevContainerBuildStatus::Completed => {
                format!("{} · {}", workspace, self.phase().label())
            }
        }
    }

    pub(crate) fn header_error(&self) -> Option<&str> {
        self.failure().map(|failure| failure.message.as_str())
    }

    pub(crate) fn shows_retry_and_close(&self) -> bool {
        self.status == DevContainerBuildStatus::Failed
    }

    pub(crate) fn cancel_handle(&self) -> DevContainerBuildCancel {
        self.cancel.clone()
    }

    pub(crate) fn is_current_attempt(&self, operation_id: Uuid, attempt_id: u64) -> bool {
        self.operation_id == operation_id
            && self.attempt_id == attempt_id
            && !matches!(
                self.status,
                DevContainerBuildStatus::Cancelled | DevContainerBuildStatus::Completed
            )
            && !self.cancel.is_cancelled()
    }

    pub(crate) fn set_phase(
        &mut self,
        phase: DevContainerBuildPhase,
        ctx: &mut ModelContext<Self>,
    ) {
        self.phase = phase;
        ctx.notify();
    }

    pub(crate) fn fail(
        &mut self,
        phase: DevContainerBuildPhase,
        message: String,
        ctx: &mut ModelContext<Self>,
    ) {
        self.phase = phase;
        self.status = DevContainerBuildStatus::Failed;
        self.failure = Some(DevContainerBuildFailure { phase, message });
        ctx.notify();
    }

    pub(crate) fn complete(&mut self, ctx: &mut ModelContext<Self>) {
        self.status = DevContainerBuildStatus::Completed;
        ctx.notify();
    }

    /// Marks the operation cancelled before the caller terminates processes or
    /// removes the pane, so a late completion is a no-op.
    pub(crate) fn tombstone(&mut self, ctx: &mut ModelContext<Self>) {
        self.cancel.cancelled.store(true, Ordering::SeqCst);
        if self.status == DevContainerBuildStatus::Running {
            self.status = DevContainerBuildStatus::Cancelling;
        }
        ctx.notify();
    }

    pub(crate) fn mark_cancelled(&mut self, ctx: &mut ModelContext<Self>) {
        self.status = DevContainerBuildStatus::Cancelled;
        ctx.notify();
    }

    pub(crate) fn begin_retry(&mut self, ctx: &mut ModelContext<Self>) -> (u64, Option<u32>) {
        self.cancel.cancelled.store(true, Ordering::SeqCst);
        let prior_process_group = self.cancel.take_process_group_id();
        self.attempt_id += 1;
        self.phase = DevContainerBuildPhase::Build;
        self.status = DevContainerBuildStatus::Running;
        self.failure = None;
        self.cancel = DevContainerBuildCancel::new();
        ctx.notify();
        (self.attempt_id, prior_process_group)
    }
}

impl Entity for DevContainerBuildOperation {
    type Event = ();
}

#[cfg(test)]
#[path = "operation_tests.rs"]
mod tests;
