use std::collections::HashMap;

use warp_core::features::FeatureFlag;

use super::*;
use crate::persistence::model::{FULL_TERMINAL_USE_CATEGORY, PRIMARY_AGENT_CATEGORY};

fn model(id: &str, warp_tokens: u32, category: &str) -> ModelTokenUsage {
    ModelTokenUsage {
        model_id: id.to_string(),
        warp_tokens,
        warp_token_usage_by_category: HashMap::from([(category.to_string(), warp_tokens)]),
        ..Default::default()
    }
}

#[test]
fn model_usage_rows_drops_zero_token_models() {
    let models = vec![
        model("gpt-5.5", 100, PRIMARY_AGENT_CATEGORY),
        ModelTokenUsage {
            model_id: "unused-model".to_string(),
            ..Default::default()
        },
    ];
    let rows = model_usage_rows(&models, &HashMap::new());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].model_id, "gpt-5.5");
}

#[test]
fn model_usage_rows_sorts_primary_agent_first() {
    let models = vec![
        model("codex-model", 50, FULL_TERMINAL_USE_CATEGORY),
        model("primary-model", 100, PRIMARY_AGENT_CATEGORY),
        model("auto-model", 10, "other_category"),
    ];
    let rows = model_usage_rows(&models, &HashMap::new());
    assert_eq!(rows[0].model_id, "primary-model");
    assert_eq!(rows[0].role_badge, Some("Primary agent"));
}

#[test]
fn model_usage_rows_assigns_full_terminal_use_badge() {
    let models = vec![model("codex-model", 50, FULL_TERMINAL_USE_CATEGORY)];
    let rows = model_usage_rows(&models, &HashMap::new());
    assert_eq!(rows[0].role_badge, Some("Full terminal use"));
}

#[test]
fn model_usage_rows_has_no_badge_for_unknown_categories() {
    let models = vec![model("auto-model", 10, "some_other_category")];
    let rows = model_usage_rows(&models, &HashMap::new());
    assert_eq!(rows[0].role_badge, None);
}

fn charged_usage_with_input_cost(cost_in_cents: f32) -> PersistedModelTokenCost {
    PersistedModelTokenCost {
        input_cost_in_cents: cost_in_cents,
        ..Default::default()
    }
}

#[test]
fn model_usage_rows_joins_charged_usage_by_model_id() {
    let models = vec![
        model("gpt-5.5", 100, PRIMARY_AGENT_CATEGORY),
        model("codex-model", 50, FULL_TERMINAL_USE_CATEGORY),
    ];
    let charged_usage_by_model =
        HashMap::from([("gpt-5.5".to_string(), charged_usage_with_input_cost(36.0))]);
    let rows = model_usage_rows(&models, &charged_usage_by_model);
    let gpt_row = rows.iter().find(|r| r.model_id == "gpt-5.5").unwrap();
    let codex_row = rows.iter().find(|r| r.model_id == "codex-model").unwrap();
    assert_eq!(gpt_row.cost_in_cents, Some(36.0));
    assert!(gpt_row.charged_usage.is_some());
    assert_eq!(codex_row.cost_in_cents, None);
    assert!(codex_row.charged_usage.is_none());
}

#[test]
fn format_token_count_abbreviates_above_1000() {
    assert_eq!(format_token_count(500), "500");
    assert_eq!(format_token_count(9600), "9.6k");
    assert_eq!(format_token_count(1000), "1.0k");
}

#[test]
fn format_token_count_abbreviates_above_1_000_000_as_m() {
    assert_eq!(format_token_count(999_999), "1000.0k");
    assert_eq!(format_token_count(1_000_000), "1.0M");
    assert_eq!(format_token_count(1_614_700), "1.6M");
}

#[test]
fn exact_token_count_tooltip_is_none_below_abbreviation_threshold() {
    assert_eq!(exact_token_count_tooltip(500), None);
    assert_eq!(exact_token_count_tooltip(999), None);
}

#[test]
fn exact_token_count_tooltip_shows_comma_separated_count_when_abbreviated() {
    assert_eq!(
        exact_token_count_tooltip(9614),
        Some("9,614 tokens".to_string())
    );
    assert_eq!(
        exact_token_count_tooltip(1_614_700),
        Some("1,614,700 tokens".to_string())
    );
}

#[test]
fn format_tokens_and_cost_omits_dollar_suffix_when_flag_disabled() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(false);

    assert_eq!(
        format_tokens_and_cost(Some(9600), Some(36.0)),
        "9.6k tokens"
    );
}

#[test]
fn format_tokens_and_cost_joins_tokens_and_dollar_with_a_slash_when_flag_enabled() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_tokens_and_cost(Some(9600), Some(36.0)),
        "9.6k tokens / $0.36"
    );
}

#[test]
fn format_tokens_and_cost_omits_dollar_suffix_when_cost_is_unknown() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(format_tokens_and_cost(Some(9600), None), "9.6k tokens");
}

#[test]
fn format_tokens_and_cost_falls_back_to_cost_only_when_tokens_are_unknown() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(format_tokens_and_cost(None, Some(36.0)), "$0.36");
}

#[test]
fn format_tokens_and_cost_shows_em_dash_when_both_are_unknown() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(format_tokens_and_cost(None, None), "\u{2014}");
}

#[test]
fn format_count_and_cost_omits_dollar_suffix_when_flag_disabled() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(false);

    assert_eq!(format_count_and_cost(3, "searches", 2.0), "3 searches");
}

#[test]
fn format_count_and_cost_appends_dollar_suffix_when_flag_enabled() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_count_and_cost(3, "searches", 2.0),
        "3 searches / $0.02"
    );
}

#[test]
fn format_cost_only_shows_em_dash_when_flag_disabled() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(false);

    assert_eq!(format_cost_only(Some(36.0)), "\u{2014}");
}

#[test]
fn format_cost_only_formats_dollars_when_flag_enabled() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(format_cost_only(Some(36.0)), "$0.36");
    assert_eq!(format_cost_only(None), "\u{2014}");
}

#[test]
fn render_model_usage_rows_section_is_empty_when_no_models_have_usage() {
    let appearance = Appearance::mock();
    let element = render_model_usage_rows_section(
        &[],
        &HashMap::new(),
        (None, None),
        &|_| true,
        None,
        None,
        &appearance,
    );
    assert_eq!(element.debug_text_content().unwrap_or_default(), "");
}

#[test]
fn render_settings_history_content_includes_model_and_omits_zero_platform_usage() {
    let appearance = Appearance::mock();
    let usage = ConversationUsageMetadata {
        token_usage: vec![model("gpt-5.5", 100, PRIMARY_AGENT_CATEGORY)],
        total_charged_usage: Some(persistence::model::ChargedUsageTotals::default()),
        ..Default::default()
    };
    let element = render_settings_history_content(&usage, &appearance);
    let text = element.debug_text_content().unwrap_or_default();
    assert!(
        text.contains("gpt-5.5"),
        "should render the model row: {text}"
    );
    assert!(
        !text.contains("PLATFORM USAGE"),
        "zero platform cost should be omitted: {text}"
    );
}
