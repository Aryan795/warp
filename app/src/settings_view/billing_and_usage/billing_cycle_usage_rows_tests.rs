use super::{MemberUsageRow, SourceFilter, team_scoped_members};
use crate::auth::UserUid;
use crate::workspaces::team::{MembershipRole, TeamMember};
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
    AiCreditsUsageSource, BillingCycleUsageEntry, WorkspaceMember, WorkspaceMemberUsageInfo,
};

const VIEWER_UID: &str = "viewer-uid";
const OTHER_UID: &str = "other-uid";

fn entry(
    subject_type: AiCreditsUsageAndCostSubjectType,
    subject_uid: Option<&str>,
    usage_source: AiCreditsUsageSource,
    credits_used: i32,
    cost_cents: i32,
) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        subject_type,
        subject_uid: subject_uid.map(|s| s.to_string()),
        subject_display_name: None,
        cost_type: AiCreditsUsageAndCostType::BaseLimit,
        usage_bucket: AiCreditsUsageBucket::Ai,
        usage_source,
        credits_used,
        cost_cents,
        attributed_team_uid: None,
    }
}

fn team_scoped_entry(
    subject_uid: &str,
    credits_used: i32,
    attributed_team_uid: &str,
) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        subject_type: AiCreditsUsageAndCostSubjectType::User,
        subject_uid: Some(subject_uid.to_string()),
        subject_display_name: None,
        cost_type: AiCreditsUsageAndCostType::BaseLimit,
        usage_bucket: AiCreditsUsageBucket::Ai,
        usage_source: AiCreditsUsageSource::Local,
        credits_used,
        cost_cents: 0,
        attributed_team_uid: Some(attributed_team_uid.to_string()),
    }
}

fn team_member(uid: &str) -> TeamMember {
    TeamMember {
        uid: UserUid::new(uid),
        email: format!("{uid}@example.com"),
        role: MembershipRole::User,
    }
}

fn workspace_member(uid: &str) -> WorkspaceMember {
    WorkspaceMember {
        uid: UserUid::new(uid),
        email: format!("{uid}@example.com"),
        role: MembershipRole::User,
        usage_info: WorkspaceMemberUsageInfo {
            is_unlimited: false,
            request_limit: 1000,
            requests_used_since_last_refresh: 0,
            is_request_limit_prorated: false,
        },
    }
}

#[test]
fn build_own_usage_row_drops_team_subject_entries() {
    // Team-aggregate rows belong to "everyone else" by construction; they
    // must never contribute to the viewer's own row totals.
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageSource::Local,
            10,
            5,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::Team,
            None,
            AiCreditsUsageSource::Aggregate,
            999,
            999,
        ),
    ];
    let row = MemberUsageRow::for_viewer(
        &entries,
        Some(VIEWER_UID),
        "viewer".to_string(),
        SourceFilter::All,
    );
    assert_eq!(row.total_credits, 10);
    assert_eq!(row.total_cost_cents, 5);
}

#[test]
fn build_own_usage_row_drops_other_users_entries() {
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageSource::Local,
            10,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(OTHER_UID),
            AiCreditsUsageSource::Local,
            999,
            999,
        ),
    ];
    let row = MemberUsageRow::for_viewer(
        &entries,
        Some(VIEWER_UID),
        "viewer".to_string(),
        SourceFilter::All,
    );
    assert_eq!(row.total_credits, 10);
    assert_eq!(row.total_cost_cents, 0);
}

#[test]
fn build_own_usage_row_local_filter_drops_cloud_entries() {
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageSource::Local,
            10,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageSource::Cloud,
            20,
            0,
        ),
    ];
    let row = MemberUsageRow::for_viewer(
        &entries,
        Some(VIEWER_UID),
        "viewer".to_string(),
        SourceFilter::Local,
    );
    assert_eq!(row.total_credits, 10);
}

#[test]
fn build_own_usage_row_cloud_filter_drops_local_entries() {
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageSource::Local,
            10,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageSource::Cloud,
            20,
            0,
        ),
    ];
    let row = MemberUsageRow::for_viewer(
        &entries,
        Some(VIEWER_UID),
        "viewer".to_string(),
        SourceFilter::Cloud,
    );
    assert_eq!(row.total_credits, 20);
}

#[test]
fn team_scoped_members_keeps_only_team_roster_members() {
    let workspace_members = [
        workspace_member("a-only"),
        workspace_member("b-only"),
        workspace_member("shared"),
    ];
    let team_a_members = [team_member("a-only"), team_member("shared")];

    let scoped = team_scoped_members(&workspace_members, &team_a_members);

    let uids: Vec<&str> = scoped.iter().map(|m| m.uid.as_str()).collect();
    assert_eq!(uids, ["a-only", "shared"]);
}

#[test]
fn team_scoped_members_empty_team_roster_yields_no_members() {
    let workspace_members = [workspace_member("a-only")];

    let scoped = team_scoped_members(&workspace_members, &[]);

    assert!(scoped.is_empty());
}

#[test]
fn build_rows_for_team_excludes_other_teams_and_preserves_zero_usage_members() {
    // Team A's roster: a-only and shared are members; b-only and the
    // service account are not, so they must never surface as rows even
    // though their entries appear in the raw (unfiltered) entries list.
    let team_a_members = team_scoped_members(
        &[
            workspace_member("a-only"),
            workspace_member("a-zero-usage"),
            workspace_member("b-only"),
            workspace_member("shared"),
        ],
        &[
            team_member("a-only"),
            team_member("a-zero-usage"),
            team_member("shared"),
        ],
    );

    // Mirrors the attributed-team filtering the caller applies to entries
    // before scoping the roster; without it, `b-only`'s entry and the
    // service account's entry would leak in below as extra "leftover"
    // rows, since `for_each_member` renders a row for any subject not in
    // the given member list -- roster scoping alone isn't sufficient.
    let team_a_entries: Vec<_> = [
        team_scoped_entry("a-only", 10, "team-a"),
        team_scoped_entry("shared", 5, "team-a"),
        team_scoped_entry("b-only", 999, "team-b"),
        team_scoped_entry("service-account-b", 888, "team-b"),
    ]
    .into_iter()
    .filter(|entry| entry.attributed_team_uid.as_deref() == Some("team-a"))
    .collect();

    let rows = MemberUsageRow::for_each_member(&team_a_entries, &team_a_members, SourceFilter::All);

    let mut subject_uids: Vec<Option<&str>> =
        rows.iter().map(|r| r.subject_uid.as_deref()).collect();
    subject_uids.sort();
    assert_eq!(
        subject_uids,
        [Some("a-only"), Some("a-zero-usage"), Some("shared")]
    );

    let zero_usage_row = rows
        .iter()
        .find(|r| r.subject_uid.as_deref() == Some("a-zero-usage"))
        .expect("zero-usage A member should still get a row");
    assert_eq!(zero_usage_row.total_credits, 0);
}
