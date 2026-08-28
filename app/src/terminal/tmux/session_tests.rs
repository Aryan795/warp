use super::{ClosePlan, ControlClientLoss, PaneRegistry, TmuxViewSlots};
use crate::features::FeatureFlag;
use crate::pane_group::NewTerminalOptions;
use crate::terminal::TerminalModel;
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
        ClosePlan::DetachClient
    );
}

#[test]
fn closing_the_last_pane_detaches_the_client() {
    let mut registry = PaneRegistry::new();
    registry.register(PaneId::from("%0"));
    assert_eq!(
        registry.unregister(&PaneId::from("%0")),
        ClosePlan::DetachClient
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

#[test]
fn gateway_requests_a_presentation_window_once() {
    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_control_mode(true);
    assert!(model.take_tmux_open_presentation());
    assert!(!model.take_tmux_open_presentation());
}

#[test]
fn presentation_models_do_not_open_another_window() {
    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_presentation(true);
    model.set_tmux_control_mode(true);
    assert!(model.is_tmux_presentation());
    assert!(!model.take_tmux_open_presentation());
}

#[test]
fn gateway_exit_requests_presentation_window_close() {
    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_control_mode(true);
    let _ = model.take_tmux_open_presentation();
    model.set_tmux_control_mode(false);
    assert!(model.take_tmux_close_presentation());
    assert!(!model.take_tmux_close_presentation());
}

#[test]
fn default_new_terminal_options_are_not_tmux_owned() {
    assert!(!NewTerminalOptions::default().tmux_presentation);
}

#[test]
fn presentation_pane_id_is_explicit() {
    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_presentation(true);
    assert_eq!(model.tmux_pane_id(), None);
    model.set_tmux_pane_id(Some("%7".to_owned()));
    assert_eq!(model.tmux_pane_id(), Some("%7"));
}

#[test]
fn layout_events_are_queued_until_taken() {
    use crate::terminal::model::terminal_model::TmuxClientEvent;
    let mut model = TerminalModel::mock(None, None);
    model.push_tmux_event(TmuxClientEvent::WindowAdd {
        window_id: "@3".to_owned(),
    });
    let events = model.take_tmux_events();
    assert_eq!(
        events,
        vec![TmuxClientEvent::WindowAdd {
            window_id: "@3".to_owned()
        }]
    );
    assert!(model.take_tmux_events().is_empty());
}

#[test]
fn feature_off_does_not_treat_panes_as_tmux_owned() {
    let _flag = FeatureFlag::TmuxControlPrototype.override_enabled(false);
    let model = TerminalModel::mock(None, None);
    assert!(!model.is_tmux_control_mode());
    assert!(!model.is_tmux_presentation());
    assert!(!NewTerminalOptions::default().tmux_presentation);
}
