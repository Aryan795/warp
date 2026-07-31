use super::{TuiBatchedInput, coalesce_scroll_events};
use crate::elements::tui::{TuiEvent, TuiPoint, TuiScrollDelta};
use crate::event::ModifiersState;

fn wheel(x: u16, y: u16, delta: isize) -> TuiBatchedInput {
    wheel_with(x, y, delta, ModifiersState::default())
}

fn wheel_with(x: u16, y: u16, delta: isize, modifiers: ModifiersState) -> TuiBatchedInput {
    TuiBatchedInput::Event(TuiEvent::ScrollWheel {
        position: TuiPoint::new(x, y),
        delta: (0, delta),
        precise: false,
        modifiers,
    })
}

fn mouse_moved(x: u16, y: u16) -> TuiBatchedInput {
    TuiBatchedInput::Event(TuiEvent::MouseMoved {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
        is_synthetic: false,
    })
}

/// The scroll deltas of each coalesced wheel event, in order. `None` marks a
/// non-wheel entry so ordering can be asserted alongside the merges.
fn deltas(inputs: Vec<TuiBatchedInput>) -> Vec<Option<TuiScrollDelta>> {
    inputs
        .into_iter()
        .map(|input| match input {
            TuiBatchedInput::Event(TuiEvent::ScrollWheel { delta, .. }) => Some(delta),
            TuiBatchedInput::Event(_) | TuiBatchedInput::Resize => None,
        })
        .collect()
}

/// A burst of notches over the same cell becomes one scroll of the summed
/// delta, so the whole gesture costs one layout + paint instead of one per
/// notch.
#[test]
fn a_wheel_burst_over_one_cell_becomes_a_single_scroll() {
    let burst = (0..8).map(|_| wheel(4, 4, -1)).collect();

    assert_eq!(deltas(coalesce_scroll_events(burst)), vec![Some((0, -8))]);
}

/// Opposite directions inside one burst cancel rather than producing two
/// scrolls that each repaint.
#[test]
fn opposite_notches_inside_a_burst_net_out() {
    let burst = vec![wheel(0, 0, -3), wheel(0, 0, 2)];

    assert_eq!(deltas(coalesce_scroll_events(burst)), vec![Some((0, -1))]);
}

/// A wheel event over a different cell may land on a different element, so it
/// starts a new scroll instead of joining the run.
#[test]
fn a_wheel_event_over_another_cell_starts_a_new_scroll() {
    let burst = vec![wheel(0, 0, -1), wheel(0, 0, -1), wheel(9, 9, -1)];

    assert_eq!(
        deltas(coalesce_scroll_events(burst)),
        vec![Some((0, -2)), Some((0, -1))],
    );
}

/// A modifier picked up partway through a gesture can change what the scroll
/// means, so it is not merged into the preceding run.
#[test]
fn a_modifier_change_starts_a_new_scroll() {
    let shift = ModifiersState {
        shift: true,
        ..ModifiersState::default()
    };
    let burst = vec![
        wheel(2, 2, -1),
        wheel_with(2, 2, -1, shift),
        wheel_with(2, 2, -1, shift),
    ];

    assert_eq!(
        deltas(coalesce_scroll_events(burst)),
        vec![Some((0, -1)), Some((0, -2))],
    );
}

/// Only adjacent wheel events merge: any other event ends the run and keeps its
/// place, so an event still sees the frame painted for the event before it.
#[test]
fn other_events_break_a_run_and_keep_their_order() {
    let batch = vec![
        wheel(1, 1, -1),
        wheel(1, 1, -1),
        mouse_moved(1, 1),
        wheel(1, 1, -1),
        TuiBatchedInput::Resize,
        wheel(1, 1, -1),
    ];

    assert_eq!(
        deltas(coalesce_scroll_events(batch)),
        vec![Some((0, -2)), None, Some((0, -1)), None, Some((0, -1))],
    );
}

/// A batch with no wheel events passes through untouched.
#[test]
fn a_batch_without_wheel_events_is_unchanged() {
    let batch = vec![mouse_moved(0, 0), TuiBatchedInput::Resize];

    assert_eq!(deltas(coalesce_scroll_events(batch)), vec![None, None]);
}
