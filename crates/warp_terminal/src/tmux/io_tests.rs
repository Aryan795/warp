use std::borrow::Cow;

use super::{TmuxFeedItem, TmuxIoState, TmuxPhaseKind, is_tmux_cc_start};
use crate::tmux::parser::{CONTROL_MODE_DCS, PaneId, WindowId};

fn start_command() -> Cow<'static, [u8]> {
    Cow::Borrowed(b"tmux -CC new-session -A -s warp -n warp -x 80 -y 24\n")
}

#[test]
fn tmux_cc_start_is_detected() {
    assert!(is_tmux_cc_start(b"tmux -CC new-session -A -s warp\n"));
    assert!(is_tmux_cc_start(b"  tmux -CC\n"));
    assert!(!is_tmux_cc_start(b"echo tmux -CC\n"));
}

#[test]
fn inputs_after_start_command_are_held_until_dcs() {
    let mut io = TmuxIoState::new();
    let written = io.enqueue_input(start_command());
    assert_eq!(written, vec![start_command()]);
    assert_eq!(io.phase(), TmuxPhaseKind::StartPending);

    assert!(io.enqueue_input(Cow::Borrowed(b"hello")).is_empty());
    let items = io.feed(CONTROL_MODE_DCS);
    assert!(
        items
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::EnteredControl { .. }))
    );
    assert_eq!(io.phase(), TmuxPhaseKind::InControl);
    assert!(io.focused_pane().is_none());
}

#[test]
fn resize_during_handshake_issues_refresh_client_on_dcs() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    assert!(io.enqueue_resize(100, 40).is_none());
    let items = io.feed(CONTROL_MODE_DCS);
    match &items[0] {
        TmuxFeedItem::EnteredControl { refresh_client } => {
            assert_eq!(
                refresh_client.as_deref(),
                Some("refresh-client -C 100x40\n")
            );
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn pending_input_is_replayed_raw_when_entry_fails() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.enqueue_input(Cow::Borrowed(b"typed-before-dcs"));
    let items = io.feed(b"tmux: command not found\n");
    let replay = items.iter().find_map(|item| match item {
        TmuxFeedItem::Exited { replay } => Some(replay.clone()),
        _ => None,
    });
    assert_eq!(replay, Some(vec![Cow::Borrowed(&b"typed-before-dcs"[..])]));
    assert_eq!(io.phase(), TmuxPhaseKind::Inactive);
}

#[test]
fn benign_startup_output_does_not_fail_start_pending() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.enqueue_input(Cow::Borrowed(b"held"));
    let items = io.feed(b"starting tmux 3.4\n");
    assert_eq!(io.phase(), TmuxPhaseKind::StartPending);
    assert!(
        items
            .iter()
            .all(|item| !matches!(item, TmuxFeedItem::Exited { .. }))
    );
    let items = io.feed(CONTROL_MODE_DCS);
    assert!(matches!(items[0], TmuxFeedItem::EnteredControl { .. }));
    assert_eq!(io.phase(), TmuxPhaseKind::InControl);
}

#[test]
fn start_pending_timeout_replays_queued_input() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.enqueue_input(Cow::Borrowed(b"typed-while-waiting"));
    let later = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let items = io.check_start_timeout(later);
    let replay = items.iter().find_map(|item| match item {
        TmuxFeedItem::Exited { replay } => Some(replay.clone()),
        _ => None,
    });
    assert_eq!(
        replay,
        Some(vec![Cow::Borrowed(&b"typed-while-waiting"[..])])
    );
    assert_eq!(io.phase(), TmuxPhaseKind::Inactive);
}

#[test]
fn command_not_found_interleaved_with_dcs_does_not_enter_control() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.enqueue_input(Cow::Borrowed(b"held"));
    let mut bytes = b"bash: tmux: command not found\n".to_vec();
    bytes.extend_from_slice(CONTROL_MODE_DCS);
    bytes.extend_from_slice(b"%output %0 leaked\n");
    let items = io.feed(&bytes);
    assert_eq!(io.phase(), TmuxPhaseKind::Inactive);
    assert!(
        items
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::Exited { .. }))
    );
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::EnteredControl { .. }))
    );
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::PaneOutput { .. }))
    );
}

#[test]
fn overflow_does_not_replay_while_tmux_still_in_control_mode() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.enqueue_input(Cow::Borrowed(b"typed-in-control"));
    let overflow = vec![b'x'; 1_048_577];
    let items = io.feed(&overflow);
    assert_eq!(io.phase(), TmuxPhaseKind::OverflowRecovering);
    assert!(items.iter().any(|item| match item {
        TmuxFeedItem::OverflowRecovering { detach } =>
            detach.as_ref() as &[u8] == b"detach-client\n",
        _ => false,
    }));
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::Exited { .. }))
    );
    assert!(io.enqueue_input(Cow::Borrowed(b"more")).is_empty());
}

#[test]
fn valid_notification_after_overflow_does_not_reach_shell() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.feed(&vec![b'x'; 1_048_577]);
    let items = io.feed(b"%output %0 leaked\n%exit\n$ ");
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::PaneOutput { .. }))
    );
    assert!(!items.iter().any(|item| matches!(
        item,
        TmuxFeedItem::Shell(bytes) if bytes.windows(b"%output".len()).any(|w| w == b"%output")
    )));
    let replay = items.iter().find_map(|item| match item {
        TmuxFeedItem::Exited { replay } => Some(replay.clone()),
        _ => None,
    });
    assert!(replay.is_some());
    assert_eq!(io.phase(), TmuxPhaseKind::Inactive);
}

#[test]
fn window_pane_changed_selects_focus_not_first_output() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.enqueue_input(Cow::Borrowed(b"keys"));
    let mut bytes = b"%output %0 one\n%output %1 two\n".to_vec();
    let items = io.feed(&bytes);
    assert!(io.focused_pane().is_none());
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::EncodedPending(_)))
    );

    bytes = b"%window-pane-changed @0 %1\n".to_vec();
    let items = io.feed(&bytes);
    assert_eq!(io.focused_pane().map(PaneId::as_str), Some("%1"));
    assert!(items.iter().any(|item| matches!(
        item,
        TmuxFeedItem::EncodedPending(encoded) if encoded.starts_with(b"send-keys -t %1")
    )));
}

#[test]
fn reattach_active_pane_is_not_percent_zero() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.feed(b"%window-pane-changed @3 %7\n");
    assert_eq!(io.focused_pane().map(PaneId::as_str), Some("%7"));
    let encoded = io.enqueue_input(Cow::Borrowed(b"x"));
    assert_eq!(encoded.len(), 1);
    assert!(encoded[0].starts_with(b"send-keys -t %7"));
}

#[test]
fn layout_with_one_pane_focuses_that_pane() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.feed(b"%layout-change @0 80x24,0,0,4\n");
    assert_eq!(io.focused_pane().map(PaneId::as_str), Some("%4"));
}

#[test]
fn client_commands_stay_raw_in_control_mode() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.feed(b"%window-pane-changed @0 %1\n");
    let written = io.enqueue_input(Cow::Borrowed(b"split-window -h -t %1\n"));
    assert_eq!(
        written,
        vec![Cow::Borrowed(&b"split-window -h -t %1\n"[..])]
    );
}

#[test]
fn interleaved_output_and_focus_does_not_steal_pending_keys() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.enqueue_input(Cow::Borrowed(b"typed"));
    let items = io.feed(b"%output %9 noise\n%window-pane-changed @2 %3\n");
    assert_eq!(io.focused_pane().map(PaneId::as_str), Some("%3"));
    assert!(items.iter().any(|item| matches!(
        item,
        TmuxFeedItem::EncodedPending(encoded) if encoded.starts_with(b"send-keys -t %3")
    )));
}

#[test]
fn latest_resize_wins_when_interleaved_with_start() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.enqueue_resize(80, 24);
    io.enqueue_resize(120, 40);
    let items = io.feed(CONTROL_MODE_DCS);
    match &items[0] {
        TmuxFeedItem::EnteredControl { refresh_client } => {
            assert_eq!(
                refresh_client.as_deref(),
                Some("refresh-client -C 120x40\n")
            );
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn window_events_are_forwarded() {
    let mut io = TmuxIoState::new();
    io.feed(CONTROL_MODE_DCS);
    let items = io.feed(b"%window-add @2\n");
    assert_eq!(
        items,
        vec![TmuxFeedItem::WindowAdd {
            window_id: WindowId::from("@2")
        }]
    );
}
