use std::cell::Cell;
use std::rc::Rc;

use super::TuiChildView;
use crate::elements::tui::{
    TuiBufferExt, TuiConstraint, TuiElement, TuiLayoutContext, TuiPaintContext, TuiPaintSurface,
    TuiPresentationContext, TuiRect, TuiScreenPosition, TuiSize, TuiText,
};
use crate::presenter::tui::TuiPresenter;
use crate::{App, AppContext, EntityId, EntityIdMap};

/// A fixed-height element that counts its layout calls, so a test can assert
/// how often a view's element tree is measured.
struct MeasuredElement {
    height: u16,
    layouts: Rc<Cell<usize>>,
}

impl TuiElement for MeasuredElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        self.layouts.set(self.layouts.get() + 1);
        constraint.clamp(TuiSize::new(constraint.max.width, self.height))
    }

    fn render(
        &mut self,
        _origin: TuiScreenPosition,
        _surface: &mut TuiPaintSurface<'_>,
        _ctx: &mut TuiPaintContext,
    ) {
    }
}

#[test]
fn embeds_and_renders_the_stub_at_the_given_area() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let view_id = EntityId::from_usize(1);
            let mut presenter = TuiPresenter::new();
            presenter
                .rendered_views
                .insert(view_id, Box::new(TuiText::new("Z")));
            let view = TuiChildView::for_view_id(view_id);
            let frame = presenter.present_element(view.finish(), TuiRect::new(1, 0, 2, 1), app_ctx);
            assert_eq!(frame.buffer.to_lines(), vec![" Z "]);
        });
    });
}

/// Measures the element the presenter already rendered for a view, so callers
/// that need a height ahead of the tree walk do not have to re-render the view
/// into a throwaway tree.
#[test]
fn measure_view_lays_out_the_presenters_rendered_element() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let view_id = EntityId::from_usize(11);
            let layouts = Rc::new(Cell::new(0));
            let mut rendered_views: EntityIdMap<Box<dyn TuiElement>> = EntityIdMap::default();
            rendered_views.insert(
                view_id,
                Box::new(MeasuredElement {
                    height: 7,
                    layouts: layouts.clone(),
                }),
            );
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };

            let measured = ctx.measure_view(
                view_id,
                TuiConstraint::loose(TuiSize::new(20, u16::MAX)),
                app_ctx,
            );

            assert_eq!(measured, Some(TuiSize::new(20, 7)));
            assert_eq!(layouts.get(), 1);
        });
    });
}

/// Reports `None` rather than a zero size when the presenter has not rendered
/// the view, so callers can fall back to their own measurement.
#[test]
fn measure_view_reports_no_size_for_an_unrendered_view() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };

            assert_eq!(
                ctx.measure_view(
                    EntityId::from_usize(12),
                    TuiConstraint::loose(TuiSize::new(20, u16::MAX)),
                    app_ctx,
                ),
                None,
            );
        });
    });
}

#[test]
fn present_records_the_child_as_a_child_of_the_current_view() {
    let root = EntityId::from_usize(7);
    let child = EntityId::from_usize(8);
    let mut rendered_views = EntityIdMap::default();
    let mut parent_by_child = EntityIdMap::default();

    {
        let mut ctx = TuiPresentationContext::new(root, &mut rendered_views, &mut parent_by_child);
        let mut view = TuiChildView::from_rendered(child, Box::new(()), ctx.rendered_views);
        view.present(&mut ctx);
    }

    assert_eq!(parent_by_child.get(&child), Some(&root));
}

#[test]
fn present_nests_grandchildren_under_their_immediate_parent() {
    let root = EntityId::from_usize(1);
    let child = EntityId::from_usize(2);
    let grandchild = EntityId::from_usize(3);
    let mut rendered_views = EntityIdMap::default();
    let mut parent_by_child = EntityIdMap::default();

    {
        let mut ctx = TuiPresentationContext::new(root, &mut rendered_views, &mut parent_by_child);
        // grandchild must be in rendered_views so the nested TuiChildView
        // node can find it during the present pass.
        TuiChildView::from_rendered(grandchild, Box::new(()), ctx.rendered_views);
        // The child's element is a TuiChildView that embeds the grandchild.
        let nested_child_view = Box::new(TuiChildView::for_view_id(grandchild));
        let mut view = TuiChildView::from_rendered(child, nested_child_view, ctx.rendered_views);
        view.present(&mut ctx);
    }

    assert_eq!(parent_by_child.get(&child), Some(&root));
    assert_eq!(parent_by_child.get(&grandchild), Some(&child));
}
