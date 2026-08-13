use std::time::Duration;

use instant::Instant;

use super::{SMOOTH_SCROLL_DURATION, SmoothScrollController};

#[test]
fn ease_out_cubic_reaches_exact_target_without_overshoot() {
    let start = Instant::now();
    let mut controller = SmoothScrollController::new(0.0);
    controller.add_delta(100.0, start);

    // Never overshoots the target at any sampled point along the way.
    for millis in [0, 15, 30, 60, 90, 119] {
        let displayed = controller.displayed_position(start + Duration::from_millis(millis));
        assert!(
            (0.0..=100.0).contains(&displayed),
            "displayed position {displayed} out of bounds at {millis}ms"
        );
    }

    // Reaches the exact target once the duration elapses, and stops animating.
    let displayed = controller.displayed_position(start + SMOOTH_SCROLL_DURATION);
    assert_eq!(displayed, 100.0);
    assert!(!controller.is_animating());
}

#[test]
fn same_direction_inputs_compose_without_restarting_existing_progress() {
    let start = Instant::now();
    let mut controller = SmoothScrollController::new(0.0);
    controller.add_delta(40.0, start);

    let midpoint = start + Duration::from_millis(60);
    let progress_before_second_notch = controller.displayed_position(midpoint);
    assert!(progress_before_second_notch > 0.0);

    // A second same-direction notch arrives mid-flight.
    controller.add_delta(40.0, midpoint);

    // The already-visible motion isn't restarted: displayed position doesn't regress.
    let just_after = controller.displayed_position(midpoint);
    assert!(just_after >= progress_before_second_notch);

    // The eventual target is the sum of both contributions.
    assert_eq!(controller.target(), 80.0);
    let final_position = controller.displayed_position(midpoint + SMOOTH_SCROLL_DURATION);
    assert_eq!(final_position, 80.0);
}

#[test]
fn opposing_input_discards_unrendered_remainder() {
    let start = Instant::now();
    let mut controller = SmoothScrollController::new(0.0);
    controller.add_delta(100.0, start);

    let reversal_time = start + Duration::from_millis(60);
    let displayed_at_reversal = controller.displayed_position(reversal_time);
    assert!(displayed_at_reversal > 0.0 && displayed_at_reversal < 100.0);

    // Reverse direction before the first contribution finishes.
    controller.add_delta(-30.0, reversal_time);

    // Reversal starts from the currently displayed position, not from 0 or from the old target.
    assert_eq!(
        controller.displayed_position(reversal_time),
        displayed_at_reversal
    );
    assert_eq!(controller.target(), displayed_at_reversal - 30.0);

    // The old (discarded) target of 100 is never reached.
    let far_future = reversal_time + SMOOTH_SCROLL_DURATION;
    let final_position = controller.displayed_position(far_future);
    assert_eq!(final_position, displayed_at_reversal - 30.0);
}

#[test]
fn late_frame_emits_exact_remaining_distance() {
    let start = Instant::now();
    let mut controller = SmoothScrollController::new(10.0);
    controller.add_delta(50.0, start);

    // A frame arrives long after the animation should have finished (e.g. the app was
    // suspended). The exact remaining distance is still applied, with no error accumulation.
    let displayed = controller.displayed_position(start + Duration::from_secs(5));
    assert_eq!(displayed, 60.0);
    assert!(!controller.is_animating());
}

#[test]
fn cancel_settles_at_displayed_position_and_stops_animation() {
    let start = Instant::now();
    let mut controller = SmoothScrollController::new(0.0);
    controller.add_delta(100.0, start);

    let cancel_time = start + Duration::from_millis(45);
    let displayed_at_cancel = controller.displayed_position(cancel_time);
    let returned = controller.cancel(cancel_time);

    assert_eq!(returned, displayed_at_cancel);
    assert!(!controller.is_animating());
    assert_eq!(controller.target(), displayed_at_cancel);

    // No further motion happens once cancelled, even much later.
    let later = cancel_time + SMOOTH_SCROLL_DURATION;
    assert_eq!(controller.displayed_position(later), displayed_at_cancel);
}

#[test]
fn set_position_immediately_overrides_in_flight_animation() {
    let start = Instant::now();
    let mut controller = SmoothScrollController::new(0.0);
    controller.add_delta(100.0, start);

    controller.set_position_immediately(250.0);

    assert!(!controller.is_animating());
    assert_eq!(controller.target(), 250.0);
    assert_eq!(
        controller.displayed_position(start + Duration::from_millis(60)),
        250.0
    );
}

#[test]
fn zero_delta_is_a_no_op() {
    let start = Instant::now();
    let mut controller = SmoothScrollController::new(5.0);
    controller.add_delta(0.0, start);

    assert!(!controller.is_animating());
    assert_eq!(controller.displayed_position(start), 5.0);
}
