use std::path::PathBuf;

use warp_core::SessionId;
use warp_terminal::shell::ShellType;

use super::{
    PaneBootstrap, control_client_argv, kill_server_argv, kill_server_command,
    pane_bootstrap_for_shell, refresh_client_command, send_keys_commands, zsh_init_bytes,
};
use crate::terminal::tmux::parser::PaneId;

#[test]
fn refresh_client_uses_columns_x_rows() {
    assert_eq!(refresh_client_command(80, 24), "refresh-client -C 80x24\n");
}

#[test]
fn kill_server_argv_targets_dedicated_socket() {
    assert_eq!(kill_server_command(), "kill-server\n");
    let argv = kill_server_argv(
        PathBuf::from("/usr/bin/tmux").as_path(),
        PathBuf::from("/tmp/warp.sock").as_path(),
    );
    let argv: Vec<String> = argv
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        argv,
        vec!["/usr/bin/tmux", "-S", "/tmp/warp.sock", "kill-server"]
    );
}

#[test]
fn kill_dedicated_server_terminates_tmux_on_socket() {
    let Some(tmux_path) = super::resolve_tmux_binary() else {
        return;
    };
    let socket =
        std::env::temp_dir().join(format!("warp-tmux-kill-test-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let started = std::process::Command::new(&tmux_path)
        .args([
            "-S",
            socket.to_str().expect("socket utf8"),
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "warp-kill-test",
        ])
        .status()
        .expect("spawn dedicated tmux server");
    assert!(started.success());
    let listed = std::process::Command::new(&tmux_path)
        .args(["-S", socket.to_str().expect("socket utf8"), "list-sessions"])
        .status()
        .expect("list dedicated tmux sessions");
    assert!(listed.success());
    super::kill_dedicated_server(&socket);
    let listed_after = std::process::Command::new(&tmux_path)
        .args(["-S", socket.to_str().expect("socket utf8"), "list-sessions"])
        .status()
        .expect("list dedicated tmux sessions after kill");
    assert!(!listed_after.success());
    assert!(!socket.exists());
}

#[test]
fn send_keys_hex_encodes_control_and_printable_bytes() {
    let commands = send_keys_commands(&PaneId::from("%0"), b"a\x03\n");
    assert_eq!(commands, vec!["send-keys -t %0 -H 61 03 0a\n".to_owned()]);
}

#[test]
fn send_keys_chunks_large_payloads() {
    let bytes = vec![b'x'; 200];
    let commands = send_keys_commands(&PaneId::from("%3"), &bytes);
    assert_eq!(commands.len(), 2);
    assert!(commands[0].starts_with("send-keys -t %3 -H "));
    assert!(commands[0].ends_with('\n'));
    assert_eq!(commands[0].matches(' ').count(), 3 + 128);
    assert_eq!(commands[1].matches(' ').count(), 3 + 72);
}

#[test]
fn control_client_argv_starts_detached_control_mode_session() {
    let bootstrap = PaneBootstrap {
        session_id: SessionId::from(42),
        shell_type: ShellType::Bash,
        shell_path: PathBuf::from("/bin/bash"),
        args: vec!["-c".into(), "echo hi".into()],
        init_script: None,
    };
    let argv = control_client_argv(
        PathBuf::from("/usr/bin/tmux").as_path(),
        PathBuf::from("/tmp/warp.sock").as_path(),
        &bootstrap,
        80,
        24,
    );
    let argv: Vec<String> = argv
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        argv,
        vec![
            "/usr/bin/tmux",
            "-S",
            "/tmp/warp.sock",
            "-f",
            "/dev/null",
            "-CC",
            "new-session",
            "-s",
            "warp-42",
            "-n",
            "warp",
            "-x",
            "80",
            "-y",
            "24",
            "--",
            "/bin/bash",
            "-c",
            "echo hi",
        ]
    );
}

#[test]
fn bash_pane_bootstrap_embeds_session_id_in_args() {
    let bootstrap = pane_bootstrap_for_shell(PathBuf::from("/bin/bash"), ShellType::Bash);
    assert!(bootstrap.init_script.is_none());
    let joined = bootstrap
        .args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(joined.contains(&bootstrap.session_id.as_u64().to_string()));
}

#[test]
fn zsh_pane_bootstrap_keeps_init_script_for_send_keys() {
    let bootstrap = pane_bootstrap_for_shell(PathBuf::from("/bin/zsh"), ShellType::Zsh);
    let init_script = bootstrap.init_script.expect("zsh needs an init script");
    assert!(init_script.contains(&bootstrap.session_id.as_u64().to_string()));
    let bytes = zsh_init_bytes(&init_script, ShellType::Zsh);
    assert!(bytes.ends_with(b"\n"));
}
