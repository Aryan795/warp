use std::collections::HashSet;

use warp_core::features::FeatureFlag;
use warpui::{AppContext, EntityId, SingletonEntity};

use super::History;
use crate::ai::blocklist::history_model::AIQueryHistory;
use crate::ai::blocklist::{BlocklistAIHistoryModel, InputConfig};
use crate::input_suggestions::HistoryInputSuggestion;
use crate::settings::AISettings;
use crate::suggestions::ignored_suggestions_model::{IgnoredSuggestionsModel, SuggestionType};
use crate::terminal::model::session::SessionId;

/// Which kind of history item an owned up-arrow entry represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpArrowHistoryEntryKind {
    /// An executed shell command.
    Command,
    /// A submitted agent prompt.
    Prompt,
}

/// An owned, frontend-agnostic entry in the combined up-arrow history.
///
/// The GUI reads borrowed [`HistoryInputSuggestion`]s directly; the headless
/// TUI consumes this owned projection instead so it never holds references
/// into the [`History`] model across frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpArrowHistoryEntry {
    /// The exact text to fill, submit, or execute.
    pub text: String,
    pub kind: UpArrowHistoryEntryKind,
}

/// Controls which item types are included in up-arrow history results.
#[derive(Copy, Clone, Debug)]
pub(crate) struct UpArrowHistoryConfig {
    pub include_commands: bool,
    pub include_prompts: bool,
}

impl UpArrowHistoryConfig {
    /// Derives the config from the current input config.
    /// When the input is locked to a specific type, only that type is included.
    /// When unlocked (auto-detection), both types are included.
    pub fn for_input_config(input_config: &InputConfig) -> Self {
        if input_config.is_locked {
            Self {
                include_commands: input_config.is_shell(),
                include_prompts: input_config.is_ai(),
            }
        } else {
            Self {
                include_commands: true,
                include_prompts: true,
            }
        }
    }
}

fn sort_and_dedupe_suggestions<'a>(
    mut suggestions: Vec<HistoryInputSuggestion<'a>>,
    session_id: Option<SessionId>,
    all_live_session_ids: &HashSet<SessionId>,
) -> Vec<HistoryInputSuggestion<'a>> {
    suggestions.sort_by(|a, b| a.cmp(b, session_id, all_live_session_ids));

    // Deduplicate commands and AI queries separately: keep the latest occurrence for each type.
    let mut seen_commands: HashSet<&str> = HashSet::new();
    let mut seen_ai_queries: HashSet<&str> = HashSet::new();
    let mut skip_indices: HashSet<usize> = HashSet::new();
    for (idx, suggestion) in suggestions.iter().enumerate().rev() {
        let text = suggestion.text();
        if suggestion.is_ai_query() {
            if seen_ai_queries.contains(text) {
                skip_indices.insert(idx);
            } else {
                seen_ai_queries.insert(text);
            }
        } else if seen_commands.contains(text) {
            skip_indices.insert(idx);
        } else {
            seen_commands.insert(text);
        }
    }

    suggestions
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| !skip_indices.contains(idx))
        .map(|(_, suggestion)| suggestion)
        .collect()
}
/// Returns de-duplicated prompt history ordered for up-arrow presentation.
///
/// Prompts from other terminal surfaces precede prompts from the requested
/// surface, and repeated text keeps its newest occurrence.
pub fn prompt_history_for_terminal_view(
    terminal_view_id: EntityId,
    app: &AppContext,
) -> Vec<AIQueryHistory> {
    prompt_history_suggestions(terminal_view_id, app)
        .into_iter()
        .filter_map(|suggestion| match suggestion {
            HistoryInputSuggestion::AIQuery { entry } => Some(entry),
            HistoryInputSuggestion::Command { .. } => None,
        })
        .collect()
}

/// Collects the ignored-filtered, non-empty prompt suggestions for a terminal
/// surface, sorted and de-duplicated for up-arrow presentation.
fn prompt_history_suggestions(
    terminal_view_id: EntityId,
    app: &AppContext,
) -> Vec<HistoryInputSuggestion<'static>> {
    let ignored_prompts = if app.has_singleton_model::<IgnoredSuggestionsModel>() {
        IgnoredSuggestionsModel::handle(app)
            .as_ref(app)
            .get_ignored_suggestions_for_type(SuggestionType::AIQuery)
    } else {
        HashSet::new()
    };
    let suggestions = BlocklistAIHistoryModel::handle(app)
        .as_ref(app)
        .all_ai_queries(Some(terminal_view_id))
        .filter(|entry| !ignored_prompts.contains(&entry.query_text))
        .filter(|entry| !entry.query_text.trim().is_empty())
        .map(|entry| HistoryInputSuggestion::AIQuery { entry })
        .collect();
    sort_and_dedupe_suggestions(suggestions, None, &HashSet::new())
}

/// Returns the owned, combined prompt + command history for a terminal
/// surface's up-arrow recall, newest entries last.
///
/// Ordering, session scoping, ignored-suggestion filtering, agent-executed
/// command filtering, and per-type de-duplication all match the GUI's
/// up-arrow history: entries from other sessions precede entries from the
/// requested session, oldest-first within each group. Whitespace-only items
/// are never returned. Commands are always included (an absent [`History`]
/// singleton or `session_id` yields none); prompts are included only when
/// `include_prompts` is set.
pub fn up_arrow_history_for_terminal_view(
    terminal_view_id: EntityId,
    session_id: Option<SessionId>,
    include_prompts: bool,
    app: &AppContext,
) -> Vec<UpArrowHistoryEntry> {
    let history_handle = app
        .has_singleton_model::<History>()
        .then(|| History::handle(app));
    let (commands, all_live_session_ids) = match &history_handle {
        Some(handle) => {
            let history = handle.as_ref(app);
            (
                history.up_arrow_command_suggestions(session_id, app),
                history.all_live_session_ids(),
            )
        }
        None => (Vec::new(), HashSet::new()),
    };

    let prompts = if include_prompts {
        prompt_history_suggestions(terminal_view_id, app)
    } else {
        Vec::new()
    };

    let suggestions = commands
        .into_iter()
        .chain(prompts)
        .filter(|suggestion| !suggestion.text().trim().is_empty())
        .collect();
    sort_and_dedupe_suggestions(suggestions, session_id, &all_live_session_ids)
        .into_iter()
        .map(|suggestion| match suggestion {
            HistoryInputSuggestion::Command { entry } => UpArrowHistoryEntry {
                text: entry.command.clone(),
                kind: UpArrowHistoryEntryKind::Command,
            },
            HistoryInputSuggestion::AIQuery { entry } => UpArrowHistoryEntry {
                text: entry.query_text,
                kind: UpArrowHistoryEntryKind::Prompt,
            },
        })
        .collect()
}

impl History {
    /// Collects the session-scoped command suggestions for up-arrow recall,
    /// applying the shared ignored-suggestion and agent-executed-command
    /// filters. Missing singletons (headless tests) fall back to the same
    /// behavior as their default states: nothing ignored, and agent-executed
    /// commands excluded per the setting's default.
    fn up_arrow_command_suggestions<'a>(
        &'a self,
        session_id: Option<SessionId>,
        app: &AppContext,
    ) -> Vec<HistoryInputSuggestion<'a>> {
        let ignored_commands = if app.has_singleton_model::<IgnoredSuggestionsModel>() {
            IgnoredSuggestionsModel::handle(app)
                .as_ref(app)
                .get_ignored_suggestions_for_type(SuggestionType::ShellCommand)
        } else {
            HashSet::new()
        };
        let include_agent_commands = app.has_singleton_model::<AISettings>()
            && *AISettings::handle(app)
                .as_ref(app)
                .include_agent_commands_in_history;

        session_id
            .and_then(|session_id| self.commands(session_id))
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| !ignored_commands.contains(&entry.command))
            .filter(|entry| include_agent_commands || !entry.is_agent_executed)
            .map(|entry| HistoryInputSuggestion::Command { entry })
            .collect()
    }

    pub(crate) fn up_arrow_suggestions_for_terminal_view<'a>(
        &'a self,
        terminal_view_id: EntityId,
        session_id: Option<SessionId>,
        config: UpArrowHistoryConfig,
        app: &'a AppContext,
    ) -> Vec<HistoryInputSuggestion<'a>> {
        let commands = if config.include_commands {
            self.up_arrow_command_suggestions(session_id, app)
        } else {
            Vec::new()
        };

        let should_include_prompts = config.include_prompts
            && FeatureFlag::AgentMode.is_enabled()
            && AISettings::handle(app).as_ref(app).is_any_ai_enabled(app);
        let all_live_session_ids = self.all_live_session_ids();
        let suggestions = if should_include_prompts {
            commands
                .into_iter()
                .chain(prompt_history_suggestions(terminal_view_id, app))
                .collect()
        } else {
            commands
        };

        sort_and_dedupe_suggestions(suggestions, session_id, &all_live_session_ids)
    }
}

#[cfg(test)]
#[path = "up_arrow_tests.rs"]
mod tests;
