//! [`tui_collapsible`]: a disclosure section — a clickable styled header with
//! a chevron over a lazily-built body that shows only when expanded.
//!
//! This is a plain composition of existing primitives: a [`TuiFlex`] column
//! whose first child is the wrapping header text (label spans followed by the
//! disclosure chevron, wrapped in a [`TuiHoverable`] for click and hover
//! tracking) and whose second child — built and present only when expanded —
//! is the body. State is owned by the caller: `collapsed` and the hover state
//! on `mouse_state` are read at composition time and `on_toggle` fires on a
//! header click, leaving the caller to flip its own state and re-render.

use super::{TuiElement, TuiEventContext, TuiFlex, TuiHoverable, TuiStyle, TuiText};
use crate::AppContext;
use crate::elements::MouseStateHandle;

/// Disclosure glyph shown when the section is collapsed.
const CHEVRON_COLLAPSED: &str = "▸";
/// Disclosure glyph shown when the section is expanded.
const CHEVRON_EXPANDED: &str = "▾";

/// Returns the disclosure glyph for a collapsed or expanded section.
fn disclosure_chevron(collapsed: bool) -> &'static str {
    if collapsed {
        CHEVRON_COLLAPSED
    } else {
        CHEVRON_EXPANDED
    }
}
/// Composes a collapsible section: a clickable wrapping rich-text header over
/// a body that is built only when `collapsed` is `false`. The disclosure
/// chevron follows the final label span, so it stays next to the text whether
/// the header occupies one row or wraps onto several. `on_toggle` runs when the
/// header is clicked. Callers own the header styles, including any
/// hover-dependent styling; hover transitions are recorded on `mouse_state`,
/// which the caller owns so it survives re-renders.
pub fn tui_collapsible(
    collapsed: bool,
    header_spans: impl IntoIterator<Item = (String, TuiStyle)>,
    chevron_style: TuiStyle,
    mouse_state: MouseStateHandle,
    body: impl FnOnce() -> Box<dyn TuiElement>,
    on_toggle: impl FnMut(&mut TuiEventContext, &AppContext) + 'static,
) -> Box<dyn TuiElement> {
    let mut header_spans = header_spans.into_iter().collect::<Vec<_>>();
    header_spans.push((format!(" {}", disclosure_chevron(collapsed)), chevron_style));
    let header = TuiText::from_spans(header_spans).finish();
    let header = TuiHoverable::new(mouse_state, header).on_click(on_toggle);

    let mut column = TuiFlex::column().child(header.finish());
    if !collapsed {
        column = column.child(body());
    }
    column.finish()
}

#[cfg(test)]
#[path = "collapsible_tests.rs"]
mod tests;
