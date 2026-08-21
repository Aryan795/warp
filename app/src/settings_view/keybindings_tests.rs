use std::cell::RefCell;
use std::rc::Rc;

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

#[derive(Debug, Clone, PartialEq)]
enum ProbeAction {
    NonEmptyDefault,
    NoDefault,
}

/// Minimal root view used to prove a keystroke actually dispatches through the real matcher,
/// rather than only checking the resolved `Trigger`.
struct DispatchProbeView {
    dispatched: Rc<RefCell<Vec<ProbeAction>>>,
}

impl Entity for DispatchProbeView {
    type Event = ();
}

impl View for DispatchProbeView {
    fn ui_name() -> &'static str {
        "DispatchProbeView"
    }

    fn render(&self, _ctx: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for DispatchProbeView {
    type Action = ProbeAction;

    fn handle_action(&mut self, action: &Self::Action, _ctx: &mut ViewContext<Self>) {
        self.dispatched.borrow_mut().push(action.clone());
    }
}

#[test]
fn test_reset_to_default_after_clear_resolves_collision_and_dispatches_via_matcher() {
    // Two `EditableBinding`s can share a name when registered per-view/context (see the dedup
    // comment in `on_page_selected`). With distinct descriptions they render as separate rows,
    // each with its own default. Clear -> Default on the row with a real default must restore
    // *that* default -- not the other same-named registration's -- in both the row, the
    // `KeybindingsView`'s internal binding-list cache, and the live matcher.
    const SHARED_NAME: &str = "shared:duplicate_action";
    const NON_EMPTY_DESCRIPTION: &str = "Shared action with a default";
    const EMPTY_DESCRIPTION: &str = "Shared action without a default";
    const DEFAULT_KEYSTROKE: &str = "cmd-,";

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let dispatched = Rc::new(RefCell::new(Vec::new()));
        let dispatched_for_view = dispatched.clone();

        app.update(|ctx| {
            ctx.register_editable_bindings([
                EditableBinding::new(
                    SHARED_NAME,
                    NON_EMPTY_DESCRIPTION,
                    ProbeAction::NonEmptyDefault,
                )
                .with_key_binding(DEFAULT_KEYSTROKE),
                EditableBinding::new(SHARED_NAME, EMPTY_DESCRIPTION, ProbeAction::NoDefault),
            ]);
        });

        let (probe_window_id, probe_view) =
            app.add_window(WindowStyle::NotStealFocus, |_ctx| DispatchProbeView {
                dispatched: dispatched_for_view,
            });
        let probe_view_id = probe_view.id();

        let (_, view_handle) = app.add_window(WindowStyle::NotStealFocus, KeybindingsView::new);
        app.update(|ctx| {
            view_handle.update(ctx, |view, ctx| {
                view.on_page_selected(false, ctx);
            });
        });

        let index = app.read(|ctx| {
            view_handle
                .as_ref(ctx)
                .rows
                .as_ref()
                .unwrap()
                .iter()
                .position(|row| row.binding.name == SHARED_NAME && row.binding.trigger.is_some())
                .expect("expected a row for the registration with a real default")
        });
        let other_id = app.read(|ctx| {
            view_handle
                .as_ref(ctx)
                .rows
                .as_ref()
                .unwrap()
                .iter()
                .find(|row| row.binding.name == SHARED_NAME && row.binding.trigger.is_none())
                .expect("expected a row for the registration without a default")
                .binding
                .id
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

        let expected_keystroke = Keystroke::parse(DEFAULT_KEYSTROKE).unwrap();
        app.read(|ctx| {
            let view = view_handle.as_ref(ctx);
            let row = &view.rows.as_ref().unwrap()[index];
            assert_eq!(
                row.binding.trigger,
                Some(expected_keystroke.clone()),
                "the row for the non-empty registration should show its own restored default"
            );

            let cached = view
                .bindings
                .as_ref()
                .unwrap()
                .iter()
                .find(|binding| binding.id == row.binding.id)
                .expect("the edited binding should still be present in the cache");
            assert_eq!(
                cached.trigger,
                Some(expected_keystroke.clone()),
                "the internal binding-list cache should be updated for the edited registration"
            );

            let other_cached = view
                .bindings
                .as_ref()
                .unwrap()
                .iter()
                .find(|binding| binding.id == other_id)
                .expect("the other same-named registration should still be present");
            assert_eq!(
                other_cached.trigger, None,
                "the other same-named registration's own (empty) default must be unaffected"
            );
        });

        let handled = app
            .dispatch_keystroke(
                probe_window_id,
                &[probe_view_id],
                &expected_keystroke,
                false,
            )
            .expect("dispatch should succeed");
        assert!(
            handled,
            "the restored default keystroke should dispatch through the real matcher"
        );
        assert_eq!(
            dispatched.borrow().as_slice(),
            [ProbeAction::NonEmptyDefault],
            "the keystroke should dispatch the non-empty registration's action, not the other's"
        );
    });
}
