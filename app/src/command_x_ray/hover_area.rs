use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;
use warpui::AppContext;
use warpui::elements::{
    AfterLayoutContext, Element, Event, EventContext, LayoutContext, PaintContext, Point,
    SizeConstraint, ZIndex,
};
use warpui::event::DispatchedEvent;

use super::hover::{CommandXRayHover, HoverOutcome, HoverProbe};

/// Resolves the text under the pointer for a host that cannot hit-test from inside this element.
///
/// A host whose text is rendered by a child view has no access to that view's layout here, so it
/// supplies the offset out of band — for the code editor that is the `MouseHovered` event the
/// rich text element already dispatches at [`Location`] granularity. This closure only has to
/// answer the two questions the state machine asks.
///
/// [`Location`]: warp_editor::render::model::Location
pub type ProbeFn = Box<dyn Fn(&AppContext) -> HoverProbe>;

/// What the host should do about the tooltip, dispatched from inside the pointer event.
pub type OnOutcomeFn = Box<dyn Fn(HoverOutcome, &mut EventContext)>;

/// Wraps a host's text surface so the shared hover state machine sees raw pointer moves.
///
/// The terminal input does not need this: its editor element already receives `MouseMoved` and
/// already owns the layout it hit-tests against, so it drives [`CommandXRayHover`] directly. A
/// host that renders its text through a `ChildView` has neither, and this element supplies both
/// halves — the pixel stream and the element bounds — without the host reimplementing any of the
/// hover rules.
pub struct CommandXRayHoverArea {
    child: Box<dyn Element>,
    hover: CommandXRayHover,
    probe: ProbeFn,
    on_outcome: OnOutcomeFn,
    origin: Option<Point>,
    size: Option<Vector2F>,
    child_max_z_index: Option<ZIndex>,
}

impl CommandXRayHoverArea {
    pub fn new(
        child: Box<dyn Element>,
        hover: CommandXRayHover,
        probe: ProbeFn,
        on_outcome: OnOutcomeFn,
    ) -> Self {
        Self {
            child,
            hover,
            probe,
            on_outcome,
            origin: None,
            size: None,
            child_max_z_index: None,
        }
    }

    pub fn finish(self) -> Box<dyn Element> {
        Box::new(self)
    }

    fn bounds(&self) -> Option<RectF> {
        Some(RectF::new(self.origin?.xy(), self.size?))
    }
}

impl Element for CommandXRayHoverArea {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let size = self.child.layout(constraint, ctx, app);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        self.child.paint(origin, ctx, app);
        self.child_max_z_index = Some(ctx.scene.max_active_z_index());
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let handled = self.child.dispatch_event(event, ctx, app);

        let Some(z_index) = self
            .child_max_z_index
            .or_else(|| self.origin.map(|origin| origin.z_index()))
        else {
            return handled;
        };

        // Pointer moves outside the bounds matter as much as those inside: a mouse leave is what
        // resets a pending hover and closes an open tooltip.
        if let Some(Event::MouseMoved { position, .. }) = event.at_z_index(z_index, ctx) {
            let is_in_bounds = self
                .bounds()
                .is_some_and(|bounds| bounds.contains_point(*position));
            let outcome =
                self.hover
                    .on_mouse_moved(*position, is_in_bounds, || (self.probe)(app), ctx);
            (self.on_outcome)(outcome, ctx);
        }

        handled
    }
}
