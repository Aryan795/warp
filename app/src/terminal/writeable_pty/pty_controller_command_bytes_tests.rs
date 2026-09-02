use super::*;

#[test]
#[cfg(windows)]
fn bracketed_paste_command_execution_normalizes_crlf_to_lf_for_posix_shells_on_windows() {
    let command = "curl 'https://google.com' \\\r\n  -H 'accept: application/json'";

    let bytes = bytes_to_execute_command(command, ShellType::Bash, true);

    let mut expected = ShellType::Bash.kill_buffer_bytes().to_vec();
    expected.extend_from_slice(escape_sequences::BRACKETED_PASTE_START);
    expected.extend_from_slice(b"curl 'https://google.com' \\\n  -H 'accept: application/json'");
    expected.extend_from_slice(escape_sequences::BRACKETED_PASTE_END);
    expected.extend_from_slice(ShellType::Bash.execute_command_bytes());

    assert_eq!(bytes, expected);
    assert!(!bytes.contains(&b'\r'));
}

#[test]
#[cfg(not(windows))]
fn bracketed_paste_command_execution_preserves_crlf_for_posix_shells_off_windows() {
    let command = "curl 'https://google.com' \\\r\n  -H 'accept: application/json'";

    let bytes = bytes_to_execute_command(command, ShellType::Bash, true);

    let mut expected = ShellType::Bash.kill_buffer_bytes().to_vec();
    expected.extend_from_slice(escape_sequences::BRACKETED_PASTE_START);
    expected.extend_from_slice(b"curl 'https://google.com' \\\r\n  -H 'accept: application/json'");
    expected.extend_from_slice(escape_sequences::BRACKETED_PASTE_END);
    expected.extend_from_slice(ShellType::Bash.execute_command_bytes());

    assert_eq!(bytes, expected);
    assert!(bytes.contains(&b'\r'));
}

#[test]
fn unbracketed_paste_command_execution_preserves_lf_for_posix_shells() {
    let command = "printf 'hello'\nprintf 'world'";

    let bytes = bytes_to_execute_command(command, ShellType::Bash, false);

    let mut expected = ShellType::Bash.kill_buffer_bytes().to_vec();
    expected.extend_from_slice(b"printf 'hello'\nprintf 'world'");
    expected.extend_from_slice(ShellType::Bash.execute_command_bytes());

    assert_eq!(bytes, expected);
    assert!(!bytes.contains(&b'\r'));
}

#[test]
fn powershell_command_execution_normalizes_linefeeds_to_carriage_returns() {
    let command = "Write-Output 'hello'\r\nWrite-Output 'world'\nWrite-Output 'again'";

    let bytes = bytes_to_execute_command(command, ShellType::PowerShell, false);

    let mut expected = ShellType::PowerShell.kill_buffer_bytes().to_vec();
    expected.extend_from_slice(b"Write-Output 'hello'\rWrite-Output 'world'\rWrite-Output 'again'");
    expected.extend_from_slice(ShellType::PowerShell.execute_command_bytes());

    assert_eq!(bytes, expected);
    assert!(!bytes.contains(&b'\n'));
}

#[test]
fn split_kill_buffer_write_splits_powershell_off_from_the_rest() {
    let bytes = bytes_to_execute_command("Get-ChildItem", ShellType::PowerShell, false);

    let (kill_buffer, rest) =
        split_kill_buffer_write(&bytes, ShellType::PowerShell).expect("PowerShell should split");

    assert_eq!(kill_buffer, ShellType::PowerShell.kill_buffer_bytes());
    let mut expected_rest = b"Get-ChildItem".to_vec();
    expected_rest.extend_from_slice(ShellType::PowerShell.execute_command_bytes());
    assert_eq!(rest, expected_rest.as_slice());
}

#[test]
fn split_kill_buffer_write_does_not_split_the_other_three_shells() {
    for shell_type in [ShellType::Zsh, ShellType::Bash, ShellType::Fish] {
        let bytes = bytes_to_execute_command("echo hi", shell_type, false);
        assert!(
            split_kill_buffer_write(&bytes, shell_type).is_none(),
            "expected no split for {shell_type:?}, which uses a single unambiguous control byte"
        );
    }
}

#[test]
fn split_kill_buffer_write_handles_a_command_with_no_content_gracefully() {
    let kill_buffer_only = ShellType::PowerShell.kill_buffer_bytes().to_vec();
    assert!(split_kill_buffer_write(&kill_buffer_only, ShellType::PowerShell).is_none());
}

#[test]
fn split_kill_buffer_write_returns_none_when_the_kill_buffer_is_not_the_prefix() {
    let kill_buffer = ShellType::PowerShell.kill_buffer_bytes();
    let not_prefixed = b"Get-ChildItem (no leading chord)".to_vec();
    assert!(
        not_prefixed.len() > kill_buffer.len(),
        "fixture must be longer than the chord so the length guard alone wouldn't catch it"
    );
    assert!(!not_prefixed.starts_with(kill_buffer));
    assert!(split_kill_buffer_write(&not_prefixed, ShellType::PowerShell).is_none());
}

#[test]
fn bash_builtin_ctrl_r_helper_queues_ctrl_r_after_enter() {
    let command = " warp_run_builtin_ctrl_r_widget 67697420737461747573";

    let bytes = bytes_to_execute_command(command, ShellType::Bash, false);

    let mut expected = ShellType::Bash.kill_buffer_bytes().to_vec();
    expected.extend_from_slice(command.as_bytes());
    expected.extend_from_slice(ShellType::Bash.execute_command_bytes());
    expected.push(escape_sequences::C0::DC2);
    assert_eq!(bytes, expected);
    assert_eq!(*bytes.last().unwrap(), escape_sequences::C0::DC2);
    assert_ne!(
        bytes[bytes.len() - 2],
        escape_sequences::C0::DC2,
        "Ctrl-R must follow execute_command_bytes, not precede them"
    );
}

#[test]
fn zsh_builtin_ctrl_r_helper_does_not_queue_ctrl_r() {
    let command = " warp_run_builtin_ctrl_r_widget 67697420737461747573";

    let bytes = bytes_to_execute_command(command, ShellType::Zsh, false);

    let mut expected = ShellType::Zsh.kill_buffer_bytes().to_vec();
    expected.extend_from_slice(command.as_bytes());
    expected.extend_from_slice(ShellType::Zsh.execute_command_bytes());
    assert_eq!(bytes, expected);
    assert!(!bytes.contains(&escape_sequences::C0::DC2));
}

#[test]
fn external_ctrl_r_helper_does_not_queue_ctrl_r() {
    let command = " warp_run_external_ctrl_r_widget";

    let bytes = bytes_to_execute_command(command, ShellType::Bash, false);

    assert!(
        !bytes.contains(&escape_sequences::C0::DC2),
        "fzf/atuin handoff must not receive a queued Ctrl-R"
    );
}

#[test]
fn builtin_ctrl_r_helper_name_does_not_match_a_prefix_of_another_command() {
    let command = " warp_run_builtin_ctrl_r_widget_extra";

    let bytes = bytes_to_execute_command(command, ShellType::Bash, false);

    assert!(!bytes.contains(&escape_sequences::C0::DC2));
}
