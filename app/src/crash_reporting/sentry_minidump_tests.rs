use std::time::Duration;

use command::blocking::Command;
use serial_test::serial;

use super::*;

/// A process that exits (almost) immediately should be reported as reaped well within
/// the bound, without needing to hit the timeout branch.
#[test]
fn wait_with_timeout_reaps_a_short_lived_process() {
    let mut child = spawn_short_lived_process();

    let reaped = wait_with_timeout(&mut child, Duration::from_secs(5))
        .expect("waiting on a valid child should not error");

    assert!(reaped, "a process that already exited should be reaped");
}

/// A killed longer-running process should be reaped once it actually exits, even though
/// it wasn't already dead when the wait started.
#[test]
fn wait_with_timeout_reaps_a_killed_process() {
    let mut child = spawn_long_lived_process();

    child.kill().expect("failed to kill helper process");

    let reaped = wait_with_timeout(&mut child, Duration::from_secs(5))
        .expect("waiting on a killed child should not error");

    assert!(reaped, "a killed process should be reaped within the bound");
}

/// If the process never exits, the wait must give up at the bound instead of hanging
/// forever -- this is the whole point of using a bounded wait instead of a plain `wait()`.
#[test]
fn wait_with_timeout_gives_up_on_a_still_running_process() {
    let mut child = spawn_long_lived_process();

    let reaped = wait_with_timeout(&mut child, Duration::from_millis(200))
        .expect("polling a running child should not error");

    assert!(
        !reaped,
        "a still-running process must not be reported as reaped"
    );

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn kill_then_wait_reaps_even_if_kill_fails() {
    let mut child = spawn_short_lived_process();
    child.wait().expect("helper should exit");

    let reaped = kill_then_wait(&mut child).expect("reap after a failed kill should not error");

    assert!(
        reaped,
        "an already-exited child must still be reaped when kill fails"
    );
}

#[test]
#[serial]
fn live_disable_returns_without_waiting_for_child_reap() {
    reset_state_for_tests();

    let child = spawn_long_lived_process();
    #[cfg(unix)]
    let pid = child.id();
    let start = instant::Instant::now();
    spawn_reaper(child);
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "live disable must not block on the bounded reap wait"
    );

    #[cfg(unix)]
    {
        let _ = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    }
    reset_state_for_tests();
}

#[test]
#[serial]
fn uninit_before_exit_joins_reaper_started_by_live_disable() {
    reset_state_for_tests();

    let mut child = spawn_long_lived_process();
    #[cfg(unix)]
    let pid = child.id();
    if let Err(err) = child.kill() {
        log::warn!("Unable to kill minidump child process: {err:#}");
    }
    spawn_reaper(child);

    uninit();
    uninit_before_exit();

    #[cfg(unix)]
    {
        let exists = unsafe { libc::kill(pid as i32, 0) };
        assert_eq!(
            exists, -1,
            "exit shutdown must join the live-disable reaper so the child is gone"
        );
    }
    reset_state_for_tests();
}

#[test]
#[serial]
fn uninit_sentry_then_before_exit_joins_reaper_after_live_disable() {
    reset_state_for_tests();

    let mut child = spawn_long_lived_process();
    #[cfg(unix)]
    let pid = child.id();
    if let Err(err) = child.kill() {
        log::warn!("Unable to kill minidump child process: {err:#}");
    }
    spawn_reaper(child);

    super::super::uninit_sentry();
    super::super::uninit_sentry_before_exit();

    #[cfg(unix)]
    {
        let exists = unsafe { libc::kill(pid as i32, 0) };
        assert_eq!(
            exists, -1,
            "uninit_sentry_before_exit must drain reapers after uninit_sentry"
        );
    }
    reset_state_for_tests();
}

fn spawn_short_lived_process() -> process::Child {
    #[cfg(unix)]
    let mut command = Command::new("true");
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "exit 0"]);
        command
    };
    command.spawn().expect("failed to spawn helper process")
}

fn spawn_long_lived_process() -> process::Child {
    #[cfg(unix)]
    let mut command = {
        let mut command = Command::new("sleep");
        command.arg("30");
        command
    };
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("ping");
        command.args(["-n", "30", "127.0.0.1"]);
        command
    };
    command.spawn().expect("failed to spawn helper process")
}
