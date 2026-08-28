use std::path::PathBuf;

use warp_terminal::shell::ShellType;

use super::{ControlTransportSpec, in_place_tmux_cc_command};
use crate::terminal::tmux::protocol::pane_bootstrap_for_shell;

#[test]
fn in_place_command_attaches_existing_session() {
    let command = in_place_tmux_cc_command("warp-host-1", 80, 24);
    assert!(command.starts_with("tmux -CC new-session -A -s warp-host-1"));
    assert!(command.contains("-x 80"));
    assert!(command.contains("-y 24"));
}

#[test]
fn local_dedicated_harness_still_targets_a_socket() {
    let bootstrap = pane_bootstrap_for_shell(PathBuf::from("/bin/zsh"), ShellType::Zsh);
    let spec = ControlTransportSpec::LocalDedicated {
        tmux_path: PathBuf::from("/usr/bin/tmux"),
        socket: PathBuf::from("/tmp/warp-1.sock"),
        config: PathBuf::from("/tmp/warp-1.conf"),
        bootstrap,
        columns: 80,
        rows: 24,
    };
    let argv: Vec<String> = spec
        .spawn_argv()
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(argv[0], "/usr/bin/tmux");
    assert!(argv.contains(&"-S".to_string()));
    assert!(argv.contains(&"/tmp/warp-1.sock".to_string()));
    assert!(argv.contains(&"-CC".to_string()));
}
