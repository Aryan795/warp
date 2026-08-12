use chrono::TimeZone;

use super::*;
use crate::auth::UserUid;
use crate::server::ids::ServerId;
use crate::workspaces::team::{MembershipRole, TeamMember};
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType as AiCostType,
    AiCreditsUsageBucket, AiCreditsUsageSource, WorkspaceMemberUsageInfo, WorkspaceUid,
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

fn workspace_member(uid: &str, email: &str) -> WorkspaceMember {
    WorkspaceMember {
        uid: UserUid::new(uid),
        email: email.to_string(),
        role: MembershipRole::User,
        usage_info: WorkspaceMemberUsageInfo {
            is_unlimited: false,
            request_limit: 0,
            requests_used_since_last_refresh: 0,
            is_request_limit_prorated: false,
        },
    }
}

fn team_member(uid: &str, email: &str) -> TeamMember {
    TeamMember {
        uid: UserUid::new(uid),
        email: email.to_string(),
        role: MembershipRole::User,
    }
}

fn usage_entry(subject_uid: &str, attributed_team_uid: Option<ServerId>) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        subject_type: AiCreditsUsageAndCostSubjectType::User,
        subject_uid: Some(subject_uid.to_string()),
        subject_display_name: None,
        cost_type: AiCostType::BaseLimit,
        usage_bucket: AiCreditsUsageBucket::Ai,
        usage_source: AiCreditsUsageSource::Local,
        credits_used: 10,
        cost_cents: 0,
        attributed_team_uid: attributed_team_uid.map(|uid| uid.to_string()),
    }
}

fn workspace_with_members(members: Vec<WorkspaceMember>) -> Workspace {
    let mut workspace = Workspace::from_local_cache(
        WorkspaceUid::from(ServerId::from(0i64)),
        "workspace".to_string(),
        None,
    );
    workspace.members = members;
    workspace
}

#[test]
fn resolve_team_scoped_usage_fails_closed_when_team_unresolved() {
    let workspace = workspace_with_members(vec![workspace_member("a", "a@warp.dev")]);
    let entries = vec![usage_entry("a", Some(ServerId::from(0i64)))];

    assert!(resolve_team_scoped_usage(None, &workspace, &entries).is_none());
}

#[test]
fn resolve_team_scoped_usage_filters_entries_and_scopes_members() {
    let team_a_uid = ServerId::from(1i64);
    let team_b_uid = ServerId::from(2i64);

    let workspace = workspace_with_members(vec![
        workspace_member("a", "a@warp.dev"),
        workspace_member("b", "b@warp.dev"),
    ]);
    let team = Team::from_local_cache(
        team_a_uid,
        "Team A".to_string(),
        None,
        None,
        Some(vec![team_member("a", "a@warp.dev")]),
    );

    let entries = vec![
        usage_entry("a", Some(team_a_uid)),
        usage_entry("b", Some(team_b_uid)),
        usage_entry("a", None),
    ];

    let (scoped_entries, scoped_members) =
        resolve_team_scoped_usage(Some(&team), &workspace, &entries)
            .expect("team resolved, should scope");

    assert_eq!(
        scoped_entries.len(),
        1,
        "only team A's attributed entry should remain"
    );
    assert_eq!(scoped_entries[0].subject_uid.as_deref(), Some("a"));

    assert_eq!(
        scoped_members.len(),
        1,
        "only team A's member should remain"
    );
    assert_eq!(scoped_members[0].email, "a@warp.dev");
}
