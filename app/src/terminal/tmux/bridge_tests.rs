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
            bootstrap_stage_count: HashMap::new(),
            bootstrap_script_count: HashMap::new(),
            next_generation: 0,
            tracked_control_pane: None,
            early_init_shell: HashMap::new(),
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

fn sid(n: u64) -> warp_core::SessionId {
    n.into()
}

#[test]
fn pane_bootstrap_queues_until_authoritative_shell_type() {
    let runtime = runtime();
    assert!(runtime.begin_pane_bootstrap("%0", sid(1)).is_none());
    assert_eq!(runtime.shell_type(), None);
    let ready = runtime.set_authoritative_shell_type(ShellType::Bash);
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].pane_id, "%0");
    assert_eq!(ready[0].shell_type, ShellType::Bash);
    assert_eq!(runtime.shell_type(), Some(ShellType::Bash));
    assert_eq!(
        runtime.pane_bootstrap_state("%0"),
        PaneBootstrapState::Staging
    );
    assert!(runtime.begin_pane_bootstrap("%0", sid(1)).is_none());
    assert_eq!(
        runtime
            .begin_pane_bootstrap("%1", sid(2))
            .map(|c| c.shell_type),
        Some(ShellType::Bash)
    );
    assert!(runtime.begin_pane_bootstrap("%1", sid(3)).is_none());
}

#[test]
fn local_launch_shell_is_not_used_when_remote_init_shell_differs() {
    let runtime = runtime();
    runtime.set_authoritative_shell_type(ShellType::Zsh);
    let ready = runtime.set_authoritative_shell_type(ShellType::Bash);
    assert!(ready.is_empty());
    assert_eq!(
        runtime
            .begin_pane_bootstrap("%0", sid(1))
            .map(|c| c.shell_type),
        Some(ShellType::Bash)
    );
    assert_eq!(
        runtime
            .begin_pane_bootstrap("%1", sid(2))
            .map(|c| c.shell_type),
        Some(ShellType::Bash)
    );
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
fn split_pane_stages_once_then_correlated_hook_bootstraps_ready() {
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    let session = sid(7);
    let claim = runtime
        .begin_pane_bootstrap("%1", session)
        .expect("stage split pane");
    assert_eq!(claim.session_id, session);
    assert_eq!(runtime.bootstrap_stage_count("%1"), 1);
    assert_eq!(runtime.bootstrap_script_count("%1"), 0);
    assert!(runtime.begin_pane_bootstrap("%1", sid(8)).is_none());
    assert_eq!(runtime.bootstrap_stage_count("%1"), 1);
    assert!(runtime.on_init_shell("%1", sid(8)).is_none());
    assert_eq!(runtime.on_init_shell("%1", session), Some(ShellType::Zsh));
    assert!(runtime.pane_bootstrap_ready("%1"));
    assert_eq!(runtime.bootstrap_script_count("%1"), 1);
    assert!(runtime.on_init_shell("%1", session).is_none());
    assert_eq!(runtime.bootstrap_script_count("%1"), 1);
    assert!(runtime.begin_pane_bootstrap("%1", session).is_none());
}

#[test]
fn dropped_first_injection_schedules_one_retry() {
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    let claim = runtime.begin_pane_bootstrap("%0", sid(1)).expect("stage");
    let armed = runtime.arm_bootstrap_timeout("%0").expect("arm timer");
    assert_eq!(armed.0, claim.generation);
    assert_eq!(armed.1, BOOTSTRAP_INFLIGHT_TIMEOUT);
    match runtime.handle_bootstrap_timeout("%0", claim.generation) {
        BootstrapTimeoutResult::Retry(retry) => {
            assert_ne!(retry.session_id, sid(1));
            assert_eq!(retry.retired_session_id, Some(sid(1)));
            assert_eq!(retry.generation, claim.generation + 1);
        }
        other => panic!("expected retry, got {other:?}"),
    }
    assert_eq!(runtime.bootstrap_stage_count("%0"), 2);
    assert_eq!(
        runtime.pane_bootstrap_state("%0"),
        PaneBootstrapState::Staging
    );
}

#[test]
fn dropped_retry_leaves_recoverable_failed_state() {
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    let claim = runtime.begin_pane_bootstrap("%0", sid(1)).expect("stage");
    let BootstrapTimeoutResult::Retry(retry) =
        runtime.handle_bootstrap_timeout("%0", claim.generation)
    else {
        panic!("expected retry");
    };
    assert!(matches!(
        runtime.handle_bootstrap_timeout("%0", retry.generation),
        BootstrapTimeoutResult::Failed
    ));
    assert!(runtime.pane_bootstrap_failed("%0"));
    let again = runtime
        .begin_pane_bootstrap("%0", sid(9))
        .expect("recover from failed");
    assert_eq!(again.session_id, sid(9));
    assert_eq!(
        runtime.pane_bootstrap_state("%0"),
        PaneBootstrapState::Staging
    );
}

#[test]
fn pane_removal_invalidates_stale_retry_and_allows_reuse() {
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    let claim = runtime.begin_pane_bootstrap("%0", sid(1)).expect("stage");
    runtime.arm_bootstrap_timeout("%0");
    runtime.unregister_pane("%0");
    assert_eq!(
        runtime.pane_bootstrap_state("%0"),
        PaneBootstrapState::Unsent
    );
    assert!(matches!(
        runtime.handle_bootstrap_timeout("%0", claim.generation),
        BootstrapTimeoutResult::Stale
    ));
    assert_eq!(runtime.bootstrap_stage_count("%0"), 0);
    let reused = runtime
        .begin_pane_bootstrap("%0", sid(2))
        .expect("reuse after removal");
    assert_eq!(reused.session_id, sid(2));
    assert_ne!(reused.generation, claim.generation);
    assert!(matches!(
        runtime.handle_bootstrap_timeout("%0", claim.generation),
        BootstrapTimeoutResult::Stale
    ));
    assert_eq!(runtime.bootstrap_stage_count("%0"), 1);
}

#[test]
fn correlated_hook_cancels_timeout() {
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    let claim = runtime.begin_pane_bootstrap("%0", sid(1)).expect("stage");
    runtime.arm_bootstrap_timeout("%0");
    assert_eq!(runtime.on_init_shell("%0", sid(1)), Some(ShellType::Zsh));
    assert!(runtime.pane_bootstrap_ready("%0"));
    assert!(matches!(
        runtime.handle_bootstrap_timeout("%0", claim.generation),
        BootstrapTimeoutResult::Stale
    ));
    assert!(runtime.arm_bootstrap_timeout("%0").is_none());
}

#[test]
fn delayed_first_hook_is_ignored_after_retry_session_rotates() {
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    let first = runtime.begin_pane_bootstrap("%0", sid(1)).expect("stage");
    let BootstrapTimeoutResult::Retry(retry) =
        runtime.handle_bootstrap_timeout("%0", first.generation)
    else {
        panic!("expected retry");
    };
    assert!(runtime.on_init_shell("%0", first.session_id).is_none());
    assert_eq!(runtime.bootstrap_script_count("%0"), 0);
    assert_eq!(
        runtime.on_init_shell("%0", retry.session_id),
        Some(ShellType::Zsh)
    );
    assert!(runtime.pane_bootstrap_ready("%0"));
    assert_eq!(runtime.bootstrap_script_count("%0"), 1);
}

#[test]
fn early_init_shell_before_bind_reaches_ready_once() {
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    runtime.note_early_init_shell("%0", sid(3));
    let claim = runtime.begin_pane_bootstrap("%0", sid(3)).expect("stage");
    assert_eq!(claim.generation, 1);
    assert_eq!(
        runtime.complete_if_early_init_shell("%0"),
        Some(ShellType::Zsh)
    );
    assert!(runtime.pane_bootstrap_ready("%0"));
    assert_eq!(runtime.bootstrap_stage_count("%0"), 1);
    assert_eq!(runtime.bootstrap_script_count("%0"), 1);
    assert!(runtime.complete_if_early_init_shell("%0").is_none());
    assert!(runtime.begin_pane_bootstrap("%0", sid(3)).is_none());
}

#[test]
fn queued_timeout_failed_uses_presentation_unready() {
    assert_eq!(
        TmuxRuntime::bootstrap_failed_client_event(),
        TmuxClientEvent::PresentationUnready
    );
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    let claim = runtime.begin_pane_bootstrap("%0", sid(1)).expect("stage");
    let BootstrapTimeoutResult::Retry(retry) =
        runtime.handle_bootstrap_timeout("%0", claim.generation)
    else {
        panic!("expected retry");
    };
    assert!(matches!(
        runtime.handle_bootstrap_timeout("%0", retry.generation),
        BootstrapTimeoutResult::Failed
    ));
    assert_eq!(
        TmuxRuntime::bootstrap_failed_client_event(),
        TmuxClientEvent::PresentationUnready
    );
}

#[test]
fn tracked_control_pane_hands_off_early_init_shell() {
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    runtime.note_tracked_control_pane("%0");
    assert_eq!(runtime.tracked_control_pane().as_deref(), Some("%0"));
    runtime.note_early_init_shell("%0", sid(4));
    runtime.begin_pane_bootstrap("%0", sid(4)).expect("stage");
    assert_eq!(
        runtime.complete_if_early_init_shell("%0"),
        Some(ShellType::Zsh)
    );
    assert!(runtime.pane_bootstrap_ready("%0"));
}

#[test]
fn late_tracked_init_shell_completes_staged_pane_without_retry() {
    use crate::terminal::model::ansi::Handler as _;
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    runtime.note_tracked_control_pane("%0");
    let model = Arc::new(FairMutex::new(crate::terminal::TerminalModel::mock(
        None, None,
    )));
    runtime.register_pane("%0", model.clone());
    let minted = sid(99);
    let retained = sid(11);
    let claim = runtime
        .begin_pane_bootstrap("%0", minted)
        .expect("stage first");
    assert_eq!(claim.session_id, minted);
    runtime.apply_claim_session(&claim);
    let armed = runtime.arm_bootstrap_timeout("%0").expect("arm timer");
    assert_eq!(armed.0, claim.generation);
    assert_eq!(
        runtime.note_early_init_shell("%0", retained),
        Some(ShellType::Zsh)
    );
    assert!(runtime.pane_bootstrap_ready("%0"));
    assert_eq!(runtime.bootstrap_script_count("%0"), 1);
    assert_eq!(runtime.pane_bootstrap_session_id("%0"), Some(retained));
    assert!(!model.lock().is_registered_session(minted));
    assert!(model.lock().is_registered_session(retained));
    assert!(matches!(
        runtime.handle_bootstrap_timeout("%0", claim.generation),
        BootstrapTimeoutResult::Stale
    ));
    assert!(runtime.begin_pane_bootstrap("%0", sid(3)).is_none());
    assert!(runtime.note_early_init_shell("%0", retained).is_none());
    assert_eq!(runtime.bootstrap_script_count("%0"), 1);
    assert!(runtime.complete_if_early_init_shell("%0").is_none());
}

#[test]
fn late_init_shell_after_pane_removal_does_not_complete() {
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    runtime.note_tracked_control_pane("%0");
    let claim = runtime.begin_pane_bootstrap("%0", sid(1)).expect("stage");
    runtime.unregister_pane("%0");
    assert!(runtime.note_early_init_shell("%0", sid(11)).is_none());
    assert_eq!(
        runtime.pane_bootstrap_state("%0"),
        PaneBootstrapState::Unsent
    );
    assert_eq!(runtime.early_init_session_id("%0"), Some(sid(11)));
    assert!(matches!(
        runtime.handle_bootstrap_timeout("%0", claim.generation),
        BootstrapTimeoutResult::Stale
    ));
}

#[test]
fn unbind_retires_session_id_from_presentation_model() {
    use crate::terminal::model::ansi::Handler as _;
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    let model = Arc::new(FairMutex::new(crate::terminal::TerminalModel::mock(
        None, None,
    )));
    runtime.register_pane("%0", model.clone());
    let claim = runtime.begin_pane_bootstrap("%0", sid(5)).expect("stage");
    runtime.apply_claim_session(&claim);
    assert!(model.lock().is_registered_session(sid(5)));
    assert_eq!(runtime.on_init_shell("%0", sid(5)), Some(ShellType::Zsh));
    runtime.unregister_pane("%0");
    assert!(!model.lock().is_registered_session(sid(5)));
    let reused = Arc::new(FairMutex::new(crate::terminal::TerminalModel::mock(
        None, None,
    )));
    runtime.register_pane("%0", reused.clone());
    let next = runtime.begin_pane_bootstrap("%0", sid(6)).expect("restage");
    runtime.apply_claim_session(&next);
    assert!(!reused.lock().is_registered_session(sid(5)));
    assert!(reused.lock().is_registered_session(sid(6)));
    assert!(runtime.on_init_shell("%0", sid(5)).is_none());
}

fn assert_runtime_maps_cleared(runtime: &TmuxRuntime, pane_id: &str) {
    assert_eq!(
        runtime.pane_bootstrap_state(pane_id),
        PaneBootstrapState::Unsent
    );
    assert!(runtime.pane_model(pane_id).is_none());
    assert_eq!(runtime.buffered_output(pane_id), None);
    assert_eq!(runtime.take_capture(), None);
    assert_eq!(runtime.bootstrap_stage_count(pane_id), 0);
    assert_eq!(runtime.bootstrap_script_count(pane_id), 0);
    assert!(runtime.tracked_control_pane().is_none());
    assert!(runtime.take_early_init_shell(pane_id).is_none());
}

#[test]
fn runtime_unregister_retires_ready_session_and_rejects_delayed_hook() {
    use crate::terminal::model::ansi::Handler as _;
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    let model = Arc::new(FairMutex::new(crate::terminal::TerminalModel::mock(
        None, None,
    )));
    runtime.register_pane("%0", model.clone());
    let claim = runtime.begin_pane_bootstrap("%0", sid(5)).expect("stage");
    runtime.apply_claim_session(&claim);
    assert_eq!(runtime.on_init_shell("%0", sid(5)), Some(ShellType::Zsh));
    runtime.note_capture("%0");
    runtime.unregister();
    assert!(!model.lock().is_registered_session(sid(5)));
    assert!(runtime.on_init_shell("%0", sid(5)).is_none());
    assert_runtime_maps_cleared(&runtime, "%0");
}

#[test]
fn runtime_unregister_retires_failed_session_and_rejects_delayed_hook() {
    use crate::terminal::model::ansi::Handler as _;
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    let model = Arc::new(FairMutex::new(crate::terminal::TerminalModel::mock(
        None, None,
    )));
    runtime.register_pane("%0", model.clone());
    let claim = runtime.begin_pane_bootstrap("%0", sid(5)).expect("stage");
    runtime.apply_claim_session(&claim);
    let BootstrapTimeoutResult::Retry(retry) =
        runtime.handle_bootstrap_timeout("%0", claim.generation)
    else {
        panic!("expected retry");
    };
    runtime.apply_claim_session(&retry);
    assert!(matches!(
        runtime.handle_bootstrap_timeout("%0", retry.generation),
        BootstrapTimeoutResult::Failed
    ));
    assert!(runtime.pane_bootstrap_failed("%0"));
    runtime.unregister();
    assert!(!model.lock().is_registered_session(sid(5)));
    assert!(!model.lock().is_registered_session(retry.session_id));
    assert!(runtime.on_init_shell("%0", retry.session_id).is_none());
    assert_runtime_maps_cleared(&runtime, "%0");
}

#[test]
fn runtime_unregister_without_pane_model_clears_bootstrap() {
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    runtime.begin_pane_bootstrap("%0", sid(1)).expect("stage");
    runtime.unregister();
    assert_runtime_maps_cleared(&runtime, "%0");
}

#[test]
fn bind_reuses_retained_early_session_id_not_a_random_id() {
    let runtime = runtime();
    runtime.note_shell_type(ShellType::Zsh);
    let retained = sid(11);
    let would_be_random = sid(99);
    runtime.note_early_init_shell("%0", retained);
    assert_eq!(runtime.early_init_session_id("%0"), Some(retained));
    let chosen = runtime
        .early_init_session_id("%0")
        .unwrap_or(would_be_random);
    assert_eq!(chosen, retained);
    assert_ne!(chosen, would_be_random);
    let claim = runtime
        .begin_pane_bootstrap("%0", would_be_random)
        .expect("stage with retained id");
    assert_eq!(claim.session_id, retained);
    assert_ne!(claim.session_id, would_be_random);
    assert_eq!(
        runtime.complete_if_early_init_shell("%0"),
        Some(ShellType::Zsh)
    );
    assert!(runtime.pane_bootstrap_ready("%0"));
}
