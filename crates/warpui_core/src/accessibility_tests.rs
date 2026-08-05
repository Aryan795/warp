//! Tests for the platform-independent UI Automation input-provider logic in
//! `accessibility.rs`. These cover the accesskit tree construction and the
//! action-request routing that the Windows adapter relies on. They run on any
//! host (the COM/`WM_GETOBJECT` wiring is exercised manually on Windows).

use accesskit::{Action as A11yAction, ActionData, Role};

use super::{
    A11Y_FOCUSED_INPUT_ID, A11Y_WINDOW_ID, AccessibilityContent, WarpA11yRole,
    build_focused_input_tree, text_to_insert_for_action,
};

fn content(role: WarpA11yRole) -> AccessibilityContent {
    AccessibilityContent::new_without_help("Command Input", role)
}

#[test]
fn tree_exposes_focused_editable_text_input() {
    let update = build_focused_input_tree(&content(WarpA11yRole::TextfieldRole));

    // The window root owns exactly the focused input node, which is where focus
    // points so UIA reports it as the focused element.
    assert_eq!(update.focus, A11Y_FOCUSED_INPUT_ID);
    let root = &update.nodes[0];
    assert_eq!(root.0, A11Y_WINDOW_ID);
    assert_eq!(root.1.role(), Role::Window);
    assert_eq!(root.1.children(), &[A11Y_FOCUSED_INPUT_ID]);

    // The focused node is an editable text input that advertises programmatic
    // insertion, which is what a dictation/automation client targets.
    let input = &update.nodes[1].1;
    assert_eq!(update.nodes[1].0, A11Y_FOCUSED_INPUT_ID);
    assert_eq!(input.role(), Role::TextInput);
    assert_eq!(input.label(), Some("Command Input"));
    assert!(input.supports_action(A11yAction::SetValue));
    assert!(input.supports_action(A11yAction::ReplaceSelectedText));
    assert!(input.value().is_some());
}

#[test]
fn textarea_maps_to_multiline_editable_input() {
    let update = build_focused_input_tree(&content(WarpA11yRole::TextareaRole));
    let input = &update.nodes[1].1;
    assert_eq!(input.role(), Role::MultilineTextInput);
    assert!(input.supports_action(A11yAction::SetValue));
}

#[test]
fn non_text_surface_is_not_settable() {
    // A focused non-text surface (e.g. a button) is represented but does not
    // advertise text insertion, so a dictation client finds no target there.
    let update = build_focused_input_tree(&content(WarpA11yRole::ButtonRole));
    let node = &update.nodes[1].1;
    assert_eq!(node.role(), Role::Button);
    assert!(!node.supports_action(A11yAction::SetValue));
    assert!(!node.supports_action(A11yAction::ReplaceSelectedText));
}

#[test]
fn set_value_action_yields_text_when_focused() {
    let data = ActionData::Value("hello world".into());
    assert_eq!(
        text_to_insert_for_action(A11yAction::SetValue, Some(&data), true),
        Some("hello world".to_string()),
    );
}

#[test]
fn replace_selected_text_action_yields_text_when_focused() {
    let data = ActionData::Value("dictated".into());
    assert_eq!(
        text_to_insert_for_action(A11yAction::ReplaceSelectedText, Some(&data), true),
        Some("dictated".to_string()),
    );
}

#[test]
fn insertion_is_a_no_op_when_nothing_is_focused() {
    // Regression guard for the "no target" bug: when no editable input is
    // focused, a value write must resolve to nothing rather than being applied.
    let data = ActionData::Value("hello".into());
    assert_eq!(
        text_to_insert_for_action(A11yAction::SetValue, Some(&data), false),
        None,
    );
}

#[test]
fn non_text_actions_do_not_insert() {
    assert_eq!(
        text_to_insert_for_action(A11yAction::Click, None, true),
        None,
    );
    // A set-value action missing its value payload inserts nothing.
    assert_eq!(
        text_to_insert_for_action(A11yAction::SetValue, None, true),
        None,
    );
}
