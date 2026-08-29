use std::io;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use futures_lite::future::block_on;
use warp_terminal::model::ansi::Processor;

use super::{STDOUT_LIMIT, drain_dev_container_pipes};
use crate::terminal::model::terminal_model::TerminalModel;
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
