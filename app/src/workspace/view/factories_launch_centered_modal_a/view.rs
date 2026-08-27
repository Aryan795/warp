// Exploration mockup (not shipped): Option A for the Factories launch modal —
// a single-panel centered card with a larger hero and the same copy/offer
// content as the bottom-right feature-intro popover, for visual comparison.
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::Fill;
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{
    Align, CacheOption, ChildAnchor, ChildView, Clipped, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Flex, Highlight, Image, MainAxisSize, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::FixedBinding;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{
    ActionButton, ActionButtonTheme, ButtonSize, PrimaryTheme,
};

const MODAL_WIDTH: f32 = 480.;
const HERO_HEIGHT: f32 = 220.;
const HERO_IMAGE_PATH: &str = "async/png/onboarding/factories_launch_intro_banner.png";
const OFFER_TEXT: &str =
    "Get hands-on implementation support and up to $10K in Factory usage during Early Access.";
const OFFER_EMPHASIS: &str = "up to $10K";

fn modal_background(appearance: &Appearance) -> Fill {
    appearance.theme().surface_3()
}

fn modal_text_main(appearance: &Appearance) -> ColorU {
    appearance
        .theme()
        .main_text_color(modal_background(appearance))
        .into_solid()
}

fn modal_text_sub(appearance: &Appearance) -> ColorU {
    appearance
        .theme()
        .sub_text_color(modal_background(appearance))
        .into_solid()
}

fn modal_overlay_1(appearance: &Appearance) -> Fill {
    appearance.theme().surface_overlay_1()
}

fn modal_terminal_magenta(appearance: &Appearance) -> ColorU {
    appearance.theme().terminal_colors().normal.magenta.into()
}

fn modal_terminal_magenta_overlay_1(appearance: &Appearance) -> ColorU {
    let magenta = appearance.theme().terminal_colors().normal.magenta;
    appearance.theme().ansi_overlay_1(magenta)
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        FactoriesLaunchCenteredModalAAction::Close,
        id!(FactoriesLaunchCenteredModalA::ui_name()),
    )]);
}

#[derive(Clone, Debug)]
pub enum FactoriesLaunchCenteredModalAAction {
    Close,
    GetEarlyAccess,
}

#[derive(Clone, Debug)]
pub enum FactoriesLaunchCenteredModalAEvent {
    Close,
    GetEarlyAccess,
}

struct CloseButtonTheme;

impl ActionButtonTheme for CloseButtonTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        if hovered {
            Some(modal_overlay_1(appearance))
        } else {
            None
        }
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        _appearance: &Appearance,
    ) -> ColorU {
        ColorU::white()
    }
}

pub struct FactoriesLaunchCenteredModalA {
    close_button: ViewHandle<ActionButton>,
    cta_button: ViewHandle<ActionButton>,
}

impl FactoriesLaunchCenteredModalA {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let close_button = ctx.add_view(|_ctx| {
            ActionButton::new("", CloseButtonTheme)
                .with_icon(Icon::X)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(FactoriesLaunchCenteredModalAAction::Close)
                })
        });

        let cta_button = ctx.add_view(|_ctx| {
            ActionButton::new("Get Early Access", PrimaryTheme)
                .with_full_width(true)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(FactoriesLaunchCenteredModalAAction::GetEarlyAccess)
                })
        });

        Self {
            close_button,
            cta_button,
        }
    }

    fn render_hero(&self) -> Box<dyn Element> {
        let hero = Clipped::new(
            ConstrainedBox::new(
                Image::new(
                    AssetSource::Bundled {
                        path: HERO_IMAGE_PATH,
                    },
                    CacheOption::Original,
                )
                .with_corner_radius(CornerRadius::with_top(Radius::Pixels(12.)))
                .cover()
                .top_aligned()
                .finish(),
            )
            .with_width(MODAL_WIDTH)
            .with_height(HERO_HEIGHT)
            .finish(),
        )
        .finish();

        let close_el = Container::new(ChildView::new(&self.close_button).finish())
            .with_uniform_padding(4.)
            .with_padding_right(2.)
            .finish();

        let mut hero_stack = Stack::new();
        hero_stack.add_child(hero);
        hero_stack.add_positioned_child(
            close_el,
            OffsetPositioning::offset_from_parent(
                vec2f(-8., 8.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopRight,
                ChildAnchor::TopRight,
            ),
        );
        hero_stack.finish()
    }

    fn render_badge(appearance: &Appearance) -> Box<dyn Element> {
        let text_color = modal_terminal_magenta(appearance);
        let background_color = modal_terminal_magenta_overlay_1(appearance);
        let text = Text::new_inline("New".to_string(), appearance.ui_font_family(), 14.)
            .with_color(text_color)
            .finish();
        ConstrainedBox::new(
            Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_child(text)
                    .finish(),
            )
            .with_horizontal_padding(8.)
            .with_background(Fill::Solid(background_color))
            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
            .finish(),
        )
        .with_height(24.)
        .finish()
    }

    fn render_title(appearance: &Appearance) -> Box<dyn Element> {
        Text::new(
            "Build your software factory on Warp",
            appearance.ui_font_family(),
            26.,
        )
        .with_color(modal_text_main(appearance))
        .with_style(Properties::default().weight(Weight::Semibold))
        .finish()
    }

    fn render_description(appearance: &Appearance) -> Box<dyn Element> {
        Text::new(
            "Open, flexible infrastructure for building cloud software factories around your team. Factories-as-code, any model or harness, with evals and self-improvement built in.",
            appearance.ui_font_family(),
            15.,
        )
        .with_color(modal_text_sub(appearance))
        .with_line_height_ratio(1.4)
        .finish()
    }

    fn render_offer(appearance: &Appearance) -> Box<dyn Element> {
        let mut text = Text::new(OFFER_TEXT, appearance.ui_font_family(), 15.)
            .with_color(modal_text_main(appearance))
            .with_line_height_ratio(1.4);
        if let Some(byte_start) = OFFER_TEXT.find(OFFER_EMPHASIS) {
            let char_start = OFFER_TEXT[..byte_start].chars().count();
            let char_count = OFFER_EMPHASIS.chars().count();
            text = text.with_single_highlight(
                Highlight::new()
                    .with_properties(Properties::default().weight(Weight::Bold))
                    .with_foreground_color(appearance.theme().accent().into_solid()),
                (char_start..char_start + char_count).collect(),
            );
        }

        Container::new(text.finish())
            .with_uniform_padding(14.)
            .with_background(appearance.theme().accent_overlay())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .finish()
    }

    fn render_body(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(16.)
                .with_child(
                    Flex::column()
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .with_spacing(12.)
                        .with_child(Self::render_badge(appearance))
                        .with_child(Self::render_title(appearance))
                        .finish(),
                )
                .with_child(Self::render_description(appearance))
                .with_child(Self::render_offer(appearance))
                .with_child(
                    Container::new(ChildView::new(&self.cta_button).finish())
                        .with_margin_top(8.)
                        .finish(),
                )
                .finish(),
        )
        .with_horizontal_padding(32.)
        .with_padding_top(28.)
        .with_padding_bottom(32.)
        .with_background(modal_background(appearance))
        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(12.)))
        .finish()
    }
}

impl Entity for FactoriesLaunchCenteredModalA {
    type Event = FactoriesLaunchCenteredModalAEvent;
}

impl View for FactoriesLaunchCenteredModalA {
    fn ui_name() -> &'static str {
        "FactoriesLaunchCenteredModalA"
    }

    fn on_focus(&mut self, _focus_ctx: &warpui::FocusContext, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let card = ConstrainedBox::new(
            Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(self.render_hero())
                    .with_child(self.render_body(appearance))
                    .finish(),
            )
            .with_background(modal_background(appearance))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
            .finish(),
        )
        .with_width(MODAL_WIDTH)
        .finish();

        Container::new(Align::new(card).finish())
            .with_background(Fill::Solid(ColorU::new(97, 97, 97, 255)).with_opacity(50))
            .finish()
    }
}

impl TypedActionView for FactoriesLaunchCenteredModalA {
    type Action = FactoriesLaunchCenteredModalAAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            FactoriesLaunchCenteredModalAAction::Close => {
                ctx.emit(FactoriesLaunchCenteredModalAEvent::Close);
            }
            FactoriesLaunchCenteredModalAAction::GetEarlyAccess => {
                ctx.emit(FactoriesLaunchCenteredModalAEvent::GetEarlyAccess);
            }
        }
    }
}
