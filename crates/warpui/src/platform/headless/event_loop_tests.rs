use super::{InterruptAction, interrupt_action};

#[test]
fn first_interrupt_requests_graceful_termination() {
    assert_eq!(interrupt_action(0), InterruptAction::RequestTermination);
}

/// A user who interrupts again is still stuck at their prompt, so stop waiting on an event loop
/// that may never service the request.
#[test]
fn repeat_interrupts_force_an_exit() {
    assert_eq!(interrupt_action(1), InterruptAction::ForceExit);
    assert_eq!(interrupt_action(7), InterruptAction::ForceExit);
}
