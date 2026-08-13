//! A deterministic, time-based animation controller for smoothing discrete (non-precise)
//! mouse-wheel scroll input.
//!
//! See `specs/CSAT-6046/TECH.md` for the design rationale. This module intentionally lives
//! outside the GUI element tree so that both the generic WarpUI scrollables (Phase 1) and, in
//! the future, `TerminalView` scrollback (Phase 2) can share the same controller.

use std::time::Duration;

use instant::Instant;

/// Duration of the ease-out tween applied to one discrete (non-precise) wheel notch.
pub const SMOOTH_SCROLL_DURATION: Duration = Duration::from_millis(120);

/// Cadence at which a view with an active [`SmoothScrollController`] should request another
/// repaint. Matches the touch-momentum timer's cadence so 120Hz displays are not capped to 60Hz.
pub const SMOOTH_SCROLL_FRAME_INTERVAL: Duration = Duration::from_millis(8);

/// Cubic ease-out: starts fast and slows into the target, reaching it exactly at `t == 1` with
/// no overshoot.
fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// A single discrete-scroll contribution easing toward `delta` over [`SMOOTH_SCROLL_DURATION`],
/// starting at `start`.
#[derive(Debug, Clone, Copy)]
struct Contribution {
    start: Instant,
    delta: f32,
}

impl Contribution {
    fn progress(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.start).as_secs_f32();
        (elapsed / SMOOTH_SCROLL_DURATION.as_secs_f32()).clamp(0.0, 1.0)
    }

    fn is_complete(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) >= SMOOTH_SCROLL_DURATION
    }

    fn eased_amount(&self, now: Instant) -> f32 {
        self.delta * ease_out_cubic(self.progress(now))
    }
}

/// Animates a single scroll axis toward an exact target position using additive, time-based
/// ease-out contributions.
///
/// Each discrete wheel notch creates a [`Contribution`] that eases toward its own delta
/// independently of any other contribution; the position that should currently be displayed is
/// the sum of a settled base position and every active contribution's eased progress at the
/// current instant. This lets same-direction notches compose without restarting the progress of
/// motion that's already visible (each new notch just adds another independently-easing
/// contribution), and lets opposite-direction input reverse immediately by collapsing the
/// currently displayed position into the settled base before animating the other way.
///
/// The controller is a pure function of injected time: every method that depends on "now" takes
/// an explicit [`Instant`] rather than reading the wall clock, which keeps it deterministic and
/// testable.
#[derive(Debug, Clone, Default)]
pub struct SmoothScrollController {
    /// The settled position: the sum of every contribution that has fully eased in.
    committed: f32,
    /// Contributions still easing toward their target delta.
    contributions: Vec<Contribution>,
}

impl SmoothScrollController {
    pub fn new(initial_position: f32) -> Self {
        Self {
            committed: initial_position,
            contributions: Vec::new(),
        }
    }

    /// Folds any contribution that has fully eased in into `committed`, removing it from the
    /// active list. Idempotent.
    fn settle_expired(&mut self, now: Instant) {
        let mut i = 0;
        while i < self.contributions.len() {
            if self.contributions[i].is_complete(now) {
                self.committed += self.contributions.remove(i).delta;
            } else {
                i += 1;
            }
        }
    }

    /// The position that should currently be displayed/painted. Settles any contribution that
    /// has fully eased in as a side effect, so the active contribution list stays bounded.
    pub fn displayed_position(&mut self, now: Instant) -> f32 {
        self.settle_expired(now);
        self.contributions
            .iter()
            .fold(self.committed, |pos, c| pos + c.eased_amount(now))
    }

    /// The exact position this controller is animating toward: the displayed position once
    /// every active contribution finishes easing in. Unlike [`Self::displayed_position`], this
    /// never lags behind, which makes it the right value to use for bounds/propagation
    /// decisions (e.g. deciding whether a nested scrollable can still move, or whether a wheel
    /// event should propagate to a parent).
    pub fn target(&self) -> f32 {
        self.contributions
            .iter()
            .fold(self.committed, |pos, c| pos + c.delta)
    }

    /// Whether a contribution is still easing in. Settles any contribution that has fully
    /// eased in as a side effect (like [`Self::displayed_position`]), so this reports the
    /// current state even if nothing else has read the controller since the contribution
    /// expired.
    pub fn is_animating(&mut self, now: Instant) -> bool {
        self.settle_expired(now);
        !self.contributions.is_empty()
    }

    /// Adds a discrete scroll contribution of `delta`, starting at `now`.
    ///
    /// A `delta` in the same direction as the controller's current net motion composes
    /// additively with it: the new contribution starts its own ease-out from `now`, without
    /// disturbing the progress of contributions already in flight, and the eventual target
    /// becomes the sum of every contribution's delta.
    ///
    /// A `delta` in the opposite direction discards the unrendered remainder of the current
    /// motion: the currently displayed position becomes the new settled base, then the new
    /// contribution eases from there.
    pub fn add_delta(&mut self, delta: f32, now: Instant) {
        if delta == 0.0 {
            return;
        }

        self.settle_expired(now);

        let net: f32 = self.contributions.iter().map(|c| c.delta).sum();
        if net != 0.0 && net.signum() != delta.signum() {
            self.committed = self.displayed_position(now);
            self.contributions.clear();
        }

        self.contributions.push(Contribution { start: now, delta });
    }

    /// Cancels any in-flight animation, settling at the currently displayed position, and
    /// returns that position.
    pub fn cancel(&mut self, now: Instant) -> f32 {
        let displayed = self.displayed_position(now);
        self.committed = displayed;
        self.contributions.clear();
        displayed
    }

    /// Immediately jumps to `position`, cancelling any in-flight animation.
    pub fn set_position_immediately(&mut self, position: f32) {
        self.committed = position;
        self.contributions.clear();
    }
}

#[cfg(test)]
#[path = "smooth_scroll_tests.rs"]
mod tests;
