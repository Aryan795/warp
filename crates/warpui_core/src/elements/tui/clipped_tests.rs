use std::cell::Cell;
use std::rc::Rc;

use super::TuiClipped;
use crate::elements::MouseStateHandle;
use crate::elements::tui::test_support::{dispatch_presented_event, render_to_lines};
use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEvent, TuiFlex, TuiHoverable,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiPoint, TuiRect, TuiScreenPoint,
    TuiScreenPosition, TuiSize, TuiText,
};
use crate::event::ModifiersState;
use crate::presenter::tui::TuiPresenter;
use crate::{App, AppContext, EntityIdMap};

/// A one-row labelled element that counts how many times it was painted, so a
/// test can assert that a clipped column only paints the rows on screen.
struct CountedRow {
    label: String,
    paints: Rc<Cell<usize>>,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl CountedRow {
    fn new(label: impl Into<String>, paints: &Rc<Cell<usize>>) -> Self {
        Self {
            label: label.into(),
            paints: paints.clone(),
            size: None,
            origin: None,
        }
    }
}

impl TuiElement for CountedRow {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        let width = u16::try_from(self.label.chars().count()).unwrap_or(u16::MAX);
        let size = constraint.clamp(TuiSize::new(width, 1));
        self.size = Some(size);
        size
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.origin = Some(ctx.scene_point(origin));
        self.paints.set(self.paints.get() + 1);
        let Some(size) = self.size else {
            return;
        };
        for (column, character) in self.label.chars().take(usize::from(size.width)).enumerate() {
            let position = origin.offset(i32::try_from(column).unwrap_or(i32::MAX), 0);
            if let Some(cell) = surface.cell_mut(position) {
                cell.set_symbol(&character.to_string());
            }
        }
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }
}

/// A tall column of counted rows clipped to a small window at `viewport_origin_y`.
fn counted_column(rows: usize, viewport_origin_y: usize, paints: &Rc<Cell<usize>>) -> TuiClipped {
    let mut column = TuiFlex::column();
    for index in 0..rows {
        column = column.child(CountedRow::new(format!("{index}"), paints).finish());
    }
    TuiClipped::new(column.finish()).with_viewport_origin_y(viewport_origin_y)
}

struct MissingRetainedSize;

impl TuiElement for MissingRetainedSize {
    /// Returns a non-empty layout without retaining it.
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        constraint.clamp(TuiSize::new(1, 1))
    }

    /// Paints nothing.
    fn render(
        &mut self,
        _origin: TuiScreenPosition,
        _surface: &mut TuiPaintSurface<'_>,
        _ctx: &mut TuiPaintContext,
    ) {
    }
}

/// Rejects children that violate the retained-size contract.
#[test]
#[should_panic(expected = "TuiClipped child size must be retained after layout")]
fn render_requires_the_child_to_retain_its_layout_size() {
    render_to_lines(
        TuiClipped::new(MissingRetainedSize.finish()),
        TuiSize::new(1, 1),
    );
}

/// Composes absolute origins through nested scratch surfaces.
#[test]
fn nested_clipping_preserves_the_requested_logical_rows() {
    let inner =
        TuiClipped::new(TuiText::new("a\nb\nc\nd").truncate().finish()).with_viewport_origin_y(1);
    let outer = TuiClipped::new(inner.finish()).with_viewport_origin_y(1);

    assert_eq!(render_to_lines(outer, TuiSize::new(1, 2)), vec!["c", "d"],);
}

#[test]
fn renders_from_the_requested_logical_row() {
    let clipped =
        TuiClipped::new(TuiText::new("a\nb\nc").truncate().finish()).with_viewport_origin_y(1);

    assert_eq!(
        render_to_lines(clipped, TuiSize::new(3, 2)),
        vec!["b  ", "c  "],
    );
}

/// Paint cost tracks the visible window, not the clipped child's full height:
/// a tall column scrolled deep into view paints only the rows on screen.
/// Regression guard for the transcript scroll stall, where a multi-thousand-row
/// agent block was painted in full every frame to show ~40 rows.
#[test]
fn clipped_column_paints_only_the_rows_inside_the_viewport() {
    let paints = Rc::new(Cell::new(0));
    let clipped = counted_column(400, 380, &paints);

    let lines = render_to_lines(clipped, TuiSize::new(3, 3));

    assert_eq!(lines, vec!["380", "381", "382"]);
    assert_eq!(
        paints.get(),
        3,
        "a clipped column must paint one row per visible row, not the whole child"
    );
}

/// Renders `element` through a clipped surface and reports the painted rows
/// alongside the scratch rows the generic widget fallback had to materialize.
fn render_clipped(element: impl TuiElement + 'static, size: TuiSize) -> (Vec<String>, usize) {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut element = element.finish();
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            element.layout(TuiConstraint::loose(size), &mut layout_ctx, app_ctx);
            let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, size.width, size.height));
            let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
            let scratch_rows = {
                let mut surface = TuiPaintSurface::new(&mut buffer);
                element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
                surface.widget_scratch_rows()
            };
            (buffer.to_lines(), scratch_rows)
        })
    })
}

/// A tall text block scrolled deep into the window paints its visible slice
/// without materializing the rows above it, so paint cost tracks the viewport
/// rather than the scroll offset. Handing the whole rect to the generic widget
/// fallback would allocate and render every row from the block's first down to
/// the last visible one.
#[test]
fn deeply_clipped_text_does_not_materialize_the_rows_above_the_window() {
    let lines = (0..400)
        .map(|index| format!("row{index:03}"))
        .collect::<Vec<_>>()
        .join("\n");
    let clipped =
        TuiClipped::new(TuiText::new(lines).truncate().finish()).with_viewport_origin_y(380);

    let (rows, scratch_rows) = render_clipped(clipped, TuiSize::new(6, 2));

    assert_eq!(rows, vec!["row380", "row381"]);
    assert_eq!(
        scratch_rows, 0,
        "a deeply clipped text block must not materialize the rows above its window"
    );
}

/// The same holds for wrapped text, where the visible slice starts partway
/// through a soft-wrapped logical line.
#[test]
fn deeply_clipped_wrapped_text_does_not_materialize_the_rows_above_the_window() {
    // 200 logical lines that each wrap into two rows at width 4.
    let lines = (0..200)
        .map(|index| format!("ab{index:02}cd"))
        .collect::<Vec<_>>()
        .join("\n");
    let clipped = TuiClipped::new(TuiText::new(lines).finish()).with_viewport_origin_y(301);

    let (rows, scratch_rows) = render_clipped(clipped, TuiSize::new(4, 2));

    // Row 301 is the tail of logical line 150 ("ab150cd" wraps to "ab15" then
    // "0cd "), row 302 the head of line 151.
    assert_eq!(rows, vec!["0cd ", "ab15"]);
    assert_eq!(scratch_rows, 0);
}

/// A multi-row child straddling the top of the window keeps painting the rows
/// that are inside it. Painting the child directly into the destination surface
/// means such a child is clipped mid-widget, which must not drop or shift rows.
#[test]
fn clipped_column_renders_children_that_straddle_the_window() {
    let column = TuiFlex::column()
        .child(TuiText::new("a1\na2\na3").truncate().finish())
        .child(TuiText::new("b1\nb2\nb3").truncate().finish());
    let clipped = TuiClipped::new(column.finish()).with_viewport_origin_y(2);

    assert_eq!(
        render_to_lines(clipped, TuiSize::new(2, 3)),
        vec!["a3", "b1", "b2"],
    );
}

/// A child skipped during paint must not be hit-tested against the origin it
/// retained from an earlier frame, so a click inside the viewport can never
/// activate content that scrolled out of it.
#[test]
fn clipped_out_children_do_not_receive_pointer_events() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let offscreen_hits = Rc::new(Cell::new(0));
            let visible_hits = Rc::new(Cell::new(0));
            let offscreen_counter = offscreen_hits.clone();
            let visible_counter = visible_hits.clone();
            let offscreen =
                TuiHoverable::new(MouseStateHandle::default(), TuiText::new("off").finish())
                    .on_click(move |_, _| offscreen_counter.set(offscreen_counter.get() + 1));
            let visible =
                TuiHoverable::new(MouseStateHandle::default(), TuiText::new("vis").finish())
                    .on_click(move |_, _| visible_counter.set(visible_counter.get() + 1));
            let column = TuiFlex::column()
                .child(offscreen.finish())
                .child(visible.finish());
            let clipped = TuiClipped::new(column.finish()).with_viewport_origin_y(1);
            let mut presenter = TuiPresenter::new();
            presenter.present_element(clipped.finish(), TuiRect::new(0, 0, 3, 1), app_ctx);

            // Row 0 of the viewport is the column's *second* child; the first
            // one is clipped above it and was never painted.
            dispatch_presented_event(&mut presenter, &left_mouse_down(1, 0), app_ctx);
            let released = TuiEvent::LeftMouseUp {
                position: TuiPoint::new(1, 0),
                modifiers: ModifiersState::default(),
            };
            dispatch_presented_event(&mut presenter, &released, app_ctx);

            assert_eq!(visible_hits.get(), 1);
            assert_eq!(offscreen_hits.get(), 0);
        });
    });
}

#[test]
fn layout_preserves_child_width_and_reports_visible_height() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut clipped = TuiClipped::new(TuiText::new("a\nb\nc").truncate().finish())
                .with_viewport_origin_y(1);
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };

            let size = clipped.layout(TuiConstraint::loose(TuiSize::new(3, 10)), &mut ctx, app_ctx);

            assert_eq!(size, TuiSize::new(1, 2));
        });
    });
}

struct CursorElement {
    cursor: (u16, u16),
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl TuiElement for CursorElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        let size = constraint.clamp(TuiSize::new(1, 3));
        self.size = Some(size);
        size
    }

    fn render(
        &mut self,
        position: TuiScreenPosition,
        _surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        let origin = ctx.scene_point(position);
        self.origin = Some(origin);
        ctx.set_terminal_cursor(TuiScreenPoint::new(
            origin.x.saturating_add(i32::from(self.cursor.0)),
            origin.y.saturating_add(i32::from(self.cursor.1)),
            origin.z_index,
        ));
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }
}

fn clipped_cursor_frame(cursor: (u16, u16)) -> crate::presenter::tui::TuiFrame {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let clipped = TuiClipped::new(
                CursorElement {
                    cursor,
                    size: None,
                    origin: None,
                }
                .finish(),
            )
            .with_viewport_origin_y(1);
            TuiPresenter::new().present_element(clipped.finish(), TuiRect::new(0, 0, 3, 2), app_ctx)
        })
    })
}

#[test]
fn cursor_position_is_shifted_into_the_visible_window() {
    assert_eq!(clipped_cursor_frame((0, 2)).cursor, Some((0, 1)));
}

#[test]
fn cursor_position_above_the_visible_window_is_hidden() {
    assert_eq!(clipped_cursor_frame((0, 0)).cursor, None);
}

fn left_mouse_down(x: u16, y: u16) -> TuiEvent {
    TuiEvent::LeftMouseDown {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
        click_count: 1,
        is_first_mouse: false,
    }
}

#[test]
fn hoverable_inside_clipped_content_uses_visible_screen_geometry() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let hits = Rc::new(Cell::new(0));
            let counter = hits.clone();
            let hoverable =
                TuiHoverable::new(MouseStateHandle::default(), TuiText::new("hit").finish())
                    .on_click(move |_, _| counter.set(counter.get() + 1));
            let child = TuiFlex::column()
                .child(TuiText::new("hidden").finish())
                .child(hoverable.finish());
            let clipped = TuiClipped::new(child.finish()).with_viewport_origin_y(1);
            let mut presenter = TuiPresenter::new();
            presenter.present_element(clipped.finish(), TuiRect::new(0, 0, 6, 1), app_ctx);

            assert!(dispatch_presented_event(&mut presenter, &left_mouse_down(1, 0), app_ctx).0);
            assert_eq!(hits.get(), 0, "click fires on release");

            let released = TuiEvent::LeftMouseUp {
                position: TuiPoint::new(1, 0),
                modifiers: ModifiersState::default(),
            };
            assert!(dispatch_presented_event(&mut presenter, &released, app_ctx).0);
            assert_eq!(hits.get(), 1);

            assert!(!dispatch_presented_event(&mut presenter, &left_mouse_down(1, 1), app_ctx).0);
        });
    });
}
