use warpui::keymap::{EditableBinding, Keystroke};
use warpui::platform::WindowStyle;
use warpui::{App, TypedActionView};

use super::*;
use crate::test_util::terminal::initialize_app_for_terminal_view;
use crate::util::bindings::{
    CustomAction, custom_tag_to_keystroke, keybinding_name_to_display_string, trigger_to_keystroke,
};
use crate::workspace::WorkspaceAction;

const BINDING_NAME: &str = "workspace:show_settings";
const DEFAULT_KEYSTROKE: &str = "cmd-,";

fn row_index_for(view: &KeybindingsView, name: &str) -> usize {
    view.rows
        .as_ref()
        .expect("rows should be populated")
        .iter()
        .position(|row| row.binding.name == name)
        .unwrap_or_else(|| panic!("expected to find a row for {name}"))
}

#[test]
fn test_reset_to_default_after_clear_restores_row_and_matcher() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.update(|ctx| {
            ctx.register_editable_bindings([EditableBinding::new(
                BINDING_NAME,
                "Open settings",
                WorkspaceAction::ShowSettings,
            )
            .with_key_binding(DEFAULT_KEYSTROKE)]);
        });

        let (window_id, view_handle) =
            app.add_window(WindowStyle::NotStealFocus, KeybindingsView::new);

        app.update(|ctx| {
            view_handle.update(ctx, |view, ctx| {
                view.on_page_selected(false, ctx);
            });
        });

        let index = app.read(|ctx| row_index_for(view_handle.as_ref(ctx), BINDING_NAME));

        // Clear the binding: the row should go empty and the matcher should stop resolving a
        // keystroke for the binding's name.
        app.update(|ctx| {
            view_handle.update(ctx, |view, ctx| {
                view.handle_action(&KeybindingsViewAction::KeybindingRowClicked(index), ctx);
                view.handle_action(&KeybindingsViewAction::RemoveKeyStroke(index), ctx);
            });
        });

        app.read(|ctx| {
            let view = view_handle.as_ref(ctx);
            assert_eq!(view.rows.as_ref().unwrap()[index].binding.trigger, None);
            assert_eq!(
                None,
                keybinding_name_to_display_string(BINDING_NAME, ctx),
                "matcher should not resolve a keystroke right after Clear"
            );
        });

        // Re-enter edit mode (as the user does before clicking Default) and reset to default.
        app.update(|ctx| {
            view_handle.update(ctx, |view, ctx| {
                view.handle_action(&KeybindingsViewAction::KeybindingRowClicked(index), ctx);
                view.handle_action(&KeybindingsViewAction::ResetToDefaultKeyStroke(index), ctx);
            });
        });

        let expected_keystroke = Keystroke::parse(DEFAULT_KEYSTROKE).unwrap();
        app.read(|ctx| {
            let view = view_handle.as_ref(ctx);
            assert_eq!(
                view.rows.as_ref().unwrap()[index].binding.trigger,
                Some(expected_keystroke.clone()),
                "the row should display the restored default keystroke"
            );
            assert_eq!(
                Some(expected_keystroke.clone()),
                ctx.get_binding_by_name(BINDING_NAME)
                    .and_then(|binding| trigger_to_keystroke(binding.trigger)),
                "the matcher should dispatch the default keystroke again in this session"
            );
        });

        let _ = window_id;
    });
}

#[test]
fn test_reset_to_default_after_clear_restores_custom_trigger_binding() {
    // Mirrors bindings whose default trigger is `Trigger::Custom` (attached to a Mac menu item)
    // rather than a literal `Trigger::Keystrokes`, and where the app never registered a
    // `custom_trigger_to_keystroke_fn` conversion (as is the case on macOS; see app/src/lib.rs).
    const CUSTOM_BINDING_NAME: &str = "workspace:command_palette";

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.update(|ctx| {
            ctx.register_editable_bindings([EditableBinding::new(
                CUSTOM_BINDING_NAME,
                "Open command palette",
                WorkspaceAction::ShowSettings,
            )
            .with_custom_action(CustomAction::CommandPalette)]);
        });

        let expected_keystroke = custom_tag_to_keystroke(CustomAction::CommandPalette.into())
            .expect("CommandPalette should have a static default keystroke");

        let (window_id, view_handle) =
            app.add_window(WindowStyle::NotStealFocus, KeybindingsView::new);

        app.update(|ctx| {
            view_handle.update(ctx, |view, ctx| {
                view.on_page_selected(false, ctx);
            });
        });

        let index = app.read(|ctx| row_index_for(view_handle.as_ref(ctx), CUSTOM_BINDING_NAME));
        app.read(|ctx| {
            let view = view_handle.as_ref(ctx);
            assert_eq!(
                view.rows.as_ref().unwrap()[index].binding.trigger,
                Some(expected_keystroke.clone()),
                "row should start out showing the custom action's default keystroke"
            );
        });

        app.update(|ctx| {
            view_handle.update(ctx, |view, ctx| {
                view.handle_action(&KeybindingsViewAction::KeybindingRowClicked(index), ctx);
                view.handle_action(&KeybindingsViewAction::RemoveKeyStroke(index), ctx);
            });
        });

        app.update(|ctx| {
            view_handle.update(ctx, |view, ctx| {
                view.handle_action(&KeybindingsViewAction::KeybindingRowClicked(index), ctx);
                view.handle_action(&KeybindingsViewAction::ResetToDefaultKeyStroke(index), ctx);
            });
        });

        app.read(|ctx| {
            let view = view_handle.as_ref(ctx);
            assert_eq!(
                view.rows.as_ref().unwrap()[index].binding.trigger,
                Some(expected_keystroke.clone()),
                "the row should display the restored default keystroke for a Custom-triggered binding"
            );
            assert_eq!(
                Some(expected_keystroke.clone()),
                ctx.get_binding_by_name(CUSTOM_BINDING_NAME)
                    .and_then(|binding| trigger_to_keystroke(binding.trigger)),
                "the matcher should dispatch the default keystroke again in this session"
            );
        });

        let _ = window_id;
    });
}
