use std::io;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use futures_lite::future::block_on;
use warp_terminal::model::ansi::Processor;
use warpui::r#async::executor::Background;

use super::{STDOUT_LIMIT, drain_dev_container_pipes};
use crate::terminal::SizeInfo;
use crate::terminal::color::{self, Colors};
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::terminal_model::TerminalModel;
use crate::terminal::model::test_utils::block_size;
use crate::terminal::view::dev_container::newline::NewlineNormalizer;

#[test]
fn devcontainer_up_drains_stdout_and_stderr_concurrently() {
    block_on(async {
        let mut command = command::r#async::Command::new("python3");
        command
            .arg("-c")
            .arg(
                r#"
import os, threading
def write(fd, payload):
    os.write(fd, payload)
blob = b"x" * (256 * 1024)
threads = [
    threading.Thread(target=write, args=(1, blob)),
    threading.Thread(target=write, args=(2, b"marker-before-exit\n" + blob)),
]
for thread in threads:
    thread.start()
for thread in threads:
    thread.join()
os.write(1, b'\n{"outcome":"success","containerId":"abc","remoteWorkspaceFolder":"/w"}\n')
"#,
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().expect("spawn fake child");

        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");
        let seen_stderr = Arc::new(Mutex::new(Vec::new()));
        let seen_for_callback = seen_stderr.clone();
        let result = drain_dev_container_pipes(stdout, stderr, move |chunk| {
            seen_for_callback.lock().unwrap().extend_from_slice(chunk);
        })
        .await
        .expect("drain");
        let status = child.status().await.expect("wait");
        assert!(status.success());
        assert!(
            seen_stderr
                .lock()
                .unwrap()
                .windows(b"marker-before-exit".len())
                .any(|window| window == b"marker-before-exit")
        );
        let stdout_text = String::from_utf8_lossy(&result.stdout.bytes);
        assert!(stdout_text.contains(r#""outcome":"success""#));
        assert!(!result.stdout.oversized);
        assert!(!result.stderr_tail.is_empty());
    });
}

#[test]
fn successful_drain_terminates_process_group_once() {
    let _ = super::take_process_group_terminations();
    block_on(async {
        let mut command = command::r#async::Command::new_with_process_group("python3");
        command
            .arg("-c")
            .arg("pass")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let (drain, success) = super::drain_dev_container_child(command, None, |_| {})
            .await
            .expect("drain");
        assert!(success);
        assert!(!drain.stdout.oversized);
    });
    assert_eq!(super::take_process_group_terminations(), 1);
}

fn sleep_process_group_command() -> command::r#async::Command {
    let mut command = command::r#async::Command::new_with_process_group("python3");
    command
        .arg("-c")
        .arg("import time; time.sleep(30)")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

#[test]
fn cancel_during_build_terminates_process_group_once() {
    use instant::Instant;

    use crate::terminal::view::dev_container::operation::DevContainerBuildCancel;

    let _ = super::take_process_group_terminations();
    let cancel = DevContainerBuildCancel::new();
    let started = Instant::now();
    block_on(async {
        let drain_fut =
            super::drain_dev_container_child(sleep_process_group_command(), Some(&cancel), |_| {});
        let kill_fut = async {
            loop {
                if cancel.has_armed_kill() {
                    break;
                }
                futures_lite::future::yield_now().await;
            }
            cancel.mark_cancelled();
        };
        let (result, _) = futures::join!(drain_fut, kill_fut);
        assert!(
            started.elapsed().as_secs() < 5,
            "build cancel must return promptly: {:?}",
            started.elapsed()
        );
        assert!(result.is_err() || result.is_ok_and(|(_, success)| !success));
    });
    assert_eq!(super::take_process_group_terminations(), 1);
}

#[test]
fn cancel_during_preflight_terminates_process_group_once() {
    use instant::Instant;

    use crate::terminal::view::dev_container::operation::DevContainerBuildCancel;

    let _ = super::take_process_group_terminations();
    let cancel = DevContainerBuildCancel::new();
    let started = Instant::now();
    block_on(async {
        let run_fut =
            super::run_cancellable_process_group_command(sleep_process_group_command(), &cancel);
        let kill_fut = async {
            loop {
                if cancel.has_armed_kill() {
                    break;
                }
                futures_lite::future::yield_now().await;
            }
            cancel.mark_cancelled();
        };
        let (result, _) = futures::join!(run_fut, kill_fut);
        assert!(
            started.elapsed().as_secs() < 5,
            "preflight cancel must return promptly: {:?}",
            started.elapsed()
        );
        assert!(result.is_err() || result.is_ok_and(|output| !output.status.success()));
    });
    assert_eq!(super::take_process_group_terminations(), 1);
}

#[test]
fn rejected_build_registration_terminates_before_wait() {
    use instant::Instant;

    use crate::terminal::view::dev_container::operation::DevContainerBuildCancel;

    let _ = super::take_process_group_terminations();
    let cancel = DevContainerBuildCancel::new();
    cancel.mark_cancelled();
    let started = Instant::now();
    let result = block_on(super::drain_dev_container_child(
        sleep_process_group_command(),
        Some(&cancel),
        |_| {},
    ));
    assert!(
        started.elapsed().as_secs() < 5,
        "rejected registration must not wait on the child: {:?}",
        started.elapsed()
    );
    assert!(result.is_err());
    assert_eq!(super::take_process_group_terminations(), 1);
}

#[test]
fn drain_marks_stdout_oversized_past_one_mib() {
    block_on(async {
        let mut command = command::r#async::Command::new("python3");
        command
            .arg("-c")
            .arg(format!(
                "import os; os.write(1, b'x' * {}); os.write(2, b'')",
                STDOUT_LIMIT + 1
            ))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().expect("spawn oversized child");
        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");
        let result = drain_dev_container_pipes(stdout, stderr, |_| {})
            .await
            .expect("drain");
        let _ = child.status().await;
        assert!(result.stdout.oversized);
        assert!(result.stdout.bytes.is_empty());
    });
}

#[cfg(unix)]
fn pid_is_alive(pid: i32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}

#[cfg(unix)]
fn wait_for_pid_file(path: &std::path::Path) -> i32 {
    use instant::Instant;

    let started = Instant::now();
    loop {
        if let Ok(contents) = std::fs::read_to_string(path)
            && let Ok(pid) = contents.trim().parse::<i32>()
            && pid > 1
        {
            return pid;
        }
        assert!(
            started.elapsed().as_secs() < 5,
            "descendant pid file was not written"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(unix)]
#[test]
fn reader_error_kills_process_group_descendants() {
    use command::r#async::Command;

    let pid_file = std::env::temp_dir().join(format!("dc-desc-{}", uuid::Uuid::new_v4()));
    block_on(async {
        let mut command = Command::new_with_process_group("python3");
        command
            .arg("-c")
            .arg(format!(
                r#"
import os, time
pid = os.fork()
if pid == 0:
    open({pid_file:?}, "w").write(str(os.getpid()))
    time.sleep(30)
    os._exit(0)
time.sleep(30)
"#
            ))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().expect("spawn");
        let process_group_id = child.id();
        let descendant = wait_for_pid_file(&pid_file);
        assert!(pid_is_alive(descendant), "descendant must start alive");
        let result = super::join_drain_and_status(
            super::ProcessGroupKillOnDrop::new(process_group_id),
            async { Err(io::Error::other("reader failed")) },
            async { child.status().await },
        )
        .await;
        assert!(result.is_err(), "reader failure must surface");
        let started = instant::Instant::now();
        while pid_is_alive(descendant) {
            assert!(
                started.elapsed().as_secs() < 5,
                "descendant must not survive a reader error"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });
    let _ = std::fs::remove_file(pid_file);
}

#[cfg(unix)]
#[test]
fn drain_reaches_failed_without_waiting_for_descendant_holding_pipes() {
    use command::r#async::Command;
    use instant::Instant;

    block_on(async {
        let mut command = Command::new_with_process_group("python3");
        command
            .arg("-c")
            .arg(
                r#"
import os, time
pid = os.fork()
if pid == 0:
    time.sleep(30)
    os._exit(0)
os.write(2, b"marker-before-exit\n")
os._exit(1)
"#,
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let started = Instant::now();
        let (drain, success) = super::drain_dev_container_child(command, None, |_| {})
            .await
            .expect("drain after parent exit");
        assert!(
            started.elapsed().as_secs() < 5,
            "descendant holding pipes must not pin drain: {:?}",
            started.elapsed()
        );
        assert!(!success);
        assert!(
            drain
                .stderr_tail
                .windows(b"marker-before-exit".len())
                .any(|window| window == b"marker-before-exit")
        );
    });
}

#[cfg(unix)]
#[test]
fn drain_reaches_success_without_waiting_for_descendant_holding_pipes() {
    use command::r#async::Command;
    use instant::Instant;

    block_on(async {
        let mut command = Command::new_with_process_group("python3");
        command
            .arg("-c")
            .arg(
                r#"
import os, time
pid = os.fork()
if pid == 0:
    time.sleep(30)
    os._exit(0)
os.write(2, b"Generated translation files for all integrations\n")
os.write(1, b'{"outcome":"success","containerId":"abc","remoteWorkspaceFolder":"/w"}\n')
os._exit(0)
"#,
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let started = Instant::now();
        let (drain, success) = super::drain_dev_container_child(command, None, |_| {})
            .await
            .expect("drain after successful parent exit");
        assert!(
            started.elapsed().as_secs() < 5,
            "descendant holding pipes must not pin drain after success: {:?}",
            started.elapsed()
        );
        assert!(success);
        assert!(
            drain
                .stderr_tail
                .windows(b"Generated translation files for all integrations".len())
                .any(|window| window == b"Generated translation files for all integrations")
        );
    });
}

#[test]
fn drain_stays_pending_while_child_is_silent_but_alive() {
    use command::r#async::Command;
    use futures::future::{self, Either};

    block_on(async {
        let mut command = Command::new_with_process_group("python3");
        command
            .arg("-c")
            .arg(
                r#"
import os, time, sys
sys.stderr.write("Generated translation files for all integrations\n")
sys.stderr.flush()
time.sleep(30)
"#,
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let drain = super::drain_dev_container_child(command, None, |_| {});
        let timeout = async {
            warpui::r#async::Timer::after(std::time::Duration::from_millis(400)).await;
        };
        match future::select(Box::pin(drain), Box::pin(timeout)).await {
            Either::Left(_) => {
                panic!("drain completed while the child was still alive")
            }
            Either::Right(_) => {}
        }
    });
}

#[test]
fn devcontainer_text_stream_renders_incrementally() {
    let mut model = TerminalModel::mock(None, None);
    model.start_commandless_output_block();
    let mut processor = Processor::new();
    let mut normalizer = NewlineNormalizer::new();
    let mut replies = Vec::new();
    let mut writer = WriteCapture(&mut replies);

    let delayed = b"step-one\n\x1b[31mred";
    let rest = b"-text\x1b[0m\nstep-two\n";
    for chunk in delayed.chunks(3) {
        let normalized = normalizer.push(chunk);
        processor.parse_bytes(&mut model, &normalized, &mut writer);
    }
    let output_so_far = model
        .block_list()
        .active_block()
        .output_grid()
        .contents_to_string(false, None);
    assert!(
        output_so_far.contains("step-one"),
        "delayed marker missing before remaining chunks: {output_so_far:?}"
    );

    let normalized_rest = normalizer.push(rest);
    processor.parse_bytes(&mut model, &normalized_rest, &mut writer);
    processor.parse_bytes(&mut model, b"\x1b[6nmore\n", &mut io::sink());
    let output = model
        .block_list()
        .active_block()
        .output_grid()
        .contents_to_string(false, None);
    assert!(output.contains("step-one"));
    assert!(output.contains("red-text") || output.contains("red") && output.contains("text"));
    assert!(output.contains("step-two"));
    assert!(output.contains("more"));
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        assert!(
            !line.starts_with(' '),
            "bare LF should left-align after normalization, got {line:?}"
        );
    }
}

#[test]
fn failure_details_with_bare_lf_render_left_aligned() {
    let mut model = TerminalModel::mock(None, None);
    model.start_commandless_output_block();
    let mut processor = Processor::new();
    let mut normalizer = NewlineNormalizer::new();
    let message = "Dev container failed to start:\nCommand failed: docker ps -q --filter \
         label=devcontainer.local_folder=/tmp/ws\nCannot connect to the Docker daemon";
    let bytes = normalizer.push(format!("\n{message}\n").as_bytes());
    processor.parse_bytes(&mut model, &bytes, &mut io::sink());
    let output = model
        .block_list()
        .active_block()
        .output_grid()
        .contents_to_string(false, None);
    for needle in ["Command failed", "Cannot connect"] {
        let line = output
            .lines()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle} missing from {output:?}"));
        assert_eq!(
            line.trim_start(),
            line,
            "failure details must start at column 0, got {line:?}"
        );
    }
}

fn jsonl_event(event_type: &str, text: &str) -> String {
    format!(
        "{}\n",
        serde_json::json!({
            "type": event_type,
            "level": 3,
            "timestamp": 1,
            "text": text,
        })
    )
}

fn wide_terminal_model() -> TerminalModel {
    let mut sizes = block_size();
    sizes.size = SizeInfo::new_without_font_metrics(24, 160);
    TerminalModel::new_for_test(
        sizes,
        color::List::from(&Colors::default()),
        ChannelEventListener::new_for_test(),
        Arc::new(Background::default()),
        false,
        None,
        false,
        false,
        None,
    )
}

fn grid_output_from_stderr_chunks(chunks: &[&[u8]]) -> String {
    let mut model = wide_terminal_model();
    model.start_commandless_output_block();
    let mut processor = Processor::new();
    let bytes = super::transform_dev_container_stderr(chunks);
    processor.parse_bytes(&mut model, &bytes, &mut io::sink());
    model
        .block_list()
        .active_block()
        .output_grid()
        .contents_to_string(false, None)
}

#[test]
fn raw_cr_progress_overwrites_in_the_grid() {
    let header = jsonl_event("text", "[cli] @devcontainers/cli 0.89.0");
    let first = jsonl_event("raw", "#15 extracting sha256:abc 1.5MB / 52.40MB");
    let last = jsonl_event("raw", "\r#15 extracting sha256:abc 52.40MB / 52.40MB");
    let done = jsonl_event("text", "#15 DONE 2.1s");
    let output = grid_output_from_stderr_chunks(&[
        header.as_bytes(),
        first.as_bytes(),
        last.as_bytes(),
        done.as_bytes(),
    ]);
    assert!(
        output.contains("@devcontainers/cli 0.89.0"),
        "ordinary logs must remain, got {output:?}"
    );
    assert!(
        output.contains("#15 DONE 2.1s"),
        "completed vertex lines must remain, got {output:?}"
    );
    let extracting_lines = output
        .lines()
        .filter(|line| line.contains("extracting sha256:abc"))
        .count();
    assert_eq!(
        extracting_lines, 1,
        "CR snapshots must overwrite in place, got {output:?}"
    );
    assert!(
        !output.contains("1.5MB"),
        "overwritten snapshots must not linger, got {output:?}"
    );
    assert!(output.contains("52.40MB / 52.40MB"));
}

#[test]
fn raw_cursor_up_progress_overwrites_in_the_grid() {
    let first = jsonl_event("raw", "layer-a 1MB\r\nlayer-b 1MB");
    let update = jsonl_event("raw", "\u{1b}[1A\rlayer-a 2MB");
    let output = grid_output_from_stderr_chunks(&[first.as_bytes(), update.as_bytes()]);
    assert!(
        output.contains("layer-a 2MB"),
        "cursor-up must apply the later snapshot, got {output:?}"
    );
    assert!(
        !output.contains("layer-a 1MB"),
        "superseded row must not linger, got {output:?}"
    );
    assert!(output.contains("layer-b 1MB"));
}

#[test]
fn drain_preserves_raw_cr_through_the_stream_boundary() {
    block_on(async {
        let mut command = command::r#async::Command::new("python3");
        command
            .arg("-c")
            .arg(
                r##"
import json, os
def emit(event_type, text):
    os.write(2, (json.dumps({"type": event_type, "level": 3, "timestamp": 1, "text": text}) + "\n").encode())
emit("text", "[cli] @devcontainers/cli 0.89.0")
emit("raw", "#15 extracting sha256:abc 1.5MB / 52.40MB")
emit("raw", "\r#15 extracting sha256:abc 52.40MB / 52.40MB")
emit("text", "#15 DONE 2.1s")
os.write(1, b'{"outcome":"success","containerId":"abc","remoteWorkspaceFolder":"/w"}\n')
"##,
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().expect("spawn jsonl child");
        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");
        let seen_stderr = Arc::new(Mutex::new(Vec::new()));
        let seen_for_callback = seen_stderr.clone();
        let result = drain_dev_container_pipes(stdout, stderr, move |chunk| {
            seen_for_callback.lock().unwrap().extend_from_slice(chunk);
        })
        .await
        .expect("drain");
        let status = child.status().await.expect("wait");
        assert!(status.success());
        assert!(String::from_utf8_lossy(&result.stdout.bytes).contains(r#""outcome":"success""#));

        let decoded = seen_stderr.lock().unwrap().clone();
        let mut model = wide_terminal_model();
        model.start_commandless_output_block();
        let mut processor = Processor::new();
        processor.parse_bytes(&mut model, &decoded, &mut io::sink());
        let output = model
            .block_list()
            .active_block()
            .output_grid()
            .contents_to_string(false, None);
        let extracting_lines = output
            .lines()
            .filter(|line| line.contains("extracting sha256:abc"))
            .count();
        assert_eq!(
            extracting_lines, 1,
            "stream-boundary CR must overwrite in the grid, got {output:?}"
        );
        assert!(!output.contains("1.5MB"), "got {output:?}");
        assert!(output.contains("52.40MB / 52.40MB"));
        assert!(output.contains("#15 DONE 2.1s"));
    });
}

#[test]
fn commandless_output_block_height_grows_with_later_batches() {
    use warpui::units::Lines;

    use crate::terminal::model::block::TranscriptScope;

    let mut model = TerminalModel::mock(None, None);
    model.start_commandless_output_block();
    let mut processor = Processor::new();

    // The mock screen is 7 rows. A later batch must still increase visible
    // height after that viewport is already full; nonzero height alone matches
    // the broken one-line-tall block.
    let first: String = (0..10).map(|i| format!("first-{i}\r\n")).collect();
    processor.parse_bytes(&mut model, first.as_bytes(), &mut io::sink());
    let height_after_first = model.block_list().block_heights().summary().height;
    assert!(
        height_after_first > Lines::zero(),
        "first batch must be visible, got {height_after_first:?}"
    );

    let later: String = (0..20).map(|i| format!("later-{i}\r\n")).collect();
    processor.parse_bytes(&mut model, later.as_bytes(), &mut io::sink());
    let height_after_later = model.block_list().block_heights().summary().height;
    assert!(
        height_after_later > height_after_first,
        "later batch must grow visible height from {height_after_first:?} to more than that, \
         got {height_after_later:?}"
    );
    assert!(
        model
            .block_list()
            .active_block()
            .is_visible(&TranscriptScope::Terminal)
    );
}

struct WriteCapture<'a>(&'a mut Vec<u8>);

impl io::Write for WriteCapture<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
