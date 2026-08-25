use super::*;

fn shell_starter(shell_type: ShellType, shell_path: &str) -> DirectShellStarter {
    DirectShellStarter::new_for_test(shell_type, PathBuf::from(shell_path), Vec::new())
}

fn dev_container_starter(remote_user: Option<&str>) -> DevContainerShellStarter {
    DevContainerShellStarter::new(
        shell_starter(ShellType::Bash, "docker"),
        PathBuf::from("/home/user/project"),
        "abc123".to_owned(),
        remote_user.map(str::to_owned),
        "/workspaces/project".to_owned(),
        "deadbeef".to_owned(),
    )
}

fn env_value(command: &Command, key: &str) -> Option<Option<String>> {
    command
        .get_envs()
        .find(|(env_key, _)| *env_key == std::ffi::OsStr::new(key))
        .map(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()))
}

#[test]
fn host_bash_command_sets_history_size_sentinels() {
    let command = build_host_shell_command(
        shell_starter(ShellType::Bash, "/bin/bash"),
        None,
        HashMap::new(),
        None,
        false,
        false,
        false,
        false,
        true,
    );

    assert_eq!(
        env_value(&command, "HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
}

#[test]
fn host_non_bash_command_does_not_set_history_size_sentinels() {
    let command = build_host_shell_command(
        shell_starter(ShellType::Zsh, "/bin/zsh"),
        None,
        HashMap::new(),
        None,
        false,
        false,
        false,
        false,
        true,
    );

    assert_eq!(env_value(&command, "HISTFILESIZE"), None);
    assert_eq!(env_value(&command, "HISTSIZE"), None);
    assert_eq!(env_value(&command, "WARP_INITIAL_HISTFILESIZE"), None);
    assert_eq!(env_value(&command, "WARP_INITIAL_HISTSIZE"), None);
}

#[test]
fn docker_sandbox_command_sets_history_size_sentinels() {
    let docker_starter =
        DockerSandboxShellStarter::new(shell_starter(ShellType::Bash, "sbx"), None);
    let command = build_docker_sandbox_command(
        &docker_starter,
        None,
        HashMap::new(),
        false,
        false,
        false,
        false,
        true,
    );

    assert_eq!(
        env_value(&command, "HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
}

#[test]
fn dev_container_exec_args_relay_over_a_plain_pipe() {
    let starter = dev_container_starter(None);
    let args = dev_container_exec_args(&starter);

    // `-i` without `-t`: the pty lives inside the container (via `script`),
    // not on the `docker exec` relay itself. See `prepare_dev_container` for
    // why that's load-bearing for handshake delivery.
    assert_eq!(args[0], "exec");
    assert_eq!(args[1], "-i");
    assert!(!args.iter().any(|arg| arg == "-it" || arg == "-t"));
}

#[test]
fn dev_container_exec_args_wraps_bash_in_script_with_quoted_init_path() {
    let starter = dev_container_starter(None);
    let args = dev_container_exec_args(&starter);

    let script_pos = args
        .iter()
        .position(|arg| arg == "script")
        .expect("args should invoke `script` inside the container");
    assert_eq!(args[script_pos + 1], "-E");
    assert_eq!(args[script_pos + 2], "never");
    assert_eq!(args[script_pos + 3], "-qfec");
    assert_eq!(
        args[script_pos + 4],
        std::ffi::OsString::from(format!(
            "exec bash --rcfile '{}' --noprofile",
            starter.container_init_script_path()
        ))
    );
    assert_eq!(args[script_pos + 5], "/dev/null");
}

#[test]
fn dev_container_exec_args_includes_remote_user_when_present() {
    let starter = dev_container_starter(Some("vscode"));
    let args = dev_container_exec_args(&starter);

    let user_pos = args
        .iter()
        .position(|arg| arg == "-u")
        .expect("args should include -u when a remote user is set");
    assert_eq!(args[user_pos + 1], "vscode");
}

#[test]
fn dev_container_cp_args_targets_the_container_init_script_path() {
    let starter = dev_container_starter(None);
    let host_path =
        PathBuf::from("/home/user/.cache/warp-terminal-local/dev-container/init/deadbeef.sh");
    let args = dev_container_cp_args(&starter, &host_path);

    assert_eq!(
        args,
        vec![
            std::ffi::OsString::from("cp"),
            std::ffi::OsString::from(&host_path),
            std::ffi::OsString::from(format!("abc123:{}", starter.container_init_script_path())),
        ]
    );
}

#[test]
fn dev_container_chown_args_run_as_root_targeting_the_given_user() {
    let starter = dev_container_starter(Some("vscode"));
    let args = dev_container_chown_args(&starter, "vscode");

    assert_eq!(
        args,
        vec![
            std::ffi::OsString::from("exec"),
            std::ffi::OsString::from("-u"),
            std::ffi::OsString::from("0"),
            std::ffi::OsString::from("abc123"),
            std::ffi::OsString::from("chown"),
            std::ffi::OsString::from("vscode"),
            std::ffi::OsString::from(starter.container_init_script_path()),
        ]
    );
}

#[test]
fn dev_container_chmod_args_lock_the_init_script_to_owner_read_only() {
    let starter = dev_container_starter(None);
    let args = dev_container_chmod_args(&starter);

    assert_eq!(
        args,
        vec![
            std::ffi::OsString::from("exec"),
            std::ffi::OsString::from("-u"),
            std::ffi::OsString::from("0"),
            std::ffi::OsString::from("abc123"),
            std::ffi::OsString::from("chmod"),
            std::ffi::OsString::from("400"),
            std::ffi::OsString::from(starter.container_init_script_path()),
        ]
    );
}

#[test]
fn dev_container_default_user_args_query_the_unqualified_exec_user() {
    let starter = dev_container_starter(None);
    let args = dev_container_default_user_args(&starter);

    // Deliberately no `-u` here: the point is to ask the container what user
    // an unqualified `docker exec` (the same as the real attach uses when
    // there's no `remoteUser`) actually runs as. Passing `-u 0` would always
    // answer "root" regardless of the image's real default user.
    assert_eq!(
        args,
        vec![
            std::ffi::OsString::from("exec"),
            std::ffi::OsString::from("abc123"),
            std::ffi::OsString::from("id"),
            std::ffi::OsString::from("-un"),
        ]
    );
}

#[test]
fn dev_container_default_user_args_do_not_force_root_unlike_chown_args() {
    let starter = dev_container_starter(None);
    let default_user_args = dev_container_default_user_args(&starter);
    let chown_args = dev_container_chown_args(&starter, "someuser");

    // The default-user probe and the chown step have opposite goals for
    // *which* user runs the command: the probe must run unqualified to
    // observe the real default user, while chown must run as root (uid 0)
    // to be able to change ownership at all. Guard against ever
    // "simplifying" the probe back to reusing the chown/chmod/rm helpers'
    // `-u 0`.
    assert!(!default_user_args.iter().any(|arg| arg == "-u"));
    assert!(chown_args.iter().any(|arg| arg == "-u"));
}

#[test]
fn dev_container_init_script_sets_window_size_before_the_shell_init_script() {
    let starter = dev_container_starter(None);
    let size = SizeInfo::new_without_font_metrics(40, 120);
    let script = dev_container_init_script(&starter, &size);

    assert!(script.starts_with("command -p stty rows 40 columns 120\n"));
}

#[test]
fn dev_container_rm_args_remove_the_container_side_init_script() {
    let starter = dev_container_starter(None);
    let args = dev_container_rm_args(&starter);

    assert_eq!(
        args,
        vec![
            std::ffi::OsString::from("exec"),
            std::ffi::OsString::from("-u"),
            std::ffi::OsString::from("0"),
            std::ffi::OsString::from("abc123"),
            std::ffi::OsString::from("rm"),
            std::ffi::OsString::from("-f"),
            std::ffi::OsString::from(starter.container_init_script_path()),
        ]
    );
}
