use std::collections::HashSet;

use super::{
    BUILTIN_CTRL_R_HELPER_COMMAND, BUILTIN_CTRL_R_HISTORY_PLUGIN_TAG, CtrlRHistoryHandoffKind,
    EXTERNAL_CTRL_R_HISTORY_PLUGIN_TAG, builtin_ctrl_r_helper_command, ctrl_r_history_handoff_kind,
};
use crate::features::FeatureFlag;
use crate::terminal::shell::ShellType;

fn plugins(tags: &[&str]) -> HashSet<String> {
    tags.iter().map(|tag| (*tag).to_owned()).collect()
}

#[test]
fn builtin_handoff_is_off_when_the_flag_is_disabled() {
    let _flag = FeatureFlag::BuiltinShellHistoryHandoff.override_enabled(false);

    assert_eq!(
        ctrl_r_history_handoff_kind(
            &plugins(&[BUILTIN_CTRL_R_HISTORY_PLUGIN_TAG]),
            ShellType::Bash
        ),
        None
    );
    assert_eq!(
        ctrl_r_history_handoff_kind(
            &plugins(&[BUILTIN_CTRL_R_HISTORY_PLUGIN_TAG]),
            ShellType::Zsh
        ),
        None
    );
}

#[test]
fn builtin_handoff_selects_bash_and_zsh_when_the_flag_is_enabled() {
    let _flag = FeatureFlag::BuiltinShellHistoryHandoff.override_enabled(true);

    assert_eq!(
        ctrl_r_history_handoff_kind(
            &plugins(&[BUILTIN_CTRL_R_HISTORY_PLUGIN_TAG]),
            ShellType::Bash
        ),
        Some(CtrlRHistoryHandoffKind::Builtin)
    );
    assert_eq!(
        ctrl_r_history_handoff_kind(
            &plugins(&[BUILTIN_CTRL_R_HISTORY_PLUGIN_TAG]),
            ShellType::Zsh
        ),
        Some(CtrlRHistoryHandoffKind::Builtin)
    );
    assert_eq!(
        ctrl_r_history_handoff_kind(
            &plugins(&[BUILTIN_CTRL_R_HISTORY_PLUGIN_TAG]),
            ShellType::Fish
        ),
        None
    );
}

#[test]
fn external_fzf_atuin_handoff_takes_precedence_over_builtin() {
    let _shell_widget = FeatureFlag::ShellWidgetHandoff.override_enabled(true);
    let _builtin = FeatureFlag::BuiltinShellHistoryHandoff.override_enabled(true);

    assert_eq!(
        ctrl_r_history_handoff_kind(
            &plugins(&[
                EXTERNAL_CTRL_R_HISTORY_PLUGIN_TAG,
                BUILTIN_CTRL_R_HISTORY_PLUGIN_TAG
            ]),
            ShellType::Zsh
        ),
        Some(CtrlRHistoryHandoffKind::External)
    );
}

#[test]
fn external_handoff_is_off_when_shell_widget_flag_is_disabled() {
    let _shell_widget = FeatureFlag::ShellWidgetHandoff.override_enabled(false);
    let _builtin = FeatureFlag::BuiltinShellHistoryHandoff.override_enabled(true);

    assert_eq!(
        ctrl_r_history_handoff_kind(
            &plugins(&[EXTERNAL_CTRL_R_HISTORY_PLUGIN_TAG]),
            ShellType::Zsh
        ),
        None
    );
    assert_eq!(
        ctrl_r_history_handoff_kind(
            &plugins(&[BUILTIN_CTRL_R_HISTORY_PLUGIN_TAG]),
            ShellType::Zsh
        ),
        Some(CtrlRHistoryHandoffKind::Builtin)
    );
}

#[test]
fn builtin_helper_command_hex_encodes_the_first_draft_line() {
    assert_eq!(
        builtin_ctrl_r_helper_command("git status"),
        format!(
            "{} {}",
            BUILTIN_CTRL_R_HELPER_COMMAND,
            hex::encode(b"git status")
        )
    );
    assert_eq!(
        builtin_ctrl_r_helper_command("first\nsecond"),
        format!(
            "{} {}",
            BUILTIN_CTRL_R_HELPER_COMMAND,
            hex::encode(b"first")
        )
    );
    assert_eq!(
        builtin_ctrl_r_helper_command(""),
        BUILTIN_CTRL_R_HELPER_COMMAND
    );
}
