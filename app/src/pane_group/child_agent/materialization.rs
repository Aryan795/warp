use session_sharing_protocol::common::SessionId;

use crate::ai::agent::api::ServerConversationToken;
use crate::ai::ambient_agents::{AmbientAgentLiveSessionState, AmbientAgentTask};

/// Whether a child pane is materialized for the process that owns the
/// orchestrator run (`Owner`) or for a passive viewer of a shared session
/// (`Viewer`).
///
/// Selects the pane *construction* strategy in
/// [`PaneGroup::attach_child_session`]; the materialization *decision*
/// ([`decide_child_pane_materialization`]) is mode-agnostic — owner and
/// viewer make the same choice given identical task state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::pane_group) enum ChildPaneMaterializationMode {
    /// This process owns the orchestrator run: the child attaches to a
    /// cloud-mode ambient pane.
    Owner,
    /// Passive view of a shared session: the child gets its own dedicated
    /// shared-session viewer pane.
    Viewer,
}

/// How to materialize a child agent pane given its [`AmbientAgentTask`].
/// See [`decide_child_pane_materialization`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::pane_group) enum ChildPaneMaterialization {
    /// Attachable live session — join it in place using `session_id`.
    AttachLive { session_id: SessionId },
    /// No live session but a server conversation token is available; load
    /// the cloud transcript for it.
    LoadTranscript {
        server_token: ServerConversationToken,
    },
    /// Neither a live session nor a loadable transcript is available yet;
    /// leave the pane pending until task data changes.
    Pending,
}

/// Mode-agnostic pane dispatch: the same decision is made for owner and
/// viewer given identical task state.
///
/// Free-standing so it's unit-testable without a `PaneGroup`.
pub(in crate::pane_group) fn decide_child_pane_materialization(
    task: &AmbientAgentTask,
) -> ChildPaneMaterialization {
    if let AmbientAgentLiveSessionState::Attachable { session_id } =
        task.active_live_session_state()
    {
        return ChildPaneMaterialization::AttachLive { session_id };
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
