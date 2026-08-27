//! Data-source-agnostic rendering for the pricing-transparency usage
//! breakdown: the per-model stacked bar, per-model rows with an expandable
//! input/output/cache/web-search cost breakdown, the platform-usage row,
//! tool-call-summary content, and the small formatting/color/tooltip
//! helpers they all share.
//!
//! This module is shared by two callers:
//! * The footer's "Conversation" usage popover
//!   ([`super::usage_popover_view::UsagePopoverView`]), which reads usage
//!   live from an in-memory `AIConversation` and wraps this module's output
//!   in its own chrome (title header, "View account usage" link, floating
//!   `Dismiss`ible container).
//! * The Settings Billing & Usage "Usage History" accordion
//!   (`crate::settings_view::billing_and_usage::usage_history_entry`), which
//!   reads usage from a persisted, server-sourced
//!   `persistence::model::ConversationUsageMetadata` snapshot and renders
//!   this module's output "chromeless" (no title, no link, no dismiss
//!   border) directly inside the accordion row.
//!
//! Functions here take plain data (token/cost maps, tool usage stats)
//! rather than an `AIConversation`, and own no popover/accordion "chrome" or
//! per-view UI state (section-collapse booleans, `Dismiss`, mouse states for
//! chrome-level affordances). Interactivity that *is* shared (the per-model
//! cost-breakdown expand/collapse) is parameterized via plain closures/flags
//! rather than a hardcoded `TypedActionView::Action`, since the two callers
//! dispatch through different action enums.
//!
//! Not included here: the orchestration rollup (per-agent breakdown, Surface
//! 6) and context-window breakdown. The rollup is popover-only for now (see
//! the TODO on [`render_settings_history_content`]); the context-window
//! breakdown was intentionally dropped from both surfaces (see
//! `conversation_usage_view`'s module docs for the legacy footer pill that
//! still renders it).

use std::cmp::Ordering;
use std::collections::HashMap;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::{Vector2F, vec2f};
use thousands::Separable;
use warp_core::ui::Icon;
use warpui::elements::{
    Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    DispatchEventResult, DropShadow, Empty, EventHandler, Expanded, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Shrinkable, Stack, Text,
};
use warpui::text_layout::ClipConfig;
use warpui::{AppContext, Element, EventContext};

use crate::ai::blocklist::usage::colors::color_for_model;
use crate::appearance::Appearance;
use crate::features::FeatureFlag;
use crate::persistence::model::{
    ConversationUsageMetadata, FULL_TERMINAL_USE_CATEGORY, ModelTokenUsage, PRIMARY_AGENT_CATEGORY,
    PersistedModelTokenCost, ToolUsageMetadata,
};
use crate::ui_components::blended_colors;

/// Height of the segmented usage bar.
pub const BAR_HEIGHT: f32 = 6.;
/// Width/height of the small color swatch next to each row label.
pub const SWATCH_SIZE: f32 = 8.;

/// Boxed click handler used by [`render_model_usage_row`] to toggle a
/// model's cost-breakdown subsection open/closed. Left generic over the
/// action it ultimately dispatches (rather than a concrete
/// `TypedActionView::Action`) so this module doesn't need to know which of
/// its two callers -- each with its own action enum -- is invoking it.
pub type ToggleModelHandler =
    Box<dyn FnMut(&mut EventContext, &AppContext, Vector2F) -> DispatchEventResult>;

/// A `Flex::row` preconfigured for `label ... value` rows: cross-axis
/// centered, and `SpaceBetween` + `MainAxisSize::Max` so the two ends
/// actually push apart (an easy warpui footgun: `SpaceBetween` alone has no
/// effect unless the row is also told to claim the max available width).
pub fn space_between_row() -> Flex {
    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_main_axis_size(MainAxisSize::Max)
}

/// One row of the per-model usage breakdown, aggregated across a model's
/// warp/byok/custom-endpoint token counts. Role badges mirror the existing
/// credits-based breakdown's category constants. `charged_usage` comes from
/// a separate per-model structure joined in by model id, since the token/
/// category source doesn't carry cost; `cost_in_cents` is derived from it
/// for the collapsed row's value text.
pub struct ModelUsageRow {
    pub model_id: String,
    pub role_badge: Option<&'static str>,
    pub tokens: u64,
    pub cost_in_cents: Option<f32>,
    pub charged_usage: Option<PersistedModelTokenCost>,
}

/// Builds the sorted per-model row list from raw per-conversation token
/// usage. Rows are ordered primary-agent-first (matching the existing
/// credits-based breakdown's sort), then alphabetically by model id.
pub fn model_usage_rows(
    models: &[ModelTokenUsage],
    charged_usage_by_model: &HashMap<String, PersistedModelTokenCost>,
) -> Vec<ModelUsageRow> {
    let mut rows: Vec<ModelUsageRow> = models
        .iter()
        .filter_map(|model| {
            let tokens = model.warp_tokens as u64
                + model.byok_tokens as u64
                + model.custom_endpoint_tokens as u64;
            if tokens == 0 {
                return None;
            }
            let role_badge = role_badge_for_model(model);
            let charged_usage = charged_usage_by_model.get(&model.model_id).copied();
            Some(ModelUsageRow {
                model_id: model.model_id.clone(),
                role_badge,
                tokens,
                cost_in_cents: charged_usage.map(|usage| usage.cost_in_cents()),
                charged_usage,
            })
        })
        .collect();
    rows.sort_by(|a, b| match (a.role_badge, b.role_badge) {
        (Some("Primary agent"), Some("Primary agent")) => a.model_id.cmp(&b.model_id),
        (Some("Primary agent"), _) => Ordering::Less,
        (_, Some("Primary agent")) => Ordering::Greater,
        _ => a.model_id.cmp(&b.model_id),
    });
    rows
}

/// Determines the role-pill text for a model based on which token-usage
/// category buckets it has non-zero tokens in. Mirrors the category
/// constants used by the existing credits-based breakdown
/// (`PRIMARY_AGENT_CATEGORY` / `FULL_TERMINAL_USE_CATEGORY`).
fn role_badge_for_model(model: &ModelTokenUsage) -> Option<&'static str> {
    let categories = [
        &model.warp_token_usage_by_category,
        &model.byok_token_usage_by_category,
        &model.custom_endpoint_token_usage_by_category,
    ];
    let has_category = |category: &str| {
        categories
            .iter()
            .any(|map| map.get(category).is_some_and(|&tokens| tokens > 0))
    };
    if has_category(PRIMARY_AGENT_CATEGORY) {
        Some("Primary agent")
    } else if has_category(FULL_TERMINAL_USE_CATEGORY) {
        Some("Full terminal use")
    } else {
        None
    }
}

/// Formats a raw token count using `k`/`M`-suffixed abbreviations above
/// 1,000 and 1,000,000 tokens respectively (e.g. `9.6k`, `1.6M`), matching
/// the Figma copy's token formatting.
pub(crate) fn format_token_count(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.)
    } else if tokens >= 1000 {
        format!("{:.1}k", tokens as f64 / 1000.)
    } else {
        tokens.to_string()
    }
}

/// Returns the exact (unabbreviated, comma-separated) token count for a
/// tooltip, e.g. `"9,614 tokens"` -- `None` when `tokens` is small enough
/// that [`format_token_count`] wouldn't have abbreviated it in the first
/// place, since a tooltip repeating an already-exact "500 tokens" would be
/// redundant.
pub(crate) fn exact_token_count_tooltip(tokens: u64) -> Option<String> {
    (tokens >= 1000).then(|| format!("{} tokens", tokens.separate_with_commas()))
}

/// Formats a token count alongside its dollar cost, e.g. `"9.6k tokens /
/// $0.36"`. Per the pricing-transparency "do not show credits" decision,
/// this is the only value format used across these surfaces -- credits are
/// never displayed. Either figure may be unknown: the two are joined with
/// `/` when both are known, and this falls back to whichever single figure
/// is available, or an em dash when neither is known. The dollar figure is
/// omitted entirely when `FeatureFlag::PricingTransparency` is disabled.
pub(crate) fn format_tokens_and_cost(tokens: Option<u64>, cost_in_cents: Option<f32>) -> String {
    let token_text = tokens.map(|tokens| format!("{} tokens", format_token_count(tokens)));
    let cost_text = FeatureFlag::PricingTransparency
        .is_enabled()
        .then(|| cost_in_cents.map(|cost| format!("${:.2}", cost / 100.)))
        .flatten();
    match (token_text, cost_text) {
        (Some(tokens), Some(cost)) => format!("{tokens} / {cost}"),
        (Some(tokens), None) => tokens,
        (None, Some(cost)) => cost,
        (None, None) => "\u{2014}".to_string(),
    }
}

/// Formats a count alongside its dollar cost, e.g. `"3 searches / $0.02"`,
/// for breakdown rows whose unit isn't tokens (currently just web
/// searches). The dollar figure is omitted when `FeatureFlag::PricingTransparency`
/// is disabled, matching [`format_tokens_and_cost`].
fn format_count_and_cost(count: u32, unit: &str, cost_in_cents: f32) -> String {
    let count_text = format!("{count} {unit}");
    if !FeatureFlag::PricingTransparency.is_enabled() {
        return count_text;
    }
    format!("{count_text} / ${:.2}", cost_in_cents / 100.)
}

/// Formats a bare dollar cost, e.g. `"$0.36"`, for values with no
/// associated token/count figure (currently the platform fee). Shows an em
/// dash when the cost is unknown or `FeatureFlag::PricingTransparency` is
/// disabled.
pub(crate) fn format_cost_only(cost_in_cents: Option<f32>) -> String {
    if !FeatureFlag::PricingTransparency.is_enabled() {
        return "\u{2014}".to_string();
    }
    match cost_in_cents {
        Some(cost) => format!("${:.2}", cost / 100.),
        None => "\u{2014}".to_string(),
    }
}

/// Renders a small rounded color swatch used to key a row to its bar
/// segment.
pub fn render_swatch(color: ColorU) -> Box<dyn Element> {
    Container::new(
        ConstrainedBox::new(Empty::new().finish())
            .with_width(SWATCH_SIZE)
            .with_height(SWATCH_SIZE)
            .finish(),
    )
    .with_background_color(color)
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.)))
    .finish()
}

/// Renders a full-width segmented bar. `segments` is a list of (color,
/// percentage) pairs; any remaining percentage up to 100 is filled with
/// `track_color`. The leading and trailing edges of the bar are rounded
/// (each visible segment's own edges stay square except at those two ends),
/// giving the overall bar a pill-like shape.
pub fn render_segmented_bar(segments: &[(ColorU, f32)], track_color: ColorU) -> Box<dyn Element> {
    let mut visible: Vec<(ColorU, f32)> = segments
        .iter()
        .copied()
        .filter(|(_, pct)| *pct > 0.)
        .collect();
    let used_pct: f32 = visible.iter().map(|(_, pct)| pct).sum();
    let remainder = (100. - used_pct).max(0.);
    if remainder > 0. {
        visible.push((track_color, remainder));
    }

    let end_radius = Radius::Pixels(BAR_HEIGHT / 2.);
    let last_index = visible.len().saturating_sub(1);
    let mut row = Flex::row();
    for (index, (color, pct)) in visible.iter().enumerate() {
        let mut corner_radius = CornerRadius::default();
        if index == 0 {
            corner_radius.merge(CornerRadius::with_left(end_radius));
        }
        if index == last_index {
            corner_radius.merge(CornerRadius::with_right(end_radius));
        }
        row.add_child(
            Expanded::new(
                *pct,
                Container::new(Empty::new().finish())
                    .with_background_color(*color)
                    .with_corner_radius(corner_radius)
                    .finish(),
            )
            .finish(),
        );
    }

    ConstrainedBox::new(row.finish())
        .with_height(BAR_HEIGHT)
        .finish()
}

pub fn render_label_value_row(
    label: &str,
    value: String,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let background = theme.surface_2();
    let font_size = appearance.ui_font_size();
    space_between_row()
        .with_child(
            Text::new(label.to_string(), appearance.ui_font_family(), font_size)
                .with_color(blended_colors::text_sub(theme, background))
                .finish(),
        )
        .with_child(
            Text::new(value, appearance.ui_font_family(), font_size)
                .with_color(blended_colors::text_main(theme, background))
                .finish(),
        )
        .finish()
}

pub fn render_diffs_row(
    lines_added: i32,
    lines_removed: i32,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let background = theme.surface_2();
    let font_size = appearance.ui_font_size();
    space_between_row()
        .with_child(
            Text::new(
                "Diffs applied".to_string(),
                appearance.ui_font_family(),
                font_size,
            )
            .with_color(blended_colors::text_sub(theme, background))
            .finish(),
        )
        .with_child(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Text::new(
                        format!("+{lines_added}"),
                        appearance.ui_font_family(),
                        font_size,
                    )
                    .with_color(theme.ansi_fg_green())
                    .finish(),
                )
                .with_child(
                    Container::new(
                        Text::new(
                            format!("-{lines_removed}"),
                            appearance.ui_font_family(),
                            font_size,
                        )
                        .with_color(theme.ansi_fg_red())
                        .finish(),
                    )
                    .with_margin_left(6.)
                    .finish(),
                )
                .finish(),
        )
        .finish()
}

/// Renders a small opaque tooltip box containing `text`. Uses an explicit
/// solid background (`theme.surface_3()`) rather than the shared `Tooltip`
/// UI component's default, which derives from `theme.background()` -- the
/// one theme color allowed to carry the user's configured window-opacity/
/// blur setting, which is exactly why that default can read as translucent
/// here. `surface_3()` is an always-opaque surface color.
pub fn render_tooltip_box(text: String, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let bg = theme.surface_3().into_solid();
    Container::new(
        Text::new(text, appearance.ui_font_family(), appearance.ui_font_size())
            .with_color(blended_colors::text_main(theme, bg))
            .with_selectable(false)
            .finish(),
    )
    .with_background_color(bg)
    .with_border(Border::all(1.).with_border_color(theme.outline().into_solid()))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
    .with_padding_left(8.)
    .with_padding_right(8.)
    .with_padding_top(4.)
    .with_padding_bottom(4.)
    .with_drop_shadow(
        DropShadow::new_with_standard_offset_and_spread(ColorU::new(0, 0, 0, 48))
            .with_offset(vec2f(0., 4.)),
    )
    .finish()
}

/// Wraps `content` in a hover tooltip showing `tooltip_text` below its
/// bottom-left corner, using the given (persistent, per-instance)
/// `hover_state` so the hover-in delay can actually fire.
pub fn with_tooltip(
    hover_state: MouseStateHandle,
    content: Box<dyn Element>,
    tooltip_text: String,
    appearance: &Appearance,
) -> Box<dyn Element> {
    Hoverable::new(hover_state, |state| {
        let mut stack = Stack::new().with_child(content);
        if state.is_hovered() {
            stack.add_positioned_overlay_child(
                render_tooltip_box(tooltip_text, appearance),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 4.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }
        stack.finish()
    })
    .finish()
}

/// Wraps `content` in a hover tooltip when both a `hover_state_for` factory
/// and `tooltip_text` are present; returns `content` unchanged otherwise.
/// `hover_state_for` is `None` for callers (e.g. the Settings history
/// accordion) that don't maintain per-value persistent hover state.
pub fn maybe_with_tooltip(
    hover_state_for: Option<&dyn Fn(String) -> MouseStateHandle>,
    key: String,
    content: Box<dyn Element>,
    tooltip_text: Option<String>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    match (hover_state_for, tooltip_text) {
        (Some(hover_state_for), Some(tooltip_text)) => {
            with_tooltip(hover_state_for(key), content, tooltip_text, appearance)
        }
        _ => content,
    }
}

/// Like [`render_label_value_row`], but wraps the value in a hover tooltip
/// (see [`maybe_with_tooltip`]) when both `hover_state_for` and
/// `tooltip_text` are present -- used for token-count rows whose displayed
/// value may be abbreviated.
pub fn render_label_value_row_with_tooltip(
    hover_state_for: Option<&dyn Fn(String) -> MouseStateHandle>,
    key: String,
    label: &str,
    value: String,
    tooltip_text: Option<String>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let background = theme.surface_2();
    let font_size = appearance.ui_font_size();
    let value_element = Text::new(value, appearance.ui_font_family(), font_size)
        .with_color(blended_colors::text_main(theme, background))
        .finish();
    let value_element = maybe_with_tooltip(
        hover_state_for,
        key,
        value_element,
        tooltip_text,
        appearance,
    );
    space_between_row()
        .with_child(
            Text::new(label.to_string(), appearance.ui_font_family(), font_size)
                .with_color(blended_colors::text_sub(theme, background))
                .finish(),
        )
        .with_child(value_element)
        .finish()
}

/// Renders a model's input/output/cache/web-search charged-usage breakdown,
/// shown beneath a per-model row when expanded. Rows are omitted for
/// categories the model didn't incur (e.g. cache tokens only apply to
/// Anthropic models, and web searches are relatively rare).
pub fn render_charged_usage_breakdown(
    hover_state_for: Option<&dyn Fn(String) -> MouseStateHandle>,
    model_id: &str,
    charged_usage: &PersistedModelTokenCost,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut column = Flex::column().with_spacing(4.);
    if charged_usage.total_input > 0 {
        column.add_child(render_label_value_row_with_tooltip(
            hover_state_for,
            format!("value:model:{model_id}:input"),
            "Input tokens",
            format_tokens_and_cost(
                Some(charged_usage.total_input),
                Some(charged_usage.input_cost_in_cents),
            ),
            exact_token_count_tooltip(charged_usage.total_input),
            appearance,
        ));
    }
    if charged_usage.output > 0 {
        column.add_child(render_label_value_row_with_tooltip(
            hover_state_for,
            format!("value:model:{model_id}:output"),
            "Output tokens",
            format_tokens_and_cost(
                Some(charged_usage.output),
                Some(charged_usage.output_cost_in_cents),
            ),
            exact_token_count_tooltip(charged_usage.output),
            appearance,
        ));
    }
    if charged_usage.input_cache_read > 0 {
        column.add_child(render_label_value_row_with_tooltip(
            hover_state_for,
            format!("value:model:{model_id}:cache_read"),
            "Cache read tokens",
            format_tokens_and_cost(
                Some(charged_usage.input_cache_read),
                Some(charged_usage.input_cache_read_cost_in_cents),
            ),
            exact_token_count_tooltip(charged_usage.input_cache_read),
            appearance,
        ));
    }
    if charged_usage.input_cache_write > 0 {
        column.add_child(render_label_value_row_with_tooltip(
            hover_state_for,
            format!("value:model:{model_id}:cache_write"),
            "Cache write tokens",
            format_tokens_and_cost(
                Some(charged_usage.input_cache_write),
                Some(charged_usage.input_cache_write_cost_in_cents),
            ),
            exact_token_count_tooltip(charged_usage.input_cache_write),
            appearance,
        ));
    }
    if charged_usage.web_search_count > 0 {
        column.add_child(render_label_value_row(
            "Web searches",
            format_count_and_cost(
                charged_usage.web_search_count as u32,
                "searches",
                charged_usage.web_search_cost_in_cents,
            ),
            appearance,
        ));
    }
    column.finish()
}

/// Renders a per-model row. When `toggle_handler` is `Some`, the row is
/// clickable (a trailing chevron indicates this) and toggles the cost
/// breakdown subsection per `expanded`; when `None`, the row is
/// non-interactive and `expanded` alone determines whether the breakdown
/// shows (used by the Settings history accordion, which always shows every
/// model's breakdown rather than tracking per-model expand state).
pub fn render_model_usage_row(
    row: &ModelUsageRow,
    expanded: bool,
    hover_state_for: Option<&dyn Fn(String) -> MouseStateHandle>,
    toggle_handler: Option<ToggleModelHandler>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let background = theme.surface_2();
    let font_size = appearance.ui_font_size();
    let color = color_for_model(&row.model_id);

    let full_label = match row.role_badge {
        Some(role) => format!("{} ({role})", row.model_id),
        None => row.model_id.clone(),
    };
    let chevron_color = blended_colors::text_disabled(theme, background);
    let chevron_icon = if expanded {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };

    // Model name and role badge are separate `Text`s (rather than one
    // formatted string) so they can use different colors. The name uses
    // `Shrinkable` (not `Expanded`) so it only claims as much width as it
    // actually needs (ellipsis-clipping once it runs out of room) instead of
    // being force-stretched to fill the row.
    let mut label_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    label_row.add_child(
        Shrinkable::new(
            1.,
            Text::new(row.model_id.clone(), appearance.ui_font_family(), font_size)
                .with_color(blended_colors::text_main(theme, background))
                .soft_wrap(false)
                .with_clip(ClipConfig::ellipsis())
                .finish(),
        )
        .finish(),
    );
    if let Some(role) = row.role_badge {
        label_row.add_child(
            Text::new(format!(" ({role})"), appearance.ui_font_family(), font_size)
                .with_color(blended_colors::text_sub(theme, background))
                .finish(),
        );
    }

    let label_with_tooltip = maybe_with_tooltip(
        hover_state_for,
        format!("label:model:{}", row.model_id),
        label_row.finish(),
        Some(full_label),
        appearance,
    );

    // The label is wrapped in `Expanded`: a plain (non-flex) `Text` in a
    // `Flex::row` sizes to its own intrinsic width regardless of the row's
    // available space, so a long model name would push the trailing token/
    // cost value and chevron off the edge instead of being ellipsis-clipped.
    let left = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(7.)
        .with_child(render_swatch(color))
        .with_child(Expanded::new(1., label_with_tooltip).finish());

    let value = Text::new(
        format_tokens_and_cost(Some(row.tokens), row.cost_in_cents),
        appearance.ui_font_family(),
        font_size,
    )
    .with_color(blended_colors::text_main(theme, background))
    .finish();
    let value = maybe_with_tooltip(
        hover_state_for,
        format!("value:model:{}", row.model_id),
        value,
        exact_token_count_tooltip(row.tokens),
        appearance,
    );

    let mut right = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(6.)
        .with_child(value);
    if toggle_handler.is_some() {
        right.add_child(
            ConstrainedBox::new(chevron_icon.to_warpui_icon(chevron_color.into()).finish())
                .with_width(10.)
                .with_height(10.)
                .finish(),
        );
    }

    let summary_row = space_between_row()
        .with_child(Expanded::new(1., left.finish()).finish())
        .with_child(Container::new(right.finish()).with_margin_left(8.).finish())
        .finish();

    let mut column = Flex::column().with_spacing(6.).with_child(summary_row);
    if expanded {
        let breakdown = match row.charged_usage {
            Some(charged_usage) => render_charged_usage_breakdown(
                hover_state_for,
                &row.model_id,
                &charged_usage,
                appearance,
            ),
            None => Text::new(
                "No detailed breakdown available".to_string(),
                appearance.ui_font_family(),
                font_size,
            )
            .with_color(blended_colors::text_disabled(theme, background))
            .finish(),
        };
        // Extra right padding pulls the breakdown's trailing token/cost
        // values in from the popover's edge, so they visibly underhang
        // (rather than overhang) the model total row's own value, which
        // stops short of the edge to make room for its chevron.
        column.add_child(
            Container::new(breakdown)
                .with_padding_left(15.)
                .with_padding_right(20.)
                .finish(),
        );
    }

    match toggle_handler {
        Some(toggle_handler) => EventHandler::new(column.finish())
            .on_left_mouse_down(toggle_handler)
            .finish(),
        None => column.finish(),
    }
}

/// Renders the full per-model usage breakdown body: an "All models" summary
/// row, the segmented stacked bar, and one row per model (see
/// [`render_model_usage_row`]). Returns an empty element when there's no
/// usage to show.
///
/// `all_models_tokens_and_cost` supplies the "All models" row's figures
/// directly (rather than summing per-model rows) so it always agrees with
/// whatever conversation-/history-wide total the caller already computed
/// (mirroring the invariant the popover established between its collapsed-
/// section summary and this row).
#[allow(clippy::too_many_arguments)]
pub fn render_model_usage_rows_section(
    models: &[ModelTokenUsage],
    charged_usage_by_model: &HashMap<String, PersistedModelTokenCost>,
    all_models_tokens_and_cost: (Option<u64>, Option<f32>),
    is_expanded: &dyn Fn(&str) -> bool,
    hover_state_for: Option<&dyn Fn(String) -> MouseStateHandle>,
    make_toggle_handler: Option<&dyn Fn(String) -> ToggleModelHandler>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let rows = model_usage_rows(models, charged_usage_by_model);
    if rows.is_empty() {
        return Empty::new().finish();
    }
    let total_tokens: u64 = rows.iter().map(|r| r.tokens).sum();
    let theme = appearance.theme();
    let background = theme.surface_2();
    let font_size = appearance.ui_font_size();

    let mut column = Flex::column().with_spacing(6.);

    let (all_models_tokens, all_models_cost) = all_models_tokens_and_cost;
    let all_models_tokens = all_models_tokens.or(Some(total_tokens));
    let all_models_value = Text::new(
        format_tokens_and_cost(all_models_tokens, all_models_cost),
        appearance.ui_font_family(),
        font_size,
    )
    .with_color(blended_colors::text_main(theme, background))
    .finish();
    let all_models_value = maybe_with_tooltip(
        hover_state_for,
        "value:all_models".to_string(),
        all_models_value,
        all_models_tokens.and_then(exact_token_count_tooltip),
        appearance,
    );
    column.add_child(
        space_between_row()
            .with_child(
                Text::new(
                    "All models".to_string(),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(blended_colors::text_sub(theme, background))
                .finish(),
            )
            .with_child(all_models_value)
            .finish(),
    );

    // Stacked bar: one segment per model, proportional to token share.
    let segments: Vec<(ColorU, f32)> = rows
        .iter()
        .map(|row| {
            let pct = if total_tokens == 0 {
                0.
            } else {
                (row.tokens as f32 / total_tokens as f32) * 100.
            };
            (color_for_model(&row.model_id), pct)
        })
        .collect();
    column.add_child(render_segmented_bar(
        &segments,
        theme.outline().into_solid(),
    ));

    for row in &rows {
        let expanded = is_expanded(&row.model_id);
        let toggle_handler = make_toggle_handler.map(|make| make(row.model_id.clone()));
        column.add_child(render_model_usage_row(
            row,
            expanded,
            hover_state_for,
            toggle_handler,
            appearance,
        ));
    }

    column.finish()
}

/// Renders a non-collapsible overline section header with a value on the
/// right: an overline `label` on the left, `value` on the right, no click
/// handling. Used for sections (e.g. Platform Usage) that have no expand/
/// collapse state or separate content rows.
pub fn render_static_label_value_header(
    label: &str,
    value: String,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let background = theme.surface_2();
    let label_color = blended_colors::text_disabled(theme, background);
    let value_color = blended_colors::text_sub(theme, background);

    space_between_row()
        .with_child(
            Text::new(
                label.to_string(),
                appearance.overline_font_family(),
                appearance.overline_font_size() + 2.,
            )
            .with_color(label_color)
            .finish(),
        )
        .with_child(
            Text::new(
                value,
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(value_color)
            .finish(),
        )
        .finish()
}

/// Renders a plain, non-interactive overline section label (no value, no
/// chevron) -- used by the Settings history accordion in place of the
/// popover's own collapsible section headers, which carry per-view
/// collapse state this module doesn't own.
pub fn render_overline_label(label: &str, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let background = theme.surface_2();
    Text::new(
        label.to_string(),
        appearance.overline_font_family(),
        appearance.overline_font_size() + 2.,
    )
    .with_color(blended_colors::text_disabled(theme, background))
    .finish()
}

/// Renders the tool-call-summary content rows (tool calls, files changed,
/// diffs applied, commands executed) -- no section header, no expand/
/// collapse.
pub fn render_tool_call_summary_content(
    tool_usage: &ToolUsageMetadata,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut inner = Flex::column().with_spacing(4.);
    inner.add_child(render_label_value_row(
        "Tool calls",
        format!("{}", tool_usage.total_tool_calls()),
        appearance,
    ));
    inner.add_child(render_label_value_row(
        "Files changed",
        format!("{}", tool_usage.apply_file_diff_stats.files_changed),
        appearance,
    ));
    inner.add_child(render_diffs_row(
        tool_usage.apply_file_diff_stats.lines_added,
        tool_usage.apply_file_diff_stats.lines_removed,
        appearance,
    ));
    inner.add_child(render_label_value_row(
        "Commands executed",
        format!("{}", tool_usage.run_command_stats.commands_executed),
        appearance,
    ));
    inner.finish()
}

/// Renders the Settings "Usage History" accordion's expanded-row content:
/// the per-model usage breakdown (bar + per-model rows, cost breakdowns
/// always shown), the platform-usage row, and the tool-call summary --
/// chromeless (no title header, no "View account usage" link, no
/// `Dismiss`/floating border) and without per-model expand/collapse
/// interactivity, since this call site has no persistent per-model UI state
/// to back a toggle (every model's breakdown renders open instead).
///
/// Does not render a context-window breakdown or an orchestration rollup
/// section (see module docs).
///
/// TODO(follow-up once the warp-server-rollup PR lands): once
/// `Query.conversationUsageForOrchestratedChildren(parentConversationId:
/// ID!): [ChildConversationUsage!]!` (each `ChildConversationUsage` carrying
/// `conversationId: ID!`, `agentDisplayName: String!`, and
/// `usageMetadata: ConversationUsageMetadata!`) is merged and deployed:
/// * Add a GraphQL query wrapping it in `crates/graphql` (mirroring
///   `GetConversationUsage`), plus a client conversion into a
///   `HashMap<String, Vec<ChildConversationUsageMetadata>>` grouped by agent
///   display name (see plan section 2d).
/// * Track a request handle + `rollup_children: Option<Vec<ChildConversationUsage>>`
///   on `UsageHistoryEntry`/its parent model, fetched on expand (2f).
/// * When present, transform the result via
///   `compute_orchestration_rollup`-equivalent logic and render an
///   "AGENT USAGE" rollup section here in place of the plain per-model
///   section, matching the popover's Surface 6 behavior. Show a loading
///   state while the fetch is in flight; omit the section (graceful
///   degradation) if there's no parent or the fetch fails.
pub fn render_settings_history_content(
    usage: &ConversationUsageMetadata,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let total_tokens: u64 = usage
        .token_usage
        .iter()
        .map(|model| (model.warp_tokens + model.byok_tokens + model.custom_endpoint_tokens) as u64)
        .sum();

    let mut column = Flex::column().with_spacing(12.);
    column.add_child(render_overline_label("INFERENCE USAGE", appearance));
    column.add_child(render_model_usage_rows_section(
        &usage.token_usage,
        &usage.cumulative_token_cost_by_model,
        (Some(total_tokens), usage.total_provider_cost_in_cents),
        &|_model_id| true,
        None,
        None,
        appearance,
    ));

    if let Some(platform_cost_in_cents) = usage
        .total_charged_usage
        .map(|charged_usage| charged_usage.platform_cost_in_cents)
        .filter(|&cost| cost > 0.)
    {
        column.add_child(render_static_label_value_header(
            "PLATFORM USAGE",
            format_cost_only(Some(platform_cost_in_cents)),
            appearance,
        ));
    }

    column.add_child(render_overline_label("TOOL CALL SUMMARY", appearance));
    column.add_child(render_tool_call_summary_content(
        &usage.tool_usage_metadata,
        appearance,
    ));

    column.finish()
}

#[cfg(test)]
#[path = "shared_usage_renderer_tests.rs"]
mod tests;
