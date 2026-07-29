//! Up-arrow history inline menu state for the TUI.
//!
//! Extends the GUI's up-arrow prompt-history recall (see `CODE-1871`) into a
//! combined prompt + command history (see `CODE-1906`). Pressing Up with the
//! caret on the first visual row opens this menu; in agent mode it interleaves
//! agent prompts and executed shell commands, and in `!` shell mode it shows
//! commands only. Ordering matches the GUI: other-session entries precede
//! current-session entries, oldest-first within each group. Selecting a command
//! previews it in shell mode and executes it on acceptance; selecting a prompt
//! previews and submits it as an agent prompt. Enter on an empty or
//! filtered-to-nothing list is a no-op.
//!
//! The list comes from the shared [`history_suggestions_for_terminal_view`]
//! getter so the TUI and GUI read identically ordered and de-duplicated history.
//! The model owns filtering, menu lifecycle, selection, preview, input-mode
//! switching, and buffer snapshot/restore; the terminal session view executes
//! accepted commands and submits accepted prompts.
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{
    ActiveSession, BlocklistAIHistoryEvent, BlocklistAIHistoryModel, BlocklistAIInputModel,
    InputConfig, InputTypeAutoDetectionSource, TuiHistorySuggestion, UpArrowHistoryConfig,
    history_suggestions_for_terminal_view,
};
use warp_editor::model::CoreEditorModel;
use warpui_core::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use crate::inline_menu::{
    MAX_INLINE_MENU_ROWS, TuiInlineMenuHeader, TuiInlineMenuListState, TuiInlineMenuRow,
    TuiInlineMenuRowStyle, TuiInlineMenuSnapshot, TuiInlineMenuStatus, result_row_capacity,
    single_line_menu_title,
};
use crate::input_mode_policy::{AI_LOCKED_CONFIG, SHELL_LOCKED_CONFIG, is_shell_mode};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);

/// Whether a history row is an executed shell command or a submitted agent
/// prompt. Drives the row's affordance (`!` prefix for commands) and how
/// acceptance is routed (shell execution vs. agent submission).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TuiHistoryKind {
    Command,
    Prompt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TuiPromptHistoryRow {
    text: String,
    kind: TuiHistoryKind,
}

/// An accepted up-arrow history entry: its text and kind. The session view
/// executes commands and submits prompts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiHistoryAcceptedItem {
    pub(crate) text: String,
    pub(crate) kind: TuiHistoryKind,
}

#[derive(Debug, Clone, Default)]
enum TuiPromptHistoryMenuState {
    #[default]
    Closed,
    Open {
        list: TuiInlineMenuListState<TuiPromptHistoryRow>,
        /// The input buffer captured when the menu opened, restored on dismiss.
        original_buffer: String,
        /// The user's typed search query. Held separately from the input buffer
        /// so selection previews (which overwrite the buffer) do not change what
        /// the list filters against.
        query: String,
        /// Whether prompts were eligible when the menu opened (agent mode).
        /// Captured at open so preview-driven input-mode switches (commands
        /// switch to shell mode for the `!` gutter) don't re-gate the list down
        /// to commands-only while the user cycles entries or types.
        include_prompts: bool,
    },
}

/// Events emitted by the TUI prompt-history menu.
#[derive(Debug, Clone, Copy)]
pub(crate) enum TuiPromptHistoryMenuEvent {
    Updated,
}

/// Query, selection, preview, and model-subscription state for the up-arrow
/// prompt-history menu.
pub(crate) struct TuiPromptHistoryMenuModel {
    input_editor: ModelHandle<CodeEditorModel>,
    input_mode: ModelHandle<BlocklistAIInputModel>,
    active_session: ModelHandle<ActiveSession>,
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    terminal_surface_id: EntityId,
    state: TuiPromptHistoryMenuState,
    /// The input config captured when the menu opened, restored on dismiss so
    /// previewing commands (which switches to shell mode for the `!` gutter)
    /// never leaves the input stranded in shell mode after the menu closes.
    original_input_config: Option<InputConfig>,
    /// The text most recently written into the input as a preview. Content
    /// changes matching it are the editor echoing our own preview write and are
    /// ignored so they don't clobber the typed query. Model events are delivered
    /// after the current update flushes, so a transient set/reset flag around the
    /// write would not survive to the deferred handler — hence a content compare.
    preview_text: Option<String>,
}

impl TuiPromptHistoryMenuModel {
    /// Creates a closed prompt-history menu and subscribes it to input/history
    /// changes.
    pub(crate) fn new(
        input_editor: ModelHandle<CodeEditorModel>,
        input_mode: ModelHandle<BlocklistAIInputModel>,
        active_session: ModelHandle<ActiveSession>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&input_editor, |model, _, event, ctx| {
            if model.is_open(ctx) && matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                model.on_content_changed(ctx);
            }
        });
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |model, _, _: &BlocklistAIHistoryEvent, ctx| {
                if model.is_open(ctx) {
                    model.refresh_rows(ctx);
                }
            },
        );
        Self {
            input_editor,
            input_mode,
            active_session,
            suggestions_mode,
            terminal_surface_id,
            state: TuiPromptHistoryMenuState::Closed,
            original_input_config: None,
            preview_text: None,
        }
    }

    fn has_open_state(&self) -> bool {
        matches!(self.state, TuiPromptHistoryMenuState::Open { .. })
    }

    pub(crate) fn is_open(&self, ctx: &AppContext) -> bool {
        self.has_open_state()
            && self.suggestions_mode.as_ref(ctx).mode() == TuiInputSuggestionsMode::PromptHistory
    }

    /// Opens the menu, snapshotting the current input as both the restorable
    /// original buffer and the initial search query, then previews the default
    /// selection.
    pub(crate) fn open(&mut self, ctx: &mut ModelContext<Self>) {
        if self.has_open_state() {
            return;
        }
        let did_open = self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.try_open(TuiInputSuggestionsMode::PromptHistory, ctx)
        });
        if !did_open {
            return;
        }
        let original_buffer = input_text(&self.input_editor, ctx);
        let original_input_config = Some(self.input_mode.as_ref(ctx).input_config());
        let include_prompts = !is_shell_mode(self.input_mode.as_ref(ctx));
        let query = original_buffer.clone();
        self.preview_text = None;
        self.state = TuiPromptHistoryMenuState::Open {
            list: TuiInlineMenuListState::default(),
            original_buffer,
            query,
            include_prompts,
        };
        self.original_input_config = original_input_config;
        self.refresh_rows(ctx);
        self.preview_selection(ctx);
    }

    /// Closes the menu and restores the buffer and input mode the user had
    /// before opening it.
    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open(ctx) {
            return;
        }
        let original_buffer = match &self.state {
            TuiPromptHistoryMenuState::Open {
                original_buffer, ..
            } => original_buffer.clone(),
            TuiPromptHistoryMenuState::Closed => return,
        };
        let original_input_config = self.original_input_config.take();
        self.close(ctx);
        self.set_input_text(&original_buffer, ctx);
        self.restore_input_config(original_input_config, ctx);
    }

    /// Moves selection toward older prompts and previews the highlighted one.
    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.has_open_state() {
            return;
        }
        if let TuiPromptHistoryMenuState::Open { list, .. } = &mut self.state {
            list.select_previous(MAX_VISIBLE_ROWS, |_| true);
        }
        self.preview_selection(ctx);
        ctx.emit(TuiPromptHistoryMenuEvent::Updated);
    }

    /// Moves selection toward newer prompts and previews the highlighted one.
    /// Moving down past the newest row, or from an empty list, closes the menu
    /// and restores the buffer.
    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let should_dismiss = match &self.state {
            TuiPromptHistoryMenuState::Open { list, .. } => {
                let count = list.rows().len();
                count == 0 || list.selected_index() == Some(count - 1)
            }
            TuiPromptHistoryMenuState::Closed => return,
        };
        if should_dismiss {
            self.dismiss(ctx);
            return;
        }
        if let TuiPromptHistoryMenuState::Open { list, .. } = &mut self.state {
            list.select_next(MAX_VISIBLE_ROWS, |_| true);
        }
        self.preview_selection(ctx);
        ctx.emit(TuiPromptHistoryMenuEvent::Updated);
    }

    /// Accepts the current selection, closing the menu and returning the accepted
    /// item. With a highlighted row that is its text and kind; with an empty or
    /// filtered-to-nothing list it returns `None` so Enter is a no-op (the menu
    /// stays open and the input keeps its current text).
    pub(crate) fn accept_selected(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Option<TuiHistoryAcceptedItem> {
        if !self.is_open(ctx) {
            return None;
        }
        let selected = match &self.state {
            TuiPromptHistoryMenuState::Open { list, .. } => list.selected_row().cloned(),
            TuiPromptHistoryMenuState::Closed => None,
        };
        let row = selected?;
        self.original_input_config.take();
        self.close(ctx);
        Some(TuiHistoryAcceptedItem {
            text: row.text,
            kind: row.kind,
        })
    }

    /// Returns the render snapshot for the open menu.
    pub(crate) fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        if !self.is_open(ctx) {
            return None;
        }
        let TuiPromptHistoryMenuState::Open { list, query, .. } = &self.state else {
            return None;
        };
        let status = list.rows().is_empty().then(|| {
            if query.trim().is_empty() {
                TuiInlineMenuStatus::Empty("No history".to_owned())
            } else {
                TuiInlineMenuStatus::Empty("No matching history".to_owned())
            }
        });
        Some(TuiInlineMenuSnapshot {
            header: Some(TuiInlineMenuHeader {
                title: Some("History".to_owned()),
                tabs: Vec::new(),
            }),
            rows: list
                .rows()
                .iter()
                .map(|row| TuiInlineMenuRow {
                    title: single_line_menu_title(&row.text),
                    description: None,
                    state_suffix: None,
                    is_selectable: true,
                    style: match row.kind {
                        TuiHistoryKind::Command => TuiInlineMenuRowStyle::ShellCommand,
                        TuiHistoryKind::Prompt => TuiInlineMenuRowStyle::Default,
                    },
                })
                .collect(),
            selected_index: list.selected_index(),
            scroll_offset: list.scroll_offset(),
            max_visible_rows: MAX_VISIBLE_ROWS,
            status,
        })
    }

    /// Re-reads the typed query from the input when the buffer changes from real
    /// typing, ignoring the editor echo of our own preview writes.
    fn on_content_changed(&mut self, ctx: &mut ModelContext<Self>) {
        let current = input_text(&self.input_editor, ctx);
        if self.preview_text.as_deref() == Some(current.as_str()) {
            return;
        }
        self.preview_text = None;
        if let TuiPromptHistoryMenuState::Open { query, .. } = &mut self.state {
            *query = current;
        }
        self.refresh_rows(ctx);
    }

    /// Closes the menu without touching the input buffer.
    fn close(&mut self, ctx: &mut ModelContext<Self>) {
        if self.has_open_state() {
            self.state = TuiPromptHistoryMenuState::Closed;
            self.preview_text = None;
            ctx.emit(TuiPromptHistoryMenuEvent::Updated);
        }
        self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.close_if_active(TuiInputSuggestionsMode::PromptHistory, ctx);
        });
    }

    /// Rebuilds rows from the current query while preserving stable selection,
    /// defaulting to the row nearest the input on first populate. In agent mode
    /// both commands and prompts are included; in `!` shell mode, commands only.
    fn refresh_rows(&mut self, ctx: &mut ModelContext<Self>) {
        let (query, include_prompts, previous_text, previous_index) = match &self.state {
            TuiPromptHistoryMenuState::Open {
                list,
                query,
                include_prompts,
                ..
            } => (
                query.clone(),
                *include_prompts,
                list.selected_row().map(|row| row.text.clone()),
                list.selected_index(),
            ),
            TuiPromptHistoryMenuState::Closed => return,
        };
        let trimmed_query = query.trim();
        // The config is fixed at open time (agent mode => prompts + commands,
        // shell mode => commands only) so preview-driven mode switches don't
        // re-filter the list while the user cycles entries.
        let config = UpArrowHistoryConfig {
            include_commands: true,
            include_prompts,
        };
        let session_id = self
            .active_session
            .as_ref(ctx)
            .session(ctx)
            .map(|session| session.id());
        let rows: Vec<TuiPromptHistoryRow> = history_suggestions_for_terminal_view(
            self.terminal_surface_id,
            session_id,
            config,
            ctx,
        )
        .into_iter()
        .filter(|suggestion| !suggestion.text().trim().is_empty())
        .filter(|suggestion| {
            trimmed_query.is_empty()
                || suggestion
                    .text()
                    .lines()
                    .any(|line| line.starts_with(trimmed_query))
        })
        .map(|suggestion| {
            let text = suggestion.text().to_owned();
            let kind = match suggestion {
                TuiHistorySuggestion::Command { .. } => TuiHistoryKind::Command,
                TuiHistorySuggestion::Prompt { .. } => TuiHistoryKind::Prompt,
            };
            TuiPromptHistoryRow { text, kind }
        })
        .collect();
        let preferred_index =
            reconciled_selection_index(&rows, previous_text.as_deref(), previous_index);
        let TuiPromptHistoryMenuState::Open { list, .. } = &mut self.state else {
            return;
        };
        list.replace_rows(rows, false, preferred_index, MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiPromptHistoryMenuEvent::Updated);
    }

    /// Writes the highlighted row into the input as an undo-agnostic preview and
    /// switches the input mode to match: shell mode for commands (so the `!`
    /// gutter renders while cycling), agent mode for prompts.
    fn preview_selection(&mut self, ctx: &mut ModelContext<Self>) {
        let row = match &self.state {
            TuiPromptHistoryMenuState::Open { list, .. } => list.selected_row().cloned(),
            TuiPromptHistoryMenuState::Closed => None,
        };
        let Some(row) = row else {
            return;
        };
        self.preview_text = Some(row.text.clone());
        self.set_input_mode_for_kind(row.kind, ctx);
        self.set_input_text(&row.text, ctx);
    }

    /// Switches the shared input mode to match a previewed row's kind, mirroring
    /// the GUI's inline-history selection (which locks shell mode while cycling
    /// commands so the `!` indicator renders).
    fn set_input_mode_for_kind(&self, kind: TuiHistoryKind, ctx: &mut ModelContext<Self>) {
        let target_config = match kind {
            TuiHistoryKind::Command => SHELL_LOCKED_CONFIG,
            TuiHistoryKind::Prompt => AI_LOCKED_CONFIG,
        };
        let is_input_buffer_empty = input_text(&self.input_editor, ctx).is_empty();
        self.input_mode.update(ctx, |input_mode, ctx| {
            input_mode.set_input_config(
                target_config,
                is_input_buffer_empty,
                Some(InputTypeAutoDetectionSource::HistorySelection),
                ctx,
            );
        });
    }

    /// Restores the input config captured when the menu opened (dismiss/escape).
    fn restore_input_config(&self, config: Option<InputConfig>, ctx: &mut ModelContext<Self>) {
        let Some(config) = config else {
            return;
        };
        let is_input_buffer_empty = input_text(&self.input_editor, ctx).is_empty();
        self.input_mode.update(ctx, |input_mode, ctx| {
            input_mode.set_input_config(
                config,
                is_input_buffer_empty,
                Some(InputTypeAutoDetectionSource::HistorySelection),
                ctx,
            );
        });
    }

    /// Replaces the input buffer text. Preview and restore both go through here.
    ///
    /// The write is undo-agnostic: after replacing the text we reset the buffer's
    /// undo stack so preview and restore never leave undoable intermediate states
    /// the user could Ctrl+Z into. This mirrors the
    /// GUI's `set_buffer_text_ignoring_undo`; the TUI's `CodeEditorModel` has no
    /// ephemeral overlay, so we clear the stack instead.
    fn set_input_text(&self, text: &str, ctx: &mut ModelContext<Self>) {
        self.input_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer(ctx);
            if !text.is_empty() {
                editor.user_insert(text, ctx);
            }
            editor
                .content()
                .update(ctx, |buffer, _| buffer.reset_undo_stack());
        });
    }
}

/// Preserves selection by prompt text, falling back to the nearest previous
/// index and finally to the last (most-recent) row.
fn reconciled_selection_index(
    rows: &[TuiPromptHistoryRow],
    previous_text: Option<&str>,
    previous_index: Option<usize>,
) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let last = rows.len() - 1;
    if let Some(text) = previous_text
        && let Some(index) = rows.iter().position(|row| row.text == text)
    {
        return Some(index);
    }
    Some(previous_index.unwrap_or(last).min(last))
}

impl Entity for TuiPromptHistoryMenuModel {
    type Event = TuiPromptHistoryMenuEvent;
}

/// Returns the input editor's current plain text.
fn input_text(editor: &ModelHandle<CodeEditorModel>, app: &AppContext) -> String {
    let model = editor.as_ref(app);
    let buffer = model.content().as_ref(app);
    if buffer.is_empty() {
        String::new()
    } else {
        buffer.text().into_string()
    }
}

#[cfg(test)]
#[path = "prompt_history_menu_tests.rs"]
mod tests;
