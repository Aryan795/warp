use std::cell::Cell;
use std::rc::Rc;

use ai::LLMProvider;
use ai::api_keys::ApiKeyManager;
use warp::tui_export::{UserWorkspaces, register_tui_session_view_test_singletons};
use warp_core::features::FeatureFlag;
use warpui::SingletonEntity as _;
use warpui_core::elements::tui::{TuiElement, TuiText};
use warpui_core::{AddWindowOptions, App, AppContext, Entity, TuiView, TypedActionView};

use super::*;

/// Stands in for the terminal surface so [`model_credential_icon_resolver`] has a real window
/// to resolve, without pulling in `TuiTerminalSessionView`'s full construction. Unlike
/// `RootTuiView`, this double does not register its own window on construction, so a caller
/// that needs the window to read as *known, no team* (rather than *unknown* -- see REV-2205)
/// must call `UserWorkspaces::register_window` itself.
struct ModelMenuTestSurface;

impl Entity for ModelMenuTestSurface {
    type Event = ();
}

impl TypedActionView for ModelMenuTestSurface {
    type Action = ();
}

impl TuiView for ModelMenuTestSurface {
    fn ui_name() -> &'static str {
        "ModelMenuTestSurface"
    }

    fn render(&self, _ctx: &AppContext) -> Box<dyn TuiElement> {
        TuiText::from_spans(Vec::new()).finish()
    }
}

fn row(
    id: &str,
    is_selectable: bool,
    is_key_connected: bool,
    is_profile_default: bool,
) -> TuiModelMenuRow {
    TuiModelMenuRow {
        id: id.into(),
        title: id.to_owned(),
        is_selectable,
        is_key_connected,
        is_profile_default,
        discount_percentage: None,
    }
}

#[test]
fn empty_query_prefers_active_model_and_filtered_query_prefers_best_match() {
    let rows = vec![
        row("auto", true, false, false),
        row("gpt-4", true, false, false),
        row("gpt-5", true, false, false),
    ];

    assert_eq!(
        preferred_selection_index(&rows, &LLMId::from("gpt-4"), true),
        Some(1)
    );
    assert_eq!(
        preferred_selection_index(&rows, &LLMId::from("gpt-4"), false),
        Some(2)
    );
}

#[test]
fn model_selection_skips_disabled_rows() {
    let rows = vec![
        row("auto", true, false, false),
        row("gpt-5", true, false, false),
        row("disabled", false, false, false),
    ];

    assert_eq!(
        preferred_selection_index(&rows, &LLMId::from("disabled"), true),
        Some(1)
    );
    assert_eq!(
        preferred_selection_index(&rows, &LLMId::from("auto"), false),
        Some(1)
    );
}

#[test]
fn snapshot_marks_only_key_connected_models() {
    let connected = snapshot_row(&row("gpt-5", true, true, false));
    assert_eq!(connected.state_suffix.as_deref(), Some("(key connected)"));
    let hosted = snapshot_row(&row("auto", true, false, false));
    assert_eq!(hosted.state_suffix, None);
}
#[test]
fn snapshot_marks_the_profile_default_model() {
    let default = snapshot_row(&row("auto", true, false, true));
    assert_eq!(default.state_suffix.as_deref(), Some("(default)"));

    let connected_default = snapshot_row(&row("gpt-5", true, true, true));
    assert_eq!(
        connected_default.state_suffix.as_deref(),
        Some("(default) (key connected)")
    );
}

#[test]
fn provider_key_controls_key_connected_callout() {
    App::test((), |mut app| async move {
        let _byok = FeatureFlag::SoloUserByok.override_enabled(true);
        register_tui_session_view_test_singletons(&mut app);
        let (window_id, surface) = app.update(|ctx| {
            ctx.add_tui_window(AddWindowOptions::default(), |_| ModelMenuTestSurface)
        });
        // A real TUI window is registered (possibly teamless) the moment `RootTuiView`
        // constructs -- see its own `register_window` call. This test double bypasses that
        // constructor, so it registers the window itself to model the same "known, no team"
        // state rather than leaving `UserWorkspaces` with no entry for it at all, which reads
        // as *unknown* (REV-2205) and would report no icon regardless of entitlement.
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.register_window(window_id, None, ctx);
        });
        let credential_icons = model_credential_icon_resolver(surface.downgrade());
        let mut llm = app.read(|ctx| {
            LLMPreferences::as_ref(ctx)
                .get_active_base_model(ctx, None)
                .clone()
        });
        llm.provider = LLMProvider::OpenAI;

        ApiKeyManager::handle(&app)
            .update(&mut app, |manager, ctx| {
                manager.persist_provider_key(LLMProvider::OpenAI, Some("test-key".to_owned()), ctx)
            })
            .unwrap();
        let connected_row = app.read(|ctx| {
            let choice = query_model_picker_choices(
                LLMPreferences::as_ref(ctx),
                [&llm],
                "",
                |llm, app| credential_icons(llm, app).is_key_connected,
                ctx,
            )
            .remove(0);
            model_menu_row(
                choice,
                &LLMId::from("profile-default"),
                &credential_icons,
                ctx,
            )
        });
        assert_eq!(
            snapshot_row(&connected_row).state_suffix.as_deref(),
            Some("(key connected)")
        );

        ApiKeyManager::handle(&app)
            .update(&mut app, |manager, ctx| {
                manager.persist_provider_key(LLMProvider::OpenAI, None, ctx)
            })
            .unwrap();
        let disconnected_row = app.read(|ctx| {
            let choice = query_model_picker_choices(
                LLMPreferences::as_ref(ctx),
                [&llm],
                "",
                |llm, app| credential_icons(llm, app).is_key_connected,
                ctx,
            )
            .remove(0);
            model_menu_row(
                choice,
                &LLMId::from("profile-default"),
                &credential_icons,
                ctx,
            )
        });
        assert_eq!(snapshot_row(&disconnected_row).state_suffix, None);
    });
}

/// The credential/host icons in an open menu are scoped to the terminal surface's window
/// team, so switching that window's team while the menu is open must repaint it -- not leave
/// it showing the previous team's BYO/host state until the menu is closed and reopened.
/// Verified by counting `TuiModelMenuEvent` emissions rather than inspecting row content,
/// since the emission is exactly what `refresh_rows`'s new subscription contributes.
#[test]
fn open_menu_repaints_when_its_own_window_switches_team() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        let (window_id, surface) = app.update(|ctx| {
            ctx.add_tui_window(AddWindowOptions::default(), |_| ModelMenuTestSurface)
        });
        let (_other_window_id, _other_surface) = app.update(|ctx| {
            ctx.add_tui_window(AddWindowOptions::default(), |_| ModelMenuTestSurface)
        });

        let input_editor = app.update(|ctx| ctx.add_model(|ctx| CodeEditorModel::new_tui(80, ctx)));
        let suggestions_mode =
            app.update(|ctx| ctx.add_model(|_| TuiInputSuggestionsModeModel::new()));
        let terminal_view_id = EntityId::new();
        let menu = app.update(|ctx| {
            ctx.add_model(|ctx| {
                TuiModelMenuModel::new(
                    input_editor.clone(),
                    suggestions_mode.clone(),
                    terminal_view_id,
                    surface.downgrade(),
                    ctx,
                )
            })
        });
        menu.update(&mut app, |menu, ctx| menu.open(ctx));

        let repaint_count = Rc::new(Cell::new(0));
        let repaint_count_for_subscription = repaint_count.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&menu, move |_, _: &TuiModelMenuEvent, _| {
                repaint_count_for_subscription.set(repaint_count_for_subscription.get() + 1);
            });
        });

        let unrelated_team = 111.into();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.switch_window_to_team(_other_window_id, unrelated_team, ctx);
        });
        assert_eq!(
            repaint_count.get(),
            0,
            "a team switch on a different window must not repaint this menu"
        );

        let this_window_team = 222.into();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.switch_window_to_team(window_id, this_window_team, ctx);
        });
        assert_eq!(
            repaint_count.get(),
            1,
            "switching this menu's own window to a different team must repaint it"
        );
    });
}
