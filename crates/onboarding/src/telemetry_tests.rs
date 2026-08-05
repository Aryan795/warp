use serde_json::json;
use warp_core::features::FeatureFlag;
use warp_core::telemetry::TelemetryEvent;

use super::{ACCOUNT_FIRST_FLOW_VERSION, OnboardingEvent};

#[test]
fn account_first_started_payload_includes_flow_metadata() {
    let _account_first_onboarding = FeatureFlag::AccountFirstOnboarding.override_enabled(true);

    assert_eq!(
        OnboardingEvent::OnboardingStarted.payload(),
        Some(json!({
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
            "entrypoint": "native_app",
        }))
    );
}

#[test]
fn account_first_lifecycle_payloads_include_flow_and_classification() {
    assert_eq!(
        OnboardingEvent::OnboardingAuthCompleted {
            account_class: "free_icp".to_string(),
            has_team: true,
            is_paid: false,
            team_discovery_outcome: "unknown".to_string(),
        }
        .payload(),
        Some(json!({
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
            "account_class": "free_icp",
            "has_team": true,
            "is_paid": false,
            "team_discovery_outcome": "unknown",
        }))
    );
    assert_eq!(
        OnboardingEvent::OnboardingUpgradeStarted {
            source_slide: "head_start".to_string(),
            account_class: "free_icp".to_string(),
            experiment_arm: None,
        }
        .payload(),
        Some(json!({
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
            "source_slide": "head_start",
            "account_class": "free_icp",
        }))
    );
    assert_eq!(
        OnboardingEvent::OnboardingUpgradeCompleted {
            source_slide: "head_start".to_string(),
            account_class: "free_icp".to_string(),
            experiment_arm: None,
        }
        .payload(),
        Some(json!({
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
            "source_slide": "head_start",
            "account_class": "free_icp",
        }))
    );
    assert_eq!(
        OnboardingEvent::OnboardingCompleted {
            completion_type: "upgrade_completed".to_string(),
            experiment_arm: None,
        }
        .payload(),
        Some(json!({
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
            "completion_type": "upgrade_completed",
        }))
    );
}

#[test]
fn offer_action_payload_includes_account_class() {
    assert_eq!(
        OnboardingEvent::OnboardingAction {
            slide_name: "head_start".to_string(),
            action: "get_more_ai".to_string(),
            account_class: Some("free_icp".to_string()),
            experiment_arm: None,
        }
        .payload(),
        Some(json!({
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
            "slide_name": "head_start",
            "action": "get_more_ai",
            "account_class": "free_icp",
        }))
    );
}

#[test]
fn account_first_slide_and_setting_payloads_include_flow_version() {
    let _account_first_onboarding = FeatureFlag::AccountFirstOnboarding.override_enabled(true);

    assert_eq!(
        OnboardingEvent::SlideViewed {
            slide_name: "customize".to_string(),
            experiment_arm: None,
        }
        .payload(),
        Some(json!({
            "slide_name": "customize",
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
        }))
    );
    assert_eq!(
        OnboardingEvent::SettingChanged {
            setting: "theme".to_string(),
            value: "Dark".to_string(),
        }
        .payload(),
        Some(json!({
            "setting": "theme",
            "value": "Dark",
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
        }))
    );
}

#[test]
fn stable_slide_payload_does_not_include_flow_version() {
    let _account_first_onboarding = FeatureFlag::AccountFirstOnboarding.override_enabled(false);

    assert_eq!(OnboardingEvent::OnboardingStarted.payload(), None);
    assert_eq!(
        OnboardingEvent::SlideViewed {
            slide_name: "intro".to_string(),
            experiment_arm: None,
        }
        .payload(),
        Some(json!({
            "slide_name": "intro",
        }))
    );
}

#[test]
fn onboarding_action_payload_omits_absent_account_class() {
    assert_eq!(
        OnboardingEvent::OnboardingAction {
            slide_name: "create_account".to_string(),
            action: "continue_signup".to_string(),
            account_class: None,
            experiment_arm: None,
        }
        .payload(),
        Some(json!({
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
            "slide_name": "create_account",
            "action": "continue_signup",
        }))
    );
}

/// REV-1939: the whole client funnel must be sliceable by the assigned arm, so
/// every event in it carries the same `experiment_arm` value.
#[test]
fn choose_how_to_start_funnel_payloads_carry_the_experiment_arm() {
    let _account_first_onboarding = FeatureFlag::AccountFirstOnboarding.override_enabled(true);

    assert_eq!(
        OnboardingEvent::SlideViewed {
            slide_name: "choose_how_to_start".to_string(),
            experiment_arm: Some("control".to_string()),
        }
        .payload(),
        Some(json!({
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
            "slide_name": "choose_how_to_start",
            "experiment_arm": "control",
        }))
    );
    assert_eq!(
        OnboardingEvent::OnboardingAction {
            slide_name: "choose_how_to_start".to_string(),
            action: "use_warp_with_ai".to_string(),
            account_class: Some("free_standard".to_string()),
            experiment_arm: Some("control".to_string()),
        }
        .payload(),
        Some(json!({
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
            "slide_name": "choose_how_to_start",
            "action": "use_warp_with_ai",
            "account_class": "free_standard",
            "experiment_arm": "control",
        }))
    );
    assert_eq!(
        OnboardingEvent::OnboardingUpgradeStarted {
            source_slide: "choose_how_to_start".to_string(),
            account_class: "free_standard".to_string(),
            experiment_arm: Some("experiment".to_string()),
        }
        .payload(),
        Some(json!({
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
            "source_slide": "choose_how_to_start",
            "account_class": "free_standard",
            "experiment_arm": "experiment",
        }))
    );
    assert_eq!(
        OnboardingEvent::OnboardingUpgradeCompleted {
            source_slide: "choose_how_to_start".to_string(),
            account_class: "free_standard".to_string(),
            experiment_arm: Some("experiment".to_string()),
        }
        .payload(),
        Some(json!({
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
            "source_slide": "choose_how_to_start",
            "account_class": "free_standard",
            "experiment_arm": "experiment",
        }))
    );
    assert_eq!(
        OnboardingEvent::OnboardingCompleted {
            completion_type: "free_standard_credits_purchased".to_string(),
            experiment_arm: Some("unassigned".to_string()),
        }
        .payload(),
        Some(json!({
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
            "completion_type": "free_standard_credits_purchased",
            "experiment_arm": "unassigned",
        }))
    );
}

#[test]
fn credit_purchase_payloads_carry_the_denomination_and_arm() {
    assert_eq!(
        OnboardingEvent::OnboardingCreditPurchaseStarted {
            source_slide: "choose_how_to_start".to_string(),
            account_class: "free_standard".to_string(),
            credits: 1_000,
            experiment_arm: Some("experiment".to_string()),
        }
        .payload(),
        Some(json!({
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
            "source_slide": "choose_how_to_start",
            "account_class": "free_standard",
            "credits": 1_000,
            "experiment_arm": "experiment",
        }))
    );
    assert_eq!(
        OnboardingEvent::OnboardingCreditPurchaseCompleted {
            source_slide: "choose_how_to_start".to_string(),
            account_class: "free_standard".to_string(),
            credits: 1_000,
            experiment_arm: Some("experiment".to_string()),
        }
        .payload(),
        Some(json!({
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
            "source_slide": "choose_how_to_start",
            "account_class": "free_standard",
            "credits": 1_000,
            "experiment_arm": "experiment",
        }))
    );
}

#[test]
fn credit_purchase_event_names_are_stable() {
    assert_eq!(
        OnboardingEvent::OnboardingCreditPurchaseStarted {
            source_slide: "choose_how_to_start".to_string(),
            account_class: "free_standard".to_string(),
            credits: 400,
            experiment_arm: None,
        }
        .name(),
        "onboarding_credit_purchase_started"
    );
    assert_eq!(
        OnboardingEvent::OnboardingCreditPurchaseCompleted {
            source_slide: "choose_how_to_start".to_string(),
            account_class: "free_standard".to_string(),
            credits: 400,
            experiment_arm: None,
        }
        .name(),
        "onboarding_credit_purchase_completed"
    );
}
