use warp::appearance::Appearance;
use warp::settings::TuiUsageDisplayMode;
use warp::tui_export::{ConversationUsageTotals, InferenceCostBreakdown};
use warp_core::features::FeatureFlag;
use warpui_core::App;
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;

use super::*;

fn totals(credits_spent: f32, cost_in_cents: f32) -> ConversationUsageTotals {
    ConversationUsageTotals {
        credits_spent,
        cost_in_cents: Some(cost_in_cents),
        has_usage: true,
        total_tokens: 0,
        inference_cost_breakdown: None,
    }
}

#[test]
fn cost_formats_cents_as_dollars() {
    assert_eq!(format_cost(0.0), "$0.00");
    assert_eq!(format_cost(0.4), "$0.00");
    assert_eq!(format_cost(3.2), "$0.03");
    assert_eq!(format_cost(123.0), "$1.23");
    assert_eq!(format_cost(10_000.0), "$100.00");
}

#[test]
fn entry_text_matches_the_gui_credits_formatting() {
    // `format_credits` is the GUI's formatter: whole values pluralize and
    // drop the decimal, fractional values keep one decimal place.
    let mode = TuiUsageDisplayMode::default();
    assert_eq!(entry_text(mode, totals(1.0, 0.0)), "1 credit");
    assert_eq!(entry_text(mode, totals(2.0, 0.0)), "2 credits");
    assert_eq!(entry_text(mode, totals(2.5, 0.0)), "2.5 credits");
}

#[test]
fn entry_text_follows_the_persisted_display_mode() {
    let usage = totals(2.5, 3.2);
    // Credits is the default mode; a click cycles credits -> cost -> detail
    // -> back to credits.
    let credits = TuiUsageDisplayMode::default();
    assert_eq!(entry_text(credits, usage), "2.5 credits");
    assert_eq!(entry_text(credits.toggled(), usage), "$0.03");
    assert_eq!(entry_text(credits.toggled().toggled(), usage), "$0.03");
    assert_eq!(
        entry_text(credits.toggled().toggled().toggled(), usage),
        "2.5 credits"
    );
}

#[test]
fn cost_mode_explicitly_marks_unknown_historical_cost() {
    assert_eq!(
        entry_text(
            TuiUsageDisplayMode::Cost,
            ConversationUsageTotals {
                credits_spent: 0.0,
                cost_in_cents: None,
                has_usage: true,
                total_tokens: 0,
                inference_cost_breakdown: None,
            },
        ),
        "Cost unavailable"
    );
}

#[test]
fn detail_mode_condenses_tokens_and_breakdown_onto_one_line() {
    let usage = ConversationUsageTotals {
        credits_spent: 2.5,
        cost_in_cents: Some(3.2),
        has_usage: true,
        total_tokens: 1234,
        inference_cost_breakdown: Some(InferenceCostBreakdown {
            input_cost_in_cents: 1.0,
            input_cache_read_cost_in_cents: 0.5,
            input_cache_write_cost_in_cents: 0.5,
            output_cost_in_cents: 1.2,
        }),
    };
    let text = entry_text(TuiUsageDisplayMode::Detail, usage);
    assert!(text.starts_with("$0.03"), "got: {text:?}");
    assert!(text.contains("1234 tok"), "got: {text:?}");
    assert!(text.contains("in $0.01"), "got: {text:?}");
    assert!(text.contains("cr $0.00"), "got: {text:?}");
    assert!(text.contains("cw $0.00"), "got: {text:?}");
    assert!(text.contains("out $0.01"), "got: {text:?}");
}

#[test]
fn detail_mode_falls_back_gracefully_when_cost_is_unavailable() {
    let usage = ConversationUsageTotals {
        credits_spent: 0.0,
        cost_in_cents: None,
        has_usage: true,
        total_tokens: 100,
        inference_cost_breakdown: None,
    };
    assert_eq!(
        entry_text(TuiUsageDisplayMode::Detail, usage),
        "Cost unavailable"
    );
}

#[test]
fn toggle_cycles_through_credits_cost_and_detail() {
    let mode = TuiUsageDisplayMode::default();
    assert_eq!(mode, TuiUsageDisplayMode::Credits);
    assert_eq!(mode.toggled(), TuiUsageDisplayMode::Cost);
    assert_eq!(mode.toggled().toggled(), TuiUsageDisplayMode::Detail);
    assert_eq!(
        mode.toggled().toggled().toggled(),
        TuiUsageDisplayMode::Credits
    );
}

/// The credits⇄dollars toggle is gated behind `PricingTransparency`: with the
/// flag off (prod/stable) the footer renders a static credits total and never
/// exposes the dollar cost even when the persisted mode is `Cost`; with the
/// flag on (dogfood/staging + local/dev) the entry follows the persisted mode.
#[test]
fn footer_usage_entry_gates_the_cost_toggle_behind_the_feature_flag() {
    App::test((), |mut app| async move {
        // `render_entry` resolves theme styles via `TuiUiBuilder`, which reads
        // the `Appearance` singleton.
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
        });
        let toggle = UsageToggle::default();
        let usage = totals(2.5, 3.2);

        // Flag OFF: static credits; the persisted `Cost` mode is ignored so the
        // dollar cost is never surfaced.
        app.read(|ctx| {
            let _guard = FeatureFlag::PricingTransparency.override_enabled(false);
            let entry = toggle.render_entry(TuiUsageDisplayMode::Cost, usage, ctx, |_, _| {});
            let line = TuiPresenter::new()
                .present_element(entry, TuiRect::new(0, 0, 20, 1), ctx)
                .buffer
                .to_lines()
                .join("");
            assert!(
                line.contains("2.5 credits"),
                "flag off must show static credits, got: {line:?}"
            );
            assert!(
                !line.contains("$0.03"),
                "flag off must not expose the dollar cost, got: {line:?}"
            );
        });

        // Flag ON: the entry follows the persisted display mode, exposing the
        // dollar cost the toggle switches to.
        app.read(|ctx| {
            let _guard = FeatureFlag::PricingTransparency.override_enabled(true);
            let entry = toggle.render_entry(TuiUsageDisplayMode::Cost, usage, ctx, |_, _| {});
            let line = TuiPresenter::new()
                .present_element(entry, TuiRect::new(0, 0, 20, 1), ctx)
                .buffer
                .to_lines()
                .join("");
            assert!(
                line.contains("$0.03"),
                "flag on must follow the persisted cost mode, got: {line:?}"
            );
        });
    });
}
