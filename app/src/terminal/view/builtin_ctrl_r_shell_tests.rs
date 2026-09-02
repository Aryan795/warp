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
fn bash_detection_prefers_fzf_atuin_over_builtin() {
    let source = read_bootstrap("bash_body.sh");
    assert!(
        source.contains(r#"[ -n "${1-}" ]"#),
        "bash helper must use nounset-safe ${{1-}}"
    );
    let stdout = run_shell(
        "bash",
        &[],
        r#"
set -u
shell_plugins=()
_WARP_EXTERNAL_CTRL_R_WIDGET=""
WARP_IN_MSYS2=false
bind () {
  if [ "$1" = -X ]; then
    printf '%s\n' '"\C-r": "__fzf_history__"'
  elif [ "$1" = -p ]; then
    printf '%s\n' '"\C-r": reverse-search-history'
  fi
}
warp_at_least_bash_version () { echo 1; }
warp_ctrl_r_binding="$(bind -X 2>/dev/null | command -p sed -n 's/^"\\C-r"[ :] *"\(.*\)"$/\1/p')"
case "$warp_ctrl_r_binding" in
  __fzf_history__|__atuin_history)
    _WARP_EXTERNAL_CTRL_R_WIDGET="$warp_ctrl_r_binding"
    shell_plugins+=(external_ctrl_r_history)
    ;;
esac
if [ -z "$_WARP_EXTERNAL_CTRL_R_WIDGET" ] && [ "$(warp_at_least_bash_version "4.0")" = "1" ]; then
  warp_ctrl_r_readline="$(bind -p 2>/dev/null | command -p sed -n 's/^"\\C-r": //p')"
  case "$warp_ctrl_r_readline" in
    reverse-search-history)
      shell_plugins+=(builtin_ctrl_r_history)
      ;;
  esac
fi
printf '%s %s\n' "$_WARP_EXTERNAL_CTRL_R_WIDGET" "${shell_plugins[*]}"
"#,
    );
    assert_eq!(stdout.trim(), "__fzf_history__ external_ctrl_r_history");
}

#[test]
fn bash_detection_reports_builtin_when_ctrl_r_is_reverse_search() {
    let stdout = run_shell(
        "bash",
        &[],
        r#"
set -u
shell_plugins=()
_WARP_EXTERNAL_CTRL_R_WIDGET=""
bind () {
  if [ "$1" = -X ]; then
    printf '%s\n' ''
  elif [ "$1" = -p ]; then
    printf '%s\n' '"\C-r": reverse-search-history'
  fi
}
warp_at_least_bash_version () { echo 1; }
warp_ctrl_r_binding="$(bind -X 2>/dev/null | command -p sed -n 's/^"\\C-r"[ :] *"\(.*\)"$/\1/p')"
case "$warp_ctrl_r_binding" in
  __fzf_history__|__atuin_history)
    _WARP_EXTERNAL_CTRL_R_WIDGET="$warp_ctrl_r_binding"
    shell_plugins+=(external_ctrl_r_history)
    ;;
esac
if [ -z "$_WARP_EXTERNAL_CTRL_R_WIDGET" ] && [ "$(warp_at_least_bash_version "4.0")" = "1" ]; then
  warp_ctrl_r_readline="$(bind -p 2>/dev/null | command -p sed -n 's/^"\\C-r": //p')"
  case "$warp_ctrl_r_readline" in
    reverse-search-history)
      shell_plugins+=(builtin_ctrl_r_history)
      ;;
  esac
fi
printf '%s\n' "${shell_plugins[*]}"
"#,
    );
    assert_eq!(stdout.trim(), "builtin_ctrl_r_history");
}

#[test]
fn zsh_detection_prefers_fzf_atuin_over_builtin() {
    let source = read_bootstrap("zsh_body.sh");
    assert!(
        source.contains(r#"[[ -n "${1-}" ]]"#),
        "zsh helper must use nounset-safe ${{1-}}"
    );
    let stdout = run_shell(
        "zsh",
        &[],
        r#"
emulate -L zsh
setopt nounset
local -a shell_plugins
_WARP_EXTERNAL_CTRL_R_WIDGET=""
_WARP_BUILTIN_CTRL_R_WIDGET=""
warp_ctrl_r_widget="fzf-history-widget"
case "$warp_ctrl_r_widget" in
  fzf-history-widget|atuin-search|atuin-search-viins|atuin-search-vicmd|_atuin_search_widget)
    _WARP_EXTERNAL_CTRL_R_WIDGET="$warp_ctrl_r_widget"
    shell_plugins+=(external_ctrl_r_history)
    ;;
  history-incremental-search-backward|history-incremental-pattern-search-backward)
    _WARP_BUILTIN_CTRL_R_WIDGET="$warp_ctrl_r_widget"
    shell_plugins+=(builtin_ctrl_r_history)
    ;;
esac
print -r -- "$_WARP_EXTERNAL_CTRL_R_WIDGET ${shell_plugins[*]}"
"#,
    );
    assert_eq!(stdout.trim(), "fzf-history-widget external_ctrl_r_history");
}

#[test]
fn zsh_detection_reports_builtin_incremental_search() {
    let stdout = run_shell(
        "zsh",
        &[],
        r#"
emulate -L zsh
setopt nounset
local -a shell_plugins
_WARP_EXTERNAL_CTRL_R_WIDGET=""
_WARP_BUILTIN_CTRL_R_WIDGET=""
warp_ctrl_r_widget="history-incremental-search-backward"
case "$warp_ctrl_r_widget" in
  fzf-history-widget|atuin-search|atuin-search-viins|atuin-search-vicmd|_atuin_search_widget)
    _WARP_EXTERNAL_CTRL_R_WIDGET="$warp_ctrl_r_widget"
    shell_plugins+=(external_ctrl_r_history)
    ;;
  history-incremental-search-backward|history-incremental-pattern-search-backward)
    _WARP_BUILTIN_CTRL_R_WIDGET="$warp_ctrl_r_widget"
    shell_plugins+=(builtin_ctrl_r_history)
    ;;
esac
print -r -- "${shell_plugins[*]}"
"#,
    );
    assert_eq!(stdout.trim(), "builtin_ctrl_r_history");
}
