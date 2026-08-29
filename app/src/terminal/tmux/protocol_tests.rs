use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use command::blocking::Command;
use instant::Instant;
use warp_core::SessionId;
use warp_terminal::shell::ShellType;

use super::{
    PaneBootstrap, capture_pane_command, cleanup_unspawned_dedicated_files, control_client_argv,
    detach_client_command, in_band_init_bytes, kill_pane_command, kill_server_argv,
    kill_server_command, kill_window_command, new_window_command, pane_bootstrap_for_shell,
    pipe_pane_journal_command, refresh_client_command, register_dedicated_server,
    resize_pane_command, schedule_kill_dedicated_server, select_pane_command,
    select_window_command, send_keys_commands, split_window_command, zsh_init_bytes,
};
use crate::terminal::tmux::parser::{PaneId, WindowId};

#[test]
fn refresh_client_uses_columns_x_rows() {
    assert_eq!(refresh_client_command(80, 24), "refresh-client -C 80x24\n");
}

#[test]
fn split_window_encodes_orientation_and_prints_pane_id() {
    let pane = PaneId::from("%0");
    assert_eq!(
        split_window_command(&pane, true),
        "split-window -h -t %0 -P -F '#{pane_id}'\n"
    );
    assert_eq!(
        split_window_command(&pane, false),
        "split-window -v -t %0 -P -F '#{pane_id}'\n"
    );
}

#[test]
fn pane_and_window_commands_target_tmux_ids() {
    let pane = PaneId::from("%1");
    let window = WindowId::from("@2");
    assert_eq!(select_pane_command(&pane), "select-pane -t %1\n");
    assert_eq!(kill_pane_command(&pane), "kill-pane -t %1\n");
    assert_eq!(
        resize_pane_command(&pane, 40, 12),
        "resize-pane -t %1 -x 40 -y 12\n"
    );
    assert_eq!(new_window_command(), "new-window -P -F '#{window_id}'\n");
    assert_eq!(select_window_command(&window), "select-window -t @2\n");
    assert_eq!(kill_window_command(&window), "kill-window -t @2\n");
    assert_eq!(detach_client_command(), "detach-client\n");
    assert_ne!(detach_client_command(), kill_server_command());
    assert_eq!(
        pipe_pane_journal_command(&pane, "/tmp/warp-%1.journal"),
        "pipe-pane -t %1 -O 'cat >> /tmp/warp-%1.journal'\n"
    );
    assert_eq!(capture_pane_command(&pane), "capture-pane -p -t %1\n");
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
fn successful_kill_helper_prunes_registered_socket() {
    let _guard = super::registry_test_lock();
    let script = unique_temp_path("warp-tmux-prune-no-server.sh");
    std::fs::write(
        &script,
        b"#!/bin/sh\necho 'no server running on /tmp/missing' >&2\nexit 1\n",
    )
    .expect("write prune no-server script");
    chmod_script(&script);
    let (socket, config) = write_placeholder_socket_and_config();
    let before = super::registered_dedicated_server_count();
    register_dedicated_server(socket.clone());
    assert!(super::spawn_detached_kill_helper(Some(&script), &socket));
    assert!(wait_until(
        || {
            !socket.exists()
                && !config.exists()
                && super::registered_dedicated_server_count() == before
        },
        Duration::from_secs(3)
    ));
    let list = std::fs::read_to_string(super::registry_list_path()).unwrap_or_default();
    assert!(!list.contains(&socket.to_string_lossy().into_owned()));
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

#[cfg(unix)]
#[test]
fn failed_kill_helper_keeps_unix_sockets_registered_for_retry() {
    let _guard = super::registry_test_lock();
    let script = unique_temp_path("warp-tmux-permission-retain.sh");
    std::fs::write(
        &script,
        b"#!/bin/sh\necho 'error connecting to /tmp/x (Permission denied)' >&2\nexit 1\n",
    )
    .expect("write permission retain script");
    chmod_script(&script);
    let (socket, config) = write_placeholder_socket_and_config();
    let before = super::registered_dedicated_server_count();
    register_dedicated_server(socket.clone());
    assert!(super::spawn_detached_kill_helper(Some(&script), &socket));
    assert!(wait_until(
        || {
            let list = std::fs::read_to_string(super::registry_list_path()).unwrap_or_default();
            socket.exists()
                && config.exists()
                && super::registered_dedicated_server_count() == before + 1
                && list.contains(&socket.to_string_lossy().into_owned())
        },
        Duration::from_secs(3)
    ));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(super::registered_dedicated_server_count(), before + 1);
    assert!(socket.exists());
    assert!(config.exists());
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(&config);
    let _ = std::fs::remove_file(&script);
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
    if super::app_exit_reaper_has_started() {
        assert!(super::ensure_app_exit_reaper_with(None));
        return;
    }
    assert!(!super::ensure_app_exit_reaper_with(None));
    assert!(!super::app_exit_reaper_has_started());
    let Some(tmux_path) = super::resolve_tmux_binary() else {
        return;
    };
    assert!(super::ensure_app_exit_reaper_with(Some(&tmux_path)));
    assert!(super::app_exit_reaper_has_started());
}

#[cfg(unix)]
#[test]
fn last_tab_exit_does_not_drop_sockets_before_cleanup_is_secured() {
    let _guard = super::registry_test_lock();
    let before = super::registered_dedicated_server_count();
    let socket = write_placeholder_socket();
    register_dedicated_server(socket.clone());
    assert_eq!(super::registered_dedicated_server_count(), before + 1);
    assert!(!super::spawn_detached_kill_helper(None, &socket));
    assert_eq!(super::registered_dedicated_server_count(), before + 1);
    super::schedule_kill_registered_dedicated_servers();
    assert_eq!(super::registered_dedicated_server_count(), before + 1);
    let list = std::fs::read_to_string(super::registry_list_path()).expect("read registry list");
    assert!(list.contains(&socket.to_string_lossy().into_owned()));
    schedule_kill_dedicated_server(socket.clone());
    let _ = std::fs::remove_file(&socket);
}

#[cfg(unix)]
#[test]
fn timed_out_helper_keeps_socket_registered_for_app_exit() {
    let _guard = super::registry_test_lock();
    let script = unique_temp_path("warp-tmux-hang-then-reaper.sh");
    std::fs::write(&script, b"#!/bin/sh\nexec sleep 30\n").expect("write hang helper");
    chmod_script(&script);
    let (socket, config) = write_placeholder_socket_and_config();
    let before = super::registered_dedicated_server_count();
    register_dedicated_server(socket.clone());
    let started = Instant::now();
    assert!(super::spawn_detached_kill_helper(Some(&script), &socket));
    assert!(started.elapsed() < Duration::from_millis(200));
    std::thread::sleep(Duration::from_millis(2500));
    assert!(socket.exists());
    assert!(config.exists());
    assert_eq!(super::registered_dedicated_server_count(), before + 1);
    let list = std::fs::read_to_string(super::registry_list_path()).expect("read registry list");
    assert!(list.contains(&socket.to_string_lossy().into_owned()));

    let reaper_tmux = unique_temp_path("warp-tmux-reaper-after-timeout.sh");
    std::fs::write(
        &reaper_tmux,
        b"#!/bin/sh\necho 'no server running on /tmp/missing' >&2\nexit 1\n",
    )
    .expect("write reaper tmux script");
    chmod_script(&reaper_tmux);
    let (parent_end, child_end) = std::os::unix::net::UnixStream::pair().expect("pipe");
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(super::APP_EXIT_REAPER_SCRIPT)
        .arg("tmux-control-prototype-exit-reaper")
        .arg(super::registry_list_path())
        .arg(&reaper_tmux)
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
        || !socket.exists() && !config.exists(),
        Duration::from_secs(3)
    ));
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&reaper_tmux);
}

#[test]
fn registry_persist_failure_does_not_mark_reaper_ready() {
    let _guard = super::registry_test_lock();
    let blocker = unique_temp_path("warp-tmux-registry-blocker");
    std::fs::write(&blocker, b"not-a-dir").expect("write blocker file");
    let path = blocker.join("active.list");
    let socket = write_placeholder_socket();
    let sockets = HashSet::from([socket.clone()]);
    assert!(super::persist_registry_list_at(&path, &sockets).is_err());
    #[cfg(unix)]
    {
        if !super::app_exit_reaper_has_started() {
            assert!(!super::ensure_app_exit_reaper_with(None));
            assert!(!super::app_exit_reaper_has_started());
        }
    }
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(&blocker);
}

#[test]
fn registry_list_filename_isolates_pid_reuse_with_token() {
    let name = super::registry_list_filename();
    let pid = std::process::id().to_string();
    assert!(name.starts_with(&format!("active-{pid}-")));
    assert!(name.ends_with(".list"));
    assert_ne!(name, format!("active-{pid}.list"));
}

#[cfg(unix)]
#[test]
fn app_exit_reaper_ignores_same_pid_list_without_instance_token() {
    let script = unique_temp_path("warp-tmux-token-reaper.sh");
    std::fs::write(
        &script,
        b"#!/bin/sh\necho 'no server running on /tmp/missing' >&2\nexit 1\n",
    )
    .expect("write token reaper tmux");
    chmod_script(&script);
    let (ours, ours_conf) = write_placeholder_socket_and_config();
    let (theirs, theirs_conf) = write_placeholder_socket_and_config();
    let tokenized = unique_temp_path("warp-tmux-tokenized.list");
    let colliding = unique_temp_path(&format!("active-{}.list", std::process::id()));
    std::fs::write(&tokenized, format!("{}\n", ours.display())).expect("write tokenized list");
    std::fs::write(&colliding, format!("{}\n", theirs.display())).expect("write colliding list");
    let (parent_end, child_end) = std::os::unix::net::UnixStream::pair().expect("pipe");
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(super::APP_EXIT_REAPER_SCRIPT)
        .arg("tmux-control-prototype-exit-reaper")
        .arg(&tokenized)
        .arg(&script)
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
        || !ours.exists() && !ours_conf.exists(),
        Duration::from_secs(3)
    ));
    assert!(theirs.exists());
    assert!(theirs_conf.exists());
    let _ = std::fs::remove_file(&theirs);
    let _ = std::fs::remove_file(&theirs_conf);
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&colliding);
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

#[test]
fn in_band_init_bytes_are_remote_safe_and_session_scoped() {
    let session_id = SessionId::from(7);
    let bytes = in_band_init_bytes(ShellType::Zsh, session_id).expect("zsh init");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("7"));
    assert!(text.contains("InitShell"));
    assert!(bytes.ends_with(b"\n"));
    let bash = in_band_init_bytes(ShellType::Bash, session_id).expect("bash init");
    assert_ne!(bytes, bash);
}

#[test]
fn bash_in_band_init_disables_history_before_interactive_body() {
    let bytes = in_band_init_bytes(ShellType::Bash, SessionId::from(3)).expect("bash init");
    let text = String::from_utf8_lossy(&bytes);
    let history_off = text.find("set +o history").expect("history off");
    let stty = text.find("stty raw").expect("stty");
    assert!(history_off < stty);
    assert!(text[history_off..stty].contains("HISTCONTROL=ignorespace"));
    let zsh = in_band_init_bytes(ShellType::Zsh, SessionId::from(3)).expect("zsh init");
    let zsh_text = String::from_utf8_lossy(&zsh);
    assert!(zsh_text.contains("hist_ignore_space"));
    let fish = in_band_init_bytes(ShellType::Fish, SessionId::from(3)).expect("fish init");
    assert!(String::from_utf8_lossy(&fish).contains("fish_history"));
}
