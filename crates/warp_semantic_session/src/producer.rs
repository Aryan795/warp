use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use warp_conversation_mutation_api::{
    AcceptedMessageContext, ActionKind, ActionMutation, ActionState, Attribution, Author,
    ContentEncoding, ContentMutation, ConversationMutation, EntryKind, EntryMutation, EntryState,
    ExecutionMutation, ExecutionState, InterruptionMutation, InterruptionReason, InterruptionScope,
    MutationIdentity, Origin, SchemaVersion, action_mutation, content_mutation,
    conversation_mutation, entry_mutation, execution_mutation,
};
use warp_multi_agent_api::{Message, ResponseEvent, client_action, message, response_event};

use crate::validation::{AcceptedMessageContextError, timestamp_is_valid};

const MAX_TRACKED_MESSAGES: usize = 4_096;
const MAX_PENDING_MESSAGE_CONTEXTS: usize = 64;

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum SemanticProducerError {
    #[error("response event did not contain a type")]
    MissingResponseType,
    #[error("stream identity did not match requested semantic execution")]
    StreamIdentityMismatch,
    #[error("client action is not supported by semantic sessions: {0}")]
    UnsupportedClientAction(&'static str),
    #[error("message is not supported by semantic sessions: {0}")]
    UnsupportedMessage(&'static str),
    #[error("message did not contain a type")]
    MissingMessageType,
    #[error("tool result did not match a previously observed tool call")]
    UnknownToolResult,
    #[error("message update referenced an unknown message")]
    UnknownMessage,
    #[error("message update used an unsupported field mask path: {0}")]
    UnsupportedFieldMask(String),
    #[error("semantic producer exceeded its tracked-message limit")]
    StateLimit,
    #[error("semantic user message did not have an accepted message context")]
    MissingAcceptedMessageContext,
    #[error("accepted message context was already registered")]
    DuplicateAcceptedMessageContext,
    #[error("semantic execution finished with an unconsumed accepted message context")]
    StaleAcceptedMessageContext,
    #[error("semantic producer exceeded its pending accepted-message-context limit")]
    AcceptedMessageContextLimit,
    #[error("invalid accepted message context: {0}")]
    InvalidAcceptedMessageContext(#[from] AcceptedMessageContextError),
}
fn canonical_timestamp(preferred: Option<&prost_types::Timestamp>) -> prost_types::Timestamp {
    if let Some(preferred) = preferred.filter(|timestamp| timestamp_is_valid(timestamp)) {
        return *preferred;
    }
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    prost_types::Timestamp {
        seconds: i64::try_from(duration.as_secs().min(253_402_300_799)).unwrap_or_default(),
        nanos: duration.subsec_nanos() as i32,
    }
}

impl PendingMutation {
    fn entry_id(&self) -> Option<&str> {
        match self {
            Self::Entry(value) => Some(&value.entry_id),
            Self::Content(value) => Some(&value.entry_id),
            Self::Action(value) => Some(&value.entry_id),
            Self::Execution(_) | Self::Interruption(_) => None,
        }
    }

    fn stable_source_key(&self) -> String {
        match self {
            Self::Execution(value) => match value.change.as_ref() {
                Some(execution_mutation::Change::Started(_)) => "execution:started".to_owned(),
                Some(execution_mutation::Change::StateChanged(change)) => {
                    format!("execution:state:{}", change.state)
                }
                Some(execution_mutation::Change::Finished(change)) => {
                    format!("execution:finished:{}", change.state)
                }
                None => "execution:missing".to_owned(),
            },
            Self::Entry(value) => match value.change.as_ref() {
                Some(entry_mutation::Change::Created(_)) => {
                    format!("entry:{}:created", value.entry_id)
                }
                Some(entry_mutation::Change::Finalized(change)) => {
                    format!("entry:{}:finalized:{}", value.entry_id, change.state)
                }
                None => format!("entry:{}:missing", value.entry_id),
            },
            Self::Content(value) => match value.change.as_ref() {
                Some(content_mutation::Change::Delta(change)) => format!(
                    "content:{}:delta:{}:{:016x}",
                    value.content_id,
                    change.delta_index,
                    stable_hash(&change.data)
                ),
                Some(content_mutation::Change::Final(change)) => format!(
                    "content:{}:final:{:016x}",
                    value.content_id,
                    stable_hash(&change.data)
                ),
                None => format!("content:{}:missing", value.content_id),
            },
            Self::Action(value) => match value.change.as_ref() {
                Some(action_mutation::Change::Started(_)) => {
                    format!("action:{}:started", value.action_id)
                }
                Some(action_mutation::Change::StateChanged(change)) => {
                    format!("action:{}:state:{}", value.action_id, change.state)
                }
                Some(action_mutation::Change::Result(result)) => {
                    format!("action:{}:result:{}", value.action_id, result.state)
                }
                None => format!("action:{}:missing", value.action_id),
            },
            Self::Interruption(value) => {
                format!(
                    "interruption:{}:{}:{}",
                    value.scope, value.target_id, value.reason
                )
            }
        }
    }
}

fn apply_message_field(
    target: &mut Message,
    source: &Message,
    path: &str,
    append: bool,
) -> Result<(), SemanticProducerError> {
    let path = path.strip_prefix("message.").unwrap_or(path);
    match (path, target.message.as_mut(), source.message.as_ref()) {
        (
            "user_query.query",
            Some(message::Message::UserQuery(target)),
            Some(message::Message::UserQuery(source)),
        ) => update_string(&mut target.query, &source.query, append),
        (
            "agent_output.text",
            Some(message::Message::AgentOutput(target)),
            Some(message::Message::AgentOutput(source)),
        ) => update_string(&mut target.text, &source.text, append),
        (
            "agent_reasoning.reasoning",
            Some(message::Message::AgentReasoning(target)),
            Some(message::Message::AgentReasoning(source)),
        ) => update_string(&mut target.reasoning, &source.reasoning, append),
        ("timestamp", _, _) if !append => target.timestamp = source.timestamp,
        _ => {
            return Err(SemanticProducerError::UnsupportedFieldMask(path.to_owned()));
        }
    }
    Ok(())
}

fn update_string(target: &mut String, source: &str, append: bool) {
    if append {
        target.push_str(source);
    } else {
        target.clear();
        target.push_str(source);
    }
}

fn attribution_for_message(message: &Message, kind: EntryKind, run_id: &str) -> Attribution {
    let is_user = kind == EntryKind::UserMessage;
    let author_id = if is_user {
        message.request_id.as_str()
    } else {
        message.task_id.as_str()
    };
    Attribution {
        author: Some(Author {
            id: if author_id.is_empty() {
                run_id.to_owned()
            } else {
                author_id.to_owned()
            },
            kind: if is_user {
                warp_conversation_mutation_api::author::Kind::User as i32
            } else {
                warp_conversation_mutation_api::author::Kind::Agent as i32
            },
            display_name: String::new(),
        }),
        origin: Some(Origin {
            kind: warp_conversation_mutation_api::origin::Kind::Warp as i32,
            source_id: run_id.to_owned(),
            subtype: "multi_agent".to_owned(),
        }),
        source_delivery: None,
    }
}

fn system_attribution(run_id: &str) -> Attribution {
    Attribution {
        author: Some(Author {
            id: run_id.to_owned(),
            kind: warp_conversation_mutation_api::author::Kind::System as i32,
            display_name: String::new(),
        }),
        origin: Some(Origin {
            kind: warp_conversation_mutation_api::origin::Kind::Warp as i32,
            source_id: run_id.to_owned(),
            subtype: "multi_agent".to_owned(),
        }),
        source_delivery: None,
    }
}

fn stable_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

pub struct SemanticMutationProducer {
    execution_identity: session_sharing_protocol::common::ExecutionIdentity,
    next_sequence: u64,
    request_id: String,
    run_id: String,
    open_entries: HashSet<String>,
    action_entries: HashMap<String, String>,
    messages: HashMap<String, Message>,
    entry_ids: HashMap<String, String>,
    entry_attributions: HashMap<String, Attribution>,
    content_delta_indexes: HashMap<String, u64>,
    pending_message_contexts: VecDeque<AcceptedMessageContext>,
    accepted_message_ids: HashSet<String>,
    accepted_message_id_order: VecDeque<String>,
    redact: Arc<dyn Fn(&str) -> String + Send + Sync>,
}

impl SemanticMutationProducer {
    pub fn new(execution_identity: session_sharing_protocol::common::ExecutionIdentity) -> Self {
        Self::new_with_redactor(execution_identity, |_| "[REDACTED]".to_owned())
    }

    pub fn new_with_redactor(
        execution_identity: session_sharing_protocol::common::ExecutionIdentity,
        redact: impl Fn(&str) -> String + Send + Sync + 'static,
    ) -> Self {
        Self {
            request_id: execution_identity.request_id.clone().unwrap_or_default(),
            run_id: execution_identity.run_id.clone().unwrap_or_default(),
            execution_identity,
            next_sequence: 1,
            open_entries: HashSet::new(),
            action_entries: HashMap::new(),
            messages: HashMap::new(),
            entry_ids: HashMap::new(),
            entry_attributions: HashMap::new(),
            content_delta_indexes: HashMap::new(),
            pending_message_contexts: VecDeque::new(),
            accepted_message_ids: HashSet::new(),
            accepted_message_id_order: VecDeque::new(),
            redact: Arc::new(redact),
        }
    }

    pub fn register_accepted_message_context(
        &mut self,
        context: AcceptedMessageContext,
    ) -> Result<(), SemanticProducerError> {
        crate::validate_accepted_message_context(
            &context,
            &self.execution_identity,
            SchemaVersion::V1 as u32,
        )?;
        if self.accepted_message_ids.contains(&context.message_id) {
            return Err(SemanticProducerError::DuplicateAcceptedMessageContext);
        }
        if self.pending_message_contexts.len() >= MAX_PENDING_MESSAGE_CONTEXTS {
            return Err(SemanticProducerError::AcceptedMessageContextLimit);
        }
        if self.accepted_message_id_order.len() >= MAX_TRACKED_MESSAGES
            && let Some(expired) = self.accepted_message_id_order.pop_front()
        {
            self.accepted_message_ids.remove(&expired);
        }
        self.accepted_message_ids.insert(context.message_id.clone());
        self.accepted_message_id_order
            .push_back(context.message_id.clone());
        self.pending_message_contexts.push_back(context);
        Ok(())
    }

    pub fn normalize(
        &mut self,
        response: &ResponseEvent,
    ) -> Result<Vec<ConversationMutation>, SemanticProducerError> {
        let pending = match response
            .r#type
            .as_ref()
            .ok_or(SemanticProducerError::MissingResponseType)?
        {
            response_event::Type::Init(init) => {
                if init.conversation_id != self.execution_identity.conversation_id
                    || self
                        .execution_identity
                        .request_id
                        .as_ref()
                        .is_some_and(|request_id| request_id != &init.request_id)
                    || self
                        .execution_identity
                        .run_id
                        .as_ref()
                        .is_some_and(|run_id| run_id != &init.run_id)
                {
                    return Err(SemanticProducerError::StreamIdentityMismatch);
                }
                self.request_id.clone_from(&init.request_id);
                self.run_id.clone_from(&init.run_id);
                vec![PendingMutation::Execution(ExecutionMutation {
                    change: Some(execution_mutation::Change::Started(
                        execution_mutation::Started { started_at: None },
                    )),
                })]
            }
            response_event::Type::ClientActions(actions) => {
                self.normalize_client_actions(&actions.actions)?
            }
            response_event::Type::Finished(finished) => self.normalize_finished(finished)?,
        };
        Ok(pending
            .into_iter()
            .map(|mutation| self.finish_mutation(mutation))
            .collect())
    }

    pub fn interruption(
        &mut self,
        reason: InterruptionReason,
        recoverable: bool,
    ) -> ConversationMutation {
        self.finish_mutation(PendingMutation::Interruption(InterruptionMutation {
            scope: InterruptionScope::Execution as i32,
            target_id: self.execution_identity.execution_id.clone(),
            reason: reason as i32,
            recoverable,
            detail: String::new(),
            interrupted_at: None,
        }))
    }

    fn normalize_client_actions(
        &mut self,
        actions: &[warp_multi_agent_api::ClientAction],
    ) -> Result<Vec<PendingMutation>, SemanticProducerError> {
        let mut mutations = Vec::new();
        for action in actions {
            match action
                .action
                .as_ref()
                .ok_or(SemanticProducerError::UnsupportedClientAction(
                    "missing_action",
                ))? {
                client_action::Action::CreateTask(create) => {
                    let task = create.task.as_ref().ok_or(
                        SemanticProducerError::UnsupportedClientAction("create_task_without_task"),
                    )?;
                    for message in &task.messages {
                        mutations.extend(self.normalize_added_message(message)?);
                    }
                }
                client_action::Action::UpdateTaskSummary(_)
                | client_action::Action::UpdateTaskDescription(_)
                | client_action::Action::BeginTransaction(_)
                | client_action::Action::CommitTransaction(_)
                | client_action::Action::UpdateTaskServerData(_) => {}
                client_action::Action::AddMessagesToTask(add) => {
                    for message in &add.messages {
                        mutations.extend(self.normalize_added_message(message)?);
                    }
                }
                client_action::Action::UpdateTaskMessage(update) => {
                    mutations.extend(self.normalize_updated_message(update)?);
                }
                client_action::Action::AppendToMessageContent(append) => {
                    mutations.push(self.normalize_appended_content(append)?);
                }
                client_action::Action::RollbackTransaction(_) => {
                    return Err(SemanticProducerError::UnsupportedClientAction(
                        "rollback_transaction",
                    ));
                }
                client_action::Action::ShowSuggestions(_) => {
                    return Err(SemanticProducerError::UnsupportedClientAction(
                        "show_suggestions",
                    ));
                }
                client_action::Action::StartNewConversation(_) => {
                    return Err(SemanticProducerError::UnsupportedClientAction(
                        "start_new_conversation",
                    ));
                }
                client_action::Action::MoveMessagesToNewTask(_) => {
                    return Err(SemanticProducerError::UnsupportedClientAction(
                        "move_messages_to_new_task",
                    ));
                }
            }
        }
        Ok(mutations)
    }
    fn normalize_updated_message(
        &mut self,
        update: &client_action::UpdateTaskMessage,
    ) -> Result<Vec<PendingMutation>, SemanticProducerError> {
        let update_message = update
            .message
            .as_ref()
            .ok_or(SemanticProducerError::MissingMessageType)?;
        let mut message = self
            .messages
            .get(&update_message.id)
            .cloned()
            .ok_or(SemanticProducerError::UnknownMessage)?;
        let paths = update
            .mask
            .as_ref()
            .map(|mask| mask.paths.as_slice())
            .unwrap_or_default();
        if paths.is_empty() {
            message = update_message.clone();
        } else {
            for path in paths {
                apply_message_field(&mut message, update_message, path, false)?;
            }
        }
        let mutations = self.normalize_final_message(&message)?;
        self.messages.insert(message.id.clone(), message);
        Ok(mutations)
    }

    fn normalize_appended_content(
        &mut self,
        append: &client_action::AppendToMessageContent,
    ) -> Result<PendingMutation, SemanticProducerError> {
        let fragment = append
            .message
            .as_ref()
            .ok_or(SemanticProducerError::MissingMessageType)?;
        let paths = append
            .mask
            .as_ref()
            .map(|mask| mask.paths.as_slice())
            .unwrap_or_default();
        if paths.len() != 1 {
            return Err(SemanticProducerError::UnsupportedFieldMask(paths.join(",")));
        }
        let mut message = self
            .messages
            .get(&fragment.id)
            .cloned()
            .ok_or(SemanticProducerError::UnknownMessage)?;
        apply_message_field(&mut message, fragment, &paths[0], true)?;
        self.messages.insert(message.id.clone(), message);
        self.normalize_content_delta(fragment)
    }

    fn normalize_added_message(
        &mut self,
        message: &Message,
    ) -> Result<Vec<PendingMutation>, SemanticProducerError> {
        let message_type = message
            .message
            .as_ref()
            .ok_or(SemanticProducerError::MissingMessageType)?;
        if let message::Message::ToolCallResult(result) = message_type {
            let entry_id = self
                .action_entries
                .get(&result.tool_call_id)
                .cloned()
                .ok_or(SemanticProducerError::UnknownToolResult)?;
            return Ok(vec![
                PendingMutation::Action(ActionMutation {
                    entry_id: entry_id.clone(),
                    action_id: result.tool_call_id.clone(),
                    change: Some(action_mutation::Change::Result(action_mutation::Result {
                        state: tool_result_state(result) as i32,
                        output_json: b"{}".to_vec(),
                        media: Vec::new(),
                        error_message: String::new(),
                        finished_at: message.timestamp,
                    })),
                }),
                finalize_entry(entry_id, message.timestamp),
            ]);
        }
        if !self.messages.contains_key(&message.id) && self.messages.len() >= MAX_TRACKED_MESSAGES {
            return Err(SemanticProducerError::StateLimit);
        }
        let (kind, keep_open) = match message_type {
            message::Message::UserQuery(_) => (EntryKind::UserMessage, false),
            message::Message::AgentOutput(_) => (EntryKind::AssistantMessage, true),
            message::Message::AgentReasoning(_) => (EntryKind::Reasoning, true),
            message::Message::ToolCall(tool_call) => {
                self.action_entries
                    .insert(tool_call.tool_call_id.clone(), message.id.clone());
                (EntryKind::Action, true)
            }
            message::Message::ToolCallResult(_) => unreachable!("handled above"),
            message::Message::SystemQuery(_) => {
                return Err(SemanticProducerError::UnsupportedMessage("system_query"));
            }
            message::Message::ServerEvent(_) => {
                return Err(SemanticProducerError::UnsupportedMessage("server_event"));
            }
            message::Message::UpdateTodos(_) => {
                return Err(SemanticProducerError::UnsupportedMessage("update_todos"));
            }
            message::Message::Summarization(_) => {
                return Err(SemanticProducerError::UnsupportedMessage("summarization"));
            }
            message::Message::CodeReview(_) => {
                return Err(SemanticProducerError::UnsupportedMessage("code_review"));
            }
            message::Message::UpdateReviewComments(_) => {
                return Err(SemanticProducerError::UnsupportedMessage(
                    "update_review_comments",
                ));
            }
            message::Message::WebSearch(_) => {
                return Err(SemanticProducerError::UnsupportedMessage("web_search"));
            }
            message::Message::WebFetch(_) => {
                return Err(SemanticProducerError::UnsupportedMessage("web_fetch"));
            }
            message::Message::DebugOutput(_) => {
                return Err(SemanticProducerError::UnsupportedMessage("debug_output"));
            }
            message::Message::ArtifactEvent(_) => {
                return Err(SemanticProducerError::UnsupportedMessage("artifact_event"));
            }
            message::Message::InvokeSkill(_) => {
                return Err(SemanticProducerError::UnsupportedMessage("invoke_skill"));
            }
            message::Message::MessagesReceivedFromAgents(_) => {
                return Err(SemanticProducerError::UnsupportedMessage(
                    "messages_received_from_agents",
                ));
            }
            message::Message::ModelUsed(_) => {
                return Err(SemanticProducerError::UnsupportedMessage("model_used"));
            }
            message::Message::EventsFromAgents(_) => {
                return Err(SemanticProducerError::UnsupportedMessage(
                    "events_from_agents",
                ));
            }
            message::Message::PassiveSuggestionResult(_) => {
                return Err(SemanticProducerError::UnsupportedMessage(
                    "passive_suggestion_result",
                ));
            }
            message::Message::OrchestrationConfigSnapshot(_) => {
                return Err(SemanticProducerError::UnsupportedMessage(
                    "orchestration_config_snapshot",
                ));
            }
        };
        let accepted_context = if kind == EntryKind::UserMessage {
            Some(
                self.pending_message_contexts
                    .pop_front()
                    .ok_or(SemanticProducerError::MissingAcceptedMessageContext)?,
            )
        } else {
            None
        };
        let entry_id = accepted_context
            .as_ref()
            .map(|context| context.message_id.clone())
            .unwrap_or_else(|| message.id.clone());
        let attribution = accepted_context
            .as_ref()
            .and_then(|context| context.attribution.clone())
            .unwrap_or_else(|| attribution_for_message(message, kind, &self.run_id));
        let media = accepted_context
            .as_ref()
            .map(|context| context.media.clone())
            .unwrap_or_default();

        self.messages.insert(message.id.clone(), message.clone());
        self.entry_ids.insert(message.id.clone(), entry_id.clone());
        self.entry_attributions
            .insert(entry_id.clone(), attribution);
        let mut mutations = vec![PendingMutation::Entry(EntryMutation {
            entry_id: entry_id.clone(),
            change: Some(entry_mutation::Change::Created(entry_mutation::Created {
                kind: kind as i32,
                parent_entry_id: String::new(),
                created_at: message.timestamp,
                media,
            })),
        })];
        match message_type {
            message::Message::UserQuery(_)
            | message::Message::AgentOutput(_)
            | message::Message::AgentReasoning(_) => {
                mutations.push(self.normalize_content(message, &entry_id, keep_open)?);
            }
            message::Message::ToolCall(tool_call) => {
                mutations.push(PendingMutation::Action(ActionMutation {
                    entry_id: entry_id.clone(),
                    action_id: tool_call.tool_call_id.clone(),
                    change: Some(action_mutation::Change::Started(action_mutation::Started {
                        kind: action_kind(tool_call) as i32,
                        name: action_name(tool_call)?.to_owned(),
                        input_json: b"{}".to_vec(),
                        started_at: message.timestamp,
                    })),
                }));
            }
            _ => unreachable!("unsupported messages returned above"),
        }
        if keep_open {
            self.open_entries.insert(entry_id.clone());
        } else {
            mutations.push(finalize_entry(entry_id, message.timestamp));
        }
        Ok(mutations)
    }

    fn normalize_final_message(
        &mut self,
        message: &Message,
    ) -> Result<Vec<PendingMutation>, SemanticProducerError> {
        let entry_id = self
            .entry_ids
            .get(&message.id)
            .cloned()
            .ok_or(SemanticProducerError::UnknownMessage)?;
        let mut mutations = vec![self.normalize_content(message, &entry_id, false)?];
        if self.open_entries.remove(&entry_id) {
            mutations.push(finalize_entry(entry_id, message.timestamp));
        }
        Ok(mutations)
    }

    fn normalize_content_delta(
        &mut self,
        message: &Message,
    ) -> Result<PendingMutation, SemanticProducerError> {
        let entry_id = self
            .entry_ids
            .get(&message.id)
            .cloned()
            .ok_or(SemanticProducerError::UnknownMessage)?;
        self.normalize_content(message, &entry_id, true)
    }

    fn normalize_content(
        &mut self,
        message: &Message,
        entry_id: &str,
        delta: bool,
    ) -> Result<PendingMutation, SemanticProducerError> {
        let (data, encoding) = display_content(message)?;
        let data = (self.redact)(&data);
        let content_id = format!("{entry_id}:content:0");
        let change = if delta {
            let delta_index = self
                .content_delta_indexes
                .entry(content_id.clone())
                .or_default();
            let current = *delta_index;
            *delta_index += 1;
            content_mutation::Change::Delta(content_mutation::Delta {
                delta_index: current,
                encoding: encoding as i32,
                data: data.into_bytes(),
            })
        } else {
            content_mutation::Change::Final(content_mutation::Final {
                encoding: encoding as i32,
                data: data.into_bytes(),
                media: Vec::new(),
            })
        };
        Ok(PendingMutation::Content(ContentMutation {
            entry_id: entry_id.to_owned(),
            content_id,
            content_index: 0,
            change: Some(change),
        }))
    }

    fn normalize_finished(
        &mut self,
        finished: &response_event::StreamFinished,
    ) -> Result<Vec<PendingMutation>, SemanticProducerError> {
        if !self.pending_message_contexts.is_empty() {
            return Err(SemanticProducerError::StaleAcceptedMessageContext);
        }
        let mut entries: Vec<_> = self.open_entries.drain().collect();
        entries.sort();
        let mut mutations: Vec<_> = entries
            .into_iter()
            .map(|entry_id| finalize_entry(entry_id, None))
            .collect();
        let state = match finished.reason {
            Some(response_event::stream_finished::Reason::Done(_)) | None => {
                ExecutionState::Succeeded
            }
            Some(_) => ExecutionState::Failed,
        };
        mutations.push(PendingMutation::Execution(ExecutionMutation {
            change: Some(execution_mutation::Change::Finished(
                execution_mutation::Finished {
                    state: state as i32,
                    finished_at: None,
                    detail: String::new(),
                },
            )),
        }));
        Ok(mutations)
    }

    fn finish_mutation(&mut self, mut mutation: PendingMutation) -> ConversationMutation {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        let mutation_id = format!(
            "{}:{sequence:020}:{}",
            self.execution_identity.execution_id,
            mutation.stable_source_key()
        );
        let attribution = mutation
            .entry_id()
            .and_then(|entry_id| self.entry_attributions.get(entry_id))
            .cloned()
            .unwrap_or_else(|| system_attribution(&self.run_id));
        let occurred_at = canonical_timestamp(mutation.source_timestamp());
        mutation.set_canonical_timestamp(occurred_at);
        ConversationMutation {
            schema_version: SchemaVersion::V1 as i32,
            identity: Some(MutationIdentity {
                conversation_id: self.execution_identity.conversation_id.clone(),
                execution_id: self.execution_identity.execution_id.clone(),
                mutation_id,
                sequence,
                run_id: self.run_id.clone(),
                request_id: self.request_id.clone(),
            }),
            attribution: Some(attribution),
            occurred_at: Some(occurred_at),
            mutation: Some(mutation.into()),
        }
    }
}

enum PendingMutation {
    Execution(ExecutionMutation),
    Entry(EntryMutation),
    Content(ContentMutation),
    Action(ActionMutation),
    Interruption(InterruptionMutation),
}
impl PendingMutation {
    fn source_timestamp(&self) -> Option<&prost_types::Timestamp> {
        match self {
            Self::Execution(execution) => match execution.change.as_ref() {
                Some(execution_mutation::Change::Started(started)) => started.started_at.as_ref(),
                Some(execution_mutation::Change::Finished(finished)) => {
                    finished.finished_at.as_ref()
                }
                Some(execution_mutation::Change::StateChanged(_)) | None => None,
            },
            Self::Entry(entry) => match entry.change.as_ref() {
                Some(entry_mutation::Change::Created(created)) => created.created_at.as_ref(),
                Some(entry_mutation::Change::Finalized(finalized)) => {
                    finalized.finalized_at.as_ref()
                }
                None => None,
            },
            Self::Action(action) => match action.change.as_ref() {
                Some(action_mutation::Change::Started(started)) => started.started_at.as_ref(),
                Some(action_mutation::Change::Result(result)) => result.finished_at.as_ref(),
                Some(action_mutation::Change::StateChanged(_)) | None => None,
            },
            Self::Interruption(interruption) => interruption.interrupted_at.as_ref(),
            Self::Content(_) => None,
        }
    }

    fn set_canonical_timestamp(&mut self, timestamp: prost_types::Timestamp) {
        match self {
            Self::Execution(execution) => match execution.change.as_mut() {
                Some(execution_mutation::Change::Started(started)) => {
                    started.started_at = Some(timestamp);
                }
                Some(execution_mutation::Change::Finished(finished)) => {
                    finished.finished_at = Some(timestamp);
                }
                Some(execution_mutation::Change::StateChanged(_)) | None => {}
            },
            Self::Entry(entry) => match entry.change.as_mut() {
                Some(entry_mutation::Change::Created(created)) => {
                    created.created_at = Some(timestamp);
                }
                Some(entry_mutation::Change::Finalized(finalized)) => {
                    finalized.finalized_at = Some(timestamp);
                }
                None => {}
            },
            Self::Action(action) => match action.change.as_mut() {
                Some(action_mutation::Change::Started(started)) => {
                    started.started_at = Some(timestamp);
                }
                Some(action_mutation::Change::Result(result)) => {
                    result.finished_at = Some(timestamp);
                }
                Some(action_mutation::Change::StateChanged(_)) | None => {}
            },
            Self::Interruption(interruption) => {
                interruption.interrupted_at = Some(timestamp);
            }
            Self::Content(_) => {}
        }
    }
}

impl From<PendingMutation> for conversation_mutation::Mutation {
    fn from(value: PendingMutation) -> Self {
        match value {
            PendingMutation::Execution(value) => Self::Execution(value),
            PendingMutation::Entry(value) => Self::Entry(value),
            PendingMutation::Content(value) => Self::Content(value),
            PendingMutation::Action(value) => Self::Action(value),
            PendingMutation::Interruption(value) => Self::Interruption(value),
        }
    }
}

fn finalize_entry(
    entry_id: String,
    finalized_at: Option<prost_types::Timestamp>,
) -> PendingMutation {
    PendingMutation::Entry(EntryMutation {
        entry_id,
        change: Some(entry_mutation::Change::Finalized(
            entry_mutation::Finalized {
                state: EntryState::Final as i32,
                finalized_at,
            },
        )),
    })
}

fn display_content(message: &Message) -> Result<(String, ContentEncoding), SemanticProducerError> {
    match message
        .message
        .as_ref()
        .ok_or(SemanticProducerError::MissingMessageType)?
    {
        message::Message::UserQuery(query) => Ok((query.query.clone(), ContentEncoding::Utf8Text)),
        message::Message::AgentOutput(output) => {
            Ok((output.text.clone(), ContentEncoding::Utf8Markdown))
        }
        message::Message::AgentReasoning(reasoning) => {
            Ok((reasoning.reasoning.clone(), ContentEncoding::Utf8Markdown))
        }
        _ => Err(SemanticProducerError::UnsupportedMessage(
            "non_display_content",
        )),
    }
}

fn action_kind(tool_call: &message::ToolCall) -> ActionKind {
    match tool_call.tool {
        Some(message::tool_call::Tool::RunShellCommand(_))
        | Some(message::tool_call::Tool::WriteToLongRunningShellCommand(_))
        | Some(message::tool_call::Tool::ReadShellCommandOutput(_))
        | Some(message::tool_call::Tool::TransferShellCommandControlToUser(_)) => {
            ActionKind::Command
        }
        Some(message::tool_call::Tool::UseComputer(_))
        | Some(message::tool_call::Tool::RequestComputerUse(_))
        | Some(message::tool_call::Tool::StartRecording(_))
        | Some(message::tool_call::Tool::StopRecording(_)) => ActionKind::ComputerUse,
        Some(message::tool_call::Tool::Subagent(_))
        | Some(message::tool_call::Tool::RunAgents(_))
        | Some(message::tool_call::Tool::SendMessageToAgent(_))
        | Some(message::tool_call::Tool::WaitForEvents(_)) => ActionKind::Subagent,
        Some(message::tool_call::Tool::ReadMcpResource(_))
        | Some(message::tool_call::Tool::CallMcpTool(_)) => ActionKind::Integration,
        Some(_) | None => ActionKind::Tool,
    }
}

#[allow(deprecated)]
fn action_name(tool_call: &message::ToolCall) -> Result<&'static str, SemanticProducerError> {
    let name = match tool_call
        .tool
        .as_ref()
        .ok_or(SemanticProducerError::UnsupportedMessage("missing_tool"))?
    {
        message::tool_call::Tool::RunShellCommand(_) => "run_shell_command",
        message::tool_call::Tool::SearchCodebase(_) => "search_codebase",
        message::tool_call::Tool::Server(_) => "server",
        message::tool_call::Tool::ReadFiles(_) => "read_files",
        message::tool_call::Tool::ApplyFileDiffs(_) => "apply_file_diffs",
        message::tool_call::Tool::SuggestPlan(_) => "suggest_plan",
        message::tool_call::Tool::SuggestCreatePlan(_) => "suggest_create_plan",
        message::tool_call::Tool::Grep(_) => "grep",
        message::tool_call::Tool::FileGlob(_) => "file_glob",
        message::tool_call::Tool::ReadMcpResource(_) => "read_mcp_resource",
        message::tool_call::Tool::CallMcpTool(_) => "call_mcp_tool",
        message::tool_call::Tool::WriteToLongRunningShellCommand(_) => {
            "write_to_long_running_shell_command"
        }
        message::tool_call::Tool::SuggestNewConversation(_) => "suggest_new_conversation",
        message::tool_call::Tool::FileGlobV2(_) => "file_glob_v2",
        message::tool_call::Tool::SuggestPrompt(_) => "suggest_prompt",
        message::tool_call::Tool::OpenCodeReview(_) => "open_code_review",
        message::tool_call::Tool::InitProject(_) => "init_project",
        message::tool_call::Tool::Subagent(_) => "subagent",
        message::tool_call::Tool::ReadDocuments(_) => "read_documents",
        message::tool_call::Tool::EditDocuments(_) => "edit_documents",
        message::tool_call::Tool::CreateDocuments(_) => "create_documents",
        message::tool_call::Tool::ReadShellCommandOutput(_) => "read_shell_command_output",
        message::tool_call::Tool::UseComputer(_) => "use_computer",
        message::tool_call::Tool::InsertReviewComments(_) => "insert_review_comments",
        message::tool_call::Tool::ReadSkill(_) => "read_skill",
        message::tool_call::Tool::RequestComputerUse(_) => "request_computer_use",
        message::tool_call::Tool::FetchConversation(_) => "fetch_conversation",
        message::tool_call::Tool::SendMessageToAgent(_) => "send_message_to_agent",
        message::tool_call::Tool::TransferShellCommandControlToUser(_) => {
            "transfer_shell_command_control_to_user"
        }
        message::tool_call::Tool::AskUserQuestion(_) => "ask_user_question",
        message::tool_call::Tool::UploadFileArtifact(_) => "upload_file_artifact",
        message::tool_call::Tool::RunAgents(_) => "run_agents",
        message::tool_call::Tool::WaitForEvents(_) => "wait_for_events",
        message::tool_call::Tool::StartRecording(_) => "start_recording",
        message::tool_call::Tool::StopRecording(_) => "stop_recording",
    };
    Ok(name)
}

fn tool_result_state(result: &message::ToolCallResult) -> ActionState {
    match result.result {
        Some(message::tool_call_result::Result::Cancel(_)) => ActionState::Cancelled,
        Some(_) => ActionState::Succeeded,
        None => ActionState::Failed,
    }
}
