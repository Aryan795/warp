use super::*;

fn runtime() -> TmuxRuntime {
    TmuxRuntime {
        id: TmuxInstanceId::new(),
        inner: Mutex::new(Inner {
            gateway_window: None,
            presentation_window: None,
            panes: HashMap::new(),
            buffers: HashMap::new(),
            pending_captures: VecDeque::new(),
            pending_client_events: Vec::new(),
            pending_client_event_bytes: 0,
            pane_bootstrap: HashMap::new(),
            pending_bootstrap_panes: VecDeque::new(),
            bootstrap_injects: HashMap::new(),
            shell_type: None,
            app_bind_deadline: None,
            presentation_ready: false,
            unregistered_bytes: 0,
        }),
        applying: AtomicBool::new(false),
    }
}

#[test]
fn applying_flag_is_scoped() {
    let runtime = runtime();
    assert!(!runtime.is_applying());
    let value = runtime.with_applying(|| {
        assert!(runtime.is_applying());
        7
    });
    assert_eq!(value, 7);
    assert!(!runtime.is_applying());
}

#[test]
fn capture_requests_are_fifo() {
    let runtime = runtime();
    runtime.note_capture("%4");
    runtime.note_capture("%7");
    assert_eq!(runtime.take_capture().as_deref(), Some("%4"));
    assert_eq!(runtime.take_capture().as_deref(), Some("%7"));
    assert_eq!(runtime.take_capture(), None);
}

#[test]
fn unregistered_pane_output_keeps_leading_bootstrap_bytes() {
    let runtime = runtime();
    assert!(runtime.deliver_output(&PaneId::from("%0"), b"HOOK"));
    assert!(runtime.deliver_output(&PaneId::from("%0"), &vec![b'x'; MAX_BUFFERED_PANE_BYTES]));
    let buffered = runtime.buffered_output("%0").expect("buffered");
    assert!(buffered.starts_with(b"HOOK"));
    assert_eq!(buffered.len(), MAX_BUFFERED_PANE_BYTES);
}

#[test]
fn unregistered_pane_count_cap_clears_and_rolls_back() {
    let runtime = runtime();
    for index in 0..MAX_UNREGISTERED_PANE_COUNT {
        let pane = format!("%{index}");
        assert!(
            runtime.deliver_output(&PaneId::from(pane.as_str()), b"x"),
            "pane {pane} should fit under the count cap"
        );
    }
    assert!(!runtime.deliver_output(&PaneId::from("%64"), b"overflow"));
    for index in 0..=MAX_UNREGISTERED_PANE_COUNT {
        let pane = format!("%{index}");
        assert_eq!(runtime.buffered_output(&pane), None);
    }
}

#[test]
fn unregistered_pane_aggregate_byte_cap_clears_and_rolls_back() {
    let runtime = runtime();
    let chunk = vec![b'y'; MAX_BUFFERED_PANE_BYTES];
    let fitting = MAX_UNREGISTERED_PANE_BYTES / MAX_BUFFERED_PANE_BYTES;
    for index in 0..fitting {
        let pane = format!("%{index}");
        assert!(runtime.deliver_output(&PaneId::from(pane.as_str()), &chunk));
    }
    let overflow_pane = format!("%{fitting}");
    assert!(!runtime.deliver_output(&PaneId::from(overflow_pane.as_str()), &chunk));
    for index in 0..=fitting {
        let pane = format!("%{index}");
        assert_eq!(runtime.buffered_output(&pane), None);
    }
}

#[test]
fn two_instances_do_not_cross_pane_output() {
    let a = runtime();
    let b = runtime();
    a.deliver_output(&PaneId::from("%0"), b"from-a");
    b.deliver_output(&PaneId::from("%0"), b"from-b");
    assert_eq!(
        a.buffered_output("%0").as_deref(),
        Some(b"from-a".as_slice())
    );
    assert_eq!(
        b.buffered_output("%0").as_deref(),
        Some(b"from-b".as_slice())
    );
}

#[test]
fn destructive_clear_is_instance_scoped() {
    let a = runtime();
    let b = runtime();
    a.note_capture("%0");
    b.note_capture("%0");
    a.deliver_output(&PaneId::from("%0"), b"keep-out-of-b");
    a.unregister();
    assert_eq!(a.take_capture(), None);
    assert_eq!(a.buffered_output("%0"), None);
    assert_eq!(b.take_capture().as_deref(), Some("%0"));
}

#[test]
fn close_looks_up_runtime_by_instance_id_after_control_exit() {
    let runtime = TmuxRuntime::new();
    let id = runtime.id();
    assert!(TmuxRuntime::for_id(id).is_some());
    runtime.unregister();
    assert!(TmuxRuntime::for_id(id).is_none());
}

#[test]
fn open_binds_the_emitting_instance_not_another_session() {
    let emitting = TmuxRuntime::new();
    let other = TmuxRuntime::new();
    let chosen = TmuxRuntime::for_id(emitting.id()).expect("emitting runtime");
    assert_eq!(chosen.id(), emitting.id());
    assert_ne!(chosen.id(), other.id());
    emitting.unregister();
    other.unregister();
}

#[test]
fn applying_on_one_instance_does_not_block_the_other() {
    let a = runtime();
    let b = runtime();
    a.with_applying(|| {
        assert!(a.is_applying());
        assert!(!b.is_applying());
    });
    assert!(!a.is_applying());
    assert!(!b.is_applying());
}

#[test]
fn pending_client_events_overflow_clears_and_reports_failure() {
    let runtime = runtime();
    let event = TmuxClientEvent::LayoutChange {
        window_id: "@0".to_owned(),
        layout: "x".repeat(MAX_PENDING_CLIENT_EVENT_BYTES + 1),
        visible_layout: None,
        flags: None,
    };
    assert!(!runtime.buffer_client_events(&[event]));
    assert!(runtime.take_client_events().is_empty());
}

#[test]
fn repeated_zero_payload_events_overflow_the_pending_queue() {
    let runtime = runtime();
    let event = TmuxClientEvent::PresentationUnready;
    let fit = MAX_PENDING_CLIENT_EVENT_BYTES / PENDING_CLIENT_EVENT_OVERHEAD;
    let batch: Vec<_> = (0..fit).map(|_| event.clone()).collect();
    assert!(runtime.buffer_client_events(&batch));
    assert!(!runtime.buffer_client_events(&[event]));
    assert!(runtime.take_client_events().is_empty());
}

#[test]
fn pane_bootstrap_queues_until_authoritative_shell_type() {
    let runtime = runtime();
    assert_eq!(runtime.begin_pane_bootstrap("%0"), None);
    assert_eq!(runtime.shell_type(), None);
    let ready = runtime.set_authoritative_shell_type(ShellType::Bash);
    assert_eq!(ready, vec!["%0".to_owned()]);
    assert_eq!(runtime.shell_type(), Some(ShellType::Bash));
    assert!(runtime.pane_bootstrap_in_flight("%0"));
    assert_eq!(runtime.begin_pane_bootstrap("%0"), None);
    assert_eq!(runtime.begin_pane_bootstrap("%1"), Some(ShellType::Bash));
    assert_eq!(runtime.begin_pane_bootstrap("%1"), None);
}

#[test]
fn local_launch_shell_is_not_used_when_remote_init_shell_differs() {
    let runtime = runtime();
    runtime.set_authoritative_shell_type(ShellType::Zsh);
    let ready = runtime.set_authoritative_shell_type(ShellType::Bash);
    assert!(ready.is_empty());
    assert_eq!(runtime.begin_pane_bootstrap("%0"), Some(ShellType::Bash));
    assert_eq!(runtime.begin_pane_bootstrap("%1"), Some(ShellType::Bash));
    assert_ne!(ShellType::Zsh, ShellType::Bash);
}

#[test]
fn app_bind_deadline_is_not_cleared_until_presentation_ready() {
    let runtime = runtime();
    runtime.start_app_bind_deadline();
    assert!(!runtime.is_presentation_ready());
    assert!(!runtime.app_bind_deadline_elapsed(instant::Instant::now()));
    let later = instant::Instant::now() + APP_BIND_TIMEOUT + std::time::Duration::from_secs(1);
    assert!(runtime.app_bind_deadline_elapsed(later));
    runtime.mark_presentation_ready();
    assert!(runtime.is_presentation_ready());
    assert!(!runtime.app_bind_deadline_elapsed(later));
}

#[test]
fn pane_bootstrap_ignores_repeated_bind_before_ready() {
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    assert_eq!(runtime.begin_pane_bootstrap("%0"), Some(ShellType::Zsh));
    assert_eq!(runtime.bootstrap_inject_count("%0"), 1);
    assert_eq!(runtime.begin_pane_bootstrap("%0"), None);
    assert_eq!(runtime.begin_pane_bootstrap("%0"), None);
    assert_eq!(runtime.begin_pane_bootstrap("%0"), None);
    assert_eq!(runtime.bootstrap_inject_count("%0"), 1);
    assert!(runtime.pane_bootstrap_in_flight("%0"));
    assert!(runtime.mark_pane_bootstrap_ready("%0"));
    assert!(runtime.pane_bootstrap_ready("%0"));
    assert_eq!(runtime.begin_pane_bootstrap("%0"), None);
    assert_eq!(runtime.bootstrap_inject_count("%0"), 1);
}

#[test]
fn pane_bootstrap_timeout_allows_exactly_one_retry() {
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    let start = instant::Instant::now();
    assert_eq!(
        runtime.begin_pane_bootstrap_at("%0", start),
        Some(ShellType::Zsh)
    );
    let timed_out = start + BOOTSTRAP_INFLIGHT_TIMEOUT + std::time::Duration::from_millis(1);
    assert_eq!(
        runtime.begin_pane_bootstrap_at("%0", timed_out),
        Some(ShellType::Zsh)
    );
    assert_eq!(runtime.bootstrap_inject_count("%0"), 2);
    let later = timed_out + BOOTSTRAP_INFLIGHT_TIMEOUT + std::time::Duration::from_secs(1);
    assert_eq!(runtime.begin_pane_bootstrap_at("%0", later), None);
    assert_eq!(runtime.bootstrap_inject_count("%0"), 2);
}

#[test]
fn pane_bootstrap_stale_generation_does_not_mark_ready() {
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    let start = instant::Instant::now();
    assert_eq!(
        runtime.begin_pane_bootstrap_at("%0", start),
        Some(ShellType::Zsh)
    );
    let stale = runtime.pane_bootstrap_generation("%0").expect("generation");
    let timed_out = start + BOOTSTRAP_INFLIGHT_TIMEOUT + std::time::Duration::from_millis(1);
    assert_eq!(
        runtime.begin_pane_bootstrap_at("%0", timed_out),
        Some(ShellType::Zsh)
    );
    let live = runtime.pane_bootstrap_generation("%0").expect("live gen");
    assert_ne!(stale, live);
    assert!(!runtime.mark_pane_bootstrap_ready_generation("%0", Some(stale)));
    assert!(runtime.pane_bootstrap_in_flight("%0"));
    assert!(runtime.mark_pane_bootstrap_ready_generation("%0", Some(live)));
    assert!(runtime.pane_bootstrap_ready("%0"));
}
