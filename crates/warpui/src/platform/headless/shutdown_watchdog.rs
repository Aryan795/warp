use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Condvar, Mutex};

/// How long headless shutdown may run before the process is terminated the hard way.
///
/// The budget starts only once the event loop has broken, so a command has already finished its
/// work and printed its output. It sits well above the longest bounded step in the shutdown
/// sequence (a five second telemetry flush) so a healthy shutdown never trips it.
pub(super) const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);

/// Guarantees the process terminates once headless shutdown has been set in motion.
///
/// Headless shutdown depends on a chain of cooperating steps — the event loop reading a
/// termination request, then a telemetry flush, persistence-writer join, language-server
/// teardown, and crash-reporter drain — and any one of them blocking indefinitely leaves a
/// finished CLI invocation hanging at the user's prompt. Arming the watchdog turns that class of
/// hang into a bounded delay followed by an exit.
pub(super) struct ShutdownWatchdog {
    state: Arc<WatchdogState>,
}

impl ShutdownWatchdog {
    /// Arms a watchdog that exits the process with `exit_code` if [`Self::disarm`] (or a drop) has
    /// not happened within `timeout`.
    pub(super) fn arm(timeout: Duration, exit_code: i32) -> Self {
        Self::arm_with(timeout, move || {
            // Logging is one of the subsystems shutdown tears down, so also report on stderr to
            // guarantee the user learns why the process left early.
            eprintln!("Warp: shutdown did not finish within {timeout:?}; exiting now.");
            log::error!("Headless shutdown timed out after {timeout:?}; forcing exit");
            std::process::exit(exit_code);
        })
    }

    /// Arms a deadline nothing can cancel: the process exits with `exit_code` once `timeout`
    /// elapses, unless it has already left on its own.
    pub(super) fn arm_permanently(timeout: Duration, exit_code: i32) {
        // Skipping the guard's drop is the point: no caller is in a position to decide the
        // deadline no longer applies.
        std::mem::forget(Self::arm(timeout, exit_code));
    }

    fn arm_with(timeout: Duration, on_timeout: impl FnOnce() + Send + 'static) -> Self {
        let state = Arc::new(WatchdogState::default());
        let spawned = std::thread::Builder::new()
            .name("warp-shutdown-watchdog".to_owned())
            .spawn({
                let state = state.clone();
                move || {
                    if !state.wait_for_disarm(timeout) {
                        on_timeout();
                    }
                }
            });

        if let Err(e) = spawned {
            log::warn!("Failed to arm the headless shutdown watchdog: {e}");
            state.disarm();
        }

        Self { state }
    }

    /// Records that shutdown finished, so the watchdog will not fire.
    pub(super) fn disarm(&self) {
        self.state.disarm();
    }
}

impl Drop for ShutdownWatchdog {
    fn drop(&mut self) {
        self.disarm();
    }
}

#[derive(Default)]
struct WatchdogState {
    disarmed: Mutex<bool>,
    disarmed_signal: Condvar,
}

impl WatchdogState {
    /// Blocks until the watchdog is disarmed, returning whether that happened before `timeout`.
    fn wait_for_disarm(&self, timeout: Duration) -> bool {
        let mut disarmed = self.disarmed.lock();
        self.disarmed_signal
            .wait_while_for(&mut disarmed, |disarmed| !*disarmed, timeout);
        *disarmed
    }

    fn disarm(&self) {
        *self.disarmed.lock() = true;
        self.disarmed_signal.notify_all();
    }
}

#[cfg(test)]
#[path = "shutdown_watchdog_tests.rs"]
mod tests;
