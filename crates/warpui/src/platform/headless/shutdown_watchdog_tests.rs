use std::sync::mpsc;
use std::time::Duration;

use super::ShutdownWatchdog;

const WATCHDOG_TIMEOUT: Duration = Duration::from_millis(50);
/// Long enough for a fired watchdog to be observed, short enough to keep the suite fast.
const OBSERVE_TIMEOUT: Duration = Duration::from_secs(5);

/// A shutdown step that never finishes must not keep the process alive, or a CLI command that has
/// already printed its output strands the user's terminal.
#[test]
fn fires_when_shutdown_never_finishes() {
    let (fired_sender, fired_receiver) = mpsc::channel();
    let _watchdog = ShutdownWatchdog::arm_with(WATCHDOG_TIMEOUT, move || {
        let _ = fired_sender.send(());
    });

    assert!(
        fired_receiver.recv_timeout(OBSERVE_TIMEOUT).is_ok(),
        "the watchdog should force termination when shutdown never finishes"
    );
}

#[test]
fn stays_silent_when_shutdown_finishes_in_time() {
    let (fired_sender, fired_receiver) = mpsc::channel();
    let watchdog = ShutdownWatchdog::arm_with(Duration::from_secs(30), move || {
        let _ = fired_sender.send(());
    });

    watchdog.disarm();

    // A watchdog that stands down drops its callback, so the channel closes without a value.
    assert!(
        fired_receiver.recv_timeout(WATCHDOG_TIMEOUT * 4).is_err(),
        "a shutdown that finishes in time must never be force-terminated"
    );
}

#[test]
fn dropping_the_guard_disarms_it() {
    let (fired_sender, fired_receiver) = mpsc::channel();
    drop(ShutdownWatchdog::arm_with(WATCHDOG_TIMEOUT, move || {
        let _ = fired_sender.send(());
    }));

    assert!(
        fired_receiver.recv_timeout(WATCHDOG_TIMEOUT * 4).is_err(),
        "a dropped watchdog must not fire"
    );
}
