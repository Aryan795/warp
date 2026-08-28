use std::collections::HashSet;

use prost::Message;
use session_sharing_protocol::common::ExecutionIdentity;
use warp_conversation_mutation_api::{
    AcceptedMessageContext, Attribution, ConversationMutation, MediaReference, SchemaVersion,
    action_mutation, author, conversation_mutation, entry_mutation, execution_mutation,
    media_reference, origin,
};

const MAX_ACCEPTED_MESSAGE_CONTEXT_BYTES: usize = 64 * 1024;
const MAX_MEDIA_REFERENCES: usize = 64;
const MAX_IDENTIFIER_BYTES: usize = 1_024;
const MAX_LABEL_BYTES: usize = 4_096;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AcceptedMessageContextError {
    #[error("accepted message context is missing")]
    Missing,
    #[error("accepted message context exceeds its encoded size limit")]
    EncodedSizeLimit,
    #[error("failed to decode accepted message context: {0}")]
    Decode(String),
    #[error("accepted message context schema does not match the semantic session")]
    SchemaMismatch,
    #[error("accepted message context conversation does not match the semantic session")]
    ConversationMismatch,
    #[error("accepted message context execution does not match the semantic session")]
    ExecutionMismatch,
    #[error("accepted message context has an invalid {0}")]
    InvalidField(&'static str),
    #[error("accepted message context has too many media references")]
    MediaLimit,
    #[error("accepted message context contains a duplicate media ID")]
    DuplicateMedia,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ConversationMutationValidationError {
    #[error("conversation mutation is missing occurred_at")]
    MissingOccurredAt,
    #[error("conversation mutation has an invalid {0} timestamp")]
    InvalidTimestamp(&'static str),
    #[error("conversation mutation is missing its {0} timestamp")]
    MissingTimestamp(&'static str),
    #[error("conversation interruption timestamp does not equal occurred_at")]
    InterruptionTimestampMismatch,
}

pub fn decode_accepted_message_context(
    bytes: &[u8],
    execution_identity: &ExecutionIdentity,
    schema_version: u32,
) -> Result<AcceptedMessageContext, AcceptedMessageContextError> {
    if bytes.is_empty() {
        return Err(AcceptedMessageContextError::Missing);
    }
    if bytes.len() > MAX_ACCEPTED_MESSAGE_CONTEXT_BYTES {
        return Err(AcceptedMessageContextError::EncodedSizeLimit);
    }
    let context = AcceptedMessageContext::decode(bytes)
        .map_err(|error| AcceptedMessageContextError::Decode(error.to_string()))?;
    validate_accepted_message_context(&context, execution_identity, schema_version)?;
    Ok(context)
}

pub fn validate_accepted_message_context(
    context: &AcceptedMessageContext,
    execution_identity: &ExecutionIdentity,
    schema_version: u32,
) -> Result<(), AcceptedMessageContextError> {
    if context.schema_version != SchemaVersion::V1 as i32
        || context.schema_version.max(0) as u32 != schema_version
    {
        return Err(AcceptedMessageContextError::SchemaMismatch);
    }
    if context.conversation_id != execution_identity.conversation_id {
        return Err(AcceptedMessageContextError::ConversationMismatch);
    }
    if context.execution_id != execution_identity.execution_id {
        return Err(AcceptedMessageContextError::ExecutionMismatch);
    }
    validate_identifier("message_id", &context.message_id)?;
    validate_attribution(
        context
            .attribution
            .as_ref()
            .ok_or(AcceptedMessageContextError::InvalidField("attribution"))?,
    )?;
    if context.media.len() > MAX_MEDIA_REFERENCES {
        return Err(AcceptedMessageContextError::MediaLimit);
    }
    let mut media_ids = HashSet::with_capacity(context.media.len());
    for media in &context.media {
        validate_media_reference(media)?;
        if !media_ids.insert(media.media_id.as_str()) {
            return Err(AcceptedMessageContextError::DuplicateMedia);
        }
    }
    Ok(())
}

fn validate_attribution(attribution: &Attribution) -> Result<(), AcceptedMessageContextError> {
    let author = attribution
        .author
        .as_ref()
        .ok_or(AcceptedMessageContextError::InvalidField(
            "attribution.author",
        ))?;
    validate_identifier("attribution.author.id", &author.id)?;
    if author::Kind::try_from(author.kind)
        .ok()
        .is_none_or(|kind| kind == author::Kind::Unspecified)
    {
        return Err(AcceptedMessageContextError::InvalidField(
            "attribution.author.kind",
        ));
    }
    validate_label("attribution.author.display_name", &author.display_name)?;

    let origin = attribution
        .origin
        .as_ref()
        .ok_or(AcceptedMessageContextError::InvalidField(
            "attribution.origin",
        ))?;
    if origin::Kind::try_from(origin.kind)
        .ok()
        .is_none_or(|kind| kind == origin::Kind::Unspecified)
    {
        return Err(AcceptedMessageContextError::InvalidField(
            "attribution.origin.kind",
        ));
    }
    validate_identifier("attribution.origin.source_id", &origin.source_id)?;
    validate_label("attribution.origin.subtype", &origin.subtype)?;

    if let Some(delivery) = attribution.source_delivery.as_ref() {
        validate_identifier(
            "attribution.source_delivery.delivery_id",
            &delivery.delivery_id,
        )?;
        validate_label(
            "attribution.source_delivery.channel_id",
            &delivery.channel_id,
        )?;
        validate_label("attribution.source_delivery.thread_id", &delivery.thread_id)?;
        validate_identifier(
            "attribution.source_delivery.message_id",
            &delivery.message_id,
        )?;
    }
    Ok(())
}

fn validate_media_reference(media: &MediaReference) -> Result<(), AcceptedMessageContextError> {
    validate_identifier("media.media_id", &media.media_id)?;
    if media_reference::Kind::try_from(media.kind)
        .ok()
        .is_none_or(|kind| kind == media_reference::Kind::Unspecified)
    {
        return Err(AcceptedMessageContextError::InvalidField("media.kind"));
    }
    validate_label("media.mime_type", &media.mime_type)?;
    if !media.sha256.is_empty() && media.sha256.len() != 32 {
        return Err(AcceptedMessageContextError::InvalidField("media.sha256"));
    }
    validate_label("media.display_name", &media.display_name)?;
    validate_identifier("media.reference", &media.reference)
}

fn validate_identifier(
    field: &'static str,
    value: &str,
) -> Result<(), AcceptedMessageContextError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > MAX_IDENTIFIER_BYTES
        || value.contains('\0')
    {
        return Err(AcceptedMessageContextError::InvalidField(field));
    }
    Ok(())
}

fn validate_label(field: &'static str, value: &str) -> Result<(), AcceptedMessageContextError> {
    if value.len() > MAX_LABEL_BYTES || value.contains('\0') {
        return Err(AcceptedMessageContextError::InvalidField(field));
    }
    Ok(())
}

pub(crate) fn timestamp_is_valid(timestamp: &prost_types::Timestamp) -> bool {
    (-62_135_596_800..=253_402_300_799).contains(&timestamp.seconds)
        && (0..1_000_000_000).contains(&timestamp.nanos)
}

pub(crate) fn validate_mutation_timestamps(
    mutation: &ConversationMutation,
) -> Result<(), ConversationMutationValidationError> {
    let occurred_at = mutation
        .occurred_at
        .as_ref()
        .ok_or(ConversationMutationValidationError::MissingOccurredAt)?;
    if !timestamp_is_valid(occurred_at) {
        return Err(ConversationMutationValidationError::InvalidTimestamp(
            "occurred_at",
        ));
    }

    let nested = match mutation.mutation.as_ref() {
        Some(conversation_mutation::Mutation::Execution(execution)) => {
            match execution.change.as_ref() {
                Some(execution_mutation::Change::Started(started)) => {
                    Some(("execution.started_at", started.started_at.as_ref()))
                }
                Some(execution_mutation::Change::Finished(finished)) => {
                    Some(("execution.finished_at", finished.finished_at.as_ref()))
                }
                Some(execution_mutation::Change::StateChanged(_)) | None => None,
            }
        }
        Some(conversation_mutation::Mutation::Entry(entry)) => match entry.change.as_ref() {
            Some(entry_mutation::Change::Created(created)) => {
                Some(("entry.created_at", created.created_at.as_ref()))
            }
            Some(entry_mutation::Change::Finalized(finalized)) => {
                Some(("entry.finalized_at", finalized.finalized_at.as_ref()))
            }
            None => None,
        },
        Some(conversation_mutation::Mutation::Action(action)) => match action.change.as_ref() {
            Some(action_mutation::Change::Started(started)) => {
                Some(("action.started_at", started.started_at.as_ref()))
            }
            Some(action_mutation::Change::Result(result)) => {
                Some(("action.finished_at", result.finished_at.as_ref()))
            }
            Some(action_mutation::Change::StateChanged(_)) | None => None,
        },
        Some(conversation_mutation::Mutation::Delivery(delivery)) => {
            Some(("delivery.changed_at", delivery.changed_at.as_ref()))
        }
        Some(conversation_mutation::Mutation::Interruption(interruption)) => {
            let interrupted_at = require_valid_timestamp(
                "interruption.interrupted_at",
                interruption.interrupted_at.as_ref(),
            )?;
            if interrupted_at != occurred_at {
                return Err(ConversationMutationValidationError::InterruptionTimestampMismatch);
            }
            None
        }
        Some(conversation_mutation::Mutation::Control(control)) => {
            Some(("control.changed_at", control.changed_at.as_ref()))
        }
        Some(conversation_mutation::Mutation::Content(_)) | None => None,
    };
    if let Some((field, timestamp)) = nested {
        require_valid_timestamp(field, timestamp)?;
    }
    Ok(())
}

fn require_valid_timestamp<'a>(
    field: &'static str,
    timestamp: Option<&'a prost_types::Timestamp>,
) -> Result<&'a prost_types::Timestamp, ConversationMutationValidationError> {
    let timestamp =
        timestamp.ok_or(ConversationMutationValidationError::MissingTimestamp(field))?;
    if !timestamp_is_valid(timestamp) {
        return Err(ConversationMutationValidationError::InvalidTimestamp(field));
    }
    Ok(timestamp)
}
