use std::ffi::OsString;
use std::path::{Path, PathBuf};

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

pub fn kill_session_command() -> &'static str {
    "kill-session\n"
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
