use super::*;

#[test]
fn hex_encodes_the_buffer_text_argument() {
    let command = generator_command_for(ShellType::Bash, "git ch");
    assert_eq!(
        command,
        "warp_run_generator_command_native_completions 676974206368"
    );
}

#[test]
fn hex_argument_round_trips_arbitrary_bytes() {
    // Characters that would otherwise require shell-specific quoting: single quotes,
    // double quotes, backslashes, and a partially-typed unbalanced quote.
    let inputs = [
        "git ch",
        "echo 'hello world'",
        "echo \"hi\\there\"",
        "echo 'unterminated",
        "",
    ];
    for input in inputs {
        let hex = generator_command_for(ShellType::Fish, input)
            .strip_prefix("warp_run_generator_command_native_completions ")
            .expect("command has the expected prefix")
            .to_owned();
        let decoded = hex::decode(&hex).expect("hex-decodes cleanly");
        assert_eq!(String::from_utf8(decoded).unwrap(), input);
    }
}

#[test]
fn each_shell_uses_a_generator_command_recognized_name() {
    for (shell_type, expected_prefix) in [
        (
            ShellType::Zsh,
            "warp_run_generator_command_foreground_completions ",
        ),
        (
            ShellType::Bash,
            "warp_run_generator_command_native_completions ",
        ),
        (
            ShellType::Fish,
            "warp_run_generator_command_native_completions ",
        ),
        (
            ShellType::PowerShell,
            "Warp-Run-GeneratorCommand-NativeCompletions ",
        ),
    ] {
        let command = generator_command_for(shell_type, "x");
        assert!(
            command.starts_with(expected_prefix),
            "expected {command:?} to start with {expected_prefix:?}"
        );
    }
}
