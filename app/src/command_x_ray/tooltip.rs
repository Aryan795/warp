use std::sync::Arc;

use warp_completer::completer::Description;
use warpui::elements::{
    AnchorPair, Border, ConstrainedBox, Container, CornerRadius, Element, Flex, OffsetPositioning,
    OffsetType, ParentElement, PositionedElementOffsetBounds, PositioningAxis, Radius, Stack,
    XAxisAnchor, YAxisAnchor,
};
use warpui::fonts::Weight;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};

use crate::appearance::Appearance;

/// Where a host wants the tooltip drawn.
///
/// The pixel geometry behind `position_id` is per-host — each host's element caches the top-left
/// of the described token under that id while it paints — but the anchoring itself is not, so
/// every host positions the tooltip the same way.
pub struct CommandXRayTooltipAnchor {
    /// The position-cache id under which the host's element cached the token's origin.
    pub position_id: String,
    /// Vertical anchoring of the tooltip against the token.
    pub y_anchor: AnchorPair<YAxisAnchor>,
    /// Extra vertical offset applied after anchoring.
    pub y_offset: OffsetType,
}

/// Adds the command x-ray tooltip to `stack`, anchored to the described token.
pub fn add_command_x_ray_overlay(
    stack: &mut Stack,
    anchor: CommandXRayTooltipAnchor,
    description: &Arc<Description>,
    appearance: &Appearance,
) {
    stack.add_positioned_overlay_child(
        render_command_token_description(description, appearance),
        OffsetPositioning::from_axes(
            PositioningAxis::relative_to_stack_child(
                anchor.position_id.clone(),
                PositionedElementOffsetBounds::ParentByPosition,
                OffsetType::Pixel(0.),
                AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
            ),
            PositioningAxis::relative_to_stack_child(
                anchor.position_id,
                PositionedElementOffsetBounds::Unbounded,
                anchor.y_offset,
                anchor.y_anchor,
            ),
        ),
    );
}

/// Renders the token description card. This is the whole visual surface of command x-ray, and is
/// pure UI over a [`Description`]: every host renders the identical card.
pub fn render_command_token_description(
    description: &Arc<Description>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    // Append an ellipsis to the description if the token has more characters than the max
    // number of characters that are allowed.
    const MAX_XRAY_LABEL_CHARS: usize = 16;
    const TOKEN_DESCRIPTION_PADDING: f32 = 12.;
    const TOKEN_DESCRIPTION_MARGIN: f32 = 10.;
    const TOKEN_DESCRIPTION_WIDTH: f32 = 240.;
    const TOKEN_LABEL_HORIZONTAL_PADDING: f32 = 8.;
    const TOKEN_LABEL_VERTICAL_PADDING: f32 = 4.;

    let truncated_label = match description
        .token
        .item
        .char_indices()
        .nth(MAX_XRAY_LABEL_CHARS)
    {
        None => description.token.item.clone(),
        Some((byte_index, _)) => format!("{}...", &description.token[..byte_index]),
    };

    let theme = appearance.theme();
    let ui_builder = appearance.ui_builder();

    let mut command_description = Flex::column().with_child(
        Flex::row()
            .with_child(
                Container::new(
                    ui_builder
                        .paragraph(truncated_label)
                        .with_style(UiComponentStyles {
                            font_family_id: Some(appearance.monospace_font_family()),
                            font_color: Some(theme.active_ui_text_color().into()),
                            font_size: Some(appearance.monospace_font_size()),
                            font_weight: Some(Weight::Bold),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_padding_top(2.)
                .finish(),
            )
            .with_child(
                Container::new(
                    ui_builder
                        .paragraph(description.suggestion_type.to_name().to_string())
                        .with_style(UiComponentStyles {
                            font_family_id: Some(appearance.ui_font_family()),
                            font_color: Some(theme.active_ui_text_color().into()),
                            font_size: Some(appearance.monospace_font_size() * 0.75),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_background(theme.outline())
                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                .with_margin_left(TOKEN_DESCRIPTION_MARGIN)
                .with_padding_left(TOKEN_LABEL_HORIZONTAL_PADDING)
                .with_padding_right(TOKEN_LABEL_HORIZONTAL_PADDING)
                .with_padding_top(TOKEN_LABEL_VERTICAL_PADDING)
                .with_padding_bottom(TOKEN_LABEL_VERTICAL_PADDING)
                .finish(),
            )
            .finish(),
    );

    if let Some(description_text) = description.description_text.clone() {
        command_description.add_child(
            Container::new(
                ui_builder
                    .paragraph(description_text)
                    .with_style(UiComponentStyles {
                        font_family_id: Some(appearance.ui_font_family()),
                        font_color: Some(theme.sub_text_color(theme.surface_2()).into()),
                        font_size: Some(appearance.monospace_font_size() * 0.9),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_margin_top(TOKEN_DESCRIPTION_MARGIN)
            .finish(),
        );
    }

    ConstrainedBox::new(
        Container::new(command_description.finish())
            .with_uniform_padding(TOKEN_DESCRIPTION_PADDING)
            .with_margin_bottom(TOKEN_DESCRIPTION_MARGIN)
            .with_border(Border::all(1.).with_border_fill(theme.split_pane_border_color()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_background_color(theme.surface_2().into_solid())
            .finish(),
    )
    .with_width(TOKEN_DESCRIPTION_WIDTH)
    .finish()
}
