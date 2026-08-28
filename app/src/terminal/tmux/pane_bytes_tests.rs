use crate::terminal::tmux::parser::{CONTROL_MODE_DCS, ControlEvent, ControlModeParser, PaneId};

#[test]
fn protocol_chatter_is_not_returned_as_pane_bytes() {
    let mut parser = ControlModeParser::new();
    let mut bytes = CONTROL_MODE_DCS.to_vec();
    bytes.extend_from_slice(
        b"%begin 1 1\nprotocol-reply\n%end 1 1\n%output %0 hello\n%session-changed $1\n",
    );
    let events = parser.push(&bytes);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ControlEvent::EnteredControlMode))
    );
    let pane_events: Vec<_> = events
        .into_iter()
        .filter_map(|event| match event {
            ControlEvent::PaneOutput { pane_id, bytes } => Some((pane_id, bytes)),
            _ => None,
        })
        .collect();
    assert_eq!(pane_events, vec![(PaneId::from("%0"), b"hello".to_vec())]);
}
