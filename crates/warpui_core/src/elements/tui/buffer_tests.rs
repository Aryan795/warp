use ratatui::style::{Color, Style};
use ratatui::text::Text;
use ratatui::widgets::Paragraph;

use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiPaintSurface, TuiRect, TuiScreenPosition, TuiSize,
};

fn buffer(width: u16, height: u16) -> TuiBuffer {
    TuiBuffer::empty(TuiRect::new(0, 0, width, height))
}

#[test]
fn to_lines_renders_rows_and_pads_with_blanks() {
    let mut b = buffer(5, 1);
    b.set_string(0, 0, "abc", Style::default());
    assert_eq!(b.to_lines(), vec!["abc  "]);
}

#[test]
fn to_lines_reads_text_written_at_an_offset() {
    let mut b = buffer(5, 2);
    b.set_string(2, 1, "hi", Style::default());
    assert_eq!(b.to_lines(), vec!["     ".to_owned(), "  hi ".to_owned()]);
}

#[test]
fn to_lines_collapses_a_wide_grapheme_trailing_column() {
    let mut b = buffer(4, 1);
    b.set_string(0, 0, "界a", Style::default());

    assert_eq!(b.to_lines(), vec!["界a "]);
    assert_eq!(b[(0, 0)].symbol(), "界");
    assert_eq!(b[(2, 0)].symbol(), "a");
}

#[test]
fn to_lines_keeps_a_combining_grapheme_in_one_cell() {
    let mut b = buffer(4, 1);
    b.set_string(0, 0, "e\u{301}x", Style::default());

    assert_eq!(b.to_lines(), vec!["e\u{301}x  "]);
    assert_eq!(b[(0, 0)].symbol(), "e\u{301}");
    assert_eq!(b[(1, 0)].symbol(), "x");
}

#[test]
fn set_string_drops_a_wide_grapheme_that_would_cross_the_edge() {
    let mut b = buffer(3, 1);
    b.set_string(0, 0, "ab界", Style::default());
    assert_eq!(b.to_lines(), vec!["ab "]);
}

#[test]
fn styled_writes_round_trip_through_cells() {
    let mut b = buffer(2, 1);
    b.set_string(0, 0, "x", Style::default().fg(Color::Red));

    assert_eq!(b[(0, 0)].symbol(), "x");
    assert_eq!(b[(0, 0)].fg, Color::Red);
}

/// Maps negative absolute positions onto a scratch buffer without exposing its origin.
#[test]
fn mapped_surface_writes_negative_absolute_positions() {
    let mut b = buffer(2, 1);
    {
        let mut surface = TuiPaintSurface::mapped(&mut b, TuiScreenPosition::new(-5, -2));
        surface
            .cell_mut(TuiScreenPosition::new(-4, -2))
            .unwrap()
            .set_symbol("x");
    }

    assert_eq!(b.to_lines(), vec![" x"]);
}

/// Rejects positions outside the active buffer instead of clamping them.
#[test]
fn surface_writes_outside_the_mapping_fail_closed() {
    let mut b = buffer(2, 1);
    {
        let mut surface = TuiPaintSurface::mapped(&mut b, TuiScreenPosition::new(-5, -2));
        assert!(surface.cell_mut(TuiScreenPosition::new(-6, -2)).is_none());
        assert!(
            surface
                .cell_mut(TuiScreenPosition::new(i32::MAX, -2))
                .is_none()
        );
    }

    assert_eq!(b.to_lines(), vec!["  "]);
}

/// Drops cell writes that fall outside a clipped surface's window.
#[test]
fn clipped_surface_drops_writes_outside_its_window() {
    let mut b = buffer(3, 3);
    {
        let mut surface = TuiPaintSurface::new(&mut b);
        let mut clipped = surface.clipped(TuiScreenPosition::new(0, 1), TuiSize::new(3, 1));
        for y in 0..3 {
            if let Some(cell) = clipped.cell_mut(TuiScreenPosition::new(0, y)) {
                cell.set_symbol("x");
            }
        }
    }

    assert_eq!(b.to_lines(), vec!["   ", "x  ", "   "]);
}

/// Reports only the rows a clipped surface can paint, so vertical containers
/// can skip children that are entirely outside the window.
#[test]
fn clipped_surface_reports_only_paintable_rows() {
    let mut b = buffer(3, 10);
    let mut surface = TuiPaintSurface::new(&mut b);
    assert_eq!(surface.paintable_rows(), 0..10);

    let clipped = surface.clipped(TuiScreenPosition::new(0, 4), TuiSize::new(3, 2));
    assert_eq!(clipped.paintable_rows(), 4..6);
    assert!(clipped.paints_any_row(5, 9));
    assert!(!clipped.paints_any_row(6, 9));
    assert!(!clipped.paints_any_row(0, 4));
}

/// Clips a widget that only partly overlaps the window, keeping the rows it
/// would have painted above and below the window out of the buffer. This is
/// what lets a tall block paint through a small viewport without a full-height
/// scratch buffer.
#[test]
fn clipped_surface_renders_only_the_visible_rows_of_a_widget() {
    let mut b = buffer(3, 3);
    {
        let mut surface = TuiPaintSurface::new(&mut b);
        let mut clipped = surface.clipped(TuiScreenPosition::new(0, 1), TuiSize::new(3, 1));
        // The widget starts two rows above the buffer, so only its third row
        // ("c") lands inside the one-row clip.
        let paragraph = Paragraph::new(Text::from("a\nb\nc\nd"));
        assert!(clipped.render_widget(
            paragraph,
            TuiScreenPosition::new(0, -1),
            TuiSize::new(3, 4)
        ));
    }

    assert_eq!(b.to_lines(), vec!["   ", "c  ", "   "]);
}

/// Confines a background fill to the clipped window.
#[test]
fn clipped_surface_confines_styling_to_its_window() {
    let mut b = buffer(2, 3);
    {
        let mut surface = TuiPaintSurface::new(&mut b);
        let mut clipped = surface.clipped(TuiScreenPosition::new(0, 1), TuiSize::new(2, 1));
        clipped.set_style(
            TuiScreenPosition::new(0, 0),
            TuiSize::new(2, 3),
            Style::default().bg(Color::Red),
        );
    }

    assert_eq!(b[(0, 0)].bg, Color::Reset);
    assert_eq!(b[(0, 1)].bg, Color::Red);
    assert_eq!(b[(0, 2)].bg, Color::Reset);
}
