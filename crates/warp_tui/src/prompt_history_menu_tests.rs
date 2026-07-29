//! Tests for [`TuiPromptHistoryMenuModel`]: combined prompt + command
//! population/ordering/dedupe, default selection and preview, prefix
//! filtering, buffer snapshot/restore, mode-aware content, acceptance
//! (command vs prompt), empty/no-match states, the `History` header, and the
//! green `!` shell-command row affordance.
use std::rc::Rc;

use warp::editor::CodeEditorModel;
use warp::settings::AISettingsChangedEvent;
use warp::tui_export::{
    BlocklistAIInputModel, ConversationSelectionEvent, InputConfig, InputModePolicy,
    PolicyConfigUpdate, blocklist_ai_history_model_with_queries,
    register_tui_history_test_singletons, register_tui_session_view_test_singletons,
};
use warp_core::features::FeatureFlag;
use warp_editor::model::CoreEditorModel;
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, EntityId, ModelHandle};

use super::{
    TuiHistoryAcceptedItem, TuiHistoryKind, TuiPromptHistoryMenuModel, TuiPromptHistoryRow,
    reconciled_selection_index,
};
use crate::inline_menu::{
    TuiInlineMenuHeader, TuiInlineMenuRow, TuiInlineMenuRowStyle, TuiInlineMenuSnapshot,
    TuiInlineMenuStatus, render_inline_menu, single_line_menu_title,
};
use crate::input_mode_policy::AI_LOCKED_CONFIG;
use crate::input_suggestions_mode::TuiInputSuggestionsModeModel;
use crate::test_fixtures::add_test_active_session;
use crate::tui_builder::TuiUiBuilder;

const W: u16 = 80;

/// A test input-mode policy that starts in agent (AI) mode, so the menu's
/// open-time `include_prompts` snapshot is `true`.
struct TestInputModePolicy;

impl InputModePolicy for TestInputModePolicy {
    fn initial_config(&self, _: &AppContext) -> InputConfig {
        AI_LOCKED_CONFIG
    }
    fn allows_locked_ai_input(&self, _: &AppContext) -> bool {
        true
    }
    fn is_autodetection_enabled(&self, _: &AppContext) -> bool {
        false
    }
    fn config_on_conversation_selection_changed(
        &self,
        _: &ConversationSelectionEvent,
        _: InputConfig,
        _: &AppContext,
    ) -> Option<PolicyConfigUpdate> {
        None
    }
    fn config_on_ai_settings_changed(
        &self,
        _: &AISettingsChangedEvent,
        _: InputConfig,
        _: bool,
        _: &AppContext,
    ) -> Option<PolicyConfigUpdate> {
        None
    }
}

/// Registers the singletons the combined history getter reads (`History`,
/// `IgnoredSuggestionsModel`, `AISettings`, auth, etc.) and seeds a
/// `BlocklistAIHistoryModel` with `prompts` (oldest-first). Must run on the
/// `App` before the `app.update` that builds the menu.
fn prepare_history_app(app: &mut App, prompts: &[&str]) {
    let prompts_owned: Vec<String> = prompts.iter().map(|prompt| (*prompt).to_owned()).collect();
    // Seeded model first so `register_tui_session_view_test_singletons` skips
    // its empty `BlocklistAIHistoryModel` default.
    app.add_singleton_model(move |_| blocklist_ai_history_model_with_queries(prompts_owned));
    register_tui_session_view_test_singletons(app);
    register_tui_history_test_singletons(app);
}

/// Builds a closed history menu over a fresh editor, an AI-locked input mode,
/// and an empty `ActiveSession` (no bootstrapped session, so commands are
/// empty). Run inside `app.update`.
fn build_menu(
    ctx: &mut AppContext,
) -> (
    ModelHandle<CodeEditorModel>,
    ModelHandle<TuiPromptHistoryMenuModel>,
) {
    let input_model = ctx.add_model(|ctx| CodeEditorModel::new_tui(W, ctx));
    let input_mode = BlocklistAIInputModel::mock(Rc::new(TestInputModePolicy), ctx);
    let suggestions_mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
    let active_session = add_test_active_session(ctx);
    let menu = ctx.add_model(|ctx| {
        TuiPromptHistoryMenuModel::new(
            input_model.clone(),
            input_mode,
            active_session,
            suggestions_mode.clone(),
            EntityId::new(),
            ctx,
        )
    });
    (input_model, menu)
}

fn set_text(input_model: &ModelHandle<CodeEditorModel>, text: &str, ctx: &mut AppContext) {
    input_model.update(ctx, |editor, ctx| {
        editor.clear_buffer(ctx);
        editor.user_insert(text, ctx);
    });
}

fn buffer_text(input_model: &ModelHandle<CodeEditorModel>, ctx: &AppContext) -> String {
    let buffer = input_model.as_ref(ctx).content().as_ref(ctx);
    if buffer.is_empty() {
        String::new()
    } else {
        buffer.text().into_string()
    }
}

fn row_titles(menu: &ModelHandle<TuiPromptHistoryMenuModel>, ctx: &AppContext) -> Vec<String> {
    menu.as_ref(ctx)
        .snapshot(ctx)
        .map(|snapshot| snapshot.rows.iter().map(|row| row.title.clone()).collect())
        .unwrap_or_default()
}

#[test]
fn open_populates_ordered_deduped_rows_excluding_whitespace() {
    App::test((), |mut app| async move {
        let _agent_mode = FeatureFlag::AgentMode.override_enabled(true);
        prepare_history_app(&mut app, &["deploy", "test", "deploy", "   ", "build"]);
        app.update(|ctx| {
            let (_input, menu) = build_menu(ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));

            assert!(menu.as_ref(ctx).is_open(ctx));
            // "deploy" is duplicated (newer wins), the whitespace-only prompt is
            // dropped; oldest-first after dedup keeps the newest occurrence.
            assert_eq!(
                row_titles(&menu, ctx),
                vec!["test".to_owned(), "deploy".to_owned(), "build".to_owned()]
            );
        });
    });
}

#[test]
fn open_selects_and_previews_last_row() {
    App::test((), |mut app| async move {
        let _agent_mode = FeatureFlag::AgentMode.override_enabled(true);
        prepare_history_app(&mut app, &["first", "second", "third"]);
        app.update(|ctx| {
            let (input, menu) = build_menu(ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));
            let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            assert_eq!(snapshot.selected_index, Some(2));
            assert_eq!(buffer_text(&input, ctx), "third");
        });
    });
}

#[test]
fn open_with_typed_text_prefix_filters_rows() {
    App::test((), |mut app| async move {
        let _agent_mode = FeatureFlag::AgentMode.override_enabled(true);
        prepare_history_app(&mut app, &["deploy the app", "delete cache", "build"]);
        app.update(|ctx| {
            let (input, menu) = build_menu(ctx);
            set_text(&input, "de", ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));
            assert_eq!(
                row_titles(&menu, ctx),
                vec!["deploy the app".to_owned(), "delete cache".to_owned()]
            );
        });
    });
}

#[test]
fn typed_text_prefix_matches_any_prompt_line() {
    App::test((), |mut app| async move {
        let _agent_mode = FeatureFlag::AgentMode.override_enabled(true);
        let prompt = "deploy the app\nverify the deployment";
        prepare_history_app(&mut app, &[prompt, "unrelated prompt"]);
        app.update(|ctx| {
            let (input, menu) = build_menu(ctx);
            set_text(&input, "verify", ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));

            assert_eq!(row_titles(&menu, ctx), vec!["deploy the app..."]);
            assert_eq!(buffer_text(&input, ctx), prompt);
        });
    });
}

#[test]
fn dismiss_restores_the_original_buffer() {
    App::test((), |mut app| async move {
        let _agent_mode = FeatureFlag::AgentMode.override_enabled(true);
        prepare_history_app(&mut app, &["deploy the app"]);
        let (input, menu) = app.update(|ctx| {
            let (input, menu) = build_menu(ctx);
            set_text(&input, "de", ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));
            assert_eq!(buffer_text(&input, ctx), "deploy the app");
            (input, menu)
        });
        app.update(|ctx| {
            menu.update(ctx, |m, ctx| m.dismiss(ctx));
            assert!(!menu.as_ref(ctx).is_open(ctx));
            assert_eq!(buffer_text(&input, ctx), "de");
        });
    });
}

#[test]
fn accept_selected_returns_highlighted_prompt_and_closes() {
    App::test((), |mut app| async move {
        let _agent_mode = FeatureFlag::AgentMode.override_enabled(true);
        prepare_history_app(&mut app, &["older prompt", "newest prompt"]);
        app.update(|ctx| {
            let (_input, menu) = build_menu(ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));
            // Default selection is the newest (last) row.
            let accepted = menu.update(ctx, |m, ctx| m.accept_selected(ctx));
            assert_eq!(
                accepted,
                Some(TuiHistoryAcceptedItem {
                    text: "newest prompt".to_owned(),
                    kind: TuiHistoryKind::Prompt,
                })
            );
            assert!(!menu.as_ref(ctx).is_open(ctx));
        });
    });
}

#[test]
fn accept_on_empty_list_is_a_noop() {
    App::test((), |mut app| async move {
        let _agent_mode = FeatureFlag::AgentMode.override_enabled(true);
        prepare_history_app(&mut app, &[]);
        app.update(|ctx| {
            let (_input, menu) = build_menu(ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));
            // No rows => Enter is a no-op: nothing accepted, menu stays open.
            let accepted = menu.update(ctx, |m, ctx| m.accept_selected(ctx));
            assert_eq!(accepted, None);
            assert!(menu.as_ref(ctx).is_open(ctx));
        });
    });
}

#[test]
fn accept_on_filtered_to_nothing_list_is_a_noop() {
    App::test((), |mut app| async move {
        let _agent_mode = FeatureFlag::AgentMode.override_enabled(true);
        prepare_history_app(&mut app, &["deploy the app"]);
        app.update(|ctx| {
            let (input, menu) = build_menu(ctx);
            set_text(&input, "no match", ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));
            let accepted = menu.update(ctx, |m, ctx| m.accept_selected(ctx));
            assert_eq!(accepted, None);
            assert!(menu.as_ref(ctx).is_open(ctx));
        });
    });
}

#[test]
fn multiline_prompt_uses_single_line_title_without_changing_prompt_text() {
    App::test((), |mut app| async move {
        let _agent_mode = FeatureFlag::AgentMode.override_enabled(true);
        let prompt = "deploy the app\nthen verify it";
        prepare_history_app(&mut app, &[prompt]);
        app.update(|ctx| {
            let (input, menu) = build_menu(ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));

            let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            assert_eq!(snapshot.rows[0].title, "deploy the app...");
            assert_eq!(buffer_text(&input, ctx), prompt);
            assert_eq!(
                menu.update(ctx, |m, ctx| m.accept_selected(ctx)),
                Some(TuiHistoryAcceptedItem {
                    text: prompt.to_owned(),
                    kind: TuiHistoryKind::Prompt,
                })
            );
        });
    });
}

#[test]
fn prompt_history_title_handles_windows_line_endings() {
    assert_eq!(
        single_line_menu_title("deploy the app\r\nthen verify it"),
        "deploy the app..."
    );
}

#[test]
fn empty_history_shows_explicit_empty_state() {
    App::test((), |mut app| async move {
        let _agent_mode = FeatureFlag::AgentMode.override_enabled(true);
        prepare_history_app(&mut app, &[]);
        app.update(|ctx| {
            let (_input, menu) = build_menu(ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));
            let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            assert!(snapshot.rows.is_empty());
            assert!(matches!(
                snapshot.status,
                Some(TuiInlineMenuStatus::Empty(label)) if label == "No history"
            ));
        });
    });
}

#[test]
fn filtered_to_nothing_shows_no_matching_history_state() {
    App::test((), |mut app| async move {
        let _agent_mode = FeatureFlag::AgentMode.override_enabled(true);
        prepare_history_app(&mut app, &["deploy the app"]);
        app.update(|ctx| {
            let (input, menu) = build_menu(ctx);
            set_text(&input, "no match", ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));
            let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            assert!(snapshot.rows.is_empty());
            assert!(matches!(
                snapshot.status,
                Some(TuiInlineMenuStatus::Empty(label)) if label == "No matching history"
            ));
        });
    });
}

#[test]
fn down_dismisses_empty_history() {
    App::test((), |mut app| async move {
        let _agent_mode = FeatureFlag::AgentMode.override_enabled(true);
        prepare_history_app(&mut app, &[]);
        let (input, _menu) = app.update(|ctx| {
            let (input, menu) = build_menu(ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));
            menu.update(ctx, |m, ctx| m.select_next(ctx));
            assert!(!menu.as_ref(ctx).is_open(ctx));
            (input, menu)
        });
        app.read(|ctx| {
            assert_eq!(buffer_text(&input, ctx), "");
        });
    });
}

#[test]
fn down_dismisses_filtered_to_empty_history_and_restores_query() {
    App::test((), |mut app| async move {
        let _agent_mode = FeatureFlag::AgentMode.override_enabled(true);
        prepare_history_app(&mut app, &["deploy the app"]);
        let (input, menu) = app.update(|ctx| {
            let (input, menu) = build_menu(ctx);
            set_text(&input, "no match", ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));
            menu.update(ctx, |m, ctx| m.select_next(ctx));
            (input, menu)
        });
        app.read(|ctx| {
            assert!(!menu.as_ref(ctx).is_open(ctx));
            assert_eq!(buffer_text(&input, ctx), "no match");
        });
    });
}

#[test]
fn open_menu_renders_history_surface_to_lines() {
    App::test((), |mut app| async move {
        let _agent_mode = FeatureFlag::AgentMode.override_enabled(true);
        prepare_history_app(&mut app, &["deploy the app", "run the tests"]);
        app.update(|ctx| {
            let (_input, menu) = build_menu(ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));
            let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                render_inline_menu(&snapshot, &TuiUiBuilder::from_app(ctx)),
                TuiRect::new(0, 0, 50, 12),
                ctx,
            );
            let rendered = frame.buffer.to_lines().join("\n");
            assert!(
                rendered.contains("History"),
                "rendered menu should show the header:\n{rendered}"
            );
            assert!(rendered.contains("deploy the app"));
            assert!(rendered.contains("run the tests"));
        });
    });
}

#[test]
fn command_rows_use_shell_command_style_and_prompts_use_default() {
    App::test((), |mut app| async move {
        // With an empty ActiveSession the menu has no rows, so this test builds a
        // snapshot directly to pin the row-style -> `!` affordance mapping.
        let snapshot = TuiInlineMenuSnapshot {
            header: Some(TuiInlineMenuHeader {
                title: Some("History".to_owned()),
                tabs: Vec::new(),
            }),
            rows: vec![
                TuiInlineMenuRow {
                    title: "ls -la".to_owned(),
                    description: None,
                    state_suffix: None,
                    is_selectable: true,
                    style: TuiInlineMenuRowStyle::ShellCommand,
                },
                TuiInlineMenuRow {
                    title: "deploy the app".to_owned(),
                    description: None,
                    state_suffix: None,
                    is_selectable: true,
                    style: TuiInlineMenuRowStyle::Default,
                },
            ],
            selected_index: None,
            scroll_offset: 0,
            max_visible_rows: 10,
            status: None,
        };
        app.add_singleton_model(|_| warp::appearance::Appearance::mock());
        app.update(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                render_inline_menu(&snapshot, &builder),
                TuiRect::new(0, 0, 50, 8),
                ctx,
            );
            let lines = frame.buffer.to_lines();
            let rendered = lines.join("\n");
            // The command row leads with the green `!` affordance; the prompt
            // row has no `!` prefix.
            assert!(rendered.contains("! ls -la"), "command row: {rendered}");
            assert!(
                !rendered.contains("! deploy the app"),
                "prompt row must not have a `!` prefix: {rendered}"
            );

            // The `!` glyph uses the green shell-command accent (bright green),
            // not the pale-green transcript background.
            let green = builder
                .shell_command_accent_style()
                .fg
                .expect("shell command accent has a foreground");
            // The command row sits one row below the "History" header.
            let bang_cell = &frame.buffer[(0, 1)];
            assert_eq!(bang_cell.fg, green);
            assert_eq!(
                bang_cell.bg,
                warpui_core::elements::tui::Color::Reset,
                "command rows must not carry the transcript background"
            );
        });
    });
}

#[test]
fn reconciled_selection_prefers_text_then_index_then_last_row() {
    let rows = vec![
        TuiPromptHistoryRow {
            text: "one".to_owned(),
            kind: TuiHistoryKind::Prompt,
        },
        TuiPromptHistoryRow {
            text: "two".to_owned(),
            kind: TuiHistoryKind::Prompt,
        },
        TuiPromptHistoryRow {
            text: "three".to_owned(),
            kind: TuiHistoryKind::Prompt,
        },
    ];

    // Stable selection by text wins over the previous index.
    assert_eq!(
        reconciled_selection_index(&rows, Some("two"), Some(0)),
        Some(1)
    );
    // No text match falls back to the (clamped) previous index.
    assert_eq!(
        reconciled_selection_index(&rows[..2], Some("gone"), Some(5)),
        Some(1)
    );
    // No prior selection defaults to the last (most-recent) row.
    assert_eq!(reconciled_selection_index(&rows, None, None), Some(2));
    // An empty list has nothing to select.
    assert_eq!(reconciled_selection_index(&[], Some("x"), Some(0)), None);
}
