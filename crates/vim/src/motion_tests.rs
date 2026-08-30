use std::ops::Range;

use string_offset::CharOffset;

use super::{HorizontalWrap, motion_destination, motion_destination_with_jump};
use crate::vim::{
    BracketChar, CharacterMotion, Direction, FirstNonWhitespaceMotion, LineMotion, VimMotion,
    WordBound, WordMotion, WordType,
};

fn full(s: &str) -> Range<CharOffset> {
    CharOffset::zero()..CharOffset::from(s.chars().count())
}

fn dest(s: &str, valid: Range<CharOffset>, offset: CharOffset, motion: VimMotion) -> CharOffset {
    motion_destination(s, valid, offset, &motion, 1, HorizontalWrap::StopAtLine)
}

#[test]
fn line_navigation_zero_caret_dollar() {
    let text = "   echo hello";
    let start = CharOffset::zero();
    assert_eq!(
        dest(text, full(text), start, VimMotion::Line(LineMotion::End)),
        CharOffset::from(13)
    );
    let at_end = CharOffset::from(12);
    assert_eq!(
        dest(
            text,
            full(text),
            at_end,
            VimMotion::Line(LineMotion::FirstNonWhitespace)
        ),
        CharOffset::from(3)
    );
    assert_eq!(
        dest(text, full(text), at_end, VimMotion::Line(LineMotion::Start)),
        CharOffset::zero()
    );
}

#[test]
fn word_forward_from_start() {
    let text = "echo hello";
    let dest = motion_destination(
        text,
        full(text),
        CharOffset::zero(),
        &VimMotion::Word(WordMotion::new(
            Direction::Forward,
            WordBound::Start,
            WordType::Default,
        )),
        1,
        HorizontalWrap::StopAtLine,
    );
    assert_eq!(dest, CharOffset::from(5));
}

#[test]
fn wrapping_stop_at_line_does_not_cross() {
    let text = "ab\ncd";
    assert_eq!(
        motion_destination(
            text,
            full(text),
            CharOffset::from(1),
            &VimMotion::Character(CharacterMotion::WrappingRight),
            3,
            HorizontalWrap::StopAtLine,
        ),
        CharOffset::from(2)
    );
}

#[test]
fn stop_at_line_forward_reaches_exclusive_end() {
    let text = "ab";
    assert_eq!(
        dest(
            text,
            full(text),
            CharOffset::from(1),
            VimMotion::Character(CharacterMotion::Right),
        ),
        CharOffset::from(2)
    );
}

#[test]
fn first_nonwhitespace_on_whitespace_only_line_is_line_start() {
    for text in ["   ", "\t\t", " \t ", "   \nnext"] {
        assert_eq!(
            dest(
                text,
                full(text),
                CharOffset::from(2),
                VimMotion::Line(LineMotion::FirstNonWhitespace),
            ),
            CharOffset::zero(),
            "text={text:?}"
        );
        assert_eq!(
            dest(
                text,
                full(text),
                CharOffset::from(2),
                VimMotion::FirstNonWhitespace(FirstNonWhitespaceMotion::DownMinusOne),
            ),
            CharOffset::zero(),
            "text={text:?}"
        );
    }
}

#[test]
fn jump_to_first_line_can_land_on_column_zero() {
    let text = "   echo\n  two";
    assert_eq!(
        motion_destination_with_jump(
            text,
            full(text),
            CharOffset::from(10),
            &VimMotion::JumpToFirstLine,
            1,
            HorizontalWrap::StopAtLine,
            false,
        ),
        CharOffset::zero()
    );
    assert_eq!(
        motion_destination_with_jump(
            text,
            full(text),
            CharOffset::from(10),
            &VimMotion::JumpToLine(2),
            1,
            HorizontalWrap::StopAtLine,
            false,
        ),
        CharOffset::from(8)
    );
    assert_eq!(
        motion_destination_with_jump(
            text,
            full(text),
            CharOffset::zero(),
            &VimMotion::JumpToLastLine,
            1,
            HorizontalWrap::StopAtLine,
            true,
        ),
        CharOffset::from(10)
    );
}

#[test]
fn percent_does_not_search_past_newline_for_a_bracket() {
    let text = "plain\n(foo)";
    assert_eq!(
        dest(
            text,
            full(text),
            CharOffset::zero(),
            VimMotion::JumpToMatchingBracket,
        ),
        CharOffset::zero()
    );
}

#[test]
fn one_based_left_at_start_does_not_cross_range_start() {
    let text = "Xhello";
    let valid = CharOffset::from(1)..CharOffset::from(6);
    assert_eq!(
        dest(
            text,
            valid,
            CharOffset::from(1),
            VimMotion::Character(CharacterMotion::Left),
        ),
        CharOffset::from(1)
    );
}

#[test]
fn one_based_jumps_and_word_stay_in_valid_range() {
    let text = "Xecho hello";
    let valid = CharOffset::from(1)..CharOffset::from(text.chars().count());
    assert_eq!(
        dest(
            text,
            valid.clone(),
            CharOffset::from(6),
            VimMotion::JumpToFirstLine,
        ),
        CharOffset::from(1)
    );
    assert_eq!(
        motion_destination(
            text,
            valid.clone(),
            CharOffset::from(1),
            &VimMotion::Word(WordMotion::new(
                Direction::Backward,
                WordBound::Start,
                WordType::Default,
            )),
            1,
            HorizontalWrap::StopAtLine,
        ),
        CharOffset::from(1)
    );
    assert_eq!(
        motion_destination(
            text,
            valid,
            CharOffset::from(1),
            &VimMotion::Word(WordMotion::new(
                Direction::Forward,
                WordBound::Start,
                WordType::Default,
            )),
            1,
            HorizontalWrap::StopAtLine,
        ),
        CharOffset::from(6)
    );
}

#[test]
fn one_based_matching_bracket_and_multibyte() {
    let text = "X(你)";
    let valid = CharOffset::from(1)..CharOffset::from(text.chars().count());
    assert_eq!(
        dest(
            text,
            valid.clone(),
            CharOffset::from(1),
            VimMotion::JumpToMatchingBracket,
        ),
        CharOffset::from(3)
    );
    assert_eq!(
        dest(
            text,
            valid.clone(),
            CharOffset::from(3),
            VimMotion::JumpToMatchingBracket,
        ),
        CharOffset::from(1)
    );
    assert_eq!(
        dest(
            text,
            valid,
            CharOffset::from(1),
            VimMotion::JumpToUnmatchedBracket(BracketChar::try_from('(').unwrap()),
        ),
        CharOffset::from(3)
    );
}

#[test]
fn empty_zero_based_motions_stay_at_start() {
    let text = "";
    assert_eq!(
        dest(
            text,
            full(text),
            CharOffset::zero(),
            VimMotion::Character(CharacterMotion::Left),
        ),
        CharOffset::zero()
    );
    assert_eq!(
        dest(
            text,
            full(text),
            CharOffset::zero(),
            VimMotion::JumpToFirstLine
        ),
        CharOffset::zero()
    );
}

#[test]
fn empty_one_based_range_stays_at_start() {
    let text = "X";
    let valid = CharOffset::from(1)..CharOffset::from(1);
    assert_eq!(
        dest(
            text,
            valid.clone(),
            CharOffset::from(1),
            VimMotion::Character(CharacterMotion::Left),
        ),
        CharOffset::from(1)
    );
    assert_eq!(
        dest(text, valid, CharOffset::from(1), VimMotion::JumpToLastLine),
        CharOffset::from(1)
    );
}

#[test]
fn line_end_exclusive_stops_before_newline() {
    let text = "ab\ncd";
    assert_eq!(
        dest(
            text,
            full(text),
            CharOffset::zero(),
            VimMotion::Line(LineMotion::End)
        ),
        CharOffset::from(2)
    );
}

#[test]
fn contiguous_range_not_starting_at_zero_or_one() {
    let text = "xxhello";
    let valid = CharOffset::from(2)..CharOffset::from(7);
    assert_eq!(
        dest(
            text,
            valid.clone(),
            CharOffset::from(2),
            VimMotion::Character(CharacterMotion::Left),
        ),
        CharOffset::from(2)
    );
    assert_eq!(
        dest(
            text,
            valid.clone(),
            CharOffset::from(4),
            VimMotion::JumpToFirstLine
        ),
        CharOffset::from(2)
    );
    assert_eq!(
        dest(
            text,
            valid,
            CharOffset::from(2),
            VimMotion::Line(LineMotion::End)
        ),
        CharOffset::from(7)
    );
}
