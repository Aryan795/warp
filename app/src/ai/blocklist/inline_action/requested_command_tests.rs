//! Unit tests for format_command_text in requested_command.rs

use super::{
    describable_title_char_len, format_command_text, mcp_blocked_title_text,
    mcp_viewing_detail_title_text,
};

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

/// The whole title is a verbatim prefix of a single-line command, so every character in it can be
/// described.
#[test]
fn whole_single_line_title_is_describable() {
    let input = "echo hello world";
    let title = format_command_text(input);

    assert_eq!(title, input);
    assert_eq!(describable_title_char_len(input), title.chars().count());
}

/// Only the first line of a multi-line command reaches the title, and the ellipsis standing in for
/// the rest is not part of the command, so its index must land outside the describable length.
#[test]
fn multi_line_title_stops_before_the_appended_ellipsis() {
    let input = "cargo build\n--release";
    let describable = describable_title_char_len(input);
    let title = format_command_text(input);

    assert_eq!(title, "cargo build…");
    assert_eq!(describable, "cargo build".chars().count());

    let ellipsis_index = title
        .chars()
        .position(|c| c == '…')
        .expect("an ellipsis is appended when content follows the first line");
    assert!(
        ellipsis_index >= describable,
        "ellipsis at char {ellipsis_index} must not be describable (length {describable})"
    );
}

/// A command whose only remaining content is a trailing newline gets no ellipsis from
/// [`format_command_text`], so the entire rendered title stays describable.
#[test]
fn trailing_newline_only_title_is_fully_describable() {
    let input = "git status\n";
    let title = format_command_text(input);

    assert_eq!(title, "git status");
    assert!(!title.contains('…'));
    assert_eq!(describable_title_char_len(input), title.chars().count());
}

/// The describable length is counted in characters, not bytes, so a multi-byte command must not
/// report the larger byte length — that would let the ellipsis, or text past the first line, be
/// treated as describable.
#[test]
fn describable_length_counts_characters_not_bytes() {
    let first_line = "echo 🚀✨";
    let input = format!("{first_line}\nrm -rf /");
    let describable = describable_title_char_len(&input);
    let title = format_command_text(&input);

    assert_eq!(title, "echo 🚀✨…");
    assert_eq!(describable, 7);
    assert_eq!(describable, first_line.chars().count());
    assert_ne!(describable, first_line.len());

    let ellipsis_index = title
        .chars()
        .position(|c| c == '…')
        .expect("an ellipsis is appended when content follows the first line");
    assert_eq!(ellipsis_index, describable);

    // The same character-counting holds with no newline to truncate at.
    assert_eq!(describable_title_char_len(first_line), 7);
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
fn ellipsis_the_user_typed_stays_describable_when_the_formatter_appends_none() {
    // `format_command_text` appends nothing here, because everything after the newline trims
    // empty — yet the title still ends in an ellipsis, because the command itself does. The
    // describable length must therefore cover that final character: it is the user's, not the
    // formatter's. Deriving the length by looking for a trailing ellipsis in the title cannot
    // tell the two apart and would wrongly hide it.
    let command = "echo …\n   ";
    let title = format_command_text(command);
    assert_eq!(title, "echo …", "formatter appends no ellipsis here");

    let describable = describable_title_char_len(command);
    assert_eq!(describable, title.chars().count());
    assert_eq!(describable, 6);
}
