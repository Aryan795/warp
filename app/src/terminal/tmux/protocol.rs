use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use warp_core::SessionId;
use warp_core::paths::cache_dir;
use warp_terminal::bootstrap::{generate_session_id, init_shell_script_for_shell};
use warp_terminal::local_tty::shell::{
    DirectShellStarter, arguments_for_session_spawning_command, supported_shell_path_and_type,
};
use warp_terminal::shell::ShellType;
use warp_util::path::resolve_executable;

use super::parser::PaneId;
use crate::ASSETS;
use crate::terminal::available_shells::AvailableShell;
use crate::terminal::shell::ShellLaunchData;

const SEND_KEYS_CHUNK_BYTES: usize = 128;
const KILL_SERVER_TIMEOUT: Duration = Duration::from_secs(2);
const KILL_SERVER_WAIT_SLICE: Duration = Duration::from_millis(10);

/// Bootstrap data for the Warp-managed pane process (not the control client).
#[derive(Debug, Clone)]
pub struct PaneBootstrap {
    pub session_id: SessionId,
    pub shell_type: ShellType,
    pub shell_path: PathBuf,
    pub args: Vec<OsString>,
    pub init_script: Option<String>,
}

impl PaneBootstrap {
    pub fn command_argv(&self) -> Vec<OsString> {
        let mut argv = Vec::with_capacity(1 + self.args.len());
        argv.push(self.shell_path.clone().into());
        argv.extend(self.args.iter().cloned());
        argv
    }
}

/// Build a Warp-bootstrapped pane command for the user's preferred supported shell.
pub fn pane_bootstrap_for_available_shell(
    preferred_shell: AvailableShell,
) -> Option<PaneBootstrap> {
    let launch_data = preferred_shell.get_valid_shell_path_and_type()?;
    let (shell_path, shell_type) = match launch_data {
        ShellLaunchData::Executable {
            executable_path,
            shell_type,
        } => (executable_path, shell_type),
        ShellLaunchData::WSL { .. }
        | ShellLaunchData::MSYS2 { .. }
        | ShellLaunchData::DockerSandbox { .. } => return None,
    };
    Some(pane_bootstrap_for_shell(shell_path, shell_type))
}

pub fn pane_bootstrap_for_shell(shell_path: PathBuf, shell_type: ShellType) -> PaneBootstrap {
    let session_id = generate_session_id();
    let args = arguments_for_session_spawning_command(
        shell_path.to_string_lossy().as_ref(),
        shell_type,
        session_id,
    );
    let init_script = matches!(shell_type, ShellType::Zsh)
        .then(|| init_shell_script_for_shell(shell_type, &ASSETS, session_id));
    PaneBootstrap {
        session_id,
        shell_type,
        shell_path,
        args,
        init_script,
    }
}

/// Dedicated tmux server socket under Warp's cache directory.
pub fn dedicated_socket_path(session_id: SessionId) -> PathBuf {
    cache_dir()
        .join("tmux-control-prototype")
        .join(format!("warp-{}.sock", session_id.as_u64()))
}

pub fn resolve_tmux_binary() -> Option<PathBuf> {
    resolve_executable("tmux").map(|path| path.into_owned())
}

/// `tmux` argv that starts a dedicated control-mode server and one Warp-bootstrapped pane.
pub fn control_client_argv(
    tmux_path: &Path,
    socket: &Path,
    bootstrap: &PaneBootstrap,
    columns: usize,
    rows: usize,
) -> Vec<OsString> {
    let mut argv = vec![
        tmux_path.as_os_str().to_owned(),
        "-S".into(),
        socket.as_os_str().to_owned(),
        "-f".into(),
        "/dev/null".into(),
        "-CC".into(),
        "new-session".into(),
        "-s".into(),
        format!("warp-{}", bootstrap.session_id.as_u64()).into(),
        "-n".into(),
        "warp".into(),
        "-x".into(),
        columns.to_string().into(),
        "-y".into(),
        rows.to_string().into(),
        "--".into(),
    ];
    argv.extend(bootstrap.command_argv());
    argv
}

pub fn tmux_shell_starter(
    argv: Vec<OsString>,
    session_id: SessionId,
) -> Option<DirectShellStarter> {
    let mut argv = argv.into_iter();
    let tmux_path = PathBuf::from(argv.next()?);
    Some(DirectShellStarter::new(
        ShellType::Bash,
        tmux_path,
        argv.collect(),
        session_id,
    ))
}

pub fn refresh_client_command(columns: usize, rows: usize) -> String {
    format!("refresh-client -C {columns}x{rows}\n")
}

/// Tear down the dedicated Warp tmux server. `kill-session` can leave that
/// server process alive on this socket after the control client detaches.
pub fn kill_server_command() -> &'static str {
    "kill-server\n"
}

pub fn kill_server_argv(tmux_path: &Path, socket: &Path) -> Vec<OsString> {
    vec![
        tmux_path.as_os_str().to_owned(),
        "-S".into(),
        socket.as_os_str().to_owned(),
        "kill-server".into(),
    ]
}

#[derive(Debug)]
enum KillDedicatedServerError {
    TmuxNotFound,
    Io(io::Error),
    NonZeroExit(ExitStatus),
    TimedOut,
}

impl std::fmt::Display for KillDedicatedServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TmuxNotFound => write!(f, "tmux binary not found"),
            Self::Io(err) => write!(f, "{err}"),
            Self::NonZeroExit(status) => write!(f, "tmux kill-server failed: {status}"),
            Self::TimedOut => write!(f, "tmux kill-server timed out"),
        }
    }
}

/// Out-of-band teardown if the control-client write never lands.
/// Unlinks the socket only after `tmux kill-server` succeeds, so a failed kill
/// cannot hide a still-running dedicated server.
pub fn kill_dedicated_server(socket: &Path) {
    kill_dedicated_server_with(
        resolve_tmux_binary().as_deref(),
        socket,
        KILL_SERVER_TIMEOUT,
    );
}

fn kill_dedicated_server_with(tmux_path: Option<&Path>, socket: &Path, timeout: Duration) {
    match try_kill_dedicated_server(tmux_path, socket, timeout) {
        Ok(()) => {
            if let Err(err) = std::fs::remove_file(socket)
                && err.kind() != io::ErrorKind::NotFound
            {
                log::warn!("failed to remove tmux socket {}: {err}", socket.display());
            }
        }
        Err(err) => {
            log::error!(
                "leaving tmux socket {} in place after kill-server failure: {err}",
                socket.display()
            );
        }
    }
}

/// Run [`kill_dedicated_server`] off the UI thread so tab close cannot stall.
pub fn schedule_kill_dedicated_server(socket: PathBuf) {
    if let Err(err) = std::thread::Builder::new()
        .name("tmux-control-prototype-kill-server".into())
        .spawn(move || kill_dedicated_server(&socket))
    {
        log::error!("failed to spawn tmux kill-server worker: {err}");
    }
}

fn try_kill_dedicated_server(
    tmux_path: Option<&Path>,
    socket: &Path,
    timeout: Duration,
) -> Result<(), KillDedicatedServerError> {
    let Some(tmux_path) = tmux_path else {
        return Err(KillDedicatedServerError::TmuxNotFound);
    };
    let argv = kill_server_argv(tmux_path, socket);
    let mut command = std::process::Command::new(&argv[0]);
    command.args(&argv[1..]);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    let child = command.spawn().map_err(KillDedicatedServerError::Io)?;
    let status = wait_child_with_timeout(child, timeout)?;
    if status.success() {
        Ok(())
    } else {
        Err(KillDedicatedServerError::NonZeroExit(status))
    }
}

fn wait_child_with_timeout(
    mut child: Child,
    timeout: Duration,
) -> Result<ExitStatus, KillDedicatedServerError> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(KillDedicatedServerError::TimedOut);
                }
                std::thread::sleep(KILL_SERVER_WAIT_SLICE);
            }
            Err(err) => return Err(KillDedicatedServerError::Io(err)),
        }
    }
}

/// Encode pane input as `send-keys -H` so arbitrary bytes never pass through tmux key-name parsing.
pub fn send_keys_commands(pane_id: &PaneId, bytes: &[u8]) -> Vec<String> {
    if bytes.is_empty() {
        return Vec::new();
    }
    bytes
        .chunks(SEND_KEYS_CHUNK_BYTES)
        .map(|chunk| {
            let mut command = format!("send-keys -t {} -H", pane_id.as_str());
            for byte in chunk {
                command.push_str(&format!(" {byte:02x}"));
            }
            command.push('\n');
            command
        })
        .collect()
}

pub fn zsh_init_bytes(init_script: &str, shell_type: ShellType) -> Vec<u8> {
    let mut bytes = init_script.as_bytes().to_vec();
    bytes.extend_from_slice(shell_type.execute_command_bytes());
    bytes
}

pub fn fallback_supported_shell() -> Option<(PathBuf, ShellType)> {
    ["zsh", "bash", "fish"]
        .into_iter()
        .find_map(supported_shell_path_and_type)
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
