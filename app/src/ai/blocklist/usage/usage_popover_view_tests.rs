use super::*;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::usage::rollup::{AgentAvatar, PerAgentCreditEntry};

fn per_agent_entry(name: &str, credits: f32) -> PerAgentCreditEntry {
    PerAgentCreditEntry {
        conversation_id: AIConversationId::new(),
        display_name: name.to_string(),
        avatar: AgentAvatar::Child,
        credits_spent: credits,
        cost_in_cents: None,
        tokens: None,
    }
}

#[test]
fn truncate_rollup_rows_shows_all_under_cap() {
    let entries: Vec<_> = (0..3)
        .map(|i| per_agent_entry(&format!("agent-{i}"), 1.0))
        .collect();
    let (shown, hidden) = truncate_rollup_rows(&entries, false);
    assert_eq!(shown.len(), 3);
    assert_eq!(hidden, 0);
}

#[test]
fn truncate_rollup_rows_truncates_over_cap_until_show_all() {
    let entries: Vec<_> = (0..8)
        .map(|i| per_agent_entry(&format!("agent-{i}"), 1.0))
        .collect();
    let (shown, hidden) = truncate_rollup_rows(&entries, false);
    assert_eq!(shown.len(), ROLLUP_TRUNCATION_CAP);
    assert_eq!(hidden, 3);

    let (shown_all, hidden_all) = truncate_rollup_rows(&entries, true);
    assert_eq!(shown_all.len(), 8);
    assert_eq!(hidden_all, 0);
}
