use std::collections::{BTreeMap, HashMap};

use prost::Message;
use session_sharing_protocol::common::{
    ExecutionIdentity, OrderedTerminalEvent, OrderedTerminalEventType, SemanticCursor,
    SemanticResyncReason, SessionContentMode, SessionId,
};
use warp_conversation_mutation_api::{
    ConversationMutation, SchemaVersion, content_mutation, conversation_mutation,
};

use crate::validation::{ConversationMutationValidationError, validate_mutation_timestamps};

const MAX_SEEN_MUTATIONS: usize = 4_096;
const MAX_SEEN_MUTATION_BYTES: usize = 16 * 1024 * 1024;
const MAX_TRACKED_CONTENTS: usize = 4_096;

#[derive(Debug, PartialEq)]
pub enum ConsumeOutcome {
    Applied(Box<ConversationMutation>),
    Duplicate,
    ResyncRequired(SemanticResyncReason),
}

#[derive(Debug, thiserror::Error)]
pub enum SemanticConsumerError {
    #[error("semantic consumer received a terminal event")]
    UnexpectedTerminalEvent,
    #[error("failed to decode conversation mutation: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("conversation mutation is missing identity")]
    MissingIdentity,
    #[error("conversation mutation is missing a payload")]
    MissingMutation,
    #[error("conversation mutation is missing a mutation ID")]
    MissingMutationId,
    #[error("conversation mutation has invalid canonical timestamps: {0}")]
    InvalidTimestamp(#[from] ConversationMutationValidationError),
}

struct SeenMutation {
    mutation_id: String,
    bytes: Vec<u8>,
}

pub struct SemanticConsumer {
    session_id: SessionId,
    execution_identity: ExecutionIdentity,
    schema_version: u32,
    next_sequence: u64,
    seen_mutations: BTreeMap<u64, SeenMutation>,
    seen_mutation_ids: HashMap<String, u64>,
    seen_mutation_bytes: usize,
    content_positions: HashMap<String, (String, u32)>,
    next_content_indexes: HashMap<String, u32>,
    next_delta_indexes: HashMap<String, u64>,
}

impl SemanticConsumer {
    pub fn new(
        session_id: SessionId,
        execution_identity: ExecutionIdentity,
        schema_version: u32,
    ) -> Self {
        Self {
            session_id,
            execution_identity,
            schema_version,
            next_sequence: 1,
            seen_mutations: BTreeMap::new(),
            seen_mutation_ids: HashMap::new(),
            seen_mutation_bytes: 0,
            content_positions: HashMap::new(),
            next_content_indexes: HashMap::new(),
            next_delta_indexes: HashMap::new(),
        }
    }

    pub fn cursor(&self) -> Option<SemanticCursor> {
        (self.next_sequence > 1).then(|| SemanticCursor {
            session_id: self.session_id,
            conversation_id: self.execution_identity.conversation_id.clone(),
            execution_id: self.execution_identity.execution_id.clone(),
            content_mode: SessionContentMode::SemanticConversationOnly,
            schema_version: self.schema_version,
            mutation_sequence: self.next_sequence - 1,
        })
    }

    pub fn consume(
        &mut self,
        event: OrderedTerminalEvent,
    ) -> Result<ConsumeOutcome, SemanticConsumerError> {
        let OrderedTerminalEventType::SemanticConversationMutation { cursor, mutation } =
            event.event_type
        else {
            return Err(SemanticConsumerError::UnexpectedTerminalEvent);
        };
        self.consume_mutation(cursor, &mutation)
    }

    pub fn consume_mutation(
        &mut self,
        cursor: SemanticCursor,
        bytes: &[u8],
    ) -> Result<ConsumeOutcome, SemanticConsumerError> {
        if cursor.session_id != self.session_id {
            return Ok(ConsumeOutcome::ResyncRequired(
                SemanticResyncReason::CursorSessionMismatch,
            ));
        }
        if cursor.conversation_id != self.execution_identity.conversation_id {
            return Ok(ConsumeOutcome::ResyncRequired(
                SemanticResyncReason::ExecutionChanged,
            ));
        }
        if cursor.execution_id != self.execution_identity.execution_id {
            return Ok(ConsumeOutcome::ResyncRequired(
                SemanticResyncReason::CursorExecutionMismatch,
            ));
        }
        if cursor.content_mode != SessionContentMode::SemanticConversationOnly {
            return Ok(ConsumeOutcome::ResyncRequired(
                SemanticResyncReason::SessionStateUnavailable,
            ));
        }
        if cursor.schema_version != self.schema_version {
            return Ok(ConsumeOutcome::ResyncRequired(
                SemanticResyncReason::SchemaMismatch {
                    expected: self.schema_version,
                    received: cursor.schema_version,
                },
            ));
        }

        let mutation = ConversationMutation::decode(bytes)?;
        let identity = mutation
            .identity
            .as_ref()
            .ok_or(SemanticConsumerError::MissingIdentity)?;
        if mutation.mutation.is_none() {
            return Err(SemanticConsumerError::MissingMutation);
        }
        if identity.mutation_id.is_empty() {
            return Err(SemanticConsumerError::MissingMutationId);
        }
        validate_mutation_timestamps(&mutation)?;
        if mutation.schema_version != SchemaVersion::V1 as i32
            || mutation.schema_version as u32 != self.schema_version
        {
            return Ok(ConsumeOutcome::ResyncRequired(
                SemanticResyncReason::SchemaMismatch {
                    expected: self.schema_version,
                    received: mutation.schema_version.max(0) as u32,
                },
            ));
        }
        if identity.conversation_id != self.execution_identity.conversation_id
            || identity.execution_id != self.execution_identity.execution_id
        {
            return Ok(ConsumeOutcome::ResyncRequired(
                SemanticResyncReason::ExecutionChanged,
            ));
        }
        if identity.sequence != cursor.mutation_sequence {
            return Ok(ConsumeOutcome::ResyncRequired(
                SemanticResyncReason::ReplayGap {
                    expected_sequence: cursor.mutation_sequence,
                    next_available_sequence: identity.sequence,
                },
            ));
        }

        if identity.sequence < self.next_sequence {
            let Some(seen) = self.seen_mutations.get(&identity.sequence) else {
                return Ok(ConsumeOutcome::ResyncRequired(
                    SemanticResyncReason::CursorExpired,
                ));
            };
            return Ok(
                if seen.mutation_id == identity.mutation_id && seen.bytes == bytes {
                    ConsumeOutcome::Duplicate
                } else {
                    ConsumeOutcome::ResyncRequired(SemanticResyncReason::ConflictingDuplicate {
                        mutation_sequence: identity.sequence,
                    })
                },
            );
        }
        if identity.sequence > self.next_sequence {
            return Ok(ConsumeOutcome::ResyncRequired(
                SemanticResyncReason::ReplayGap {
                    expected_sequence: self.next_sequence,
                    next_available_sequence: identity.sequence,
                },
            ));
        }

        if self.seen_mutation_ids.contains_key(&identity.mutation_id) {
            return Ok(ConsumeOutcome::ResyncRequired(
                SemanticResyncReason::ConflictingDuplicate {
                    mutation_sequence: identity.sequence,
                },
            ));
        }
        if !self.validate_content_indexes(&mutation) {
            return Ok(ConsumeOutcome::ResyncRequired(
                SemanticResyncReason::SessionStateUnavailable,
            ));
        }
        self.seen_mutation_bytes = self.seen_mutation_bytes.saturating_add(bytes.len());
        self.seen_mutation_ids
            .insert(identity.mutation_id.clone(), identity.sequence);
        self.seen_mutations.insert(
            identity.sequence,
            SeenMutation {
                mutation_id: identity.mutation_id.clone(),
                bytes: bytes.to_vec(),
            },
        );
        while self.seen_mutations.len() > MAX_SEEN_MUTATIONS
            || self.seen_mutation_bytes > MAX_SEEN_MUTATION_BYTES
        {
            let Some((sequence, seen)) = self.seen_mutations.pop_first() else {
                break;
            };
            self.seen_mutation_bytes = self.seen_mutation_bytes.saturating_sub(seen.bytes.len());
            if self.seen_mutation_ids.get(&seen.mutation_id) == Some(&sequence) {
                self.seen_mutation_ids.remove(&seen.mutation_id);
            }
        }
        self.next_sequence += 1;
        Ok(ConsumeOutcome::Applied(Box::new(mutation)))
    }

    pub fn resync_required(reason: SemanticResyncReason) -> ConsumeOutcome {
        ConsumeOutcome::ResyncRequired(reason)
    }

    fn validate_content_indexes(&mut self, mutation: &ConversationMutation) -> bool {
        let Some(conversation_mutation::Mutation::Content(content)) = mutation.mutation.as_ref()
        else {
            return true;
        };
        let position = (content.entry_id.clone(), content.content_index);
        if let Some(existing) = self.content_positions.get(&content.content_id) {
            if existing != &position {
                return false;
            }
        } else {
            if self.content_positions.len() >= MAX_TRACKED_CONTENTS {
                return false;
            }
            let next_index = self
                .next_content_indexes
                .entry(content.entry_id.clone())
                .or_default();
            if content.content_index != *next_index {
                return false;
            }
            *next_index += 1;
            self.content_positions
                .insert(content.content_id.clone(), position);
        }
        if let Some(content_mutation::Change::Delta(delta)) = content.change.as_ref() {
            let next_delta = self
                .next_delta_indexes
                .entry(content.content_id.clone())
                .or_default();
            if delta.delta_index != *next_delta {
                return false;
            }
            *next_delta += 1;
        }
        true
    }

    pub fn retained_mutation_count(&self) -> usize {
        self.seen_mutations.len()
    }
}
