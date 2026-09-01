//! Regression tests for position-aware multi-click detection.
//!
//! See <https://github.com/warpdotdev/warp/issues/15706>: quickly clicking two different tab rows
//! (different position, same button, within [`MULTI_CLICK_INTERVAL`]) was incorrectly treated as
//! a double-click on the second row, since click-count detection only considered button and
//! timing, unlike macOS's native `NSEvent.clickCount`, which also resets on excessive movement.

use pathfinder_geometry::vector::vec2f;
use winit::event::MouseButton;

use super::*;

#[test]
fn test_repeated_click_at_same_position_increments_click_count() {
    let mut window_state = WindowState::new(crate::WindowId::new());
    let position = vec2f(10., 20.);

    let first =
        window_state.determine_click_count_and_update_button_state(MouseButton::Left, position);
    let second =
        window_state.determine_click_count_and_update_button_state(MouseButton::Left, position);
    let third =
        window_state.determine_click_count_and_update_button_state(MouseButton::Left, position);

    assert_eq!(first, 1);
    assert_eq!(second, 2);
    assert_eq!(third, 3);
}

#[test]
fn test_click_within_multi_click_distance_still_increments_click_count() {
    let mut window_state = WindowState::new(crate::WindowId::new());
    let first_position = vec2f(10., 20.);
    // Small jitter, well within `MULTI_CLICK_DISTANCE`, that should still count as the same spot.
    let second_position = first_position + vec2f(1., 1.);

    let first = window_state
        .determine_click_count_and_update_button_state(MouseButton::Left, first_position);
    let second = window_state
        .determine_click_count_and_update_button_state(MouseButton::Left, second_position);

    assert_eq!(first, 1);
    assert_eq!(second, 2);
}

#[test]
fn test_click_on_sufficiently_separated_position_resets_click_count() {
    let mut window_state = WindowState::new(crate::WindowId::new());
    let first_position = vec2f(10., 20.);
    // Simulates quickly clicking a different tab row well outside `MULTI_CLICK_DISTANCE`, even
    // though it happens within `MULTI_CLICK_INTERVAL` of the first click.
    let second_position = first_position + vec2f(0., 40.);

    let first = window_state
        .determine_click_count_and_update_button_state(MouseButton::Left, first_position);
    let second = window_state
        .determine_click_count_and_update_button_state(MouseButton::Left, second_position);

    assert_eq!(first, 1);
    assert_eq!(
        second, 1,
        "a click far from the previous one must not be counted as a double-click"
    );
}

#[test]
fn test_click_after_separated_click_can_start_a_new_multi_click_sequence() {
    let mut window_state = WindowState::new(crate::WindowId::new());
    let first_position = vec2f(10., 20.);
    let second_position = first_position + vec2f(0., 40.);

    window_state.determine_click_count_and_update_button_state(MouseButton::Left, first_position);
    window_state.determine_click_count_and_update_button_state(MouseButton::Left, second_position);
    // A third click at the same (new) position as the second click should count as the start of
    // a fresh multi-click sequence at that position, i.e. a double-click there.
    let third = window_state
        .determine_click_count_and_update_button_state(MouseButton::Left, second_position);

    assert_eq!(third, 2);
}
