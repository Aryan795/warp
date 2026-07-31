//! The styled cell grid the element tree paints into.
//!
//! This is ratatui's `Buffer` (re-exported as [`TuiBuffer`]) with `Style`
//! re-exported as [`TuiStyle`] and `Cell` re-exported for convenience. Elements
//! paint with the buffer's own grapheme-aware writers (`set_string`,
//! `cell_mut`, `set_style`); the diff/flush to the terminal is the ratatui
//! `Terminal`'s job, wired up by the runtime.
//!
//! [`TuiBufferExt::to_lines`] is the headless assertion hook used throughout the
//! element tests: it renders each row to a `String`, skipping the trailing
//! columns of wide graphemes so every glyph appears exactly once (mirroring how
//! ratatui's own `Buffer` debug output collapses multi-width cells).

use std::cell::Cell as StdCell;
use std::ops::Range;
use std::rc::Rc;

use ratatui::buffer::CellWidth;
pub use ratatui::buffer::{Buffer as TuiBuffer, Cell};
pub use ratatui::style::{Color, Modifier, Style as TuiStyle};
use ratatui::widgets::Widget;

use super::geometry::{TuiPoint, TuiRect, TuiSize};
use super::scene::TuiScreenPosition;

/// A half-open rectangle in absolute screen space, used to bound paint writes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ScreenBounds {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl ScreenBounds {
    /// An empty rectangle that paints nothing.
    const EMPTY: Self = Self {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };

    /// The bounds covered by an element's absolute origin and size.
    fn of(origin: TuiScreenPosition, size: TuiSize) -> Self {
        Self {
            left: origin.x,
            top: origin.y,
            right: origin.x.saturating_add(i32::from(size.width)),
            bottom: origin.y.saturating_add(i32::from(size.height)),
        }
    }

    fn is_empty(self) -> bool {
        self.left >= self.right || self.top >= self.bottom
    }

    /// The overlap with `other`, or [`Self::EMPTY`] when they are disjoint.
    fn intersection(self, other: Self) -> Self {
        let intersection = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        if intersection.is_empty() {
            Self::EMPTY
        } else {
            intersection
        }
    }

    /// Whether `other` lies entirely inside these bounds.
    fn contains(self, other: Self) -> bool {
        other.left >= self.left
            && other.top >= self.top
            && other.right <= self.right
            && other.bottom <= self.bottom
    }

    fn contains_point(self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// Absolute-coordinate paint access to one ratatui buffer.
///
/// A surface can carry a screen-space **clip**: every write is dropped outside
/// it, so an element tree can be painted straight into the destination buffer
/// at a negative offset instead of into a full-size scratch buffer that is then
/// windowed (see [`TuiClipped`](super::TuiClipped)). Clipped surfaces are
/// created with [`clipped`](Self::clipped), and elements that lay children out
/// along the vertical axis can skip children entirely outside
/// [`paintable_rows`](Self::paintable_rows).
pub struct TuiPaintSurface<'a> {
    buffer: &'a mut TuiBuffer,
    screen_origin: TuiScreenPosition,
    buffer_origin: TuiPoint,
    /// Screen-space clip applied on top of the buffer's own extent. `None`
    /// means the buffer's extent is the only bound.
    clip: Option<ScreenBounds>,
    /// Rows of scratch buffer the generic [`render_widget`](Self::render_widget)
    /// fallback has materialized, shared with every surface clipped from this
    /// one. See [`widget_scratch_rows`](Self::widget_scratch_rows).
    scratch_rows: Rc<StdCell<usize>>,
}

impl<'a> TuiPaintSurface<'a> {
    /// Creates an identity-mapped surface over `buffer`.
    pub fn new(buffer: &'a mut TuiBuffer) -> Self {
        let buffer_origin = TuiPoint::new(buffer.area.x, buffer.area.y);
        Self {
            buffer,
            screen_origin: TuiScreenPosition::new(
                i32::from(buffer_origin.x),
                i32::from(buffer_origin.y),
            ),
            buffer_origin,
            clip: None,
            scratch_rows: Rc::default(),
        }
    }

    /// Maps `screen_origin` to the top-left cell of `buffer`.
    pub fn mapped(buffer: &'a mut TuiBuffer, screen_origin: TuiScreenPosition) -> Self {
        Self {
            buffer_origin: TuiPoint::new(buffer.area.x, buffer.area.y),
            buffer,
            screen_origin,
            clip: None,
            scratch_rows: Rc::default(),
        }
    }

    /// Borrows this surface restricted to `origin`/`size`, intersected with any
    /// clip already in force. Writes outside the result are dropped, so a child
    /// can paint through it at an arbitrary (even negative) offset.
    pub fn clipped(&mut self, origin: TuiScreenPosition, size: TuiSize) -> TuiPaintSurface<'_> {
        let requested = ScreenBounds::of(origin, size);
        let clip = match self.clip {
            Some(current) => current.intersection(requested),
            None => requested,
        };
        TuiPaintSurface {
            buffer: self.buffer,
            screen_origin: self.screen_origin,
            buffer_origin: self.buffer_origin,
            clip: Some(clip),
            scratch_rows: self.scratch_rows.clone(),
        }
    }

    /// How many scratch rows the generic widget fallback has materialized
    /// through this surface, or any surface clipped from it.
    ///
    /// That fallback is the one paint path whose cost is not bounded by the
    /// visible window, so this is the signal that an element handed over more
    /// rows than the surface can show. Elements that window their own content
    /// keep it at zero.
    pub fn widget_scratch_rows(&self) -> usize {
        self.scratch_rows.get()
    }

    /// The half-open range of absolute screen rows this surface can paint.
    pub fn paintable_rows(&self) -> Range<i32> {
        let bounds = self.paintable_bounds();
        bounds.top..bounds.bottom
    }

    /// Whether any row of the half-open screen range `top..bottom` is paintable.
    /// Vertical containers use this to skip children that are clipped out
    /// entirely, keeping paint cost proportional to the visible window rather
    /// than to the full height of the content.
    pub fn paints_any_row(&self, top: i32, bottom: i32) -> bool {
        let rows = self.paintable_rows();
        top < rows.end && bottom > rows.start && top < bottom
    }

    /// The half-open range of an element's **own** rows (row 0 being its first)
    /// that this surface can paint, or `None` when it is clipped away entirely.
    ///
    /// An element whose content can be produced from an arbitrary starting row
    /// uses this to render only its visible slice, instead of handing the whole
    /// rect to [`render_widget`](Self::render_widget) and paying for every row
    /// above the window (see [`TuiText`](super::TuiText)).
    pub fn visible_element_rows(
        &self,
        origin: TuiScreenPosition,
        size: TuiSize,
    ) -> Option<Range<u16>> {
        let visible = self
            .paintable_bounds()
            .intersection(ScreenBounds::of(origin, size));
        if visible.is_empty() {
            return None;
        }
        let start = u16::try_from(visible.top.saturating_sub(origin.y)).unwrap_or(u16::MAX);
        let end = u16::try_from(visible.bottom.saturating_sub(origin.y))
            .unwrap_or(u16::MAX)
            .min(size.height);
        (start < end).then_some(start..end)
    }

    /// Renders a ratatui widget within absolute screen bounds.
    ///
    /// A widget that fits entirely inside the paintable bounds is rendered
    /// straight into the destination buffer. One that is only partly visible is
    /// rendered into a scratch buffer covering its own rows down to the last
    /// visible one, and only the visible window is copied out — ratatui widgets
    /// clip their area to the target buffer without skipping leading rows, so
    /// they cannot be rendered directly at a negative offset.
    ///
    /// That fallback costs a row for every row above the window, so an element
    /// able to produce its content from an arbitrary starting row should ask
    /// [`visible_element_rows`](Self::visible_element_rows) and hand over only
    /// its visible slice. [`TuiText`](super::TuiText) does; the fallback remains
    /// for partial *horizontal* clipping and for widgets with no such seam.
    pub fn render_widget(
        &mut self,
        widget: impl Widget,
        origin: TuiScreenPosition,
        size: TuiSize,
    ) -> bool {
        if size.width == 0 || size.height == 0 {
            return false;
        }
        let requested = ScreenBounds::of(origin, size);
        let visible = self.paintable_bounds().intersection(requested);
        if visible.is_empty() {
            return false;
        }
        if let Some(area) = self
            .paintable_bounds()
            .contains(requested)
            .then(|| self.buffer_rect(origin, size))
            .flatten()
        {
            widget.render(area, self.buffer);
            return true;
        }

        // Rows below the visible window are never read back, so the scratch
        // buffer stops at the last visible row.
        let scratch_height = u16::try_from(visible.bottom.saturating_sub(origin.y))
            .unwrap_or(u16::MAX)
            .min(size.height);
        self.scratch_rows
            .set(self.scratch_rows.get() + usize::from(scratch_height));
        let mut scratch = TuiBuffer::empty(TuiRect::new(0, 0, size.width, scratch_height));
        widget.render(scratch.area, &mut scratch);
        for y in visible.top..visible.bottom {
            let scratch_y = u16::try_from(y.saturating_sub(origin.y)).unwrap_or(u16::MAX);
            for x in visible.left..visible.right {
                let scratch_x = u16::try_from(x.saturating_sub(origin.x)).unwrap_or(u16::MAX);
                let Some(cell) = scratch.cell((scratch_x, scratch_y)) else {
                    continue;
                };
                let cell = cell.clone();
                self.set_cell(TuiScreenPosition::new(x, y), cell);
            }
        }
        true
    }

    /// Applies `style` to the visible part of the absolute screen bounds.
    pub fn set_style(&mut self, origin: TuiScreenPosition, size: TuiSize, style: TuiStyle) {
        let visible = self
            .paintable_bounds()
            .intersection(ScreenBounds::of(origin, size));
        if visible.is_empty() {
            return;
        }
        let Some(area) = self.buffer_rect(
            TuiScreenPosition::new(visible.left, visible.top),
            TuiSize::new(
                u16::try_from(visible.right - visible.left).unwrap_or(u16::MAX),
                u16::try_from(visible.bottom - visible.top).unwrap_or(u16::MAX),
            ),
        ) else {
            return;
        };
        let area = area.intersection(self.buffer.area);
        if !area.is_empty() {
            self.buffer.set_style(area, style);
        }
    }

    /// Returns the cell at an absolute screen position. Reads are not clipped:
    /// callers such as selection snapshotting sample cells other elements
    /// painted.
    pub fn cell(&self, position: TuiScreenPosition) -> Option<&Cell> {
        self.buffer_point(position)
            .and_then(|position| self.buffer.cell(position))
    }

    /// Returns the mutable cell at an absolute screen position, or `None` when
    /// the position is outside the buffer or the active clip.
    pub fn cell_mut(&mut self, position: TuiScreenPosition) -> Option<&mut Cell> {
        if !self
            .paintable_bounds()
            .contains_point(position.x, position.y)
        {
            return None;
        }
        self.buffer_point(position)
            .and_then(|position| self.buffer.cell_mut(position))
    }

    /// Replaces the cell at an absolute screen position.
    pub fn set_cell(&mut self, position: TuiScreenPosition, cell: Cell) -> bool {
        let Some(destination) = self.cell_mut(position) else {
            return false;
        };
        *destination = cell;
        true
    }

    /// The buffer's own extent in screen space, narrowed by the active clip.
    fn paintable_bounds(&self) -> ScreenBounds {
        let buffer = ScreenBounds::of(
            self.screen_origin,
            TuiSize::new(self.buffer.area.width, self.buffer.area.height),
        );
        match self.clip {
            Some(clip) => buffer.intersection(clip),
            None => buffer,
        }
    }

    fn buffer_rect(&self, origin: TuiScreenPosition, size: TuiSize) -> Option<TuiRect> {
        let origin = self.buffer_point(origin)?;
        origin.x.checked_add(size.width)?;
        origin.y.checked_add(size.height)?;
        Some(TuiRect::new(origin.x, origin.y, size.width, size.height))
    }

    fn buffer_point(&self, position: TuiScreenPosition) -> Option<TuiPoint> {
        let x = i64::from(self.buffer_origin.x)
            .checked_add(i64::from(position.x).checked_sub(i64::from(self.screen_origin.x))?)?;
        let y = i64::from(self.buffer_origin.y)
            .checked_add(i64::from(position.y).checked_sub(i64::from(self.screen_origin.y))?)?;
        Some(TuiPoint::new(
            u16::try_from(x).ok()?,
            u16::try_from(y).ok()?,
        ))
    }
}

/// Headless rendering of a [`TuiBuffer`] to one `String` per row.
pub trait TuiBufferExt {
    /// Renders the buffer to one `String` per row, emitting each grapheme once
    /// by skipping the trailing columns a wide grapheme occupies.
    fn to_lines(&self) -> Vec<String>;
}

impl TuiBufferExt for TuiBuffer {
    fn to_lines(&self) -> Vec<String> {
        let area = self.area;
        (0..area.height)
            .map(|row| {
                let mut line = String::new();
                let mut skip = 0u16;
                for column in 0..area.width {
                    let cell = &self[(area.x + column, area.y + row)];
                    if skip == 0 {
                        line.push_str(cell.symbol());
                        skip = cell.cell_width().max(1) - 1;
                    } else {
                        skip -= 1;
                    }
                }
                line
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "buffer_tests.rs"]
mod tests;
