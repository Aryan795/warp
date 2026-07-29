//! Tests for [`TuiPromptHistoryMenuModel`]: population/ordering/dedupe, default
//! selection and initial preview, prefix filtering, buffer snapshot/restore,
//! acceptance, and empty states.
use std::rc::Rc;
use warp::appearance::Appearance;
use warp::editor::CodeEditorModel;
use warp::settings::AISettingsChangedEvent;
use warp::tui_export::{
    BlocklistAIHistoryModel, BlocklistAIInputModel, ConversationSelectionEvent, InputConfig,
    InputModePolicy, InputType, PolicyConfigUpdate, TuiHistoryEntry, TuiHistoryEntryKind,
};
use warp_editor::model::CoreEditorModel;
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, EntityId, ModelHandle};

use super::{TuiPromptHistoryMenuModel, TuiPromptHistoryRow, reconciled_selection_index};
use crate::inline_menu::{render_inline_menu, single_line_menu_title};
use crate::input_mode_policy::{AI_LOCKED_CONFIG, SHELL_LOCKED_CONFIG};
use crate::input_suggestions_mode::TuiInputSuggestionsModeModel;
use crate::tui_builder::TuiUiBuilder;

const W: u16 = 80;
struct TestInputModePolicy;

impl InputModePolicy for TestInputModePolicy {
    fn initial_config(&self, _app: &AppContext) -> InputConfig {
        AI_LOCKED_CONFIG
    }

    fn allows_locked_ai_input(&self, _app: &AppContext) -> bool {
        true
    }

    fn is_autodetection_enabled(&self, _app: &AppContext) -> bool {
        false
    }

    fn config_on_conversation_selection_changed(
        &self,
        _event: &ConversationSelectionEvent,
        _current: InputConfig,
        _app: &AppContext,
    ) -> Option<PolicyConfigUpdate> {
        None
    }

    fn config_on_ai_settings_changed(
        &self,
        _event: &AISettingsChangedEvent,
        _current: InputConfig,
        _is_autodetection_enabled_for_current_context: bool,
        _app: &AppContext,
    ) -> Option<PolicyConfigUpdate> {
        None
    }
}

#[test]
fn unlocked_auto_detected_shell_input_keeps_combined_history() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (_input, menu) = setup_history_with_config(
                ctx,
                vec![prompt("explain this"), command("cargo test")],
                InputConfig {
                    input_type: InputType::Shell,
                    is_locked: false,
                },
            );
            menu.update(ctx, |m, ctx| m.open(ctx));

            assert_eq!(
                row_titles(&menu, ctx),
                vec!["explain this".to_owned(), "cargo test".to_owned()]
            );
        });
    });
}

fn prompt(text: &str) -> TuiHistoryEntry {
    TuiHistoryEntry {
        kind: TuiHistoryEntryKind::Prompt,
        text: text.to_owned(),
    }
}

fn command(text: &str) -> TuiHistoryEntry {
    TuiHistoryEntry {
        kind: TuiHistoryEntryKind::Command,
        text: text.to_owned(),
    }
}

/// Builds a closed prompt-history menu over a fresh editor and a history model
/// seeded with `prompts` (oldest-first).
fn setup(
    ctx: &mut AppContext,
    prompts: &[&str],
) -> (
    ModelHandle<CodeEditorModel>,
    ModelHandle<TuiPromptHistoryMenuModel>,
) {
    setup_history(ctx, prompts.iter().map(|text| prompt(text)).collect())
}

fn setup_history(
    ctx: &mut AppContext,
    history: Vec<TuiHistoryEntry>,
) -> (
    ModelHandle<CodeEditorModel>,
    ModelHandle<TuiPromptHistoryMenuModel>,
) {
    setup_history_with_config(ctx, history, AI_LOCKED_CONFIG)
}

fn setup_history_with_config(
    ctx: &mut AppContext,
    history: Vec<TuiHistoryEntry>,
    input_config: InputConfig,
) -> (
    ModelHandle<CodeEditorModel>,
    ModelHandle<TuiPromptHistoryMenuModel>,
) {
    ctx.add_singleton_model(|_| Appearance::mock());
    ctx.add_singleton_model(|_| BlocklistAIHistoryModel::default());
    let input_model = ctx.add_model(|ctx| CodeEditorModel::new_tui(W, ctx));
    let input_mode = BlocklistAIInputModel::mock(Rc::new(TestInputModePolicy), ctx);
    input_mode.update(ctx, |input_mode, ctx| {
        input_mode.set_input_config(input_config, true, None, ctx);
    });
    let suggestions_mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
    let menu = ctx.add_model(|ctx| {
        TuiPromptHistoryMenuModel::new_for_test(
            input_model.clone(),
            input_mode,
            suggestions_mode.clone(),
            EntityId::new(),
            history,
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
fn open_preserves_shared_history_projection_order() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (_input, menu) = setup_history(
                ctx,
                vec![prompt("test"), command("cargo test"), prompt("build")],
            );
            menu.update(ctx, |m, ctx| m.open(ctx));

            assert!(menu.as_ref(ctx).is_open(ctx));
            assert_eq!(
                row_titles(&menu, ctx),
                vec![
                    "test".to_owned(),
                    "cargo test".to_owned(),
                    "build".to_owned()
                ]
            );
        });
    });
}

#[test]
fn agent_history_previews_each_entry_in_its_matching_mode() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (input, menu) = setup_history(
                ctx,
                vec![
                    prompt("explain this"),
                    command("cargo test"),
                    prompt("fix it"),
                ],
            );
            menu.update(ctx, |m, ctx| m.open(ctx));

            assert_eq!(buffer_text(&input, ctx), "fix it");
            assert!(
                menu.as_ref(ctx)
                    .input_mode
                    .as_ref(ctx)
                    .input_config()
                    .is_ai()
            );

            menu.update(ctx, |m, ctx| m.select_previous(ctx));
            assert_eq!(buffer_text(&input, ctx), "cargo test");
            assert!(
                menu.as_ref(ctx)
                    .input_mode
                    .as_ref(ctx)
                    .input_config()
                    .is_shell()
            );
        });
    });
}

#[test]
fn shell_history_excludes_prompts_and_accepts_a_typed_command() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (input, menu) = setup_history_with_config(
                ctx,
                vec![
                    prompt("explain this"),
                    command("cargo test"),
                    prompt("fix it"),
                    command("git status"),
                ],
                SHELL_LOCKED_CONFIG,
            );
            menu.update(ctx, |m, ctx| m.open(ctx));

            assert_eq!(
                row_titles(&menu, ctx),
                vec!["cargo test".to_owned(), "git status".to_owned()]
            );
            assert_eq!(buffer_text(&input, ctx), "git status");
            assert_eq!(
                menu.update(ctx, |m, ctx| m.accept_selected(ctx)),
                Some(command("git status"))
            );
        });
    });
}

#[test]
fn filtered_history_shows_no_match_and_enter_is_a_no_op() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (input, menu) = setup(ctx, &["deploy the app"]);
            set_text(&input, "build", ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));

            let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            assert!(snapshot.rows.is_empty());
            assert_eq!(
                snapshot.status,
                Some(crate::inline_menu::TuiInlineMenuStatus::Empty(
                    "No matching history".to_owned()
                ))
            );
            assert_eq!(menu.update(ctx, |m, ctx| m.accept_selected(ctx)), None);
            assert!(menu.as_ref(ctx).is_open(ctx));
            assert_eq!(buffer_text(&input, ctx), "build");
        });
    });
}

#[test]
fn open_selects_and_previews_last_row() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (input, menu) = setup(ctx, &["first", "second", "third"]);
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
        app.update(|ctx| {
            let (input, menu) = setup(ctx, &["deploy the app", "delete cache", "build"]);
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
        app.update(|ctx| {
            let prompt = "deploy the app\nverify the deployment";
            let (input, menu) = setup(ctx, &[prompt, "unrelated prompt"]);
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
        app.update(|ctx| {
            let (input, menu) = setup(ctx, &["deploy the app"]);
            set_text(&input, "de", ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));
            assert_eq!(buffer_text(&input, ctx), "deploy the app");
            menu.update(ctx, |m, ctx| m.dismiss(ctx));

            assert!(!menu.as_ref(ctx).is_open(ctx));
            assert_eq!(buffer_text(&input, ctx), "de");
        });
    });
}

#[test]
fn accept_selected_returns_highlighted_prompt_and_closes() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (_input, menu) = setup(ctx, &["older prompt", "newest prompt"]);
            menu.update(ctx, |m, ctx| m.open(ctx));
            // Default selection is the newest (last) row.
            let accepted = menu.update(ctx, |m, ctx| m.accept_selected(ctx));
            assert_eq!(accepted, Some(prompt("newest prompt")));
            assert!(!menu.as_ref(ctx).is_open(ctx));
        });
    });
}

#[test]
fn multiline_prompt_uses_single_line_title_without_changing_prompt_text() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let text = "deploy the app\nthen verify it";
            let (input, menu) = setup(ctx, &[text]);
            menu.update(ctx, |m, ctx| m.open(ctx));

            let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            assert_eq!(snapshot.rows[0].title, "deploy the app...");
            assert_eq!(buffer_text(&input, ctx), text);
            assert_eq!(
                menu.update(ctx, |m, ctx| m.accept_selected(ctx)),
                Some(prompt(text))
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
        app.update(|ctx| {
            let (_input, menu) = setup(ctx, &[]);
            menu.update(ctx, |m, ctx| m.open(ctx));
            let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            assert!(snapshot.rows.is_empty());
            assert_eq!(
                snapshot.status,
                Some(crate::inline_menu::TuiInlineMenuStatus::Empty(
                    "No history".to_owned()
                ))
            );
            assert_eq!(menu.update(ctx, |m, ctx| m.accept_selected(ctx)), None);
            assert!(menu.as_ref(ctx).is_open(ctx));
        });
    });
}

#[test]
fn down_dismisses_empty_history() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (input, menu) = setup(ctx, &[]);
            menu.update(ctx, |m, ctx| m.open(ctx));
            menu.update(ctx, |m, ctx| m.select_next(ctx));

            assert!(!menu.as_ref(ctx).is_open(ctx));
            assert_eq!(buffer_text(&input, ctx), "");
        });
    });
}

#[test]
fn down_dismisses_filtered_to_empty_history_and_restores_query() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (input, menu) = setup(ctx, &["deploy the app"]);
            set_text(&input, "no match", ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));
            menu.update(ctx, |m, ctx| m.select_next(ctx));

            assert!(!menu.as_ref(ctx).is_open(ctx));
            assert_eq!(buffer_text(&input, ctx), "no match");
        });
    });
}
#[test]
fn open_menu_renders_prompt_history_surface_to_lines() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (_input, menu) = setup(ctx, &["deploy the app", "run the tests"]);
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
fn reconciled_selection_prefers_text_then_index_then_last_row() {
    let rows = vec![
        TuiPromptHistoryRow {
            entry: prompt("one"),
        },
        TuiPromptHistoryRow {
            entry: prompt("two"),
        },
        TuiPromptHistoryRow {
            entry: prompt("three"),
        },
    ];

    // Stable selection by text wins over the previous index.
    assert_eq!(
        reconciled_selection_index(&rows, Some(&prompt("two")), Some(0)),
        Some(1)
    );
    // No text match falls back to the (clamped) previous index.
    assert_eq!(
        reconciled_selection_index(&rows[..2], Some(&prompt("gone")), Some(5)),
        Some(1)
    );
    // No prior selection defaults to the last (most-recent) row.
    assert_eq!(reconciled_selection_index(&rows, None, None), Some(2));
    // An empty list has nothing to select.
    assert_eq!(
        reconciled_selection_index(&[], Some(&prompt("x")), Some(0)),
        None
    );
}
