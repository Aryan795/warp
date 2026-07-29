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

/// Controls which item types are included in up-arrow history results.
#[derive(Copy, Clone, Debug)]
pub struct UpArrowHistoryConfig {
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
    let sorted = sort_and_dedupe_suggestions(suggestions, None, &HashSet::new());

    sorted
        .into_iter()
        .filter_map(|suggestion| match suggestion {
            HistoryInputSuggestion::AIQuery { entry } => Some(entry),
            HistoryInputSuggestion::Command { .. } => None,
        })
        .collect()
}

/// An owned, frontend-agnostic snapshot of one up-arrow history entry.
///
/// The shared [`History::up_arrow_suggestions_for_terminal_view`] getter borrows
/// the live `HistoryEntry`/`AIQueryHistory`, which cannot cross the crate
/// boundary into the TUI. This enum is the owned projection consumed by TUI
/// `tui_export`; the GUI keeps using the borrowed `HistoryInputSuggestion`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiHistorySuggestion {
    /// An executed shell command.
    Command { command: String },
    /// A submitted agent prompt.
    Prompt { query_text: String },
}

impl TuiHistorySuggestion {
    /// The text to display for this history entry.
    pub fn text(&self) -> &str {
        match self {
            TuiHistorySuggestion::Command { command } => command.as_str(),
            TuiHistorySuggestion::Prompt { query_text } => query_text.as_str(),
        }
    }

    /// Whether this entry is an executed shell command.
    pub fn is_command(&self) -> bool {
        matches!(self, TuiHistorySuggestion::Command { .. })
    }

    /// Whether this entry is a submitted agent prompt.
    pub fn is_prompt(&self) -> bool {
        matches!(self, TuiHistorySuggestion::Prompt { .. })
    }
}

/// Returns the combined, ordered, de-duplicated up-arrow history (executed shell
/// commands and agent prompts) for a terminal view, as owned
/// [`TuiHistorySuggestion`]s for the TUI.
///
/// Ordering and de-duplication match the GUI's inline history menu: entries
/// from other sessions precede the current session's entries, oldest-first
/// within each group, with commands and prompts de-duplicated separately (the
/// newest occurrence kept). Ignored-suggestion handling, session-scoping, and
/// the agent-executed-command setting are all respected via the shared
/// [`History::up_arrow_suggestions_for_terminal_view`] getter, so the TUI and
/// GUI read identically ordered history.
pub fn history_suggestions_for_terminal_view(
    terminal_view_id: EntityId,
    session_id: Option<SessionId>,
    config: UpArrowHistoryConfig,
    app: &AppContext,
) -> Vec<TuiHistorySuggestion> {
    // The shared getter reads the `History`, `AISettings`, and
    // `IgnoredSuggestionsModel` singletons. Production always registers them;
    // lighter test fixtures (e.g. a bare input view that doesn't drive the full
    // session stack) may not, so fall back to an empty result rather than
    // panicking. Commands additionally require a bootstrapped session, which is
    // also absent in those fixtures.
    if !app.has_singleton_model::<History>()
        || !app.has_singleton_model::<AISettings>()
        || !app.has_singleton_model::<IgnoredSuggestionsModel>()
    {
        return Vec::new();
    }
    History::handle(app)
        .as_ref(app)
        .up_arrow_suggestions_for_terminal_view(terminal_view_id, session_id, config, app)
        .into_iter()
        .map(|suggestion| match suggestion {
            HistoryInputSuggestion::Command { entry } => TuiHistorySuggestion::Command {
                command: entry.command.clone(),
            },
            HistoryInputSuggestion::AIQuery { entry } => TuiHistorySuggestion::Prompt {
                query_text: entry.query_text,
            },
        })
        .collect()
}

impl History {
    pub(crate) fn up_arrow_suggestions_for_terminal_view<'a>(
        &'a self,
        terminal_view_id: EntityId,
        session_id: Option<SessionId>,
        config: UpArrowHistoryConfig,
        app: &'a AppContext,
    ) -> Vec<HistoryInputSuggestion<'a>> {
        let ignored_suggestions = IgnoredSuggestionsModel::handle(app).as_ref(app);

        let include_agent_commands = *AISettings::handle(app)
            .as_ref(app)
            .include_agent_commands_in_history;

        let commands = session_id
            .and_then(|session_id| self.commands(session_id))
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| {
                !ignored_suggestions.is_ignored(&entry.command, SuggestionType::ShellCommand)
            })
            .filter(move |entry| include_agent_commands || !entry.is_agent_executed)
            .map(|entry| HistoryInputSuggestion::Command { entry });

        let should_include_prompts = config.include_prompts
            && FeatureFlag::AgentMode.is_enabled()
            && AISettings::handle(app).as_ref(app).is_any_ai_enabled(app);
        let all_live_session_ids = self.all_live_session_ids();
        if !should_include_prompts {
            if !config.include_commands {
                return vec![];
            }
            return sort_and_dedupe_suggestions(
                commands.collect(),
                session_id,
                &all_live_session_ids,
            );
        }

        let ai_queries = prompt_history_for_terminal_view(terminal_view_id, app)
            .into_iter()
            .map(|entry| HistoryInputSuggestion::AIQuery { entry });

        let suggestions: Vec<HistoryInputSuggestion<'a>> =
            match (config.include_commands, config.include_prompts) {
                (true, true) => commands.chain(ai_queries).collect(),
                (true, false) => commands.collect(),
                (false, true) => ai_queries.collect(),
                (false, false) => vec![],
            };

        sort_and_dedupe_suggestions(suggestions, session_id, &all_live_session_ids)
    }
}

#[cfg(test)]
#[path = "up_arrow_tests.rs"]
mod tests;
