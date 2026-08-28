use session_sharing_protocol::common::SessionId;
use warp_semantic_session::RequestedSessionContent;

use crate::ai::agent::api::ServerConversationToken;
use crate::ai::ambient_agents::{AmbientAgentLiveSessionState, AmbientAgentTask};

/// How to materialize a child agent pane given its [`AmbientAgentTask`].
/// See [`decide_child_pane_materialization`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChildPaneMaterialization {
    /// Attachable live session — join it in place using `session_id`.
    AttachLive {
        session_id: SessionId,
        requested_content: RequestedSessionContent,
    },
    /// The server explicitly requested semantic mode, but its capability was
    /// malformed or unsupported. Never downgrade this child to terminal mode.
    InvalidSemanticCapability,
    /// No live session but a server conversation token is available; load
    /// the cloud transcript for it.
    LoadTranscript {
        server_token: ServerConversationToken,
    },
    /// Neither a live session nor a loadable transcript is available yet;
    /// leave the pane pending until task data changes.
    Pending,
}

/// Origin-agnostic pane dispatch: identical task state produces the same
/// materialization action.
///
/// Free-standing so it's unit-testable without a `PaneGroup`.
pub(crate) fn decide_child_pane_materialization(
    task: &AmbientAgentTask,
) -> ChildPaneMaterialization {
    if let AmbientAgentLiveSessionState::Attachable { session_id } =
        task.active_live_session_state()
    {
        return match task.requested_session_content() {
            Ok(requested_content) => ChildPaneMaterialization::AttachLive {
                session_id,
                requested_content,
            },
            Err(err) => {
                log::error!(
                    "Refusing child viewer with invalid Factory semantic capability: {err:#}"
                );
                ChildPaneMaterialization::InvalidSemanticCapability
            }
        };
    }

    // Only terminal runs load a transcript. Empty/whitespace tokens would
    // drive a no-op cloud fetch, so treat them as absent.
    if task.is_terminal_run_state()
        && let Some(server_token) = task
            .conversation_id()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| ServerConversationToken::new(t.to_string()))
    {
        return ChildPaneMaterialization::LoadTranscript { server_token };
    }

    ChildPaneMaterialization::Pending
}

#[cfg(test)]
#[path = "materialization_tests.rs"]
mod tests;
