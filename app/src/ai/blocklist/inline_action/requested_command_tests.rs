//! Unit tests for format_command_text in requested_command.rs

use pathfinder_color::ColorU;
use string_offset::CharOffset;
use warp_completer::completer::{Description, SuggestionType, TopLevelCommandCaseSensitivity};
use warp_completer::meta::Span;
use warp_completer::{ParsedTokenData, ParsedTokensSnapshot};
use warp_core::ui::theme::{AnsiColor, AnsiColors};

use super::{
    command_highlight_color_ranges, format_command_text, header_highlight_ranges,
    mcp_blocked_title_text, mcp_viewing_detail_title_text,
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

fn parsed_token(
    text: &str,
    span: (usize, usize),
    description: Option<SuggestionType>,
) -> ParsedTokenData {
    let span = Span::new(span.0, span.1);
    ParsedTokenData {
        token: warp_completer::meta::Spanned {
            span,
            item: text.to_string(),
        },
        token_index: 0,
        token_description: description.map(|suggestion_type| Description {
            token: warp_completer::meta::Spanned {
                span,
                item: text.to_string(),
            },
            description_text: None,
            suggestion_type,
        }),
    }
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
fn command_highlight_color_ranges_maps_described_tokens_to_colors() {
    let parsed = ParsedTokensSnapshot {
        buffer_text: "echo hello".to_string(),
        parsed_tokens: vec![
            parsed_token(
                "echo",
                (0, 4),
                Some(SuggestionType::Command(
                    TopLevelCommandCaseSensitivity::CaseSensitive,
                )),
            ),
            // Undescribed tokens (e.g. unrecognized arguments) are left uncolored, matching the
            // terminal input's own behavior.
            parsed_token("hello", (5, 10), None),
        ],
    };

    let colors = test_terminal_colors();
    let ranges = command_highlight_color_ranges(&parsed, &colors);

    assert_eq!(
        ranges,
        vec![(
            CharOffset::from(0)..CharOffset::from(4),
            ColorU::from(colors.green)
        )]
    );
}

#[test]
fn command_highlight_color_ranges_converts_byte_spans_to_char_offsets() {
    // "echo " (5 bytes/chars) + "🚀" (4 bytes, 1 char) + " " (1 byte/char) + "arg" starting at
    // byte 10, char 7.
    let text = "echo 🚀 arg";
    let parsed = ParsedTokensSnapshot {
        buffer_text: text.to_string(),
        parsed_tokens: vec![parsed_token(
            "arg",
            (10, 13),
            Some(SuggestionType::Argument),
        )],
    };

    let colors = test_terminal_colors();
    let ranges = command_highlight_color_ranges(&parsed, &colors);

    assert_eq!(
        ranges,
        vec![(
            CharOffset::from(7)..CharOffset::from(10),
            ColorU::from(colors.cyan)
        )]
    );
}

#[test]
fn header_highlight_ranges_passes_through_when_untruncated() {
    let color_ranges = vec![(CharOffset::from(0)..CharOffset::from(4), ColorU::black())];
    let highlighted = header_highlight_ranges(&color_ranges, None);

    assert_eq!(highlighted.len(), 1);
    assert_eq!(highlighted[0].highlight_indices, vec![0, 1, 2, 3]);
}

#[test]
fn header_highlight_ranges_clips_range_straddling_the_truncation_boundary() {
    let color_ranges = vec![(CharOffset::from(2)..CharOffset::from(8), ColorU::black())];
    let highlighted = header_highlight_ranges(&color_ranges, Some(5));

    assert_eq!(highlighted.len(), 1);
    assert_eq!(highlighted[0].highlight_indices, vec![2, 3, 4]);
}

#[test]
fn header_highlight_ranges_drops_range_entirely_past_the_truncation_boundary() {
    let color_ranges = vec![(CharOffset::from(6)..CharOffset::from(9), ColorU::black())];
    let highlighted = header_highlight_ranges(&color_ranges, Some(5));

    assert!(highlighted.is_empty());
}
