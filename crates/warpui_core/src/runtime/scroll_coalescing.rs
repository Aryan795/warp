//! Coalescing of buffered wheel input for the TUI dispatch loops.
//!
//! Every dispatched event that changes state ends in a flush, and the window's
//! invalidation callback repaints the whole frame from there. One wheel notch
//! therefore costs one full layout + paint. When a frame is slow — a long agent
//! transcript, say — a trackpad gesture keeps producing notches while that
//! frame is on screen, and each queued notch buys another serial full frame.
//! The viewport ends up several seconds behind the gesture even though only the
//! final scroll position is visible.
//!
//! Both dispatch loops therefore drain whatever input is already buffered
//! before dispatching, and merge each run of adjacent wheel events over the
//! same cell into one event carrying the summed delta. A burst of N notches
//! applies as a single scroll of N notches' worth of rows and costs one frame
//! instead of N. Nothing else is merged, and the order of everything else is
//! preserved, so an event still sees the frame painted for the event before it.

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

/// Merges each run of adjacent wheel events over the same cell into a single
/// event carrying their summed delta.
///
/// Two wheel events merge only when their position, modifiers, and precision
/// match, so a gesture that crosses into another element — or picks up a
/// modifier partway through — is still delivered as separate scrolls. Any other
/// event (including a resize) ends the run.
pub(crate) fn coalesce_scroll_events(inputs: Vec<TuiBatchedInput>) -> Vec<TuiBatchedInput> {
    let mut coalesced: Vec<TuiBatchedInput> = Vec::with_capacity(inputs.len());
    for input in inputs {
        let TuiBatchedInput::Event(TuiEvent::ScrollWheel {
            position,
            delta,
            precise,
            modifiers,
        }) = &input
        else {
            coalesced.push(input);
            continue;
        };
        if let Some(TuiBatchedInput::Event(TuiEvent::ScrollWheel {
            position: pending_position,
            delta: pending_delta,
            precise: pending_precise,
            modifiers: pending_modifiers,
        })) = coalesced.last_mut()
            && pending_position == position
            && pending_precise == precise
            && pending_modifiers == modifiers
        {
            pending_delta.0 = pending_delta.0.saturating_add(delta.0);
            pending_delta.1 = pending_delta.1.saturating_add(delta.1);
            continue;
        }
        coalesced.push(input);
    }
    coalesced
}

#[cfg(test)]
#[path = "scroll_coalescing_tests.rs"]
mod tests;
