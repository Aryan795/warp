use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::features::FeatureFlag;
use warp_core::ui::theme::Fill;
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{
    Border, CacheOption, ChildAnchor, ChildView, Clipped, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Empty, Expanded, Flex, Highlight, Image, MainAxisAlignment, MainAxisSize,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::settings_view::{SettingsSection, custom_model_routers_widget_id};
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{
    ActionButton, ActionButtonTheme, ButtonSize, NakedTheme, PrimaryTheme,
};

const MODAL_WIDTH: f32 = 340.;
const HERO_HEIGHT: f32 = 110.;

/// Identifies a single feature announced through the reusable feature-intro
/// popover. The string form ([`FeatureIntroId::as_key`]) is the persisted
/// "seen" key, so it must remain stable across releases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureIntroId {
    CustomModelRouter,
    FactoriesLaunch,
}

impl FeatureIntroId {
    /// The stable key used to record that this feature intro has been seen.
    pub fn as_key(self) -> &'static str {
        match self {
            FeatureIntroId::CustomModelRouter => "custom_model_router",
            FeatureIntroId::FactoriesLaunch => "factories_launch",
        }
    }
}

#[derive(Clone, Copy)]
pub enum FeatureIntroCtaTarget {
    SettingsWidget {
        page: SettingsSection,
        widget_id: fn() -> &'static str,
    },
    /// Opens the Factories launch modal's server-configured booking
    /// destination (see `UserWorkspaces::factories_launch_modal_cta_url`).
    FactoriesLaunchModalBooking,
}

/// A promotional callout (e.g. a limited-time incentive) rendered in its
/// own visually distinct block below a [`FeatureIntro`]'s description,
/// rather than as another line of body copy. `emphasis` must be an exact
/// substring of `text`; it renders with the strongest visual emphasis in
/// the block. See [`FeatureIntroModal::render_offer`].
#[derive(Clone, Copy)]
pub struct FeatureIntroOffer {
    pub text: &'static str,
    pub emphasis: &'static str,
}

/// A data-driven description of a single feature-intro popover. New feature
/// announcements are added by appending an entry to [`FEATURE_INTROS`]; no new
/// view, model, settings, or workspace wiring is required.
pub struct FeatureIntro {
    /// Stable identifier; also the persisted "seen" key.
    pub id: FeatureIntroId,
    /// Bundled hero image shown at the top of the card.
    pub hero_image_path: &'static str,
    /// Optional metadata label rendered above the title (e.g. "NEW").
    pub badge: Option<&'static str>,
    pub title: &'static str,
    /// `\n` breaks onto a new line without a full paragraph-sized gap; see
    /// [`FeatureIntroModal::render_description`].
    pub description: &'static str,
    /// Optional icon rendered to the left of the description.
    pub description_icon: Option<Icon>,
    /// Optional promotional callout rendered in its own visually distinct
    /// block below the description. `None` renders no such block.
    pub offer: Option<FeatureIntroOffer>,
    /// Label for the primary call-to-action button.
    pub cta_label: &'static str,
    /// Destination opened when the user clicks the call-to-action. `None`
    /// simply dismisses the popover.
    pub cta_target: Option<FeatureIntroCtaTarget>,
    /// Additional runtime gate checked immediately before marking this intro
    /// seen, beyond "not yet shown" (e.g. server-driven targeting). An
    /// ineligible intro is skipped without consuming its one-time slot, so it
    /// can still show later once the user becomes eligible.
    pub eligible: fn(&AppContext) -> bool,
    /// Whether showing this intro requires first winning an atomic,
    /// server-side claim (see `AuthClient::claim_feature_intro_impression`).
    /// Used for intros whose one-time impression must be consistent across a
    /// user's devices, rather than merely once per device.
    pub requires_server_claim: bool,
}

/// The registry of feature-intro popovers, in priority order. On startup the
/// first eligible entry whose id has not yet been seen is shown.
pub const FEATURE_INTROS: &[FeatureIntro] = &[
    FeatureIntro {
        id: FeatureIntroId::CustomModelRouter,
        hero_image_path: "async/png/onboarding/custom_model_router_intro_banner.png",
        badge: Some("NEW"),
        title: "Build a custom model router for the Warp Agent.",
        description: "Custom routers can be complexity-based, where tasks are routed based on how difficult they are, or rule-based, where they are routed based on a set of natural language prompts.",
        description_icon: Some(Icon::Compass),
        offer: None,
        cta_label: "Get started",
        cta_target: Some(FeatureIntroCtaTarget::SettingsWidget {
            page: SettingsSection::WarpAgent,
            widget_id: custom_model_routers_widget_id,
        }),
        // This intro has no server-driven targeting of its own, so it reuses the
        // general "has AI enabled at all" gate that used to apply to every intro.
        eligible: |app| crate::settings::AISettings::as_ref(app).is_any_ai_enabled(app),
        requires_server_claim: false,
    },
    FeatureIntro {
        id: FeatureIntroId::FactoriesLaunch,
        hero_image_path: "async/png/onboarding/factories_launch_intro_banner.png",
        badge: Some("NEW"),
        title: "Build your software factory on Warp",
        description: "Open, flexible infrastructure for building cloud software factories around your team. Factories-as-code, any model or harness, with evals and self-improvement built in.",
        description_icon: None,
        offer: Some(FeatureIntroOffer {
            text: "Get hands-on implementation support and up to $10K in Factory usage during Early Access.",
            emphasis: "up to $10K",
        }),
        cta_label: "Get Early Access",
        cta_target: Some(FeatureIntroCtaTarget::FactoriesLaunchModalBooking),
        // Purely server-driven: the feature flag reflects cohort membership, and a
        // validated CTA URL (see `UserWorkspaces::has_validated_factories_launch_modal_cta_url`)
        // ensures the modal never shows before a real booking link is configured.
        eligible: |app| {
            FeatureFlag::FactoriesLaunchModal.is_enabled()
                && crate::workspaces::user_workspaces::UserWorkspaces::as_ref(app)
                    .has_validated_factories_launch_modal_cta_url()
        },
        requires_server_claim: true,
    },
];

/// Looks up a feature-intro descriptor by its id.
pub fn feature_intro_by_id(id: FeatureIntroId) -> Option<&'static FeatureIntro> {
    FEATURE_INTROS.iter().find(|intro| intro.id == id)
}

/// Appends the signed-in user's `email` to `cta_url` as an `id` query
/// parameter, Chili Piper's documented smart parameter for identifying and
/// prefilling a guest on a Round-Robin scheduling link. Leaves `cta_url`
/// unchanged when `email` is `None`, empty (an anonymous user), or when
/// `cta_url` doesn't parse as an absolute URL.
pub fn with_email_id_prefill(cta_url: &str, email: Option<&str>) -> String {
    let Some(email) = email.filter(|email| !email.is_empty()) else {
        return cta_url.to_string();
    };
    let Ok(mut parsed) = url::Url::parse(cta_url) else {
        return cta_url.to_string();
    };
    parsed.query_pairs_mut().append_pair("id", email);
    parsed.to_string()
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;

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

struct CloseButtonTheme;

impl ActionButtonTheme for CloseButtonTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        NakedTheme.background(hovered, appearance)
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        _appearance: &Appearance,
    ) -> ColorU {
        ColorU::black()
    }
}

pub fn init(_app: &mut AppContext) {
    // Escape is registered on Workspace (gated on FEATURE_INTRO_MODAL_OPEN) because this
    // popover intentionally never takes focus, so a FeatureIntroModal-scoped binding would
    // never fire while the terminal keeps focus.
}

#[derive(Clone, Debug)]
pub enum FeatureIntroModalAction {
    Close,
    GetStarted,
}

#[derive(Clone, Debug)]
pub enum FeatureIntroModalEvent {
    /// The user dismissed the popover (close button or escape).
    Close(FeatureIntroId),
    /// The user clicked the primary call-to-action.
    GetStarted(FeatureIntroId),
}

/// A single, reusable popover for introducing new features. The popover is a
/// non-blocking bottom-right overlay (no scrim, does not grab focus); the
/// content is driven entirely by the [`FeatureIntro`] descriptor set via
/// [`FeatureIntroModal::set_feature`].
pub struct FeatureIntroModal {
    close_button: ViewHandle<ActionButton>,
    cta_button: ViewHandle<ActionButton>,
    /// The feature currently being shown, if any.
    current: Option<&'static FeatureIntro>,
}

impl FeatureIntroModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let close_button = ctx.add_view(|_ctx| {
            ActionButton::new("", CloseButtonTheme)
                .with_icon(Icon::X)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| ctx.dispatch_typed_action(FeatureIntroModalAction::Close))
        });

        let cta_button = ctx.add_view(|_ctx| {
            ActionButton::new("Get started", PrimaryTheme)
                .on_click(|ctx| ctx.dispatch_typed_action(FeatureIntroModalAction::GetStarted))
        });

        Self {
            close_button,
            cta_button,
            current: None,
        }
    }

    /// Sets the feature descriptor that the popover should render. Passing
    /// `None` leaves the popover empty (the workspace simply stops rendering it).
    pub fn set_feature(
        &mut self,
        intro: Option<&'static FeatureIntro>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.current = intro;
        if let Some(intro) = intro {
            self.cta_button.update(ctx, |button, ctx| {
                button.set_label(intro.cta_label, ctx);
            });
        }
        ctx.notify();
    }

    fn render_hero(&self, intro: &FeatureIntro) -> Box<dyn Element> {
        let hero = Clipped::new(
            ConstrainedBox::new(
                Image::new(
                    AssetSource::Bundled {
                        path: intro.hero_image_path,
                    },
                    CacheOption::Original,
                )
                .with_corner_radius(CornerRadius::with_top(Radius::Pixels(8.)))
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
                vec2f(-4., 0.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopRight,
                ChildAnchor::TopRight,
            ),
        );
        hero_stack.finish()
    }

    fn render_badge(label: &'static str, appearance: &Appearance) -> Box<dyn Element> {
        Text::new_inline(label.to_string(), appearance.ui_font_family(), 11.)
            .with_color(modal_text_sub(appearance))
            .with_style(Properties::default().weight(Weight::Semibold))
            .finish()
    }

    fn render_title(title: &'static str, appearance: &Appearance) -> Box<dyn Element> {
        Text::new(title, appearance.ui_font_family(), 20.)
            .with_color(modal_text_main(appearance))
            .with_style(Properties::default().weight(Weight::Semibold))
            .finish()
    }

    /// Splits `intro.description` on `\n`, rendering each line as its own
    /// `Text` element in a tightly-spaced column. This gives explicit control
    /// over where a multi-line description breaks, without the larger
    /// vertical gap of a full paragraph break.
    fn render_description(intro: &FeatureIntro, appearance: &Appearance) -> Box<dyn Element> {
        let mut lines = Flex::column().with_spacing(4.);
        for line in intro.description.split('\n') {
            lines.add_child(
                Text::new(line, appearance.ui_font_family(), 14.)
                    .with_color(modal_text_sub(appearance))
                    .finish(),
            );
        }
        let description = lines.finish();

        if let Some(icon) = intro.description_icon {
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(
                    Container::new(
                        ConstrainedBox::new(
                            icon.to_warpui_icon(Fill::Solid(modal_text_sub(appearance)))
                                .finish(),
                        )
                        .with_width(16.)
                        .with_height(16.)
                        .finish(),
                    )
                    .with_margin_top(2.)
                    .with_margin_right(8.)
                    .finish(),
                )
                .with_child(Expanded::new(1., description).finish())
                .finish()
        } else {
            description
        }
    }

    /// Renders `offer` as its own visually distinct block: an accent-tinted,
    /// rounded container (distinguishing it from the plain-text description
    /// above it) with `emphasis` highlighted in bold, accent-colored text as
    /// the strongest emphasis in the line.
    fn render_offer(offer: &FeatureIntroOffer, appearance: &Appearance) -> Box<dyn Element> {
        debug_assert!(
            offer.text.contains(offer.emphasis),
            "FeatureIntroOffer::emphasis must be a substring of its text"
        );

        let mut text = Text::new(offer.text, appearance.ui_font_family(), 14.)
            .with_color(modal_text_main(appearance));
        if let Some(byte_start) = offer.text.find(offer.emphasis) {
            let char_start = offer.text[..byte_start].chars().count();
            let char_count = offer.emphasis.chars().count();
            text = text.with_single_highlight(
                Highlight::new()
                    .with_properties(Properties::default().weight(Weight::Bold))
                    .with_foreground_color(appearance.theme().accent().into_solid()),
                (char_start..char_start + char_count).collect(),
            );
        }

        Container::new(text.finish())
            .with_horizontal_padding(10.)
            .with_vertical_padding(8.)
            .with_background(appearance.theme().accent_overlay())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .finish()
    }

    fn render_body(&self, intro: &FeatureIntro, appearance: &Appearance) -> Box<dyn Element> {
        let mut header = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(8.);
        if let Some(badge) = intro.badge {
            header.add_child(Self::render_badge(badge, appearance));
        }
        header.add_child(Self::render_title(intro.title, appearance));
        header.add_child(Self::render_description(intro, appearance));
        if let Some(offer) = &intro.offer {
            header.add_child(Self::render_offer(offer, appearance));
        }

        // The offer block reads as its own section, so it earns more room before
        // the footer divider than the plain description text does.
        let body_bottom_padding = if intro.offer.is_some() { 20. } else { 16. };
        let body = Container::new(header.finish())
            .with_horizontal_padding(16.)
            .with_padding_top(16.)
            .with_padding_bottom(body_bottom_padding)
            .with_background(modal_background(appearance))
            .finish();
        let footer = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(ChildView::new(&self.cta_button).finish())
                .finish(),
        )
        .with_horizontal_padding(16.)
        .with_vertical_padding(12.)
        .with_background(modal_background(appearance))
        .with_border(Border::top(1.).with_border_fill(appearance.theme().outline()))
        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
        .finish();

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(body)
            .with_child(footer)
            .finish()
    }
}

impl Entity for FeatureIntroModal {
    type Event = FeatureIntroModalEvent;
}

impl View for FeatureIntroModal {
    fn ui_name() -> &'static str {
        "FeatureIntroModal"
    }

    // NOTE: intentionally no `on_focus` override. The popover is non-blocking and
    // must not steal focus from the terminal/input; its buttons work on click.

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let Some(intro) = self.current else {
            return Empty::new().finish();
        };

        ConstrainedBox::new(
            Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(self.render_hero(intro))
                    .with_child(self.render_body(intro, appearance))
                    .finish(),
            )
            .with_background(modal_background(appearance))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_foreground_border(appearance.theme().outline().into_solid())
            .finish(),
        )
        .with_width(MODAL_WIDTH)
        .finish()
    }
}

impl TypedActionView for FeatureIntroModal {
    type Action = FeatureIntroModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        let Some(intro) = self.current else {
            return;
        };
        match action {
            FeatureIntroModalAction::Close => {
                ctx.emit(FeatureIntroModalEvent::Close(intro.id));
            }
            FeatureIntroModalAction::GetStarted => {
                ctx.emit(FeatureIntroModalEvent::GetStarted(intro.id));
            }
        }
    }
}
