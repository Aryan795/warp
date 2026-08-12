use super::{MemberUsageRow, SourceFilter};
use crate::auth::UserUid;
use crate::workspaces::team::MembershipRole;
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
    AiCreditsUsageSource, BillingCycleUsageEntry, WorkspaceMember, WorkspaceMemberUsageInfo,
};

const VIEWER_UID: &str = "viewer-uid";
const OTHER_UID: &str = "other-uid";
const ADMIN_UID: &str = "admin-uid";
const A_ONLY_UID: &str = "a-only-uid";
const B_ONLY_UID: &str = "b-only-uid";

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

fn workspace_member(uid: &str) -> WorkspaceMember {
    WorkspaceMember {
        uid: UserUid::new(uid),
        email: format!("{uid}@example.com"),
        role: MembershipRole::User,
        usage_info: WorkspaceMemberUsageInfo {
            is_unlimited: false,
            request_limit: 100,
            requests_used_since_last_refresh: 0,
            is_request_limit_prorated: false,
        },
    }
}

#[test]
fn for_each_member_yields_one_row_per_given_member_not_the_whole_workspace() {
    // Roster already scoped to team A: {admin, a-only}. `entries` mirrors
    // what the real pipeline would hand in after `scope_entries_to_team`,
    // i.e. b-only's usage has already been dropped upstream.
    let members = vec![workspace_member(ADMIN_UID), workspace_member(A_ONLY_UID)];
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(ADMIN_UID),
            AiCreditsUsageSource::Local,
            10,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(A_ONLY_UID),
            AiCreditsUsageSource::Local,
            5,
            0,
        ),
    ];

    let rows = MemberUsageRow::for_each_member(&entries, &members, SourceFilter::All);

    let subjects: Vec<_> = rows.iter().map(|r| r.subject_uid.clone()).collect();
    assert_eq!(rows.len(), 2, "expected exactly one row per scoped member");
    assert!(subjects.contains(&Some(ADMIN_UID.to_string())));
    assert!(subjects.contains(&Some(A_ONLY_UID.to_string())));
    assert!(
        !subjects.contains(&Some(B_ONLY_UID.to_string())),
        "b-only must not appear when scoped to team A's roster"
    );
}

#[test]
fn for_each_member_still_gives_zero_usage_members_a_row() {
    // a-only has no usage this cycle but must still render a zeroed row.
    let members = vec![workspace_member(ADMIN_UID), workspace_member(A_ONLY_UID)];
    let entries = vec![entry(
        AiCreditsUsageAndCostSubjectType::User,
        Some(ADMIN_UID),
        AiCreditsUsageSource::Local,
        10,
        0,
    )];

    let rows = MemberUsageRow::for_each_member(&entries, &members, SourceFilter::All);

    let a_only_row = rows
        .iter()
        .find(|r| r.subject_uid.as_deref() == Some(A_ONLY_UID))
        .expect("a-only should still render a zero-usage row");
    assert_eq!(a_only_row.total_credits, 0);
}
