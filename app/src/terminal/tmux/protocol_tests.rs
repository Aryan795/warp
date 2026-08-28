use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use command::blocking::Command;
use instant::Instant;
use warp_core::SessionId;
use warp_terminal::shell::ShellType;

use super::{
    PaneBootstrap, cleanup_unspawned_dedicated_files, control_client_argv, kill_server_argv,
    kill_server_command, pane_bootstrap_for_shell, refresh_client_command,
    register_dedicated_server, schedule_kill_dedicated_server, send_keys_commands, zsh_init_bytes,
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

static TEST_PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_temp_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        TEST_PATH_COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

fn write_placeholder_socket() -> PathBuf {
    let socket = unique_temp_path("warp-tmux-kill-test.sock");
    std::fs::write(&socket, b"keep-me").expect("write placeholder socket");
    socket
}

fn write_placeholder_socket_and_config() -> (PathBuf, PathBuf) {
    let socket = unique_temp_path("warp-tmux-stale").with_extension("sock");
    let config = socket.with_extension("conf");
    std::fs::write(&socket, b"keep-me").expect("write placeholder socket");
    std::fs::write(&config, b"keep-me").expect("write placeholder config");
    (socket, config)
}

fn wait_until(pred: impl Fn() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if pred() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    pred()
}

#[cfg(unix)]
fn chmod_script(script: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let mut permissions = std::fs::metadata(script)
        .expect("script metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(script, permissions).expect("chmod script");
}

#[test]
fn kill_dedicated_server_terminates_tmux_on_socket() {
    let Some(tmux_path) = super::resolve_tmux_binary() else {
        return;
    };
    let socket = unique_temp_path("warp-tmux-kill-test.sock");
    let _ = std::fs::remove_file(&socket);
    let started = Command::new(&tmux_path)
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
    let listed = Command::new(&tmux_path)
        .args(["-S", socket.to_str().expect("socket utf8"), "list-sessions"])
        .status()
        .expect("list dedicated tmux sessions");
    assert!(listed.success());
    super::kill_dedicated_server(&socket);
    let listed_after = Command::new(&tmux_path)
        .args(["-S", socket.to_str().expect("socket utf8"), "list-sessions"])
        .status()
        .expect("list dedicated tmux sessions after kill");
    assert!(!listed_after.success());
    assert!(!socket.exists());
}

#[test]
fn kill_dedicated_server_preserves_socket_when_tmux_is_missing() {
    let socket = write_placeholder_socket();
    super::kill_dedicated_server_with(None, &socket, Duration::from_secs(1));
    assert!(socket.exists());
    let _ = std::fs::remove_file(&socket);
}

#[test]
fn kill_dedicated_server_preserves_socket_when_kill_fails() {
    let false_bin = Path::new("/bin/false");
    if !false_bin.exists() {
        return;
    }
    let socket = write_placeholder_socket();
    super::kill_dedicated_server_with(Some(false_bin), &socket, Duration::from_secs(1));
    assert!(socket.exists());
    let _ = std::fs::remove_file(&socket);
}

#[test]
fn kill_dedicated_server_removes_socket_when_no_server_is_running() {
    let script = unique_temp_path("warp-tmux-no-server.sh");
    std::fs::write(
        &script,
        b"#!/bin/sh\necho 'no server running on /tmp/missing' >&2\nexit 1\n",
    )
    .expect("write no-server script");
    #[cfg(unix)]
    chmod_script(&script);
    let socket = write_placeholder_socket();
    let config = socket.with_extension("conf");
    std::fs::write(&config, b"keep-me").expect("write placeholder config");
    super::kill_dedicated_server_with(Some(&script), &socket, Duration::from_secs(1));
    assert!(!socket.exists());
    assert!(!config.exists());
    let _ = std::fs::remove_file(&script);
}

#[test]
fn kill_dedicated_server_preserves_socket_when_kill_times_out() {
    let script = unique_temp_path("warp-tmux-hang-kill-server.sh");
    std::fs::write(&script, b"#!/bin/sh\nexec sleep 30\n").expect("write hang script");
    #[cfg(unix)]
    chmod_script(&script);
    let socket = write_placeholder_socket();
    let started = Instant::now();
    super::kill_dedicated_server_with(Some(&script), &socket, Duration::from_millis(80));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(socket.exists());
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(&script);
}

#[test]
fn schedule_kill_dedicated_server_does_not_block_caller() {
    let socket = write_placeholder_socket();
    let started = Instant::now();
    schedule_kill_dedicated_server(socket.clone());
    assert!(started.elapsed() < Duration::from_millis(200));
    std::thread::sleep(Duration::from_millis(50));
    let _ = std::fs::remove_file(&socket);
}

#[test]
fn kill_dedicated_server_preserves_socket_on_permission_error() {
    let script = unique_temp_path("warp-tmux-permission.sh");
    std::fs::write(
        &script,
        b"#!/bin/sh\necho 'error connecting to /tmp/x (Permission denied)' >&2\nexit 1\n",
    )
    .expect("write permission script");
    #[cfg(unix)]
    chmod_script(&script);
    let (socket, config) = write_placeholder_socket_and_config();
    super::kill_dedicated_server_with(Some(&script), &socket, Duration::from_secs(1));
    assert!(socket.exists());
    assert!(config.exists());
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(&config);
    let _ = std::fs::remove_file(&script);
}

#[test]
fn cleanup_unspawned_dedicated_files_removes_socket_and_config() {
    let (socket, config) = write_placeholder_socket_and_config();
    cleanup_unspawned_dedicated_files(&socket);
    assert!(!socket.exists());
    assert!(!config.exists());
}

#[cfg(unix)]
#[test]
fn detached_kill_helper_unlinks_socket_and_config_when_server_is_gone() {
    let script = unique_temp_path("warp-tmux-detached-no-server.sh");
    std::fs::write(
        &script,
        b"#!/bin/sh\necho 'no server running on /tmp/missing' >&2\nexit 1\n",
    )
    .expect("write detached no-server script");
    chmod_script(&script);
    let (socket, config) = write_placeholder_socket_and_config();
    super::spawn_detached_kill_helper(Some(&script), &socket);
    assert!(wait_until(
        || !socket.exists() && !config.exists(),
        Duration::from_secs(3)
    ));
    let _ = std::fs::remove_file(&script);
}

#[cfg(unix)]
#[test]
fn detached_kill_helper_times_out_hanging_tmux_and_keeps_files() {
    let script = unique_temp_path("warp-tmux-hang-helper.sh");
    std::fs::write(&script, b"#!/bin/sh\nexec sleep 30\n").expect("write hang helper");
    chmod_script(&script);
    let (socket, config) = write_placeholder_socket_and_config();
    let started = Instant::now();
    super::spawn_detached_kill_helper(Some(&script), &socket);
    assert!(started.elapsed() < Duration::from_millis(200));
    std::thread::sleep(Duration::from_millis(2500));
    assert!(started.elapsed() < Duration::from_secs(4));
    assert!(socket.exists());
    assert!(config.exists());
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(&config);
    let _ = std::fs::remove_file(&script);
}

#[test]
fn register_then_kill_does_not_accumulate_tracked_sockets() {
    let _guard = super::registry_test_lock();
    let before = super::registered_dedicated_server_count();
    let sockets: Vec<_> = (0..3).map(|_| write_placeholder_socket()).collect();
    for socket in &sockets {
        register_dedicated_server(socket.clone());
    }
    assert_eq!(super::registered_dedicated_server_count(), before + 3);
    for socket in &sockets {
        schedule_kill_dedicated_server(socket.clone());
    }
    assert_eq!(super::registered_dedicated_server_count(), before);
    for socket in sockets {
        let _ = std::fs::remove_file(socket);
    }
}

#[test]
fn registry_list_contains_only_this_process_registered_sockets() {
    let _guard = super::registry_test_lock();
    let foreign = super::dedicated_server_dir().join("warp-foreign.sock");
    let _ = std::fs::create_dir_all(super::dedicated_server_dir());
    std::fs::write(&foreign, b"other-process").expect("write foreign socket");
    let socket = write_placeholder_socket();
    register_dedicated_server(socket.clone());
    let list = std::fs::read_to_string(super::registry_list_path()).expect("read registry list");
    let socket_s = socket.to_string_lossy().into_owned();
    let foreign_s = foreign.to_string_lossy().into_owned();
    assert!(list.contains(&socket_s));
    assert!(!list.contains(&foreign_s));
    assert!(
        super::registry_list_path()
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains(&std::process::id().to_string()))
    );
    schedule_kill_dedicated_server(socket.clone());
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(&foreign);
}

#[cfg(unix)]
#[test]
fn app_exit_reaper_script_kills_listed_sockets_after_stdin_eof() {
    let script = unique_temp_path("warp-tmux-reaper-tmux.sh");
    std::fs::write(
        &script,
        b"#!/bin/sh\necho 'no server running on /tmp/missing' >&2\nexit 1\n",
    )
    .expect("write reaper tmux script");
    chmod_script(&script);
    let (listed, listed_conf) = write_placeholder_socket_and_config();
    let (unlisted, unlisted_conf) = write_placeholder_socket_and_config();
    let list = unique_temp_path("warp-tmux-reaper.list");
    std::fs::write(&list, format!("{}\n", listed.display())).expect("write list");
    let (parent_end, child_end) = std::os::unix::net::UnixStream::pair().expect("pipe");
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(super::APP_EXIT_REAPER_SCRIPT)
        .arg("tmux-control-prototype-exit-reaper")
        .arg(&list)
        .arg(&script)
        // SAFETY: child_end is the unique owner of this socket.
        .stdin(unsafe {
            use std::os::fd::{FromRawFd as _, IntoRawFd as _};
            Stdio::from_raw_fd(child_end.into_raw_fd())
        })
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn reaper script");
    drop(parent_end);
    let _ = child.wait();
    assert!(wait_until(
        || !listed.exists() && !listed_conf.exists(),
        Duration::from_secs(3)
    ));
    assert!(unlisted.exists());
    assert!(unlisted_conf.exists());
    let _ = std::fs::remove_file(&unlisted);
    let _ = std::fs::remove_file(&unlisted_conf);
    let _ = std::fs::remove_file(&script);
}

#[cfg(unix)]
#[test]
fn spawn_app_exit_reaper_marks_started_only_on_success() {
    let _guard = super::registry_test_lock();
    super::reset_app_exit_reaper_started();
    assert!(!super::app_exit_reaper_has_started());
    let Some(tmux_path) = super::resolve_tmux_binary() else {
        return;
    };
    let list = unique_temp_path("warp-tmux-started.list");
    std::fs::write(&list, b"").expect("write empty list");
    assert!(super::spawn_app_exit_reaper(&list, &tmux_path));
    super::reset_app_exit_reaper_started();
    assert!(!super::app_exit_reaper_has_started());
    let socket = write_placeholder_socket();
    super::register_dedicated_server(socket.clone());
    assert!(super::app_exit_reaper_has_started());
    super::schedule_kill_dedicated_server(socket.clone());
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(&list);
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
        PathBuf::from("/tmp/warp.conf").as_path(),
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
            "/tmp/warp.conf",
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
