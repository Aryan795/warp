use std::sync::Arc;
use std::time::Duration;

use async_channel::Sender;
use byte_unit::Byte;
use futures_util::stream::AbortHandle;
use instant::Instant;
use parking_lot::FairMutex;
use prost::Message as _;
use session_sharing_protocol::common::{
    ActivePrompt, AgentPromptFailureReason, AgentPromptRequest, AgentPromptRequestId,
    ExecutionIdentity, NegotiatedSessionContent, OrderedTerminalEvent, OrderedTerminalEventType,
    ParticipantId, Selection, SemanticCursor, SessionContentMode, SessionId,
};
use session_sharing_protocol::sharer::{
    DownstreamMessage, FailedToInitializeSessionReason, QuotaType, ReconnectToken, UpstreamMessage,
};
use warp_conversation_mutation_api::{
    AcceptedMessageContext, Attribution, Author, ConversationMutation, Origin, SchemaVersion,
    author, origin,
};
use warp_multi_agent_api::{ResponseEvent, response_event};
use warp_semantic_session::{RequestedSessionContent, SemanticMutationProducer};
use warp_server_client::iap::IapManager;
use warpui::r#async::FutureExt as _;
use warpui::{App, ModelHandle};
use websocket::{Message, WebsocketMessage as _};

use super::{
    AMBIENT_CREATE_SESSION_MAX_ATTEMPTS, Network, PTY_READS_BATCH_THRESHOLD, PtyBytesBatchStatus,
    Stage, StartupFailure, StartupRetryState, semantic_initial_terminal_state,
    session_terminated_reason_diagnostic_label, session_terminated_reason_string,
    startup_max_attempts,
};
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::server::server_api::ServerApiProvider;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::terminal::TerminalModel;
use crate::terminal::shared_session::{MAX_BYTES_SHAREABLE, SharedSessionSource};
use crate::test_util::assert_eventually;

fn execution() -> ExecutionIdentity {
    ExecutionIdentity {
        conversation_id: "conversation".to_owned(),
        execution_id: "execution".to_owned(),
        run_id: Some("run".to_owned()),
        request_id: Some("request".to_owned()),
    }
}

fn semantic_cursor(session_id: SessionId, mutation_sequence: u64) -> SemanticCursor {
    SemanticCursor {
        session_id,
        conversation_id: "conversation".to_owned(),
        execution_id: "execution".to_owned(),
        content_mode: SessionContentMode::SemanticConversationOnly,
        schema_version: 1,
        mutation_sequence,
    }
}

fn semantic_echo(execution_identity: ExecutionIdentity) -> NegotiatedSessionContent {
    NegotiatedSessionContent {
        content_mode: SessionContentMode::SemanticConversationOnly,
        semantic_schema_version: 1,
        execution_identity,
    }
}

fn accepted_message_context() -> AcceptedMessageContext {
    AcceptedMessageContext {
        schema_version: SchemaVersion::V1 as i32,
        conversation_id: "conversation".to_owned(),
        execution_id: "execution".to_owned(),
        message_id: "accepted-message".to_owned(),
        attribution: Some(Attribution {
            author: Some(Author {
                id: "factory-user".to_owned(),
                kind: author::Kind::User as i32,
                display_name: "Factory User".to_owned(),
            }),
            origin: Some(Origin {
                kind: origin::Kind::Factory as i32,
                source_id: "factory".to_owned(),
                subtype: "slack".to_owned(),
            }),
            source_delivery: None,
        }),
        media: Vec::new(),
    }
}

fn is_upstream_message_pty_bytes_read(
    message: UpstreamMessage,
    expected_event_no: usize,
    expected_bytes: Vec<u8>,
) -> bool {
    let compressed_bytes = lz4_flex::block::compress_prepend_size(&expected_bytes);
    matches!(message, UpstreamMessage::OrderedTerminalEvent(OrderedTerminalEvent {
        event_no,
        event_type: OrderedTerminalEventType::PtyBytesRead { bytes },
    }) if event_no == expected_event_no && bytes == compressed_bytes)
}

#[test]
fn test_startup_max_attempts_only_retries_ambient_agent_sources() {
    assert_eq!(
        startup_max_attempts(&SharedSessionSource::ambient_agent(Some(
            "task-id".to_string()
        ))),
        AMBIENT_CREATE_SESSION_MAX_ATTEMPTS
    );
    assert_eq!(startup_max_attempts(&SharedSessionSource::user(None)), 1);
}

#[test]
fn test_startup_failure_retryability() {
    assert!(StartupFailure::Transport.is_retryable());
    assert!(StartupFailure::InitializeSend.is_retryable());
    assert!(!StartupFailure::Negotiation.is_retryable());
    assert!(StartupFailure::WebsocketClosedBeforeStarted.is_retryable());
    assert!(StartupFailure::WebsocketError.is_retryable());
    assert!(StartupFailure::Timeout.is_retryable());
    assert!(
        StartupFailure::ServerRejected(FailedToInitializeSessionReason::InternalServerError {
            details: "transient".to_string(),
        })
        .is_retryable()
    );

    assert!(
        !StartupFailure::ServerRejected(FailedToInitializeSessionReason::ScrollbackTooLarge {})
            .is_retryable()
    );
    assert!(
        !StartupFailure::ServerRejected(FailedToInitializeSessionReason::NoUserQuotaRemaining {
            quota_type: QuotaType::SessionsCreated,
        })
        .is_retryable()
    );
    assert!(
        !StartupFailure::ServerRejected(FailedToInitializeSessionReason::UserNotFound)
            .is_retryable()
    );
}
#[test]
fn test_semantic_initial_terminal_state_is_sanitized() {
    let state = semantic_initial_terminal_state();

    assert!(matches!(state.active_prompt, ActivePrompt::PS1));
    assert_eq!(state.window_size.num_rows, 0);
    assert_eq!(state.window_size.num_cols, 0);
    assert!(matches!(state.selection, Selection::None));
    assert!(state.init_block_id.to_string().is_empty());
    assert!(state.input_replica_id.to_string().is_empty());
    assert!(state.universal_developer_input_context.is_none());
}

#[test]
fn test_should_retry_startup_failure_respects_attempt_budget() {
    App::test((), |mut app| async move {
        let network = create_network(&mut app, false).0;

        network.update(&mut app, |network, _| {
            network.stage = Stage::BeforeStarted {
                startup_retry: StartupRetryState {
                    current_attempt: 1,
                    max_attempts: AMBIENT_CREATE_SESSION_MAX_ATTEMPTS,
                    timeout_abort_handle: None,
                    transport_abort_handle: None,
                },
            };
            assert!(network.should_retry_startup_failure(&StartupFailure::Timeout));

            network.stage = Stage::BeforeStarted {
                startup_retry: StartupRetryState {
                    current_attempt: AMBIENT_CREATE_SESSION_MAX_ATTEMPTS,
                    max_attempts: AMBIENT_CREATE_SESSION_MAX_ATTEMPTS,
                    timeout_abort_handle: None,
                    transport_abort_handle: None,
                },
            };
            assert!(!network.should_retry_startup_failure(&StartupFailure::Timeout));

            let mut startup_retry = StartupRetryState::new(1);
            startup_retry.current_attempt = 1;
            network.stage = Stage::BeforeStarted { startup_retry };
            assert!(
                !network.should_retry_startup_failure(&StartupFailure::ServerRejected(
                    FailedToInitializeSessionReason::InternalServerError {
                        details: "transient".to_string(),
                    }
                ))
            );
        });
    });
}

#[test]
fn test_startup_attempt_stale_filtering() {
    App::test((), |mut app| async move {
        let network = create_network(&mut app, false).0;

        network.update(&mut app, |network, _| {
            network.stage = Stage::BeforeStarted {
                startup_retry: StartupRetryState {
                    current_attempt: 1,
                    max_attempts: AMBIENT_CREATE_SESSION_MAX_ATTEMPTS,
                    timeout_abort_handle: None,
                    transport_abort_handle: None,
                },
            };
            assert!(!network.should_ignore_startup_attempt_websocket_callback(1));
            assert!(network.should_ignore_startup_attempt_websocket_callback(0));
            network.stage = Stage::StartedSuccessfully {
                startup_attempt: Some(1),
            };
            assert!(!network.should_ignore_startup_attempt_websocket_callback(1));
            assert!(network.should_ignore_startup_attempt_websocket_callback(0));

            network.stage = Stage::StartedSuccessfully {
                startup_attempt: None,
            };
            assert!(!network.should_ignore_startup_attempt_websocket_callback(0));
        });
    });
}

fn is_upstream_message_command_executed(
    message: &UpstreamMessage,
    expected_event_no: usize,
) -> bool {
    matches!(message, UpstreamMessage::OrderedTerminalEvent(OrderedTerminalEvent {
        event_no,
        event_type: OrderedTerminalEventType::CommandExecutionStarted { .. },
    }) if *event_no == expected_event_no)
}

fn create_network(
    app: &mut App,
    session_initialized: bool,
) -> (ModelHandle<Network>, Sender<OrderedTerminalEventType>) {
    let (ordered_events_tx, ordered_events_rx) = async_channel::unbounded();
    let active_prompt = ActivePrompt::default();
    let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));

    let network = app.add_model(|ctx| {
        Network::new_for_test(
            terminal_model,
            ordered_events_rx,
            active_prompt,
            Selection::None,
            Byte::from_u64(MAX_BYTES_SHAREABLE as u64),
            ctx,
        )
    });

    if session_initialized {
        network.update(app, |network, _| {
            network.stage = Stage::StartedSuccessfully {
                startup_attempt: None,
            };
        });
    }

    (network, ordered_events_tx)
}

fn configure_semantic_network(network: &ModelHandle<Network>, app: &mut App) -> SessionId {
    let session_id = SessionId::new();
    network.update(app, |network, _| {
        let execution_identity = execution();
        network.content = RequestedSessionContent::semantic_v1(execution_identity.clone());
        network.semantic_producer = Some(SemanticMutationProducer::new(execution_identity));
        network.session_id = Some(session_id);
        network.stage = Stage::StartedSuccessfully {
            startup_attempt: None,
        };
    });
    session_id
}

#[test]
fn test_semantic_negotiation_fails_closed_without_exact_echo() {
    App::test((), |mut app| async move {
        let missing_echo = create_network(&mut app, false).0;
        missing_echo.update(&mut app, |network, ctx| {
            let execution_identity = execution();
            network.content = RequestedSessionContent::semantic_v1(execution_identity.clone());
            network.semantic_producer = Some(SemanticMutationProducer::new(execution_identity));
            let Stage::BeforeStarted { startup_retry } = &mut network.stage else {
                panic!("test network should be starting");
            };
            startup_retry.current_attempt = 1;
            let message = DownstreamMessage::SessionInitialized {
                session_id: SessionId::new(),
                session_secret: Default::default(),
                reconnect_token: ReconnectToken::new(),
                sharer_id: ParticipantId::new(),
                sharer_firebase_uid: "mock_firebase_uid".to_owned(),
                negotiated_content: None,
            };
            network.process_websocket_message(Message::new(message.to_json().unwrap()), ctx);
        });
        missing_echo.read(&app, |network, _| {
            assert!(matches!(network.stage, Stage::Finished));
            assert!(network.session_id.is_none());
        });

        let matching_echo = create_network(&mut app, false).0;
        matching_echo.update(&mut app, |network, ctx| {
            let execution_identity = execution();
            network.content = RequestedSessionContent::semantic_v1(execution_identity.clone());
            network.semantic_producer =
                Some(SemanticMutationProducer::new(execution_identity.clone()));
            let message = DownstreamMessage::SessionInitialized {
                session_id: SessionId::new(),
                session_secret: Default::default(),
                reconnect_token: ReconnectToken::new(),
                sharer_id: ParticipantId::new(),
                sharer_firebase_uid: "mock_firebase_uid".to_owned(),
                negotiated_content: Some(semantic_echo(execution_identity)),
            };
            network.process_websocket_message(Message::new(message.to_json().unwrap()), ctx);
        });
        matching_echo.read(&app, |network, _| {
            assert!(matches!(network.stage, Stage::StartedSuccessfully { .. }));
            assert!(network.session_id.is_some());
        });
    });
}

#[test]
fn test_semantic_prompt_requires_valid_context_after_negotiation() {
    App::test((), |mut app| async move {
        let missing = create_network(&mut app, true).0;
        configure_semantic_network(&missing, &mut app);
        let missing_responses = missing.read(&app, |network, _| network.ws_proxy_rx.clone());
        missing.update(&mut app, |network, ctx| {
            let request_id = AgentPromptRequestId::default();
            let message = DownstreamMessage::AgentPromptRequested {
                id: request_id.clone(),
                participant_id: ParticipantId::new(),
                request: AgentPromptRequest {
                    id: request_id,
                    server_conversation_token: None,
                    prompt: "missing context".to_owned(),
                    attachments: Vec::new(),
                    accepted_message_context: None,
                },
            };
            network.process_websocket_message(Message::new(message.to_json().unwrap()), ctx);
        });
        assert!(matches!(
            missing_responses.try_recv().unwrap(),
            UpstreamMessage::RejectAgentPromptRequest {
                reason: AgentPromptFailureReason::InvalidConversation,
                ..
            }
        ));

        let invalid = create_network(&mut app, true).0;
        configure_semantic_network(&invalid, &mut app);
        let invalid_responses = invalid.read(&app, |network, _| network.ws_proxy_rx.clone());
        invalid.update(&mut app, |network, ctx| {
            let request_id = AgentPromptRequestId::default();
            let mut context = accepted_message_context();
            context.execution_id = "different".to_owned();
            let message = DownstreamMessage::AgentPromptRequested {
                id: request_id.clone(),
                participant_id: ParticipantId::new(),
                request: AgentPromptRequest {
                    id: request_id,
                    server_conversation_token: None,
                    prompt: "invalid context".to_owned(),
                    attachments: Vec::new(),
                    accepted_message_context: Some(context.encode_to_vec()),
                },
            };
            network.process_websocket_message(Message::new(message.to_json().unwrap()), ctx);
        });
        assert!(matches!(
            invalid_responses.try_recv().unwrap(),
            UpstreamMessage::RejectAgentPromptRequest {
                reason: AgentPromptFailureReason::InvalidConversation,
                ..
            }
        ));

        let valid = create_network(&mut app, true).0;
        configure_semantic_network(&valid, &mut app);
        let captured = Arc::new(parking_lot::Mutex::new(None));
        let captured_clone = captured.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&valid, move |_, event, _| {
                if let super::NetworkEvent::AgentPromptRequested {
                    accepted_message_context,
                    ..
                } = event
                {
                    *captured_clone.lock() = accepted_message_context.as_deref().cloned();
                }
            });
        });
        valid.update(&mut app, |network, ctx| {
            let request_id = AgentPromptRequestId::default();
            let message = DownstreamMessage::AgentPromptRequested {
                id: request_id.clone(),
                participant_id: ParticipantId::new(),
                request: AgentPromptRequest {
                    id: request_id,
                    server_conversation_token: None,
                    prompt: "valid context".to_owned(),
                    attachments: Vec::new(),
                    accepted_message_context: Some(accepted_message_context().encode_to_vec()),
                },
            };
            network.process_websocket_message(Message::new(message.to_json().unwrap()), ctx);
        });
        assert_eq!(*captured.lock(), Some(accepted_message_context()));
    });
}

#[test]
fn test_actual_producer_mutation_transport_and_retransmission_are_identical() {
    App::test((), |mut app| async move {
        let network = create_network(&mut app, true).0;
        configure_semantic_network(&network, &mut app);
        let ws_proxy_rx = network.read(&app, |network, _| network.ws_proxy_rx.clone());
        let mut producer = SemanticMutationProducer::new(execution());
        let mutation = producer
            .normalize(&ResponseEvent {
                r#type: Some(response_event::Type::Init(response_event::StreamInit {
                    conversation_id: "conversation".to_owned(),
                    request_id: "request".to_owned(),
                    run_id: "run".to_owned(),
                })),
            })
            .unwrap()
            .pop()
            .unwrap();

        network.update(&mut app, |network, _| {
            assert!(network.send_semantic_mutation(mutation.clone()));
        });
        let first = ws_proxy_rx.recv().await.unwrap();
        let first_json = first.to_json().unwrap();
        let UpstreamMessage::OrderedTerminalEvent(first_event) = first else {
            panic!("producer mutation should use ordered semantic transport");
        };
        let OrderedTerminalEventType::SemanticConversationMutation {
            mutation: encoded, ..
        } = first_event.event_type
        else {
            panic!("producer mutation should remain semantic");
        };
        assert_eq!(
            ConversationMutation::decode(encoded.as_slice()).unwrap(),
            mutation
        );

        network.update(&mut app, |network, _| {
            network.flush_terminal_events_to_server(0);
        });
        let retransmitted = ws_proxy_rx.recv().await.unwrap();
        assert_eq!(retransmitted.to_json().unwrap(), first_json);
    });
}

#[test]
fn test_storage_unavailable_has_exhaustive_diagnostic_and_user_message() {
    let reason = session_sharing_protocol::sharer::SessionTerminatedReason::StorageUnavailable;
    assert_eq!(
        session_terminated_reason_diagnostic_label(&reason),
        "storage_unavailable"
    );
    assert!(session_terminated_reason_string(&reason, Byte::from_u64(1)).contains("storage"));
}

#[test]
fn test_semantic_filtering_happens_before_batching_and_event_numbering() {
    App::test((), |mut app| async move {
        let network = create_network(&mut app, true).0;
        let session_id = configure_semantic_network(&network, &mut app);
        let ws_proxy_rx = network.read(&app, |network, _| network.ws_proxy_rx.clone());

        network.update(&mut app, |network, _| {
            network.send_ordered_terminal_event_message(
                OrderedTerminalEventType::CommandExecutionStarted {
                    participant_id: ParticipantId::new(),
                    ai_metadata: None,
                },
            );
            network.pty_bytes_batch_status = PtyBytesBatchStatus::Batching {
                accumulated: b"terminal output".to_vec(),
                abort_handle: AbortHandle::new_pair().0,
            };
            network.send_pty_bytes_read_message();
        });

        network.read(&app, |network, _| {
            assert_eq!(usize::from(network.event_no), 0);
            assert!(network.unacked_terminal_events.is_empty());
            assert!(matches!(
                network.pty_bytes_batch_status,
                PtyBytesBatchStatus::NotBatching { .. }
            ));
        });
        assert!(ws_proxy_rx.is_empty());

        network.update(&mut app, |network, _| {
            network.send_ordered_terminal_event_message(
                OrderedTerminalEventType::SemanticConversationMutation {
                    cursor: semantic_cursor(session_id, 1),
                    mutation: vec![1, 2, 3],
                },
            );
        });

        let message = ws_proxy_rx.recv().await.unwrap();
        assert!(matches!(
            message,
            UpstreamMessage::OrderedTerminalEvent(OrderedTerminalEvent {
                event_no: 0,
                event_type: OrderedTerminalEventType::SemanticConversationMutation { .. },
            })
        ));
        network.read(&app, |network, _| {
            assert_eq!(usize::from(network.event_no), 1);
            assert_eq!(network.unacked_terminal_events.len(), 1);
            assert_eq!(network.unacked_terminal_event_bytes, 3);
        });

        network.update(&mut app, |network, ctx| {
            let ack = DownstreamMessage::EventsProcessedAck {
                latest_processed_event_no: 0,
            };
            network.process_websocket_message(Message::new(ack.to_json().unwrap()), ctx);
        });
        network.read(&app, |network, _| {
            assert!(network.unacked_terminal_events.is_empty());
            assert_eq!(network.unacked_terminal_event_bytes, 0);
        });
    });
}

#[test]
fn test_semantic_final_sender_rejects_direct_terminal_event() {
    App::test((), |mut app| async move {
        let network = create_network(&mut app, true).0;
        configure_semantic_network(&network, &mut app);
        let ws_proxy_rx = network.read(&app, |network, _| network.ws_proxy_rx.clone());

        network.update(&mut app, |network, _| {
            network.send_message_to_server(UpstreamMessage::OrderedTerminalEvent(
                OrderedTerminalEvent {
                    event_no: 0,
                    event_type: OrderedTerminalEventType::PtyBytesRead {
                        bytes: b"terminal output".to_vec(),
                    },
                },
            ));
        });

        network.read(&app, |network, _| {
            assert!(network.unacked_terminal_events.is_empty());
            assert_eq!(network.unacked_terminal_event_bytes, 0);
        });
        assert!(ws_proxy_rx.is_empty());
    });
}

#[test]
fn test_unacked_count_and_byte_limits_stop_before_allocating_an_event_number() {
    App::test((), |mut app| async move {
        let byte_limited = create_network(&mut app, true).0;
        let session_id = configure_semantic_network(&byte_limited, &mut app);
        byte_limited.update(&mut app, |network, _| {
            network.unacked_terminal_event_bytes = super::MAX_UNACKED_EVENT_BYTES;
            network.send_ordered_terminal_event_message(
                OrderedTerminalEventType::SemanticConversationMutation {
                    cursor: semantic_cursor(session_id, 1),
                    mutation: vec![1],
                },
            );
        });
        byte_limited.read(&app, |network, _| {
            assert!(matches!(network.stage, Stage::Finished));
            assert_eq!(usize::from(network.event_no), 0);
        });

        let count_limited = create_network(&mut app, true).0;
        let session_id = configure_semantic_network(&count_limited, &mut app);
        count_limited.update(&mut app, |network, _| {
            network
                .unacked_terminal_events
                .extend((0..super::MAX_UNACKED_EVENTS).map(|event_no| {
                    (
                        event_no,
                        OrderedTerminalEvent {
                            event_no,
                            event_type: OrderedTerminalEventType::CommandExecutionStarted {
                                participant_id: ParticipantId::new(),
                                ai_metadata: None,
                            },
                        },
                    )
                }));
            network.send_ordered_terminal_event_message(
                OrderedTerminalEventType::SemanticConversationMutation {
                    cursor: semantic_cursor(session_id, 1),
                    mutation: vec![1],
                },
            );
        });
        count_limited.read(&app, |network, _| {
            assert!(matches!(network.stage, Stage::Finished));
            assert_eq!(usize::from(network.event_no), 0);
            assert_eq!(
                network.unacked_terminal_events.len(),
                super::MAX_UNACKED_EVENTS
            );
        });
    });
}

#[test]
fn test_semantic_unacked_limits_do_not_stop_full_terminal_sessions() {
    App::test((), |mut app| async move {
        let network = create_network(&mut app, true).0;
        network.update(&mut app, |network, _| {
            network.unacked_terminal_event_bytes = super::MAX_UNACKED_EVENT_BYTES;
            network.send_ordered_terminal_event_message(OrderedTerminalEventType::PtyBytesRead {
                bytes: vec![1],
            });
        });

        network.read(&app, |network, _| {
            assert!(matches!(network.stage, Stage::StartedSuccessfully { .. }));
            assert_eq!(usize::from(network.event_no), 1);
        });
    });
}

#[test]
fn test_send_ordered_terminal_event_message_advances_event_no() {
    App::test((), |mut app| async move {
        let network = create_network(&mut app, true).0;

        // Make sure the event no starts at 0.
        network.read(&app, |network, _ctx| {
            assert_eq!(usize::from(network.event_no), 0);
        });

        // Try to send an ordered terminal event message to the server.
        let event = OrderedTerminalEventType::PtyBytesRead { bytes: "a".into() };
        network.update(&mut app, |network, _| {
            network.send_ordered_terminal_event_message(event);
        });

        // The event no should be 1 now.
        network.read(&app, |network, _ctx| {
            assert_eq!(usize::from(network.event_no), 1);
        });
    });
}

#[test]
fn test_send_ordered_terminal_event_message_max_reached() {
    App::test((), |mut app| async move {
        let network = create_network(&mut app, true).0;
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());

        // Make sure the ws_proxy_tx is open.
        let ws_proxy_tx = network.read(&app, |network, _ctx| network.ws_proxy_tx.clone());
        assert!(!ws_proxy_tx.is_closed());

        // Try to send an ordered terminal event that would exceed the max bytes allowed limit.
        let overflow_event = OrderedTerminalEventType::PtyBytesRead {
            bytes: "a".repeat(MAX_BYTES_SHAREABLE + 1).into(),
        };
        network.update(&mut app, |network, _| {
            network.send_ordered_terminal_event_message(overflow_event);
        });

        // Make sure the item we put on the ws_proxy_tx was correct.
        assert_eq!(ws_proxy_rx.len(), 1);
        let item = ws_proxy_rx.recv().await;
        assert!(matches!(item.unwrap(), UpstreamMessage::EndSession { .. }));

        // Make sure the ws_proxy_tx is closed and nothing was sent.
        assert!(ws_proxy_tx.is_closed());
    });
}

#[test]
fn test_send_pty_read_event_while_batching() {
    App::test((), |mut app| async move {
        let network = create_network(&mut app, true).0;
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());
        let init_time = Instant::now();

        // Set the batch status to batching.
        network.update(&mut app, |network, _ctx| {
            network.pty_bytes_batch_status = PtyBytesBatchStatus::Batching {
                accumulated: "a".into(),
                abort_handle: AbortHandle::new_pair().0,
            };
        });

        // Try to send a PtyBytesRead message to the server.
        network.update(&mut app, |network, _| {
            network.send_pty_bytes_read_message();
        });

        // Make sure the item we put on the ws_proxy_tx was correct.
        let item = ws_proxy_rx.recv().await;
        assert!(is_upstream_message_pty_bytes_read(
            item.unwrap(),
            0,
            "a".into()
        ));

        // The batch status should be NotBatching now and the last_sent_at should be updated.
        network.read(&app, |network, _ctx| {
            assert!(matches!(network.pty_bytes_batch_status, PtyBytesBatchStatus::NotBatching { last_sent_at } if last_sent_at > init_time ));
        });
    });
}

#[test]
fn test_send_pty_read_event_while_not_batching() {
    App::test((), |mut app| async move {
        let network = create_network(&mut app, true).0;
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());
        let init_time = Instant::now();

        // Set the batch status to not batching.
        network.update(&mut app, |network, _ctx| {
            network.pty_bytes_batch_status = PtyBytesBatchStatus::NotBatching {
                last_sent_at: init_time,
            }
        });

        // Try to send a PtyBytesRead message to the server.
        network.update(&mut app, |network, _| {
            network.send_pty_bytes_read_message();
        });

        // Make sure we didn't try to send anything to the server..
        assert_eq!(ws_proxy_rx.len(), 0);

        // The batch status should be unchanged.
        network.read(&app, |network, _ctx| {
            assert!(matches!(network.pty_bytes_batch_status, PtyBytesBatchStatus::NotBatching { last_sent_at } if last_sent_at == init_time));
        });
    });
}

#[test]
fn test_handle_pty_read_event_while_batching() {
    App::test((), |mut app| async move {
        let (network, ordered_events_tx) = create_network(&mut app, true);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());
        let init_time = Instant::now();

        // Set the batch status to batching.
        network.update(&mut app, |network, _ctx| {
            network.pty_bytes_batch_status = PtyBytesBatchStatus::Batching {
                accumulated: "a".into(),
                abort_handle: AbortHandle::new_pair().0,
            };
        });

        // Send a PtyBytesRead event to the Network model.
        let event = OrderedTerminalEventType::PtyBytesRead { bytes: "a".into() };
        ordered_events_tx
            .try_send(event)
            .expect("Can send event over ordered_events_tx");

        // The batching status should reflect the accumulated bytes.
        assert_eventually!(
            network.read(&app, |network, _ctx| {
                matches!(&network.pty_bytes_batch_status, PtyBytesBatchStatus::Batching { accumulated, .. } if accumulated == b"aa" )
            }), "Batching status should reflect accumulated bytes"
        );

        // Technically, we didn't start a task to send the event to the server after a timer. So let's do it manually.
        network.update(&mut app, |network, _| {
            network.send_pty_bytes_read_message();
        });

        // Eventually, the accumulated event should be sent to the server.
        assert_eq!(ws_proxy_rx.len(), 1);
        let item = ws_proxy_rx.recv().await;
        assert!(is_upstream_message_pty_bytes_read(
            item.unwrap(),
            0,
            "aa".into()
        ));

        // The batching status should be reset.
        network.read(&app, |network, _ctx| {
            assert!(matches!(network.pty_bytes_batch_status, PtyBytesBatchStatus::NotBatching { last_sent_at } if last_sent_at > init_time));
        });
    })
}

#[test]
fn test_handle_pty_read_event_while_not_batching() {
    App::test((), |mut app| async move {
        let (network, ordered_events_tx) = create_network(&mut app, true);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());
        let init_time = Instant::now();

        // Set the batch status to not batching.
        network.update(&mut app, |network, _ctx| {
            network.pty_bytes_batch_status = PtyBytesBatchStatus::NotBatching {
                last_sent_at: init_time,
            }
        });

        // Send a PtyBytesRead event to the Network model.
        let event = OrderedTerminalEventType::PtyBytesRead { bytes: "a".into() };
        ordered_events_tx
            .try_send(event)
            .expect("Can send event over ordered_events_tx");

        // The test executor uses real (async_io) timers with no mock clock, so this
        // test relies on the batch timer actually firing. Under test builds
        // PTY_READS_BATCH_THRESHOLD is larger than the ~50ms production value so the
        // transient `Batching` state below is reliably observable instead of racing the
        // timer under coarse scheduler granularity (which flaked on Windows CI).
        assert_eventually!(
            200 =>
            network.read(&app, |network, _ctx| {
                matches!(&network.pty_bytes_batch_status, PtyBytesBatchStatus::Batching { accumulated, .. } if accumulated == b"a" )
            }),
            "Batching status should be batching"
        );

        // When the batch timer fires, the accumulated event is flushed to the server.
        // Await the flush directly rather than polling a fixed tick budget, but bound the
        // wait (generously, relative to the test-build batch threshold) so a regression in
        // the timer/flush path fails this test promptly instead of hanging until the CI
        // timeout.
        let item = ws_proxy_rx
            .recv()
            .with_timeout(PTY_READS_BATCH_THRESHOLD * 20)
            .await
            .expect("Accumulated event should be flushed before the timeout");
        assert!(is_upstream_message_pty_bytes_read(
            item.unwrap(),
            0,
            "a".into()
        ));

        // The batching status should be reset.
        network.read(&app, |network, _ctx| {
            assert!(matches!(network.pty_bytes_batch_status, PtyBytesBatchStatus::NotBatching { last_sent_at } if last_sent_at > init_time));
        });
    });
}

/// Waits until the mock terminal model reports its active block as bootstrapped.
///
/// `start_ordered_terminal_events_listener` silently drops ordered events until this is
/// true, so callers must wait for it instead of racing it: sending an event beforehand can
/// flake if the listener task hasn't observed the bootstrapped state yet. Uses the same
/// generous 2s budget as the `recv()` timeouts below it, rather than the default
/// `assert_eventually!` tick budget, so this wait can't reintroduce a fixed-window race of
/// its own.
async fn wait_for_bootstrapped(network: &ModelHandle<Network>, app: &App) {
    assert_eventually!(
        400 =>
        network.read(app, |network, _ctx| network
            .model
            .lock()
            .is_active_block_bootstrapped()),
        "Mock terminal model should report the active block as bootstrapped"
    );
}

#[test]
fn test_handle_non_pty_read_event_while_batching() {
    App::test((), |mut app| async move {
        let (network, ordered_events_tx) = create_network(&mut app, true);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());
        let init_time = Instant::now();

        // Set the batch status to batching.
        network.update(&mut app, |network, _ctx| {
            network.pty_bytes_batch_status = PtyBytesBatchStatus::Batching {
                accumulated: "a".into(),
                abort_handle: AbortHandle::new_pair().0,
            };
        });

        wait_for_bootstrapped(&network, &app).await;

        // Send a non PtyBytesRead event to the Network model.
        let event = OrderedTerminalEventType::CommandExecutionStarted {
            participant_id: Default::default(),
            ai_metadata: None,
        };
        ordered_events_tx
            .try_send(event)
            .expect("Can send event over ordered_events_tx");

        // Await each flush directly rather than polling a fixed tick budget, so a scheduling
        // delay under load can't race a fixed timeout window (which flaked on Windows CI).
        // Make sure that we flush the PtyBytesRead message first.
        let item = ws_proxy_rx
            .recv()
            .with_timeout(Duration::from_secs(2))
            .await
            .expect("PtyBytesRead flush message should be sent before the timeout");
        assert!(is_upstream_message_pty_bytes_read(
            item.unwrap(),
            0,
            "a".into()
        ));

        // And that the non PtyBytesRead message follows suit.
        let item = ws_proxy_rx
            .recv()
            .with_timeout(Duration::from_secs(2))
            .await
            .expect("Non-PtyBytesRead message should be sent before the timeout");
        assert!(is_upstream_message_command_executed(&item.unwrap(), 1));

        assert_eq!(ws_proxy_rx.len(), 0);

        // The batching status should be reset.
        network.read(&app, |network, _ctx| {
            assert!(matches!(network.pty_bytes_batch_status, PtyBytesBatchStatus::NotBatching { last_sent_at } if last_sent_at > init_time));
        })
    })
}

#[test]
fn test_handle_non_pty_read_event_while_not_batching() {
    App::test((), |mut app| async move {
        let (network, ordered_events_tx) = create_network(&mut app, true);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());
        let init_time = Instant::now();

        // Set the batch status to not batching.
        network.update(&mut app, |network, _ctx| {
            network.pty_bytes_batch_status = PtyBytesBatchStatus::NotBatching {
                last_sent_at: init_time,
            }
        });

        wait_for_bootstrapped(&network, &app).await;

        // Send a non PtyBytesRead event to the Network model.
        let event = OrderedTerminalEventType::CommandExecutionStarted {
            participant_id: Default::default(),
            ai_metadata: None,
        };
        ordered_events_tx
            .try_send(event)
            .expect("Can send event over ordered_events_tx");

        // Await the flush directly rather than polling a fixed tick budget; see
        // test_handle_non_pty_read_event_while_batching for why.
        let item = ws_proxy_rx
            .recv()
            .with_timeout(Duration::from_secs(2))
            .await
            .expect("Message should be sent before the timeout");
        assert!(is_upstream_message_command_executed(&item.unwrap(), 0));

        // The batching status should be unchanged.
        network.read(&app, |network, _ctx| {
            assert!(matches!(network.pty_bytes_batch_status, PtyBytesBatchStatus::NotBatching { last_sent_at } if last_sent_at == init_time));
        })
    });
}

#[test]
fn test_ignore_duplicate_prompt_updates() {
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app, true);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());

        assert_eq!(ws_proxy_rx.len(), 0);
        // First prompt update should go through.
        network.update(&mut app, |network, _ctx| {
            network.send_active_prompt_update_if_changed(ActivePrompt::WarpPrompt(
                "test warp prompt".to_owned(),
            ));
        });
        assert_eq!(ws_proxy_rx.len(), 1);

        // Duplicate prompt updates should be ignored.
        network.update(&mut app, |network, _ctx| {
            network.send_active_prompt_update_if_changed(ActivePrompt::WarpPrompt(
                "test warp prompt".to_owned(),
            ));
        });
        assert_eq!(ws_proxy_rx.len(), 1);
        network.update(&mut app, |network, _ctx| {
            network.send_active_prompt_update_if_changed(ActivePrompt::WarpPrompt(
                "test warp prompt".to_owned(),
            ));
        });
        assert_eq!(ws_proxy_rx.len(), 1);

        // Different prompt should go through.
        network.update(&mut app, |network, _ctx| {
            network.send_active_prompt_update_if_changed(ActivePrompt::WarpPrompt(
                "different warp prompt".to_owned(),
            ));
        });
        assert_eq!(ws_proxy_rx.len(), 2);
    });
}

#[test]
fn test_selection_updates_throttled_and_duplicates_ignored() {
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app, true);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());

        assert_eq!(ws_proxy_rx.len(), 0);
        // Rapid fire selection updates. Only the last should be sent up the websocket due to throttling.
        network.update(&mut app, |network, _ctx| {
            for i in 0..5 {
                network.send_presence_selection_if_changed(Selection::Blocks {
                    block_ids: vec![format!("block{i}").to_string().into()],
                });
            }
        });

        // Only the very first and the last updates should go through, but not any of the intermediate ones.
        assert_eventually!(
            ws_proxy_rx.len() == 2,
            "Selection updates should be throttled"
        );

        // Last sent block ID should be block4, and duplicate selection updates should be ignored.
        network.update(&mut app, |network, _ctx| {
            network.send_presence_selection_if_changed(Selection::Blocks {
                block_ids: vec!["block4".to_string().into()],
            });
        });
        assert_eventually!(
            ws_proxy_rx.len() == 2,
            "Duplicate selection updates should be ignored"
        );

        // Different selection update should go through.
        network.update(&mut app, |network, _ctx| {
            network.send_presence_selection_if_changed(Selection::None);
        });
        assert_eventually!(
            ws_proxy_rx.len() == 3,
            "Different selection updates should go through"
        );
    });
}

#[test]
fn test_messages_are_buffered_before_session_initialized() {
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app, false);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());

        // The network should start in the BeforeStarted state with no events.
        assert_eq!(ws_proxy_rx.len(), 0);
        network.read(&app, |network, _| {
            assert!(matches!(&network.stage, Stage::BeforeStarted { .. }));
            assert_eq!(network.unacked_terminal_events.len(), 0);
        });

        // Try to send a message to the server.
        let event_type = OrderedTerminalEventType::CommandExecutionStarted {
            participant_id: Default::default(),
            ai_metadata: None,
        };
        let event = OrderedTerminalEvent {
            event_no: 0,
            event_type,
        };
        let message = UpstreamMessage::OrderedTerminalEvent(event);
        network.update(&mut app, |network, _ctx| {
            network.send_message_to_server(message)
        });

        // The message should not be sent to the server but should instead be buffered.
        assert_eq!(ws_proxy_rx.len(), 0);
        network.read(&app, |network, _| {
            assert!(matches!(&network.stage, Stage::BeforeStarted { .. }));
            assert!(is_upstream_message_command_executed(
                &UpstreamMessage::OrderedTerminalEvent(
                    network.unacked_terminal_events.get(&0).unwrap().clone()
                ),
                0
            ));
        });

        // Simulate receiving the SessionInitialized message from the server.
        network.update(&mut app, |network, ctx| {
            let downstream_message = DownstreamMessage::SessionInitialized {
                session_id: SessionId::new(),
                session_secret: Default::default(),
                reconnect_token: ReconnectToken::new(),
                sharer_id: ParticipantId::new(),
                sharer_firebase_uid: "mock_firebase_uid".to_string(),
                negotiated_content: None,
            };
            let serialized = downstream_message.to_json().unwrap();
            network.process_websocket_message(Message::new(serialized), ctx);
        });

        // The message should be flushed to the server and the stage should be advanced.
        // We should also re-send the active prompt.
        assert_eq!(ws_proxy_rx.len(), 2);
        let item = ws_proxy_rx.recv().await;
        assert!(is_upstream_message_command_executed(&item.unwrap(), 0));
        let item = ws_proxy_rx.recv().await;
        matches!(item.unwrap(), UpstreamMessage::UpdateActivePrompt(_));

        network.read(&app, |network, _| {
            assert!(matches!(&network.stage, Stage::StartedSuccessfully { .. }));
        });
    });
}

#[test]
fn test_messages_are_buffered_while_reconnecting() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        // Disabled (`None`) IapManager so the reconnect path, which reads the
        // singleton, doesn't panic; inert no-op in tests.
        app.add_singleton_model(|ctx| {
            IapManager::new(
                None,
                Box::new(|_| futures::FutureExt::boxed(futures::future::ready(None::<String>))),
                None,
                ctx,
            )
        });
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
        app.add_singleton_model(AuthManager::new_for_test);
        let (network, _) = create_network(&mut app, false);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());

        // The network should start in the BeforeStarted state with no events.
        assert_eq!(ws_proxy_rx.len(), 0);
        network.read(&app, |network, _| {
            assert!(matches!(&network.stage, Stage::BeforeStarted { .. }));
            assert_eq!(network.unacked_terminal_events.len(), 0);
        });

        // Simulate receiving the SessionInitialized message from the server.
        network.update(&mut app, |network, ctx| {
            let downstream_message = DownstreamMessage::SessionInitialized {
                session_id: SessionId::new(),
                session_secret: Default::default(),
                reconnect_token: ReconnectToken::new(),
                sharer_id: ParticipantId::new(),
                sharer_firebase_uid: "mock_firebase_uid".to_string(),
                negotiated_content: None,
            };
            let serialized = downstream_message.to_json().unwrap();
            network.process_websocket_message(Message::new(serialized), ctx);
        });

        // We should have sent the latest prompt on connection.
        assert_eq!(ws_proxy_rx.len(), 1);
        let item = ws_proxy_rx.recv().await;
        matches!(item.unwrap(), UpstreamMessage::UpdateActivePrompt(_));

        // Simulate reconnecting to the server after server disconnects. Nothing we need to do in this test to disconnect first.
        network.update(&mut app, |network, ctx| {
            network.reconnect_websocket(ctx);
        });

        network.read(&app, |network, _| {
            assert!(matches!(&network.stage, Stage::Reconnecting { .. }));
        });

        // Try to send a message to the server.
        let event_type = OrderedTerminalEventType::CommandExecutionStarted {
            participant_id: Default::default(),
            ai_metadata: None,
        };
        let event = OrderedTerminalEvent {
            event_no: 0,
            event_type,
        };
        let message = UpstreamMessage::OrderedTerminalEvent(event);
        network.update(&mut app, |network, _ctx| {
            network.send_message_to_server(message)
        });

        // The message should not be sent to the server but should instead be stored.
        assert_eq!(ws_proxy_rx.len(), 0);
        network.read(&app, |network, _| {
            assert!(matches!(&network.stage, Stage::Reconnecting { .. }));
            assert_eq!(network.unacked_terminal_events.len(), 1);
            assert!(is_upstream_message_command_executed(
                &UpstreamMessage::OrderedTerminalEvent(
                    network.unacked_terminal_events.get(&0).unwrap().clone()
                ),
                0
            ));
        });

        // Simulate receiving the SessionReconnected message from the server.
        network.update(&mut app, |network, ctx| {
            let downstream_message = DownstreamMessage::SessionReconnected {
                last_received_event_no: None,
                participant_list: Default::default(),
                negotiated_content: None,
            };
            let serialized = downstream_message.to_json().unwrap();
            network.process_websocket_message(Message::new(serialized), ctx);
        });

        // The message should be flushed to the server and the stage should be advanced.
        // We should also re-send the active prompt.
        assert_eq!(ws_proxy_rx.len(), 2);
        let item = ws_proxy_rx.recv().await;
        assert!(is_upstream_message_command_executed(&item.unwrap(), 0));
        let item = ws_proxy_rx.recv().await;
        matches!(item.unwrap(), UpstreamMessage::UpdateActivePrompt(_));

        network.read(&app, |network, _| {
            assert!(matches!(&network.stage, Stage::StartedSuccessfully { .. }));
        });
    });
}

#[test]
fn test_events_are_saved_on_send_and_removed_on_ack() {
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app, false);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());

        // Simulate receiving the SessionInitialized message from the server.
        network.update(&mut app, |network, ctx| {
            let downstream_message = DownstreamMessage::SessionInitialized {
                session_id: SessionId::new(),
                session_secret: Default::default(),
                reconnect_token: ReconnectToken::new(),
                sharer_id: ParticipantId::new(),
                sharer_firebase_uid: "mock_firebase_uid".to_string(),
                negotiated_content: None,
            };
            let serialized = downstream_message.to_json().unwrap();
            network.process_websocket_message(Message::new(serialized), ctx);
        });

        // We should have sent the latest prompt on connection.
        assert_eq!(ws_proxy_rx.len(), 1);
        let item = ws_proxy_rx.recv().await;
        matches!(item.unwrap(), UpstreamMessage::UpdateActivePrompt(_));

        // Try to send a couple messages to the server.
        let event_type = OrderedTerminalEventType::CommandExecutionStarted {
            participant_id: Default::default(),
            ai_metadata: None,
        };
        let event = OrderedTerminalEvent {
            event_no: 0,
            event_type,
        };
        let message = UpstreamMessage::OrderedTerminalEvent(event);
        network.update(&mut app, |network, _ctx| {
            network.send_message_to_server(message)
        });
        let event_type = OrderedTerminalEventType::CommandExecutionStarted {
            participant_id: Default::default(),
            ai_metadata: None,
        };
        let event = OrderedTerminalEvent {
            event_no: 1,
            event_type,
        };
        let message = UpstreamMessage::OrderedTerminalEvent(event);
        network.update(&mut app, |network, _ctx| {
            network.send_message_to_server(message)
        });

        // The messages should be both sent and stored.
        assert_eq!(ws_proxy_rx.len(), 2);
        let item = ws_proxy_rx.recv().await;
        assert!(is_upstream_message_command_executed(&item.unwrap(), 0));
        let item = ws_proxy_rx.recv().await;
        assert!(is_upstream_message_command_executed(&item.unwrap(), 1));
        network.read(&app, |network, _| {
            assert_eq!(network.unacked_terminal_events.len(), 2);
            assert!(is_upstream_message_command_executed(
                &UpstreamMessage::OrderedTerminalEvent(
                    network.unacked_terminal_events.get(&0).unwrap().clone()
                ),
                0
            ));
            assert!(is_upstream_message_command_executed(
                &UpstreamMessage::OrderedTerminalEvent(
                    network.unacked_terminal_events.get(&1).unwrap().clone()
                ),
                1
            ));
        });

        // Simulate receiving the EventsProcessedAck message from the server.
        network.update(
            &mut app,
            |network, ctx: &mut warpui::ModelContext<'_, Network>| {
                let downstream_message = DownstreamMessage::EventsProcessedAck {
                    latest_processed_event_no: 1,
                };
                let serialized = downstream_message.to_json().unwrap();
                network.process_websocket_message(Message::new(serialized), ctx);
            },
        );

        // Both messages should be removed from the stored events to free up memory.
        network.read(&app, |network, _| {
            assert_eq!(network.unacked_terminal_events.len(), 0);
        });
    });
}
