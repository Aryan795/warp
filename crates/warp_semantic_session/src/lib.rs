mod consumer;
mod producer;
mod reducer;
mod validation;

pub use consumer::{ConsumeOutcome, SemanticConsumer, SemanticConsumerError};
pub use producer::{SemanticMutationProducer, SemanticProducerError};
pub use reducer::{
    SemanticAction, SemanticContent, SemanticEntry, SemanticMedia, SemanticReducerError,
    SemanticTranscript, semantic_payload_key,
};
use session_sharing_protocol::common::{
    ExecutionIdentity, NegotiatedSessionContent, SemanticCursor, SemanticNegotiationError,
    SessionContentMode, SessionId, validate_negotiated_content,
};
pub use validation::{
    AcceptedMessageContextError, ConversationMutationValidationError,
    decode_accepted_message_context, validate_accepted_message_context,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub enum RequestedSessionContent {
    #[default]
    FullTerminal,
    SemanticConversation {
        schema_version: u32,
        execution_identity: ExecutionIdentity,
        initial_accepted_message_context:
            Option<Box<warp_conversation_mutation_api::AcceptedMessageContext>>,
    },
}
impl Eq for RequestedSessionContent {}

impl RequestedSessionContent {
    pub fn semantic_v1(execution_identity: ExecutionIdentity) -> Self {
        Self::SemanticConversation {
            schema_version:
                session_sharing_protocol::common::SEMANTIC_CONVERSATION_SCHEMA_VERSION_V1,
            execution_identity,
            initial_accepted_message_context: None,
        }
    }
    pub fn semantic_v1_with_initial_context(
        execution_identity: ExecutionIdentity,
        context: warp_conversation_mutation_api::AcceptedMessageContext,
    ) -> Result<Self, AcceptedMessageContextError> {
        validate_accepted_message_context(
            &context,
            &execution_identity,
            session_sharing_protocol::common::SEMANTIC_CONVERSATION_SCHEMA_VERSION_V1,
        )?;
        Ok(Self::SemanticConversation {
            schema_version:
                session_sharing_protocol::common::SEMANTIC_CONVERSATION_SCHEMA_VERSION_V1,
            execution_identity,
            initial_accepted_message_context: Some(Box::new(context)),
        })
    }

    pub fn content_mode(&self) -> SessionContentMode {
        match self {
            Self::FullTerminal => SessionContentMode::FullTerminal,
            Self::SemanticConversation { .. } => SessionContentMode::SemanticConversationOnly,
        }
    }

    pub fn schema_version(&self) -> Option<u32> {
        match self {
            Self::FullTerminal => None,
            Self::SemanticConversation { schema_version, .. } => Some(*schema_version),
        }
    }

    pub fn execution_identity(&self) -> Option<&ExecutionIdentity> {
        match self {
            Self::FullTerminal => None,
            Self::SemanticConversation {
                execution_identity, ..
            } => Some(execution_identity),
        }
    }
    pub fn initial_accepted_message_context(
        &self,
    ) -> Option<&warp_conversation_mutation_api::AcceptedMessageContext> {
        match self {
            Self::FullTerminal => None,
            Self::SemanticConversation {
                initial_accepted_message_context,
                ..
            } => initial_accepted_message_context.as_deref(),
        }
    }

    pub fn is_semantic(&self) -> bool {
        matches!(self, Self::SemanticConversation { .. })
    }

    pub fn validate_echo(
        &self,
        echoed: Option<&NegotiatedSessionContent>,
    ) -> Result<(), SemanticNegotiationError> {
        validate_negotiated_content(
            self.content_mode(),
            self.schema_version(),
            self.execution_identity(),
            echoed,
        )
    }

    pub fn cursor(&self, session_id: SessionId, mutation_sequence: u64) -> Option<SemanticCursor> {
        let execution_identity = self.execution_identity()?;
        Some(SemanticCursor {
            session_id,
            conversation_id: execution_identity.conversation_id.clone(),
            execution_id: execution_identity.execution_id.clone(),
            content_mode: self.content_mode(),
            schema_version: self.schema_version()?,
            mutation_sequence,
        })
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
