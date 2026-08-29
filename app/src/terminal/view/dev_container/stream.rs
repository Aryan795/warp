use std::io;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use command::r#async::Command;
use futures_util::future::try_join;
use futures_util::io::AsyncReadExt;
use parking_lot::Mutex;

use super::newline::NewlineNormalizer;

pub(crate) const STDOUT_LIMIT: usize = 1024 * 1024;

pub(crate) struct DevContainerUpStdout {
    pub bytes: Vec<u8>,
    pub oversized: bool,
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

pub(crate) async fn drain_dev_container_pipes<R1, R2, F>(
    stdout: R1,
    stderr: R2,
    on_stderr: F,
) -> io::Result<DevContainerUpStdout>
where
    R1: futures_util::AsyncRead + Unpin,
    R2: futures_util::AsyncRead + Unpin,
    F: FnMut(&[u8]) + Send,
{
    let on_stderr = Arc::new(Mutex::new(on_stderr));
    let stdout_task = drain_stdout(stdout);
    let stderr_task = drain_stderr(stderr, on_stderr);
    let (stdout, ()) = try_join(stdout_task, stderr_task).await?;
    Ok(stdout)
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

async fn drain_stderr<R, F>(mut stderr: R, on_stderr: Arc<Mutex<F>>) -> io::Result<()>
where
    R: futures_util::AsyncRead + Unpin,
    F: FnMut(&[u8]),
{
    let mut buf = [0_u8; 8192];
    let mut normalizer = NewlineNormalizer::new();
    loop {
        let n = stderr.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        let normalized = normalizer.push(&buf[..n]);
        (on_stderr.lock())(&normalized);
    }
    let trailing = normalizer.finish();
    if !trailing.is_empty() {
        (on_stderr.lock())(&trailing);
    }
    Ok(())
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
