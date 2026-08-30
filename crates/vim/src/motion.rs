use std::cmp;

use string_offset::CharOffset;
use warpui_core::text::TextBuffer;

use crate::vim::{
    BracketChar, CharacterMotion, Direction, FindCharMotion, LineMotion, VimMotion, WordMotion,
};
use crate::{
    find_next_paragraph_end, find_previous_paragraph_start, vim_find_char_on_line,
    vim_find_matching_bracket, vim_word_iterator_from_offset,
};

/// How wrapping horizontal motions (`space` / backspace) treat newlines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HorizontalWrap {
    /// Never leave the current line (TUI prompt).
    StopAtLine,
    /// Skip newlines without counting them (terminal input).
    SkipNewlines,
    /// Cross onto the next/previous line, landing on empty lines (code editor).
    CrossLine,
}

pub fn motion_destination(
    text: &str,
    offset: CharOffset,
    motion: &VimMotion,
    count: u32,
    wrap: HorizontalWrap,
) -> CharOffset {
    motion_destination_with_jump(text, offset, motion, count, wrap, true)
}

pub fn motion_destination_with_jump(
    text: &str,
    offset: CharOffset,
    motion: &VimMotion,
    count: u32,
    wrap: HorizontalWrap,
    jump_first_nonwhitespace: bool,
) -> CharOffset {
    let offset = clamp_offset(text, offset);
    let jump = |offset: CharOffset| {
        if jump_first_nonwhitespace {
            first_nonwhitespace(text, offset)
        } else {
            line_start(text, offset)
        }
    };
    match motion {
        VimMotion::Character(CharacterMotion::Left) => move_horizontal(
            text,
            offset,
            count,
            Direction::Backward,
            HorizontalWrap::StopAtLine,
        ),
        VimMotion::Character(CharacterMotion::Right) => move_horizontal(
            text,
            offset,
            count,
            Direction::Forward,
            HorizontalWrap::StopAtLine,
        ),
        VimMotion::Character(CharacterMotion::WrappingLeft) => {
            move_horizontal(text, offset, count, Direction::Backward, wrap)
        }
        VimMotion::Character(CharacterMotion::WrappingRight) => {
            move_horizontal(text, offset, count, Direction::Forward, wrap)
        }
        VimMotion::Character(CharacterMotion::Up | CharacterMotion::Down) => offset,
        VimMotion::Word(word_motion) => move_by_word(text, offset, count, word_motion),
        VimMotion::Line(LineMotion::Start) => line_start(text, offset),
        VimMotion::Line(LineMotion::FirstNonWhitespace) => first_nonwhitespace(text, offset),
        VimMotion::Line(LineMotion::End) => line_end_exclusive(text, offset),
        VimMotion::FirstNonWhitespace(_) => first_nonwhitespace(text, offset),
        VimMotion::FindChar(find) => move_to_found_char(text, offset, count, find),
        VimMotion::Paragraph(direction) => move_by_paragraph(text, offset, count, *direction),
        VimMotion::JumpToFirstLine => jump(CharOffset::zero()),
        VimMotion::JumpToLastLine => {
            if text.ends_with('\n') {
                CharOffset::from(char_len(text))
            } else {
                jump(CharOffset::from(char_len(text)))
            }
        }
        VimMotion::JumpToLine(line_number) => jump(jump_to_line_start(text, *line_number)),
        VimMotion::JumpToMatchingBracket => jump_to_matching_bracket(text, offset),
        VimMotion::JumpToUnmatchedBracket(bracket) => {
            vim_find_matching_bracket(text, bracket, offset).unwrap_or(offset)
        }
    }
}

fn char_len(text: &str) -> usize {
    text.chars().count()
}

fn clamp_offset(text: &str, offset: CharOffset) -> CharOffset {
    CharOffset::from(offset.as_usize().min(char_len(text)))
}

fn char_at(text: &str, offset: CharOffset) -> Option<char> {
    text.chars().nth(offset.as_usize())
}

fn line_start(text: &str, offset: CharOffset) -> CharOffset {
    let mut steps = 0;
    let Ok(iter) = text.chars_rev_at(offset) else {
        return offset;
    };
    for c in iter {
        if c == '\n' {
            break;
        }
        steps += 1;
    }
    CharOffset::from(offset.as_usize().saturating_sub(steps))
}

fn line_end_exclusive(text: &str, offset: CharOffset) -> CharOffset {
    let mut steps = 0;
    let Ok(iter) = text.chars_at(offset) else {
        return offset;
    };
    for c in iter {
        if c == '\n' {
            break;
        }
        steps += 1;
    }
    offset + steps
}

fn line_last_char(text: &str, offset: CharOffset) -> CharOffset {
    let start = line_start(text, offset);
    let end = line_end_exclusive(text, offset);
    if end > start { end - 1 } else { start }
}

fn first_nonwhitespace(text: &str, offset: CharOffset) -> CharOffset {
    let start = line_start(text, offset);
    let end = line_end_exclusive(text, offset);
    let Ok(iter) = text.chars_at(start) else {
        return start;
    };
    let mut steps = 0;
    for c in iter.take(end.as_usize().saturating_sub(start.as_usize())) {
        if !c.is_whitespace() || c == '\n' {
            break;
        }
        steps += 1;
    }
    start + steps
}

fn move_horizontal(
    text: &str,
    offset: CharOffset,
    count: u32,
    direction: Direction,
    wrap: HorizontalWrap,
) -> CharOffset {
    match wrap {
        HorizontalWrap::StopAtLine => {
            let start = line_start(text, offset);
            let last = line_last_char(text, offset);
            match direction {
                Direction::Backward => {
                    let dist = u32::min(
                        count,
                        offset.as_usize().saturating_sub(start.as_usize()) as u32,
                    );
                    CharOffset::from(offset.as_usize().saturating_sub(dist as usize))
                }
                Direction::Forward => {
                    let dist = u32::min(
                        count,
                        last.as_usize().saturating_sub(offset.as_usize()) as u32,
                    );
                    cmp::min(last, offset + dist as usize)
                }
            }
        }
        HorizontalWrap::SkipNewlines => move_skipping_newlines(text, offset, count, direction),
        HorizontalWrap::CrossLine => move_crossing_lines(text, offset, count, direction),
    }
}

fn move_skipping_newlines(
    text: &str,
    offset: CharOffset,
    count: u32,
    direction: Direction,
) -> CharOffset {
    let max = CharOffset::from(char_len(text));
    match direction {
        Direction::Backward => {
            let Ok(iter) = text.chars_rev_at(offset) else {
                return offset;
            };
            let mut seen = 0u32;
            let mut steps = 0usize;
            for c in iter {
                steps += 1;
                if c != '\n' {
                    seen += 1;
                    if seen == count {
                        return CharOffset::from(offset.as_usize().saturating_sub(steps));
                    }
                }
            }
            CharOffset::zero()
        }
        Direction::Forward => {
            let Ok(iter) = text.chars_at(offset) else {
                return offset;
            };
            let mut seen = 0u32;
            let mut steps = 0usize;
            for c in iter {
                if c != '\n' {
                    seen += 1;
                }
                steps += 1;
                if seen == count {
                    return cmp::min(max, offset + steps);
                }
            }
            max
        }
    }
}

fn move_crossing_lines(
    text: &str,
    mut offset: CharOffset,
    count: u32,
    direction: Direction,
) -> CharOffset {
    let max = CharOffset::from(char_len(text));
    for _ in 0..count {
        match direction {
            Direction::Forward => {
                if offset >= max {
                    break;
                }
                let next = cmp::min(max, offset + 1);
                if char_at(text, next) == Some('\n') {
                    let after = cmp::min(max, next + 1);
                    offset = if char_at(text, after) == Some('\n') {
                        next
                    } else {
                        after
                    };
                } else {
                    offset = next;
                }
            }
            Direction::Backward => {
                if offset == CharOffset::zero() {
                    break;
                }
                let prev = CharOffset::from(offset.as_usize().saturating_sub(1));
                if char_at(text, prev) == Some('\n') {
                    let prev2 = CharOffset::from(prev.as_usize().saturating_sub(1));
                    offset = if char_at(text, prev2) == Some('\n') {
                        prev
                    } else {
                        prev2
                    };
                } else {
                    offset = prev;
                }
            }
        }
    }
    offset
}

fn move_by_word(
    text: &str,
    offset: CharOffset,
    count: u32,
    word_motion: &WordMotion,
) -> CharOffset {
    let WordMotion {
        direction,
        bound,
        word_type,
    } = word_motion;
    match vim_word_iterator_from_offset(offset, text, *direction, *bound, *word_type) {
        Ok(iter) => iter.take(count as usize).last().unwrap_or(offset),
        Err(_) => offset,
    }
}

fn move_to_found_char(
    text: &str,
    offset: CharOffset,
    occurrence_count: u32,
    motion: &FindCharMotion,
) -> CharOffset {
    let start = line_start(text, offset);
    let end = line_end_exclusive(text, offset);
    let Some(line) = char_slice_owned(text, start, end) else {
        return offset;
    };
    let column = offset.as_usize().saturating_sub(start.as_usize());
    match vim_find_char_on_line(&line, column, motion, occurrence_count, false) {
        Some(new_column) => start + new_column,
        None => offset,
    }
}

fn move_by_paragraph(
    text: &str,
    offset: CharOffset,
    count: u32,
    direction: Direction,
) -> CharOffset {
    let max = CharOffset::from(char_len(text));
    let mut current = offset;
    match direction {
        Direction::Forward => {
            for _ in 0..count {
                current = find_next_paragraph_end(text, current).unwrap_or(max);
            }
        }
        Direction::Backward => {
            for _ in 0..count {
                current =
                    find_previous_paragraph_start(text, current).unwrap_or(CharOffset::zero());
            }
        }
    }
    current
}

fn jump_to_line_start(text: &str, line_number: u32) -> CharOffset {
    let mut start = CharOffset::zero();
    let max = CharOffset::from(char_len(text));
    let target = line_number.max(1);
    for _ in 1..target {
        let end = line_end_exclusive(text, start);
        if end >= max {
            break;
        }
        start = end + 1;
    }
    start
}

fn jump_to_matching_bracket(text: &str, offset: CharOffset) -> CharOffset {
    let end = line_end_exclusive(text, offset);
    let Some(line) = char_slice_owned(text, offset, end) else {
        return offset;
    };
    let mut iter = line.chars();
    let Some(c) = iter.next() else {
        return offset;
    };
    let (bracket, start_offset) = match BracketChar::try_from(c) {
        Ok(bracket) => (bracket, offset),
        Err(()) => match iter
            .enumerate()
            .find_map(|(i, ch)| Some((i, BracketChar::try_from(ch).ok()?)))
        {
            None => return offset,
            Some((i, bracket)) => (bracket, offset + i + 1),
        },
    };
    vim_find_matching_bracket(text, &bracket, start_offset).unwrap_or(offset)
}

fn char_slice_owned(text: &str, start: CharOffset, end: CharOffset) -> Option<String> {
    let s = start.as_usize();
    let e = end.as_usize();
    if e < s {
        return None;
    }
    Some(text.chars().skip(s).take(e.saturating_sub(s)).collect())
}

#[cfg(test)]
#[path = "motion_tests.rs"]
mod tests;
