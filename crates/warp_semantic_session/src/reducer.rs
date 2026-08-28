use std::collections::BTreeMap;

use prost::Message;
use warp_conversation_mutation_api::{
    ActionState, ConversationMutation, EntryKind, EntryState, ExecutionState, MediaReference,
    action_mutation, content_mutation, conversation_mutation, entry_mutation, execution_mutation,
};

const MAX_TRANSCRIPT_ENTRIES: usize = 4_096;
const MAX_TRANSCRIPT_ACTIONS: usize = 16_384;
const MAX_TRANSCRIPT_MEDIA_REFERENCES: usize = 16_384;
const MAX_TRANSCRIPT_RETAINED_BYTES: usize = 16 * 1024 * 1024;
const MAX_TRANSCRIPT_PROCESSED_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SemanticMedia {
    pub media_id: String,
    pub kind: i32,
    pub mime_type: String,
    pub size_bytes: u64,
    pub display_name: String,
    pub reference: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SemanticContent {
    pub content_id: String,
    pub encoding: i32,
    pub data: Vec<u8>,
    pub media: Vec<SemanticMedia>,
    next_delta_index: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SemanticAction {
    pub kind: i32,
    pub name: String,
    pub state: i32,
    pub input_json: Vec<u8>,
    pub output_json: Vec<u8>,
    pub media: Vec<SemanticMedia>,
    pub detail: String,
    pub error_message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SemanticEntry {
    pub kind: i32,
    pub state: i32,
    pub parent_entry_id: String,
    pub media: Vec<SemanticMedia>,
    pub contents: BTreeMap<u32, SemanticContent>,
    pub actions: BTreeMap<String, SemanticAction>,
    pub action_order: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SemanticReducerError {
    #[error("mutation is missing a payload")]
    MissingMutation,
    #[error("mutation referenced an unknown entry")]
    UnknownEntry,
    #[error("content delta index mismatch")]
    DeltaIndexMismatch,
    #[error("content index conflicts with an existing content block")]
    ContentIndexConflict,
    #[error("content identity conflicts with an existing content block")]
    ContentIdentityConflict,
    #[error("semantic transcript exceeded its retention limit")]
    RetentionLimit,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SemanticTranscript {
    pub execution_state: i32,
    pub entries: BTreeMap<String, SemanticEntry>,
    pub entry_order: Vec<String>,
    retained_bytes: usize,
    processed_bytes: usize,
    action_count: usize,
    media_reference_count: usize,
}

impl SemanticTranscript {
    pub fn apply(&mut self, mutation: &ConversationMutation) -> Result<(), SemanticReducerError> {
        let processed_bytes = self
            .processed_bytes
            .checked_add(mutation.encoded_len())
            .filter(|bytes| *bytes <= MAX_TRANSCRIPT_PROCESSED_BYTES)
            .ok_or(SemanticReducerError::RetentionLimit)?;
        match mutation
            .mutation
            .as_ref()
            .ok_or(SemanticReducerError::MissingMutation)?
        {
            conversation_mutation::Mutation::Execution(execution) => {
                self.execution_state = match execution.change.as_ref() {
                    Some(execution_mutation::Change::Started(_)) => ExecutionState::Running as i32,
                    Some(execution_mutation::Change::StateChanged(state)) => state.state,
                    Some(execution_mutation::Change::Finished(finished)) => finished.state,
                    None => self.execution_state,
                };
            }
            conversation_mutation::Mutation::Entry(entry) => match entry.change.as_ref() {
                Some(entry_mutation::Change::Created(created)) => {
                    let existing_media = self
                        .entries
                        .get(&entry.entry_id)
                        .map_or(0, |entry| entry.media.len());
                    self.ensure_replacement_media(existing_media, created.media.len())?;
                    if !self.entries.contains_key(&entry.entry_id) {
                        if self.entries.len() >= MAX_TRANSCRIPT_ENTRIES {
                            return Err(SemanticReducerError::RetentionLimit);
                        }
                        self.entry_order.push(entry.entry_id.clone());
                    }
                    self.media_reference_count =
                        self.media_reference_count.saturating_sub(existing_media)
                            + created.media.len();
                    self.entries
                        .entry(entry.entry_id.clone())
                        .and_modify(|projected| {
                            projected.kind = created.kind;
                            projected
                                .parent_entry_id
                                .clone_from(&created.parent_entry_id);
                            projected.media = semantic_media(&created.media);
                        })
                        .or_insert_with(|| SemanticEntry {
                            kind: created.kind,
                            state: EntryState::Streaming as i32,
                            parent_entry_id: created.parent_entry_id.clone(),
                            media: semantic_media(&created.media),
                            ..Default::default()
                        });
                }
                Some(entry_mutation::Change::Finalized(finalized)) => {
                    self.entries
                        .get_mut(&entry.entry_id)
                        .ok_or(SemanticReducerError::UnknownEntry)?
                        .state = finalized.state;
                }
                None => {}
            },
            conversation_mutation::Mutation::Content(content) => {
                let entry = self
                    .entries
                    .get(&content.entry_id)
                    .ok_or(SemanticReducerError::UnknownEntry)?;
                if !entry.contents.contains_key(&content.content_index)
                    && entry.contents.len() != content.content_index as usize
                {
                    return Err(SemanticReducerError::ContentIndexConflict);
                }
                let existing = entry.contents.get(&content.content_index);
                if existing.is_some_and(|projected| {
                    !projected.content_id.is_empty() && projected.content_id != content.content_id
                }) {
                    return Err(SemanticReducerError::ContentIdentityConflict);
                }
                let existing_data_len = existing.map_or(0, |content| content.data.len());
                let existing_media = existing.map_or(0, |content| content.media.len());

                match content.change.as_ref() {
                    Some(content_mutation::Change::Delta(delta)) => {
                        if existing.map_or(0, |content| content.next_delta_index)
                            != delta.delta_index
                        {
                            return Err(SemanticReducerError::DeltaIndexMismatch);
                        }
                        let next_bytes = self.checked_retained_bytes(0, delta.data.len())?;
                        let projected = self
                            .entries
                            .get_mut(&content.entry_id)
                            .expect("entry was checked")
                            .contents
                            .entry(content.content_index)
                            .or_default();
                        projected.content_id.clone_from(&content.content_id);
                        projected.encoding = delta.encoding;
                        projected.data.extend_from_slice(&delta.data);
                        projected.next_delta_index += 1;
                        self.retained_bytes = next_bytes;
                    }
                    Some(content_mutation::Change::Final(final_content)) => {
                        self.ensure_replacement_media(existing_media, final_content.media.len())?;
                        let next_bytes = self
                            .checked_retained_bytes(existing_data_len, final_content.data.len())?;
                        let projected = self
                            .entries
                            .get_mut(&content.entry_id)
                            .expect("entry was checked")
                            .contents
                            .entry(content.content_index)
                            .or_default();
                        projected.content_id.clone_from(&content.content_id);
                        projected.encoding = final_content.encoding;
                        projected.data.clone_from(&final_content.data);
                        projected.media = semantic_media(&final_content.media);
                        self.retained_bytes = next_bytes;
                        self.media_reference_count =
                            self.media_reference_count.saturating_sub(existing_media)
                                + final_content.media.len();
                    }
                    None => {}
                }
            }
            conversation_mutation::Mutation::Action(action) => {
                let entry = self
                    .entries
                    .get(&action.entry_id)
                    .ok_or(SemanticReducerError::UnknownEntry)?;
                let existing = entry.actions.get(&action.action_id);
                let is_new = existing.is_none();
                if is_new && self.action_count >= MAX_TRANSCRIPT_ACTIONS {
                    return Err(SemanticReducerError::RetentionLimit);
                }

                match action.change.as_ref() {
                    Some(action_mutation::Change::Started(started)) => {
                        let existing_len = existing.map_or(0, |action| action.input_json.len());
                        let next_bytes =
                            self.checked_retained_bytes(existing_len, started.input_json.len())?;
                        let entry = self
                            .entries
                            .get_mut(&action.entry_id)
                            .expect("entry was checked");
                        if is_new {
                            entry.action_order.push(action.action_id.clone());
                            self.action_count += 1;
                        }
                        let projected = entry.actions.entry(action.action_id.clone()).or_default();
                        projected.kind = started.kind;
                        projected.name.clone_from(&started.name);
                        projected.state = ActionState::Running as i32;
                        projected.input_json.clone_from(&started.input_json);
                        self.retained_bytes = next_bytes;
                    }
                    Some(action_mutation::Change::StateChanged(state)) => {
                        let existing_len = existing.map_or(0, |action| action.detail.len());
                        let next_bytes =
                            self.checked_retained_bytes(existing_len, state.detail.len())?;
                        let entry = self
                            .entries
                            .get_mut(&action.entry_id)
                            .expect("entry was checked");
                        if is_new {
                            entry.action_order.push(action.action_id.clone());
                            self.action_count += 1;
                        }
                        let projected = entry.actions.entry(action.action_id.clone()).or_default();
                        projected.state = state.state;
                        projected.detail.clone_from(&state.detail);
                        self.retained_bytes = next_bytes;
                    }
                    Some(action_mutation::Change::Result(result)) => {
                        let existing_len = existing.map_or(0, |action| {
                            action.output_json.len() + action.error_message.len()
                        });
                        let existing_media = existing.map_or(0, |action| action.media.len());
                        self.ensure_replacement_media(existing_media, result.media.len())?;
                        let next_bytes = self.checked_retained_bytes(
                            existing_len,
                            result.output_json.len() + result.error_message.len(),
                        )?;
                        let entry = self
                            .entries
                            .get_mut(&action.entry_id)
                            .expect("entry was checked");
                        if is_new {
                            entry.action_order.push(action.action_id.clone());
                            self.action_count += 1;
                        }
                        let projected = entry.actions.entry(action.action_id.clone()).or_default();
                        projected.state = result.state;
                        projected.output_json.clone_from(&result.output_json);
                        projected.media = semantic_media(&result.media);
                        projected.error_message.clone_from(&result.error_message);
                        self.retained_bytes = next_bytes;
                        self.media_reference_count =
                            self.media_reference_count.saturating_sub(existing_media)
                                + result.media.len();
                    }
                    None => {}
                }
            }
            conversation_mutation::Mutation::Interruption(interruption) => {
                if interruption.scope
                    == warp_conversation_mutation_api::InterruptionScope::Execution as i32
                {
                    self.execution_state = ExecutionState::Interrupted as i32;
                }
            }
            conversation_mutation::Mutation::Delivery(_)
            | conversation_mutation::Mutation::Control(_) => {}
        }
        self.processed_bytes = processed_bytes;
        Ok(())
    }

    fn checked_retained_bytes(
        &self,
        replaced: usize,
        replacement: usize,
    ) -> Result<usize, SemanticReducerError> {
        let next = self
            .retained_bytes
            .saturating_sub(replaced)
            .checked_add(replacement)
            .ok_or(SemanticReducerError::RetentionLimit)?;
        if next > MAX_TRANSCRIPT_RETAINED_BYTES {
            return Err(SemanticReducerError::RetentionLimit);
        }
        Ok(next)
    }

    fn ensure_replacement_media(
        &self,
        replaced: usize,
        replacement: usize,
    ) -> Result<(), SemanticReducerError> {
        if self
            .media_reference_count
            .saturating_sub(replaced)
            .saturating_add(replacement)
            > MAX_TRANSCRIPT_MEDIA_REFERENCES
        {
            return Err(SemanticReducerError::RetentionLimit);
        }
        Ok(())
    }
}

impl SemanticEntry {
    pub fn kind(&self) -> Option<EntryKind> {
        EntryKind::try_from(self.kind).ok()
    }
}

pub fn semantic_payload_key(
    mutation: &ConversationMutation,
) -> Result<String, SemanticReducerError> {
    let payload = match mutation
        .mutation
        .as_ref()
        .ok_or(SemanticReducerError::MissingMutation)?
    {
        conversation_mutation::Mutation::Execution(execution) => match execution.change.as_ref() {
            Some(execution_mutation::Change::Started(_)) => "execution:started".to_owned(),
            Some(execution_mutation::Change::StateChanged(state)) => {
                format!("execution:state:{}", state.state)
            }
            Some(execution_mutation::Change::Finished(finished)) => {
                format!("execution:finished:{}", finished.state)
            }
            None => "execution:empty".to_owned(),
        },
        conversation_mutation::Mutation::Entry(entry) => match entry.change.as_ref() {
            Some(entry_mutation::Change::Created(_)) => {
                format!("entry:{}:created", entry.entry_id)
            }
            Some(entry_mutation::Change::Finalized(finalized)) => {
                format!("entry:{}:finalized:{}", entry.entry_id, finalized.state)
            }
            None => format!("entry:{}:empty", entry.entry_id),
        },
        conversation_mutation::Mutation::Content(content) => match content.change.as_ref() {
            Some(content_mutation::Change::Delta(delta)) => format!(
                "content:{}:{}:{}:delta:{}",
                content.entry_id, content.content_id, content.content_index, delta.delta_index
            ),
            Some(content_mutation::Change::Final(_)) => format!(
                "content:{}:{}:{}:final",
                content.entry_id, content.content_id, content.content_index
            ),
            None => format!(
                "content:{}:{}:{}:empty",
                content.entry_id, content.content_id, content.content_index
            ),
        },
        conversation_mutation::Mutation::Action(action) => match action.change.as_ref() {
            Some(action_mutation::Change::Started(_)) => {
                format!("action:{}:{}:started", action.entry_id, action.action_id)
            }
            Some(action_mutation::Change::StateChanged(state)) => format!(
                "action:{}:{}:state:{}",
                action.entry_id, action.action_id, state.state
            ),
            Some(action_mutation::Change::Result(result)) => format!(
                "action:{}:{}:result:{}",
                action.entry_id, action.action_id, result.state
            ),
            None => format!("action:{}:{}:empty", action.entry_id, action.action_id),
        },
        conversation_mutation::Mutation::Delivery(delivery) => format!(
            "delivery:{}:{}:{}",
            delivery.delivery_id, delivery.entry_id, delivery.state
        ),
        conversation_mutation::Mutation::Interruption(interruption) => format!(
            "interruption:{}:{}:{}",
            interruption.scope, interruption.target_id, interruption.reason
        ),
        conversation_mutation::Mutation::Control(control) => format!(
            "control:{}:{}:{}:{}",
            control.control_id, control.kind, control.state, control.target_id
        ),
    };
    Ok(payload)
}

fn semantic_media(media: &[MediaReference]) -> Vec<SemanticMedia> {
    media
        .iter()
        .map(|reference| SemanticMedia {
            media_id: reference.media_id.clone(),
            kind: reference.kind,
            mime_type: reference.mime_type.clone(),
            size_bytes: reference.size_bytes,
            display_name: reference.display_name.clone(),
            reference: reference.reference.clone(),
        })
        .collect()
}
