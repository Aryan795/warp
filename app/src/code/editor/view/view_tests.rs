use std::sync::Arc;

use warp_core::features::FeatureFlag;
use warp_core::settings::Setting as _;
use warp_core::ui::appearance::Appearance;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_util::user_input::UserInput;
use warpui::elements::ScrollbarWidth;
use warpui::elements::new_scrollable::ScrollableAppearance;
use warpui::platform::WindowStyle;
use warpui::{App, SingletonEntity, TypedActionView, UpdateModel, ViewHandle, WindowId};

use super::{CodeEditorRenderOptions, CodeEditorView, CodeEditorViewAction};
use crate::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::code::editor::find::view::FindAction;
use crate::editor::{EditorAction, InteractionState};
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::settings::AppEditorSettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::vim_registers::VimRegisters;
use crate::workspace::ActiveSession;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspaces::user_workspaces::UserWorkspaces;

fn initialize_editor(app: &mut App) -> (WindowId, ViewHandle<CodeEditorView>) {
    initialize_settings_for_tests(app);

    // Add all required singleton models for EditorView dependencies
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| SyncedInputState::mock());
    app.add_singleton_model(|_| VimRegisters::new());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());

    // Add mocks required by rich text editor (used in CommentEditor)
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(NotebookKeybindings::new);

    // Add UserWorkspaces mock (required by EditorView)
    let team_client_mock = Arc::new(MockTeamClient::new());
    let workspace_client_mock = Arc::new(MockWorkspaceClient::new());
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            team_client_mock.clone(),
            workspace_client_mock.clone(),
            vec![],
            ctx,
        )
    });

    let (window, editor_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        CodeEditorView::new(
            None,
            None,
            CodeEditorRenderOptions::new(VerticalExpansionBehavior::GrowToMaxHeight),
            ctx,
        )
        .with_horizontal_scrollbar_appearance(ScrollableAppearance::new(ScrollbarWidth::Auto, true))
    });

    (window, editor_view)
}

#[test]
fn test_interaction_state_prevents_editing() {
    App::test((), |mut app| async move {
        let (_window, editor_view) = initialize_editor(&mut app);

        let text = editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::UserTyped(UserInput::new("abc")), ctx);
            view.text(ctx)
        });

        assert_eq!(text.as_str(), "abc");

        // Set to be only selectable
        editor_view.update(&mut app, |view, ctx| {
            view.set_interaction_state(InteractionState::Selectable, ctx);
        });

        let text = editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::UserTyped(UserInput::new("def")), ctx);
            view.text(ctx)
        });

        assert_eq!(text.as_str(), "abc");
    });
}

/// Regression test for the find bar query field becoming permanently unclickable after Vim's
/// Enter handling disables it. Clicking the field (simulated here via `FindAction::ClickQueryField`)
/// should restore it to an editable, focused state.
#[test]
fn test_vim_find_query_field_click_restores_editability() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        let (_window, editor_view) = initialize_editor(&mut app);

        // Enable Vim mode.
        app.update_model(
            &AppEditorSettings::handle(&app),
            |settings: &mut AppEditorSettings, ctx| {
                settings.vim_mode.set_value(true, ctx).unwrap();
            },
        );

        // Opening the find bar focuses the query field and makes it editable.
        editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::ShowFindBar, ctx);
        });

        let find_bar = editor_view
            .read(&app, |view, _| view.find_bar.clone())
            .expect("find bar should be available");
        let query_field = find_bar.read(&app, |find_bar, _| find_bar.find_editor_handle_for_test());

        find_bar.update(&mut app, |find_bar, ctx| {
            find_bar.set_find_query(ctx, "hello");
        });

        // Simulate pressing Enter in the query field. In Vim mode, this ends query entry:
        // the field becomes non-editable and focus moves to the main editor.
        query_field.update(&mut app, |editor, ctx| {
            editor.handle_action(&EditorAction::Enter, ctx);
        });

        assert!(!find_bar.read(&app, |find_bar, app| find_bar.is_find_input_editable(app)));
        assert!(!app.read(|ctx| query_field.is_focused(ctx)));
        assert!(app.read(|ctx| editor_view.is_focused(ctx)));

        // Clicking the query field should restore it to an editable, focused state.
        find_bar.update(&mut app, |find_bar, ctx| {
            find_bar.handle_action(&FindAction::ClickQueryField, ctx);
        });

        assert!(find_bar.read(&app, |find_bar, app| find_bar.is_find_input_editable(app)));
        assert!(app.read(|ctx| query_field.is_focused(ctx)));
    });
}
