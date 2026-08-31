//! Chart colors for the usage popover's stacked bars and row swatches.
//!
//! Colors come from the Figma "Pricing transparency" chart palette
//! (`191:367` / `408:23019`) rather than the app's ANSI palette, so the bars
//! read as data-visualization segments rather than terminal-themed content.

use pathfinder_color::ColorU;

/// The six chart colors, in the order sampled from Figma: magenta, blue,
/// yellow, cyan/lavender, green, red.
///
/// A plain function (rather than a `const` array) because `ColorU::new` is
/// not a `const fn`.
fn chart_palette() -> [ColorU; 6] {
    [
        ColorU::new(0xff, 0x8f, 0xfd, 0xff), // magenta
        ColorU::new(0xa5, 0xd5, 0xfe, 0xff), // blue
        ColorU::new(0xfe, 0xfd, 0xc2, 0xff), // yellow
        ColorU::new(0xd0, 0xd1, 0xfe, 0xff), // cyan / lavender
        ColorU::new(0xb4, 0xfa, 0x72, 0xff), // green
        ColorU::new(0xff, 0x82, 0x72, 0xff), // red
    ]
}

/// Chart color for the row at `index` in a breakdown list, cycling once the
/// palette is exhausted.
///
/// Assigning by position rather than by hashing the row's identity keeps the
/// legend unambiguous: hashing into six buckets collides often enough that two
/// rows in the same bar would frequently share a swatch. Callers must pass a
/// stable index, which every breakdown list here already has since the rows are
/// deterministically sorted before rendering.
pub fn chart_color(index: usize) -> ColorU {
    let palette = chart_palette();
    palette[index % palette.len()]
}
