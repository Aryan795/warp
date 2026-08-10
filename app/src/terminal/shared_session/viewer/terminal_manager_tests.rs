//! Regression tests for the viewer `TerminalManager`'s `on_view_detached`
//! discriminator and the OVM-teardown helper.
//!
//! Before the fix, closing a viewer pane (tab close / split-pane close) did
//! not flow through any of the network-event paths
//! (`SessionEnded` / `ViewerRemoved` / `FailedToReconnect`), so the
//! orchestration viewer model — and its viewer-mode registration on the
//! shared [`OrchestrationEventStreamer`] — leaked until the app exited.
//! `TerminalManager::on_view_detached` now tears down the OVM on
//! `DetachType::Closed`, while deliberately preserving it on
//! `HiddenForClose` (undo-close grace window) and `Moved`.

use std::collections::HashSet;

use async_broadcast::broadcast;
use chrono::Local;
use session_sharing_protocol::common::AgentPromptRequestId;
use warpui::App;

use super::*;
use crate::ai::agent::{AIAgentExchange, AIAgentExchangeId, AIAgentOutputStatus};
use crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer;
use crate::ai::blocklist::{QueuedQueryModel, QueuedQueryOrigin, ResponseStream, ResponseStreamId};
use crate::ai::llms::LLMId;
// Bring the `TerminalManager` trait into scope (named under a different alias
// since the local `TerminalManager` struct shadows it) so the trait method
// `on_view_detached` is callable on the struct.
use crate::terminal::TerminalManager as _;
use crate::terminal::model::session::Sessions;
use crate::terminal::shared_session::viewer::network::Stage;
use crate::test_util::add_window_with_terminal;
use crate::test_util::shared_session_viewer::{
    drain_agent_prompts, reconnecting_stage, sent_agent_prompt, submit_viewer_prompt, viewer_pane,
};
use crate::test_util::terminal::initialize_app_for_terminal_view;
use crate::workspace::ToastStack;

/// Stub UUID used for the orchestrator's `AmbientAgentTaskId`; opaque to
/// the manager.
const PARENT_TASK_ID: &str = "11111111-1111-1111-1111-111111111111";

fn task_id(s: &str) -> AmbientAgentTaskId {
    s.parse().expect("hardcoded task id parses")
}

/// Constructs a viewer `TerminalManager` whose `orchestration_viewer_model`
/// slot is populated with a real OVM registered against the
/// [`OrchestrationEventStreamer`]. The returned `parent_task_id` is the one
/// used to register the OVM, so callers can look it up via
/// [`OrchestrationEventStreamer::viewer_mode_consumer_count_for_test`].
///
/// Deliberately bypasses `TerminalManager::new_internal` / `new_deferred`
/// (which would create a whole ambient-agent view stack with a real
/// `TerminalView::new` instead of `TerminalView::new_for_test`); the
/// `on_view_detached` path only depends on a small subset of the manager's
/// fields, so a struct-literal construction keeps the test focused.
fn build_manager_with_registered_ovm(app: &mut App) -> (TerminalManager, AmbientAgentTaskId) {
    let parent = task_id(PARENT_TASK_ID);

    let terminal_view = add_window_with_terminal(app, None);
    let terminal_view_id = terminal_view.id();

    // Set up the orchestrator placeholder conversation in the shape the
    // viewer model requires (is_viewing_shared_session == true, no parent
    // conversation id, marked active for the view).
    let history = BlocklistAIHistoryModel::handle(app);
    history.update(app, |history, ctx| {
        let id = history.start_new_conversation(terminal_view_id, false, true, false, ctx);
        history.set_viewing_shared_session_for_conversation(id, true);
        history.set_active_conversation_id(id, terminal_view_id, ctx);
    });

    // The OVM registers with the streamer on construction.
    let ovm_handle = app.add_model(|ctx| {
        OrchestrationViewerModel::new(parent, terminal_view_id, terminal_view.downgrade(), ctx)
    });

    // Build the minimal field values the `TerminalManager` struct needs.
    // The network-side fields are left in their `Idle` / `None` defaults
    // so `on_view_detached` short-circuits the live-session teardown
    // branches and only the OVM-teardown branch is exercised.
    let (wakeups_tx, _wakeups_rx) = async_channel::unbounded();
    let (events_tx, events_rx) = async_channel::unbounded();
    let (pty_reads_tx, pty_reads_rx) = broadcast(8);
    let inactive_pty_reads_rx = pty_reads_rx.deactivate();
    let channel_event_proxy = ChannelEventListener::new(wakeups_tx, events_tx, pty_reads_tx);

    let model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    let sessions = app.add_model(|_| Sessions::new_for_test());
    let model_events =
        app.add_model(|ctx| ModelEventDispatcher::new(events_rx, sessions.clone(), ctx));
    let prompt_type =
        app.add_model(|_| PromptType::new_static(vec![], false, WarpPromptSeparator::None));

    let manager = TerminalManager {
        model,
        view: terminal_view,
        _model_events: model_events,
        _inactive_pty_reads_rx: inactive_pty_reads_rx,
        network_state: NetworkState::Idle,
        network_resources: NetworkResources {
            prompt_type,
            channel_event_proxy,
        },
        current_network: Arc::new(FairMutex::new(None)),
        viewer_remote_update_guard: RemoteUpdateGuard::new(),
        outbound_handlers_registered: false,
        orchestration_viewer_model: Arc::new(FairMutex::new(Some(ovm_handle))),
        enable_orchestration_polling: true,
    };
    (manager, parent)
}

#[test]
fn command_execution_request_failed_clears_queued_command_in_flight() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(|_| ToastStack);

        let terminal = add_window_with_terminal(&mut app, None);
        let terminal_view_id = terminal.id();
        let conversation_id =
            BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, ctx| {
                let id = history.start_new_conversation(terminal_view_id, false, false, false, ctx);
                history.set_active_conversation_id(id, terminal_view_id, ctx);
                id
            });
        QueuedQueryModel::handle(&app).update(&mut app, |model, _ctx| {
            model.arm_command_in_flight(conversation_id);
        });

        terminal.update(&mut app, |view, ctx| {
            TerminalManager::handle_command_execution_request_failed(
                view,
                &CommandExecutionFailureReason::StaleBuffer,
                ctx,
            );
        });

        QueuedQueryModel::handle(&app).read(&app, |model, _ctx| {
            assert!(!model.has_command_in_flight(conversation_id));
        });
    });
}
#[test]
fn on_view_detached_closed_clears_orchestration_viewer_model_slot() {
    // Regression: closing a viewer pane must drop the OVM and release its
    // streamer registration so the ancestor SSE can be torn down.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let (manager, parent) = build_manager_with_registered_ovm(&mut app);
        let slot = manager.orchestration_viewer_model.clone();

        // Sanity: OVM registered with the streamer.
        let streamer = OrchestrationEventStreamer::handle(&app);
        streamer.read(&app, |me, _| {
            assert_eq!(
                me.viewer_mode_consumer_count_for_test(parent),
                1,
                "pre-detach: OVM should have a viewer-mode registration on the streamer"
            );
        });
        assert!(
            slot.lock().is_some(),
            "pre-detach: OVM slot should be populated"
        );

        app.update(|ctx| manager.on_view_detached(DetachType::Closed, ctx));

        assert!(
            slot.lock().is_none(),
            "post-detach (Closed): OVM slot should be cleared"
        );
        streamer.read(&app, |me, _| {
            assert_eq!(
                me.viewer_mode_consumer_count_for_test(parent),
                0,
                "post-detach (Closed): streamer's viewer-mode registration count should drop to 0"
            );
        });
    });
}

#[test]
fn on_view_detached_hidden_for_close_keeps_orchestration_viewer_model_alive() {
    // Negative case: HiddenForClose is part of the undo-close grace
    // window. OVM (and the ancestor SSE registration) must stay alive so
    // the pill bar restores seamlessly if the user undoes the close.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let (manager, parent) = build_manager_with_registered_ovm(&mut app);
        let slot = manager.orchestration_viewer_model.clone();

        app.update(|ctx| manager.on_view_detached(DetachType::HiddenForClose, ctx));

        assert!(
            slot.lock().is_some(),
            "HiddenForClose must NOT clear the OVM slot (undo-close grace window)"
        );
        let streamer = OrchestrationEventStreamer::handle(&app);
        streamer.read(&app, |me, _| {
            assert_eq!(
                me.viewer_mode_consumer_count_for_test(parent),
                1,
                "HiddenForClose must NOT unregister from the streamer"
            );
        });
    });
}

#[test]
fn on_view_detached_moved_keeps_orchestration_viewer_model_alive() {
    // Negative case: Moved transfers the `TerminalManager` (and its OVM)
    // to a new pane group. Tearing down the OVM would orphan the pill
    // bar on the moved pane.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let (manager, parent) = build_manager_with_registered_ovm(&mut app);
        let slot = manager.orchestration_viewer_model.clone();

        app.update(|ctx| manager.on_view_detached(DetachType::Moved, ctx));

        assert!(
            slot.lock().is_some(),
            "Moved must NOT clear the OVM slot (the manager is reused in the new pane group)"
        );
        let streamer = OrchestrationEventStreamer::handle(&app);
        streamer.read(&app, |me, _| {
            assert_eq!(
                me.viewer_mode_consumer_count_for_test(parent),
                1,
                "Moved must NOT unregister from the streamer"
            );
        });
    });
}

/// Evaluates the two conditions that make
/// [`BlocklistAIStatusBar::render_warping_indicator_for_latest_exchange`] render `Warping...`
/// (`app/src/ai/blocklist/block/status_bar.rs:787-790`): an in-progress conversation, or an
/// agent-driven active block. The other terms in that gate can only suppress the indicator
/// further, so this disjunction is exactly what an undelivered prompt can wrongly leave true.
fn warping_gate_is_satisfied(
    terminal_view: &ViewHandle<TerminalView>,
    conversation_id: crate::ai::agent::conversation::AIConversationId,
    app: &App,
) -> bool {
    let conversation_in_progress = BlocklistAIHistoryModel::handle(app).read(app, |history, _| {
        history
            .conversation(&conversation_id)
            .is_some_and(|conversation| conversation.status().is_in_progress())
    });
    let agent_drives_active_block = terminal_view.read(app, |view, _| {
        let model = view.model.lock();
        let active_block = model.block_list().active_block();
        active_block.is_agent_in_control() && !active_block.is_agent_blocked()
    });
    conversation_in_progress || agent_drives_active_block
}

/// Registers an in-flight response stream for `conversation_id`, standing in for a turn that is
/// genuinely still streaming when an unrelated prompt fails to send.
///
/// The stream has to be attached on both sides: registered with the controller *and* bound to a
/// streaming exchange on the conversation, because `has_active_stream_for_conversation` only
/// counts a stream the conversation reports it is processing.
fn register_active_stream(
    app: &mut App,
    terminal_view: &ViewHandle<TerminalView>,
    conversation_id: crate::ai::agent::conversation::AIConversationId,
) {
    terminal_view.update(app, |view, ctx| {
        let stream_id = ResponseStreamId::new_for_test();
        let terminal_view_id = view.view_id();
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history
                .conversation_mut(&conversation_id)
                .expect("the pane's conversation exists")
                .append_reassigned_exchange(&stream_id, streaming_exchange(), terminal_view_id, ctx)
                .expect("a streaming exchange appends");
        });
        let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
        view.ai_controller().clone().update(ctx, |controller, ctx| {
            controller.register_mock_stream_for_test(stream_id, conversation_id, stream, ctx);
        });
    });
}

/// A minimal exchange in the streaming state, which is what marks its response stream in flight.
fn streaming_exchange() -> AIAgentExchange {
    AIAgentExchange {
        id: AIAgentExchangeId::new(),
        input: Vec::new(),
        output_status: AIAgentOutputStatus::Streaming { output: None },
        added_message_ids: HashSet::new(),
        start_time: Local::now(),
        finish_time: None,
        time_to_first_token_ms: None,
        working_directory: None,
        model_id: LLMId::from("test-model"),
        request_cost: None,
        coding_model_id: LLMId::from("test-coding-model"),
        cli_agent_model_id: LLMId::from("test-cli-agent-model"),
        computer_use_model_id: LLMId::from("test-computer-use-model"),
        response_initiator: None,
    }
}

#[test]
fn viewer_prompt_submitted_while_reconnecting_is_preserved_as_an_editable_queue_row() {
    // The reported bug. `SharedSessionStatus` still says `ActiveViewer` while the websocket
    // reconnects, so the prompt was routed to a network that dropped it silently: nothing reached
    // the sharer, nothing was kept, and the input stayed frozen. The prompt must now survive as
    // the user's own queue row.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane(&mut app, reconnecting_stage());
        let (terminal_view, conversation_id, network) = (
            pane.view.clone(),
            pane.conversation_id,
            pane.network.clone(),
        );
        let session_id = network.read(&app, |network, _| network.session_id());

        submit_viewer_prompt(&mut app, &terminal_view, "finish the refactor");

        assert!(
            drain_agent_prompts(&app, &network).is_empty(),
            "a reconnecting network cannot carry the prompt"
        );
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            let queue = queue_model.queue(conversation_id);
            assert_eq!(
                queue.len(),
                1,
                "the undelivered prompt must be preserved as exactly one queue row"
            );
            let row = &queue[0];
            assert_eq!(row.text(), "finish the refactor");
            assert_eq!(row.origin(), QueuedQueryOrigin::DisconnectedViewer);
            assert!(!row.is_locked(), "the row must be editable and deletable");
            let target = row
                .disconnected_viewer_target()
                .expect("the row records where it should be retried");
            assert_eq!(
                target.session_id(),
                session_id,
                "the row must stay pinned to the session it was addressed to"
            );
        });

        // The other half of the reported symptom: with the prompt gone and nothing running, the
        // conversation must stop advertising a turn, or `Warping...` hangs around forever.
        assert!(
            !warping_gate_is_satisfied(&terminal_view, conversation_id, &app),
            "an undelivered prompt must not leave the Warping... gate satisfied"
        );
    });
}

#[test]
fn a_conversation_advertises_a_turn_before_the_prompt_is_even_submitted() {
    // Anchors the precondition the fix depends on: a conversation reports `InProgress` from
    // creation, so the status by itself proves nothing about whether work is running. Without
    // this the reproduction test could pass simply because the gate was never armed.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane(&mut app, reconnecting_stage());
        let (terminal_view, conversation_id) = (pane.view.clone(), pane.conversation_id);

        assert!(
            warping_gate_is_satisfied(&terminal_view, conversation_id, &app),
            "the Warping... gate is expected to be armed before submission"
        );
    });
}

#[test]
fn an_undelivered_prompt_leaves_warping_alone_while_a_stream_is_still_running() {
    // The dangerous over-correction: a prompt that fails to send must not silence the indicator
    // for a turn that is genuinely still streaming. The fallback row is still filed either way.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane(&mut app, reconnecting_stage());
        let (terminal_view, conversation_id) = (pane.view.clone(), pane.conversation_id);
        register_active_stream(&mut app, &terminal_view, conversation_id);

        submit_viewer_prompt(&mut app, &terminal_view, "another thought");

        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert_eq!(
                queue_model.queue(conversation_id).len(),
                1,
                "the undelivered prompt is still preserved as a queue row"
            );
        });
        assert!(
            warping_gate_is_satisfied(&terminal_view, conversation_id, &app),
            "a genuinely streaming turn must keep its Warping... indicator"
        );
    });
}

#[test]
fn viewer_prompt_delivered_to_a_joined_session_leaves_no_queue_row() {
    // The happy path must be untouched: a prompt the sharer acknowledges belongs to the sharer,
    // and no fallback row should linger in the panel.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane(&mut app, Stage::JoinedSuccessfully);
        let (terminal_view, conversation_id, network) = (
            pane.view.clone(),
            pane.conversation_id,
            pane.network.clone(),
        );

        submit_viewer_prompt(&mut app, &terminal_view, "ship it");

        let request = sent_agent_prompt(&app, &network);
        assert_eq!(request.prompt, "ship it");
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert!(
                queue_model.queue(conversation_id).is_empty(),
                "a locally accepted prompt must not produce a visible fallback row"
            );
        });

        terminal_view.update(&mut app, |view, ctx| {
            assert!(
                view.on_viewer_prompt_acknowledged(&request.id, ctx),
                "the pane's own request id must resolve"
            );
        });
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert!(queue_model.queue(conversation_id).is_empty());
        });
    });
}

#[test]
fn an_unrelated_or_duplicate_acknowledgement_resolves_nothing() {
    // Matching by request id is what makes a late echo for a retired revision, or a duplicate of
    // one already handled, incapable of resolving a prompt twice.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane(&mut app, Stage::JoinedSuccessfully);
        let (terminal_view, network) = (pane.view.clone(), pane.network.clone());

        submit_viewer_prompt(&mut app, &terminal_view, "ship it");
        let request = sent_agent_prompt(&app, &network);

        terminal_view.update(&mut app, |view, ctx| {
            assert!(
                !view.on_viewer_prompt_acknowledged(&AgentPromptRequestId::new(), ctx),
                "an acknowledgement for a request this pane never sent must be a no-op"
            );
            assert!(view.on_viewer_prompt_acknowledged(&request.id, ctx));
            assert!(
                !view.on_viewer_prompt_acknowledged(&request.id, ctx),
                "a duplicate acknowledgement must be a no-op"
            );
        });
    });
}

#[test]
fn handle_viewer_session_end_ignores_stale_ambient_end() {
    // A stale ambient end (the ended network is no longer the current one) must
    // be ignored: `handle_viewer_session_end` routes ambient panes through
    // `end_current_ambient_session`, whose current-network guard bails, so the
    // helper returns `false` and performs no teardown.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal_view = add_window_with_terminal(&mut app, None);
        let model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));

        let (wakeups_tx, _wakeups_rx) = async_channel::unbounded();
        let (events_tx, _events_rx) = async_channel::unbounded();
        let (pty_reads_tx, pty_reads_rx) = broadcast(8);
        let _inactive_pty_reads_rx = pty_reads_rx.deactivate();
        let channel_event_proxy = ChannelEventListener::new(wakeups_tx, events_tx, pty_reads_tx);
        let (_write_to_pty_tx, write_to_pty_rx) = async_channel::unbounded();

        let ended_network = app.add_model(|ctx| {
            Network::new_for_test(
                channel_event_proxy,
                terminal_view.downgrade(),
                model.clone(),
                write_to_pty_rx,
                RemoteUpdateGuard::new(),
                ctx,
            )
        });

        // Empty `current_network` => the ended network is stale.
        let current_network = Arc::new(FairMutex::new(None));
        let orchestration_viewer_model = Arc::new(FairMutex::new(None));

        let mut handled = true;
        app.update(|ctx| {
            handled = TerminalManager::handle_viewer_session_end(
                &terminal_view,
                model.clone(),
                &current_network,
                &ended_network,
                &orchestration_viewer_model,
                /* is_ambient_agent */ true,
                ctx,
            );
        });

        assert!(
            !handled,
            "a stale ambient end (ended network != current) must be ignored"
        );
        assert!(
            !model.lock().shared_session_status().is_finished_viewer(),
            "an ignored stale ambient end must not finish the viewer"
        );
    });
}
