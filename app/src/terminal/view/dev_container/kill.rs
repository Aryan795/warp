use std::sync::Arc;

use parking_lot::Mutex;

/// Holds the process-group id until the first terminate, so Drop cannot
/// SIGKILL a pid that has already been reused.
#[derive(Clone)]
pub(crate) struct ProcessGroupKillOnDrop {
    process_group_id: Arc<Mutex<Option<u32>>>,
}

impl ProcessGroupKillOnDrop {
    pub(crate) fn new(process_group_id: u32) -> Self {
        Self {
            process_group_id: Arc::new(Mutex::new(Some(process_group_id))),
        }
    }

    pub(crate) fn terminate_now(&self) {
        if let Some(process_group_id) = self.process_group_id.lock().take() {
            terminate_process_group(process_group_id);
        }
    }
}

impl Drop for ProcessGroupKillOnDrop {
    fn drop(&mut self) {
        self.terminate_now();
    }
}

#[cfg(test)]
thread_local! {
    static PROCESS_GROUP_TERMINATIONS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn take_process_group_terminations() -> u32 {
    PROCESS_GROUP_TERMINATIONS.with(std::cell::Cell::take)
}

pub(crate) fn terminate_process_group(process_group_id: u32) {
    #[cfg(test)]
    PROCESS_GROUP_TERMINATIONS.with(|count| count.set(count.get() + 1));
    #[cfg(unix)]
    {
        use nix::sys::signal::{Signal, kill};
        use nix::unistd::Pid;
        if process_group_id < 2 {
            log::warn!("Refusing to signal process group {process_group_id}: pid is below 2");
            return;
        }
        match kill(Pid::from_raw(-(process_group_id as i32)), Signal::SIGKILL) {
            Ok(()) => log::info!("Sent SIGKILL to process group {process_group_id}"),
            Err(error @ nix::errno::Errno::ESRCH) => {
                log::info!("Process group {process_group_id} had already exited: {error}");
            }
            Err(error) => {
                log::warn!("Failed to kill process group {process_group_id}: {error}");
            }
        }
    }
    #[cfg(not(unix))]
    let _ = process_group_id;
}
