use super::{
    AIAgentActionResultType, FetchConversationResult, RunAgentsAgentOutcome,
    RunAgentsAgentOutcomeKind, RunAgentsLaunchedExecutionMode, RunAgentsResult,
};

fn launched_agent(name: &str) -> RunAgentsAgentOutcome {
    RunAgentsAgentOutcome {
        name: name.to_string(),
        kind: RunAgentsAgentOutcomeKind::Launched {
            agent_id: format!("{name}-id"),
        },
        resolved_model_id: String::new(),
    }
}

fn failed_agent(name: &str) -> RunAgentsAgentOutcome {
    RunAgentsAgentOutcome {
        name: name.to_string(),
        kind: RunAgentsAgentOutcomeKind::Failed {
            error: "launch failed".to_string(),
        },
        resolved_model_id: String::new(),
    }
}

fn run_agents_result(agents: Vec<RunAgentsAgentOutcome>) -> AIAgentActionResultType {
    AIAgentActionResultType::RunAgents(RunAgentsResult::Launched {
        model_id: "auto".to_string(),
        harness_type: "oz".to_string(),
        execution_mode: RunAgentsLaunchedExecutionMode::Local,
        agents,
    })
}

#[test]
fn run_agents_is_successful_when_all_agents_launch() {
    let result = run_agents_result(vec![launched_agent("first"), launched_agent("second")]);

    assert!(result.is_successful());
    assert!(!result.is_failed());
}

#[test]
fn run_agents_is_successful_when_some_agents_launch() {
    let result = run_agents_result(vec![launched_agent("first"), failed_agent("second")]);
    assert!(result.is_successful());
    assert!(!result.is_failed());
}

#[test]
fn run_agents_is_failed_when_no_agents_launch() {
    let result = run_agents_result(vec![failed_agent("first"), failed_agent("second")]);

    assert!(!result.is_successful());
    assert!(result.is_failed());
}

#[test]
fn cancelled_fetch_conversation_still_triggers_a_follow_up_request() {
    // FetchConversation is a nested, server-driven tool call inside a
    // ConversationSearch subagent flow with no user checkpoint of its own. A
    // cancelled fetch is converted to an error result (see convert.rs) that
    // must still reach the server via a follow-up, or the subagent hangs
    // instead of finishing deterministically.
    let result = AIAgentActionResultType::FetchConversation(FetchConversationResult::Cancelled);

    assert!(result.is_cancelled());
    assert!(result.should_trigger_request_upon_completion());
}

#[test]
fn most_other_cancelled_results_do_not_trigger_a_follow_up_request() {
    // Contrast with the general rule: a user-facing cancellable action
    // (e.g. RunAgents' Reject button) should not auto-trigger a follow-up.
    let result = AIAgentActionResultType::RunAgents(RunAgentsResult::Cancelled);

    assert!(result.is_cancelled());
    assert!(!result.should_trigger_request_upon_completion());
}
