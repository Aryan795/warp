//! Command x-ray for the agent permission prompt's expanded command body.
//!
//! This module is the agent-command side's own implementation of the three pieces command x-ray
//! needs: the hover state machine, the pointer-to-offset hit test, and the overlay anchor. The
//! terminal input keeps its own, untouched implementations of the same three pieces
//! (`app/src/editor/view/element.rs`, `app/src/terminal/input.rs`, and
//! `app/src/terminal/input/common.rs`); nothing here is shared with them.
//!
//! Only the data plane is shared: `warp_completer`'s `describe` produces the description, and
//! `crate::terminal::input::common::render_command_token_description` renders it.
//!
//! Wherever a rule is copied from the terminal input's implementation, the comment names the
//! original. Those are the points where the two copies must be kept in step.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use instant::Instant;
use parking_lot::Mutex;
use pathfinder_geometry::vector::Vector2F;
use string_offset::ByteOffset;
use warpui::elements::{Element, Point, SelectableElement, ZIndex};
use warpui::event::DispatchedEvent;
use warpui::{
    AfterLayoutContext, AppContext, Event, EventContext, LayoutContext, PaintContext,
    SizeConstraint, TaskId,
};

use super::requested_command::RequestedCommandViewAction;

/// How long the pointer must rest on a token before its description is shown.
/// Copied from `COMMAND_X_RAY_HOVER_DELAY` (`app/src/editor/view/element.rs`); both hosts must
/// use the same delay for x-ray to feel like one feature.
const COMMAND_X_RAY_HOVER_DELAY: Duration = Duration::from_millis(500);

/// How far the pointer may drift and still count as resting in the same place.
/// Copied from `COMMAND_X_RAY_HOVER_THRESHOLD_PX` (`app/src/editor/view/element.rs`).
const COMMAND_X_RAY_HOVER_THRESHOLD_PX: f32 = 3.;

/// Pointer-tracking state for one hover gesture over the command body.
///
/// Mirrors `CommandXRayMouseState` (`app/src/editor/view/element.rs`) field for field, because
/// the open/close rules below are the same rules.
#[derive(Clone, Debug)]
struct CommandXRayPointerState {
    /// The point at which the hover originated, in pixels.
    hover_point: Vector2F,

    /// Whether the x-ray tooltip is visible.
    visible: bool,

    /// The instant at which the x-ray should show.
    hover_at: Instant,

    /// The timer id for the x-ray, in case we need to cancel it.
    timer_id: TaskId,

    /// Whether the user has dismissed the tooltip through some action (editing the command or
    /// toggling x-ray off from the keyboard).
    user_dismissed: bool,
}

/// State shared between [`CommandXRayHoverElement`], which owns the pointer, and
/// `RequestedCommandView`, which owns the description and the code editor.
///
/// The element decides *when* to ask for a description; the view resolves *what* is under the
/// pointer and keeps the two offset fields below current.
#[derive(Debug, Default)]
pub(super) struct CommandXRayHoverState {
    pointer: Option<CommandXRayPointerState>,

    /// The character index in the command text under the pointer, as resolved by the code
    /// editor's own hit test (`RichTextElement::position_to_location`, surfaced to the view as
    /// `CodeEditorEvent::MouseHovered`). `None` while the pointer is not over command text.
    hovered_char_index: Option<usize>,

    /// The character range of the token the visible tooltip describes, or `None` when no tooltip
    /// is showing.
    described_token_range: Option<Range<usize>>,
}

pub(super) type CommandXRayHoverStateHandle = Arc<Mutex<CommandXRayHoverState>>;

impl CommandXRayHoverState {
    /// Records the character index the code editor resolved for the current pointer position.
    pub(super) fn set_hovered_char_index(&mut self, hovered_char_index: Option<usize>) {
        self.hovered_char_index = hovered_char_index;
    }

    /// The character index currently under the pointer, if any.
    pub(super) fn hovered_char_index(&self) -> Option<usize> {
        self.hovered_char_index
    }

    /// Records the token range the visible tooltip describes.
    pub(super) fn set_described_token_range(&mut self, token_range: Option<Range<usize>>) {
        if token_range.is_none()
            && let Some(pointer) = &mut self.pointer
        {
            pointer.visible = false;
        }
        self.described_token_range = token_range;
    }

    /// Marks the tooltip as dismissed by the user, so it stays dismissed until the pointer moves
    /// somewhere else. Mirrors `EditorView::clear_command_x_ray` (`app/src/editor/view/mod.rs`),
    /// which sets the same flag on the input's pointer state.
    pub(super) fn mark_user_dismissed(&mut self) {
        if let Some(pointer) = &mut self.pointer {
            pointer.user_dismissed = true;
            pointer.visible = false;
        }
        self.described_token_range = None;
    }

    /// Whether the pointer is still inside the bounds of the token the tooltip describes.
    ///
    /// Mirrors `EditorElement::is_position_within_x_ray_token_bounds`
    /// (`app/src/editor/view/element.rs`): the input compares the offset under the pointer
    /// against the described token's span, and so do we - in characters, because that is the
    /// unit the code editor's hit test reports.
    fn is_pointer_within_described_token(&self) -> bool {
        match (self.hovered_char_index, &self.described_token_range) {
            (Some(hovered), Some(token)) => token.start <= hovered && hovered < token.end,
            _ => false,
        }
    }
}

/// Wraps the permission prompt's command body and runs the x-ray hover state machine over it.
///
/// The element forwards every event to the code editor first, so the editor's own hit test has
/// already resolved the hovered location by the time the state machine below runs for that same
/// pointer position.
pub(super) struct CommandXRayHoverElement {
    child: Box<dyn Element>,
    state: CommandXRayHoverStateHandle,
    origin: Option<Point>,
    child_max_z_index: Option<ZIndex>,
}

impl CommandXRayHoverElement {
    pub(super) fn new(child: Box<dyn Element>, state: CommandXRayHoverStateHandle) -> Self {
        Self {
            child,
            state,
            origin: None,
            child_max_z_index: None,
        }
    }

    /// The hover state machine.
    ///
    /// This is the agent-command copy of `EditorElement::mouse_moved`
    /// (`app/src/editor/view/element.rs`), including its four cases and the reasoning behind
    /// them. A change to the input's rules has to be made here too.
    ///
    /// The hovered offset this reads is the one the code editor resolved for the *previous*
    /// mouse-move event, because the editor reports it through a dispatched action rather than
    /// synchronously. That only matters while the pointer is moving; the tooltip opens and
    /// closes on a pointer that has come to rest, by which point the offset has caught up.
    fn mouse_moved(&mut self, position: Vector2F, ctx: &mut EventContext) -> bool {
        let mut state = self.state.lock();
        let pointer = state.pointer.clone();

        if let Some(pointer) = &pointer
            && pointer.user_dismissed
        {
            return if (pointer.hover_point - position).length() < COMMAND_X_RAY_HOVER_THRESHOLD_PX {
                // Early exit if some user action has dismissed the tooltip and the mouse hasn't
                // moved.
                false
            } else {
                Self::reset_x_ray(&mut state, Some(position), ctx)
            };
        }

        if !self.is_pointer_over_command_body(position, ctx) {
            // The pointer is outside the command body, so clear any pending x-ray and exit early.
            return Self::reset_x_ray(&mut state, None, ctx);
        }

        let Some(pointer) = pointer else {
            // Case 4: no timer set, set a new one.
            return Self::reset_x_ray(&mut state, Some(position), ctx);
        };

        let within_last_mouse_move_radius =
            (pointer.hover_point - position).length() < COMMAND_X_RAY_HOVER_THRESHOLD_PX;
        let is_visible = pointer.visible;
        let timer_elapsed = Instant::now() >= pointer.hover_at;
        let timer_id = pointer.timer_id;
        let is_within_token_bounds = state.is_pointer_within_described_token();
        // Case 1: the tooltip is open. We only close it if the pointer is outside the described
        // token *and* has moved more than a radius away. The latter condition keeps the tooltip
        // open through small movements, even ones that leave the token.
        if is_visible && !is_within_token_bounds && !within_last_mouse_move_radius {
            return Self::reset_x_ray(&mut state, Some(position), ctx);
        }

        if !is_visible {
            // Case 2: the tooltip is not open yet. Open it once enough time has elapsed, the
            // pointer is still resting within the same radius, and it is over command text
            // rather than past the end of it.
            if timer_elapsed && within_last_mouse_move_radius && state.hovered_char_index.is_some()
            {
                ctx.dispatch_typed_action(RequestedCommandViewAction::ShowCommandXRayAtPointer);
                ctx.clear_notify_timer(timer_id);
                if let Some(pointer) = &mut state.pointer {
                    pointer.visible = true;
                }
                return true;
            } else if !within_last_mouse_move_radius {
                // Case 3: the pointer moved more than a radius away before the timer elapsed, so
                // restart from the new position.
                return Self::reset_x_ray(&mut state, Some(position), ctx);
            }
        }

        false
    }

    /// Resets the timer and (optional) position for triggering command x-ray.
    ///
    /// Mirrors `EditorElement::reset_x_ray` (`app/src/editor/view/element.rs`).
    fn reset_x_ray(
        state: &mut CommandXRayHoverState,
        new_position: Option<Vector2F>,
        ctx: &mut EventContext,
    ) -> bool {
        let mut updated = false;
        if let Some(pointer) = &state.pointer {
            if pointer.visible {
                ctx.dispatch_typed_action(RequestedCommandViewAction::HideCommandXRay);
                state.described_token_range = None;
                updated = true;
            }
            ctx.clear_notify_timer(pointer.timer_id);
        }

        state.pointer = new_position.map(|position| {
            let (timer_id, hover_at) = ctx.notify_after(COMMAND_X_RAY_HOVER_DELAY);
            CommandXRayPointerState {
                visible: false,
                hover_point: position,
                hover_at,
                timer_id,
                user_dismissed: false,
            }
        });

        updated
    }

    /// Whether the pointer is over the command body and not covered by anything drawn on top of
    /// it. The input's element reads this off its own paint rect
    /// (`paint.rect.contains_point(position)`); we ask the scene for ours.
    fn is_pointer_over_command_body(&self, position: Vector2F, ctx: &EventContext) -> bool {
        let (Some(origin), Some(size), Some(z_index)) =
            (self.origin, self.child.size(), self.child_max_z_index)
        else {
            return false;
        };

        let is_within_bounds = ctx
            .visible_rect(origin, size)
            .is_some_and(|bounds| bounds.contains_point(position));

        is_within_bounds && !ctx.is_covered(Point::from_vec2f(position, z_index))
    }
}

impl Element for CommandXRayHoverElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.child.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        self.child.paint(origin, ctx, app);
        self.child_max_z_index = Some(ctx.scene.max_active_z_index());
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        // The code editor must see the event first: its hit test resolves the hovered location
        // that the state machine reads back for this same pointer position.
        let handled = self.child.dispatch_event(event, ctx, app);

        if let Event::MouseMoved { position, .. } = event.raw_event() {
            // Deliberate divergence from `EditorElement::dispatch_event`
            // (`app/src/editor/view/element.rs`), which returns its `mouse_moved` result and so
            // claims a move that opened or closed the tooltip. The terminal input's editor can
            // afford that; this command body sits inside the block list, whose ancestors drive
            // their own hover states off the same event, so swallowing it here would regress
            // unrelated hover UI. The tooltip re-renders through the dispatched actions instead,
            // so nothing is lost by letting the move keep propagating.
            self.mouse_moved(*position, ctx);
        }

        handled
    }

    fn as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        self.child.as_selectable_element()
    }

    #[cfg(any(test, feature = "test-util"))]
    fn debug_text_content(&self) -> Option<String> {
        self.child.debug_text_content()
    }
}

/// Snaps a character index to the start of the token that contains it and returns that position
/// as a byte offset into `command_text`.
///
/// The terminal input snaps the same way with `EditorView::start_byte_offset_at_point`
/// (`app/src/editor/view/mod.rs`) before calling `describe`: hovering anywhere inside a token
/// must describe the whole token, and the hit test reports a caret position, which can land on
/// either side of the glyph under the pointer.
pub(super) fn token_start_byte_offset(command_text: &str, char_index: usize) -> ByteOffset {
    let byte_index = char_index_to_byte_index(command_text, char_index);
    let token_start = command_text[..byte_index]
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_whitespace())
        .map(|(index, character)| index + character.len_utf8())
        .unwrap_or(0);

    ByteOffset::from(token_start)
}

/// Converts a character index into `text` to a byte index, clamping past-the-end indices to the
/// end of the text.
pub(super) fn char_index_to_byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(byte_index, _)| byte_index)
        .unwrap_or(text.len())
}

/// Converts a byte index into `text` to a character index, clamping past-the-end indices and
/// snapping indices that land inside a multi-byte character.
pub(super) fn byte_index_to_char_index(text: &str, byte_index: usize) -> usize {
    let mut byte_index = byte_index.min(text.len());
    while !text.is_char_boundary(byte_index) {
        byte_index -= 1;
    }
    text[..byte_index].chars().count()
}

/// The character range covered by a described token, whose span `describe` reports in bytes.
pub(super) fn token_char_range(text: &str, token_span: Range<usize>) -> Range<usize> {
    byte_index_to_char_index(text, token_span.start)..byte_index_to_char_index(text, token_span.end)
}

#[cfg(test)]
#[path = "requested_command_x_ray_tests.rs"]
mod tests;
