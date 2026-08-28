use prost::Message;
use serde_json::{Value, json};
use warp_conversation_mutation_api::ConversationMutation;
use warp_semantic_session::{SemanticMedia, SemanticTranscript, semantic_payload_key};
use wasm_bindgen::prelude::*;
const MAX_CANONICAL_MUTATION_BYTES: usize = 256 * 1024;

#[wasm_bindgen]
#[derive(Default)]
pub struct SemanticTranscriptClient {
    transcript: SemanticTranscript,
}

#[wasm_bindgen]
impl SemanticTranscriptClient {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply_protobuf_mutation(&mut self, bytes: &[u8]) -> Result<(), JsError> {
        let mutation = decode_mutation(bytes)?;
        self.transcript
            .apply(&mutation)
            .map_err(|error| JsError::new(&error.to_string()))
    }

    pub fn reset(&mut self) {
        self.transcript = SemanticTranscript::default();
    }

    pub fn mutation_key(&self, bytes: &[u8]) -> Result<String, JsError> {
        let mutation = decode_mutation(bytes)?;
        semantic_payload_key(&mutation).map_err(|error| JsError::new(&error.to_string()))
    }

    pub fn execution_state(&self) -> i32 {
        self.transcript.execution_state
    }

    pub fn entry_count(&self) -> usize {
        self.transcript.entries.len()
    }

    pub fn entry_text(&self, entry_id: &str) -> Option<String> {
        let entry = self.transcript.entries.get(entry_id)?;
        let mut bytes = Vec::new();
        for content in entry.contents.values() {
            bytes.extend_from_slice(&content.data);
        }
        String::from_utf8(bytes).ok()
    }

    pub fn renderable_state_json(&self) -> Result<String, JsError> {
        let entries = self
            .transcript
            .entry_order
            .iter()
            .filter_map(|entry_id| {
                let entry = self.transcript.entries.get(entry_id)?;
                let contents = entry
                    .contents
                    .iter()
                    .map(|(content_index, content)| {
                        json!({
                            "content_id": content.content_id,
                            "content_index": content_index,
                            "encoding": content.encoding,
                            "text": String::from_utf8(content.data.clone()).ok(),
                            "media": media_json(&content.media),
                        })
                    })
                    .collect::<Vec<_>>();
                let actions = entry
                    .action_order
                    .iter()
                    .filter_map(|action_id| {
                        let action = entry.actions.get(action_id)?;
                        Some(json!({
                            "action_id": action_id,
                            "kind": action.kind,
                            "name": action.name,
                            "state": action.state,
                            "input": parse_json(&action.input_json),
                            "output": parse_json(&action.output_json),
                            "media": media_json(&action.media),
                            "detail": action.detail,
                            "error_message": action.error_message,
                        }))
                    })
                    .collect::<Vec<_>>();
                Some(json!({
                    "entry_id": entry_id,
                    "kind": entry.kind,
                    "state": entry.state,
                    "parent_entry_id": entry.parent_entry_id,
                    "media": media_json(&entry.media),
                    "contents": contents,
                    "actions": actions,
                }))
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&json!({
            "execution_state": self.transcript.execution_state,
            "entries": entries,
        }))
        .map_err(|error| JsError::new(&format!("failed to encode transcript projection: {error}")))
    }
}

fn decode_mutation(bytes: &[u8]) -> Result<ConversationMutation, JsError> {
    if bytes.is_empty() || bytes.len() > MAX_CANONICAL_MUTATION_BYTES {
        return Err(JsError::new(
            "canonical conversation mutation size is invalid",
        ));
    }
    ConversationMutation::decode(bytes)
        .map_err(|error| JsError::new(&format!("invalid conversation mutation: {error}")))
}

fn parse_json(bytes: &[u8]) -> Value {
    if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes).into_owned()))
    }
}

fn media_json(media: &[SemanticMedia]) -> Vec<Value> {
    media
        .iter()
        .map(|reference| {
            json!({
                "media_id": reference.media_id,
                "kind": reference.kind,
                "mime_type": reference.mime_type,
                "size_bytes": reference.size_bytes,
                "display_name": reference.display_name,
                "reference": reference.reference,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use prost::Message as _;
    use serde_json::Value;
    use warp_conversation_mutation_api::{
        ContentEncoding, ContentMutation, ConversationMutation, EntryKind, EntryMutation,
        MutationIdentity, content_mutation, conversation_mutation, entry_mutation,
    };

    use super::*;

    #[test]
    fn decodes_protobuf_mutations_through_the_wasm_boundary() {
        let mutation = ConversationMutation {
            mutation: Some(conversation_mutation::Mutation::Entry(EntryMutation {
                entry_id: "entry".to_owned(),
                change: Some(entry_mutation::Change::Created(entry_mutation::Created {
                    kind: EntryKind::AssistantMessage as i32,
                    ..Default::default()
                })),
            })),
            ..Default::default()
        };
        let mut client = SemanticTranscriptClient::new();
        client
            .apply_protobuf_mutation(&mutation.encode_to_vec())
            .unwrap();
        assert_eq!(client.entry_count(), 1);
    }

    #[test]
    fn exposes_ordered_renderable_entries_and_stable_delta_keys() {
        let mut client = SemanticTranscriptClient::new();
        let entry = ConversationMutation {
            identity: Some(MutationIdentity {
                mutation_id: "entry-created".to_owned(),
                ..Default::default()
            }),
            mutation: Some(conversation_mutation::Mutation::Entry(EntryMutation {
                entry_id: "entry".to_owned(),
                change: Some(entry_mutation::Change::Created(entry_mutation::Created {
                    kind: EntryKind::AssistantMessage as i32,
                    ..Default::default()
                })),
            })),
            ..Default::default()
        };
        client
            .apply_protobuf_mutation(&entry.encode_to_vec())
            .unwrap();

        let delta = ConversationMutation {
            identity: Some(MutationIdentity {
                mutation_id: "content-delta".to_owned(),
                ..Default::default()
            }),
            mutation: Some(conversation_mutation::Mutation::Content(ContentMutation {
                entry_id: "entry".to_owned(),
                content_id: "content".to_owned(),
                content_index: 0,
                change: Some(content_mutation::Change::Delta(content_mutation::Delta {
                    delta_index: 0,
                    encoding: ContentEncoding::Utf8Markdown as i32,
                    data: b"hello".to_vec(),
                })),
            })),
            ..Default::default()
        };
        let bytes = delta.encode_to_vec();
        assert_eq!(
            client.mutation_key(&bytes).unwrap(),
            "content:entry:content:0:delta:0"
        );
        client.apply_protobuf_mutation(&bytes).unwrap();

        let state: Value = serde_json::from_str(&client.renderable_state_json().unwrap()).unwrap();
        assert_eq!(state["entries"][0]["entry_id"], "entry");
        assert_eq!(state["entries"][0]["contents"][0]["text"], "hello");

        client.reset();
        assert_eq!(client.entry_count(), 0);
    }
}
