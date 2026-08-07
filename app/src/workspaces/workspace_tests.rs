use super::*;
use crate::server::ids::ServerId;

// `ServerId::from_string_lossy` requires exactly 22 characters.
const TEST_WORKSPACE_UID: &str = "workspace_uid123456789";

#[test]
fn ftue_account_classes_have_stable_telemetry_labels() {
    assert_eq!(FtueAccountClass::Paid.as_str(), "paid");
    assert_eq!(FtueAccountClass::FreeIcp.as_str(), "free_icp");
    assert_eq!(FtueAccountClass::FreeStandard.as_str(), "free_standard");
}
fn make_workspace(policy: Option<UsageVisibilityPolicy>) -> Workspace {
    let mut workspace = Workspace::from_local_cache(
        ServerId::from_string_lossy(TEST_WORKSPACE_UID).into(),
        "Test Workspace".to_string(),
        None,
    );
    workspace.billing_metadata.tier.usage_visibility_policy = policy;
    workspace
}

fn policy(
    granularity: UsageVisibilityGranularity,
    max_prior_cycles: MaxPriorCycles,
) -> UsageVisibilityPolicy {
    UsageVisibilityPolicy {
        admin_granularity: granularity,
        max_prior_cycles,
    }
}

#[test]
fn missing_policy_returns_defaults_for_admin_and_non_admin() {
    let workspace = make_workspace(None);

    let as_admin = workspace.resolve_usage_visibility(true);
    assert_eq!(as_admin.granularity, UsageVisibilityGranularity::OwnOnly);
    assert_eq!(as_admin.max_prior_cycles, MaxPriorCycles::None);

    let as_non_admin = workspace.resolve_usage_visibility(false);
    assert_eq!(
        as_non_admin.granularity,
        UsageVisibilityGranularity::OwnOnly
    );
    assert_eq!(as_non_admin.max_prior_cycles, MaxPriorCycles::None);
}

#[test]
fn non_admin_collapses_granularity_but_keeps_max_prior_cycles() {
    let workspace = make_workspace(Some(policy(
        UsageVisibilityGranularity::FullBreakdown,
        MaxPriorCycles::Limited(11),
    )));

    let resolved = workspace.resolve_usage_visibility(false);

    assert_eq!(resolved.granularity, UsageVisibilityGranularity::OwnOnly);
    assert_eq!(resolved.max_prior_cycles, MaxPriorCycles::Limited(11));
}

#[test]
fn admin_inherits_tier_team_aggregate_granularity() {
    let workspace = make_workspace(Some(policy(
        UsageVisibilityGranularity::TeamAggregate,
        MaxPriorCycles::Limited(11),
    )));

    let resolved = workspace.resolve_usage_visibility(true);

    assert_eq!(
        resolved.granularity,
        UsageVisibilityGranularity::TeamAggregate
    );
    assert_eq!(resolved.max_prior_cycles, MaxPriorCycles::Limited(11));
}

#[test]
fn admin_inherits_tier_per_user_totals_unlimited() {
    let workspace = make_workspace(Some(policy(
        UsageVisibilityGranularity::PerUserTotals,
        MaxPriorCycles::Unlimited,
    )));

    let resolved = workspace.resolve_usage_visibility(true);

    assert_eq!(
        resolved.granularity,
        UsageVisibilityGranularity::PerUserTotals
    );
    assert_eq!(resolved.max_prior_cycles, MaxPriorCycles::Unlimited);
}

#[test]
fn admin_inherits_tier_full_breakdown_unlimited() {
    let workspace = make_workspace(Some(policy(
        UsageVisibilityGranularity::FullBreakdown,
        MaxPriorCycles::Unlimited,
    )));

    let resolved = workspace.resolve_usage_visibility(true);

    assert_eq!(
        resolved.granularity,
        UsageVisibilityGranularity::FullBreakdown
    );
    assert_eq!(resolved.max_prior_cycles, MaxPriorCycles::Unlimited);
}

fn billing_metadata_with_purchase_policy(
    purchase_policy: Option<PurchaseAddOnCreditsPolicy>,
) -> BillingMetadata {
    let mut billing_metadata = BillingMetadata::default();
    billing_metadata.tier.purchase_add_on_credits_policy = purchase_policy;
    billing_metadata
}

#[test]
fn purchase_policy_disabled_without_policy() {
    let billing_metadata = billing_metadata_with_purchase_policy(None);

    assert!(!billing_metadata.is_purchase_add_on_credits_policy_enabled());
    assert!(!billing_metadata.is_premium_addon_credits_purchase());
    assert_eq!(billing_metadata.addon_credits_price_premium_bps(), 0);
}

#[test]
fn purchase_policy_standard_plan_has_no_premium() {
    let billing_metadata =
        billing_metadata_with_purchase_policy(Some(PurchaseAddOnCreditsPolicy {
            enabled: true,
            premium_enabled: false,
            price_premium_bps: 0,
        }));

    assert!(billing_metadata.is_purchase_add_on_credits_policy_enabled());
    assert!(!billing_metadata.is_premium_addon_credits_purchase());
    assert_eq!(billing_metadata.addon_credits_price_premium_bps(), 0);
}

#[test]
fn purchase_policy_premium_plan_enables_surcharged_purchasing() {
    let billing_metadata =
        billing_metadata_with_purchase_policy(Some(PurchaseAddOnCreditsPolicy {
            enabled: false,
            premium_enabled: true,
            price_premium_bps: 1000,
        }));

    assert!(billing_metadata.is_purchase_add_on_credits_policy_enabled());
    assert!(billing_metadata.is_premium_addon_credits_purchase());
    assert_eq!(billing_metadata.addon_credits_price_premium_bps(), 1000);
}

#[test]
fn purchase_policy_fully_disabled_plan_remains_disabled() {
    let billing_metadata =
        billing_metadata_with_purchase_policy(Some(PurchaseAddOnCreditsPolicy {
            enabled: false,
            premium_enabled: false,
            price_premium_bps: 1000,
        }));

    assert!(!billing_metadata.is_purchase_add_on_credits_policy_enabled());
    assert!(!billing_metadata.is_premium_addon_credits_purchase());
    assert_eq!(billing_metadata.addon_credits_price_premium_bps(), 0);
}

#[test]
fn purchase_policy_standard_purchasing_wins_over_premium() {
    // Standard (list price) purchasing takes precedence if the server ever
    // sends both flags; no surcharge should be displayed or applied.
    let billing_metadata =
        billing_metadata_with_purchase_policy(Some(PurchaseAddOnCreditsPolicy {
            enabled: true,
            premium_enabled: true,
            price_premium_bps: 1000,
        }));

    assert!(billing_metadata.is_purchase_add_on_credits_policy_enabled());
    assert!(!billing_metadata.is_premium_addon_credits_purchase());
    assert_eq!(billing_metadata.addon_credits_price_premium_bps(), 0);
}

#[test]
fn workspace_defaults_marks_scalar_settings_as_workspace_enforced() {
    let mut ws = WorkspaceSettings::default();
    ws.secret_redaction_settings.enabled = true;
    ws.ai_permissions_settings.allow_ai_in_remote_sessions = false;
    ws.link_sharing_settings.anyone_with_link_sharing_enabled = true;
    ws.codebase_context_settings.setting = AdminEnablementSetting::Enable;

    let team_settings = TeamSettings::from_workspace_defaults(&ws);

    assert!(team_settings.secret_redaction.enabled.value);
    assert!(
        team_settings
            .secret_redaction
            .enabled
            .is_enforced_by_workspace
    );
    assert!(
        !team_settings
            .ai_permissions
            .allow_ai_in_remote_sessions
            .value
    );
    assert!(
        team_settings
            .ai_permissions
            .allow_ai_in_remote_sessions
            .is_enforced_by_workspace
    );
    assert!(
        team_settings
            .link_sharing
            .anyone_with_link_sharing_enabled
            .value
    );
    assert_eq!(
        team_settings.codebase_context.value,
        AdminEnablementSetting::Enable
    );
    assert!(team_settings.codebase_context.is_enforced_by_workspace);
}

#[test]
fn workspace_defaults_attributes_list_entries_to_the_workspace_layer() {
    let mut ws = WorkspaceSettings::default();
    ws.secret_redaction_settings.regexes = vec![EnterpriseSecretRegex {
        pattern: "sk-[a-z0-9]+".to_string(),
        name: Some("api key".to_string()),
    }];
    ws.ai_autonomy_settings.read_files_allowlist = Some(vec![PathBuf::from("/tmp/allowed")]);
    ws.ai_autonomy_settings.execute_commands_denylist = Some(vec![
        AgentModeCommandExecutionPredicate::new_regex("rm -rf.*").unwrap(),
    ]);

    let team_settings = TeamSettings::from_workspace_defaults(&ws);

    let regexes = &team_settings.secret_redaction.regexes;
    assert_eq!(regexes.values, ws.secret_redaction_settings.regexes);
    assert_eq!(
        regexes.workspace_entries,
        ws.secret_redaction_settings.regexes
    );
    assert!(regexes.team_entries.is_empty());

    let allowlist = &team_settings.ai_autonomy.read_files_allowlist;
    assert_eq!(allowlist.values, vec!["/tmp/allowed".to_string()]);
    assert!(allowlist.is_configured);
    assert_eq!(
        allowlist.workspace_entries,
        vec!["/tmp/allowed".to_string()]
    );
    assert!(allowlist.team_entries.is_empty());

    let denylist = &team_settings.ai_autonomy.execute_commands_denylist;
    assert_eq!(denylist.values, vec!["rm -rf.*".to_string()]);
    assert!(denylist.is_configured);
    assert!(denylist.team_entries.is_empty());
}

#[test]
fn workspace_defaults_distinguishes_unconfigured_from_explicit_empty_override() {
    // Regression coverage for the fallback path: `Option<Vec<T>>::is_some()` on
    // the underlying `WorkspaceSettings` field must drive `is_configured`, not
    // the resulting list's emptiness -- otherwise an admin's explicit empty
    // override ("auto-allow nothing") would be indistinguishable from the
    // field never having been configured at all.
    let mut ws = WorkspaceSettings::default();
    // Never configured: `None`.
    ws.ai_autonomy_settings.read_files_allowlist = None;
    // Explicitly configured to empty: `Some(vec![])`.
    ws.ai_autonomy_settings.execute_commands_allowlist = Some(vec![]);

    let team_settings = TeamSettings::from_workspace_defaults(&ws);

    assert!(!team_settings.ai_autonomy.read_files_allowlist.is_configured);
    assert!(
        team_settings
            .ai_autonomy
            .execute_commands_allowlist
            .is_configured
    );
    assert!(
        team_settings
            .ai_autonomy
            .execute_commands_allowlist
            .values
            .is_empty()
    );

    // The distinction must survive all the way through to the shape the
    // permission-checking code actually reads.
    let autonomy = AiAutonomySettings::from(&team_settings.ai_autonomy);
    assert_eq!(autonomy.read_files_allowlist, None);
    assert_eq!(autonomy.execute_commands_allowlist, Some(vec![]));
}

#[test]
fn workspace_defaults_has_no_enforced_value_for_create_plans() {
    // `WorkspaceSettings`/`AiAutonomySettings` has no `create_plans_setting` field
    // (unlike the team-level `TeamAiAutonomySettings`), so the fallback can't claim
    // the workspace enforces a value for it.
    let ws = WorkspaceSettings::default();

    let team_settings = TeamSettings::from_workspace_defaults(&ws);

    assert_eq!(team_settings.ai_autonomy.create_plans.value, None);
    assert!(
        !team_settings
            .ai_autonomy
            .create_plans
            .is_enforced_by_workspace
    );
}

#[test]
fn workspace_defaults_treats_absent_sandboxed_agent_settings_as_empty() {
    // `sandboxed_agent_settings` is already `None` via `Default`; this test
    // documents that the fallback path handles that case explicitly.
    let ws = WorkspaceSettings::default();

    let team_settings = TeamSettings::from_workspace_defaults(&ws);

    assert!(
        team_settings
            .sandboxed_agent
            .execute_commands_denylist
            .values
            .is_empty()
    );
}

#[test]
fn workspace_defaults_passes_through_unwrapped_fields_verbatim() {
    let ws = WorkspaceSettings {
        default_host_slug: Some("my-host".to_string()),
        enable_warp_attribution: AdminEnablementSetting::Disable,
        ..Default::default()
    };

    let team_settings = TeamSettings::from_workspace_defaults(&ws);

    assert_eq!(team_settings.default_host_slug.as_deref(), Some("my-host"));
    assert_eq!(
        team_settings.enable_warp_attribution,
        AdminEnablementSetting::Disable
    );
}
