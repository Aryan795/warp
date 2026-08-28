use prost::Message;
use session_sharing_protocol::common::{
    ExecutionIdentity, OrderedTerminalEvent, OrderedTerminalEventType, SemanticCursor,
    SemanticNegotiationError, SessionId,
};
use warp_conversation_mutation_api::{
    AcceptedMessageContext, Attribution, Author, ContentEncoding, ContentMutation,
    ConversationMutation, MediaReference, MutationIdentity, Origin, SchemaVersion,
    SourceDeliveryReference, action_mutation, author, content_mutation, conversation_mutation,
    entry_mutation, media_reference, origin,
};
use warp_multi_agent_api::{
    ClientAction, Message as AgentMessage, ResponseEvent, Task, client_action, message,
    response_event,
};

use super::*;

fn execution() -> ExecutionIdentity {
    ExecutionIdentity {
        conversation_id: "conversation".to_owned(),
        execution_id: "execution".to_owned(),
        run_id: Some("run".to_owned()),
        request_id: Some("request".to_owned()),
    }
}

fn timestamp() -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: 1_700_000_000,
        nanos: 123_456_789,
    }
}

fn accepted_context(message_id: &str) -> AcceptedMessageContext {
    AcceptedMessageContext {
        schema_version: SchemaVersion::V1 as i32,
        conversation_id: "conversation".to_owned(),
        execution_id: "execution".to_owned(),
        message_id: message_id.to_owned(),
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
            source_delivery: Some(SourceDeliveryReference {
                delivery_id: "delivery".to_owned(),
                channel_id: "channel".to_owned(),
                thread_id: "thread".to_owned(),
                message_id: "source-message".to_owned(),
            }),
        }),
        media: vec![MediaReference {
            media_id: "media".to_owned(),
            kind: media_reference::Kind::File as i32,
            mime_type: "text/plain".to_owned(),
            size_bytes: 12,
            sha256: vec![7; 32],
            display_name: "input.txt".to_owned(),
            reference: "artifact".to_owned(),
        }],
    }
}

#[test]
fn producer_binds_accepted_context_once_to_user_entry() {
    let context = accepted_context("canonical-user-message");
    let mut producer = SemanticMutationProducer::new_with_redactor(execution(), ToOwned::to_owned);
    producer
        .register_accepted_message_context(context.clone())
        .unwrap();
    assert_eq!(
        producer.register_accepted_message_context(context.clone()),
        Err(SemanticProducerError::DuplicateAcceptedMessageContext)
    );

    let response = client_actions(vec![ClientAction {
        action: Some(client_action::Action::AddMessagesToTask(
            client_action::AddMessagesToTask {
                task_id: "task".to_owned(),
                messages: vec![AgentMessage {
                    id: "server-user-message".to_owned(),
                    task_id: "task".to_owned(),
                    request_id: "request".to_owned(),
                    timestamp: Some(timestamp()),
                    message: Some(message::Message::UserQuery(message::UserQuery {
                        query: "hello".to_owned(),
                        ..Default::default()
                    })),
                    ..Default::default()
                }],
            },
        )),
    }]);
    let mutations = producer.normalize(&response).unwrap();
    assert_eq!(mutations.len(), 3);
    assert!(mutations.iter().all(|mutation| {
        mutation.attribution == context.attribution
            && mutation
                .occurred_at
                .as_ref()
                .is_some_and(super::validation::timestamp_is_valid)
    }));
    let Some(conversation_mutation::Mutation::Entry(entry)) = mutations[0].mutation.as_ref() else {
        panic!("first user mutation should create an entry");
    };
    assert_eq!(entry.entry_id, context.message_id);
    let Some(entry_mutation::Change::Created(created)) = entry.change.as_ref() else {
        panic!("first user mutation should create an entry");
    };
    assert_eq!(created.media, context.media);

    assert_eq!(
        producer.normalize(&response),
        Err(SemanticProducerError::MissingAcceptedMessageContext)
    );
}

#[test]
fn producer_rejects_stale_and_wrong_execution_context() {
    let mut producer = SemanticMutationProducer::new(execution());
    let mut wrong_execution = accepted_context("wrong-execution");
    wrong_execution.execution_id = "other".to_owned();
    assert!(matches!(
        producer.register_accepted_message_context(wrong_execution),
        Err(SemanticProducerError::InvalidAcceptedMessageContext(
            AcceptedMessageContextError::ExecutionMismatch
        ))
    ));

    producer
        .register_accepted_message_context(accepted_context("stale"))
        .unwrap();
    let finished = ResponseEvent {
        r#type: Some(response_event::Type::Finished(
            response_event::StreamFinished::default(),
        )),
    };
    assert_eq!(
        producer.normalize(&finished),
        Err(SemanticProducerError::StaleAcceptedMessageContext)
    );
}

#[test]
fn accepted_context_validation_rejects_invalid_nested_attribution_and_media() {
    let mut invalid_author = accepted_context("invalid-author");
    invalid_author
        .attribution
        .as_mut()
        .unwrap()
        .author
        .as_mut()
        .unwrap()
        .kind = author::Kind::Unspecified as i32;
    assert_eq!(
        validate_accepted_message_context(&invalid_author, &execution(), 1),
        Err(AcceptedMessageContextError::InvalidField(
            "attribution.author.kind"
        ))
    );

    let mut invalid_digest = accepted_context("invalid-digest");
    invalid_digest.media[0].sha256 = vec![1; 31];
    assert_eq!(
        validate_accepted_message_context(&invalid_digest, &execution(), 1),
        Err(AcceptedMessageContextError::InvalidField("media.sha256"))
    );

    let mut duplicate_media = accepted_context("duplicate-media");
    duplicate_media.media.push(duplicate_media.media[0].clone());
    assert_eq!(
        validate_accepted_message_context(&duplicate_media, &execution(), 1),
        Err(AcceptedMessageContextError::DuplicateMedia)
    );
}

#[test]
fn producer_emits_valid_nested_timestamps_and_unique_repeated_state_ids() {
    let mut producer = SemanticMutationProducer::new(execution());
    let init = ResponseEvent {
        r#type: Some(response_event::Type::Init(response_event::StreamInit {
            conversation_id: "conversation".to_owned(),
            request_id: "request".to_owned(),
            run_id: "run".to_owned(),
        })),
    };
    let started = producer.normalize(&init).unwrap().pop().unwrap();
    super::validation::validate_mutation_timestamps(&started).unwrap();

    let first_finished = ResponseEvent {
        r#type: Some(response_event::Type::Finished(
            response_event::StreamFinished::default(),
        )),
    };
    let first = producer.normalize(&first_finished).unwrap().pop().unwrap();
    let second = producer.normalize(&first_finished).unwrap().pop().unwrap();
    super::validation::validate_mutation_timestamps(&first).unwrap();
    super::validation::validate_mutation_timestamps(&second).unwrap();
    assert_ne!(
        first.identity.as_ref().unwrap().mutation_id,
        second.identity.as_ref().unwrap().mutation_id
    );

    let interruption = producer.interruption(
        warp_conversation_mutation_api::InterruptionReason::Disconnected,
        true,
    );
    super::validation::validate_mutation_timestamps(&interruption).unwrap();
    let Some(conversation_mutation::Mutation::Interruption(interruption_payload)) =
        interruption.mutation.as_ref()
    else {
        panic!("expected interruption mutation");
    };
    assert_eq!(
        interruption_payload.interrupted_at,
        interruption.occurred_at
    );
}

#[test]
fn encoded_context_and_producer_mutation_conform_through_transport() {
    let context = accepted_context("transport-user");
    let encoded_context = context.encode_to_vec();
    let decoded_context =
        decode_accepted_message_context(&encoded_context, &execution(), 1).unwrap();
    assert_eq!(decoded_context, context);

    let session_id = SessionId::new();
    let mut producer = SemanticMutationProducer::new(execution());
    let init = ResponseEvent {
        r#type: Some(response_event::Type::Init(response_event::StreamInit {
            conversation_id: "conversation".to_owned(),
            request_id: "request".to_owned(),
            run_id: "run".to_owned(),
        })),
    };
    let mutation = producer.normalize(&init).unwrap().pop().unwrap();
    let encoded_mutation = mutation.encode_to_vec();
    let message = session_sharing_protocol::sharer::UpstreamMessage::OrderedTerminalEvent(
        OrderedTerminalEvent {
            event_no: 0,
            event_type: OrderedTerminalEventType::SemanticConversationMutation {
                cursor: cursor(session_id, 1),
                mutation: encoded_mutation.clone(),
            },
        },
    );
    let json = message.to_json().unwrap();
    let decoded = session_sharing_protocol::sharer::UpstreamMessage::from_json(&json).unwrap();
    let session_sharing_protocol::sharer::UpstreamMessage::OrderedTerminalEvent(event) = decoded
    else {
        panic!("semantic mutation should remain an ordered event");
    };
    let mut consumer = SemanticConsumer::new(session_id, execution(), 1);
    let ConsumeOutcome::Applied(applied) = consumer.consume(event).unwrap() else {
        panic!("transported semantic mutation should apply");
    };
    assert_eq!(applied.encode_to_vec(), encoded_mutation);
}

#[test]
fn producer_redacts_visible_content_before_encoding() {
    let mut producer = SemanticMutationProducer::new_with_redactor(execution(), |text| {
        text.replace("sk-live-secret", "[REDACTED]")
    });
    let response = client_actions(vec![ClientAction {
        action: Some(client_action::Action::CreateTask(
            client_action::CreateTask {
                task: Some(Task {
                    id: "task".to_owned(),
                    messages: vec![AgentMessage {
                        id: "assistant".to_owned(),
                        task_id: "task".to_owned(),
                        request_id: "request".to_owned(),
                        message: Some(message::Message::AgentOutput(message::AgentOutput {
                            text: "prefix sk-live-secret suffix".to_owned(),
                        })),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            },
        )),
    }]);

    let mutations = producer.normalize(&response).unwrap();
    assert!(mutations.iter().all(|mutation| {
        !mutation
            .encode_to_vec()
            .windows("sk-live-secret".len())
            .any(|window| window == b"sk-live-secret")
    }));
    let attribution = mutations[0].attribution.as_ref().unwrap();
    assert_eq!(attribution.author.as_ref().unwrap().id, "task");
    assert_eq!(attribution.origin.as_ref().unwrap().source_id, "run");
}

#[test]
fn producer_applies_append_masks_and_derives_stable_mutation_ids() {
    let mut producer = SemanticMutationProducer::new_with_redactor(execution(), ToOwned::to_owned);
    let add = client_actions(vec![ClientAction {
        action: Some(client_action::Action::AddMessagesToTask(
            client_action::AddMessagesToTask {
                task_id: "task".to_owned(),
                messages: vec![AgentMessage {
                    id: "assistant".to_owned(),
                    task_id: "task".to_owned(),
                    request_id: "request".to_owned(),
                    message: Some(message::Message::AgentOutput(message::AgentOutput {
                        text: "hello".to_owned(),
                    })),
                    ..Default::default()
                }],
            },
        )),
    }]);
    let initial = producer.normalize(&add).unwrap();
    assert_eq!(
        initial[0].identity.as_ref().unwrap().mutation_id,
        "execution:00000000000000000001:entry:assistant:created"
    );

    let append = client_actions(vec![ClientAction {
        action: Some(client_action::Action::AppendToMessageContent(
            client_action::AppendToMessageContent {
                task_id: "task".to_owned(),
                message: Some(AgentMessage {
                    id: "assistant".to_owned(),
                    message: Some(message::Message::AgentOutput(message::AgentOutput {
                        text: " world".to_owned(),
                    })),
                    ..Default::default()
                }),
                mask: Some(prost_types::FieldMask {
                    paths: vec!["agent_output.text".to_owned()],
                }),
            },
        )),
    }]);
    let mutations = producer.normalize(&append).unwrap();
    let identity = mutations[0].identity.as_ref().unwrap();
    assert!(
        identity
            .mutation_id
            .contains("content:assistant:content:0:delta:1:")
    );
}

#[test]
fn producer_projects_tool_results_without_phantom_result_entries() {
    let mut producer = SemanticMutationProducer::new_with_redactor(execution(), ToOwned::to_owned);
    let call = client_actions(vec![ClientAction {
        action: Some(client_action::Action::AddMessagesToTask(
            client_action::AddMessagesToTask {
                task_id: "task".to_owned(),
                messages: vec![AgentMessage {
                    id: "tool-entry".to_owned(),
                    task_id: "task".to_owned(),
                    message: Some(message::Message::ToolCall(message::ToolCall {
                        tool_call_id: "tool-call".to_owned(),
                        tool: Some(message::tool_call::Tool::RunShellCommand(
                            message::tool_call::RunShellCommand::default(),
                        )),
                    })),
                    ..Default::default()
                }],
            },
        )),
    }]);
    producer.normalize(&call).unwrap();
    let result = client_actions(vec![ClientAction {
        action: Some(client_action::Action::AddMessagesToTask(
            client_action::AddMessagesToTask {
                task_id: "task".to_owned(),
                messages: vec![AgentMessage {
                    id: "result-entry".to_owned(),
                    message: Some(message::Message::ToolCallResult(message::ToolCallResult {
                        tool_call_id: "tool-call".to_owned(),
                        result: Some(message::tool_call_result::Result::Cancel(())),
                        ..Default::default()
                    })),
                    ..Default::default()
                }],
            },
        )),
    }]);

    let mutations = producer.normalize(&result).unwrap();
    assert_eq!(mutations.len(), 2);
    assert!(
        mutations
            .iter()
            .all(|mutation| match mutation.mutation.as_ref() {
                Some(conversation_mutation::Mutation::Entry(entry)) =>
                    entry.entry_id == "tool-entry",
                Some(conversation_mutation::Mutation::Action(action)) => {
                    action.entry_id == "tool-entry"
                }
                _ => false,
            })
    );
}

fn cursor(session_id: SessionId, mutation_sequence: u64) -> SemanticCursor {
    SemanticCursor {
        session_id,
        conversation_id: "conversation".to_owned(),
        execution_id: "execution".to_owned(),
        content_mode:
            session_sharing_protocol::common::SessionContentMode::SemanticConversationOnly,
        schema_version: 1,
        mutation_sequence,
    }
}

fn mutation(sequence: u64, mutation_id: &str) -> ConversationMutation {
    ConversationMutation {
        schema_version: SchemaVersion::V1 as i32,
        identity: Some(MutationIdentity {
            conversation_id: "conversation".to_owned(),
            execution_id: "execution".to_owned(),
            mutation_id: mutation_id.to_owned(),
            sequence,
            run_id: "run".to_owned(),
            request_id: "request".to_owned(),
        }),
        attribution: None,
        occurred_at: Some(timestamp()),
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

#[test]
fn semantic_negotiation_fails_closed_without_echo() {
    let requested = RequestedSessionContent::semantic_v1(execution());
    assert_eq!(
        requested.validate_echo(None),
        Err(SemanticNegotiationError::MissingServerEcho)
    );
}

#[test]
fn consumer_applies_contiguous_events_and_dedupes_exact_replay() {
    let session_id = SessionId::new();
    let mut consumer = SemanticConsumer::new(session_id, execution(), 1);
    let first = mutation(1, "mutation-1");
    let event = OrderedTerminalEvent {
        event_no: 0,
        event_type: OrderedTerminalEventType::SemanticConversationMutation {
            cursor: cursor(session_id, 1),
            mutation: first.encode_to_vec(),
        },
    };
    assert!(matches!(
        consumer.consume(event.clone()).unwrap(),
        ConsumeOutcome::Applied(_)
    ));
    assert_eq!(consumer.consume(event).unwrap(), ConsumeOutcome::Duplicate);
    assert_eq!(consumer.cursor().unwrap().mutation_sequence, 1);
    for sequence in 2..=4 {
        let event = OrderedTerminalEvent {
            event_no: (sequence - 1) as usize,
            event_type: OrderedTerminalEventType::SemanticConversationMutation {
                cursor: cursor(session_id, sequence),
                mutation: mutation(sequence, &format!("mutation-{sequence}")).encode_to_vec(),
            },
        };
        assert!(matches!(
            consumer.consume(event).unwrap(),
            ConsumeOutcome::Applied(_)
        ));
        assert_eq!(consumer.cursor().unwrap().mutation_sequence, sequence);
    }
}

#[test]
fn consumer_requests_resync_for_gap_and_conflicting_duplicate() {
    let session_id = SessionId::new();
    let mut consumer = SemanticConsumer::new(session_id, execution(), 1);
    let gap = mutation(2, "mutation-2");
    let gap_outcome = consumer
        .consume_mutation(cursor(session_id, 2), &gap.encode_to_vec())
        .unwrap();
    assert!(matches!(
        gap_outcome,
        ConsumeOutcome::ResyncRequired(
            session_sharing_protocol::common::SemanticResyncReason::ReplayGap {
                expected_sequence: 1,
                next_available_sequence: 2
            }
        )
    ));

    let first = mutation(1, "mutation-1");
    consumer
        .consume_mutation(cursor(session_id, 1), &first.encode_to_vec())
        .unwrap();
    let conflicting = mutation(1, "different-id");
    assert!(matches!(
        consumer
            .consume_mutation(cursor(session_id, 1), &conflicting.encode_to_vec(),)
            .unwrap(),
        ConsumeOutcome::ResyncRequired(
            session_sharing_protocol::common::SemanticResyncReason::ConflictingDuplicate {
                mutation_sequence: 1
            }
        )
    ));

    let mut conflicting_payload = first;
    let Some(conversation_mutation::Mutation::Content(content)) =
        conflicting_payload.mutation.as_mut()
    else {
        panic!("test mutation should contain content");
    };
    let Some(content_mutation::Change::Delta(delta)) = content.change.as_mut() else {
        panic!("test mutation should contain a delta");
    };
    delta.data = b"different".to_vec();
    assert!(matches!(
        consumer
            .consume_mutation(cursor(session_id, 1), &conflicting_payload.encode_to_vec(),)
            .unwrap(),
        ConsumeOutcome::ResyncRequired(
            session_sharing_protocol::common::SemanticResyncReason::ConflictingDuplicate {
                mutation_sequence: 1
            }
        )
    ));
}

fn client_actions(actions: Vec<ClientAction>) -> ResponseEvent {
    ResponseEvent {
        r#type: Some(response_event::Type::ClientActions(
            response_event::ClientActions { actions },
        )),
    }
}

#[test]
fn producer_redacts_tool_input() {
    let mut producer = SemanticMutationProducer::new(execution());
    let response = client_actions(vec![ClientAction {
        action: Some(client_action::Action::AddMessagesToTask(
            client_action::AddMessagesToTask {
                task_id: "task".to_owned(),
                messages: vec![AgentMessage {
                    id: "tool-entry".to_owned(),
                    message: Some(message::Message::ToolCall(message::ToolCall {
                        tool_call_id: "tool-call".to_owned(),
                        tool: Some(message::tool_call::Tool::RunShellCommand(
                            message::tool_call::RunShellCommand {
                                command: "printf super-secret".to_owned(),
                                ..Default::default()
                            },
                        )),
                    })),
                    ..Default::default()
                }],
            },
        )),
    }]);

    let mutations = producer.normalize(&response).unwrap();
    let action = mutations
        .iter()
        .find_map(|mutation| match mutation.mutation.as_ref() {
            Some(conversation_mutation::Mutation::Action(action)) => Some(action),
            _ => None,
        })
        .expect("tool call produces an action mutation");
    let Some(action_mutation::Change::Started(started)) = action.change.as_ref() else {
        panic!("tool action should be started");
    };
    assert_eq!(started.input_json, b"{}");
    assert!(mutations.iter().all(|mutation| {
        !mutation
            .encode_to_vec()
            .windows(12)
            .any(|w| w == b"super-secret")
    }));
}

#[test]
fn transcript_reducer_applies_entries_and_rejects_delta_gaps() {
    let mut transcript = SemanticTranscript::default();
    let created = ConversationMutation {
        mutation: Some(conversation_mutation::Mutation::Entry(
            warp_conversation_mutation_api::EntryMutation {
                entry_id: "assistant".to_owned(),
                change: Some(entry_mutation::Change::Created(entry_mutation::Created {
                    kind: warp_conversation_mutation_api::EntryKind::AssistantMessage as i32,
                    ..Default::default()
                })),
            },
        )),
        ..Default::default()
    };
    transcript.apply(&created).unwrap();
    let mut content = mutation(1, "content");
    let Some(conversation_mutation::Mutation::Content(content_mutation)) =
        content.mutation.as_mut()
    else {
        panic!("test mutation should contain content");
    };
    content_mutation.entry_id = "assistant".to_owned();
    transcript.apply(&content).unwrap();
    assert_eq!(transcript.entries["assistant"].contents[&0].data, b"text");

    let mut gap = content;
    let Some(conversation_mutation::Mutation::Content(content_mutation)) = gap.mutation.as_mut()
    else {
        panic!("test mutation should contain content");
    };
    let Some(content_mutation::Change::Delta(delta)) = content_mutation.change.as_mut() else {
        panic!("test mutation should contain delta");
    };
    delta.delta_index = 2;
    assert_eq!(
        transcript.apply(&gap),
        Err(SemanticReducerError::DeltaIndexMismatch)
    );
}

#[test]
fn producer_rejects_unsupported_message_without_raw_fallback() {
    let mut producer = SemanticMutationProducer::new(execution());
    let response = client_actions(vec![ClientAction {
        action: Some(client_action::Action::AddMessagesToTask(
            client_action::AddMessagesToTask {
                task_id: "task".to_owned(),
                messages: vec![AgentMessage {
                    id: "system".to_owned(),
                    message: Some(message::Message::SystemQuery(
                        message::SystemQuery::default(),
                    )),
                    ..Default::default()
                }],
            },
        )),
    }]);

    assert_eq!(
        producer.normalize(&response),
        Err(SemanticProducerError::UnsupportedMessage("system_query"))
    );
}

#[test]
fn producer_rejects_stream_identity_mismatch() {
    let mut producer = SemanticMutationProducer::new(execution());
    let response = ResponseEvent {
        r#type: Some(response_event::Type::Init(response_event::StreamInit {
            conversation_id: "different".to_owned(),
            request_id: "request".to_owned(),
            run_id: "run".to_owned(),
        })),
    };
    assert_eq!(
        producer.normalize(&response),
        Err(SemanticProducerError::StreamIdentityMismatch)
    );
}

#[test]
fn consumer_requests_resync_for_session_and_execution_mismatch() {
    let session_id = SessionId::new();
    let mut consumer = SemanticConsumer::new(session_id, execution(), 1);
    let first = mutation(1, "mutation-1").encode_to_vec();

    assert!(matches!(
        consumer
            .consume_mutation(
                SemanticCursor {
                    session_id: SessionId::new(),
                    ..cursor(session_id, 1)
                },
                &first,
            )
            .unwrap(),
        ConsumeOutcome::ResyncRequired(
            session_sharing_protocol::common::SemanticResyncReason::CursorSessionMismatch
        )
    ));
    assert!(matches!(
        consumer
            .consume_mutation(
                SemanticCursor {
                    session_id,
                    execution_id: "different".to_owned(),
                    ..cursor(session_id, 1)
                },
                &first,
            )
            .unwrap(),
        ConsumeOutcome::ResyncRequired(
            session_sharing_protocol::common::SemanticResyncReason::CursorExecutionMismatch
        )
    ));
}
