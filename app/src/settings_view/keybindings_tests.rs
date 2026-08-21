use warpui::keymap::{EditableBinding, Keystroke};
use warpui::platform::{OperatingSystem, WindowStyle};
use warpui::{App, AppContext, Element, Entity, TypedActionView, View, WindowId};

use super::*;
use crate::util::bindings::keybinding_name_to_display_string;
use crate::workspace::WorkspaceAction;

#[derive(Default)]
struct TestRootView;

impl Entity for TestRootView {
    type Event = ();
}

impl View for TestRootView {
    fn ui_name() -> &'static str {
        "TestRootView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        warpui::elements::Empty::new().finish()
    }
}

impl TypedActionView for TestRootView {
    type Action = ();
}

fn create_test_window(app: &mut App) -> WindowId {
    let (window_id, _root_view) = app.add_window(WindowStyle::NotStealFocus, |_| TestRootView);
    window_id
}

/// Simulates clicking a row's "Clear" button followed by re-entering edit mode and clicking
/// "Default": the built-in keystroke should come back, both in the row shown in Settings and in
/// the underlying matcher used to dispatch the action.
#[test]
fn test_reset_to_default_after_clear_restores_row_and_matcher() {
    App::test((), |mut app| async move {
        crate::test_util::settings::initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| crate::appearance::Appearance::mock());
        app.add_singleton_model(|_| KeybindingChangedNotifier::new());
        app.add_singleton_model(
            crate::server::telemetry::context_provider::AppTelemetryContextProvider::new_context_provider,
        );
        app.add_singleton_model(|_| crate::server::server_api::ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| crate::auth::AuthStateProvider::new_for_test());
        app.add_singleton_model(crate::auth::auth_manager::AuthManager::new_for_test);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            ctx.register_editable_bindings([EditableBinding::new(
                "workspace:show_settings",
                "Open settings",
                WorkspaceAction::ShowSettings,
            )
            .with_key_binding("cmd-,")]);

            let view_handle = ctx.add_typed_action_view(window_id, KeybindingsView::new);
            view_handle.update(ctx, |view, ctx| {
                view.on_page_selected(false, ctx);
            });

            let index = view_handle
                .as_ref(ctx)
                .rows
                .as_ref()
                .unwrap()
                .iter()
                .position(|row| row.binding.name == "workspace:show_settings")
                .expect("binding should be listed as a row");

            // Click the row, then click "Clear".
            view_handle.update(ctx, |view, ctx| {
                view.handle_action(&KeybindingsViewAction::KeybindingRowClicked(index), ctx);
                view.handle_action(&KeybindingsViewAction::RemoveKeyStroke(index), ctx);
            });

            let cleared_trigger = view_handle.as_ref(ctx).rows.as_ref().unwrap()[index]
                .binding
                .trigger
                .clone();
            assert_eq!(cleared_trigger, None);
            assert_eq!(
                None,
                keybinding_name_to_display_string("workspace:show_settings", ctx)
            );

            // Click the row again, then click "Default".
            view_handle.update(ctx, |view, ctx| {
                view.handle_action(&KeybindingsViewAction::KeybindingRowClicked(index), ctx);
                view.handle_action(&KeybindingsViewAction::ResetToDefaultKeyStroke(index), ctx);
            });

            let restored_trigger = view_handle.as_ref(ctx).rows.as_ref().unwrap()[index]
                .binding
                .trigger
                .clone();
            let displayed_keybinding = if OperatingSystem::get().is_mac() {
                "⌘,"
            } else {
                "Logo ,"
            };
            assert_eq!(restored_trigger, Keystroke::parse("cmd-,").ok());
            assert_eq!(
                Some(displayed_keybinding),
                keybinding_name_to_display_string("workspace:show_settings", ctx).as_deref()
            );
        });
    });
}
