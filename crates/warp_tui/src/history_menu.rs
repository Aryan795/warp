//! Up-arrow history inline menu state for the TUI.
//!
//! Mirrors the GUI's inline history recall: pressing Up with the caret on the
//! first visual row opens this menu of previously-submitted agent prompts and
//! previously-executed shell commands, filtered by whatever is already typed.
//! In agent mode prompts and commands are interleaved; in `!` shell mode only
//! commands are shown. Selection previews the highlighted item into the input
//! — commands preview in shell mode — Enter executes a command or fills and
//! submits a prompt, and Escape (or moving down past the newest row) restores
//! the buffer and input mode the user started with.
//!
//! The item list comes from the shared [`up_arrow_history_for_terminal_view`]
//! getter so the TUI and GUI read identically ordered, session-scoped, and
//! de-duplicated history. The model owns filtering, menu lifecycle, selection,
//! preview, and buffer/mode snapshot/restore; the terminal session view
//! executes or submits an accepted item.
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{
    ActiveSession, BlocklistAIHistoryEvent, BlocklistAIHistoryModel, BlocklistAIInputModel,
    History as CommandHistory, HistoryEvent as CommandHistoryEvent, InputConfig,
    InputTypeAutoDetectionSource, UpArrowHistoryEntry, UpArrowHistoryEntryKind,
    up_arrow_history_for_terminal_view,
};
use warp_editor::model::CoreEditorModel;
use warpui_core::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use crate::inline_menu::{
    MAX_INLINE_MENU_ROWS, TuiInlineMenuHeader, TuiInlineMenuListState, TuiInlineMenuRow,
    TuiInlineMenuRowStyle, TuiInlineMenuSnapshot, TuiInlineMenuStatus, result_row_capacity,
    single_line_menu_title,
};
use crate::input_mode_policy::{self, SHELL_LOCKED_CONFIG};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);

#[derive(Debug, Clone, Default)]
enum TuiHistoryMenuState {
    #[default]
    Closed,
    Open {
        list: TuiInlineMenuListState<UpArrowHistoryEntry>,
        /// The input buffer captured when the menu opened, restored on dismiss.
        original_buffer: String,
        /// The input config captured when the menu opened. Command previews
        /// lock the input to shell mode, so dismissing restores this config
        /// alongside the buffer.
        original_config: InputConfig,
        /// Whether prompts are shown. Snapshotted at open (`!` shell mode
        /// shows commands only) so command previews flipping the live input
        /// mode to shell never re-filter the open menu.
        include_prompts: bool,
        /// The user's typed search query. Held separately from the input buffer
        /// so selection previews (which overwrite the buffer) do not change what
        /// the list filters against.
        query: String,
    },
}

/// Events emitted by the TUI history menu.
#[derive(Debug, Clone, Copy)]
pub(crate) enum TuiHistoryMenuEvent {
    Updated,
}

/// Query, selection, preview, and model-subscription state for the up-arrow
/// history menu.
pub(crate) struct TuiHistoryMenuModel {
    input_editor: ModelHandle<CodeEditorModel>,
    /// Shared input-mode model: read to scope the list to commands in shell
    /// mode, written so command previews carry the `!` shell affordance.
    input_mode: ModelHandle<BlocklistAIInputModel>,
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    /// Resolves the live session whose command history is shown.
    active_session: ModelHandle<ActiveSession>,
    terminal_surface_id: EntityId,
    state: TuiHistoryMenuState,
    /// The text most recently written into the input as a preview. Content
    /// changes matching it are the editor echoing our own preview write and are
    /// ignored so they don't clobber the typed query. Model events are delivered
    /// after the current update flushes, so a transient set/reset flag around the
    /// write would not survive to the deferred handler — hence a content compare.
    preview_text: Option<String>,
}

impl TuiHistoryMenuModel {
    /// Creates a closed history menu and subscribes it to input/history
    /// changes.
    pub(crate) fn new(
        input_editor: ModelHandle<CodeEditorModel>,
        input_mode: ModelHandle<BlocklistAIInputModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        active_session: ModelHandle<ActiveSession>,
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
        // The shell-command history singleton is absent in some headless test
        // setups; refreshing on its events only matters where it exists (e.g.
        // an async histfile read completing while the menu is open).
        if ctx.has_singleton_model::<CommandHistory>() {
            ctx.subscribe_to_model(
                &CommandHistory::handle(ctx),
                |model, _, _: &CommandHistoryEvent, ctx| {
                    if model.is_open(ctx) {
                        model.refresh_rows(ctx);
                    }
                },
            );
        }
        Self {
            input_editor,
            input_mode,
            suggestions_mode,
            active_session,
            terminal_surface_id,
            state: TuiHistoryMenuState::Closed,
            preview_text: None,
        }
    }

    fn has_open_state(&self) -> bool {
        matches!(self.state, TuiHistoryMenuState::Open { .. })
    }

    pub(crate) fn is_open(&self, ctx: &AppContext) -> bool {
        self.has_open_state()
            && self.suggestions_mode.as_ref(ctx).mode() == TuiInputSuggestionsMode::History
    }

    /// Opens the menu, snapshotting the current input and input mode as the
    /// restorable originals and the current text as the initial search query,
    /// then previews the default selection.
    pub(crate) fn open(&mut self, ctx: &mut ModelContext<Self>) {
        if self.has_open_state() {
            return;
        }
        let did_open = self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.try_open(TuiInputSuggestionsMode::History, ctx)
        });
        if !did_open {
            return;
        }
        let original_buffer = input_text(&self.input_editor, ctx);
        let original_config = self.input_mode.as_ref(ctx).input_config();
        let include_prompts = !input_mode_policy::is_shell_mode(self.input_mode.as_ref(ctx));
        let query = original_buffer.clone();
        self.preview_text = None;
        self.state = TuiHistoryMenuState::Open {
            list: TuiInlineMenuListState::default(),
            original_buffer,
            original_config,
            include_prompts,
            query,
        };
        self.refresh_rows(ctx);
        self.preview_selection(ctx);
    }

    /// Closes the menu and restores the buffer and input mode the user had
    /// before opening it.
    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open(ctx) {
            return;
        }
        let (original_buffer, original_config) = match &self.state {
            TuiHistoryMenuState::Open {
                original_buffer,
                original_config,
                ..
            } => (original_buffer.clone(), *original_config),
            TuiHistoryMenuState::Closed => return,
        };
        self.close(ctx);
        self.set_input_text(&original_buffer, ctx);
        self.set_input_config(original_config, ctx);
    }

    /// Moves selection toward older items and previews the highlighted one.
    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.has_open_state() {
            return;
        }
        if let TuiHistoryMenuState::Open { list, .. } = &mut self.state {
            list.select_previous(MAX_VISIBLE_ROWS, |_| true);
        }
        self.preview_selection(ctx);
        ctx.emit(TuiHistoryMenuEvent::Updated);
    }

    /// Moves selection toward newer items and previews the highlighted one.
    /// Moving down past the newest row, or from an empty list, closes the menu
    /// and restores the buffer and input mode.
    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let should_dismiss = match &self.state {
            TuiHistoryMenuState::Open { list, .. } => {
                let count = list.rows().len();
                count == 0 || list.selected_index() == Some(count - 1)
            }
            TuiHistoryMenuState::Closed => return,
        };
        if should_dismiss {
            self.dismiss(ctx);
            return;
        }
        if let TuiHistoryMenuState::Open { list, .. } = &mut self.state {
            list.select_next(MAX_VISIBLE_ROWS, |_| true);
        }
        self.preview_selection(ctx);
        ctx.emit(TuiHistoryMenuEvent::Updated);
    }

    /// Accepts the current selection, closing the menu and returning the item
    /// to execute (command) or submit (prompt). With an empty or
    /// filtered-to-nothing list there is nothing to accept: Enter is a no-op
    /// and the menu stays open.
    pub(crate) fn accept_selected(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Option<UpArrowHistoryEntry> {
        if !self.is_open(ctx) {
            return None;
        }
        let selected = match &self.state {
            TuiHistoryMenuState::Open { list, .. } => list.selected_row().cloned(),
            TuiHistoryMenuState::Closed => None,
        };
        let selected = selected?;
        self.close(ctx);
        Some(selected)
    }

    /// Returns the render snapshot for the open menu.
    pub(crate) fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        if !self.is_open(ctx) {
            return None;
        }
        let TuiHistoryMenuState::Open { list, query, .. } = &self.state else {
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
                .map(|entry| TuiInlineMenuRow {
                    title: single_line_menu_title(&entry.text),
                    description: None,
                    state_suffix: None,
                    is_selectable: true,
                    style: match entry.kind {
                        UpArrowHistoryEntryKind::Command => TuiInlineMenuRowStyle::ShellCommand,
                        UpArrowHistoryEntryKind::Prompt => TuiInlineMenuRowStyle::Default,
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
        if let TuiHistoryMenuState::Open { query, .. } = &mut self.state {
            *query = current;
        }
        self.refresh_rows(ctx);
    }

    /// Closes the menu without touching the input buffer or input mode.
    fn close(&mut self, ctx: &mut ModelContext<Self>) {
        if self.has_open_state() {
            self.state = TuiHistoryMenuState::Closed;
            self.preview_text = None;
            ctx.emit(TuiHistoryMenuEvent::Updated);
        }
        self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.close_if_active(TuiInputSuggestionsMode::History, ctx);
        });
    }

    /// Rebuilds rows from the current query while preserving stable selection,
    /// defaulting to the row nearest the input on first populate.
    fn refresh_rows(&mut self, ctx: &mut ModelContext<Self>) {
        let (query, include_prompts, previous_row, previous_index) = match &self.state {
            TuiHistoryMenuState::Open {
                list,
                query,
                include_prompts,
                ..
            } => (
                query.clone(),
                *include_prompts,
                list.selected_row().cloned(),
                list.selected_index(),
            ),
            TuiHistoryMenuState::Closed => return,
        };
        let trimmed_query = query.trim();
        let session_id = self
            .active_session
            .as_ref(ctx)
            .session(ctx)
            .map(|session| session.id());
        let rows: Vec<UpArrowHistoryEntry> = up_arrow_history_for_terminal_view(
            self.terminal_surface_id,
            session_id,
            include_prompts,
            ctx,
        )
        .into_iter()
        .filter(|entry| {
            trimmed_query.is_empty()
                || entry
                    .text
                    .lines()
                    .any(|line| line.starts_with(trimmed_query))
        })
        .collect();
        let preferred_index =
            reconciled_selection_index(&rows, previous_row.as_ref(), previous_index);
        let TuiHistoryMenuState::Open { list, .. } = &mut self.state else {
            return;
        };
        list.replace_rows(rows, false, preferred_index, MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiHistoryMenuEvent::Updated);
    }

    /// Writes the highlighted item into the input as an undo-agnostic preview.
    /// Commands preview in `!` shell mode; prompts restore the mode the menu
    /// opened with.
    fn preview_selection(&mut self, ctx: &mut ModelContext<Self>) {
        let (selected, original_config) = match &self.state {
            TuiHistoryMenuState::Open {
                list,
                original_config,
                ..
            } => (list.selected_row().cloned(), Some(*original_config)),
            TuiHistoryMenuState::Closed => (None, None),
        };
        let (Some(entry), Some(original_config)) = (selected, original_config) else {
            return;
        };
        self.preview_text = Some(entry.text.clone());
        self.set_input_text(&entry.text, ctx);
        let config = match entry.kind {
            UpArrowHistoryEntryKind::Command => SHELL_LOCKED_CONFIG,
            UpArrowHistoryEntryKind::Prompt => original_config,
        };
        self.set_input_config(config, ctx);
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

    /// Applies an input config for a preview or a dismiss-time restore. Shell
    /// previews carry the same `ShellPrefix` source as a typed `!` so the
    /// shared model treats them exactly like the manual shell affordance.
    fn set_input_config(&self, config: InputConfig, ctx: &mut ModelContext<Self>) {
        if self.input_mode.as_ref(ctx).input_config() == config {
            return;
        }
        let source =
            (config == SHELL_LOCKED_CONFIG).then_some(InputTypeAutoDetectionSource::ShellPrefix);
        let is_input_buffer_empty = input_text(&self.input_editor, ctx).is_empty();
        self.input_mode.update(ctx, |input_mode, ctx| {
            input_mode.set_input_config(config, is_input_buffer_empty, source, ctx);
        });
    }
}

/// Preserves selection by item text and kind, falling back to the nearest
/// previous index and finally to the last (most-recent) row.
fn reconciled_selection_index(
    rows: &[UpArrowHistoryEntry],
    previous_row: Option<&UpArrowHistoryEntry>,
    previous_index: Option<usize>,
) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let last = rows.len() - 1;
    if let Some(previous_row) = previous_row
        && let Some(index) = rows.iter().position(|row| row == previous_row)
    {
        return Some(index);
    }
    Some(previous_index.unwrap_or(last).min(last))
}

impl Entity for TuiHistoryMenuModel {
    type Event = TuiHistoryMenuEvent;
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
#[path = "history_menu_tests.rs"]
mod tests;
