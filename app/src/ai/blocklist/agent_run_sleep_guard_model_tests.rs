use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::*;

#[test]
fn agent_run_sleep_guard_model_lifecycle_refresh_and_cap_expiry() {
    let mut model = AgentRunSleepGuardModel {
        guards: HashMap::new(),
        expiry_timer: None,
    };
    let conversation_id = AIConversationId::new();

    model.apply_status_for_test(conversation_id, ConversationStatus::InProgress);
    assert_eq!(model.held_guard_count(), 1);
    model.apply_status_for_test(conversation_id, ConversationStatus::TransientError);
    assert_eq!(model.held_guard_count(), 1);

    let deadline = Instant::now() + Duration::from_secs(1);
    model.set_deadline_for_test(conversation_id, deadline);
    model.refresh_for_test(conversation_id);
    model.expire_for_test_no_ctx(Instant::now() + Duration::from_secs(2));
    assert_eq!(model.held_guard_count(), 1);
    model.set_deadline_for_test(conversation_id, Instant::now() - Duration::from_secs(1));
    model.expire_for_test_no_ctx(Instant::now());
    assert_eq!(model.held_guard_count(), 0);

    model.apply_status_for_test(conversation_id, ConversationStatus::InProgress);
    assert_eq!(model.held_guard_count(), 1);
    for status in [
        ConversationStatus::Success,
        ConversationStatus::Error,
        ConversationStatus::Cancelled,
        ConversationStatus::WaitingForEvents,
        ConversationStatus::Blocked {
            blocked_action: "approval".to_string(),
        },
    ] {
        model.apply_status_for_test(conversation_id, status);
        assert_eq!(model.held_guard_count(), 0);
        model.apply_status_for_test(conversation_id, ConversationStatus::InProgress);
        assert_eq!(model.held_guard_count(), 1);
    }

    model.release(conversation_id);
    assert_eq!(model.held_guard_count(), 0);
}
