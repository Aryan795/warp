use chrono::Utc;
use warp_graphql::billing::{ServiceAgreement, ServiceAgreementStatus, ServiceAgreementType};
use warp_graphql::scalars::time::ServerTimestamp;

use super::*;
use crate::workspaces::workspace::BillingMetadata;

fn make_service_agreement(status: ServiceAgreementStatus) -> ServiceAgreement {
    ServiceAgreement {
        addon_credit_auto_reload_status: None,
        current_period_end: ServerTimestamp::new(Utc::now() + chrono::Duration::days(30)),
        status,
        stripe_subscription_id: None,
        type_: ServiceAgreementType::SelfServe,
        sunsetted_to_build_ts: None,
    }
}

fn solo_owner_team_with_billing(email: &str, billing_metadata: BillingMetadata) -> Team {
    Team {
        uid: 1_i64.into(),
        name: "Test Team".to_string(),
        invite_code: None,
        members: vec![TeamMember {
            uid: UserUid::new(email),
            email: email.to_string(),
            role: MembershipRole::Owner,
        }],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata,
        stripe_customer_id: None,
        organization_settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
    }
}

/// An active subscription must block deletion so users cannot lose billing data.
#[test]
fn test_get_delete_disabled_reason_active_subscription_blocks_delete() {
    let billing = BillingMetadata {
        service_agreements: vec![make_service_agreement(ServiceAgreementStatus::Active)],
        ..Default::default()
    };
    let team = solo_owner_team_with_billing("owner@example.com", billing);
    let reason = team.get_delete_disabled_reason("owner@example.com", 0);
    assert_eq!(
        reason,
        Some(TeamDeleteDisabledReason::ActivePaidSubscription),
    );
}

/// A cancelled subscription must NOT block deletion — users who cancelled their
/// plan should still be able to delete their team and join another one (REV-1795).
#[test]
fn test_get_delete_disabled_reason_cancelled_subscription_allows_delete() {
    let billing = BillingMetadata {
        service_agreements: vec![make_service_agreement(ServiceAgreementStatus::Canceled)],
        ..Default::default()
    };
    let team = solo_owner_team_with_billing("owner@example.com", billing);
    let reason = team.get_delete_disabled_reason("owner@example.com", 0);
    assert_eq!(reason, None);
}

/// When there are no service agreements on file, deletion should be permitted.
#[test]
fn test_get_delete_disabled_reason_no_service_agreements_allows_delete() {
    let billing = BillingMetadata::default();
    let team = solo_owner_team_with_billing("owner@example.com", billing);
    let reason = team.get_delete_disabled_reason("owner@example.com", 0);
    assert_eq!(reason, None);
}

/// Other team members must always block deletion, regardless of billing state.
#[test]
fn test_get_delete_disabled_reason_other_members_block_delete() {
    let team = Team {
        uid: 1_i64.into(),
        name: "Test Team".to_string(),
        invite_code: None,
        members: vec![
            TeamMember {
                uid: UserUid::new("owner@example.com"),
                email: "owner@example.com".to_string(),
                role: MembershipRole::Owner,
            },
            TeamMember {
                uid: UserUid::new("other@example.com"),
                email: "other@example.com".to_string(),
                role: MembershipRole::User,
            },
        ],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: BillingMetadata::default(),
        stripe_customer_id: None,
        organization_settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
    };
    let reason = team.get_delete_disabled_reason("owner@example.com", 0);
    assert_eq!(reason, Some(TeamDeleteDisabledReason::OtherMembers));
}

/// Remaining bonus credits must block deletion regardless of subscription state.
#[test]
fn test_get_delete_disabled_reason_remaining_credits_block_delete() {
    let billing = BillingMetadata::default();
    let team = solo_owner_team_with_billing("owner@example.com", billing);
    let reason = team.get_delete_disabled_reason("owner@example.com", 100);
    assert_eq!(
        reason,
        Some(TeamDeleteDisabledReason::RemainingBonusCredits),
    );
}
