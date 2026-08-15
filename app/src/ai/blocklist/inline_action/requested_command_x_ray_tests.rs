//! Unit tests for the agent permission prompt's command x-ray helpers.

use super::{
    CommandXRayHoverState, byte_index_to_char_index, char_index_to_byte_index, token_char_range,
    token_start_byte_offset,
};

#[test]
fn token_start_snaps_to_the_start_of_the_hovered_token() {
    let command = "git commit --amend";

    // Anywhere inside "commit" resolves to the start of "commit".
    for char_index in 4..=9 {
        assert_eq!(
            token_start_byte_offset(command, char_index).as_usize(),
            4,
            "char index {char_index} should snap to the start of `commit`"
        );
    }
}

#[test]
fn token_start_snaps_to_the_start_of_the_first_token() {
    let command = "git commit";

    assert_eq!(token_start_byte_offset(command, 0).as_usize(), 0);
    assert_eq!(token_start_byte_offset(command, 2).as_usize(), 0);
}

#[test]
fn token_start_on_whitespace_snaps_to_the_preceding_token() {
    let command = "git commit";

    // The space between the tokens: the character before it belongs to `git`.
    assert_eq!(token_start_byte_offset(command, 3).as_usize(), 0);
}

#[test]
fn token_start_handles_multibyte_characters() {
    let command = "echo 🚀 --flag";
    let rocket_char_index = 5;
    let rocket_byte_index = 5;
    let flag_char_index = 7;
    let flag_byte_index = "echo 🚀 ".len();

    assert_eq!(
        token_start_byte_offset(command, rocket_char_index).as_usize(),
        rocket_byte_index
    );
    assert_eq!(
        token_start_byte_offset(command, flag_char_index + 2).as_usize(),
        flag_byte_index
    );
}

#[test]
fn token_start_clamps_past_the_end_of_the_command() {
    let command = "ls -la";

    assert_eq!(token_start_byte_offset(command, 999).as_usize(), 3);
}

#[test]
fn char_and_byte_indices_round_trip_across_multibyte_characters() {
    let command = "echo 🚀 done";

    for (byte_index, _) in command.char_indices() {
        let char_index = byte_index_to_char_index(command, byte_index);
        assert_eq!(char_index_to_byte_index(command, char_index), byte_index);
    }
}

#[test]
fn byte_index_inside_a_multibyte_character_snaps_down() {
    let command = "🚀 go";

    // Byte 1 is inside the rocket, which starts at char index 0.
    assert_eq!(byte_index_to_char_index(command, 1), 0);
}

#[test]
fn char_and_byte_index_conversions_clamp_past_the_end() {
    let command = "ls";

    assert_eq!(char_index_to_byte_index(command, 99), command.len());
    assert_eq!(byte_index_to_char_index(command, 99), 2);
}

#[test]
fn token_char_range_converts_a_byte_span_to_characters() {
    let command = "echo 🚀 --flag";
    let flag_byte_start = "echo 🚀 ".len();
    let flag_byte_end = command.len();

    assert_eq!(
        token_char_range(command, flag_byte_start..flag_byte_end),
        7..13
    );
}

#[test]
fn pointer_is_within_the_described_token_only_inside_its_range() {
    let mut state = CommandXRayHoverState::default();
    state.set_described_token_range(Some(4..10));

    state.set_hovered_char_index(Some(3));
    assert!(!state.is_pointer_within_described_token());

    state.set_hovered_char_index(Some(4));
    assert!(state.is_pointer_within_described_token());

    state.set_hovered_char_index(Some(9));
    assert!(state.is_pointer_within_described_token());

    // The end of the range is exclusive, matching the input's token-bounds check.
    state.set_hovered_char_index(Some(10));
    assert!(!state.is_pointer_within_described_token());
}

#[test]
fn pointer_is_not_within_a_token_when_nothing_is_described_or_hovered() {
    let mut state = CommandXRayHoverState::default();

    state.set_hovered_char_index(Some(4));
    assert!(!state.is_pointer_within_described_token());

    state.set_described_token_range(Some(4..10));
    state.set_hovered_char_index(None);
    assert!(!state.is_pointer_within_described_token());
}

#[test]
fn clearing_the_described_token_leaves_no_token_bounds() {
    let mut state = CommandXRayHoverState::default();
    state.set_described_token_range(Some(0..3));
    state.set_hovered_char_index(Some(1));
    assert!(state.is_pointer_within_described_token());

    state.set_described_token_range(None);
    assert!(!state.is_pointer_within_described_token());
}

#[test]
fn dismissing_clears_the_described_token() {
    let mut state = CommandXRayHoverState::default();
    state.set_described_token_range(Some(0..3));
    state.set_hovered_char_index(Some(1));

    state.mark_user_dismissed();
    assert!(!state.is_pointer_within_described_token());
}
