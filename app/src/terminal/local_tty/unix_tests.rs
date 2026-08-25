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
    assert_eq!(args[script_pos + 1], "-qfec");
    assert_eq!(
        args[script_pos + 2],
        std::ffi::OsString::from(format!(
            "exec bash --rcfile '{}' --noprofile",
            starter.container_init_script_path()
        ))
    );
    assert_eq!(args[script_pos + 3], "/dev/null");
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
