//! The docked, closeable "Turn" panel (Surface 3 of the pricing-transparency
//! usage surfaces). Unlike [`super::conversation_usage_view::ConversationUsageView`]
//! (which shows conversation-cumulative totals, optionally alongside a
//! "last response" annotation), every numeric value in this view is scoped
//! to a single agent turn ("block") -- see the turn-scoped getters on
//! `AIConversation` (e.g. `tool_calls_for_last_block`).
//!
//! Per the per-turn-usage-panel spec's resolved decisions, this panel:
//! * is triggered independently from (and has no cross-navigation link to)
//!   the "Conversation" popover (Surface 2);
//! * has collapsible sections (MODEL USAGE / TOOL CALL SUMMARY / RESPONSE
//!   TIME), each with its own chevron, matching Surface 2's popover rather
//!   than the always-expanded treatment shown in the original mockup;
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

/// Turn-scoped usage data backing the "MODEL USAGE" section. All fields are
/// scoped to a single agent turn (block), not the whole conversation.
pub struct TurnUsageInfo {
    /// The active model's display identifier (e.g. `auto (cost-efficient)`).
    pub model_id: String,
    /// Total tokens (across warp/byok/custom-endpoint usage) spent during
    /// this turn.
    pub tokens: u64,
    /// Provider cost incurred during this turn, in US cents. `None` when a
    /// turn-scoped cost cannot be derived (e.g. no server-provided cost
    /// baseline yet), in which case the cost row is omitted rather than
    /// rendered as `$0.00`.
    pub cost_in_cents: Option<f32>,
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
    ToggleModelUsageExpanded,
    ToggleToolCallSummaryExpanded,
    ToggleResponseTimeExpanded,
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
    model_usage_expanded: bool,
    tool_call_summary_expanded: bool,
    response_time_expanded: bool,
    model_usage_toggle_mouse_state: MouseStateHandle,
    tool_call_summary_toggle_mouse_state: MouseStateHandle,
    response_time_toggle_mouse_state: MouseStateHandle,
    close_button_mouse_state: MouseStateHandle,
}

impl TurnUsageView {
    pub fn new(usage_info: TurnUsageInfo, timing_info: Option<TimingInfo>) -> Self {
        Self {
            usage_info,
            timing_info,
            // All three sections start expanded, matching the original
            // mockup's default appearance; the chevrons let the user
            // collapse whichever sections they don't need.
            model_usage_expanded: true,
            tool_call_summary_expanded: true,
            response_time_expanded: true,
            model_usage_toggle_mouse_state: MouseStateHandle::default(),
            tool_call_summary_toggle_mouse_state: MouseStateHandle::default(),
            response_time_toggle_mouse_state: MouseStateHandle::default(),
            close_button_mouse_state: MouseStateHandle::default(),
        }
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        let font_size = appearance.ui_font_size() + 2.;

        let title = Text::new(
            "Turn".to_string(),
            appearance.ui_font_family(),
            font_size,
        )
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
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(title)
            .with_child(close_button)
            .finish()
    }

    fn render_section(
        &self,
        label: &str,
        expanded: bool,
        toggle_mouse_state: MouseStateHandle,
        action: TurnUsageViewAction,
        rows: Vec<(Box<dyn Element>, Box<dyn Element>)>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();

        let chevron_icon = if expanded {
            Icon::ChevronUp
        } else {
            Icon::ChevronDown
        };
        let icon_size = appearance.overline_font_size();
        let icon_color = blended_colors::text_disabled(theme, background);
        let header_row = Hoverable::new(toggle_mouse_state, move |_state| {
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_child(
                    Text::new(
                        label.to_string(),
                        appearance.overline_font_family(),
                        appearance.overline_font_size(),
                    )
                    .with_color(icon_color)
                    .finish(),
                )
                .with_child(
                    ConstrainedBox::new(chevron_icon.to_warpui_icon(icon_color.into()).finish())
                        .with_width(icon_size)
                        .with_height(icon_size)
                        .finish(),
                )
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action.clone());
        })
        .finish();

        let mut section = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(Container::new(header_row).with_margin_bottom(6.).finish());

        if expanded {
            for (label_el, value_el) in rows {
                section = section.with_child(
                    Container::new(
                        Flex::row()
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                            .with_child(label_el)
                            .with_child(value_el)
                            .finish(),
                    )
                    .with_margin_bottom(6.)
                    .finish(),
                );
            }
        }

        section.finish()
    }

    fn render_model_usage_section(&self, appearance: &Appearance) -> Box<dyn Element> {
        let font_size = appearance.ui_font_size() + 2.;
        let theme = appearance.theme();
        let background = theme.surface_2();
        let text_color = blended_colors::text_main(theme, background);
        let label_color = blended_colors::text_sub(theme, background);

        let mut model_value_parts = vec![format_tokens(self.usage_info.tokens)];
        if let Some(cost_in_cents) = self.usage_info.cost_in_cents {
            model_value_parts.push(format_dollars(cost_in_cents));
        }
        let model_label = Text::new(
            self.usage_info.model_id.clone(),
            appearance.ui_font_family(),
            font_size,
        )
        .with_style(warpui::fonts::Properties {
            weight: warpui::fonts::Weight::Medium,
            ..Default::default()
        })
        .with_color(text_color)
        .finish();
        let model_value = Text::new(
            model_value_parts.join("  /  "),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(text_color)
        .finish();

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
                Text::new(
                    format!("{context_usage_pct}%"),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(text_color)
                .finish(),
            )
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
            .finish();

        self.render_section(
            "MODEL USAGE",
            self.model_usage_expanded,
            self.model_usage_toggle_mouse_state.clone(),
            TurnUsageViewAction::ToggleModelUsageExpanded,
            vec![
                (model_label, model_value),
                (context_window_label, context_window_value),
            ],
            appearance,
        )
    }

    fn render_tool_call_summary_section(&self, appearance: &Appearance) -> Box<dyn Element> {
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

        self.render_section(
            "TOOL CALL SUMMARY",
            self.tool_call_summary_expanded,
            self.tool_call_summary_toggle_mouse_state.clone(),
            TurnUsageViewAction::ToggleToolCallSummaryExpanded,
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
            ],
            appearance,
        )
    }

    fn render_response_time_section(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
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

        Some(self.render_section(
            "RESPONSE TIME",
            self.response_time_expanded,
            self.response_time_toggle_mouse_state.clone(),
            TurnUsageViewAction::ToggleResponseTimeExpanded,
            rows,
            appearance,
        ))
    }
}

impl View for TurnUsageView {
    fn ui_name() -> &'static str {
        "TurnUsageView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(self.render_header(appearance))
                    .with_margin_bottom(12.)
                    .finish(),
            )
            .with_child(
                Container::new(self.render_model_usage_section(appearance))
                    .with_margin_bottom(12.)
                    .finish(),
            )
            .with_child(
                Container::new(self.render_tool_call_summary_section(appearance))
                    .with_margin_bottom(12.)
                    .finish(),
            );

        if let Some(response_time_section) = self.render_response_time_section(appearance) {
            content = content.with_child(response_time_section);
        }

        Container::new(content.finish())
            .with_uniform_padding(12.)
            .with_background(theme.surface_2())
            .with_border(Border::all(1.0).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .with_uniform_margin(16.)
            .with_drop_shadow(DropShadow::new_with_standard_offset_and_spread(
                ColorU::new(0, 0, 0, 32),
            ).with_offset(vec2f(0., 2.)))
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
            TurnUsageViewAction::ToggleModelUsageExpanded => {
                self.model_usage_expanded = !self.model_usage_expanded;
                ctx.notify();
            }
            TurnUsageViewAction::ToggleToolCallSummaryExpanded => {
                self.tool_call_summary_expanded = !self.tool_call_summary_expanded;
                ctx.notify();
            }
            TurnUsageViewAction::ToggleResponseTimeExpanded => {
                self.response_time_expanded = !self.response_time_expanded;
                ctx.notify();
            }
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

fn format_tokens(tokens: u64) -> String {
    format!("{tokens} token{}", if tokens == 1 { "" } else { "s" })
}

fn format_dollars(cost_in_cents: f32) -> String {
    format!("${:.2}", cost_in_cents / 100.0)
}

fn format_seconds(ms: i64) -> String {
    format!("{:.1} seconds", ms as f64 / 1000.0)
}

#[cfg(test)]
#[path = "turn_usage_view_tests.rs"]
mod tests;
