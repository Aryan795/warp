use std::io;
use std::path::Path;
use std::process::{Output, Stdio};
use std::sync::Arc;

use command::r#async::Command;
use futures_util::future::{try_join, try_join3};
use futures_util::io::AsyncReadExt;
use parking_lot::Mutex;

use super::newline::NewlineNormalizer;
use super::operation::DevContainerBuildCancel;

pub(crate) const STDOUT_LIMIT: usize = 1024 * 1024;
const STDERR_TAIL_LIMIT: usize = 8 * 1024;

pub(crate) struct DevContainerUpStdout {
    pub bytes: Vec<u8>,
    pub oversized: bool,
}

pub(crate) struct DevContainerDrain {
    pub stdout: DevContainerUpStdout,
    pub stderr_tail: Vec<u8>,
}

pub(crate) fn dev_container_up_command(
    cli: &Path,
    workspace_folder: &Path,
    config_file: &Path,
) -> Command {
    let mut command = Command::new_with_process_group(cli);
    command
        .arg("up")
        .arg("--workspace-folder")
        .arg(workspace_folder)
        .arg("--config")
        .arg(config_file)
        .arg("--log-format")
        .arg("text")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

pub(crate) fn terminate_process_group(process_group_id: u32) {
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

pub(crate) async fn drain_dev_container_child<F>(
    mut command: Command,
    cancel: Option<&DevContainerBuildCancel>,
    on_stderr: F,
) -> io::Result<(DevContainerDrain, bool)>
where
    F: FnMut(&[u8]) + Send,
{
    let mut child = command.spawn()?;
    let process_group_id = child.id();
    if let Some(cancel) = cancel
        && !cancel.register_process_group(process_group_id)
    {
        terminate_process_group(process_group_id);
        let _ = child.status().await;
        return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
    }
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("devcontainer up stdout was not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("devcontainer up stderr was not piped"))?;
    let drain_fut = drain_dev_container_pipes(stdout, stderr, on_stderr);
    let status_fut = async {
        let status = child.status().await?;
        terminate_process_group(process_group_id);
        io::Result::Ok(status)
    };
    let (drain, status): (DevContainerDrain, std::process::ExitStatus) =
        try_join(drain_fut, status_fut).await?;
    Ok((drain, status.success()))
}

pub(crate) async fn run_cancellable_process_group_command(
    mut command: Command,
    cancel: &DevContainerBuildCancel,
) -> io::Result<Output> {
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let process_group_id = child.id();
    if !cancel.register_process_group(process_group_id) {
        terminate_process_group(process_group_id);
        let _ = child.status().await;
        return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
    }
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("stdout was not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("stderr was not piped"))?;
    let stdout_fut = read_to_end(stdout);
    let stderr_fut = read_to_end(stderr);
    let status_fut = async {
        let status = child.status().await?;
        terminate_process_group(process_group_id);
        io::Result::Ok(status)
    };
    let (stdout, stderr, status): (Vec<u8>, Vec<u8>, std::process::ExitStatus) =
        try_join3(stdout_fut, stderr_fut, status_fut).await?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

async fn read_to_end<R>(mut reader: R) -> io::Result<Vec<u8>>
where
    R: futures_util::AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await?;
    Ok(bytes)
}

pub(crate) async fn drain_dev_container_pipes<R1, R2, F>(
    stdout: R1,
    stderr: R2,
    on_stderr: F,
) -> io::Result<DevContainerDrain>
where
    R1: futures_util::AsyncRead + Unpin,
    R2: futures_util::AsyncRead + Unpin,
    F: FnMut(&[u8]) + Send,
{
    let on_stderr = Arc::new(Mutex::new(on_stderr));
    let stdout_task = drain_stdout(stdout);
    let stderr_task = drain_stderr(stderr, on_stderr);
    let (stdout, stderr_tail) = try_join(stdout_task, stderr_task).await?;
    Ok(DevContainerDrain {
        stdout,
        stderr_tail,
    })
}

async fn drain_stdout<R>(mut stdout: R) -> io::Result<DevContainerUpStdout>
where
    R: futures_util::AsyncRead + Unpin,
{
    let mut buf = [0_u8; 8192];
    let mut bytes = Vec::new();
    let mut oversized = false;
    loop {
        let n = stdout.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        if oversized {
            continue;
        }
        if bytes.len() + n > STDOUT_LIMIT {
            oversized = true;
            bytes.clear();
            continue;
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    Ok(DevContainerUpStdout { bytes, oversized })
}

async fn drain_stderr<R, F>(mut stderr: R, on_stderr: Arc<Mutex<F>>) -> io::Result<Vec<u8>>
where
    R: futures_util::AsyncRead + Unpin,
    F: FnMut(&[u8]),
{
    let mut buf = [0_u8; 8192];
    let mut normalizer = NewlineNormalizer::new();
    let mut stderr_tail = Vec::new();
    loop {
        let n = stderr.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        append_bounded_tail(&mut stderr_tail, &buf[..n]);
        let normalized = normalizer.push(&buf[..n]);
        (on_stderr.lock())(&normalized);
    }
    let trailing = normalizer.finish();
    if !trailing.is_empty() {
        (on_stderr.lock())(&trailing);
    }
    Ok(stderr_tail)
}

fn append_bounded_tail(tail: &mut Vec<u8>, chunk: &[u8]) {
    tail.extend_from_slice(chunk);
    if tail.len() > STDERR_TAIL_LIMIT {
        let overflow = tail.len() - STDERR_TAIL_LIMIT;
        tail.drain(..overflow);
    }
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
