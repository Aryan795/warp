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

#[test]
#[serial]
fn duplicate_init_does_not_deadlock() {
    reset_state_for_tests();

    init();
    let installed = has_guard();
    init();

    assert_eq!(
        has_guard(),
        installed,
        "a second init must not replace or drop an existing guard"
    );
    reset_state_for_tests();
}

#[test]
#[serial]
fn init_after_exit_does_not_install_a_guard() {
    reset_state_for_tests();

    uninit_before_exit();
    init();

    assert!(
        !has_guard(),
        "init after exit drain must not spawn a child that will not be joined"
    );
    reset_state_for_tests();
}

#[test]
fn rollback_spawned_child_reaps_a_live_process() {
    let mut child = spawn_long_lived_process();
    let pid = child.id();
    assert_ne!(pid, 0, "test setup must spawn a real child");
    assert!(
        child
            .try_wait()
            .expect("try_wait on a live child")
            .is_none(),
        "test setup must have a still-running child"
    );

    rollback_spawned_child(child);

    #[cfg(unix)]
    {
        let exists = unsafe { libc::kill(pid as i32, 0) };
        assert_eq!(exists, -1, "rollback must reap the spawned child");
    }
}

#[test]
#[serial]
fn exit_waits_until_rejected_in_flight_start_is_reaped() {
    reset_state_for_tests();

    let mut child = spawn_long_lived_process();
    #[cfg(unix)]
    let pid = child.id();
    assert!(
        child
            .try_wait()
            .expect("try_wait on a live child")
            .is_none(),
        "test setup must have a still-running child"
    );

    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let start_thread = std::thread::spawn(move || {
        init_with_child_stalled_for_tests(child, release_rx);
    });

    let wait_for_in_flight = instant::Instant::now();
    while in_flight_inits() == 0 {
        assert!(
            wait_for_in_flight.elapsed() < Duration::from_secs(2),
            "in-flight init never registered"
        );
        std::thread::yield_now();
    }

    let exit_thread = std::thread::spawn(uninit_before_exit);

    let wait_for_exit_block = instant::Instant::now();
    while wait_for_exit_block.elapsed() < Duration::from_millis(500) {
        if STATE.lock().exiting {
            break;
        }
        std::thread::yield_now();
    }
    assert!(STATE.lock().exiting, "exit must have committed exiting");
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        !exit_thread.is_finished(),
        "exit must not return while an in-flight start still holds a child"
    );

    #[cfg(unix)]
    {
        let exists = unsafe { libc::kill(pid as i32, 0) };
        assert_eq!(exists, 0, "child must still be alive until start resumes");
    }

    release_tx.send(()).expect("start thread should be waiting");
    start_thread.join().expect("start thread should finish");
    exit_thread.join().expect("exit should return after reap");

    #[cfg(unix)]
    {
        let exists = unsafe { libc::kill(pid as i32, 0) };
        assert_eq!(
            exists, -1,
            "exit must not return before the rejected child is reaped"
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
