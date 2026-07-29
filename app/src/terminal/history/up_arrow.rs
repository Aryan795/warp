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
/// A single up-arrow history item projected for the headless TUI, decoupled from
/// the GUI's borrowed [`HistoryInputSuggestion`] so the TUI owns its own data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiUpArrowHistoryKind {
    /// A previously executed shell command.
    Command,
    /// A previously submitted agent prompt.
    Prompt,
}

/// An owned up-arrow history entry for the headless TUI history menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiUpArrowHistoryEntry {
    /// The command or prompt text to display, preview, and accept.
    pub text: String,
    /// Whether this entry is a shell command or an agent prompt.
    pub kind: TuiUpArrowHistoryKind,
}

/// Returns the combined command + prompt up-arrow history for a terminal view,
/// projected into owned, frontend-agnostic entries for the headless TUI.
///
/// Commands are always included; prompts are included only when `include_prompts`
/// is set (the TUI's agent input mode, versus commands-only `!` shell mode).
/// Ordering, de-duplication, ignored-suggestion, session-scope, and
/// agent-executed filtering all match the shared
/// [`History::up_arrow_suggestions_for_terminal_view`] logic. Whitespace-only or
/// empty items are dropped so they never appear as blank rows.
pub fn up_arrow_history_for_terminal_view(
    terminal_view_id: EntityId,
    session_id: Option<SessionId>,
    include_prompts: bool,
    app: &AppContext,
) -> Vec<TuiUpArrowHistoryEntry> {
    let suggestions = if app.has_singleton_model::<History>() {
        History::handle(app).as_ref(app).up_arrow_history_suggestions(
            terminal_view_id,
            session_id,
            true,
            include_prompts,
            app,
        )
    } else if include_prompts {
        // Without a command history model (e.g. isolated menu tests), fall back
        // to prompts only so the shared prompt path keeps working.
        prompt_history_for_terminal_view(terminal_view_id, app)
            .into_iter()
            .map(|entry| HistoryInputSuggestion::AIQuery { entry })
            .collect()
    } else {
        Vec::new()
    };

    suggestions
        .into_iter()
        .filter_map(|suggestion| {
            let kind = match &suggestion {
                HistoryInputSuggestion::Command { .. } => TuiUpArrowHistoryKind::Command,
                HistoryInputSuggestion::AIQuery { .. } => TuiUpArrowHistoryKind::Prompt,
            };
            let text = suggestion.text().to_owned();
            (!text.trim().is_empty()).then_some(TuiUpArrowHistoryEntry { text, kind })
        })
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

impl History {
    pub(crate) fn up_arrow_suggestions_for_terminal_view<'a>(
        &'a self,
        terminal_view_id: EntityId,
        session_id: Option<SessionId>,
        config: UpArrowHistoryConfig,
        app: &'a AppContext,
    ) -> Vec<HistoryInputSuggestion<'a>> {
        // Prompts are only surfaced in the GUI when agent mode is available and
        // AI is enabled for the account; commands are unconditional.
        let should_include_prompts = config.include_prompts
            && FeatureFlag::AgentMode.is_enabled()
            && AISettings::handle(app).as_ref(app).is_any_ai_enabled(app);
        self.up_arrow_history_suggestions(
            terminal_view_id,
            session_id,
            config.include_commands,
            should_include_prompts,
            app,
        )
    }

    /// Frontend-agnostic core shared by the GUI up-arrow menu and the headless
    /// TUI history menu.
    ///
    /// Gathers the session's executed commands and (optionally) agent prompts,
    /// applying ignored-suggestion, agent-executed, and session-scope filtering,
    /// then sorts and de-dupes them for up-arrow presentation: other-session
    /// entries precede current-session entries, oldest-first within each group,
    /// and repeated text keeps its newest occurrence (de-duplicated separately
    /// for commands and prompts).
    ///
    /// Unlike [`Self::up_arrow_suggestions_for_terminal_view`], this core performs
    /// no agent-availability gating — the caller decides whether prompts are
    /// included, so the same combining logic serves both frontends.
    fn up_arrow_history_suggestions<'a>(
        &'a self,
        terminal_view_id: EntityId,
        session_id: Option<SessionId>,
        include_commands: bool,
        include_prompts: bool,
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

        let all_live_session_ids = self.all_live_session_ids();
        if !include_prompts {
            if !include_commands {
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
            match (include_commands, include_prompts) {
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
