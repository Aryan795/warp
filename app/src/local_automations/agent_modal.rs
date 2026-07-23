//! Two-option agent modal for Local Automations entry points.
//!
//! Shared by the Automations page's "New" button, the Suggested rows, and a
//! row's "Move to cloud" action. The modal (styled after
//! [`crate::terminal::view::init_environment::mode_selector::EnvironmentSetupModeSelector`])
//! offers two ways to run the flow: hand it to a Warp agent conversation, or
//! copy an equivalent prompt for another agent (Claude Code, Codex, ...).
//! Selecting an option dispatches the matching [`WorkspaceAction`] and closes
//! the modal.

use warp_core::ui::color::blend::Blend;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss, Element,
    Empty, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement,
    Radius, Shrinkable, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::{FixedBinding, Keystroke};
use warpui::platform::Cursor;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::appearance::Appearance;
use crate::local_automations::{LocalAutomation, SuggestedAutomation};
use crate::ui_components::icons::Icon;
use crate::workspace::WorkspaceAction;

const MODAL_WIDTH: f32 = 441.;
const DIALOG_CORNER_RADIUS: f32 = 8.;

const HEADER_PADDING_TOP: f32 = 16.;
const HEADER_PADDING_BOTTOM: f32 = 16.;
const HEADER_PADDING_HORIZONTAL: f32 = 24.;

const BODY_PADDING_VERTICAL: f32 = 16.;
const BODY_PADDING_HORIZONTAL: f32 = 20.;

const OPTION_PADDING_VERTICAL: f32 = 8.;
const OPTION_PADDING_HORIZONTAL: f32 = 12.;
const OPTION_CORNER_RADIUS: f32 = 4.;
const OPTION_GAP: f32 = 12.;
const OPTIONS_VERTICAL_GAP: f32 = 8.;

const AVATAR_SIZE: f32 = 48.;
const AVATAR_ICON_SIZE: f32 = 24.;

const TITLE_FONT_SIZE: f32 = 16.;
const OPTION_TITLE_FONT_SIZE: f32 = 14.;
const OPTION_DESC_FONT_SIZE: f32 = 12.;

/// Which automation flow the modal is currently offering agent options for.
#[derive(Debug, Clone)]
pub enum LocalAutomationsAgentModalMode {
    /// The "New" button: create an automation from scratch.
    NewAutomation,
    /// A "Suggested" row: set up the given recipe.
    SetUpSuggestion(SuggestedAutomation),
    /// A row's "Move to cloud" action: recreate the automation as an Oz
    /// cloud schedule.
    MoveToCloud(Box<LocalAutomation>),
}

impl LocalAutomationsAgentModalMode {
    fn title(&self) -> String {
        match self {
            Self::NewAutomation => "Create an automation".to_string(),
            Self::SetUpSuggestion(suggestion) => format!("Set up \"{}\"", suggestion.title()),
            Self::MoveToCloud(automation) => format!("Move \"{}\" to cloud", automation.name),
        }
    }

    fn warp_agent_description(&self) -> &'static str {
        match self {
            Self::NewAutomation | Self::SetUpSuggestion(_) => {
                "Start an agent conversation that sets up the automation with you"
            }
            Self::MoveToCloud(_) => {
                "Start an agent conversation that recreates this automation as an Oz cloud \
                 schedule"
            }
        }
    }

    fn copy_prompt_description(&self) -> &'static str {
        match self {
            Self::NewAutomation | Self::SetUpSuggestion(_) => {
                "Copy a setup prompt to paste into another agent, like Claude Code or Codex"
            }
            Self::MoveToCloud(_) => {
                "Copy a move-to-cloud prompt to paste into another agent, like Claude Code or \
                 Codex"
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum LocalAutomationsAgentModalAction {
    SelectWarpAgent,
    SelectCopyPrompt,
    Dismiss,
    HoveredIn(usize),
    ArrowUp,
    ArrowDown,
    Enter,
}

#[derive(Debug)]
pub enum LocalAutomationsAgentModalEvent {
    /// The modal closed, either by selecting an option (whose
    /// [`WorkspaceAction`] the modal already dispatched) or by dismissal.
    Closed,
}

/// Modal offering "Use Warp Agent" / "Copy agent prompt" for an automation
/// flow. Hidden until [`Self::show`] is called with a mode.
pub struct LocalAutomationsAgentModal {
    mode: Option<LocalAutomationsAgentModalMode>,
    close_button_mouse_state: MouseStateHandle,
    warp_agent_mouse_state: MouseStateHandle,
    copy_prompt_mouse_state: MouseStateHandle,
    selected_option_index: usize,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings(vec![
        FixedBinding::new(
            "escape",
            LocalAutomationsAgentModalAction::Dismiss,
            id!("LocalAutomationsAgentModal"),
        ),
        FixedBinding::new(
            "enter",
            LocalAutomationsAgentModalAction::Enter,
            id!("LocalAutomationsAgentModal"),
        ),
        FixedBinding::new(
            "numpadenter",
            LocalAutomationsAgentModalAction::Enter,
            id!("LocalAutomationsAgentModal"),
        ),
        FixedBinding::new(
            "up",
            LocalAutomationsAgentModalAction::ArrowUp,
            id!("LocalAutomationsAgentModal"),
        ),
        FixedBinding::new(
            "down",
            LocalAutomationsAgentModalAction::ArrowDown,
            id!("LocalAutomationsAgentModal"),
        ),
        FixedBinding::new(
            "tab",
            LocalAutomationsAgentModalAction::ArrowDown,
            id!("LocalAutomationsAgentModal"),
        ),
        FixedBinding::new(
            "shift-tab",
            LocalAutomationsAgentModalAction::ArrowUp,
            id!("LocalAutomationsAgentModal"),
        ),
    ]);
}

impl LocalAutomationsAgentModal {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        Self {
            mode: None,
            close_button_mouse_state: MouseStateHandle::default(),
            warp_agent_mouse_state: MouseStateHandle::default(),
            copy_prompt_mouse_state: MouseStateHandle::default(),
            selected_option_index: 0,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.mode.is_some()
    }

    /// Opens the modal for the given flow and focuses it.
    pub fn show(&mut self, mode: LocalAutomationsAgentModalMode, ctx: &mut ViewContext<Self>) {
        self.mode = Some(mode);
        self.selected_option_index = 0;
        ctx.focus_self();
        ctx.notify();
    }

    /// Hides the modal without emitting [`LocalAutomationsAgentModalEvent`].
    pub fn hide(&mut self, ctx: &mut ViewContext<Self>) {
        if self.mode.take().is_some() {
            ctx.notify();
        }
    }

    fn dismiss(&mut self, ctx: &mut ViewContext<Self>) {
        if self.mode.take().is_some() {
            ctx.emit(LocalAutomationsAgentModalEvent::Closed);
            ctx.notify();
        }
    }

    /// Dispatches the [`WorkspaceAction`] for the current mode and choice,
    /// then closes the modal.
    fn select(&mut self, use_warp_agent: bool, ctx: &mut ViewContext<Self>) {
        let Some(mode) = self.mode.take() else {
            return;
        };
        use LocalAutomationsAgentModalMode::*;
        let action = match (mode, use_warp_agent) {
            (NewAutomation, true) => WorkspaceAction::NewLocalAutomationWithWarpAgent,
            (NewAutomation, false) => WorkspaceAction::CopyLocalAutomationPrompt,
            (SetUpSuggestion(suggestion), true) => {
                WorkspaceAction::NewLocalAutomationFromSuggestion { suggestion }
            }
            (SetUpSuggestion(suggestion), false) => {
                WorkspaceAction::CopyLocalAutomationSuggestionPrompt { suggestion }
            }
            (MoveToCloud(automation), true) => WorkspaceAction::PromoteLocalAutomationToCloud {
                automation: *automation,
            },
            (MoveToCloud(automation), false) => {
                WorkspaceAction::CopyLocalAutomationPromotionPrompt {
                    automation: *automation,
                }
            }
        };
        ctx.dispatch_typed_action(&action);
        ctx.emit(LocalAutomationsAgentModalEvent::Closed);
        ctx.notify();
    }

    fn render_header(&self, title: String, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let title = Text::new(title, appearance.ui_font_family(), TITLE_FONT_SIZE)
            .with_style(Properties::default().weight(Weight::Bold))
            .with_color(theme.active_ui_text_color().into())
            .soft_wrap(true)
            .finish();

        let close_button = appearance
            .ui_builder()
            .close_button(16., self.close_button_mouse_state.clone())
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(LocalAutomationsAgentModalAction::Dismiss);
            })
            .finish();

        let esc_keystroke = Keystroke::parse("escape").expect("escape keystroke parses");
        let esc_pill = appearance
            .ui_builder()
            .keyboard_shortcut(&esc_keystroke)
            .with_style(UiComponentStyles {
                font_size: Some(OPTION_DESC_FONT_SIZE),
                font_color: Some(theme.nonactive_ui_text_color().into_solid()),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
                padding: Some(Coords {
                    top: 0.,
                    bottom: 0.,
                    left: 3.,
                    right: 3.,
                }),
                height: Some(16.),
                ..Default::default()
            })
            .build()
            .finish();

        let right_controls = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Container::new(esc_pill).with_margin_right(8.).finish())
            .with_child(close_button)
            .finish();

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(12.)
            .with_child(Shrinkable::new(1., title).finish())
            .with_child(right_controls)
            .finish()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_option(
        &self,
        index: usize,
        icon: Icon,
        title: &'static str,
        description: &'static str,
        mouse_state: MouseStateHandle,
        action: LocalAutomationsAgentModalAction,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let font_family = appearance.ui_font_family();
        let active_text = theme.active_ui_text_color();
        let nonactive_text = theme.nonactive_ui_text_color();

        let base_background = theme.surface_2();
        let hover_background = base_background.blend(&internal_colors::accent_overlay_1(theme));

        let base_border = internal_colors::neutral_4(theme);
        let hover_border = theme.accent().into_solid();

        // Avatar styling - lighter background for the circular icon container.
        let avatar_background = internal_colors::neutral_2(theme);
        let avatar_border = internal_colors::neutral_3(theme);

        let is_selected = self.selected_option_index == index;
        Hoverable::new(mouse_state, move |state| {
            let is_hovered = state.is_hovered() || state.is_clicked();
            let (background, border_color) = if is_hovered || is_selected {
                (hover_background, hover_border)
            } else {
                (base_background, base_border)
            };

            let avatar_icon = ConstrainedBox::new(icon.to_warpui_icon(nonactive_text).finish())
                .with_width(AVATAR_ICON_SIZE)
                .with_height(AVATAR_ICON_SIZE)
                .finish();

            let avatar_contents = ConstrainedBox::new(
                Flex::row()
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_child(avatar_icon)
                    .finish(),
            )
            .with_width(AVATAR_SIZE)
            .with_height(AVATAR_SIZE)
            .finish();

            let avatar = Container::new(avatar_contents)
                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                .with_background(avatar_background)
                .with_border(Border::all(1.).with_border_color(avatar_border))
                .finish();

            let title_text = Text::new(title.to_string(), font_family, OPTION_TITLE_FONT_SIZE)
                .with_style(Properties::default().weight(Weight::Semibold))
                .with_color(active_text.into())
                .finish();

            let description_text =
                Text::new(description.to_string(), font_family, OPTION_DESC_FONT_SIZE)
                    .with_style(Properties::default().weight(Weight::Normal))
                    .with_color(nonactive_text.into())
                    .soft_wrap(true)
                    .finish();

            let text_content = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(title_text)
                .with_child(
                    Container::new(description_text)
                        .with_margin_top(4.)
                        .finish(),
                )
                .finish();

            Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(OPTION_GAP)
                    .with_child(avatar)
                    .with_child(Shrinkable::new(1., text_content).finish())
                    .finish(),
            )
            .with_padding_left(OPTION_PADDING_HORIZONTAL)
            .with_padding_right(OPTION_PADDING_HORIZONTAL)
            .with_padding_top(OPTION_PADDING_VERTICAL)
            .with_padding_bottom(OPTION_PADDING_VERTICAL)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(OPTION_CORNER_RADIUS)))
            .with_border(Border::all(1.).with_border_color(border_color))
            .with_background(background)
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .additional_on_hover(move |is_hovered, ctx, _app, _pos| {
            if is_hovered {
                ctx.dispatch_typed_action(LocalAutomationsAgentModalAction::HoveredIn(index));
            }
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action.clone());
        })
        .finish()
    }

    fn render_modal(
        &self,
        mode: &LocalAutomationsAgentModalMode,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let header = Container::new(self.render_header(mode.title(), appearance))
            .with_padding_top(HEADER_PADDING_TOP)
            .with_padding_bottom(HEADER_PADDING_BOTTOM)
            .with_padding_left(HEADER_PADDING_HORIZONTAL)
            .with_padding_right(HEADER_PADDING_HORIZONTAL)
            .finish();

        let warp_agent_option = self.render_option(
            0,
            Icon::Warp,
            "Use Warp Agent",
            mode.warp_agent_description(),
            self.warp_agent_mouse_state.clone(),
            LocalAutomationsAgentModalAction::SelectWarpAgent,
            appearance,
        );

        let copy_prompt_option = self.render_option(
            1,
            Icon::Copy,
            "Copy agent prompt",
            mode.copy_prompt_description(),
            self.copy_prompt_mouse_state.clone(),
            LocalAutomationsAgentModalAction::SelectCopyPrompt,
            appearance,
        );

        let options = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(OPTIONS_VERTICAL_GAP)
            .with_child(warp_agent_option)
            .with_child(copy_prompt_option)
            .finish();

        let body = Container::new(options)
            .with_padding_top(BODY_PADDING_VERTICAL)
            .with_padding_bottom(BODY_PADDING_VERTICAL)
            .with_padding_left(BODY_PADDING_HORIZONTAL)
            .with_padding_right(BODY_PADDING_HORIZONTAL)
            .finish();

        let dialog_contents = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(body)
            .finish();

        let dialog_background = theme
            .surface_1()
            .blend(&internal_colors::fg_overlay_1(theme));
        let dialog_border = internal_colors::neutral_4(theme);

        let dialog = Container::new(dialog_contents)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(DIALOG_CORNER_RADIUS)))
            .with_border(Border::all(1.).with_border_color(dialog_border))
            .with_background(dialog_background)
            .finish();

        let constrained_dialog = ConstrainedBox::new(dialog).with_width(MODAL_WIDTH).finish();

        let dismiss_dialog = Dismiss::new(constrained_dialog)
            .prevent_interaction_with_other_elements()
            .on_dismiss(|ctx, _app| {
                ctx.dispatch_typed_action(LocalAutomationsAgentModalAction::Dismiss);
            })
            .finish();

        Container::new(Align::new(dismiss_dialog).finish())
            .with_background(theme.dark_overlay())
            .finish()
    }
}

impl Entity for LocalAutomationsAgentModal {
    type Event = LocalAutomationsAgentModalEvent;
}

impl TypedActionView for LocalAutomationsAgentModal {
    type Action = LocalAutomationsAgentModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            LocalAutomationsAgentModalAction::SelectWarpAgent => {
                self.select(true, ctx);
            }
            LocalAutomationsAgentModalAction::SelectCopyPrompt => {
                self.select(false, ctx);
            }
            LocalAutomationsAgentModalAction::Dismiss => {
                self.dismiss(ctx);
            }
            LocalAutomationsAgentModalAction::HoveredIn(index) => {
                self.selected_option_index = *index;
                ctx.notify();
            }
            LocalAutomationsAgentModalAction::ArrowUp
            | LocalAutomationsAgentModalAction::ArrowDown => {
                self.selected_option_index = if self.selected_option_index == 0 {
                    1
                } else {
                    0
                };
                ctx.notify();
            }
            LocalAutomationsAgentModalAction::Enter => {
                self.select(self.selected_option_index == 0, ctx);
            }
        }
    }
}

impl View for LocalAutomationsAgentModal {
    fn ui_name() -> &'static str {
        "LocalAutomationsAgentModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let Some(mode) = &self.mode else {
            return Empty::new().finish();
        };
        self.render_modal(mode, app)
    }
}
