use super::{TuiBatchedInput, TuiInputGroup, group_wheel_runs, merge_wheel_events};
use crate::elements::tui::{TuiEvent, TuiPoint, TuiScrollDelta};
use crate::event::ModifiersState;

fn wheel_event(x: u16, y: u16, delta: isize, modifiers: ModifiersState) -> TuiEvent {
    TuiEvent::ScrollWheel {
        position: TuiPoint::new(x, y),
        delta: (0, delta),
        precise: false,
        modifiers,
    }
}

fn wheel(x: u16, y: u16, delta: isize) -> TuiBatchedInput {
    TuiBatchedInput::Event(wheel_event(x, y, delta, ModifiersState::default()))
}

fn wheel_with(x: u16, y: u16, delta: isize, modifiers: ModifiersState) -> TuiBatchedInput {
    TuiBatchedInput::Event(wheel_event(x, y, delta, modifiers))
}

fn mouse_moved(x: u16, y: u16) -> TuiBatchedInput {
    TuiBatchedInput::Event(TuiEvent::MouseMoved {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
        is_synthetic: false,
    })
}

/// A readable shape for each dispatch unit: the notch count of a wheel run, or
/// `None` for anything dispatched on its own.
fn shapes(groups: &[TuiInputGroup]) -> Vec<Option<usize>> {
    groups
        .iter()
        .map(|group| match group {
            TuiInputGroup::WheelRun(events) => Some(events.len()),
            TuiInputGroup::Single(_) => None,
        })
        .collect()
}

fn merged_delta(events: &[TuiEvent]) -> Option<TuiScrollDelta> {
    match merge_wheel_events(events)? {
        TuiEvent::ScrollWheel { delta, .. } => Some(delta),
        _ => None,
    }
}

/// A burst of notches over the same cell forms one run, so the loop can apply
/// its tail as a single scroll instead of one full frame per notch.
#[test]
fn a_wheel_burst_over_one_cell_forms_one_run() {
    let burst = (0..8).map(|_| wheel(4, 4, -1)).collect();

    assert_eq!(shapes(&group_wheel_runs(burst)), vec![Some(8)]);
}

/// A lone notch is dispatched exactly as an undrained loop would dispatch it.
#[test]
fn a_lone_notch_is_not_a_run() {
    assert_eq!(shapes(&group_wheel_runs(vec![wheel(0, 0, -1)])), vec![None]);
}

/// A wheel event over a different cell may land on a different element, so it
/// starts a new run rather than joining the previous one.
#[test]
fn a_wheel_event_over_another_cell_starts_a_new_run() {
    let burst = vec![wheel(0, 0, -1), wheel(0, 0, -1), wheel(9, 9, -1)];

    assert_eq!(shapes(&group_wheel_runs(burst)), vec![Some(2), None]);
}

/// A modifier picked up part-way through a gesture can change what the scroll
/// means, so it does not join the preceding run.
#[test]
fn a_modifier_change_starts_a_new_run() {
    let shift = ModifiersState {
        shift: true,
        ..ModifiersState::default()
    };
    let burst = vec![
        wheel(2, 2, -1),
        wheel_with(2, 2, -1, shift),
        wheel_with(2, 2, -1, shift),
    ];

    assert_eq!(shapes(&group_wheel_runs(burst)), vec![None, Some(2)]);
}

/// Only adjacent wheel events group: any other event ends the run and keeps its
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
        wheel(1, 1, -1),
    ];

    assert_eq!(
        shapes(&group_wheel_runs(batch)),
        vec![Some(2), None, None, None, Some(2)],
    );
}

/// Merging a run sums its notches so the whole distance is applied at once.
#[test]
fn merging_a_run_sums_its_notches() {
    let run = (0..5)
        .map(|_| wheel_event(4, 4, -1, ModifiersState::default()))
        .collect::<Vec<_>>();

    assert_eq!(merged_delta(&run), Some((0, -5)));
}

/// Opposite directions inside one run cancel rather than producing two scrolls
/// that each repaint.
#[test]
fn opposite_notches_inside_a_run_net_out() {
    let run = vec![
        wheel_event(0, 0, -3, ModifiersState::default()),
        wheel_event(0, 0, 2, ModifiersState::default()),
    ];

    assert_eq!(merged_delta(&run), Some((0, -1)));
}

/// An empty tail has nothing to merge, which is how the loop learns there is no
/// second dispatch to make.
#[test]
fn merging_an_empty_run_yields_nothing() {
    assert!(merge_wheel_events(&[]).is_none());
}
