use chrono::Utc;

use super::{ChildPaneMaterialization, decide_child_pane_materialization};
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::ambient_agents::task::FactorySemanticSession;
use crate::ai::ambient_agents::{AmbientAgentTask, AmbientAgentTaskState};

/// Builds a minimal [`AmbientAgentTask`] for materialization tests.
///
/// `state`, `is_sandbox_running`, and `session_id` combine to determine the
/// task's live-session state (see
/// [`AmbientAgentTask::active_live_session_state`]): an `InProgress` task
/// with a running sandbox and a parseable UUID-shaped `session_id` is
/// `Attachable`. `conversation_id` populates the token used by the
/// `LoadTranscript` branch.
fn task(
    state: AmbientAgentTaskState,
    is_sandbox_running: bool,
    session_id: Option<&str>,
    conversation_id: Option<&str>,
) -> AmbientAgentTask {
    let now = Utc::now();
    AmbientAgentTask {
        task_id: "11111111-1111-1111-1111-111111111111".parse().unwrap(),
        parent_run_id: None,
        title: "Task".to_string(),
        state,
        prompt: String::new(),
        created_at: now,
        started_at: Some(now),
        updated_at: now,
        run_time: None,
        status_message: None,
        source: None,
        execution_location: None,
        session_id: session_id.map(str::to_string),
        session_link: None,
        creator: None,
        executor: None,
        conversation_id: conversation_id.map(str::to_string),
        request_usage: None,
        is_sandbox_running,
        agent_config_snapshot: None,
        artifacts: vec![],
        last_event_sequence: None,
        factory_semantic_session: None,
        children: vec![],
    }
}

#[test]
fn attachable_factory_task_preserves_semantic_content() {
    let session_id = "22222222-2222-2222-2222-222222222222";
    let mut task = task(
        AmbientAgentTaskState::InProgress,
        true,
        Some(session_id),
        Some("server-token-irrelevant-for-attach"),
    );
    task.factory_semantic_session = Some(FactorySemanticSession {
        content_mode: "semantic_conversation_only".to_string(),
        schema_version: 1,
        conversation_id: "factory-conversation".to_string(),
        execution_id: "factory-execution".to_string(),
        run_id: task.task_id.to_string(),
        request_id: Some("factory-request".to_string()),
        session_id: Some(session_id.to_string()),
        accepted_message_context: None,
    });

    let ChildPaneMaterialization::AttachLive {
        session_id: attached_session_id,
        requested_content,
    } = decide_child_pane_materialization(&task)
    else {
        panic!("valid Factory capability should attach the semantic child");
    };
    assert_eq!(attached_session_id, session_id.parse().unwrap());
    assert_eq!(
        requested_content.execution_identity().unwrap().execution_id,
        "factory-execution"
    );
    assert!(requested_content.is_semantic());
}

#[test]
fn attachable_factory_task_rejects_invalid_semantic_capability() {
    let session_id = "22222222-2222-2222-2222-222222222222";
    let mut task = task(
        AmbientAgentTaskState::InProgress,
        true,
        Some(session_id),
        None,
    );
    task.factory_semantic_session = Some(FactorySemanticSession {
        content_mode: "semantic_conversation_only".to_string(),
        schema_version: 2,
        conversation_id: "factory-conversation".to_string(),
        execution_id: "factory-execution".to_string(),
        run_id: task.task_id.to_string(),
        request_id: Some("factory-request".to_string()),
        session_id: Some(session_id.to_string()),
        accepted_message_context: None,
    });

    assert_eq!(
        decide_child_pane_materialization(&task),
        ChildPaneMaterialization::InvalidSemanticCapability
    );
}

#[test]
fn terminal_task_with_stale_session_id_loads_transcript() {
    let task = task(
        AmbientAgentTaskState::Succeeded,
        false,
        Some("22222222-2222-2222-2222-222222222222"),
        Some("completed-child-token"),
    );

    assert_eq!(
        decide_child_pane_materialization(&task),
        ChildPaneMaterialization::LoadTranscript {
            server_token: ServerConversationToken::new("completed-child-token".to_string()),
        },
        "a terminal child must never join its stale execution session",
    );
}

#[test]
fn attachable_task_attaches_live_with_session_id() {
    let task = task(
        AmbientAgentTaskState::InProgress,
        true,
        Some("22222222-2222-2222-2222-222222222222"),
        // A conversation_id is present but must be ignored in favor of the
        // live session.
        Some("server-token-irrelevant-for-attach"),
    );

    assert_eq!(
        decide_child_pane_materialization(&task),
        ChildPaneMaterialization::AttachLive {
            session_id: "22222222-2222-2222-2222-222222222222".parse().unwrap(),
            requested_content: warp_semantic_session::RequestedSessionContent::FullTerminal,
        },
    );
}

#[test]
fn terminal_task_with_conversation_id_loads_transcript() {
    let task = task(
        AmbientAgentTaskState::Succeeded,
        false,
        None,
        Some("my-server-token"),
    );

    assert_eq!(
        decide_child_pane_materialization(&task),
        ChildPaneMaterialization::LoadTranscript {
            server_token: ServerConversationToken::new("my-server-token".to_string()),
        },
    );
}

#[test]
fn non_terminal_non_attachable_tasks_are_pending() {
    for state in [
        AmbientAgentTaskState::Queued,
        AmbientAgentTaskState::Pending,
        AmbientAgentTaskState::Claimed,
    ] {
        // A conversation_id is present but the run is not terminal, so there
        // is no transcript to load yet.
        let task = task(state.clone(), false, None, Some("server-token"));
        assert_eq!(
            decide_child_pane_materialization(&task),
            ChildPaneMaterialization::Pending,
            "state={state:?} should be Pending",
        );
    }
}

#[test]
fn terminal_task_without_conversation_id_is_pending() {
    let task = task(AmbientAgentTaskState::Succeeded, false, None, None);
    assert_eq!(
        decide_child_pane_materialization(&task),
        ChildPaneMaterialization::Pending,
    );
}

#[test]
fn terminal_task_with_blank_conversation_id_is_pending() {
    for blank in ["", "   ", "\t\n"] {
        let task = task(AmbientAgentTaskState::Succeeded, false, None, Some(blank));
        assert_eq!(
            decide_child_pane_materialization(&task),
            ChildPaneMaterialization::Pending,
            "blank conversation_id={blank:?} should be Pending",
        );
    }
}
