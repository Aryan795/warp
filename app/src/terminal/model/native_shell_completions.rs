use crate::terminal::shell::ShellType;

/// Returns the command line to write to the PTY to request native shell completions for
/// `buffer_text` (the portion of the input editor's buffer up to the cursor).
///
/// The command hex-encodes `buffer_text` as its sole argument so it can be embedded directly in
/// the command line without any shell-specific quoting -- a hex string only ever contains `[0-9a-f]`
/// characters, none of which are special to any of the supported shells. Each shell's bootstrap
/// script decodes the argument back to the original bytes before use.
///
/// The invoked function name is chosen so each shell's own bookkeeping recognizes it as a
/// generator command (hidden from history, not treated as a normal foreground command, etc.),
/// matching the naming convention already used by `warp_run_generator_command`:
/// - zsh's `_is_warp_generator_command` and `_warp_zshaddhistory` do a substring match on
///   `warp_run_generator_command`.
/// - bash's `warp_preexec` prefix-matches `warp_run_generator_command*`, as does its
///   `HISTIGNORE` entry (`*warp_run_generator_command*`).
/// - fish's `warp_preexec` prefix-matches `warp_run_generator_command*`.
/// - PowerShell's `Warp-Preexec` regex-matches `^Warp-Run-GeneratorCommand`.
pub fn generator_command_for(shell_type: ShellType, buffer_text: &str) -> String {
    let hex_encoded_buffer_text = hex::encode(buffer_text.as_bytes());
    match shell_type {
        // zsh cannot run this through the ordinary (backgrounded) generator command path: it
        // must run in the foreground, in the main shell, with no command substitution around the
        // `select` loop that activates ZLE. See `warp_run_generator_command_foreground_completions`
        // in zsh_body.sh for the full explanation.
        ShellType::Zsh => {
            format!("warp_run_generator_command_foreground_completions {hex_encoded_buffer_text}")
        }
        ShellType::Bash | ShellType::Fish => {
            format!("warp_run_generator_command_native_completions {hex_encoded_buffer_text}")
        }
        ShellType::PowerShell => {
            format!("Warp-Run-GeneratorCommand-NativeCompletions {hex_encoded_buffer_text}")
        }
    }
}

#[cfg(test)]
#[path = "native_shell_completions_tests.rs"]
mod tests;
