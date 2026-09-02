//! Runs the bootstrap helper/detection snippets in real bash and zsh.
//!
//! Interactive `read -e` / `vared` need a TTY and are stubbed so these tests stay
//! deterministic. Ctrl-R byte ordering is covered by `pty_controller_command_bytes_tests`
//! (`bytes_to_execute_command` is the event-loop write payload).

use std::path::PathBuf;

use command::blocking::Command;

fn bootstrap_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets/bundled/bootstrap")
        .join(name)
}

fn read_bootstrap(name: &str) -> String {
    std::fs::read_to_string(bootstrap_path(name))
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", bootstrap_path(name).display()))
}

fn extract_function(source: &str, name: &str) -> String {
    let needle = format!("{name} ()");
    let start = source
        .find(&needle)
        .or_else(|| source.find(&format!("function {name} ()")))
        .or_else(|| source.find(&format!("function {name} {{")))
        .unwrap_or_else(|| panic!("function {name} not found"));
    let rest = &source[start..];
    let brace_start = rest
        .find('{')
        .unwrap_or_else(|| panic!("opening brace for {name} not found"));
    let mut depth = 0usize;
    for (i, ch) in rest[brace_start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return rest[..=brace_start + i].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("unclosed function {name}")
}

fn run_shell(program: &str, extra_args: &[&str], script: &str) -> String {
    let output = Command::new(program)
        .args(extra_args)
        .arg("-c")
        .arg(script)
        .output()
        .unwrap_or_else(|err| panic!("failed to spawn {program}: {err}"));
    assert!(
        output.status.success(),
        "{program} failed ({:?})\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn run_bash_detect(setup: &str) -> String {
    let source = read_bootstrap("bash_body.sh");
    let version_fn = extract_function(&source, "warp_at_least_bash_version");
    let detect_fn = extract_function(&source, "warp_detect_ctrl_r_history");
    run_shell(
        "bash",
        &["--norc", "--noprofile"],
        &format!(
            r#"
set -u
shell_plugins=()
WARP_IN_MSYS2=false
{version_fn}
{detect_fn}
{setup}
warp_detect_ctrl_r_history
printf '%s\n' "${{shell_plugins[*]-}}"
"#
        ),
    )
}

fn run_zsh_detect(setup: &str) -> String {
    let source = read_bootstrap("zsh_body.sh");
    let detect_fn = extract_function(&source, "warp_detect_ctrl_r_history");
    run_shell(
        "zsh",
        &["-f"],
        &format!(
            r#"
emulate -L zsh
setopt nounset
local -a shell_plugins
{detect_fn}
{setup}
warp_detect_ctrl_r_history
print -r -- "${{(j: :)shell_plugins}}"
"#
        ),
    )
}

fn assert_selection_hook(stdout: &str, expected_buffer: &str) {
    assert!(
        stdout.contains(r#""hook": "ExternalShellWidgetSelection""#)
            || stdout.contains(r#"\"hook\": \"ExternalShellWidgetSelection\""#),
        "expected ExternalShellWidgetSelection in {stdout:?}"
    );
    let escaped = expected_buffer.replace('\\', r#"\\"#).replace('"', r#"\""#);
    assert!(
        stdout.contains(&format!(r#""buffer": "{escaped}""#))
            || stdout.contains(&format!(r#"\"buffer\": \"{escaped}\""#)),
        "expected buffer {expected_buffer:?} in {stdout:?}"
    );
}

fn bash_harness(invocation: &str, read_result: Option<&str>) -> String {
    let source = read_bootstrap("bash_body.sh");
    let hex_decode = extract_function(&source, "warp_hex_decode_string");
    let escape_json = extract_function(&source, "warp_escape_json");
    let mut helper = extract_function(&source, "warp_run_builtin_ctrl_r_widget");
    helper = helper.replace(
        r#"IFS= read -r -e -i "$draft" result"#,
        r#"result="${WARP_TEST_READ_RESULT-$draft}""#,
    );
    let result_line = match read_result {
        Some(value) => format!("WARP_TEST_READ_RESULT={value}"),
        None => String::new(),
    };
    format!(
        r#"
set -u
WARP_SESSION_ID=1
{result_line}
warp_send_json_message () {{ printf '%s\n' "$1"; }}
{hex_decode}
{escape_json}
{helper}
{invocation}
"#
    )
}

fn zsh_harness(invocation: &str, read_result: Option<&str>) -> String {
    let source = read_bootstrap("zsh_body.sh");
    let hex_decode = extract_function(&source, "warp_hex_decode_string");
    let escape_json = extract_function(&source, "warp_escape_json");
    let mut helper = extract_function(&source, "warp_run_builtin_ctrl_r_widget");
    helper = helper.replace(
        "vared -h -i warp_builtin_history_search_init result",
        r#"result="${WARP_TEST_READ_RESULT-$result}""#,
    );
    let result_line = match read_result {
        Some(value) => format!("WARP_TEST_READ_RESULT={value}"),
        None => String::new(),
    };
    format!(
        r#"
emulate -L zsh
setopt nounset
WARP_SESSION_ID=1
{result_line}
warp_send_json_message () {{ print -r -- "$1"; }}
zle () {{ : }}
{hex_decode}
{escape_json}
{helper}
{invocation}
"#
    )
}

#[test]
fn bash_empty_draft_under_nounset_cancels_with_empty_selection() {
    let stdout = run_shell(
        "bash",
        &[],
        &bash_harness("warp_run_builtin_ctrl_r_widget", Some("''")),
    );
    assert_selection_hook(&stdout, "");
}

#[test]
fn zsh_empty_draft_under_nounset_cancels_with_empty_selection() {
    let stdout = run_shell(
        "zsh",
        &[],
        &zsh_harness("warp_run_builtin_ctrl_r_widget", Some("''")),
    );
    assert_selection_hook(&stdout, "");
}

#[test]
fn bash_unicode_draft_is_decoded_and_selected() {
    let hex = hex::encode("café git".as_bytes());
    let stdout = run_shell(
        "bash",
        &[],
        &bash_harness(&format!("warp_run_builtin_ctrl_r_widget {hex}"), None),
    );
    assert_selection_hook(&stdout, "café git");
}

#[test]
fn zsh_unicode_draft_is_decoded_and_selected() {
    let hex = hex::encode("café git".as_bytes());
    let stdout = run_shell(
        "zsh",
        &[],
        &zsh_harness(&format!("warp_run_builtin_ctrl_r_widget {hex}"), None),
    );
    assert_selection_hook(&stdout, "café git");
}

#[test]
fn bash_successful_selection_emits_external_shell_widget_selection() {
    let stdout = run_shell(
        "bash",
        &[],
        &bash_harness("warp_run_builtin_ctrl_r_widget", Some("'echo selected'")),
    );
    assert_selection_hook(&stdout, "echo selected");
}

#[test]
fn zsh_successful_selection_emits_external_shell_widget_selection() {
    let stdout = run_shell(
        "zsh",
        &[],
        &zsh_harness("warp_run_builtin_ctrl_r_widget", Some("'echo selected'")),
    );
    assert_selection_hook(&stdout, "echo selected");
}

#[test]
fn bash_helper_uses_nounset_safe_draft_arg() {
    let source = read_bootstrap("bash_body.sh");
    assert!(
        source.contains(r#"[ -n "${1-}" ]"#),
        "bash helper must use nounset-safe ${{1-}}"
    );
}

#[test]
fn bash_default_ctrl_r_emits_builtin_capability() {
    let stdout = run_bash_detect("");
    assert_eq!(stdout.trim(), "builtin_ctrl_r_history");
}

#[test]
fn bash_detection_prefers_fzf_atuin_over_builtin() {
    let stdout = run_bash_detect(r#"bind -x '"\C-r": __fzf_history__'"#);
    assert_eq!(stdout.trim(), "external_ctrl_r_history");
}

#[test]
fn bash_custom_ctrl_r_binding_is_untagged() {
    let stdout = run_bash_detect(r#"bind -x '"\C-r": echo custom'"#);
    assert_eq!(stdout.trim(), "");
}

#[test]
fn bash_unbound_ctrl_r_is_untagged() {
    let stdout = run_bash_detect("bind -r \"\\C-r\"");
    assert_eq!(stdout.trim(), "");
}

#[test]
fn bash_older_than_4_skips_builtin_capability() {
    let stdout = run_bash_detect("warp_at_least_bash_version () { echo 0; }");
    assert_eq!(stdout.trim(), "");
}

#[test]
fn bash_msys2_skips_ctrl_r_detection() {
    let stdout = run_bash_detect("WARP_IN_MSYS2=true");
    assert_eq!(stdout.trim(), "");
}

#[test]
fn zsh_helper_uses_nounset_safe_draft_arg() {
    let source = read_bootstrap("zsh_body.sh");
    assert!(
        source.contains(r#"[[ -n "${1-}" ]]"#),
        "zsh helper must use nounset-safe ${{1-}}"
    );
}

#[test]
fn zsh_default_ctrl_r_emits_builtin_capability() {
    let stdout = run_zsh_detect("");
    assert_eq!(stdout.trim(), "builtin_ctrl_r_history");
}

#[test]
fn zsh_detection_prefers_fzf_atuin_over_builtin() {
    let stdout = run_zsh_detect(
        r#"
fzf-history-widget() { }
zle -N fzf-history-widget
bindkey '^R' fzf-history-widget
"#,
    );
    assert_eq!(stdout.trim(), "external_ctrl_r_history");
}

#[test]
fn zsh_custom_ctrl_r_binding_is_untagged() {
    let stdout = run_zsh_detect(
        r#"
my-custom() { }
zle -N my-custom
bindkey '^R' my-custom
"#,
    );
    assert_eq!(stdout.trim(), "");
}

#[test]
fn zsh_undefined_ctrl_r_is_untagged() {
    let stdout = run_zsh_detect(r#"bindkey -r '^R'"#);
    assert_eq!(stdout.trim(), "");
}
