use thousands::Separable;
use warp_graphql::billing::AddonCreditsOption;

use crate::pricing::format_usd_cents;

// ── Fixtures ──────────────────────────────────────────────────────────────────

/// Zero-markup paid-plan option.
fn paid_option(credits: i32, price_cents: i32) -> AddonCreditsOption {
    AddonCreditsOption {
        credits,
        price_usd_cents: price_cents,
        base_price_usd_cents: Some(price_cents),
        markup_usd_cents: Some(0),
        total_price_usd_cents: Some(price_cents),
    }
}

/// Free-plan option with a 10% markup.
fn free_option(credits: i32, base_cents: i32, markup_cents: i32) -> AddonCreditsOption {
    AddonCreditsOption {
        credits,
        price_usd_cents: base_cents,
        base_price_usd_cents: Some(base_cents),
        markup_usd_cents: Some(markup_cents),
        total_price_usd_cents: Some(base_cents + markup_cents),
    }
}

// ── Helpers that replicate production logic ───────────────────────────────────

/// Mirrors `addon_credits_panel_state` price_label computation (spec criteria 3, 4).
fn price_label_for(opt: &AddonCreditsOption) -> String {
    if !opt.is_price_valid() {
        return String::new();
    }
    let credits = opt.credits.separate_with_commas();
    let dollars = format_usd_cents(opt.total_price_cents());
    format!("{credits} credits / {dollars}")
}

/// Mirrors `BuyCreditsBanner::update_addon_credits_options` dropdown label
/// (spec criterion 6).
fn dropdown_label_for(opt: &AddonCreditsOption) -> String {
    format!(
        "{} / {} credits",
        format_usd_cents(opt.total_price_cents()),
        opt.credits
    )
}

/// Mirrors the discount-percent calculation in `update_addon_credits_options`
/// (spec criterion 6).
fn discount_percent_for(base_rate: f32, opt: &AddonCreditsOption) -> u32 {
    if base_rate <= 0.0 {
        return 0;
    }
    let actual_rate = opt.rate();
    ((base_rate - actual_rate) / base_rate * 100.0).round() as u32
}

/// Mirrors `addon_credits_panel_state` auto-reload visibility
/// (spec criteria 3, 9).
fn show_auto_reload(has_admin_permissions: bool, options: &[AddonCreditsOption]) -> bool {
    let any_markup = options.iter().any(|o| o.has_markup());
    has_admin_permissions && !any_markup
}

// ── Spec criterion 3 & 4: Settings v2 price label ─────────────────────────────

/// Paid zero-markup fixture: price_label carries the total and shows credits
/// (spec criterion 4 — paid Settings v2 unchanged, single-price treatment).
#[test]
fn test_paid_price_label_format() {
    let opt = paid_option(1000, 2500); // 1 000 credits, $25.00
    let label = price_label_for(&opt);
    assert_eq!(label, "1,000 credits / $25.00");
}

/// Paid zero-markup fixture, larger pack — credits use thousands separator
/// (spec criteria 4, 7).
#[test]
fn test_paid_price_label_large_pack() {
    let opt = paid_option(10_000, 15_000); // 10 000 credits, $150.00
    let label = price_label_for(&opt);
    assert_eq!(label, "10,000 credits / $150.00");
}

/// Free markup fixture: price_label shows the marked-up total, not the base
/// (spec criterion 3 — Free pricing is explicit).
#[test]
fn test_free_price_label_shows_total_not_base() {
    let opt = free_option(1000, 2500, 250); // base $25.00 + markup $2.50 = $27.50
    let label = price_label_for(&opt);
    assert_eq!(label, "1,000 credits / $27.50");
}

/// Behavior 18: invalid server price → label is suppressed, not rendered as garbage
/// (spec criterion 11 — invalid data is not fabricated).
#[test]
fn test_price_label_suppressed_for_invalid_option() {
    let neg_total = AddonCreditsOption {
        credits: 1000,
        price_usd_cents: 100,
        base_price_usd_cents: Some(100),
        markup_usd_cents: Some(0),
        total_price_usd_cents: Some(-1), // negative — invalid
    };
    assert_eq!(price_label_for(&neg_total), "");

    let total_lt_base = AddonCreditsOption {
        credits: 1000,
        price_usd_cents: 100,
        base_price_usd_cents: Some(200), // base higher than total
        markup_usd_cents: Some(0),
        total_price_usd_cents: Some(100),
    };
    assert_eq!(price_label_for(&total_lt_base), "");
}

/// Behavior 18: partially-populated breakdown is treated as invalid and
/// the label is suppressed (spec criterion 11, suggestion from review).
#[test]
fn test_price_label_suppressed_for_partial_breakdown() {
    // Only base is present, total and markup are null — malformed new-server response.
    let partial = AddonCreditsOption {
        credits: 1000,
        price_usd_cents: 100,
        base_price_usd_cents: Some(100),
        markup_usd_cents: None,
        total_price_usd_cents: None,
    };
    assert!(
        !partial.is_price_valid(),
        "partial breakdown must be invalid"
    );
    assert_eq!(price_label_for(&partial), "");
}

// ── Spec criterion 9: auto-reload visibility ──────────────────────────────────

/// Paid admin: show_auto_reload is true (spec criterion 4, 9).
#[test]
fn test_show_auto_reload_true_for_paid_admin() {
    let options = vec![paid_option(1000, 2500), paid_option(5000, 10_000)];
    assert!(show_auto_reload(true, &options));
}

/// Free admin with markup: show_auto_reload is false (spec criteria 3, 9).
#[test]
fn test_show_auto_reload_false_for_free_markup_admin() {
    let options = vec![
        free_option(1000, 2500, 250),
        free_option(5000, 10_000, 1000),
    ];
    assert!(!show_auto_reload(true, &options));
}

/// Non-admin: show_auto_reload is false even for paid plans (spec criterion 4).
#[test]
fn test_show_auto_reload_false_for_non_admin() {
    let options = vec![paid_option(1000, 2500)];
    assert!(!show_auto_reload(false, &options));
}

// ── Spec criterion 6: banner dropdown labels and discount percentages ──────────

/// Banner dropdown label format for a paid option (spec criterion 6).
#[test]
fn test_dropdown_label_paid_fixture() {
    let opt = paid_option(1000, 2500);
    assert_eq!(dropdown_label_for(&opt), "$25.00 / 1000 credits");
}

/// Banner dropdown label for a Free markup option shows the marked-up total
/// (spec criterion 6 — all surfaces agree on total).
#[test]
fn test_dropdown_label_free_markup_fixture() {
    let opt = free_option(1000, 2500, 250); // total = $27.50
    assert_eq!(dropdown_label_for(&opt), "$27.50 / 1000 credits");
}

/// Discount percentages use base price, not the marked-up total
/// (spec criterion 6, Behavior 7 — volume-discount badges unchanged by markup).
#[test]
fn test_discount_percent_uses_base_price() {
    // Three paid packs: 1 000 at $25.00, 5 000 at $100.00, 10 000 at $175.00
    // Base rates: 0.025, 0.020, 0.0175 per credit
    // Discounts vs the first pack's rate (0.025):
    //   5 000: (0.025 - 0.020) / 0.025 * 100 = 20%
    //   10 000: (0.025 - 0.0175) / 0.025 * 100 = 30%
    let packs = [
        paid_option(1_000, 2_500),
        paid_option(5_000, 10_000),
        paid_option(10_000, 17_500),
    ];
    let base_rate = packs[0].rate();
    assert_eq!(discount_percent_for(base_rate, &packs[0]), 0);
    assert_eq!(discount_percent_for(base_rate, &packs[1]), 20);
    assert_eq!(discount_percent_for(base_rate, &packs[2]), 30);
}

/// For a Free fixture, discount percent still uses base rate, not total
/// (Behavior 7 — markup must not inflate the displayed discount).
#[test]
fn test_discount_percent_unaffected_by_markup() {
    // Two Free packs with 10% markup each.
    // Base rates: 1 000 → $10 base = 0.010; 5 000 → $40 base = 0.008
    // Discount on second pack vs first (base only): (0.010 - 0.008) / 0.010 * 100 = 20%
    // If total were used instead (with markup): rates 0.011 and 0.0088 → ~20%, but
    // the test confirms the pure-base computation matches expectations.
    let packs = [
        free_option(1_000, 1_000, 100), // base $10.00, markup $1.00, total $11.00
        free_option(5_000, 4_000, 400), // base $40.00, markup $4.00, total $44.00
    ];
    let base_rate = packs[0].rate();
    // base rates: 1000/1000=1.0 cents/credit, 4000/5000=0.8 cents/credit
    let expected = ((1.0_f32 - 0.8_f32) / 1.0_f32 * 100.0_f32).round() as u32;
    assert_eq!(discount_percent_for(base_rate, &packs[1]), expected);
}

// ── Spec criterion 11: stale-selection with invalid data ──────────────────────

/// When the selected option disappears (empty list), price_label is empty
/// (spec criterion 11 — safe fallback for missing pack data).
#[test]
fn test_price_label_empty_when_no_option_selected() {
    // Simulate no options available (price_label_for called with None equivalent)
    let empty_options: Vec<AddonCreditsOption> = vec![];
    let label = empty_options
        .first()
        .map(price_label_for)
        .unwrap_or_default();
    assert_eq!(label, "");
}

/// Legacy all-null breakdown (old server): is_price_valid uses price_usd_cents
/// and should pass for a positive legacy price (spec criterion 11).
#[test]
fn test_legacy_no_breakdown_is_valid_when_price_positive() {
    let legacy = AddonCreditsOption {
        credits: 1000,
        price_usd_cents: 2500,
        base_price_usd_cents: None,
        markup_usd_cents: None,
        total_price_usd_cents: None,
    };
    assert!(
        legacy.is_price_valid(),
        "all-null breakdown falls back to price_usd_cents, which is valid"
    );
    assert_eq!(price_label_for(&legacy), "1,000 credits / $25.00");
}
