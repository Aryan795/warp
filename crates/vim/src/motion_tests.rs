use string_offset::CharOffset;

use super::{HorizontalWrap, OffsetText, motion_destination, motion_destination_with_jump};
use crate::vim::{
    CharacterMotion, Direction, FirstNonWhitespaceMotion, LineMotion, VimMotion, WordBound,
    WordMotion, WordType,
};

fn as_text(s: &str) -> OffsetText<'_, str> {
    OffsetText::new(s, CharOffset::zero(), s.chars().count())
}

#[test]
fn line_navigation_zero_caret_dollar() {
    let text = "   echo hello";
    let start = CharOffset::zero();
    assert_eq!(
        motion_destination(
            &as_text(text),
            start,
            &VimMotion::Line(LineMotion::End),
            1,
            HorizontalWrap::StopAtLine
        ),
        CharOffset::from(13)
    );
    let at_end = CharOffset::from(12);
    assert_eq!(
        motion_destination(
            &as_text(text),
            at_end,
            &VimMotion::Line(LineMotion::FirstNonWhitespace),
            1,
            HorizontalWrap::StopAtLine
        ),
        CharOffset::from(3)
    );
    assert_eq!(
        motion_destination(
            &as_text(text),
            at_end,
            &VimMotion::Line(LineMotion::Start),
            1,
            HorizontalWrap::StopAtLine
        ),
        CharOffset::zero()
    );
}

#[test]
fn word_forward_from_start() {
    let text = "echo hello";
    let dest = motion_destination(
        &as_text(text),
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
    let dest = motion_destination(
        &as_text(text),
        CharOffset::from(1),
        &VimMotion::Character(CharacterMotion::WrappingRight),
        3,
        HorizontalWrap::StopAtLine,
    );
    assert_eq!(dest, CharOffset::from(2));
}

#[test]
fn stop_at_line_forward_reaches_exclusive_end() {
    let text = "ab";
    let dest = motion_destination(
        &as_text(text),
        CharOffset::from(1),
        &VimMotion::Character(CharacterMotion::Right),
        1,
        HorizontalWrap::StopAtLine,
    );
    assert_eq!(dest, CharOffset::from(2));
}

#[test]
fn first_nonwhitespace_on_whitespace_only_line_is_line_start() {
    for text in ["   ", "\t\t", " \t ", "   \nnext"] {
        let dest = motion_destination(
            &as_text(text),
            CharOffset::from(2),
            &VimMotion::Line(LineMotion::FirstNonWhitespace),
            1,
            HorizontalWrap::StopAtLine,
        );
        assert_eq!(dest, CharOffset::zero(), "text={text:?}");
        let dest = motion_destination(
            &as_text(text),
            CharOffset::from(2),
            &VimMotion::FirstNonWhitespace(FirstNonWhitespaceMotion::DownMinusOne),
            1,
            HorizontalWrap::StopAtLine,
        );
        assert_eq!(dest, CharOffset::zero(), "text={text:?}");
    }
}

#[test]
fn jump_to_first_line_can_land_on_column_zero() {
    let text = "   echo\n  two";
    assert_eq!(
        motion_destination_with_jump(
            &as_text(text),
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
            &as_text(text),
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
            &as_text(text),
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
    let dest = motion_destination(
        &as_text(text),
        CharOffset::zero(),
        &VimMotion::JumpToMatchingBracket,
        1,
        HorizontalWrap::StopAtLine,
    );
    assert_eq!(dest, CharOffset::zero());
}
