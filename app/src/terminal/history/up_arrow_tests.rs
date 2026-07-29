//! Tests for the shared up-arrow history getters used by the GUI and TUI
//! menus: [`prompt_history_for_terminal_view`] and the owned combined
//! [`up_arrow_history_for_terminal_view`] projection.
use std::sync::Arc;

use chrono::{Duration, Local};
use warpui::{App, EntityId, SingletonEntity as _};

use super::{
    UpArrowHistoryEntry, UpArrowHistoryEntryKind, prompt_history_for_terminal_view,
    up_arrow_history_for_terminal_view,
};
use crate::ai::agent::AIAgentExchangeId;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::history_model::AIQueryHistoryOutputStatus;
use crate::ai::blocklist::{BlocklistAIHistoryModel, PersistedAIInput, PersistedAIInputType};
use crate::ai::llms::LLMId;
use crate::suggestions::ignored_suggestions_model::{IgnoredSuggestionsModel, SuggestionType};
use crate::terminal::History;
use crate::terminal::history::HistoryEntry;
use crate::terminal::model::session::command_executor::testing::TestCommandExecutor;
use crate::terminal::model::session::{Session, SessionId, SessionInfo};

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

/// Registers a `History` singleton seeded with `history_file_commands`
/// (other-session entries) and `session_commands` for one test session, and
/// returns that session's ID.
fn seed_command_history(
    app: &mut App,
    history_file_commands: Vec<&str>,
    session_commands: Vec<HistoryEntry>,
) -> SessionId {
    let session = Arc::new(Session::new(
        SessionInfo::new_for_test(),
        Arc::new(TestCommandExecutor::default()),
    ));
    let session_id = session.id();
    let history_file_commands: Vec<String> = history_file_commands
        .into_iter()
        .map(str::to_owned)
        .collect();
    app.add_singleton_model(|_| History::default());
    app.update(|ctx| {
        History::handle(ctx).update(ctx, |history, ctx| {
            history.init_session_with_history_file_commands_for_test(
                &session,
                history_file_commands,
                ctx,
            );
            history.append_commands(session_id, session_commands);
        });
    });
    session_id
}

/// Builds a current-session command entry executed at `start_ts`.
fn session_command(
    session_id: SessionId,
    command: &str,
    start_ts: chrono::DateTime<Local>,
) -> HistoryEntry {
    HistoryEntry {
        session_id: Some(session_id),
        start_ts: Some(start_ts),
        ..HistoryEntry::command_only(command)
    }
}

fn entry_texts_and_kinds(entries: &[UpArrowHistoryEntry]) -> Vec<(&str, UpArrowHistoryEntryKind)> {
    entries
        .iter()
        .map(|entry| (entry.text.as_str(), entry.kind))
        .collect()
}

#[test]
fn combined_history_orders_other_session_items_before_current_session_commands() {
    App::test((), |mut app| async move {
        let terminal_surface_id = EntityId::new();
        // Persisted prompts are other-surface history; histfile commands are
        // other-session history. Both precede this session's commands, and
        // current-session commands order by execution time, not insertion.
        app.add_singleton_model(move |_| {
            build_history_model(vec!["prompt one".to_owned(), "prompt two".to_owned()])
        });
        let now = Local::now();
        let session_id =
            seed_command_history(&mut app, vec!["histfile one", "histfile two"], Vec::new());
        app.update(|ctx| {
            History::handle(ctx).update(ctx, |history, _| {
                history.append_commands(
                    session_id,
                    vec![
                        session_command(session_id, "session late", now + Duration::hours(2)),
                        session_command(session_id, "session early", now + Duration::hours(1)),
                    ],
                );
            });
        });
        app.read(|ctx| {
            let entries = up_arrow_history_for_terminal_view(
                terminal_surface_id,
                Some(session_id),
                true,
                ctx,
            );
            assert_eq!(
                entry_texts_and_kinds(&entries),
                vec![
                    ("histfile one", UpArrowHistoryEntryKind::Command),
                    ("histfile two", UpArrowHistoryEntryKind::Command),
                    ("prompt one", UpArrowHistoryEntryKind::Prompt),
                    ("prompt two", UpArrowHistoryEntryKind::Prompt),
                    ("session early", UpArrowHistoryEntryKind::Command),
                    ("session late", UpArrowHistoryEntryKind::Command),
                ]
            );
        });
    });
}

#[test]
fn combined_history_dedupes_commands_and_prompts_separately() {
    App::test((), |mut app| async move {
        let terminal_surface_id = EntityId::new();
        // "deploy" exists as both a command and a prompt: both survive, but
        // repeated text within each type keeps a single occurrence.
        app.add_singleton_model(move |_| {
            build_history_model(vec!["deploy".to_owned(), "deploy".to_owned()])
        });
        let session_id =
            seed_command_history(&mut app, vec!["deploy", "build", "deploy"], Vec::new());
        app.read(|ctx| {
            let entries = up_arrow_history_for_terminal_view(
                terminal_surface_id,
                Some(session_id),
                true,
                ctx,
            );
            assert_eq!(
                entry_texts_and_kinds(&entries),
                vec![
                    ("build", UpArrowHistoryEntryKind::Command),
                    ("deploy", UpArrowHistoryEntryKind::Command),
                    ("deploy", UpArrowHistoryEntryKind::Prompt),
                ]
            );
        });
    });
}

#[test]
fn combined_history_excludes_whitespace_and_can_exclude_prompts() {
    App::test((), |mut app| async move {
        let terminal_surface_id = EntityId::new();
        app.add_singleton_model(move |_| build_history_model(vec!["a prompt".to_owned()]));
        let session_id = seed_command_history(&mut app, vec!["   ", "ls"], Vec::new());
        app.read(|ctx| {
            // Shell mode: commands only, whitespace-only commands dropped.
            let entries = up_arrow_history_for_terminal_view(
                terminal_surface_id,
                Some(session_id),
                false,
                ctx,
            );
            assert_eq!(
                entry_texts_and_kinds(&entries),
                vec![("ls", UpArrowHistoryEntryKind::Command)]
            );
        });
    });
}

#[test]
fn combined_history_filters_ignored_and_agent_executed_commands() {
    App::test((), |mut app| async move {
        let terminal_surface_id = EntityId::new();
        app.add_singleton_model(move |_| build_history_model(Vec::new()));
        app.add_singleton_model(|_| {
            IgnoredSuggestionsModel::new(vec![("ignored".to_owned(), SuggestionType::ShellCommand)])
        });
        let now = Local::now();
        let session_id = seed_command_history(&mut app, vec!["ignored"], Vec::new());
        app.update(|ctx| {
            History::handle(ctx).update(ctx, |history, _| {
                let mut agent_entry = session_command(session_id, "agent run", now);
                agent_entry.is_agent_executed = true;
                history.append_commands(
                    session_id,
                    vec![
                        agent_entry,
                        session_command(session_id, "kept", now + Duration::seconds(1)),
                    ],
                );
            });
        });
        app.read(|ctx| {
            // The ignored command and the agent-executed command (excluded by
            // the setting's default) are both dropped.
            let entries = up_arrow_history_for_terminal_view(
                terminal_surface_id,
                Some(session_id),
                true,
                ctx,
            );
            assert_eq!(
                entry_texts_and_kinds(&entries),
                vec![("kept", UpArrowHistoryEntryKind::Command)]
            );
        });
    });
}

#[test]
fn combined_history_without_command_history_returns_prompts_only() {
    App::test((), |app| async move {
        let terminal_surface_id = EntityId::new();
        // No `History` singleton and no session: prompts still surface.
        app.add_singleton_model(move |_| build_history_model(vec!["a prompt".to_owned()]));
        app.read(|ctx| {
            let entries = up_arrow_history_for_terminal_view(terminal_surface_id, None, true, ctx);
            assert_eq!(
                entry_texts_and_kinds(&entries),
                vec![("a prompt", UpArrowHistoryEntryKind::Prompt)]
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
