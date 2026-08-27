use regex::Regex;
use serial_test::serial;

use super::*;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentActionId, AIAgentActionResult, GrepResult};
use crate::terminal::model::secrets;

fn action_result_input(context: Arc<[AIAgentContext]>) -> AIAgentInput {
    AIAgentInput::ActionResult {
        result: AIAgentActionResult {
            id: AIAgentActionId::from("action-id".to_string()),
            task_id: TaskId::new("task-id".to_string()),
            result: AIAgentActionResultType::Grep(GrepResult::Cancelled),
        },
        context,
    }
}

fn context_arc() -> Arc<[AIAgentContext]> {
    Arc::from(vec![AIAgentContext::SelectedText("SECRET123".to_string())])
}

/// A batch of inputs sharing one context arc (as `send_query` produces for a batch of completed
/// action results) should redact the shared content exactly once and have every sharer end up
/// pointing at that single redacted allocation, rather than each cloning its own copy.
#[test]
#[serial]
fn test_redact_inputs_reuses_one_redacted_copy_for_a_shared_context() {
    secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("SECRET123").expect("valid regex")],
        std::iter::empty(),
    );

    let shared_context = context_arc();
    let mut inputs = vec![
        action_result_input(shared_context.clone()),
        action_result_input(shared_context.clone()),
        action_result_input(shared_context.clone()),
    ];
    // Drop the extra local reference so only the inputs themselves hold the arc, matching how
    // `send_query` releases its `context` variable before the request is redacted.
    drop(shared_context);

    redact_inputs(&mut inputs);

    let contexts: Vec<&Arc<[AIAgentContext]>> = inputs
        .iter()
        .map(|input| match input {
            AIAgentInput::ActionResult { context, .. } => context,
            _ => unreachable!(),
        })
        .collect();

    // All three inputs should share the exact same redacted allocation...
    assert!(Arc::ptr_eq(contexts[0], contexts[1]));
    assert!(Arc::ptr_eq(contexts[1], contexts[2]));
    // ...redacted only once, not once per sharer.
    assert_eq!(Arc::strong_count(contexts[0]), 3);

    // Redaction still took effect.
    let AIAgentContext::SelectedText(text) = &contexts[0][0] else {
        panic!("expected SelectedText context");
    };
    assert_eq!(text, "*********");
}

/// Inputs that do not share a context should each keep their own independently-redacted copy.
#[test]
#[serial]
fn test_redact_inputs_does_not_share_contexts_across_unrelated_inputs() {
    secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("SECRET123").expect("valid regex")],
        std::iter::empty(),
    );

    let mut inputs = vec![
        action_result_input(context_arc()),
        action_result_input(context_arc()),
    ];

    redact_inputs(&mut inputs);

    let contexts: Vec<&Arc<[AIAgentContext]>> = inputs
        .iter()
        .map(|input| match input {
            AIAgentInput::ActionResult { context, .. } => context,
            _ => unreachable!(),
        })
        .collect();

    assert!(!Arc::ptr_eq(contexts[0], contexts[1]));
    for context in contexts {
        let AIAgentContext::SelectedText(text) = &context[0] else {
            panic!("expected SelectedText context");
        };
        assert_eq!(text, "*********");
    }
}
