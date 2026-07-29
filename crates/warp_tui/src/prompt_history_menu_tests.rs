//! Tests for the combined up-arrow prompt and command history menu.
use std::rc::Rc;

use warp::appearance::Appearance;
use warp::editor::CodeEditorModel;
use warp::settings::AISettingsChangedEvent;
use warp::tui_export::{
    BlocklistAIInputModel, ConversationSelectionEvent, InputConfig, InputModePolicy, InputType,
    PolicyConfigUpdate, TuiHistoryItem,
};
use warp_editor::model::CoreEditorModel;
use warpui_core::elements::tui::{Color, TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, EntityId, ModelHandle};

use super::{TuiPromptHistoryMenuModel, TuiPromptHistoryRow, reconciled_selection_index};
use crate::inline_menu::{render_inline_menu, single_line_menu_title};
use crate::input_mode_policy::{AI_UNLOCKED_CONFIG, SHELL_LOCKED_CONFIG};
use crate::input_suggestions_mode::TuiInputSuggestionsModeModel;
use crate::tui_builder::TuiUiBuilder;

const W: u16 = 80;

struct TestInputModePolicy(InputConfig);

impl InputModePolicy for TestInputModePolicy {
    fn initial_config(&self, _app: &AppContext) -> InputConfig {
        self.0
    }

    fn allows_locked_ai_input(&self, _app: &AppContext) -> bool {
        true
    }

    fn is_autodetection_enabled(&self, _app: &AppContext) -> bool {
        !self.0.is_locked
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

fn setup_items(
    ctx: &mut AppContext,
    items: Vec<TuiHistoryItem>,
    input_config: InputConfig,
) -> (
    ModelHandle<CodeEditorModel>,
    ModelHandle<BlocklistAIInputModel>,
    ModelHandle<TuiPromptHistoryMenuModel>,
) {
    if !ctx.has_singleton_model::<Appearance>() {
        ctx.add_singleton_model(|_| Appearance::mock());
    }
    let input = ctx.add_model(|ctx| CodeEditorModel::new_tui(W, ctx));
    let input_mode = BlocklistAIInputModel::mock(Rc::new(TestInputModePolicy(input_config)), ctx);
    let suggestions_mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
    let menu = ctx.add_model(|ctx| {
        TuiPromptHistoryMenuModel::new_for_test(
            input.clone(),
            input_mode.clone(),
            suggestions_mode,
            EntityId::new(),
            items,
            ctx,
        )
    });
    (input, input_mode, menu)
}

fn setup_prompts(
    ctx: &mut AppContext,
    prompts: &[&str],
) -> (
    ModelHandle<CodeEditorModel>,
    ModelHandle<BlocklistAIInputModel>,
    ModelHandle<TuiPromptHistoryMenuModel>,
) {
    setup_items(
        ctx,
        prompts
            .iter()
            .map(|prompt| TuiHistoryItem::Prompt((*prompt).to_owned()))
            .collect(),
        AI_UNLOCKED_CONFIG,
    )
}

fn set_text(input: &ModelHandle<CodeEditorModel>, text: &str, ctx: &mut AppContext) {
    input.update(ctx, |editor, ctx| {
        editor.clear_buffer(ctx);
        editor.user_insert(text, ctx);
    });
}

fn buffer_text(input: &ModelHandle<CodeEditorModel>, ctx: &AppContext) -> String {
    let buffer = input.as_ref(ctx).content().as_ref(ctx);
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
fn unlocked_agent_mode_interleaves_prompt_and_command_rows() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (_input, _mode, menu) = setup_items(
                ctx,
                vec![
                    TuiHistoryItem::Prompt("explain this error".to_owned()),
                    TuiHistoryItem::Command("cargo test".to_owned()),
                    TuiHistoryItem::Prompt("fix the failure".to_owned()),
                ],
                AI_UNLOCKED_CONFIG,
            );
            menu.update(ctx, |menu, ctx| menu.open(ctx));

            assert_eq!(
                row_titles(&menu, ctx),
                vec!["explain this error", "cargo test", "fix the failure"]
            );
        });
    });
}

#[test]
fn explicit_shell_mode_includes_commands_only() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (_input, _mode, menu) = setup_items(
                ctx,
                vec![
                    TuiHistoryItem::Prompt("explain this error".to_owned()),
                    TuiHistoryItem::Command("cargo test".to_owned()),
                ],
                SHELL_LOCKED_CONFIG,
            );
            menu.update(ctx, |menu, ctx| menu.open(ctx));

            assert_eq!(row_titles(&menu, ctx), vec!["cargo test"]);
        });
    });
}

#[test]
fn same_text_prompt_and_command_remain_distinct_rows() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (_input, _mode, menu) = setup_items(
                ctx,
                vec![
                    TuiHistoryItem::Prompt("status".to_owned()),
                    TuiHistoryItem::Command("status".to_owned()),
                ],
                AI_UNLOCKED_CONFIG,
            );
            menu.update(ctx, |menu, ctx| menu.open(ctx));

            assert_eq!(row_titles(&menu, ctx), vec!["status", "status"]);
        });
    });
}

#[test]
fn typed_prefix_filters_both_types_and_matches_any_line() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (input, _mode, menu) = setup_items(
                ctx,
                vec![
                    TuiHistoryItem::Prompt("deploy app\nverify deployment".to_owned()),
                    TuiHistoryItem::Command("verify cache".to_owned()),
                    TuiHistoryItem::Command("cargo test".to_owned()),
                ],
                AI_UNLOCKED_CONFIG,
            );
            set_text(&input, "verify", ctx);
            menu.update(ctx, |menu, ctx| menu.open(ctx));

            assert_eq!(
                row_titles(&menu, ctx),
                vec!["deploy app...", "verify cache"]
            );
        });
    });
}

#[test]
fn preview_switches_type_and_dismiss_restores_buffer_and_config() {
    App::test((), |mut app| async move {
        let (input, input_mode, menu) = app.update(|ctx| {
            let handles = setup_items(
                ctx,
                vec![
                    TuiHistoryItem::Prompt("older prompt".to_owned()),
                    TuiHistoryItem::Command("newest-command".to_owned()),
                ],
                AI_UNLOCKED_CONFIG,
            );
            set_text(&handles.0, "draft", ctx);
            handles.2.update(ctx, |menu, ctx| menu.open(ctx));
            assert_eq!(buffer_text(&handles.0, ctx), "newest-command");
            assert_eq!(handles.1.as_ref(ctx).input_type(), InputType::Shell);
            handles.2.update(ctx, |menu, ctx| menu.select_previous(ctx));
            assert_eq!(buffer_text(&handles.0, ctx), "older prompt");
            assert_eq!(handles.1.as_ref(ctx).input_type(), InputType::AI);
            handles
        });

        app.update(|ctx| menu.update(ctx, |menu, ctx| menu.dismiss(ctx)));
        app.read(|ctx| {
            assert_eq!(buffer_text(&input, ctx), "draft");
            assert_eq!(input_mode.as_ref(ctx).input_config(), AI_UNLOCKED_CONFIG);
        });
    });
}

#[test]
fn acceptance_retains_item_type_and_empty_acceptance_is_noop() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (_input, _mode, command_menu) = setup_items(
                ctx,
                vec![TuiHistoryItem::Command("cargo test".to_owned())],
                AI_UNLOCKED_CONFIG,
            );
            command_menu.update(ctx, |menu, ctx| menu.open(ctx));
            assert_eq!(
                command_menu.update(ctx, |menu, ctx| menu.accept_selected(ctx)),
                Some(TuiHistoryItem::Command("cargo test".to_owned()))
            );
            assert!(!command_menu.as_ref(ctx).is_open(ctx));

            let (input, _mode, empty_menu) = setup_items(ctx, vec![], AI_UNLOCKED_CONFIG);
            set_text(&input, "no match", ctx);
            empty_menu.update(ctx, |menu, ctx| menu.open(ctx));
            assert_eq!(
                empty_menu.update(ctx, |menu, ctx| menu.accept_selected(ctx)),
                None
            );
            assert!(empty_menu.as_ref(ctx).is_open(ctx));
            assert_eq!(buffer_text(&input, ctx), "no match");
        });
    });
}

#[test]
fn history_header_and_empty_status_text_are_explicit() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (input, _mode, menu) = setup_items(ctx, vec![], AI_UNLOCKED_CONFIG);
            menu.update(ctx, |menu, ctx| menu.open(ctx));
            let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            assert_eq!(snapshot.header.unwrap().title.as_deref(), Some("History"));
            assert_eq!(
                snapshot.status,
                Some(crate::inline_menu::TuiInlineMenuStatus::Empty(
                    "No history".to_owned()
                ))
            );

            menu.update(ctx, |menu, ctx| menu.dismiss(ctx));
            set_text(&input, "missing", ctx);
            menu.update(ctx, |menu, ctx| menu.open(ctx));
            assert_eq!(
                menu.as_ref(ctx).snapshot(ctx).unwrap().status,
                Some(crate::inline_menu::TuiInlineMenuStatus::Empty(
                    "No matching history".to_owned()
                ))
            );
        });
    });
}

#[test]
fn command_row_renders_green_bang_without_transcript_background() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (_input, _mode, menu) = setup_items(
                ctx,
                vec![
                    TuiHistoryItem::Command("cargo test".to_owned()),
                    TuiHistoryItem::Prompt("fix the test".to_owned()),
                ],
                AI_UNLOCKED_CONFIG,
            );
            menu.update(ctx, |menu, ctx| menu.open(ctx));
            let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            let builder = TuiUiBuilder::from_app(ctx);
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                render_inline_menu(&snapshot, &builder),
                TuiRect::new(0, 0, 50, 12),
                ctx,
            );

            assert!(frame.buffer.to_lines()[1].starts_with("! cargo test"));
            assert_eq!(
                frame.buffer[(0, 1)].fg,
                builder
                    .shell_command_accent_style()
                    .fg
                    .expect("shell command accent has a foreground")
            );
            assert_eq!(frame.buffer[(0, 1)].bg, Color::Reset);
            assert_ne!(frame.buffer[(0, 1)].bg, builder.shell_command_background());
        });
    });
}

#[test]
fn down_dismisses_empty_history_and_restores_query() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (input, _mode, menu) = setup_prompts(ctx, &[]);
            set_text(&input, "query", ctx);
            menu.update(ctx, |menu, ctx| menu.open(ctx));
            menu.update(ctx, |menu, ctx| menu.select_next(ctx));

            assert!(!menu.as_ref(ctx).is_open(ctx));
            assert_eq!(buffer_text(&input, ctx), "query");
        });
    });
}

#[test]
fn multiline_title_preserves_full_acceptance_text() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let prompt = "deploy the app\nthen verify it";
            let (input, _mode, menu) = setup_prompts(ctx, &[prompt]);
            menu.update(ctx, |menu, ctx| menu.open(ctx));

            assert_eq!(row_titles(&menu, ctx), vec!["deploy the app..."]);
            assert_eq!(buffer_text(&input, ctx), prompt);
            assert_eq!(
                menu.update(ctx, |menu, ctx| menu.accept_selected(ctx)),
                Some(TuiHistoryItem::Prompt(prompt.to_owned()))
            );
            assert_eq!(
                single_line_menu_title("deploy the app\r\nthen verify it"),
                "deploy the app..."
            );
        });
    });
}

#[test]
fn reconciled_selection_is_type_aware_then_falls_back_to_index() {
    let rows = vec![
        TuiPromptHistoryRow {
            item: TuiHistoryItem::Prompt("same".to_owned()),
        },
        TuiPromptHistoryRow {
            item: TuiHistoryItem::Command("same".to_owned()),
        },
        TuiPromptHistoryRow {
            item: TuiHistoryItem::Prompt("three".to_owned()),
        },
    ];

    assert_eq!(
        reconciled_selection_index(
            &rows,
            Some(&TuiHistoryItem::Command("same".to_owned())),
            Some(0)
        ),
        Some(1)
    );
    assert_eq!(
        reconciled_selection_index(
            &rows[..2],
            Some(&TuiHistoryItem::Prompt("gone".to_owned())),
            Some(5)
        ),
        Some(1)
    );
    assert_eq!(reconciled_selection_index(&rows, None, None), Some(2));
    assert_eq!(
        reconciled_selection_index(&[], Some(&TuiHistoryItem::Prompt("x".to_owned())), Some(0)),
        None
    );
}
