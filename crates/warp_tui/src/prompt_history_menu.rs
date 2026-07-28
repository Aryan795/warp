//! Up-arrow combined history inline menu state for the TUI.
//!
//! Extends the GUI-parity prompt-history recall into a combined prompt + command
//! history menu: pressing Up with the caret on the first visual row opens this
//! menu of previously-submitted agent prompts and executed shell commands,
//! filtered by whatever is already typed.
//!
//! - In agent mode, prompts and commands are interleaved.
//! - In `!` shell mode, only commands are shown.
//! - Selecting a command previews it in shell mode; selecting a prompt previews
//!   it as an agent prompt. Enter accepts the selection (submit prompt / execute
//!   command). Enter on an empty/filtered-to-nothing list is a no-op.
//!
//! The list comes from the shared [`up_arrow_history_for_terminal_view`] getter so
//! the TUI and GUI read identically ordered and de-duplicated history. The model
//! owns filtering, menu lifecycle, selection, preview, and buffer snapshot/restore;
//! the terminal session view submits an accepted entry.
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{
    ActiveSession, BlocklistAIHistoryEvent, BlocklistAIHistoryModel, BlocklistAIInputModel,
    InputTypeAutoDetectionSource, UpArrowHistoryConfig, UpArrowHistoryEntry,
    up_arrow_history_for_terminal_view,
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

/// Kind of accepted history entry for the input/session owner to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiHistoryAccepted {
    /// Submit `text` as an agent prompt.
    Prompt { text: String },
    /// Execute `text` as a shell command (input is already in shell mode).
    Command { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TuiHistoryRow {
    pub(crate) entry: UpArrowHistoryEntry,
}

impl TuiHistoryRow {
    fn text(&self) -> &str {
        self.entry.text()
    }

    fn is_command(&self) -> bool {
        self.entry.is_command()
    }
}

#[derive(Debug, Clone, Default)]
enum TuiPromptHistoryMenuState {
    #[default]
    Closed,
    Open {
        list: TuiInlineMenuListState<TuiHistoryRow>,
        /// The input buffer captured when the menu opened, restored on dismiss.
        original_buffer: String,
        /// Whether the input was in shell mode when the menu opened.
        original_was_shell: bool,
        /// The user's typed search query. Held separately from the input buffer
        /// so selection previews (which overwrite the buffer) do not change what
        /// the list filters against.
        query: String,
        /// Whether this open session includes prompts (agent mode) or is
        /// commands-only (`!` shell mode).
        include_prompts: bool,
    },
}

/// Events emitted by the TUI history menu.
#[derive(Debug, Clone, Copy)]
pub(crate) enum TuiPromptHistoryMenuEvent {
    Updated,
}

/// Query, selection, preview, and model-subscription state for the up-arrow
/// combined history menu.
pub(crate) struct TuiPromptHistoryMenuModel {
    input_editor: ModelHandle<CodeEditorModel>,
    input_mode: ModelHandle<BlocklistAIInputModel>,
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    terminal_surface_id: EntityId,
    /// Active shell session used for command-history scoping/ordering.
    active_session: Option<ModelHandle<ActiveSession>>,
    state: TuiPromptHistoryMenuState,
    /// The text most recently written into the input as a preview. Content
    /// changes matching it are the editor echoing our own preview write and are
    /// ignored so they don't clobber the typed query. Model events are delivered
    /// after the current update flushes, so a transient set/reset flag around the
    /// write would not survive to the deferred handler — hence a content compare.
    preview_text: Option<String>,
}

impl TuiPromptHistoryMenuModel {
    /// Creates a closed history menu and subscribes it to input/history changes.
    pub(crate) fn new(
        input_editor: ModelHandle<CodeEditorModel>,
        input_mode: ModelHandle<BlocklistAIInputModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        terminal_surface_id: EntityId,
        active_session: Option<ModelHandle<ActiveSession>>,
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
            suggestions_mode,
            terminal_surface_id,
            active_session,
            state: TuiPromptHistoryMenuState::Closed,
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
        let original_was_shell = is_shell_mode(self.input_mode.as_ref(ctx));
        let include_prompts = !original_was_shell;
        let query = original_buffer.clone();
        self.preview_text = None;
        self.state = TuiPromptHistoryMenuState::Open {
            list: TuiInlineMenuListState::default(),
            original_buffer,
            original_was_shell,
            query,
            include_prompts,
        };
        self.refresh_rows(ctx);
        self.preview_selection(ctx);
    }

    /// Closes the menu and restores the buffer (and input mode) the user had
    /// before opening it.
    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open(ctx) {
            return;
        }
        let (original_buffer, original_was_shell) = match &self.state {
            TuiPromptHistoryMenuState::Open {
                original_buffer,
                original_was_shell,
                ..
            } => (original_buffer.clone(), *original_was_shell),
            TuiPromptHistoryMenuState::Closed => return,
        };
        self.close(ctx);
        self.restore_input_mode(original_was_shell, &original_buffer, ctx);
        self.set_input_text(&original_buffer, ctx);
    }

    /// Moves selection toward older entries and previews the highlighted one.
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

    /// Moves selection toward newer entries and previews the highlighted one.
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

    /// Accepts the current selection, closing the menu and returning the entry
    /// to act on. With an empty or filtered-to-nothing list, returns `None`
    /// (Enter is a no-op) without restoring the original buffer — the current
    /// typed text stays in place.
    pub(crate) fn accept_selected(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Option<TuiHistoryAccepted> {
        if !self.is_open(ctx) {
            return None;
        }
        let selected = match &self.state {
            TuiPromptHistoryMenuState::Open { list, .. } => list.selected_row().map(|row| {
                let text = row.text().to_owned();
                if row.is_command() {
                    TuiHistoryAccepted::Command { text }
                } else {
                    TuiHistoryAccepted::Prompt { text }
                }
            }),
            TuiPromptHistoryMenuState::Closed => None,
        };
        // Empty list: close without accepting or restoring (typed text remains).
        if selected.is_none() {
            self.close(ctx);
            return None;
        }
        self.close(ctx);
        selected
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
                .map(|row| {
                    let title = if row.is_command() {
                        format!("! {}", single_line_menu_title(row.text()))
                    } else {
                        single_line_menu_title(row.text())
                    };
                    TuiInlineMenuRow {
                        title,
                        description: None,
                        state_suffix: None,
                        is_selectable: true,
                        style: if row.is_command() {
                            TuiInlineMenuRowStyle::ShellCommand
                        } else {
                            TuiInlineMenuRowStyle::Default
                        },
                    }
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
    /// defaulting to the row nearest the input on first populate.
    fn refresh_rows(&mut self, ctx: &mut ModelContext<Self>) {
        let (query, previous_text, previous_index, include_prompts) = match &self.state {
            TuiPromptHistoryMenuState::Open {
                list,
                query,
                include_prompts,
                ..
            } => (
                query.clone(),
                list.selected_row().map(|row| row.text().to_owned()),
                list.selected_index(),
                *include_prompts,
            ),
            TuiPromptHistoryMenuState::Closed => return,
        };
        let trimmed_query = query.trim();
        let config = if include_prompts {
            UpArrowHistoryConfig::agent_mode()
        } else {
            UpArrowHistoryConfig::shell_mode()
        };
        let session_id = self
            .active_session
            .as_ref()
            .and_then(|session| session.as_ref(ctx).session(ctx))
            .map(|session| session.id());
        let rows: Vec<TuiHistoryRow> =
            up_arrow_history_for_terminal_view(self.terminal_surface_id, session_id, config, ctx)
                .into_iter()
                .filter(|entry| {
                    trimmed_query.is_empty()
                        || entry
                            .text()
                            .lines()
                            .any(|line| line.starts_with(trimmed_query))
                })
                .map(|entry| TuiHistoryRow { entry })
                .collect();
        let preferred_index =
            reconciled_selection_index(&rows, previous_text.as_deref(), previous_index);
        let TuiPromptHistoryMenuState::Open { list, .. } = &mut self.state else {
            return;
        };
        list.replace_rows(rows, false, preferred_index, MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiPromptHistoryMenuEvent::Updated);
    }

    /// Writes the highlighted entry into the input as an undo-agnostic preview,
    /// switching to shell mode for commands and agent mode for prompts.
    fn preview_selection(&mut self, ctx: &mut ModelContext<Self>) {
        let selected = match &self.state {
            TuiPromptHistoryMenuState::Open { list, .. } => list
                .selected_row()
                .map(|row| (row.text().to_owned(), row.is_command())),
            TuiPromptHistoryMenuState::Closed => None,
        };
        let Some((text, is_command)) = selected else {
            return;
        };
        if is_command {
            self.enter_shell_mode_for_preview(ctx);
        } else {
            self.enter_agent_mode_for_preview(ctx);
        }
        self.preview_text = Some(text.clone());
        self.set_input_text(&text, ctx);
    }

    fn enter_shell_mode_for_preview(&self, ctx: &mut ModelContext<Self>) {
        if is_shell_mode(self.input_mode.as_ref(ctx)) {
            return;
        }
        let is_empty = input_text(&self.input_editor, ctx).is_empty();
        self.input_mode.update(ctx, |input_mode, ctx| {
            input_mode.set_input_config(
                SHELL_LOCKED_CONFIG,
                is_empty,
                Some(InputTypeAutoDetectionSource::HistorySelection),
                ctx,
            );
        });
    }

    fn enter_agent_mode_for_preview(&self, ctx: &mut ModelContext<Self>) {
        if !is_shell_mode(self.input_mode.as_ref(ctx)) {
            return;
        }
        let is_empty = input_text(&self.input_editor, ctx).is_empty();
        self.input_mode.update(ctx, |input_mode, ctx| {
            input_mode.set_input_config(
                AI_LOCKED_CONFIG,
                is_empty,
                Some(InputTypeAutoDetectionSource::HistorySelection),
                ctx,
            );
        });
    }

    fn restore_input_mode(
        &self,
        original_was_shell: bool,
        original_buffer: &str,
        ctx: &mut ModelContext<Self>,
    ) {
        let currently_shell = is_shell_mode(self.input_mode.as_ref(ctx));
        if original_was_shell == currently_shell {
            return;
        }
        let is_empty = original_buffer.is_empty();
        let config = if original_was_shell {
            SHELL_LOCKED_CONFIG
        } else {
            AI_LOCKED_CONFIG
        };
        self.input_mode.update(ctx, |input_mode, ctx| {
            input_mode.set_input_config(
                config,
                is_empty,
                Some(InputTypeAutoDetectionSource::RestoreSavedConfig),
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

/// Preserves selection by entry text, falling back to the nearest previous
/// index and finally to the last (most-recent) row.
pub(crate) fn reconciled_selection_index(
    rows: &[TuiHistoryRow],
    previous_text: Option<&str>,
    previous_index: Option<usize>,
) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let last = rows.len() - 1;
    if let Some(text) = previous_text
        && let Some(index) = rows.iter().position(|row| row.text() == text)
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
