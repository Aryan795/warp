use warp_core::features::FeatureFlag;

use super::*;
use crate::settings::UsageDisplayUnit;

#[test]
fn format_credits_with_cost_returns_credits_only_when_flag_disabled() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(false);

    assert_eq!(
        format_credits_with_cost(20.0, Some(12345), Some(36.0), UsageDisplayUnit::Dollars),
        format_credits(20.0)
    );
}

#[test]
fn format_credits_with_cost_uses_credits_unit() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_credits_with_cost(20.0, Some(12345), Some(36.0), UsageDisplayUnit::Credits),
        "12,345 tokens / 20 credits"
    );
}

#[test]
fn format_credits_with_cost_uses_dollars_unit() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_credits_with_cost(20.0, Some(12345), Some(36.0), UsageDisplayUnit::Dollars),
        "12,345 tokens / $0.36"
    );
}

#[test]
fn format_credits_with_cost_formats_large_token_counts_with_thousands_separators() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_credits_with_cost(26.9, Some(719_124), Some(48.0), UsageDisplayUnit::Dollars),
        "719,124 tokens / $0.48"
    );
}

#[test]
fn format_credits_with_cost_falls_back_to_credits_when_dollars_unavailable() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_credits_with_cost(20.0, Some(12345), None, UsageDisplayUnit::Dollars),
        format_credits(20.0)
    );
}

#[test]
fn format_credits_with_cost_omits_tokens_when_tokens_is_unknown() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_credits_with_cost(20.0, None, Some(36.0), UsageDisplayUnit::Dollars),
        "$0.36"
    );
    assert_eq!(
        format_credits_with_cost(20.0, None, Some(36.0), UsageDisplayUnit::Credits),
        format_credits(20.0)
    );
}

#[test]
fn format_credits_with_cost_omits_tokens_when_tokens_is_zero() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_credits_with_cost(20.0, Some(0), Some(36.0), UsageDisplayUnit::Dollars),
        "$0.36"
    );
}

#[test]
fn format_credits_with_cost_credits_unit_omits_tokens_when_tokens_is_zero() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_credits_with_cost(20.0, Some(0), None, UsageDisplayUnit::Credits),
        format_credits(20.0)
    );
}
