//! Grouping of buffered wheel input for the TUI dispatch loops.
//!
//! Every dispatched event that changes state ends in a flush, and the window's
//! invalidation callback repaints the whole frame from there. One wheel notch
//! therefore costs one full layout + paint. When a frame is slow — a long agent
//! transcript, say — a trackpad gesture keeps producing notches while that
//! frame is on screen, and each queued notch buys another serial full frame.
//! The viewport ends up several seconds behind the gesture even though only the
//! final scroll position is visible.
//!
//! Both dispatch loops therefore drain whatever input is already buffered and
//! group each run of adjacent wheel notches over the same cell. Matching cells
//! alone does **not** establish that a run has a single dispatch target: the
//! viewport offers a wheel event to its visible children before its scroll
//! wrapper, so scrolling can move a different consumer — an alt-screen terminal
//! forwarding wheel input to its process, say — under that cell part-way
//! through the burst. The loops therefore dispatch a run's first notch on its
//! own and merge the remainder only when the scroll owner that handled it
//! reports that no other wheel consumer sits inside it (see
//! [`TuiEventContext::set_wheel_coalescable`]). A burst then costs two frames
//! instead of N, and falls back to one frame per notch whenever the target can
//! change.
//!
//! [`TuiEventContext::set_wheel_coalescable`]: crate::elements::tui::TuiEventContext::set_wheel_coalescable

use crate::elements::tui::TuiEvent;

/// Upper bound on how many buffered terminal events one dispatch batch drains,
/// so a flood of input cannot stall the loop in the drain phase.
pub(crate) const MAX_BATCHED_EVENTS: usize = 256;

/// One drained terminal event, in dispatch order.
#[derive(Debug)]
pub(crate) enum TuiBatchedInput {
    /// A terminal resize, which the loops handle by invalidating rather than by
    /// dispatching through the element tree.
    Resize,
    /// An input event converted into the TUI vocabulary.
    Event(TuiEvent),
}

/// A batch split into dispatch units.
#[derive(Debug)]
pub(crate) enum TuiInputGroup {
    /// An input dispatched on its own, exactly as an undrained loop would.
    Single(TuiBatchedInput),
    /// Two or more adjacent wheel notches over the same cell, in order. The
    /// caller dispatches the first and merges the rest only once that dispatch
    /// reports a stable target (see the module docs).
    WheelRun(Vec<TuiEvent>),
}

/// Splits `inputs` into dispatch units, gathering runs of adjacent wheel events
/// that are candidates for merging.
///
/// Two wheel events join the same run only when their position, modifiers, and
/// precision match, so a gesture that crosses into another cell — or picks up a
/// modifier part-way through — starts a new run. Any other event (including a
/// resize) ends the run, and the order of everything is preserved.
pub(crate) fn group_wheel_runs(inputs: Vec<TuiBatchedInput>) -> Vec<TuiInputGroup> {
    let mut groups: Vec<TuiInputGroup> = Vec::with_capacity(inputs.len());
    let mut run: Vec<TuiEvent> = Vec::new();
    for input in inputs {
        match input {
            TuiBatchedInput::Event(event) if matches!(event, TuiEvent::ScrollWheel { .. }) => {
                // A notch that does not continue the current run closes it and
                // opens its own, rather than being dispatched on its own: the
                // notches after it may still merge together.
                if !run.last().is_none_or(|last| joins_run(last, &event)) {
                    flush_run(&mut run, &mut groups);
                }
                run.push(event);
            }
            input => {
                flush_run(&mut run, &mut groups);
                groups.push(TuiInputGroup::Single(input));
            }
        }
    }
    flush_run(&mut run, &mut groups);
    groups
}

/// Sums a run's notches into one wheel event carrying the whole distance,
/// positioned at the shared cell. Returns `None` for an empty run.
pub(crate) fn merge_wheel_events(events: &[TuiEvent]) -> Option<TuiEvent> {
    let (first, rest) = events.split_first()?;
    let TuiEvent::ScrollWheel {
        position,
        delta,
        precise,
        modifiers,
    } = first
    else {
        return None;
    };
    let mut merged = *delta;
    for event in rest {
        let TuiEvent::ScrollWheel { delta, .. } = event else {
            return None;
        };
        merged.0 = merged.0.saturating_add(delta.0);
        merged.1 = merged.1.saturating_add(delta.1);
    }
    Some(TuiEvent::ScrollWheel {
        position: *position,
        delta: merged,
        precise: *precise,
        modifiers: *modifiers,
    })
}

/// Whether `event` continues the run that `last` belongs to.
fn joins_run(last: &TuiEvent, event: &TuiEvent) -> bool {
    let (
        TuiEvent::ScrollWheel {
            position: last_position,
            precise: last_precise,
            modifiers: last_modifiers,
            ..
        },
        TuiEvent::ScrollWheel {
            position,
            precise,
            modifiers,
            ..
        },
    ) = (last, event)
    else {
        return false;
    };
    last_position == position && last_precise == precise && last_modifiers == modifiers
}

/// Emits the pending run: a lone notch is an ordinary single input.
fn flush_run(run: &mut Vec<TuiEvent>, groups: &mut Vec<TuiInputGroup>) {
    match run.len() {
        0 => {}
        1 => groups.push(TuiInputGroup::Single(TuiBatchedInput::Event(
            run.drain(..)
                .next()
                .expect("the run holds exactly one event"),
        ))),
        _ => groups.push(TuiInputGroup::WheelRun(std::mem::take(run))),
    }
}

#[cfg(test)]
#[path = "scroll_coalescing_tests.rs"]
mod tests;
