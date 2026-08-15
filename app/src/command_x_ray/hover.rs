use std::sync::Arc;
use std::time::Duration;

use instant::Instant;
use parking_lot::Mutex;
use pathfinder_geometry::vector::Vector2F;
use warpui::{EventContext, TaskId};

/// How far the pointer may drift, in pixels, before a pending hover is restarted or an open
/// tooltip is allowed to close.
pub const COMMAND_X_RAY_HOVER_THRESHOLD_PX: f32 = 3.;

/// How long the pointer must rest before the tooltip opens.
pub const COMMAND_X_RAY_HOVER_DELAY: Duration = Duration::from_millis(500);

/// The timer that opens the tooltip once the pointer has rested for [`COMMAND_X_RAY_HOVER_DELAY`].
///
/// In the app this is always the element's [`EventContext`], which schedules a redraw. It is a
/// trait so the state machine can be exercised without a live presenter.
pub trait HoverTimer {
    /// Schedules a wake-up after `delay`, returning the timer's id and the instant it fires at.
    fn arm(&mut self, delay: Duration) -> (TaskId, Instant);
    /// Cancels a previously armed timer.
    fn cancel(&mut self, timer_id: TaskId);
    /// The current time on the same clock `arm` returns instants on.
    fn now(&self) -> Instant {
        Instant::now()
    }
}

impl HoverTimer for EventContext<'_> {
    fn arm(&mut self, delay: Duration) -> (TaskId, Instant) {
        self.notify_after(delay)
    }

    fn cancel(&mut self, timer_id: TaskId) {
        self.clear_notify_timer(timer_id);
    }
}

/// What the host's geometry says about the pointer position. Everything the state machine needs
/// to know about the text under the pointer, with no buffer or layout types in it.
pub struct HoverProbe {
    /// Whether the resolved position was clamped to the edge of the text, i.e. the pointer is
    /// past the end of a line rather than over a glyph.
    pub is_clamped: bool,
    /// Whether the resolved position falls inside the bounds of the token currently described.
    pub is_within_token: bool,
}

/// What the host should do in response to a pointer move.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum HoverOutcome {
    /// Nothing to do. The host should not treat the event as handled.
    Idle,
    /// Hide the tooltip.
    Hide,
    /// Show the tooltip for the position that was just probed.
    Show,
}

impl HoverOutcome {
    /// Whether the host should report the pointer event as handled.
    pub fn is_handled(&self) -> bool {
        !matches!(self, HoverOutcome::Idle)
    }
}

/// The pending or open hover tracked between pointer moves.
#[derive(Clone, Debug)]
struct HoverState {
    /// The point at which the hover originated, in pixels.
    hover_point: Vector2F,

    /// Whether the x-ray tooltip is visible.
    visible: bool,

    /// The instant at which the x-ray should show.
    hover_at: Instant,

    /// The timer id for the x-ray, in case we need to cancel it.
    timer_id: TaskId,

    /// Whether the user has dismissed the hover through some action (e.g. esc or cmd-i or typing
    /// or scrolling).
    user_dismissed: bool,
}

/// The command x-ray hover state machine, shared by every host that shows token descriptions.
///
/// It owns the hover delay, the movement threshold, whether the tooltip is open, whether the user
/// dismissed it, and the timer that opens it. It knows nothing about buffers, editor layout, or
/// the description itself: hosts feed it pointer positions plus a [`HoverProbe`] resolved with
/// their own geometry, and act on the returned [`HoverOutcome`].
///
/// The handle is cheap to clone and is shared between a host view and its element.
#[derive(Clone, Default)]
pub struct CommandXRayHover(Arc<Mutex<Option<HoverState>>>);

impl CommandXRayHover {
    /// Advances the state machine for a pointer move at `position`.
    ///
    /// `is_in_bounds` is whether the pointer is over the host's text area. `probe` resolves the
    /// text under the pointer, and is only called when it can affect the outcome, so hosts do not
    /// pay for a hit test on every move outside their bounds. Hosts that need the hit result to
    /// act on [`HoverOutcome::Show`] can capture it from inside `probe`.
    pub fn on_mouse_moved(
        &self,
        position: Vector2F,
        is_in_bounds: bool,
        probe: impl FnOnce() -> HoverProbe,
        timer: &mut impl HoverTimer,
    ) -> HoverOutcome {
        let mut guard = self.0.lock();

        if let Some(state) = &mut *guard
            && state.user_dismissed
        {
            return if within_threshold(state.hover_point, position) {
                // Early exit if some user action has caused the x-ray tooltip to be dismissed
                // and the mouse hasn't moved.
                HoverOutcome::Idle
            } else {
                Self::restart(&mut guard, Some(position), timer)
            };
        }

        if !is_in_bounds {
            // Mouse is outside of the host, so clear any pending x-ray and exit early.
            return Self::restart(&mut guard, None, timer);
        }

        let probe = probe();

        if let Some(state) = &mut *guard {
            let within_last_mouse_move_radius = within_threshold(state.hover_point, position);

            // Case 1: the command xray tooltip is open. We only want to close it if:
            //  - the cursor is not within the word boundary for the token being described
            //  - and the cursor is more than a radius away from the token
            // The latter condition ensures that we only close the tooltip when
            // there is a substantial mouse movement (if the cursor is only slightly
            // moved, even outside of the word boundary, we still want to keep it open).
            if state.visible && !probe.is_within_token && !within_last_mouse_move_radius {
                return Self::restart(&mut guard, Some(position), timer);
            } else if !state.visible {
                // Case 2: the command xray tooltip is not open yet. We only want to open it if
                //  - enough time has elapsed
                //  - the mouse is still within the same radius as it was before
                //  - the point we are considering isn't a clamped point (since we don't
                //    want to describe the last token if the cursor is actually well past the
                //    buffer text)
                let timer_elapsed = timer.now() >= state.hover_at;
                if timer_elapsed && within_last_mouse_move_radius && !probe.is_clamped {
                    timer.cancel(state.timer_id);
                    state.visible = true;
                    return HoverOutcome::Show;
                } else if !within_last_mouse_move_radius {
                    // Case 3: the command xray tooltip is not open yet. We should reset the
                    // state as long as the mouse has moved more than a radius away since the
                    // last tracked mouse position.
                    return Self::restart(&mut guard, Some(position), timer);
                }
            }
        } else {
            // Case 4: No timer set, set a new one.
            return Self::restart(&mut guard, Some(position), timer);
        }

        HoverOutcome::Idle
    }

    /// Marks the hover as dismissed by the user, so it stays closed until the pointer moves
    /// beyond the threshold.
    pub fn mark_user_dismissed(&self) {
        if let Some(state) = &mut *self.0.lock() {
            state.user_dismissed = true;
        }
    }

    /// Resets the timer and (optional) position for triggering command x-ray.
    fn restart(
        state: &mut Option<HoverState>,
        new_position: Option<Vector2F>,
        timer: &mut impl HoverTimer,
    ) -> HoverOutcome {
        let mut outcome = HoverOutcome::Idle;
        if let Some(state) = &mut *state {
            if state.visible {
                outcome = HoverOutcome::Hide;
            }
            timer.cancel(state.timer_id);
        }

        *state = new_position.map(|position| {
            let (timer_id, hover_at) = timer.arm(COMMAND_X_RAY_HOVER_DELAY);
            HoverState {
                visible: false,
                hover_point: position,
                hover_at,
                timer_id,
                user_dismissed: false,
            }
        });
        outcome
    }
}

fn within_threshold(from: Vector2F, to: Vector2F) -> bool {
    (from - to).length() < COMMAND_X_RAY_HOVER_THRESHOLD_PX
}

#[cfg(test)]
impl CommandXRayHover {
    pub(crate) fn is_visible_for_test(&self) -> bool {
        self.0.lock().as_ref().is_some_and(|state| state.visible)
    }

    pub(crate) fn is_armed_for_test(&self) -> bool {
        self.0.lock().is_some()
    }

    pub(crate) fn is_user_dismissed_for_test(&self) -> bool {
        self.0
            .lock()
            .as_ref()
            .is_some_and(|state| state.user_dismissed)
    }
}
