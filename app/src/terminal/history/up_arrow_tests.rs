//! Tests for the shared [`prompt_history_for_terminal_view`] getter used by the
//! GUI and TUI up-arrow prompt-history menus.
use std::sync::Arc;

use chrono::Local;
use warpui::{App, EntityId, ModelHandle};

use super::{
    TuiUpArrowHistoryEntry, TuiUpArrowHistoryKind, prompt_history_for_terminal_view,
    up_arrow_history_for_terminal_view,
};
use crate::ai::agent::AIAgentExchangeId;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::history_model::AIQueryHistoryOutputStatus;
use crate::ai::blocklist::{BlocklistAIHistoryModel, PersistedAIInput, PersistedAIInputType};
use crate::ai::llms::LLMId;
use crate::settings::AISettings;
use crate::suggestions::ignored_suggestions_model::{IgnoredSuggestionsModel, SuggestionType};
use crate::terminal::History;
use crate::terminal::model::session::command_executor::testing::TestCommandExecutor;
use crate::terminal::model::session::{Session, SessionId, SessionInfo};
use crate::terminal::shell::ShellType;

/// Builds a [`BlocklistAIHistoryModel`] seeded with `prompts` (oldest-first)
/// as persisted queries, matching how `ai_queries` rows are restored at startup.
fn build_history_model(prompts: Vec<String>) -> BlocklistAIHistoryModel {
    let base = Local::now();
    let persisted_queries = prompts
        .into_iter()
        .enumerate()
        .map(|(index, text)| PersistedAIInput {
            exchange_id: AIAgentExchangeId::new(),
            conversation_id: AIConversationId::new(),
            start_ts: base + chrono::Duration::milliseconds(index as i64),
            inputs: vec![PersistedAIInputType::Query {
                text,
                context: Default::default(),
                referenced_attachments: Default::default(),
            }],
            output_status: AIQueryHistoryOutputStatus::Completed,
            working_directory: None,
            model_id: LLMId::from("test-model"),
            coding_model_id: LLMId::from("test-model"),
        })
        .collect();
    BlocklistAIHistoryModel::new(persisted_queries, vec![], &[])
}

/// Asserts that querying a history seeded with `prompts` (oldest-first) yields
/// exactly `expected`.
fn assert_prompt_history(prompts: &[&str], expected: &[&str]) {
    let prompts: Vec<String> = prompts.iter().map(|prompt| (*prompt).to_owned()).collect();
    let expected: Vec<String> = expected.iter().map(|entry| (*entry).to_owned()).collect();
    App::test((), |app| async move {
        let terminal_surface_id = EntityId::new();
        app.add_singleton_model(move |_| build_history_model(prompts));
        app.read(|ctx| {
            let texts: Vec<String> = prompt_history_for_terminal_view(terminal_surface_id, ctx)
                .into_iter()
                .map(|entry| entry.query_text)
                .collect();
            assert_eq!(texts, expected);
        });
    });
}

#[test]
fn prompt_history_dedupes_orders_and_excludes_whitespace() {
    // Oldest-first submission order. "deploy the app" appears twice; the newer
    // occurrence wins and the older is dropped. The whitespace-only prompt must
    // never appear.
    assert_prompt_history(
        &[
            "deploy the app",
            "delete the cache",
            "deploy the app",
            "   ",
            "build the project",
        ],
        &["delete the cache", "deploy the app", "build the project"],
    );
}

/// Asserts that the TUI projection over a history seeded with `prompts`
/// (oldest-first) yields exactly `expected` prompt entries. With no command
/// history model registered and no session, the projection falls back to prompts
/// only, so this exercises the prompt path, ordering, and kind tagging.
fn assert_tui_prompt_projection(prompts: &[&str], expected: &[&str]) {
    let prompts: Vec<String> = prompts.iter().map(|prompt| (*prompt).to_owned()).collect();
    let expected: Vec<String> = expected.iter().map(|entry| (*entry).to_owned()).collect();
    App::test((), |app| async move {
        let terminal_surface_id = EntityId::new();
        app.add_singleton_model(move |_| build_history_model(prompts));
        app.read(|ctx| {
            let entries =
                up_arrow_history_for_terminal_view(terminal_surface_id, None, true, ctx);
            assert!(
                entries
                    .iter()
                    .all(|entry| entry.kind == TuiUpArrowHistoryKind::Prompt),
                "every projected entry should be a prompt without a command history"
            );
            let texts: Vec<String> = entries.into_iter().map(|entry| entry.text).collect();
            assert_eq!(texts, expected);
        });
    });
}

#[test]
fn tui_projection_orders_dedupes_and_tags_prompts() {
    assert_tui_prompt_projection(
        &[
            "deploy the app",
            "delete the cache",
            "deploy the app",
            "   ",
            "build the project",
        ],
        &["delete the cache", "deploy the app", "build the project"],
    );
}

#[test]
fn tui_projection_in_shell_mode_omits_prompts() {
    let prompts: Vec<String> = ["deploy the app", "build the project"]
        .iter()
        .map(|prompt| (*prompt).to_owned())
        .collect();
    App::test((), |app| async move {
        let terminal_surface_id = EntityId::new();
        app.add_singleton_model(move |_| build_history_model(prompts));
        app.read(|ctx| {
            // Shell mode (`include_prompts = false`) with no session commands
            // yields nothing — prompts are never surfaced there.
            let entries: Vec<TuiUpArrowHistoryEntry> =
                up_arrow_history_for_terminal_view(terminal_surface_id, None, false, ctx);
            assert!(entries.is_empty());
        });
    });
}

#[test]
fn tui_projection_excludes_ignored_prompts() {
    let prompts: Vec<String> = ["deploy the app", "delete the cache", "build the project"]
        .iter()
        .map(|prompt| (*prompt).to_owned())
        .collect();
    App::test((), |app| async move {
        let terminal_surface_id = EntityId::new();
        app.add_singleton_model(move |_| build_history_model(prompts));
        app.add_singleton_model(|_| {
            IgnoredSuggestionsModel::new(vec![(
                "delete the cache".to_owned(),
                SuggestionType::AIQuery,
            )])
        });
        app.read(|ctx| {
            let texts: Vec<String> =
                up_arrow_history_for_terminal_view(terminal_surface_id, None, true, ctx)
                    .into_iter()
                    .map(|entry| entry.text)
                    .collect();
            assert_eq!(
                texts,
                vec!["deploy the app".to_owned(), "build the project".to_owned()]
            );
        });
    });
}

#[test]
fn prompt_history_excludes_ignored_prompts() {
    let prompts: Vec<String> = ["deploy the app", "delete the cache", "build the project"]
        .iter()
        .map(|prompt| (*prompt).to_owned())
        .collect();
    App::test((), |app| async move {
        let terminal_surface_id = EntityId::new();
        app.add_singleton_model(move |_| build_history_model(prompts));
        app.add_singleton_model(|_| {
            IgnoredSuggestionsModel::new(vec![(
                "delete the cache".to_owned(),
                SuggestionType::AIQuery,
            )])
        });
        app.read(|ctx| {
            let texts: Vec<String> = prompt_history_for_terminal_view(terminal_surface_id, ctx)
                .into_iter()
                .map(|entry| entry.query_text)
                .collect();
            // The ignored prompt is excluded; the rest remain in order.
            assert_eq!(
                texts,
                vec!["deploy the app".to_owned(), "build the project".to_owned()]
            );
        });
    });
}

/// Registers a `History` singleton with a bootstrapped Bash session whose shell
/// history file contains `commands`, returning the session id so command-scoped
/// projection queries can be exercised.
async fn register_history_with_commands(app: &mut App, commands: Vec<String>) -> SessionId {
    let session = Arc::new(Session::new(
        SessionInfo::new_for_test().with_shell_type(ShellType::Bash),
        Arc::new(TestCommandExecutor::default()),
    ));
    let session_id = session.id();
    let mut history_handle: ModelHandle<History> = app.add_singleton_model(|_| History::default());
    let session_for_init = session.clone();
    history_handle.update(app, move |history, ctx| {
        history.init_session_with(session_for_init, async move { commands }, ctx);
    });
    History::initialized_sessions(&mut history_handle, app, vec![session_id]).await;
    session_id
}

#[test]
fn tui_projection_interleaves_commands_and_prompts_in_agent_mode() {
    App::test((), |mut app| async move {
        let terminal_surface_id = EntityId::new();
        let session_id =
            register_history_with_commands(&mut app, vec!["ls".to_owned(), "git status".to_owned()])
                .await;
        app.add_singleton_model(|_| build_history_model(vec!["deploy the app".to_owned()]));
        app.add_singleton_model(AISettings::new_with_defaults);
        app.add_singleton_model(|_| IgnoredSuggestionsModel::new(vec![]));
        app.read(|ctx| {
            let entries =
                up_arrow_history_for_terminal_view(terminal_surface_id, Some(session_id), true, ctx);
            let commands: Vec<&str> = entries
                .iter()
                .filter(|entry| entry.kind == TuiUpArrowHistoryKind::Command)
                .map(|entry| entry.text.as_str())
                .collect();
            let prompts: Vec<&str> = entries
                .iter()
                .filter(|entry| entry.kind == TuiUpArrowHistoryKind::Prompt)
                .map(|entry| entry.text.as_str())
                .collect();
            // Agent mode interleaves both executed commands and submitted prompts.
            assert_eq!(commands, vec!["ls", "git status"]);
            assert_eq!(prompts, vec!["deploy the app"]);
        });
    });
}

#[test]
fn tui_projection_shell_mode_shows_commands_only() {
    App::test((), |mut app| async move {
        let terminal_surface_id = EntityId::new();
        let session_id =
            register_history_with_commands(&mut app, vec!["ls".to_owned(), "git status".to_owned()])
                .await;
        app.add_singleton_model(|_| build_history_model(vec!["deploy the app".to_owned()]));
        app.add_singleton_model(AISettings::new_with_defaults);
        app.add_singleton_model(|_| IgnoredSuggestionsModel::new(vec![]));
        app.read(|ctx| {
            // `include_prompts = false` (`!` shell mode) surfaces commands only.
            let entries = up_arrow_history_for_terminal_view(
                terminal_surface_id,
                Some(session_id),
                false,
                ctx,
            );
            assert!(
                entries
                    .iter()
                    .all(|entry| entry.kind == TuiUpArrowHistoryKind::Command),
                "shell mode must not surface prompts"
            );
            let texts: Vec<&str> = entries.iter().map(|entry| entry.text.as_str()).collect();
            assert_eq!(texts, vec!["ls", "git status"]);
        });
    });
}
