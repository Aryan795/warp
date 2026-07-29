//! Up-arrow history inline menu state for the TUI.
//!
//! Mirrors the GUI's inline history recall (see `CODE-1871` / `CODE-1906`):
//! pressing Up with the caret on the first visual row opens this menu of
//! previously-run history, filtered by whatever is already typed. In agent input
//! mode the menu interleaves submitted agent prompts and executed shell commands;
//! in `!` shell mode it shows commands only. Selection previews the highlighted
//! item into the input (commands preview in shell mode, prompts in agent mode),
//! Enter accepts it — executing a command or submitting a prompt — and Escape (or
//! moving down past the newest row) restores the buffer and input mode the user
//! started with.
//!
//! The combined, ordered, de-duplicated history comes from the shared,
//! frontend-agnostic [`up_arrow_history_for_terminal_view`] projection so the TUI
//! and GUI read identical history. The model owns filtering, menu lifecycle,
//! selection, preview, buffer/mode snapshot-and-restore, and the accepted item's
//! kind; the terminal session view executes or submits it.
use std::rc::Rc;

use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, BlocklistAIInputModel, InputConfig,
    InputTypeAutoDetectionSource, SessionId, TuiUpArrowHistoryEntry, TuiUpArrowHistoryKind,
    up_arrow_history_for_terminal_view,
};
use warp_editor::model::CoreEditorModel;
use warpui_core::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use crate::inline_menu::{
    MAX_INLINE_MENU_ROWS, TuiInlineMenuHeader, TuiInlineMenuListState, TuiInlineMenuRow,
    TuiInlineMenuRowStyle, TuiInlineMenuSnapshot, TuiInlineMenuStatus, result_row_capacity,
    single_line_menu_title,
};
use crate::input_mode_policy::{self, AI_LOCKED_CONFIG, SHELL_LOCKED_CONFIG};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);

/// Resolves the current terminal session id when the menu opens. Held as a
/// closure so the model stays decoupled from the session plumbing (production
/// captures the surface's `ActiveSession`; tests can supply a fixed id or none).
pub(crate) type SessionIdResolver = Rc<dyn Fn(&AppContext) -> Option<SessionId>>;

/// A history item accepted from the up-arrow menu, tagged with how it should be
/// applied: a shell command to execute or an agent prompt to submit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiHistoryAcceptance {
    Command(String),
    Prompt(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TuiHistoryRow {
    text: String,
    kind: TuiUpArrowHistoryKind,
}

#[derive(Default)]
enum TuiHistoryMenuState {
    #[default]
    Closed,
    Open {
        list: TuiInlineMenuListState<TuiHistoryRow>,
        /// The input buffer captured when the menu opened, restored on dismiss.
        original_buffer: String,
        /// The input config captured when the menu opened, restored on dismiss so
        /// previewing a command (shell mode) or prompt (agent mode) never leaks.
        original_config: InputConfig,
        /// Whether agent prompts are interleaved with commands. Captured once at
        /// open (agent mode) so previewing across kinds never reshapes the list.
        include_prompts: bool,
        /// The session whose executed commands are shown, resolved at open.
        session_id: Option<SessionId>,
        /// The user's typed search query. Held separately from the input buffer
        /// so selection previews (which overwrite the buffer) do not change what
        /// the list filters against.
        query: String,
    },
}

/// Events emitted by the TUI history menu.
#[derive(Debug, Clone, Copy)]
pub(crate) enum TuiPromptHistoryMenuEvent {
    Updated,
}

/// Query, selection, preview, and model-subscription state for the up-arrow
/// history menu.
pub(crate) struct TuiPromptHistoryMenuModel {
    input_editor: ModelHandle<CodeEditorModel>,
    input_mode: ModelHandle<BlocklistAIInputModel>,
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    session_id_resolver: SessionIdResolver,
    terminal_surface_id: EntityId,
    state: TuiHistoryMenuState,
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
        session_id_resolver: SessionIdResolver,
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
            suggestions_mode,
            session_id_resolver,
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
            && self.suggestions_mode.as_ref(ctx).mode() == TuiInputSuggestionsMode::PromptHistory
    }

    /// Opens the menu, snapshotting the current input buffer and mode as the
    /// restorable state and the initial search query, then previews the default
    /// selection. Prompts are interleaved only when the input is in agent mode.
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
        let query = original_buffer.clone();
        let original_config = self.input_mode.as_ref(ctx).input_config();
        // Shell mode shows commands only; agent mode interleaves prompts too.
        let include_prompts = !input_mode_policy::is_shell_mode(self.input_mode.as_ref(ctx));
        let session_id = (self.session_id_resolver)(ctx);
        self.preview_text = None;
        self.state = TuiHistoryMenuState::Open {
            list: TuiInlineMenuListState::default(),
            original_buffer,
            original_config,
            include_prompts,
            session_id,
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
        let original_buffer = match &self.state {
            TuiHistoryMenuState::Open {
                original_buffer, ..
            } => original_buffer.clone(),
            TuiHistoryMenuState::Closed => return,
        };
        self.close(ctx);
        self.set_input_text(&original_buffer, ctx);
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
        ctx.emit(TuiPromptHistoryMenuEvent::Updated);
    }

    /// Moves selection toward newer items and previews the highlighted one.
    /// Moving down past the newest row, or from an empty list, closes the menu
    /// and restores the buffer.
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
        ctx.emit(TuiPromptHistoryMenuEvent::Updated);
    }

    /// Accepts the current selection, closing the menu and returning how it
    /// should be applied: a highlighted command is executed, a prompt submitted.
    /// With an empty or filtered-to-nothing list there is nothing selected, so
    /// Enter is a no-op — the menu stays open and returns `None`.
    pub(crate) fn accept_selected(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Option<TuiHistoryAcceptance> {
        if !self.is_open(ctx) {
            return None;
        }
        let selected = match &self.state {
            TuiHistoryMenuState::Open { list, .. } => list.selected_row().map(|row| {
                let text = row.text.clone();
                match row.kind {
                    TuiUpArrowHistoryKind::Command => TuiHistoryAcceptance::Command(text),
                    TuiUpArrowHistoryKind::Prompt => TuiHistoryAcceptance::Prompt(text),
                }
            }),
            TuiHistoryMenuState::Closed => None,
        }?;
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
                .map(|row| TuiInlineMenuRow {
                    title: single_line_menu_title(&row.text),
                    description: None,
                    state_suffix: None,
                    is_selectable: true,
                    style: TuiInlineMenuRowStyle::Default,
                    // Commands carry the green `!` shell affordance; prompts don't.
                    shell_command_affordance: matches!(
                        row.kind,
                        TuiUpArrowHistoryKind::Command
                    ),
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

    /// Closes the menu without touching the input buffer, restoring the input
    /// mode captured at open so a command/prompt preview never leaks.
    fn close(&mut self, ctx: &mut ModelContext<Self>) {
        if let TuiHistoryMenuState::Open {
            original_config, ..
        } = &self.state
        {
            let original_config = *original_config;
            self.restore_input_config(original_config, ctx);
            self.state = TuiHistoryMenuState::Closed;
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
        let (query, include_prompts, session_id, previous, previous_index) = match &self.state {
            TuiHistoryMenuState::Open {
                list,
                query,
                include_prompts,
                session_id,
                ..
            } => (
                query.clone(),
                *include_prompts,
                *session_id,
                list.selected_row()
                    .map(|row| (row.text.clone(), row.kind)),
                list.selected_index(),
            ),
            TuiHistoryMenuState::Closed => return,
        };
        let trimmed_query = query.trim();
        let rows: Vec<TuiHistoryRow> = up_arrow_history_for_terminal_view(
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
        .map(|TuiUpArrowHistoryEntry { text, kind }| TuiHistoryRow { text, kind })
        .collect();
        let preferred_index = reconciled_selection_index(&rows, previous.as_ref(), previous_index);
        let TuiHistoryMenuState::Open { list, .. } = &mut self.state else {
            return;
        };
        list.replace_rows(rows, false, preferred_index, MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiPromptHistoryMenuEvent::Updated);
    }

    /// Writes the highlighted item into the input as an undo-agnostic preview and
    /// switches the input mode to match its kind (shell for a command, agent for
    /// a prompt) so the `!` affordance previews what acceptance will do.
    fn preview_selection(&mut self, ctx: &mut ModelContext<Self>) {
        let selected = match &self.state {
            TuiHistoryMenuState::Open { list, .. } => {
                list.selected_row().map(|row| (row.text.clone(), row.kind))
            }
            TuiHistoryMenuState::Closed => None,
        };
        let Some((text, kind)) = selected else {
            return;
        };
        self.apply_preview_mode(kind, ctx);
        self.preview_text = Some(text.clone());
        self.set_input_text(&text, ctx);
    }

    /// Locks the shared input mode to match the previewed item's kind.
    fn apply_preview_mode(&self, kind: TuiUpArrowHistoryKind, ctx: &mut ModelContext<Self>) {
        let (config, source) = match kind {
            TuiUpArrowHistoryKind::Command => {
                (SHELL_LOCKED_CONFIG, InputTypeAutoDetectionSource::ShellPrefix)
            }
            TuiUpArrowHistoryKind::Prompt => {
                (AI_LOCKED_CONFIG, InputTypeAutoDetectionSource::ManualToggle)
            }
        };
        if self.input_mode.as_ref(ctx).input_config() == config {
            return;
        }
        self.input_mode.clone().update(ctx, |input_mode, ctx| {
            input_mode.set_input_config(config, false, Some(source), ctx);
        });
    }

    /// Restores the input mode captured at open, if it has since changed.
    fn restore_input_config(&self, config: InputConfig, ctx: &mut ModelContext<Self>) {
        if self.input_mode.as_ref(ctx).input_config() == config {
            return;
        }
        let is_buffer_empty = input_text(&self.input_editor, ctx).is_empty();
        self.input_mode.clone().update(ctx, |input_mode, ctx| {
            input_mode.set_input_config(config, is_buffer_empty, None, ctx);
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

/// Preserves selection by item identity (text + kind), falling back to the
/// nearest previous index and finally to the last (most-recent) row.
fn reconciled_selection_index(
    rows: &[TuiHistoryRow],
    previous: Option<&(String, TuiUpArrowHistoryKind)>,
    previous_index: Option<usize>,
) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let last = rows.len() - 1;
    if let Some((text, kind)) = previous
        && let Some(index) = rows
            .iter()
            .position(|row| row.text == *text && row.kind == *kind)
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
