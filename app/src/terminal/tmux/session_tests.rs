use super::{ClosePlan, ControlClientLoss, PaneRegistry, TmuxViewSlots};
use crate::terminal::tmux::parser::PaneId;

#[test]
fn output_is_delivered_to_registered_panes_only() {
    let mut registry = PaneRegistry::new();
    registry.register(PaneId::from("%0"));
    registry.register(PaneId::from("%1"));
    assert!(registry.should_deliver_output(&PaneId::from("%0")));
    assert!(registry.should_deliver_output(&PaneId::from("%1")));
    assert!(!registry.should_deliver_output(&PaneId::from("%2")));
}

#[test]
fn first_registered_pane_is_focused_until_select() {
    let mut registry = PaneRegistry::new();
    registry.register(PaneId::from("%0"));
    registry.register(PaneId::from("%1"));
    assert_eq!(registry.focused().map(PaneId::as_str), Some("%0"));
    assert!(registry.focus(&PaneId::from("%1")));
    assert_eq!(registry.focused().map(PaneId::as_str), Some("%1"));
    assert!(!registry.focus(&PaneId::from("%9")));
}

#[test]
fn closing_a_sibling_keeps_the_session() {
    let mut registry = PaneRegistry::new();
    registry.register(PaneId::from("%0"));
    registry.register(PaneId::from("%1"));
    assert_eq!(
        registry.close_plan(&PaneId::from("%1")),
        ClosePlan::KillPane
    );
    assert_eq!(
        registry.unregister(&PaneId::from("%1")),
        ClosePlan::KillPane
    );
    assert!(registry.contains(&PaneId::from("%0")));
    assert_eq!(
        registry.close_plan(&PaneId::from("%0")),
        ClosePlan::TearDownSession
    );
}

#[test]
fn closing_the_last_pane_tears_down_the_session() {
    let mut registry = PaneRegistry::new();
    registry.register(PaneId::from("%0"));
    assert_eq!(
        registry.unregister(&PaneId::from("%0")),
        ClosePlan::TearDownSession
    );
    assert!(registry.is_empty());
}

#[test]
fn two_pane_ids_materialize_two_view_slots() {
    let mut views = TmuxViewSlots::default();
    views.deliver(PaneId::from("%0"), b"one");
    views.deliver(PaneId::from("%1"), b"two");
    views.deliver(PaneId::from("%0"), b"+");
    assert_eq!(views.view_count(), 2);
    assert_eq!(views.output(&PaneId::from("%0")), Some(b"one+".as_slice()));
    assert_eq!(views.output(&PaneId::from("%1")), Some(b"two".as_slice()));
}

#[test]
fn transport_eof_never_kills_the_session() {
    assert_eq!(
        ControlClientLoss::TransportEof.close_plan(true),
        ClosePlan::DetachClient
    );
    assert_eq!(
        ControlClientLoss::ExplicitClose.close_plan(true),
        ClosePlan::DetachClient
    );
    assert_eq!(
        ControlClientLoss::ExplicitClose.close_plan(false),
        ClosePlan::KillPane
    );
}

#[test]
fn unknown_pane_close_is_a_no_op() {
    let registry = PaneRegistry::new();
    assert_eq!(
        registry.close_plan(&PaneId::from("%0")),
        ClosePlan::UnknownPane
    );
}
