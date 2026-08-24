//! The docked, closeable "Turn" panel (Surface 3 of the pricing-transparency
//! usage surfaces). Unlike [`super::conversation_usage_view::ConversationUsageView`]
//! (which shows conversation-cumulative totals, optionally alongside a
//! "last response" annotation), every numeric value in this view is scoped
//! to a single agent turn ("block") -- see the turn-scoped getters on
//! `AIConversation` (e.g. `tool_calls_for_last_block`).
//!
//! Per resolved user feedback on the per-turn-usage-panel spec, this panel:
//! * is triggered independently from (and has no cross-navigation link to)
//!   the "Conversation" popover (Surface 2);
//! * has no per-section collapse/expand affordance -- all sections (MODEL
//!   USAGE / TOOL CALL SUMMARY / RESPONSE TIME) are always fully expanded;
//! * aligns the value column across all sections, not just within each
//!   section;
//! * is dismissed via a standard "X" close button in the header.

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DropShadow, Flex,
    Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, Text,
};
use warpui::platform::Cursor;
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use super::conversation_usage_view::TimingInfo;
use super::render_context_window_usage_icon;
use crate::appearance::Appearance;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;

/// A single label/value pair rendered as a row in the panel's shared
/// label/value columns (see [`TurnUsageView::render`]).
type LabelValueRow = (Box<dyn Element>, Box<dyn Element>);

/// The panel's two shared columns, as parallel `(labels, values)` vectors.
/// See [`TurnUsageView::build_label_value_columns`].
type LabelValueColumns = (Vec<Box<dyn Element>>, Vec<Box<dyn Element>>);

/// Turn-scoped token/cost usage for a single model. A turn can involve
/// multiple models (e.g. if the user or router switched models mid-turn).
pub struct TurnModelUsage {
    /// The model's display identifier (e.g. `auto (cost-efficient)`).
    pub model_id: String,
    /// Total tokens (across warp/byok/custom-endpoint usage) spent on this
    /// model during this turn.
    pub tokens: u64,
    /// Provider cost incurred on this model during this turn, in US cents.
    /// `None` when a turn-scoped cost cannot be derived, in which case the
    /// cost is omitted from the row rather than rendered as `$0.00`.
    pub cost_in_cents: Option<f32>,
}

/// Turn-scoped usage data backing the "MODEL USAGE" section. All fields are
/// scoped to a single agent turn (block), not the whole conversation.
pub struct TurnUsageInfo {
    /// Per-model token/cost usage for this turn. One row is rendered per
    /// entry.
    pub models: Vec<TurnModelUsage>,
    /// Context window usage (0.0-1.0). This is inherently a
    /// conversation-level running total -- it cannot be scoped to a single
    /// turn -- but is shown here per the spec, which explicitly calls out
    /// this scope mixing as deliberate.
    pub context_window_usage: f32,
    pub tool_calls: i32,
    pub files_changed: i32,
    pub lines_added: i32,
    pub lines_removed: i32,
    pub commands_executed: i32,
}

/// Typed actions dispatched by widgets inside [`TurnUsageView`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnUsageViewAction {
    /// The user clicked the header's close ("X") button.
    Close,
}

/// Emitted so the owning view (the terminal view) can remove this panel
/// from the blocklist when the user clicks the close button.
#[derive(Clone, Debug)]
pub enum TurnUsageViewEvent {
    CloseRequested,
}

/// The docked "Turn" panel view. See module docs for scope/behavior.
pub struct TurnUsageView {
    pub usage_info: TurnUsageInfo,
    pub timing_info: Option<TimingInfo>,
    close_button_mouse_state: MouseStateHandle,
}

impl TurnUsageView {
    pub fn new(usage_info: TurnUsageInfo, timing_info: Option<TimingInfo>) -> Self {
        Self {
            usage_info,
            timing_info,
            close_button_mouse_state: MouseStateHandle::default(),
        }
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        let font_size = appearance.ui_font_size() + 2.;

        let title = Text::new("Turn".to_string(), appearance.ui_font_family(), font_size)
            .with_color(blended_colors::text_main(theme, background))
            .finish();

        let close_icon_size = font_size;
        let close_button = Hoverable::new(self.close_button_mouse_state.clone(), {
            let icon_color = blended_colors::text_sub(theme, background);
            move |state| {
                let mut container = Container::new(
                    ConstrainedBox::new(Icon::X.to_warpui_icon(icon_color.into()).finish())
                        .with_width(close_icon_size)
                        .with_height(close_icon_size)
                        .finish(),
                )
                .with_uniform_padding(2.);
                if state.is_hovered() {
                    container = container
                        .with_background(blended_colors::neutral_4(appearance.theme()))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
                }
                container.finish()
            }
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(TurnUsageViewAction::Close);
        })
        .finish();

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(title)
            .with_child(close_button)
            .finish()
    }

    /// Renders a section's small-caps label as a standalone row, to be
    /// followed by that section's data rows in the shared label/value
    /// columns. Unlike the original mockup, this is purely decorative (no
    /// chevron, not clickable): all sections are always fully expanded.
    fn render_section_header(label: &str, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        // A couple of points larger than the base overline size so the
        // section headers read clearly against the smaller body text.
        let header_font_size = appearance.overline_font_size() + 2.;
        Text::new(
            label.to_string(),
            appearance.overline_font_family(),
            header_font_size,
        )
        .with_color(blended_colors::text_disabled(theme, background))
        .soft_wrap(false)
        .finish()
    }

    fn model_usage_rows(&self, appearance: &Appearance) -> Vec<LabelValueRow> {
        let font_size = appearance.ui_font_size() + 2.;
        let theme = appearance.theme();
        let background = theme.surface_2();
        let text_color = blended_colors::text_main(theme, background);
        let label_color = blended_colors::text_sub(theme, background);

        let mut rows: Vec<LabelValueRow> = self
            .usage_info
            .models
            .iter()
            .map(|model| {
                let mut value_parts = vec![format_tokens(model.tokens)];
                if let Some(cost_in_cents) = model.cost_in_cents {
                    value_parts.push(format_dollars(cost_in_cents));
                }
                let label = Text::new(
                    model.model_id.clone(),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_style(warpui::fonts::Properties {
                    weight: warpui::fonts::Weight::Medium,
                    ..Default::default()
                })
                .with_color(text_color)
                .finish();
                let value = Text::new(
                    value_parts.join("  /  "),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(text_color)
                .finish();
                (label, value)
            })
            .collect();

        let context_window_label = Text::new(
            "Context window usage".to_string(),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(label_color)
        .finish();
        let context_usage_pct = (self.usage_info.context_window_usage * 100.).round();
        let context_window_value = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(4.)
            .with_child(
                ConstrainedBox::new(render_context_window_usage_icon(
                    self.usage_info.context_window_usage,
                    theme,
                    None,
                ))
                .with_width(font_size)
                .with_height(font_size)
                .finish(),
            )
            .with_child(
                Text::new(
                    format!("{context_usage_pct}%"),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(text_color)
                .finish(),
            )
            .finish();

        rows.push((context_window_label, context_window_value));
        rows
    }

    fn tool_call_summary_rows(&self, appearance: &Appearance) -> Vec<LabelValueRow> {
        let font_size = appearance.ui_font_size() + 2.;
        let theme = appearance.theme();

        let diffs_value = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(8.)
            .with_child(
                Text::new(
                    format!("+{}", self.usage_info.lines_added),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(theme.ansi_fg_green())
                .finish(),
            )
            .with_child(
                Text::new(
                    format!("-{}", self.usage_info.lines_removed),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(theme.ansi_fg_red())
                .finish(),
            )
            .finish();

        vec![
            (
                render_label_text("Tool calls", appearance),
                render_value_text(self.usage_info.tool_calls.to_string(), appearance),
            ),
            (
                render_label_text("Files changed", appearance),
                render_value_text(self.usage_info.files_changed.to_string(), appearance),
            ),
            (render_label_text("Diffs applied", appearance), diffs_value),
            (
                render_label_text("Commands executed", appearance),
                render_value_text(self.usage_info.commands_executed.to_string(), appearance),
            ),
        ]
    }

    fn response_time_rows(&self, appearance: &Appearance) -> Option<Vec<LabelValueRow>> {
        let timing = self.timing_info.as_ref()?;

        let mut rows = vec![
            (
                render_label_text("Time to first token", appearance),
                render_value_text(format_seconds(timing.time_to_first_token_ms), appearance),
            ),
            (
                render_label_text("Total agent response time", appearance),
                render_value_text(
                    format_seconds(timing.total_agent_response_time_ms),
                    appearance,
                ),
            ),
        ];
        if let Some(wall_ms) = timing.wall_to_wall_response_time_ms {
            rows.push((
                render_label_text("Total time (including tool calls)", appearance),
                render_value_text(format_seconds(wall_ms), appearance),
            ));
        }

        Some(rows)
    }
}

impl TurnUsageView {
    /// Builds the panel's two shared label/value columns as flat, parallel
    /// vectors (row `i` in `labels` always corresponds to row `i` in
    /// `values`). Extracted from `render()` so tests can verify row-by-row
    /// alignment without needing a full GUI layout pass.
    ///
    /// Section headers occupy the label column with a *value-column
    /// companion* built via the same [`Self::render_section_header`]
    /// helper (passed an empty label) rather than an `Empty` placeholder.
    /// This matters because `Empty`'s layout height resolves to zero while
    /// the header `Text`'s height reflects real font metrics -- pairing
    /// `Empty` opposite a real header would shift every later row in the
    /// value column up relative to its label, compounding once per section
    /// header. Using the same `Text`-producing helper for both columns
    /// guarantees identical heights regardless of content, matching
    /// `ConversationUsageView::render_section_header`'s established
    /// pattern of pairing a real (if empty) header `Text` in both columns.
    fn build_label_value_columns(&self, appearance: &Appearance) -> LabelValueColumns {
        let mut labels: Vec<Box<dyn Element>> = Vec::new();
        let mut values: Vec<Box<dyn Element>> = Vec::new();
        let mut push_row =
            |label: Box<dyn Element>, value: Box<dyn Element>, margin_bottom: f32| {
                labels.push(
                    Container::new(label)
                        .with_margin_bottom(margin_bottom)
                        .finish(),
                );
                values.push(
                    Container::new(value)
                        .with_margin_bottom(margin_bottom)
                        .finish(),
                );
            };

        push_row(
            Self::render_section_header("MODEL USAGE", appearance),
            Self::render_section_header("", appearance),
            8.,
        );
        for (label, value) in self.model_usage_rows(appearance) {
            push_row(label, value, 6.);
        }

        push_row(
            Self::render_section_header("TOOL CALL SUMMARY", appearance),
            Self::render_section_header("", appearance),
            8.,
        );
        for (label, value) in self.tool_call_summary_rows(appearance) {
            push_row(label, value, 6.);
        }

        if let Some(rows) = self.response_time_rows(appearance) {
            push_row(
                Self::render_section_header("RESPONSE TIME", appearance),
                Self::render_section_header("", appearance),
                8.,
            );
            for (label, value) in rows {
                push_row(label, value, 6.);
            }
        }

        (labels, values)
    }
}

impl View for TurnUsageView {
    fn ui_name() -> &'static str {
        "TurnUsageView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        // All rows across all three sections share one pair of label/value
        // columns, so the value column stays vertically aligned across
        // section boundaries (not just within a single section). See
        // `build_label_value_columns` for how row pairing is maintained.
        let (labels, values) = self.build_label_value_columns(appearance);

        let content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(self.render_header(appearance))
                    .with_margin_bottom(12.)
                    .finish(),
            )
            .with_child(
                Flex::row()
                    .with_spacing(16.)
                    .with_child(Flex::column().with_children(labels).finish())
                    .with_child(Flex::column().with_children(values).finish())
                    .finish(),
            )
            .finish();

        Container::new(content)
            .with_uniform_padding(12.)
            .with_background(theme.surface_2())
            .with_border(Border::all(1.0).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .with_uniform_margin(16.)
            .with_drop_shadow(
                DropShadow::new_with_standard_offset_and_spread(ColorU::new(0, 0, 0, 32))
                    .with_offset(vec2f(0., 2.)),
            )
            .finish()
    }
}

impl Entity for TurnUsageView {
    type Event = TurnUsageViewEvent;
}

impl TypedActionView for TurnUsageView {
    type Action = TurnUsageViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TurnUsageViewAction::Close => {
                ctx.emit(TurnUsageViewEvent::CloseRequested);
            }
        }
    }
}

fn render_label_text(text: &str, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let font_size = appearance.ui_font_size() + 2.;
    Text::new(text.to_string(), appearance.ui_font_family(), font_size)
        .with_color(blended_colors::text_sub(theme, theme.surface_2()))
        .finish()
}

fn render_value_text(text: String, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let font_size = appearance.ui_font_size() + 2.;
    Text::new(text, appearance.ui_font_family(), font_size)
        .with_color(blended_colors::text_main(theme, theme.surface_2()))
        .finish()
}

pub(crate) fn format_tokens(tokens: u64) -> String {
    format!("{tokens} token{}", if tokens == 1 { "" } else { "s" })
}

pub(crate) fn format_dollars(cost_in_cents: f32) -> String {
    format!("${:.2}", cost_in_cents / 100.0)
}

fn format_seconds(ms: i64) -> String {
    format!("{:.1} seconds", ms as f64 / 1000.0)
}

#[cfg(test)]
#[path = "turn_usage_view_tests.rs"]
mod tests;
