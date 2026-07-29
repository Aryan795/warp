//! Tests for [`TuiHistoryMenuModel`]: population/ordering/dedupe of prompts and
//! commands, default selection and initial preview, shell-mode scoping and
//! command previews, prefix filtering, buffer + input-mode snapshot/restore,
//! acceptance, and empty states.
use std::rc::Rc;

use warp::appearance::Appearance;
use warp::editor::CodeEditorModel;
use warp::settings::AISettingsChangedEvent;
use warp::tui_export::{
    ActiveSession, BlocklistAIInputModel, ConversationSelectionEvent, InputConfig, InputModePolicy,
    PolicyConfigUpdate, UpArrowHistoryEntry, UpArrowHistoryEntryKind,
    blocklist_ai_history_model_with_queries, register_tui_command_history_session,
    register_tui_input_mode_settings,
};
use warp_editor::model::CoreEditorModel;
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, EntityId, ModelHandle};

use super::{TuiHistoryMenuModel, reconciled_selection_index};
use crate::inline_menu::{TuiInlineMenuRowStyle, render_inline_menu, single_line_menu_title};
use crate::input_mode_policy::{self, AI_LOCKED_CONFIG, SHELL_LOCKED_CONFIG};
use crate::input_suggestions_mode::TuiInputSuggestionsModeModel;
use crate::test_fixtures::add_empty_active_session;
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

type MenuFixture = (
    ModelHandle<CodeEditorModel>,
    ModelHandle<BlocklistAIInputModel>,
    ModelHandle<TuiHistoryMenuModel>,
);

/// Builds a closed history menu over a fresh editor, an agent-locked input
/// mode, and a prompt-history model seeded with `prompts` (oldest-first). No
/// shell command history exists.
fn setup(ctx: &mut AppContext, prompts: &[&str]) -> MenuFixture {
    ctx.add_singleton_model(|_| Appearance::mock());
    ctx.add_singleton_model(|_| {
        blocklist_ai_history_model_with_queries(
            prompts.iter().map(|prompt| (*prompt).to_owned()).collect(),
        )
    });
    let active_session = add_empty_active_session(ctx);
    build_menu(ctx, active_session)
}

/// Like [`setup`], but with a `History` singleton seeded with
/// `history_file_commands` (other-session entries) and `session_commands`
/// (`(command, is_agent_executed)`, oldest-first) for a live active session.
fn setup_with_commands(
    app: &mut App,
    prompts: &[&str],
    history_file_commands: &[&str],
    session_commands: &[(&str, bool)],
) -> MenuFixture {
    // Previewing a command flips the shared input mode to shell and back,
    // which records usage on the `AISettings` singleton.
    register_tui_input_mode_settings(app);
    app.update(|ctx| {
        ctx.add_singleton_model(|_| Appearance::mock());
        let prompts: Vec<String> = prompts.iter().map(|prompt| (*prompt).to_owned()).collect();
        ctx.add_singleton_model(move |_| blocklist_ai_history_model_with_queries(prompts));
    });
    let active_session = register_tui_command_history_session(
        app,
        history_file_commands
            .iter()
            .map(|command| (*command).to_owned())
            .collect(),
        session_commands
            .iter()
            .map(|(command, is_agent_executed)| ((*command).to_owned(), *is_agent_executed))
            .collect(),
    );
    app.update(|ctx| build_menu(ctx, active_session))
}

fn build_menu(ctx: &mut AppContext, active_session: ModelHandle<ActiveSession>) -> MenuFixture {
    let input_model = ctx.add_model(|ctx| CodeEditorModel::new_tui(W, ctx));
    let input_mode = BlocklistAIInputModel::mock(Rc::new(TestInputModePolicy), ctx);
    let suggestions_mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
    let input_model_for_menu = input_model.clone();
    let input_mode_for_menu = input_mode.clone();
    let menu = ctx.add_model(|ctx| {
        TuiHistoryMenuModel::new(
            input_model_for_menu,
            input_mode_for_menu,
            suggestions_mode,
            active_session,
            EntityId::new(),
            ctx,
        )
    });
    (input_model, input_mode, menu)
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

fn row_titles(menu: &ModelHandle<TuiHistoryMenuModel>, ctx: &AppContext) -> Vec<String> {
    menu.as_ref(ctx)
        .snapshot(ctx)
        .map(|snapshot| snapshot.rows.iter().map(|row| row.title.clone()).collect())
        .unwrap_or_default()
}

fn row_titles_and_styles(
    menu: &ModelHandle<TuiHistoryMenuModel>,
    ctx: &AppContext,
) -> Vec<(String, TuiInlineMenuRowStyle)> {
    menu.as_ref(ctx)
        .snapshot(ctx)
        .map(|snapshot| {
            snapshot
                .rows
                .iter()
                .map(|row| (row.title.clone(), row.style))
                .collect()
        })
        .unwrap_or_default()
}

fn is_shell_mode(input_mode: &ModelHandle<BlocklistAIInputModel>, ctx: &AppContext) -> bool {
    input_mode_policy::is_shell_mode(input_mode.as_ref(ctx))
}

#[test]
fn open_populates_ordered_deduped_rows_excluding_whitespace() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            // Oldest-first. "deploy" is duplicated (newer occurrence wins) and a
            // whitespace-only prompt must be dropped.
            let (_input, _mode, menu) = setup(ctx, &["deploy", "test", "deploy", "   ", "build"]);
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
fn open_interleaves_commands_and_prompts_with_shared_ordering() {
    App::test((), |mut app| async move {
        // Other-session history (the histfile command) precedes the persisted
        // prompts, which precede this session's own commands; command rows use
        // the shell style and prompt rows the default style.
        let (_input, _mode, menu) = setup_with_commands(
            &mut app,
            &["explain this repo"],
            &["git status"],
            &[("cargo build", false)],
        );
        app.update(|ctx| {
            menu.update(ctx, |m, ctx| m.open(ctx));
            assert_eq!(
                row_titles_and_styles(&menu, ctx),
                vec![
                    ("git status".to_owned(), TuiInlineMenuRowStyle::ShellCommand),
                    (
                        "explain this repo".to_owned(),
                        TuiInlineMenuRowStyle::Default
                    ),
                    (
                        "cargo build".to_owned(),
                        TuiInlineMenuRowStyle::ShellCommand
                    ),
                ]
            );
        });
    });
}

#[test]
fn shell_mode_open_lists_commands_only() {
    App::test((), |mut app| async move {
        let (_input, input_mode, menu) = setup_with_commands(
            &mut app,
            &["explain this repo"],
            &["git status"],
            &[("cargo build", false)],
        );
        app.update(|ctx| {
            input_mode.update(ctx, |input_mode, ctx| {
                input_mode.set_input_config(SHELL_LOCKED_CONFIG, true, None, ctx);
            });
            menu.update(ctx, |m, ctx| m.open(ctx));
            assert_eq!(
                row_titles(&menu, ctx),
                vec!["git status".to_owned(), "cargo build".to_owned()]
            );
        });
    });
}

#[test]
fn command_selection_previews_in_shell_mode_and_dismiss_restores_agent_mode() {
    App::test((), |mut app| async move {
        let (input, input_mode, menu) =
            setup_with_commands(&mut app, &["explain this repo"], &["git status"], &[]);
        app.update(|ctx| {
            menu.update(ctx, |m, ctx| m.open(ctx));
            // Default selection is the newest row: the prompt, in agent mode.
            assert_eq!(buffer_text(&input, ctx), "explain this repo");
            assert!(!is_shell_mode(&input_mode, ctx));

            // Selecting the command previews it with the `!` shell affordance.
            menu.update(ctx, |m, ctx| m.select_previous(ctx));
            assert_eq!(buffer_text(&input, ctx), "git status");
            assert!(is_shell_mode(&input_mode, ctx));

            // Moving back to the prompt restores the opening (agent) mode.
            menu.update(ctx, |m, ctx| m.select_next(ctx));
            assert_eq!(buffer_text(&input, ctx), "explain this repo");
            assert!(!is_shell_mode(&input_mode, ctx));

            // Dismissing from a command preview restores buffer *and* mode.
            menu.update(ctx, |m, ctx| m.select_previous(ctx));
            assert!(is_shell_mode(&input_mode, ctx));
            menu.update(ctx, |m, ctx| m.dismiss(ctx));
            assert!(!menu.as_ref(ctx).is_open(ctx));
            assert_eq!(buffer_text(&input, ctx), "");
            assert!(!is_shell_mode(&input_mode, ctx));
        });
    });
}

#[test]
fn accept_selected_returns_highlighted_command_and_closes() {
    App::test((), |mut app| async move {
        let (input, input_mode, menu) =
            setup_with_commands(&mut app, &["explain this repo"], &["git status"], &[]);
        app.update(|ctx| {
            menu.update(ctx, |m, ctx| m.open(ctx));
            menu.update(ctx, |m, ctx| m.select_previous(ctx));
            let accepted = menu.update(ctx, |m, ctx| m.accept_selected(ctx));
            assert_eq!(
                accepted,
                Some(UpArrowHistoryEntry {
                    text: "git status".to_owned(),
                    kind: UpArrowHistoryEntryKind::Command,
                })
            );
            assert!(!menu.as_ref(ctx).is_open(ctx));
            // The accepted command stays previewed in shell mode; the session
            // view executes it and clears the input.
            assert_eq!(buffer_text(&input, ctx), "git status");
            assert!(is_shell_mode(&input_mode, ctx));
        });
    });
}

#[test]
fn open_selects_and_previews_last_row() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (input, _mode, menu) = setup(ctx, &["first", "second", "third"]);
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
            let (input, _mode, menu) = setup(ctx, &["deploy the app", "delete cache", "build"]);
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
            let (input, _mode, menu) = setup(ctx, &[prompt, "unrelated prompt"]);
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
            let (input, _mode, menu) = setup(ctx, &["deploy the app"]);
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
            let (_input, _mode, menu) = setup(ctx, &["older prompt", "newest prompt"]);
            menu.update(ctx, |m, ctx| m.open(ctx));
            // Default selection is the newest (last) row.
            let accepted = menu.update(ctx, |m, ctx| m.accept_selected(ctx));
            assert_eq!(
                accepted,
                Some(UpArrowHistoryEntry {
                    text: "newest prompt".to_owned(),
                    kind: UpArrowHistoryEntryKind::Prompt,
                })
            );
            assert!(!menu.as_ref(ctx).is_open(ctx));
        });
    });
}

#[test]
fn accept_with_nothing_selected_is_a_no_op() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (input, _mode, menu) = setup(ctx, &["deploy the app"]);
            set_text(&input, "no match", ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));
            assert!(row_titles(&menu, ctx).is_empty());

            let accepted = menu.update(ctx, |m, ctx| m.accept_selected(ctx));
            assert_eq!(accepted, None);
            assert!(
                menu.as_ref(ctx).is_open(ctx),
                "Enter on a filtered-to-nothing list must not close the menu"
            );
            assert_eq!(buffer_text(&input, ctx), "no match");
        });
    });
}

#[test]
fn multiline_prompt_uses_single_line_title_without_changing_prompt_text() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let prompt = "deploy the app\nthen verify it";
            let (input, _mode, menu) = setup(ctx, &[prompt]);
            menu.update(ctx, |m, ctx| m.open(ctx));

            let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            assert_eq!(snapshot.rows[0].title, "deploy the app...");
            assert_eq!(buffer_text(&input, ctx), prompt);
            assert_eq!(
                menu.update(ctx, |m, ctx| m.accept_selected(ctx))
                    .map(|entry| entry.text),
                Some(prompt.to_owned())
            );
        });
    });
}

#[test]
fn history_title_handles_windows_line_endings() {
    assert_eq!(
        single_line_menu_title("deploy the app\r\nthen verify it"),
        "deploy the app..."
    );
}

#[test]
fn empty_history_shows_explicit_empty_state() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (_input, _mode, menu) = setup(ctx, &[]);
            menu.update(ctx, |m, ctx| m.open(ctx));
            let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            assert!(snapshot.rows.is_empty());
            assert_eq!(
                snapshot.status,
                Some(crate::inline_menu::TuiInlineMenuStatus::Empty(
                    "No history".to_owned()
                ))
            );
        });
    });
}

#[test]
fn filtered_to_nothing_shows_no_matching_state() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (input, _mode, menu) = setup(ctx, &["deploy the app"]);
            set_text(&input, "no match", ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));
            let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            assert!(snapshot.rows.is_empty());
            assert_eq!(
                snapshot.status,
                Some(crate::inline_menu::TuiInlineMenuStatus::Empty(
                    "No matching history".to_owned()
                ))
            );
        });
    });
}

#[test]
fn down_dismisses_empty_history() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let (input, _mode, menu) = setup(ctx, &[]);
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
            let (input, _mode, menu) = setup(ctx, &["deploy the app"]);
            set_text(&input, "no match", ctx);
            menu.update(ctx, |m, ctx| m.open(ctx));
            menu.update(ctx, |m, ctx| m.select_next(ctx));

            assert!(!menu.as_ref(ctx).is_open(ctx));
            assert_eq!(buffer_text(&input, ctx), "no match");
        });
    });
}

#[test]
fn open_menu_renders_history_surface_to_lines() {
    App::test((), |mut app| async move {
        let (_input, _mode, menu) =
            setup_with_commands(&mut app, &["run the tests"], &["git status"], &[]);
        app.update(|ctx| {
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
            assert!(
                rendered.contains("! git status"),
                "command rows should carry the `!` affordance:\n{rendered}"
            );
            assert!(rendered.contains("run the tests"));
            assert!(
                !rendered.contains("! run the tests"),
                "prompt rows must not carry the `!` affordance:\n{rendered}"
            );
        });
    });
}

#[test]
fn reconciled_selection_prefers_row_then_index_then_last_row() {
    let entry = |text: &str| UpArrowHistoryEntry {
        text: text.to_owned(),
        kind: UpArrowHistoryEntryKind::Prompt,
    };
    let rows = vec![entry("one"), entry("two"), entry("three")];

    // Stable selection by row wins over the previous index.
    assert_eq!(
        reconciled_selection_index(&rows, Some(&entry("two")), Some(0)),
        Some(1)
    );
    // No row match falls back to the (clamped) previous index.
    assert_eq!(
        reconciled_selection_index(&rows[..2], Some(&entry("gone")), Some(5)),
        Some(1)
    );
    // No prior selection defaults to the last (most-recent) row.
    assert_eq!(reconciled_selection_index(&rows, None, None), Some(2));
    // An empty list has nothing to select.
    assert_eq!(
        reconciled_selection_index(&[], Some(&entry("x")), Some(0)),
        None
    );
}
