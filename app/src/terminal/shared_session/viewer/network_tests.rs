use std::sync::Arc;
use std::time::Duration;

use async_channel::Sender;
use async_io::Timer;
use instant::Instant;
use parking_lot::FairMutex;
use prost::Message as _;
use session_sharing_protocol::common::{
    ActivePrompt, BlockId, ExecutionIdentity, InputReplicaId, OrderedTerminalEvent,
    OrderedTerminalEventType, Scrollback, Selection, SemanticCursor, SessionContentMode, SessionId,
    WindowSize,
};
use session_sharing_protocol::viewer::{DownstreamMessage, UpstreamMessage};
use warp_conversation_mutation_api::{
    ContentEncoding, ContentMutation, ConversationMutation, MutationIdentity, SchemaVersion,
    content_mutation, conversation_mutation,
};
use warp_semantic_session::{RequestedSessionContent, SemanticConsumer};
use warpui::{App, ModelHandle};
use websocket::{Message, WebsocketMessage as _};

use super::{Network, PtyBytesBatchStatus, Stage, semantic_join_has_terminal_state};
use crate::terminal::TerminalModel;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::shared_session::shared_handlers::RemoteUpdateGuard;
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

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

fn mutation(sequence: u64) -> ConversationMutation {
    ConversationMutation {
        schema_version: SchemaVersion::V1 as i32,
        identity: Some(MutationIdentity {
            conversation_id: "conversation".to_owned(),
            execution_id: "execution".to_owned(),
            mutation_id: format!("mutation-{sequence}"),
            sequence,
            run_id: "run".to_owned(),
            request_id: "request".to_owned(),
        }),
        attribution: None,
        occurred_at: Some(prost_types::Timestamp {
            seconds: 1_700_000_000,
            nanos: 0,
        }),
        mutation: Some(conversation_mutation::Mutation::Content(ContentMutation {
            entry_id: "entry".to_owned(),
            content_id: "content".to_owned(),
            content_index: 0,
            change: Some(content_mutation::Change::Delta(content_mutation::Delta {
                delta_index: sequence - 1,
                encoding: ContentEncoding::Utf8Text as i32,
                data: b"text".to_vec(),
            })),
        })),
    }
}

fn create_network(app: &mut App) -> (ModelHandle<Network>, Sender<Vec<u8>>) {
    initialize_app_for_terminal_view(app);
    let terminal_view = add_window_with_terminal(app, None).downgrade();
    let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    let channel_event_proxy = ChannelEventListener::new_for_test();
    let (write_to_pty_events_tx, write_to_pty_events_rx) = async_channel::unbounded();

    let network = app.add_model(|ctx| {
        Network::new_for_test(
            channel_event_proxy,
            terminal_view,
            terminal_model,
            write_to_pty_events_rx,
            RemoteUpdateGuard::new(),
            ctx,
        )
    });

    network.update(app, |network, _| {
        network.stage = Stage::JoinedSuccessfully;
    });

    (network, write_to_pty_events_tx)
}

fn configure_semantic_network(network: &ModelHandle<Network>, app: &mut App) -> SessionId {
    network.update(app, |network, _| {
        network.content = RequestedSessionContent::semantic_v1(execution());
        network.semantic_consumer = Some(SemanticConsumer::new(network.session_id, execution(), 1));
    });
    network.read(app, |network, _| network.session_id)
}

#[test]
fn test_semantic_join_rejects_any_terminal_state() {
    let scrollback = Scrollback {
        blocks: Vec::new(),
        is_alt_screen_active: false,
    };
    assert!(!semantic_join_has_terminal_state(
        &scrollback,
        &ActivePrompt::default(),
        WindowSize::default(),
        &BlockId::default(),
        &InputReplicaId::default(),
        &None,
    ));
    assert!(semantic_join_has_terminal_state(
        &scrollback,
        &ActivePrompt::default(),
        WindowSize {
            num_rows: 24,
            num_cols: 80,
        },
        &BlockId::default(),
        &InputReplicaId::default(),
        &None,
    ));
}

#[test]
fn test_semantic_viewer_suppresses_terminal_upstream_state_before_numbering() {
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app);
        configure_semantic_network(&network, &mut app);
        let ws_proxy_rx = network.read(&app, |network, _| network.ws_proxy_rx.clone());

        network.update(&mut app, |network, ctx| {
            let abort_handle = ctx.spawn_abortable(
                Timer::after(Duration::from_secs(1)),
                move |_, _, _| {},
                |_, _| {},
            );
            network.pty_bytes_batch_status = PtyBytesBatchStatus::Batching {
                accumulated: b"terminal input".to_vec(),
                abort_handle,
            };
            network.send_write_to_pty();
            network.send_presence_selection(Selection::None);
        });

        network.read(&app, |network, _| {
            assert_eq!(network.write_to_pty_event_no.as_usize(), 0);
            assert_eq!(usize::from(network.selection_event_no), 0);
            assert!(matches!(
                network.pty_bytes_batch_status,
                PtyBytesBatchStatus::NotBatching { .. }
            ));
        });
        assert!(ws_proxy_rx.is_empty());
    });
}

#[test]
fn test_semantic_viewer_rejects_events_before_negotiation() {
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app);
        let session_id = configure_semantic_network(&network, &mut app);
        network.update(&mut app, |network, ctx| {
            network.stage = Stage::BeforeJoined;
            let message = DownstreamMessage::OrderedTerminalEvent(OrderedTerminalEvent {
                event_no: 0,
                event_type: OrderedTerminalEventType::SemanticConversationMutation {
                    cursor: semantic_cursor(session_id, 1),
                    mutation: mutation(1).encode_to_vec(),
                },
            });
            network.process_websocket_message(Message::new(message.to_json().unwrap()), ctx);
        });

        network.read(&app, |network, _| {
            assert!(matches!(network.stage, Stage::Finished));
            assert!(network.semantic_last_event_no.is_none());
            assert!(
                network
                    .semantic_consumer
                    .as_ref()
                    .and_then(SemanticConsumer::cursor)
                    .is_none()
            );
        });
    });
}

#[test]
fn test_semantic_viewer_routes_typed_mutations_and_fails_closed_on_event_gaps() {
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app);
        let session_id = configure_semantic_network(&network, &mut app);
        let first_mutation = mutation(1);
        let semantic_event = OrderedTerminalEvent {
            event_no: 0,
            event_type: OrderedTerminalEventType::SemanticConversationMutation {
                cursor: semantic_cursor(session_id, 1),
                mutation: first_mutation.encode_to_vec(),
            },
        };

        network.update(&mut app, |network, ctx| {
            let message = DownstreamMessage::OrderedTerminalEvent(semantic_event.clone());
            network.process_websocket_message(Message::new(message.to_json().unwrap()), ctx);
        });
        network.read(&app, |network, _| {
            assert_eq!(network.semantic_last_event_no, Some(0));
            assert_eq!(
                network
                    .semantic_consumer
                    .as_ref()
                    .and_then(SemanticConsumer::cursor)
                    .unwrap()
                    .mutation_sequence,
                1
            );
            assert!(matches!(network.stage, Stage::JoinedSuccessfully));
        });
        network.update(&mut app, |network, ctx| {
            let message = DownstreamMessage::OrderedTerminalEvent(semantic_event);
            network.process_websocket_message(Message::new(message.to_json().unwrap()), ctx);
        });
        network.read(&app, |network, _| {
            assert_eq!(network.semantic_last_event_no, Some(0));
            assert!(matches!(network.stage, Stage::JoinedSuccessfully));
        });

        network.update(&mut app, |network, ctx| {
            let message = DownstreamMessage::OrderedTerminalEvent(OrderedTerminalEvent {
                event_no: 1,
                event_type: OrderedTerminalEventType::SemanticConversationMutation {
                    cursor: semantic_cursor(session_id, 3),
                    mutation: mutation(3).encode_to_vec(),
                },
            });
            network.process_websocket_message(Message::new(message.to_json().unwrap()), ctx);
        });
        network.read(&app, |network, _| {
            assert!(matches!(network.stage, Stage::Finished));
        });
    });
}

#[test]
fn test_semantic_viewer_rejects_event_number_overflow() {
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app);
        let session_id = configure_semantic_network(&network, &mut app);
        network.update(&mut app, |network, ctx| {
            let message = DownstreamMessage::OrderedTerminalEvent(OrderedTerminalEvent {
                event_no: usize::MAX,
                event_type: OrderedTerminalEventType::SemanticConversationMutation {
                    cursor: semantic_cursor(session_id, u64::MAX),
                    mutation: mutation(u64::MAX).encode_to_vec(),
                },
            });
            network.process_websocket_message(Message::new(message.to_json().unwrap()), ctx);
        });
        network.read(&app, |network, _| {
            assert!(matches!(network.stage, Stage::Finished));
            assert!(network.semantic_last_event_no.is_none());
        });
    });
}

#[test]
fn test_send_pty_write_event_advances_event_no() {
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app);

        // Event number should start at 0.
        network.read(&app, |network, _ctx| {
            assert_eq!(network.write_to_pty_event_no.as_usize(), 0);
        });

        // Try to send a write to pty event message to the server.
        network.update(&mut app, |network, ctx| {
            let abort_handle = ctx.spawn_abortable(
                Timer::after(Duration::from_millis(1)),
                move |_, _, _| {},
                |_, _| {},
            );
            network.pty_bytes_batch_status = PtyBytesBatchStatus::Batching {
                accumulated: "a".into(),
                abort_handle,
            };
        });

        network.update(&mut app, |network, _| {
            network.send_write_to_pty();
        });

        // Event number is advanced to 1.
        network.read(&app, |network, _ctx| {
            assert_eq!(network.write_to_pty_event_no.as_usize(), 1);
        });
    });
}

#[test]
fn test_send_pty_write_event_while_batching() {
    App::test((), |mut app| async move {
        let (network, tx) = create_network(&mut app);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());
        let init_time = Instant::now();

        // Reset batching status.
        network.update(&mut app, |network, _ctx| {
            network.pty_bytes_batch_status = PtyBytesBatchStatus::NotBatching {
                last_sent_at: init_time,
            };
        });

        // Try to send write to pty events.
        tx.try_send("a".into())
            .expect("Can send event over write_to_pty_tx");
        tx.try_send("b".into())
            .expect("Can send event over write_to_pty_tx");

        // Ensure the accumulated event is sent to the server, and the item in ws_proxy_tx is correct.
        let item = ws_proxy_rx.recv().await;
        assert!(
            matches!(item.unwrap(), UpstreamMessage::WriteToPty { bytes, .. } if bytes == b"ab")
        );

        // The batch status should be updated.
        network.read(&app, |network, _ctx| {
            assert!(matches!(network.pty_bytes_batch_status, PtyBytesBatchStatus::NotBatching { last_sent_at } if last_sent_at > init_time));
        });
    });
}

#[test]
fn test_send_pty_write_event_while_not_batching() {
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());
        let init_time = Instant::now();

        // Set batch status to not batching.
        network.update(&mut app, |network, _ctx| {
            network.pty_bytes_batch_status = PtyBytesBatchStatus::NotBatching {
                last_sent_at: init_time,
            };
        });

        // Try to send write to pty message to server.
        network.update(&mut app, |network, _| {
            network.send_write_to_pty();
        });

        // Make sure we didn't try to send anything to the server.
        assert_eq!(ws_proxy_rx.len(), 0);

        // The batch status should be unchanged.
        network.read(&app, |network, _ctx| {
            assert!(matches!(network.pty_bytes_batch_status, PtyBytesBatchStatus::NotBatching { last_sent_at } if last_sent_at == init_time));
        });
    });
}
