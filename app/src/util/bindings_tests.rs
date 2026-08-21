use warpui::App;
use warpui::keymap::{EditableBinding, Keystroke, Trigger};
use warpui::platform::OperatingSystem;

use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::terminal;
use crate::util::bindings::{
    keybinding_name_to_display_string, reset_keybinding_to_default, trigger_to_keystroke,
};
use crate::workspace::WorkspaceAction;

#[test]
fn test_reset_to_default_after_clear() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| KeybindingChangedNotifier::mock());
            ctx.register_editable_bindings([EditableBinding::new(
                "workspace:show_settings",
                "Open settings",
                WorkspaceAction::ShowSettings,
            )
            .with_key_binding("cmd-,")]);

            let binding_id = ctx
                .get_binding_by_name("workspace:show_settings")
                .expect("binding should be registered")
                .id;

            ctx.set_custom_trigger("workspace:show_settings".to_owned(), Trigger::Empty);
            assert_eq!(
                None,
                keybinding_name_to_display_string("workspace:show_settings", ctx)
            );

            let restored = reset_keybinding_to_default("workspace:show_settings", binding_id, ctx);
            assert_eq!(restored, Keystroke::parse("cmd-,").ok());
        });
    });
}

#[test]
fn test_reset_to_default_after_clear_resolves_correct_binding_when_names_collide() {
    // Two editable bindings can share the same name when registered for different
    // views/contexts (see the dedup comment in `KeybindingsView::on_page_selected`). Resolving
    // the default trigger by name alone can non-deterministically pick the wrong registration's
    // default; resolving by the binding's own `BindingId` must always restore the correct one.
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| KeybindingChangedNotifier::mock());
            ctx.register_editable_bindings([
                EditableBinding::new(
                    "shared:action",
                    "Shared action",
                    WorkspaceAction::ShowSettings,
                )
                .with_key_binding("cmd-,"),
                // Registered later, so it takes precedence in `editable_bindings()`'s
                // reverse-registration-order iteration. It has no default trigger.
                EditableBinding::new(
                    "shared:action",
                    "Shared action",
                    WorkspaceAction::ToggleResourceCenter,
                ),
            ]);

            let binding_id = ctx
                .editable_bindings()
                .find(|binding| {
                    binding.name == "shared:action" && binding.trigger != &Trigger::Empty
                })
                .expect("the binding with a real default should be registered")
                .id;

            ctx.set_custom_trigger("shared:action".to_owned(), Trigger::Empty);
            let restored = reset_keybinding_to_default("shared:action", binding_id, ctx);
            assert_eq!(
                restored,
                Keystroke::parse("cmd-,").ok(),
                "should restore the specific binding's own default, not another same-named \
                 registration's"
            );
        });
    });
}

#[test]
fn test_keybinding_name_to_display_string() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.register_editable_bindings([
                EditableBinding::new(
                    "workspace:show_settings",
                    "Open settings",
                    WorkspaceAction::ShowSettings,
                )
                .with_key_binding("cmd-,"),
                EditableBinding::new(
                    "workspace:toggle_resource_center",
                    "Toggle Resource Center",
                    WorkspaceAction::ToggleResourceCenter,
                ),
            ]);

            let displayed_keybinding = if OperatingSystem::get().is_mac() {
                "⌘,"
            } else {
                "Logo ,"
            };
            assert_eq!(
                Some(displayed_keybinding),
                keybinding_name_to_display_string("workspace:show_settings", ctx).as_deref()
            );

            assert_eq!(
                None,
                keybinding_name_to_display_string("workspace:toggle_resource_center", ctx)
            );

            ctx.set_custom_trigger(
                "workspace:show_settings".to_owned(),
                Trigger::Keystrokes(vec![Keystroke::parse("cmd-shift-<").unwrap()]),
            );

            let displayed_keybinding = if OperatingSystem::get().is_mac() {
                "⇧⌘<"
            } else {
                "Shift Logo <"
            };
            assert_eq!(
                Some(displayed_keybinding),
                keybinding_name_to_display_string("workspace:show_settings", ctx).as_deref()
            );

            ctx.set_custom_trigger(
                "workspace:toggle_resource_center".to_owned(),
                Trigger::Keystrokes(vec![Keystroke::parse("cmd-alt-/").unwrap()]),
            );

            let expected_keybinding = if OperatingSystem::get().is_mac() {
                "⌥⌘/"
            } else {
                "Alt Logo /"
            };
            assert_eq!(
                Some(expected_keybinding),
                keybinding_name_to_display_string("workspace:toggle_resource_center", ctx)
                    .as_deref()
            );
        });
    });
}

#[test]
fn test_orchestration_cycle_bindings_are_editable() {
    App::test((), |mut app| async move {
        app.update(terminal::init);

        app.update(|ctx| {
            let next = ctx
                .editable_bindings()
                .find(|binding| binding.name == "terminal:cycle_next_orchestration_child_agent")
                .and_then(|binding| trigger_to_keystroke(binding.trigger));
            let previous = ctx
                .editable_bindings()
                .find(|binding| binding.name == "terminal:cycle_previous_orchestration_child_agent")
                .and_then(|binding| trigger_to_keystroke(binding.trigger));

            assert_eq!(next, Keystroke::parse("ctrl-alt-]").ok());
            assert_eq!(previous, Keystroke::parse("ctrl-alt-[").ok());
        });
    });
}

#[test]
fn test_toggle_maximize_pane_binding_is_editable() {
    App::test((), |mut app| async move {
        app.update(crate::pane_group::init);

        app.update(|ctx| {
            use crate::pane_group::TOGGLE_MAXIMIZE_PANE_BINDING_NAME;

            // The toggle-maximize-pane action is registered as an editable binding so
            // it can be assigned a shortcut in Settings → Keyboard shortcuts.
            assert!(
                ctx.editable_bindings()
                    .any(|binding| binding.name == TOGGLE_MAXIMIZE_PANE_BINDING_NAME),
                "{TOGGLE_MAXIMIZE_PANE_BINDING_NAME} should be registered as an editable binding"
            );

            // It ships with a mac-only default shortcut (cmd-shift-enter) via its custom
            // action; other platforms have no default until the user assigns one. Either
            // way, whatever resolves here is what the pane header menu item surfaces.
            let default = keybinding_name_to_display_string(TOGGLE_MAXIMIZE_PANE_BINDING_NAME, ctx);
            if OperatingSystem::get().is_mac() {
                assert_eq!(Some("⇧⌘⏎"), default.as_deref());
            } else {
                assert_eq!(None, default);
            }

            // A reassigned shortcut resolves to its display string on every platform.
            ctx.set_custom_trigger(
                TOGGLE_MAXIMIZE_PANE_BINDING_NAME.to_owned(),
                Trigger::Keystrokes(vec![Keystroke::parse("cmd-shift-M").unwrap()]),
            );

            let displayed_keybinding = if OperatingSystem::get().is_mac() {
                "⇧⌘M"
            } else {
                "Shift Logo M"
            };
            assert_eq!(
                Some(displayed_keybinding),
                keybinding_name_to_display_string(TOGGLE_MAXIMIZE_PANE_BINDING_NAME, ctx)
                    .as_deref()
            );
        });
    });
}

#[test]
fn test_terminal_page_scroll_bindings_are_editable() {
    App::test((), |mut app| async move {
        app.update(terminal::init);

        app.update(|ctx| {
            let page_up = ctx
                .editable_bindings()
                .find(|binding| binding.name == "terminal:scroll_up_one_page")
                .and_then(|binding| trigger_to_keystroke(binding.trigger));
            let page_down = ctx
                .editable_bindings()
                .find(|binding| binding.name == "terminal:scroll_down_one_page")
                .and_then(|binding| trigger_to_keystroke(binding.trigger));

            assert_eq!(page_up, Keystroke::parse("pageup").ok());
            assert_eq!(page_down, Keystroke::parse("pagedown").ok());
        });
    });
}
