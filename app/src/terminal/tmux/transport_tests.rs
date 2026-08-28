use std::path::PathBuf;

use warp_terminal::shell::ShellType;

use super::{
    ControlTransportSpec, TmuxCommandError, in_place_tmux_cc_command, tmux_cc_argv,
    tmux_cc_shell_command,
};
use crate::terminal::tmux::protocol::pane_bootstrap_for_shell;

#[test]
fn in_place_command_attaches_existing_session_on_warp_socket() {
    let command = in_place_tmux_cc_command("warp-host-1", 80, 24);
    assert!(command.starts_with("tmux -CC -L warp-control-v1 new-session -A -s warp-host-1"));
    assert!(command.contains("-x 80"));
    assert!(command.contains("-y 24"));
    assert!(!command.contains(" -S "));
}

#[test]
fn attach_uses_dedicated_socket_not_the_default_server() {
    let command = tmux_cc_shell_command("attach -t api", None, 80, 24).unwrap();
    assert_eq!(
        command,
        "tmux -CC -L warp-control-v1 attach-session -t api\n"
    );
}

#[test]
fn user_socket_flags_are_rejected() {
    assert_eq!(
        tmux_cc_shell_command("-L default attach", None, 80, 24),
        Err(TmuxCommandError::IsolatedSocketOverride)
    );
    assert_eq!(
        tmux_cc_shell_command("-S /tmp/tmux.sock new-session", None, 80, 24),
        Err(TmuxCommandError::IsolatedSocketOverride)
    );
    assert_eq!(
        tmux_cc_shell_command("-Lother attach", None, 80, 24),
        Err(TmuxCommandError::IsolatedSocketOverride)
    );
    assert_eq!(
        tmux_cc_shell_command("-S/tmp/foo new-session", None, 80, 24),
        Err(TmuxCommandError::IsolatedSocketOverride)
    );
    assert_eq!(
        tmux_cc_shell_command("--socket-name default attach", None, 80, 24),
        Err(TmuxCommandError::IsolatedSocketOverride)
    );
    assert_eq!(
        tmux_cc_shell_command("--socket=/tmp/foo attach", None, 80, 24),
        Err(TmuxCommandError::IsolatedSocketOverride)
    );
    let command = tmux_cc_shell_command("attach -t api", None, 80, 24).unwrap();
    assert!(command.contains("-L warp-control-v1"));
}

#[test]
fn new_session_gets_size_and_quotes_unsafe_names() {
    let argv = tmux_cc_argv(
        &["new-session".into(), "-s".into(), "api prod".into()],
        "warp",
        120,
        40,
    )
    .unwrap();
    assert_eq!(
        argv,
        vec![
            "tmux",
            "-CC",
            "-L",
            "warp-control-v1",
            "new-session",
            "-s",
            "api prod",
            "-x",
            "120",
            "-y",
            "40",
        ]
    );
    let command = tmux_cc_shell_command("new-session -s 'api prod'", None, 120, 40).unwrap();
    assert!(command.contains("-L warp-control-v1"));
    assert!(command.contains("'api prod'"));
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
