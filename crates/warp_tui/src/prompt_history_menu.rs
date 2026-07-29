//! Up-arrow prompt and command history inline menu state for the TUI.
//!
//! Mirrors the GUI's inline prompt-history recall (see `CODE-1871`): pressing Up
//! with the caret on the first visual row opens this menu of previously-submitted
//! prompts and commands, filtered by whatever is already typed. Selection previews the
//! highlighted item into the input, Enter accepts it, and Escape (or
//! moving down past the newest row) restores the buffer the user started with.
//!
//! The list comes from the shared [`tui_history_for_terminal_view`] getter so
//! the TUI and GUI read identically ordered, filtered, and de-duplicated history.
//! The model owns filtering, menu lifecycle, selection, preview, and buffer
//! snapshot/restore; the terminal session view routes accepted items by type.
use std::rc::Rc;

use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{
    ActiveSession, BlocklistAIHistoryEvent, BlocklistAIHistoryModel, BlocklistAIInputModel,
    History, HistoryEvent, InputConfig, InputTypeAutoDetectionSource, TuiHistoryItem,
    tui_history_for_terminal_view,
};
use warp_core::SessionId;
use warp_editor::model::CoreEditorModel;
use warpui_core::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use crate::inline_menu::{
    MAX_INLINE_MENU_ROWS, TuiInlineMenuHeader, TuiInlineMenuListState, TuiInlineMenuRow,
    TuiInlineMenuRowStyle, TuiInlineMenuSnapshot, TuiInlineMenuStatus, result_row_capacity,
    single_line_menu_title,
};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);
type SessionIdProvider = dyn Fn(&AppContext) -> Option<SessionId>;
type HistoryItemsProvider =
    dyn Fn(EntityId, Option<SessionId>, InputConfig, &AppContext) -> Vec<TuiHistoryItem>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct TuiPromptHistoryRow {
    item: TuiHistoryItem,
}

impl TuiPromptHistoryRow {
    fn text(&self) -> &str {
        self.item.text()
    }
}

#[derive(Debug, Clone, Default)]
enum TuiPromptHistoryMenuState {
    #[default]
    Closed,
    Open {
        list: TuiInlineMenuListState<TuiPromptHistoryRow>,
        /// The input buffer captured when the menu opened, restored on dismiss.
        original_buffer: String,
        /// The input type/lock state captured when the menu opened. History
        /// previews may change the type but must not change which item types
        /// are included in the current menu.
        original_input_config: InputConfig,
        /// The user's typed search query. Held separately from the input buffer
        /// so selection previews (which overwrite the buffer) do not change what
        /// the list filters against.
        query: String,
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
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    terminal_surface_id: EntityId,
    session_id: Rc<SessionIdProvider>,
    history_items: Rc<HistoryItemsProvider>,
    state: TuiPromptHistoryMenuState,
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
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        active_session: ModelHandle<ActiveSession>,
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let active_session_for_id = active_session.clone();
        Self::new_with_providers(
            input_editor,
            input_mode,
            suggestions_mode,
            terminal_surface_id,
            Rc::new(move |app| {
                active_session_for_id
                    .as_ref(app)
                    .session(app)
                    .map(|session| session.id())
            }),
            Rc::new(tui_history_for_terminal_view),
            ctx,
        )
    }

    fn new_with_providers(
        input_editor: ModelHandle<CodeEditorModel>,
        input_mode: ModelHandle<BlocklistAIInputModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        terminal_surface_id: EntityId,
        session_id: Rc<SessionIdProvider>,
        history_items: Rc<HistoryItemsProvider>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&input_editor, |model, _, event, ctx| {
            if model.is_open(ctx) && matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                model.on_content_changed(ctx);
            }
        });
        if ctx.has_singleton_model::<BlocklistAIHistoryModel>() {
            ctx.subscribe_to_model(
                &BlocklistAIHistoryModel::handle(ctx),
                |model, _, _: &BlocklistAIHistoryEvent, ctx| {
                    if model.is_open(ctx) {
                        model.refresh_rows(ctx);
                    }
                },
            );
        }
        if ctx.has_singleton_model::<History>() {
            ctx.subscribe_to_model(&History::handle(ctx), |model, _, _: &HistoryEvent, ctx| {
                if model.is_open(ctx) {
                    model.refresh_rows(ctx);
                }
            });
        }
        Self {
            input_editor,
            input_mode,
            suggestions_mode,
            terminal_surface_id,
            session_id,
            history_items,
            state: TuiPromptHistoryMenuState::Closed,
            preview_text: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        input_editor: ModelHandle<CodeEditorModel>,
        input_mode: ModelHandle<BlocklistAIInputModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        terminal_surface_id: EntityId,
        items: Vec<TuiHistoryItem>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        Self::new_with_providers(
            input_editor,
            input_mode,
            suggestions_mode,
            terminal_surface_id,
            Rc::new(|_| None),
            Rc::new(move |_, _, config, _| {
                let explicit_shell = config.is_locked && config.is_shell();
                items
                    .iter()
                    .filter(|item| !explicit_shell || matches!(item, TuiHistoryItem::Command(_)))
                    .cloned()
                    .collect()
            }),
            ctx,
        )
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
        let original_input_config = self.input_mode.as_ref(ctx).input_config();
        let query = original_buffer.clone();
        self.preview_text = None;
        self.state = TuiPromptHistoryMenuState::Open {
            list: TuiInlineMenuListState::default(),
            original_buffer,
            original_input_config,
            query,
        };
        self.refresh_rows(ctx);
        self.preview_selection(ctx);
    }

    /// Closes the menu and restores the buffer the user had before opening it.
    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open(ctx) {
            return;
        }
        let (original_buffer, original_input_config) = match &self.state {
            TuiPromptHistoryMenuState::Open {
                original_buffer,
                original_input_config,
                ..
            } => (original_buffer.clone(), *original_input_config),
            TuiPromptHistoryMenuState::Closed => return,
        };
        self.close(ctx);
        self.set_input_config(
            original_input_config,
            original_buffer.is_empty(),
            InputTypeAutoDetectionSource::RestoreSavedConfig,
            ctx,
        );
        self.set_input_text(&original_buffer, ctx);
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

    /// Accepts the current selection, preserving its prompt/command type. An
    /// empty or filtered-to-nothing list is a no-op and remains open.
    pub(crate) fn accept_selected(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Option<TuiHistoryItem> {
        if !self.is_open(ctx) {
            return None;
        }
        let selected = match &self.state {
            TuiPromptHistoryMenuState::Open { list, .. } => {
                list.selected_row().map(|row| row.item.clone())
            }
            TuiPromptHistoryMenuState::Closed => None,
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
                    title: single_line_menu_title(row.text()),
                    description: None,
                    state_suffix: None,
                    is_selectable: true,
                    style: match &row.item {
                        TuiHistoryItem::Command(_) => TuiInlineMenuRowStyle::HistoryCommand,
                        TuiHistoryItem::Prompt(_) => TuiInlineMenuRowStyle::Default,
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
    /// defaulting to the row nearest the input on first populate.
    fn refresh_rows(&mut self, ctx: &mut ModelContext<Self>) {
        let (query, input_config, previous_item, previous_index) = match &self.state {
            TuiPromptHistoryMenuState::Open {
                list,
                query,
                original_input_config,
                ..
            } => (
                query.clone(),
                *original_input_config,
                list.selected_row().map(|row| row.item.clone()),
                list.selected_index(),
            ),
            TuiPromptHistoryMenuState::Closed => return,
        };
        let trimmed_query = query.trim();
        let session_id = (self.session_id)(ctx);
        let rows: Vec<TuiPromptHistoryRow> =
            (self.history_items)(self.terminal_surface_id, session_id, input_config, ctx)
                .into_iter()
                .filter(|item| {
                    trimmed_query.is_empty()
                        || item
                            .text()
                            .lines()
                            .any(|line| line.starts_with(trimmed_query))
                })
                .map(|item| TuiPromptHistoryRow { item })
                .collect();
        let preferred_index =
            reconciled_selection_index(&rows, previous_item.as_ref(), previous_index);
        let TuiPromptHistoryMenuState::Open { list, .. } = &mut self.state else {
            return;
        };
        list.replace_rows(rows, false, preferred_index, MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiPromptHistoryMenuEvent::Updated);
    }

    /// Writes the highlighted prompt into the input as an undo-agnostic preview.
    fn preview_selection(&mut self, ctx: &mut ModelContext<Self>) {
        let item = match &self.state {
            TuiPromptHistoryMenuState::Open { list, .. } => {
                list.selected_row().map(|row| row.item.clone())
            }
            TuiPromptHistoryMenuState::Closed => None,
        };
        let Some(item) = item else {
            return;
        };
        let text = item.text().to_owned();
        self.input_mode.update(ctx, |input_mode, ctx| {
            input_mode.set_input_type(
                item.input_type(),
                Some(InputTypeAutoDetectionSource::HistorySelection),
                ctx,
            );
        });
        self.preview_text = Some(text.clone());
        self.set_input_text(&text, ctx);
    }

    fn set_input_config(
        &self,
        config: InputConfig,
        is_input_buffer_empty: bool,
        source: InputTypeAutoDetectionSource,
        ctx: &mut ModelContext<Self>,
    ) {
        self.input_mode.update(ctx, |input_mode, ctx| {
            input_mode.set_input_config(config, is_input_buffer_empty, Some(source), ctx);
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
    previous_item: Option<&TuiHistoryItem>,
    previous_index: Option<usize>,
) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let last = rows.len() - 1;
    if let Some(item) = previous_item
        && let Some(index) = rows.iter().position(|row| &row.item == item)
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
