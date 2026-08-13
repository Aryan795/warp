use chrono::TimeZone;

use super::*;
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageBucket, AiCreditsUsageSource,
};

fn utc(year: i32, month: u32, day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
}

fn summary(start: DateTime<Utc>, end: DateTime<Utc>) -> BillingCycleUsageSummary {
    BillingCycleUsageSummary {
        period_start: start,
        period_end: end,
        entries: vec![],
    }
}

fn sample_summaries() -> Vec<BillingCycleUsageSummary> {
    vec![
        summary(utc(2026, 6, 27), utc(2026, 7, 27)),
        summary(utc(2026, 5, 27), utc(2026, 6, 27)),
        summary(utc(2026, 4, 27), utc(2026, 5, 27)),
    ]
}

#[test]
fn builds_one_plain_item_per_period() {
    let summaries = sample_summaries();
    let items = build_period_menu_items(&summaries);

    assert_eq!(items.len(), summaries.len());
    for (item, summary) in items.iter().zip(summaries.iter()) {
        match item {
            MenuItem::Item(fields) => {
                assert_eq!(fields.icon(), None, "items should not carry a marker icon");
                match fields.on_select_action() {
                    Some(BillingCycleUsageAction::SelectPeriod(Some(end))) => {
                        assert_eq!(*end, summary.period_end);
                    }
                    other => panic!("expected SelectPeriod action, got {other:?}"),
                }
            }
            other => panic!("expected MenuItem::Item, got {other:?}"),
        }
    }
}

#[test]
fn selects_most_recent_period_when_none_selected() {
    let summaries = sample_summaries();
    assert_eq!(selected_period_index(&summaries, None), Some(0));
}

#[test]
fn selects_explicitly_selected_period() {
    let summaries = sample_summaries();
    assert_eq!(
        selected_period_index(&summaries, Some(utc(2026, 6, 27))),
        Some(1),
    );
    assert_eq!(
        selected_period_index(&summaries, Some(utc(2026, 5, 27))),
        Some(2),
    );
}

#[test]
fn selects_nothing_when_selection_absent() {
    let summaries = sample_summaries();
    assert_eq!(
        selected_period_index(&summaries, Some(utc(1999, 1, 1))),
        None
    );
}

#[test]
fn selects_nothing_when_no_summaries() {
    assert_eq!(selected_period_index(&[], None), None);
    assert_eq!(selected_period_index(&[], Some(utc(2026, 7, 27))), None);
}

fn usage_entry(subject_uid: &str, attributed_team_uid: Option<&str>) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        subject_type: AiCreditsUsageAndCostSubjectType::User,
        subject_uid: Some(subject_uid.to_string()),
        subject_display_name: None,
        cost_type: AiCreditsUsageAndCostType::BaseLimit,
        usage_bucket: AiCreditsUsageBucket::Ai,
        usage_source: AiCreditsUsageSource::Local,
        credits_used: 10,
        cost_cents: 5,
        attributed_team_uid: attributed_team_uid.map(|s| s.to_string()),
    }
}

#[test]
fn filter_entries_by_attributed_team_keeps_only_matching_team() {
    let entries = vec![
        usage_entry("a-member", Some("team-a")),
        usage_entry("b-member", Some("team-b")),
        usage_entry("unassigned", None),
    ];

    let filtered = filter_entries_by_attributed_team(&entries, "team-a");

    let subject_uids: Vec<&str> = filtered
        .iter()
        .map(|e| e.subject_uid.as_deref().unwrap())
        .collect();
    assert_eq!(subject_uids, ["a-member"]);
}

#[test]
fn filter_entries_by_attributed_team_keeps_service_accounts_attributed_to_team() {
    let mut service_account = usage_entry("service-account-a", Some("team-a"));
    service_account.subject_type = AiCreditsUsageAndCostSubjectType::ServiceAccount;
    let entries = vec![
        service_account,
        usage_entry("service-account-b", Some("team-b")),
    ];

    let filtered = filter_entries_by_attributed_team(&entries, "team-a");

    assert_eq!(filtered.len(), 1);
    assert_eq!(
        filtered[0].subject_uid.as_deref(),
        Some("service-account-a")
    );
}
