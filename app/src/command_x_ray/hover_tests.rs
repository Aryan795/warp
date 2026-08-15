use std::collections::HashSet;
use std::time::Duration;

use instant::Instant;
use pathfinder_geometry::vector::vec2f;
use warpui::TaskId;

use super::hover::{
    COMMAND_X_RAY_HOVER_DELAY, CommandXRayHover, HoverOutcome, HoverProbe, HoverTimer,
};

/// A stand-in for the presenter's timer, so the state machine's delay rule can be exercised
/// without a live `EventContext`. The clock is advanced by the test rather than by wall time.
struct FakeTimer {
    now: Instant,
    armed: Vec<TaskId>,
    cancelled: HashSet<TaskId>,
}

impl FakeTimer {
    fn new() -> Self {
        Self {
            now: Instant::now(),
            armed: Vec::new(),
            cancelled: HashSet::new(),
        }
    }

    /// Moves the fake clock forward, so timers armed before this call have elapsed.
    fn advance(&mut self, by: Duration) {
        self.now += by;
    }

    fn is_cancelled(&self, timer_id: TaskId) -> bool {
        self.cancelled.contains(&timer_id)
    }

    fn last_armed(&self) -> TaskId {
        *self.armed.last().expect("a timer was armed")
    }
}

impl HoverTimer for FakeTimer {
    fn arm(&mut self, delay: Duration) -> (TaskId, Instant) {
        let timer_id = TaskId::new();
        self.armed.push(timer_id);
        (timer_id, self.now + delay)
    }

    fn cancel(&mut self, timer_id: TaskId) {
        self.cancelled.insert(timer_id);
    }

    fn now(&self) -> Instant {
        self.now
    }
}

fn over_token() -> HoverProbe {
    HoverProbe {
        is_clamped: false,
        is_within_token: true,
    }
}

fn off_token() -> HoverProbe {
    HoverProbe {
        is_clamped: false,
        is_within_token: false,
    }
}

fn clamped() -> HoverProbe {
    HoverProbe {
        is_clamped: true,
        is_within_token: false,
    }
}

/// Moves the pointer to `position`, rests long enough for the delay to elapse, and moves again
/// within the threshold, which is what actually opens the tooltip.
fn hover_until_shown(hover: &CommandXRayHover, timer: &mut FakeTimer, x: f32, y: f32) {
    assert_eq!(
        hover.on_mouse_moved(vec2f(x, y), true, off_token, timer),
        HoverOutcome::Idle,
        "the first move only arms the timer"
    );
    timer.advance(COMMAND_X_RAY_HOVER_DELAY);
    assert_eq!(
        hover.on_mouse_moved(vec2f(x + 1., y), true, off_token, timer),
        HoverOutcome::Show
    );
}

#[test]
fn first_move_arms_the_timer_without_showing() {
    let hover = CommandXRayHover::default();
    let mut timer = FakeTimer::new();

    assert_eq!(
        hover.on_mouse_moved(vec2f(10., 10.), true, off_token, &mut timer),
        HoverOutcome::Idle
    );
    assert!(hover.is_armed_for_test());
    assert!(!hover.is_visible_for_test());
    assert_eq!(timer.armed.len(), 1);
}

#[test]
fn does_not_show_before_the_hover_delay_elapses() {
    let hover = CommandXRayHover::default();
    let mut timer = FakeTimer::new();

    hover.on_mouse_moved(vec2f(10., 10.), true, off_token, &mut timer);
    // Just short of the delay, and within the movement threshold.
    timer.advance(COMMAND_X_RAY_HOVER_DELAY - Duration::from_millis(1));
    assert_eq!(
        hover.on_mouse_moved(vec2f(11., 10.), true, off_token, &mut timer),
        HoverOutcome::Idle
    );
    assert!(!hover.is_visible_for_test());
}

#[test]
fn shows_once_the_delay_has_elapsed_and_the_pointer_has_barely_moved() {
    let hover = CommandXRayHover::default();
    let mut timer = FakeTimer::new();

    hover.on_mouse_moved(vec2f(10., 10.), true, off_token, &mut timer);
    let armed = timer.last_armed();
    timer.advance(COMMAND_X_RAY_HOVER_DELAY);

    assert_eq!(
        hover.on_mouse_moved(vec2f(12., 10.), true, off_token, &mut timer),
        HoverOutcome::Show
    );
    assert!(hover.is_visible_for_test());
    assert!(
        timer.is_cancelled(armed),
        "the pending timer is cancelled once the tooltip is open"
    );
}

#[test]
fn movement_past_the_threshold_restarts_the_delay_instead_of_showing() {
    let hover = CommandXRayHover::default();
    let mut timer = FakeTimer::new();

    hover.on_mouse_moved(vec2f(10., 10.), true, off_token, &mut timer);
    timer.advance(COMMAND_X_RAY_HOVER_DELAY);

    // 4px away is past COMMAND_X_RAY_HOVER_THRESHOLD_PX, so this is a new hover, not a show.
    assert_eq!(
        hover.on_mouse_moved(vec2f(14., 10.), true, off_token, &mut timer),
        HoverOutcome::Idle
    );
    assert!(!hover.is_visible_for_test());
    assert_eq!(timer.armed.len(), 2, "a fresh timer is armed");
}

#[test]
fn does_not_show_for_a_clamped_position() {
    let hover = CommandXRayHover::default();
    let mut timer = FakeTimer::new();

    hover.on_mouse_moved(vec2f(10., 10.), true, clamped, &mut timer);
    timer.advance(COMMAND_X_RAY_HOVER_DELAY);

    assert_eq!(
        hover.on_mouse_moved(vec2f(11., 10.), true, clamped, &mut timer),
        HoverOutcome::Idle,
        "a position past the end of the text describes nothing"
    );
    assert!(!hover.is_visible_for_test());
}

#[test]
fn stays_open_while_the_pointer_is_still_over_the_token() {
    let hover = CommandXRayHover::default();
    let mut timer = FakeTimer::new();
    hover_until_shown(&hover, &mut timer, 10., 10.);

    assert_eq!(
        hover.on_mouse_moved(vec2f(40., 10.), true, over_token, &mut timer),
        HoverOutcome::Idle
    );
    assert!(hover.is_visible_for_test());
}

#[test]
fn stays_open_for_a_sub_threshold_move_off_the_token() {
    let hover = CommandXRayHover::default();
    let mut timer = FakeTimer::new();
    hover_until_shown(&hover, &mut timer, 10., 10.);

    // Off the token, but only 1px from the last tracked position.
    assert_eq!(
        hover.on_mouse_moved(vec2f(12., 10.), true, off_token, &mut timer),
        HoverOutcome::Idle
    );
    assert!(hover.is_visible_for_test());
}

#[test]
fn hides_when_the_pointer_leaves_the_token_bounds() {
    let hover = CommandXRayHover::default();
    let mut timer = FakeTimer::new();
    hover_until_shown(&hover, &mut timer, 10., 10.);

    assert_eq!(
        hover.on_mouse_moved(vec2f(60., 10.), true, off_token, &mut timer),
        HoverOutcome::Hide
    );
    assert!(!hover.is_visible_for_test());
    assert!(
        hover.is_armed_for_test(),
        "leaving the token re-arms rather than clearing, so a new hover can open"
    );
}

#[test]
fn hides_and_clears_when_the_pointer_leaves_the_host() {
    let hover = CommandXRayHover::default();
    let mut timer = FakeTimer::new();
    hover_until_shown(&hover, &mut timer, 10., 10.);

    assert_eq!(
        hover.on_mouse_moved(vec2f(500., 500.), false, off_token, &mut timer),
        HoverOutcome::Hide
    );
    assert!(!hover.is_armed_for_test(), "a mouse leave resets the state");
}

#[test]
fn a_dismissal_keeps_the_tooltip_closed_until_the_pointer_moves() {
    let hover = CommandXRayHover::default();
    let mut timer = FakeTimer::new();
    hover_until_shown(&hover, &mut timer, 10., 10.);

    // This is what an edit, an escape, or an `InspectCommand` toggle does.
    hover.mark_user_dismissed();
    assert!(hover.is_user_dismissed_for_test());
    timer.advance(COMMAND_X_RAY_HOVER_DELAY);

    // Jitter within the threshold must not re-open it.
    assert_eq!(
        hover.on_mouse_moved(vec2f(12., 10.), true, over_token, &mut timer),
        HoverOutcome::Idle
    );
    assert!(hover.is_user_dismissed_for_test());

    // A real move clears the dismissal and starts a fresh hover.
    assert_eq!(
        hover.on_mouse_moved(vec2f(60., 10.), true, over_token, &mut timer),
        HoverOutcome::Hide
    );
    assert!(!hover.is_user_dismissed_for_test());
    assert!(hover.is_armed_for_test());
}

#[test]
fn a_dismissed_hover_can_open_again_after_the_pointer_moves() {
    let hover = CommandXRayHover::default();
    let mut timer = FakeTimer::new();
    hover_until_shown(&hover, &mut timer, 10., 10.);
    hover.mark_user_dismissed();

    hover.on_mouse_moved(vec2f(60., 10.), true, off_token, &mut timer);
    timer.advance(COMMAND_X_RAY_HOVER_DELAY);
    assert_eq!(
        hover.on_mouse_moved(vec2f(61., 10.), true, off_token, &mut timer),
        HoverOutcome::Show
    );
}

#[test]
fn out_of_bounds_moves_are_not_reported_as_handled_when_nothing_was_pending() {
    let hover = CommandXRayHover::default();
    let mut timer = FakeTimer::new();

    let outcome = hover.on_mouse_moved(vec2f(500., 500.), false, off_token, &mut timer);
    assert_eq!(outcome, HoverOutcome::Idle);
    assert!(!outcome.is_handled());
}
