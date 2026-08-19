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
fn cancelled_fetch_conversation_always_triggers_follow_up_request() {
    // FetchConversation has no user-facing cancel affordance, so any cancellation of it
    // is always collateral damage from cancelling the surrounding conversation's
    // progress, never a deliberate user click. The nested ConversationSearchAgent
    // subagent blocks on a result for this tool call, so completion must always trigger
    // a follow-up request rather than silently leaving the conversation stuck (which
    // otherwise surfaces to the user as a spurious "cancelled" conversation, since
    // `BlocklistAIController` marks the conversation Cancelled locally instead of
    // sending a follow-up when no finished result triggers one).
    let result = AIAgentActionResultType::FetchConversation(FetchConversationResult::Cancelled);

    assert!(result.is_cancelled());
    assert!(result.should_trigger_request_upon_completion());
}

#[test]
fn other_cancelled_results_do_not_trigger_a_follow_up_request() {
    // Sanity check that the FetchConversation carve-out is scoped narrowly and doesn't
    // change the documented behavior for actions with a genuine user-facing cancel
    // affordance (e.g. RunAgents' Reject button), which rely on the server's input
    // interceptor to synthesize the generic `ToolCallResult.Cancel` marker instead.
    let result = AIAgentActionResultType::RunAgents(RunAgentsResult::Cancelled);

    assert!(result.is_cancelled());
    assert!(!result.should_trigger_request_upon_completion());
}
