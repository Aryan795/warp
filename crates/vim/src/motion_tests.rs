use string_offset::CharOffset;

use super::{HorizontalWrap, motion_destination};
use crate::vim::{
    CharacterMotion, Direction, LineMotion, VimMotion, WordBound, WordMotion, WordType,
};

#[test]
fn line_navigation_zero_caret_dollar() {
    let text = "   echo hello";
    let start = CharOffset::zero();
    assert_eq!(
        motion_destination(
            text,
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
            text,
            at_end,
            &VimMotion::Line(LineMotion::FirstNonWhitespace),
            1,
            HorizontalWrap::StopAtLine
        ),
        CharOffset::from(3)
    );
    assert_eq!(
        motion_destination(
            text,
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
        text,
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
        text,
        CharOffset::from(1),
        &VimMotion::Character(CharacterMotion::WrappingRight),
        3,
        HorizontalWrap::StopAtLine,
    );
    assert_eq!(dest, CharOffset::from(1));
}
