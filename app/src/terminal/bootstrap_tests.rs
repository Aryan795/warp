use std::collections::{HashMap, HashSet};

use warp_core::session_id::SessionId;

use super::*;
use crate::terminal::ShellLaunchData;
use crate::terminal::model::session::{HostInfo, IsSSHWrapperSession};
use crate::terminal::model::terminal_model::SubshellInitializationInfo;
use crate::terminal::shell::Shell;

struct TestAssetProvider;

impl AssetProvider for TestAssetProvider {
    fn get(&self, path: &str) -> anyhow::Result<Cow<'_, [u8]>> {
        let content = match path {
            "bundled/bootstrap/bash.sh" => "#include hello_world",
            "bundled/bootstrap/fish.sh" => "# this is a comment\nthis_is_a_command",
            "bundled/bootstrap/zsh.sh" => {
                "asdf\n#include whitespace\n    prepended whitespace\n\n\n"
            }
            "bundled/bootstrap/pwsh.ps1" => {
                r#"# This is a comment
                Write-Output 'Testing some output'
                function test1 {
                    [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingInvokeExpression', '', Justification = 'We actually need it')]
                    param([string]$command)
                    Invoke-Expression $command
                }"#
            }
            "hello_world" => "hello world!",
            "whitespace" => "no whitespace\n\n\n yes whitespace!",
            _ => anyhow::bail!("path not found in assets"),
        };
        Ok(Cow::Borrowed(content.as_bytes()))
    }
}

#[test]
fn test_include_directive() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Bash, &TestAssetProvider)),
        "hello world!\n"
    );
}

#[test]
fn test_trims_comments() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Fish, &TestAssetProvider)),
        "this_is_a_command\n"
    );
}

#[test]
fn test_trims_whitespace() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Zsh, &TestAssetProvider)),
        "asdf\nno whitespace\n yes whitespace!\n prepended whitespace\n"
    );
}

#[test]
fn test_trims_powershell_specifics() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::PowerShell, &TestAssetProvider)),
        " Write-Output 'Testing some output'\n function test1 {\n param([string]$command)\n Invoke-Expression $command\n }\n"
    );
}

fn decode_script(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("should not fail to decode")
}

fn session_info_for_test(
    launch_data: Option<ShellLaunchData>,
    subshell_info: Option<SubshellInitializationInfo>,
) -> SessionInfo {
    SessionInfo {
        session_id: SessionId::from(1),
        shell: Shell::new(ShellType::Bash, None, None, HashSet::new(), None),
        launch_data,
        histfile: None,
        user: "test-user".to_owned(),
        hostname: "test-host".to_owned(),
        subshell_info,
        path: None,
        environment_variable_names: HashSet::new(),
        aliases: HashMap::new(),
        abbreviations: HashMap::new(),
        function_names: HashSet::new(),
        builtins: HashSet::new(),
        keywords: Vec::new(),
        is_ssh_wrapper_session: IsSSHWrapperSession::No,
        home_dir: None,
        cdpath: None,
        editor: None,
        session_type: BootstrapSessionType::Local,
        host_info: HostInfo::default(),
        wsl_name: None,
        spawning_session_id: None,
    }
}

fn subshell_info_for_command(spawning_command: &str) -> SubshellInitializationInfo {
    SubshellInitializationInfo {
        spawning_command: spawning_command.to_owned(),
        was_triggered_by_rc_file_snippet: false,
        env_var_collection_name: None,
        ssh_connection_info: None,
    }
}

#[test]
fn dev_container_top_level_session_is_container_exec_relayed() {
    let session_info = session_info_for_test(
        Some(ShellLaunchData::DevContainer {
            workspace_folder: "/home/user/project".into(),
            docker_path: "/usr/bin/docker".into(),
            container_id: "abc123".to_owned(),
            remote_user: None,
            remote_workspace_folder: "/workspaces/project".to_owned(),
            sandbox_id: "deadbeef".to_owned(),
            session_id: SessionId::from(1),
        }),
        None,
    );
    assert!(is_container_exec_relayed_session(&session_info));
}

#[test]
fn detected_docker_exec_subshell_is_container_exec_relayed() {
    let session_info = session_info_for_test(
        None,
        Some(subshell_info_for_command(
            "docker exec -it my-container bash",
        )),
    );
    assert!(is_container_exec_relayed_session(&session_info));
}

#[test]
fn plain_local_session_is_not_container_exec_relayed() {
    let session_info = session_info_for_test(
        Some(ShellLaunchData::Executable {
            executable_path: "/bin/bash".into(),
            shell_type: ShellType::Bash,
        }),
        None,
    );
    assert!(!is_container_exec_relayed_session(&session_info));
}
