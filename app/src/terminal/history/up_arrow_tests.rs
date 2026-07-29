//! Tests for the shared [`prompt_history_for_terminal_view`] getter used by the
//! GUI and TUI up-arrow prompt-history menus.
use std::collections::HashSet;

use chrono::Local;
use warpui::{App, EntityId};

use super::{
    prompt_history_for_terminal_view, should_include_command, sort_and_dedupe_suggestions,
};
use crate::ai::agent::AIAgentExchangeId;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::history_model::AIQueryHistoryOutputStatus;
use crate::ai::blocklist::{BlocklistAIHistoryModel, PersistedAIInput, PersistedAIInputType};
use crate::ai::llms::LLMId;
use crate::input_suggestions::{HistoryInputSuggestion, HistoryOrder};
use crate::suggestions::ignored_suggestions_model::{IgnoredSuggestionsModel, SuggestionType};
use crate::terminal::HistoryEntry;

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
fn combined_history_groups_other_sessions_first_and_orders_each_group_oldest_first() {
    let now = Local::now();
    let current_session_id = crate::terminal::model::session::SessionId::from(42);
    let other_command = HistoryEntry::command_at_time("other command".to_owned(), now, None, false);
    let current_command = HistoryEntry::command_at_time(
        "current command".to_owned(),
        now + chrono::Duration::seconds(2),
        Some(current_session_id),
        false,
    );
    let other_prompt = crate::ai::blocklist::history_model::AIQueryHistory::new_for_test(
        "other prompt",
        now + chrono::Duration::seconds(1),
        HistoryOrder::DifferentSession,
    );
    let current_prompt = crate::ai::blocklist::history_model::AIQueryHistory::new_for_test(
        "current prompt",
        now + chrono::Duration::seconds(3),
        HistoryOrder::CurrentSession,
    );

    let suggestions = sort_and_dedupe_suggestions(
        vec![
            HistoryInputSuggestion::AIQuery {
                entry: current_prompt,
            },
            HistoryInputSuggestion::Command {
                entry: &current_command,
            },
            HistoryInputSuggestion::AIQuery {
                entry: other_prompt,
            },
            HistoryInputSuggestion::Command {
                entry: &other_command,
            },
        ],
        Some(current_session_id),
        &HashSet::new(),
    );

    assert_eq!(
        suggestions
            .iter()
            .map(HistoryInputSuggestion::text)
            .collect::<Vec<_>>(),
        vec![
            "other command",
            "other prompt",
            "current command",
            "current prompt"
        ]
    );
}

#[test]
fn combined_history_dedupes_commands_and_prompts_separately() {
    let now = Local::now();
    let old_command = HistoryEntry::command_at_time("same".to_owned(), now, None, false);
    let new_command = HistoryEntry::command_at_time(
        "same".to_owned(),
        now + chrono::Duration::seconds(2),
        None,
        false,
    );
    let old_prompt = crate::ai::blocklist::history_model::AIQueryHistory::new_for_test(
        "same",
        now + chrono::Duration::seconds(1),
        HistoryOrder::DifferentSession,
    );
    let new_prompt = crate::ai::blocklist::history_model::AIQueryHistory::new_for_test(
        "same",
        now + chrono::Duration::seconds(3),
        HistoryOrder::DifferentSession,
    );

    let suggestions = sort_and_dedupe_suggestions(
        vec![
            HistoryInputSuggestion::Command {
                entry: &old_command,
            },
            HistoryInputSuggestion::AIQuery { entry: old_prompt },
            HistoryInputSuggestion::Command {
                entry: &new_command,
            },
            HistoryInputSuggestion::AIQuery { entry: new_prompt },
        ],
        None,
        &HashSet::new(),
    );

    assert_eq!(suggestions.len(), 2);
    assert!(matches!(
        suggestions[0],
        HistoryInputSuggestion::Command { .. }
    ));
    assert!(matches!(
        suggestions[1],
        HistoryInputSuggestion::AIQuery { .. }
    ));
}

#[test]
fn command_visibility_filters_whitespace_ignored_and_agent_executed_items() {
    let visible = HistoryEntry::command_only("echo visible");
    let whitespace = HistoryEntry::command_only("   ");
    let ignored = HistoryEntry::command_only("echo ignored");
    let mut agent = HistoryEntry::command_only("echo agent");
    agent.is_agent_executed = true;
    let ignored_commands = HashSet::from(["echo ignored".to_owned()]);

    assert!(should_include_command(&visible, &ignored_commands, false));
    assert!(!should_include_command(
        &whitespace,
        &ignored_commands,
        false
    ));
    assert!(!should_include_command(&ignored, &ignored_commands, false));
    assert!(!should_include_command(&agent, &ignored_commands, false));
    assert!(should_include_command(&agent, &ignored_commands, true));
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
