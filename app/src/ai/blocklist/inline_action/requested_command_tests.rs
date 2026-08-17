//! Unit tests for format_command_text in requested_command.rs

use pathfinder_color::ColorU;
use rangemap::RangeMap;
use string_offset::CharOffset;
use warp_core::ui::theme::{AnsiColor, AnsiColors};

use super::{
    format_command_text, header_highlight_ranges, mcp_blocked_title_text,
    mcp_viewing_detail_title_text, shell_highlight_color_ranges,
};

/// Distinct-per-channel `AnsiColors` fixture so tests can assert exact colors without depending
/// on any real theme.
fn test_terminal_colors() -> AnsiColors {
    AnsiColors::new(
        AnsiColor {
            r: 10,
            g: 10,
            b: 10,
        },
        AnsiColor { r: 20, g: 0, b: 0 },
        AnsiColor { r: 0, g: 30, b: 0 },
        AnsiColor { r: 40, g: 40, b: 0 },
        AnsiColor { r: 0, g: 0, b: 50 },
        AnsiColor { r: 60, g: 0, b: 60 },
        AnsiColor { r: 0, g: 70, b: 70 },
        AnsiColor {
            r: 80,
            g: 80,
            b: 80,
        },
    )
}

#[test]
fn single_line_without_newline_is_unchanged_ascii() {
    let input = "echo hello world";
    let output = format_command_text(input);
    assert_eq!(output, input);
}

#[test]
fn single_line_without_newline_preserves_multibyte_characters() {
    let input = "echo 🚀✨";
    let output = format_command_text(input);
    assert_eq!(output, input);

    // Additional sanity check: string is valid UTF-8 and can be iterated by chars without panic
    let collected: String = output.chars().collect();
    assert_eq!(collected, output);
}

#[test]
fn truncates_at_first_newline_and_appends_ellipsis_when_more_content_exists() {
    let input = "cargo build\n--release";
    let output = format_command_text(input);
    assert_eq!(output, "cargo build…");
}

#[test]
fn truncates_at_first_newline_without_ellipsis_when_rest_is_whitespace() {
    let input = "git status\n   \t  ";
    let output = format_command_text(input);
    assert_eq!(output, "git status");
}

#[test]
fn does_not_split_multibyte_char_across_utf8_boundaries_when_newline_follows() {
    // The emoji is a multi-byte sequence; ensure truncation at the newline does not split it.
    let input = "echo 🧪\nthen do something";
    let output = format_command_text(input);
    assert_eq!(output, "echo 🧪…");

    // Validate resulting string is valid UTF-8 by iterating graphemes via chars
    let reconstructed: String = output.chars().collect();
    assert_eq!(reconstructed, output);
}

#[test]
fn preserves_combining_characters_when_newline_is_after_cluster() {
    // "e" + combining acute accent
    // Sanity checks that the formatter doesn't split this unicode sequence
    let composed = format!("{}{}", 'e', '\u{0301}');
    let input = format!("echo {composed}\nnext");
    let output = format_command_text(&input);
    assert_eq!(output, format!("echo {composed}…"));

    // Still valid UTF-8 and same when re-collected from chars
    let reconstructed: String = output.chars().collect();
    assert_eq!(reconstructed, output);
}

#[test]
fn newline_then_multibyte_results_in_ellipsis_only() {
    let input = "\n🚀";
    let output = format_command_text(input);
    assert_eq!(output, "…");

    // Sanity: output remains valid UTF-8
    let reconstructed: String = output.chars().collect();
    assert_eq!(reconstructed, output);
}

#[test]
fn mcp_blocked_title_surfaces_tool_and_server_when_known() {
    assert_eq!(
        mcp_blocked_title_text("create_issue", Some("github")),
        "OK if I call MCP tool create_issue on server github"
    );
}

#[test]
fn mcp_blocked_title_falls_back_to_tool_name_when_server_unknown() {
    assert_eq!(
        mcp_blocked_title_text("create_issue", None),
        "OK if I call MCP tool create_issue"
    );
}

#[test]
fn mcp_blocked_title_falls_back_to_generic_message_when_tool_name_empty() {
    assert_eq!(
        mcp_blocked_title_text("", Some("github")),
        "OK if I call this MCP tool?"
    );
    assert_eq!(
        mcp_blocked_title_text("", None),
        "OK if I call this MCP tool?"
    );
}

#[test]
fn mcp_viewing_detail_title_surfaces_tool_and_server_when_known() {
    assert_eq!(
        mcp_viewing_detail_title_text("create_issue", Some("github")),
        "Viewing MCP tool create_issue on github"
    );
    assert_eq!(
        mcp_viewing_detail_title_text("create_issue", None),
        "Viewing MCP tool create_issue"
    );
}

#[test]
fn mcp_viewing_detail_title_falls_back_to_generic_message_when_tool_name_empty() {
    assert_eq!(
        mcp_viewing_detail_title_text("", Some("github")),
        "Viewing MCP tool call detail"
    );
}

#[test]
fn shell_highlight_color_ranges_colors_the_command_name() {
    let colors = test_terminal_colors();
    let ranges = shell_highlight_color_ranges("echo hello", &colors);

    let (range, color) = ranges
        .iter()
        .next()
        .expect("expected the command name to be highlighted");
    assert_eq!(*range, CharOffset::from(0)..CharOffset::from(4));
    assert_eq!(*color, ColorU::from(colors.blue));
}

#[test]
fn shell_highlight_color_ranges_colors_quoted_strings() {
    let colors = test_terminal_colors();
    let ranges = shell_highlight_color_ranges("echo \"hello\"", &colors);

    let (range, _) = ranges
        .iter()
        .find(|(_, color)| **color == ColorU::from(colors.green))
        .expect("expected the quoted string to be highlighted green");
    assert_eq!(*range, CharOffset::from(5)..CharOffset::from(12));
}

#[test]
fn shell_highlight_color_ranges_leaves_flags_uncolored() {
    // Unlike the completer's own highlighting (which colors options distinctly, see PR #15171),
    // the bundled bash grammar's highlights query captures flags as `@constant`, which has no
    // mapped color in `ansi_syntax_highlighting_color_map`/`convert_capture_name_to_color`.
    let colors = test_terminal_colors();
    let text = "git commit -m \"fix\"";
    let ranges = shell_highlight_color_ranges(text, &colors);

    let flag_start = CharOffset::from(text.find("-m").expect("fixture contains -m"));
    assert!(
        !ranges.iter().any(|(range, _)| range.contains(&flag_start)),
        "expected -m to remain uncolored, got {ranges:?}"
    );
}

#[test]
fn shell_highlight_color_ranges_is_empty_for_empty_text() {
    let colors = test_terminal_colors();
    assert!(shell_highlight_color_ranges("", &colors).is_empty());
}

#[test]
fn header_highlight_ranges_passes_through_when_untruncated() {
    let mut color_ranges = RangeMap::new();
    color_ranges.insert(CharOffset::from(0)..CharOffset::from(4), ColorU::black());
    let highlighted = header_highlight_ranges(&color_ranges, None);

    assert_eq!(highlighted.len(), 1);
    assert_eq!(highlighted[0].highlight_indices, vec![0, 1, 2, 3]);
}

#[test]
fn header_highlight_ranges_clips_range_straddling_the_truncation_boundary() {
    let mut color_ranges = RangeMap::new();
    color_ranges.insert(CharOffset::from(2)..CharOffset::from(8), ColorU::black());
    let highlighted = header_highlight_ranges(&color_ranges, Some(5));

    assert_eq!(highlighted.len(), 1);
    assert_eq!(highlighted[0].highlight_indices, vec![2, 3, 4]);
}

#[test]
fn header_highlight_ranges_drops_range_entirely_past_the_truncation_boundary() {
    let mut color_ranges = RangeMap::new();
    color_ranges.insert(CharOffset::from(6)..CharOffset::from(9), ColorU::black());
    let highlighted = header_highlight_ranges(&color_ranges, Some(5));

    assert!(highlighted.is_empty());
}
