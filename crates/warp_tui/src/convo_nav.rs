//! Conversation navigation prototypes — three layout variants selectable via
//! the `WARP_TUI_CONVO_NAV` environment variable.
//!
//! When the variable is set, pressing ← at position 0 of the empty input opens
//! the prototype navigator instead of the built-in inline conversation menu,
//! so the variants can be compared against it.
//!
//! # Variants
//! - `modal` — centred bordered overlay with a preview pane; session shows through
//! - `page` — the conversation list fills the transcript slot; input stays
//! - `sidepane` / `sidebar` — narrow left column beside the transcript
//!
//! # Controls (when the nav is open)
//! - `↑` / `↓` — move selection
//! - `Enter` — open the selected conversation
//! - `Esc` — dismiss without switching
//!
//! Mock data: set `WARP_TUI_MOCK_CONVOS=1` to seed a dozen mock conversations
//! at startup for layout testing.

use warp::tui_export::{
    AIConversationId, AgentViewEntryOrigin, BlocklistAIHistoryModel, ConversationSelectionHandle,
    ConversationStatus,
};
use warpui_core::elements::tui::{
    Cell, Color, Modifier, TuiClipBounds, TuiConstrainedBox, TuiConstraint, TuiContainer,
    TuiElement, TuiEvent, TuiEventContext, TuiFlex, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiPresentationContext, TuiScreenPoint, TuiScreenPosition, TuiSize, TuiStyle,
    TuiText,
};
use warpui_core::keymap::EditableBinding;
use warpui_core::keymap::macros::*;
use warpui_core::{
    AppContext, Entity, EntityId, SingletonEntity, TuiView, TypedActionView, ViewContext,
};

use crate::keybindings::TUI_BINDING_GROUP;

// ─────────────────────────────────────────────────────────────────────────────
// Style selection
// ─────────────────────────────────────────────────────────────────────────────

/// Which navigation layout to use for the prototype comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConvoNavStyle {
    /// Centred modal overlay with a conversation preview pane on the right.
    Modal,
    /// Full transcript-slot conversation list (input bar stays visible).
    Page,
    /// Narrow left panel shown beside the live session transcript.
    SidePane,
}

/// Reads `WARP_TUI_CONVO_NAV` and returns the requested prototype style, or
/// `None` when the prototype is disabled (variable absent or unrecognized).
pub(crate) fn prototype_style() -> Option<ConvoNavStyle> {
    match std::env::var("WARP_TUI_CONVO_NAV")
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "modal" => Some(ConvoNavStyle::Modal),
        "page" => Some(ConvoNavStyle::Page),
        "sidepane" | "side_pane" | "side-pane" | "sidebar" => Some(ConvoNavStyle::SidePane),
        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Mock conversation seeding
// ─────────────────────────────────────────────────────────────────────────────

/// Seeds mock conversations when `WARP_TUI_MOCK_CONVOS` is set (dev-only
/// convenience for testing the navigator layouts without real prompts).
pub(crate) fn seed_mock_conversations_if_requested(
    terminal_surface_id: EntityId,
    ctx: &mut AppContext,
) {
    if std::env::var("WARP_TUI_MOCK_CONVOS")
        .unwrap_or_default()
        .is_empty()
    {
        return;
    }

    const MOCK_TITLES: &[&str] = &[
        "New coding session",
        "Journaling prompt game spec",
        "New conversation",
        "PR review comments handling",
        "New coding task",
        "Figma to TUI conversion",
        "Pomodoro timer app",
        "Warp server setup",
        "ASCII speed animation",
        "Build ASCII art editor with animations",
        "Untitled",
        "Create product and tech spec for conversation nav",
    ];

    let history_handle = BlocklistAIHistoryModel::handle(ctx);
    for title in MOCK_TITLES {
        let id = history_handle.update(ctx, |history, ctx| {
            history.start_new_conversation(terminal_surface_id, false, false, false, ctx)
        });
        history_handle.update(ctx, |history, _| {
            if let Some(conv) = history.conversation_mut(&id) {
                conv.set_fallback_display_title(title.to_string());
            }
        });
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared prototype elements
// ─────────────────────────────────────────────────────────────────────────────

/// A one-column divider that paints `│` down the full height it is given.
pub(crate) struct VerticalDivider {
    pub(crate) style: TuiStyle,
    size: Option<TuiSize>,
}

impl VerticalDivider {
    pub(crate) fn new(style: TuiStyle) -> Self {
        Self { style, size: None }
    }
}

impl TuiElement for VerticalDivider {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        // One column wide; fill the offered height.
        let size = constraint.clamp(TuiSize::new(1, constraint.max.height));
        self.size = Some(size);
        size
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        _ctx: &mut TuiPaintContext,
    ) {
        let Some(size) = self.size else {
            return;
        };
        for y in 0..size.height {
            if let Some(cell) = surface.cell_mut(origin.offset(0, i32::from(y))) {
                cell.set_symbol("│").set_style(self.style);
            }
        }
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }
}

/// Resets every cell in its render area to `Cell::default()` (space + no color)
/// before delegating to its child. Gives the modal box a solid background
/// without clearing the centering spacers (the session shows through those).
struct ClearingBox {
    child: Box<dyn TuiElement>,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl ClearingBox {
    fn new(child: Box<dyn TuiElement>) -> Self {
        Self {
            child,
            size: None,
            origin: None,
        }
    }
}

impl TuiElement for ClearingBox {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let size = self.child.layout(constraint, ctx, app);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut TuiLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.origin = Some(ctx.scene_point(origin));
        if let Some(size) = self.size {
            for y in 0..size.height {
                for x in 0..size.width {
                    if let Some(cell) =
                        surface.cell_mut(origin.offset(i32::from(x), i32::from(y)))
                    {
                        *cell = Cell::default();
                    }
                }
            }
            // Register the cleared box as a hit rect so mouse events over the
            // modal don't fall through to the session behind it.
            if let Some(bounds) = self.bounds() {
                ctx.scene.record_hit_rect(bounds);
            }
        }
        self.child.render(origin, surface, ctx);
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        event_ctx: &mut TuiEventContext<'_>,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, event_ctx, app)
    }
}

/// Renders `background` at full size then `overlay` on top in an overlay scene
/// layer. Used for the Modal navigator so the live session remains visible
/// behind the centred dialog box.
pub(crate) struct ModalOverlay {
    pub(crate) background: Box<dyn TuiElement>,
    pub(crate) overlay: Box<dyn TuiElement>,
}

impl TuiElement for ModalOverlay {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let size = self.background.layout(constraint, ctx, app);
        self.overlay.layout(constraint, ctx, app);
        size
    }

    fn after_layout(&mut self, ctx: &mut TuiLayoutContext, app: &AppContext) {
        self.background.after_layout(ctx, app);
        self.overlay.after_layout(ctx, app);
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.background.render(origin, surface, ctx);
        // The overlay renders in its own overlay scene layer so its hit rects
        // and cursor take priority over the session behind it. The modal box
        // clears only its own cells (`ClearingBox`), so the session shows
        // through the centering spacers.
        ctx.with_overlay_layer(TuiClipBounds::None, |ctx| {
            self.overlay.render(origin, surface, ctx);
        });
    }

    fn size(&self) -> Option<TuiSize> {
        self.background.size()
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.background.origin()
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.background.present(ctx);
        self.overlay.present(ctx);
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        event_ctx: &mut TuiEventContext<'_>,
        app: &AppContext,
    ) -> bool {
        // Overlay gets events first; background handles anything it ignores.
        self.overlay.dispatch_event(event, event_ctx, app)
            || self.background.dispatch_event(event, event_ctx, app)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Keybindings
// ─────────────────────────────────────────────────────────────────────────────

/// Registers the conversation nav view's keybindings. Called from
/// `keybindings::init` at TUI startup.
pub(crate) fn init(app: &mut AppContext) {
    app.register_editable_bindings([
        EditableBinding::new(
            "tui:convo_nav:up",
            "Move selection up in conversation list",
            ConvoNavAction::Up,
        )
        .with_context_predicate(id!(ConvoNavView::ui_name()))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("up"),
        EditableBinding::new(
            "tui:convo_nav:down",
            "Move selection down in conversation list",
            ConvoNavAction::Down,
        )
        .with_context_predicate(id!(ConvoNavView::ui_name()))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("down"),
        EditableBinding::new(
            "tui:convo_nav:select",
            "Open the selected conversation",
            ConvoNavAction::Select,
        )
        .with_context_predicate(id!(ConvoNavView::ui_name()))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("enter"),
        EditableBinding::new(
            "tui:convo_nav:close",
            "Close the conversation navigator",
            ConvoNavAction::Close,
        )
        .with_context_predicate(id!(ConvoNavView::ui_name()))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("escape"),
    ]);
}

// ─────────────────────────────────────────────────────────────────────────────
// Events and actions
// ─────────────────────────────────────────────────────────────────────────────

/// Events emitted by [`ConvoNavView`].
#[derive(Debug, Clone)]
pub(crate) enum ConvoNavEvent {
    /// The user chose a conversation to switch to. The ID is carried for
    /// subscribers that want to observe which conversation was opened; the
    /// switch itself is performed by [`ConvoNavView`] before this fires.
    #[allow(dead_code)]
    Selected(AIConversationId),
    /// The user dismissed the navigator without switching.
    Closed,
}

/// Typed actions handled by [`ConvoNavView`].
#[derive(Debug, Clone)]
pub(crate) enum ConvoNavAction {
    /// Move the selection up one entry.
    Up,
    /// Move the selection down one entry.
    Down,
    /// Open the currently-selected conversation.
    Select,
    /// Dismiss the navigator without switching.
    Close,
}

// ─────────────────────────────────────────────────────────────────────────────
// A lightweight summary of one conversation, built at render time
// ─────────────────────────────────────────────────────────────────────────────

struct ConvoEntry {
    id: AIConversationId,
    /// Display title — initial query or "Untitled" when unknown.
    title: String,
    /// Preview text shown in the modal's right pane.
    preview: String,
    /// Number of exchanges; kept for future stats display.
    #[allow(dead_code)]
    exchange_count: usize,
    /// Whether the conversation is actively running.
    is_active: bool,
    /// Diff stats `(added, modified, removed)` shown after the title.
    /// Mocked deterministically from the title for the prototype.
    diff: Option<(u32, u32, u32)>,
    /// Status glyph and color shown before the title:
    /// ■ blocked (cream) · ● working (cream) · ✓ done (green) · × failed (red).
    status_glyph: (&'static str, Color),
    /// Relative age label (e.g. "2w ago"), mocked from the title.
    age: String,
}

/// Pale cream color used for the ■/● status glyphs.
const STATUS_CREAM: Color = Color::Rgb(0xF5, 0xF5, 0xC0);
/// Soft green for the ✓ done glyph.
const STATUS_GREEN: Color = Color::Rgb(0xA8, 0xE8, 0x6B);
/// Salmon red for the × failed glyph.
const STATUS_RED: Color = Color::Rgb(0xF0, 0x80, 0x78);

/// FNV-1a hash used by the deterministic mock generators below.
fn title_hash(title: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in title.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// The status glyph for a real conversation status.
fn status_glyph_for(status: &ConversationStatus) -> (&'static str, Color) {
    match status {
        ConversationStatus::Blocked { .. } => ("■", STATUS_CREAM),
        ConversationStatus::InProgress
        | ConversationStatus::TransientError
        | ConversationStatus::WaitingForEvents => ("●", STATUS_CREAM),
        ConversationStatus::Success => ("✓", STATUS_GREEN),
        ConversationStatus::Error | ConversationStatus::Cancelled => ("×", STATUS_RED),
    }
}

/// Deterministic mock status glyph from a title (fresh mock conversations all
/// share InProgress, which would render a uniform column).
fn mock_status_glyph(title: &str) -> (&'static str, Color) {
    match (title_hash(title) >> 16) % 4 {
        0 => ("■", STATUS_CREAM),
        1 => ("●", STATUS_CREAM),
        2 => ("✓", STATUS_GREEN),
        _ => ("×", STATUS_RED),
    }
}

/// Deterministic mock diff stats from a title. Roughly one in three
/// conversations reports no diff.
fn mock_diff_stats(title: &str) -> Option<(u32, u32, u32)> {
    let hash = title_hash(title);
    if hash % 3 == 0 {
        return None;
    }
    let added = (hash >> 8) % 700;
    let modified = (hash >> 24) % 40;
    let removed = (hash >> 40) % 120;
    Some((added as u32, modified as u32, removed as u32))
}

/// Deterministic mock relative-age label from a title.
fn mock_age(title: &str) -> String {
    match (title_hash(title) >> 32) % 8 {
        0 => "5h ago".to_string(),
        1 => "1d ago".to_string(),
        2 => "2d ago".to_string(),
        3 => "6d ago".to_string(),
        4 => "1w ago".to_string(),
        5 => "2w ago".to_string(),
        6 => "3w ago".to_string(),
        _ => "4w ago".to_string(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// View
// ─────────────────────────────────────────────────────────────────────────────

/// The conversation navigation view. Rendered by `TuiTerminalSessionView` as a
/// modal overlay, a transcript-slot page, or a left side panel depending on
/// [`ConvoNavStyle`].
pub(crate) struct ConvoNavView {
    terminal_surface_id: EntityId,
    conversation_selection: ConversationSelectionHandle,
    selected_index: usize,
    style: ConvoNavStyle,
}

impl ConvoNavView {
    pub(crate) fn new(
        terminal_surface_id: EntityId,
        conversation_selection: ConversationSelectionHandle,
        style: ConvoNavStyle,
        _ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self {
            terminal_surface_id,
            conversation_selection,
            selected_index: 0,
            style,
        }
    }

    /// Resets the selection index when the nav is (re-)opened so the cursor
    /// starts at the top of the list.
    pub(crate) fn reset_selection(&mut self) {
        self.selected_index = 0;
    }

    /// Returns the conversation entry at the current selection, if any.
    fn selected_id(&self, entries: &[ConvoEntry]) -> Option<AIConversationId> {
        entries.get(self.selected_index).map(|e| e.id)
    }

    /// Builds the conversation entry list from the history model.
    fn entries(terminal_surface_id: EntityId, ctx: &AppContext) -> Vec<ConvoEntry> {
        BlocklistAIHistoryModel::as_ref(ctx)
            .all_live_conversations_for_terminal_surface(terminal_surface_id)
            .filter(|conv| !conv.should_exclude_from_navigation())
            .map(|conv| {
                let title = conv
                    .title()
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| "Untitled".to_string());
                let preview = conv
                    .initial_query()
                    .filter(|q| !q.is_empty())
                    .unwrap_or_else(|| title.clone());
                let exchange_count = conv.exchange_count();
                let is_active = matches!(conv.status(), ConversationStatus::InProgress);
                let diff = mock_diff_stats(&title);
                // Empty (mock) conversations all share the InProgress status;
                // use the title-derived mock glyph for variety. Real
                // conversations with exchanges show their actual status.
                let status_glyph = if exchange_count == 0 {
                    mock_status_glyph(&title)
                } else {
                    status_glyph_for(conv.status())
                };
                let age = mock_age(&title);
                ConvoEntry {
                    id: conv.id(),
                    title,
                    preview,
                    exchange_count,
                    is_active,
                    diff,
                    status_glyph,
                    age,
                }
            })
            .collect()
    }

    /// Builds the colored `+a -r` diff-stat cells for a row (green added,
    /// red removed). Returns an empty row when the entry has no diff.
    fn diff_cells(diff: Option<(u32, u32, u32)>) -> TuiFlex {
        let mut row = TuiFlex::row();
        let Some((added, _modified, removed)) = diff else {
            return row;
        };
        row = row
            .child(
                TuiText::new(format!("+{added}"))
                    .with_style(TuiStyle::default().fg(Color::Green))
                    .truncate()
                    .finish(),
            )
            .child(
                TuiText::new(format!(" -{removed}"))
                    .with_style(TuiStyle::default().fg(Color::Red))
                    .truncate()
                    .finish(),
            );
        row
    }

    /// Renders the scrollable conversation list entries.
    fn render_list(entries: &[ConvoEntry], selected_index: usize, max_title_len: usize) -> TuiFlex {
        let mut column = TuiFlex::column();

        if entries.is_empty() {
            column = column.child(
                TuiText::new("No conversations yet.")
                    .with_style(TuiStyle::default().add_modifier(Modifier::DIM))
                    .truncate()
                    .finish(),
            );
        }

        for (i, entry) in entries.iter().enumerate() {
            let is_selected = i == selected_index;

            // Truncate title to available width.
            let title = if entry.title.chars().count() > max_title_len {
                let t: String = entry
                    .title
                    .chars()
                    .take(max_title_len.saturating_sub(1))
                    .collect();
                format!("{t}…")
            } else {
                entry.title.clone()
            };

            let style = if is_selected {
                TuiStyle::default().add_modifier(Modifier::REVERSED)
            } else {
                TuiStyle::default()
            };

            // Status glyph, then title, then diff stats + age at the right edge.
            let (glyph, glyph_color) = entry.status_glyph;
            let row = TuiFlex::row()
                .child(
                    TuiText::new(format!("{glyph} "))
                        .with_style(TuiStyle::default().fg(glyph_color))
                        .truncate()
                        .finish(),
                )
                .child(TuiText::new(title).with_style(style).truncate().finish())
                .flex_child(TuiFlex::row().finish())
                .child(Self::diff_cells(entry.diff).finish())
                .child(
                    TuiText::new(format!("  {}", entry.age))
                        .with_style(TuiStyle::default().fg(Color::DarkGray))
                        .truncate()
                        .finish(),
                );
            column = column.child(TuiConstrainedBox::new(row.finish()).with_max_rows(1).finish());
        }

        column
    }

    /// Renders the cyan `>` + gray "type to search" prompt row shared by the
    /// modal and sidepane layouts.
    fn render_search_hint() -> Box<dyn TuiElement> {
        TuiConstrainedBox::new(
            TuiFlex::row()
                .child(
                    TuiText::new("> ")
                        .with_style(TuiStyle::default().fg(Color::Cyan))
                        .truncate()
                        .finish(),
                )
                .child(
                    TuiText::new("type to search")
                        .with_style(TuiStyle::default().fg(Color::DarkGray))
                        .truncate()
                        .finish(),
                )
                .finish(),
        )
        .with_max_rows(1)
        .finish()
    }

    /// Renders the hint bar shown at the bottom of the navigator.
    fn render_hint() -> TuiConstrainedBox {
        TuiConstrainedBox::new(
            TuiText::new("↑↓ navigate · enter select · esc close")
                .with_style(TuiStyle::default().add_modifier(Modifier::DIM))
                .truncate()
                .finish(),
        )
        .with_max_rows(1)
    }

    /// Renders the preview pane for the modal style.
    fn render_preview(entries: &[ConvoEntry], selected_index: usize) -> TuiContainer {
        let dim = TuiStyle::default().add_modifier(Modifier::DIM);
        let mut column = TuiFlex::column()
            .child(
                TuiText::new("Conversation Preview")
                    .with_style(TuiStyle::default().add_modifier(Modifier::BOLD))
                    .truncate()
                    .finish(),
            )
            .child(TuiText::new("").finish());

        if let Some(entry) = entries.get(selected_index) {
            for line in entry.preview.lines().take(12) {
                column = column.child(TuiText::new(line.to_string()).truncate().finish());
            }
            if entry.preview.is_empty() {
                column = column.child(
                    TuiText::new("(no preview)")
                        .with_style(dim)
                        .truncate()
                        .finish(),
                );
            }
        } else {
            column = column.child(
                TuiText::new("Select a conversation to preview.")
                    .with_style(dim)
                    .truncate()
                    .finish(),
            );
        }

        TuiContainer::new(column.finish()).with_padding(1)
    }

    /// Full render for the Modal style: centred bordered box with list + preview.
    fn render_modal(&self, entries: &[ConvoEntry]) -> Box<dyn TuiElement> {
        let border_style = TuiStyle::default().fg(Color::DarkGray);

        let header = TuiConstrainedBox::new(
            TuiText::new("Switch Conversation")
                .with_style(TuiStyle::default().add_modifier(Modifier::BOLD))
                .truncate()
                .finish(),
        )
        .with_max_rows(1)
        .finish();
        let search_hint = Self::render_search_hint();

        let list = Self::render_list(entries, self.selected_index, 38);
        let preview = Self::render_preview(entries, self.selected_index);

        // Left panel (list) + divider + right panel (preview).
        let panels = TuiFlex::row()
            .child(
                TuiConstrainedBox::new(list.finish())
                    .with_max_cols(42)
                    .finish(),
            )
            .child(VerticalDivider::new(TuiStyle::default().add_modifier(Modifier::DIM)).finish())
            .child(
                TuiConstrainedBox::new(preview.finish())
                    .with_max_cols(50)
                    .finish(),
            )
            .finish();

        let inner = TuiFlex::column()
            .child(header)
            .child(search_hint)
            .child(TuiText::new("").finish())
            .flex_child(panels)
            .child(Self::render_hint().finish());

        // Wrap in ClearingBox so the modal box resets its cells before drawing,
        // then constrain and center. The centering spacers are not cleared so
        // the session transcript shows through behind the modal.
        let modal = TuiConstrainedBox::new(
            ClearingBox::new(
                TuiContainer::new(inner.finish())
                    .with_border_style(border_style)
                    .with_padding(1)
                    .finish(),
            )
            .finish(),
        )
        .with_max_rows(20)
        .with_max_cols(96)
        .finish();

        // Center the modal vertically and horizontally with flex spacers.
        TuiFlex::column()
            .flex_child(TuiFlex::column().finish())
            .child(
                TuiFlex::row()
                    .flex_child(TuiFlex::row().finish())
                    .child(modal)
                    .flex_child(TuiFlex::row().finish())
                    .finish(),
            )
            .flex_child(TuiFlex::column().finish())
            .finish()
    }

    /// Full render for the Page style: fills the transcript slot without a
    /// border. The session view keeps the input bar and footer visible below.
    fn render_page(&self, entries: &[ConvoEntry]) -> Box<dyn TuiElement> {
        let header = TuiConstrainedBox::new(
            TuiText::new("Switch Conversation")
                .with_style(TuiStyle::default().add_modifier(Modifier::BOLD))
                .truncate()
                .finish(),
        )
        .with_max_rows(1)
        .finish();

        let list = Self::render_list(entries, self.selected_index, 60);

        TuiFlex::column()
            .child(header)
            .child(Self::render_search_hint())
            .child(TuiText::new("").finish())
            .flex_child(list.finish())
            .child(Self::render_hint().finish())
            .finish()
    }

    /// Renders the narrow borderless sidebar panel used in the SidePane style.
    /// The session view constrains the width and places this beside the
    /// transcript with a [`VerticalDivider`].
    fn render_sidepane(&self, entries: &[ConvoEntry]) -> Box<dyn TuiElement> {
        let mut column = TuiFlex::column()
            .child(TuiText::new("conversations").truncate().finish())
            .child(Self::render_search_hint())
            .child(TuiText::new("").finish());

        for (i, entry) in entries.iter().enumerate() {
            let is_selected = i == self.selected_index;
            // Keep titles short; the column is narrow, and the glyph + diff
            // stats need room.
            let title: String = if entry.title.chars().count() > 16 {
                let t: String = entry.title.chars().take(15).collect();
                format!("{t}…")
            } else {
                entry.title.clone()
            };

            let style = if is_selected {
                TuiStyle::default().add_modifier(Modifier::REVERSED)
            } else if entry.is_active {
                TuiStyle::default().add_modifier(Modifier::BOLD)
            } else {
                TuiStyle::default()
            };

            // Status glyph, then title, then diff stats + age right-aligned.
            let (glyph, glyph_color) = entry.status_glyph;
            let row = TuiFlex::row()
                .child(
                    TuiText::new(format!("{glyph} "))
                        .with_style(TuiStyle::default().fg(glyph_color))
                        .truncate()
                        .finish(),
                )
                .child(TuiText::new(title).with_style(style).truncate().finish())
                .flex_child(TuiFlex::row().finish())
                .child(Self::diff_cells(entry.diff).finish())
                .child(
                    TuiText::new(format!("  {}", entry.age))
                        .with_style(TuiStyle::default().fg(Color::DarkGray))
                        .truncate()
                        .finish(),
                );
            column = column.child(TuiConstrainedBox::new(row.finish()).with_max_rows(1).finish());
        }

        // Hint line at the bottom.
        TuiFlex::column()
            .flex_child(column.finish())
            .child(
                TuiText::new("↑↓ · enter · esc")
                    .with_style(TuiStyle::default().add_modifier(Modifier::DIM))
                    .truncate()
                    .finish(),
            )
            .finish()
    }
}

impl Entity for ConvoNavView {
    type Event = ConvoNavEvent;
}

impl TuiView for ConvoNavView {
    fn ui_name() -> &'static str {
        "ConvoNavView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        let entries = Self::entries(self.terminal_surface_id, ctx);
        match self.style {
            ConvoNavStyle::Modal => self.render_modal(&entries),
            ConvoNavStyle::Page => self.render_page(&entries),
            ConvoNavStyle::SidePane => self.render_sidepane(&entries),
        }
    }

    fn keymap_context(&self, _ctx: &AppContext) -> warpui_core::keymap::Context {
        let mut context = warpui_core::keymap::Context::default();
        context.set.insert("ConvoNavView");
        context
    }
}

impl TypedActionView for ConvoNavView {
    type Action = ConvoNavAction;

    fn handle_action(&mut self, action: &ConvoNavAction, ctx: &mut ViewContext<Self>) {
        let entries = Self::entries(self.terminal_surface_id, ctx);
        match action {
            ConvoNavAction::Up => {
                if !entries.is_empty() {
                    self.selected_index = self
                        .selected_index
                        .checked_sub(1)
                        .unwrap_or(entries.len() - 1);
                }
                ctx.notify();
            }
            ConvoNavAction::Down => {
                if !entries.is_empty() {
                    self.selected_index = (self.selected_index + 1) % entries.len();
                }
                ctx.notify();
            }
            ConvoNavAction::Select => {
                if let Some(id) = self.selected_id(&entries) {
                    self.conversation_selection.update(ctx, |sel, ctx| {
                        sel.select_existing_conversation(id, AgentViewEntryOrigin::Tui, ctx);
                    });
                    ctx.emit(ConvoNavEvent::Selected(id));
                }
            }
            ConvoNavAction::Close => {
                ctx.emit(ConvoNavEvent::Closed);
            }
        }
    }
}
