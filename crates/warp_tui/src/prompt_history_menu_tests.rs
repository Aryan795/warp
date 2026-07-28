//! Tests for [`TuiPromptHistoryMenuModel`]: population/ordering/dedupe, default
//! selection and initial preview, prefix filtering, buffer snapshot/restore,
//! acceptance, and empty states.
use std::rc::Rc;

use warp::appearance::Appearance;
use warp::editor::CodeEditorModel;
use warp::settings::AISettingsChangedEvent;
use warp::tui_export::{
    BlocklistAIInputModel, ConversationSelectionEvent, InputConfig, InputModePolicy, InputType,
    PolicyConfigUpdate, blocklist_ai_history_model_with_queries,
};
use warp_editor::model::CoreEditorModel;
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, EntityId, ModelHandle};

use super::{
    TuiHistoryAccepted, TuiHistoryRow, TuiPromptHistoryMenuModel, reconciled_selection_index,
};
use crate::inline_menu::{render_inline_menu, single_line_menu_title};
use crate::input_suggestions_mode::TuiInputSuggestionsModeModel;
use crate::tui_builder::TuiUiBuilder;

const W: u16 = 80;

struct TestInputModePolicy;

impl InputModePolicy for TestInputModePolicy {
    fn initial_config(&self, _app: &AppContext) -> InputConfig {
        InputConfig {
            input_type: InputType::AI,
            is_locked: true,
        }
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

/// Builds a closed prompt-history menu over a fresh editor and a history model
/// seeded with `prompts` (oldest-first).
fn setup(
    ctx: &mut AppContext,
    prompts: &[&str],
) -> (
    ModelHandle<CodeEditorModel>,
    ModelHandle<TuiPromptHistoryMenuModel>,
) {
    ctx.add_singleton_model(|_| Appearance::mock());
    ctx.add_singleton_model(|_| {
        blocklist_ai_history_model_with_queries(
            prompts.iter().map(|prompt| (*prompt).to_owned()).collect(),
        )
    });
    let input_model = ctx.add_model(|ctx| CodeEditorModel::new_tui(W, ctx));
    let input_mode = BlocklistAIInputModel::mock(Rc::new(TestInputModePolicy), ctx);
    let suggestions_mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
    let menu = ctx.add_model(|ctx| {
        TuiPromptHistoryMenuModel::new(
            input_model.clone(),
            input_mode.clone(),
            suggestions_mode.clone(),
            EntityId::new(),
            None,
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
        app.update(|ctx| {
            // Oldest-first. "deploy" is duplicated (newer occurrence wins) and a
            // whitespace-only prompt must be dropped.
            let (_input, menu) = setup(ctx, &["deploy", "test", "deploy", "   ", "build"]);
            menu.update(ctx, |m, ctx| m.open(ctx));

            assert!(menu.as_ref(ctx).is_open(ctx));
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
            assert_eq!(
                accepted,
                Some(TuiHistoryAccepted::Prompt {
                    text: "newest prompt".to_owned()
                })
            );
            assert!(!menu.as_ref(ctx).is_open(ctx));
        });
    });
}

#[test]
fn multiline_prompt_uses_single_line_title_without_changing_prompt_text() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let prompt = "deploy the app\nthen verify it";
            let (input, menu) = setup(ctx, &[prompt]);
            menu.update(ctx, |m, ctx| m.open(ctx));

            let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            assert_eq!(snapshot.rows[0].title, "deploy the app...");
            assert_eq!(buffer_text(&input, ctx), prompt);
            assert_eq!(
                menu.update(ctx, |m, ctx| m.accept_selected(ctx)),
                Some(TuiHistoryAccepted::Prompt {
                    text: prompt.to_owned()
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
        app.update(|ctx| {
            let (_input, menu) = setup(ctx, &[]);
            menu.update(ctx, |m, ctx| m.open(ctx));
            let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            assert!(snapshot.rows.is_empty());
            assert!(matches!(
                snapshot.status,
                Some(crate::inline_menu::TuiInlineMenuStatus::Empty(_))
            ));
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
    use warp::tui_export::UpArrowHistoryEntry;
    let rows = vec![
        TuiHistoryRow {
            entry: UpArrowHistoryEntry::Prompt {
                text: "one".to_owned(),
            },
        },
        TuiHistoryRow {
            entry: UpArrowHistoryEntry::Prompt {
                text: "two".to_owned(),
            },
        },
        TuiHistoryRow {
            entry: UpArrowHistoryEntry::Prompt {
                text: "three".to_owned(),
            },
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

#[test]
fn accept_empty_history_is_noop_and_keeps_typed_text() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (input, menu) = setup(ctx, &[]);
            set_text(&input, "typed query", ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));
            let accepted = menu.update(ctx, |m, ctx| m.accept_selected(ctx));
            assert_eq!(accepted, None);
            assert!(!menu.as_ref(ctx).is_open(ctx));
            assert_eq!(buffer_text(&input, ctx), "typed query");
        });
    });
}

#[test]
fn empty_history_status_message_and_header() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (_input, menu) = setup(ctx, &[]);
            menu.update(ctx, |m, ctx| m.open(ctx));
            let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("open");
            assert_eq!(
                snapshot.status,
                Some(crate::inline_menu::TuiInlineMenuStatus::Empty(
                    "No history".to_owned()
                ))
            );
            assert_eq!(
                snapshot
                    .header
                    .as_ref()
                    .and_then(|h| h.title.clone())
                    .as_deref(),
                Some("History")
            );
        });
    });
}

#[test]
fn no_match_history_status_message() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (input, menu) = setup(ctx, &["deploy the app"]);
            set_text(&input, "zzz", ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));
            let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("open");
            assert_eq!(
                snapshot.status,
                Some(crate::inline_menu::TuiInlineMenuStatus::Empty(
                    "No matching history".to_owned()
                ))
            );
        });
    });
}
