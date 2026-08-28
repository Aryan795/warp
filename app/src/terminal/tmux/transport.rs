use std::ffi::OsString;
use std::path::PathBuf;

use super::protocol::{PaneBootstrap, control_client_argv};

/// How a tmux `-CC` byte stream is obtained.
///
/// Product path is in-place on the **current** shell PTY: the user (or `/tmux`) runs
/// `tmux -CC` in the active local or already-remote shell, the event loop detects
/// `DCS 1000p`, and that same writable PTY becomes the control stream. Locality is
/// implicit in whichever shell is attached.
///
/// [`Self::LocalDedicated`] is only a feature-flagged test harness that spawns a
/// private socket. It must not define product ownership, and a sibling SSH
/// ControlMaster exec is not the remote product path.
#[derive(Debug, Clone)]
pub enum ControlTransportSpec {
    /// Test harness: `tmux -CC` on a Warp-owned local socket.
    LocalDedicated {
        tmux_path: PathBuf,
        socket: PathBuf,
        config: PathBuf,
        bootstrap: PaneBootstrap,
        columns: usize,
        rows: usize,
    },
}

impl ControlTransportSpec {
    pub fn spawn_argv(&self) -> Vec<OsString> {
        match self {
            Self::LocalDedicated {
                tmux_path,
                socket,
                config,
                bootstrap,
                columns,
                rows,
            } => control_client_argv(tmux_path, socket, config, bootstrap, *columns, *rows),
        }
    }
}

/// Shell command written to the **active** PTY to enter or resume control mode.
///
/// `-A` attaches if `session_name` already exists so SSH reconnect can rediscover
/// the Warp-managed session instead of creating a second one.
pub fn in_place_tmux_cc_command(session_name: &str, columns: usize, rows: usize) -> String {
    format!("tmux -CC new-session -A -s {session_name} -n warp -x {columns} -y {rows}\n")
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
