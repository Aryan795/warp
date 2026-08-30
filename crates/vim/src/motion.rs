use std::cmp;

use anyhow::anyhow;
use string_offset::CharOffset;
use warpui_core::text::TextBuffer;
use warpui_core::text::point::Point;

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

/// Character-offset text used by shared vim motions. Implementations must not copy the document.
pub trait VimText {
    fn char_len(&self) -> usize;
    fn chars_at(&self, offset: CharOffset) -> Box<dyn Iterator<Item = char> + '_>;
    fn chars_rev_at(&self, offset: CharOffset) -> Box<dyn Iterator<Item = char> + '_>;
}

/// Zero-based view over a [`TextBuffer`], optionally skipping a native origin (e.g. 1-based editors).
pub struct OffsetText<'a, B: TextBuffer + ?Sized> {
    buffer: &'a B,
    origin: CharOffset,
    len: usize,
}

impl<'a, B: TextBuffer + ?Sized> OffsetText<'a, B> {
    pub fn new(buffer: &'a B, origin: CharOffset, len: usize) -> Self {
        Self {
            buffer,
            origin,
            len,
        }
    }
}

impl<'a, B: TextBuffer + ?Sized> VimText for OffsetText<'a, B> {
    fn char_len(&self) -> usize {
        self.len
    }

    fn chars_at(&self, offset: CharOffset) -> Box<dyn Iterator<Item = char> + '_> {
        let remaining = self.len.saturating_sub(offset.as_usize());
        let native = CharOffset::from(self.origin.as_usize().saturating_add(offset.as_usize()));
        match self.buffer.chars_at(native) {
            Ok(iter) => Box::new(iter.take(remaining)),
            Err(_) => Box::new(std::iter::empty()),
        }
    }

    fn chars_rev_at(&self, offset: CharOffset) -> Box<dyn Iterator<Item = char> + '_> {
        let native = CharOffset::from(self.origin.as_usize().saturating_add(offset.as_usize()));
        match self.buffer.chars_rev_at(native) {
            Ok(iter) => Box::new(iter.take(offset.as_usize())),
            Err(_) => Box::new(std::iter::empty()),
        }
    }
}

impl VimText for str {
    fn char_len(&self) -> usize {
        self.chars().count()
    }

    fn chars_at(&self, offset: CharOffset) -> Box<dyn Iterator<Item = char> + '_> {
        Box::new(self.chars().skip(offset.as_usize()))
    }

    fn chars_rev_at(&self, offset: CharOffset) -> Box<dyn Iterator<Item = char> + '_> {
        let n = self.chars().count();
        Box::new(self.chars().rev().skip(n.saturating_sub(offset.as_usize())))
    }
}

struct DynBuf<'a>(&'a dyn VimText);

impl TextBuffer for DynBuf<'_> {
    type Chars<'b>
        = Box<dyn Iterator<Item = char> + 'b>
    where
        Self: 'b;
    type CharsReverse<'b>
        = Box<dyn Iterator<Item = char> + 'b>
    where
        Self: 'b;

    fn chars_at(&self, offset: CharOffset) -> anyhow::Result<Self::Chars<'_>> {
        Ok(self.0.chars_at(offset))
    }

    fn chars_rev_at(&self, offset: CharOffset) -> anyhow::Result<Self::CharsReverse<'_>> {
        Ok(self.0.chars_rev_at(offset))
    }

    fn to_point(&self, offset: CharOffset) -> anyhow::Result<Point> {
        Ok(Point::new(0, offset.as_usize() as u32))
    }

    fn to_offset(&self, point: Point) -> anyhow::Result<CharOffset> {
        if point.row == 0 {
            Ok(CharOffset::from(point.column as usize))
        } else {
            Err(anyhow!("vim motion text is addressed by character offset"))
        }
    }
}

pub fn motion_destination(
    text: &dyn VimText,
    offset: CharOffset,
    motion: &VimMotion,
    count: u32,
    wrap: HorizontalWrap,
) -> CharOffset {
    motion_destination_with_jump(text, offset, motion, count, wrap, true)
}

pub fn motion_destination_with_jump(
    text: &dyn VimText,
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
            if ends_with_newline(text) {
                CharOffset::from(char_len(text))
            } else {
                jump(CharOffset::from(char_len(text)))
            }
        }
        VimMotion::JumpToLine(line_number) => jump(jump_to_line_start(text, *line_number)),
        VimMotion::JumpToMatchingBracket => jump_to_matching_bracket(text, offset),
        VimMotion::JumpToUnmatchedBracket(bracket) => {
            vim_find_matching_bracket(&DynBuf(text), bracket, offset).unwrap_or(offset)
        }
    }
}

fn char_len(text: &dyn VimText) -> usize {
    text.char_len()
}

fn clamp_offset(text: &dyn VimText, offset: CharOffset) -> CharOffset {
    CharOffset::from(offset.as_usize().min(char_len(text)))
}

fn char_at(text: &dyn VimText, offset: CharOffset) -> Option<char> {
    text.chars_at(offset).next()
}

fn ends_with_newline(text: &dyn VimText) -> bool {
    let len = char_len(text);
    len > 0 && char_at(text, CharOffset::from(len - 1)) == Some('\n')
}

fn line_start(text: &dyn VimText, offset: CharOffset) -> CharOffset {
    let mut steps = 0;
    for c in text.chars_rev_at(offset) {
        if c == '\n' {
            break;
        }
        steps += 1;
    }
    CharOffset::from(offset.as_usize().saturating_sub(steps))
}

fn line_end_exclusive(text: &dyn VimText, offset: CharOffset) -> CharOffset {
    let mut steps = 0;
    for c in text.chars_at(offset) {
        if c == '\n' {
            break;
        }
        steps += 1;
    }
    offset + steps
}

fn first_nonwhitespace(text: &dyn VimText, offset: CharOffset) -> CharOffset {
    let start = line_start(text, offset);
    let end = line_end_exclusive(text, offset);
    for (steps, c) in text
        .chars_at(start)
        .take(end.as_usize().saturating_sub(start.as_usize()))
        .enumerate()
    {
        if !c.is_whitespace() {
            return start + steps;
        }
    }
    start
}

fn move_horizontal(
    text: &dyn VimText,
    offset: CharOffset,
    count: u32,
    direction: Direction,
    wrap: HorizontalWrap,
) -> CharOffset {
    match wrap {
        HorizontalWrap::StopAtLine => {
            let start = line_start(text, offset);
            let end = line_end_exclusive(text, offset);
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
                        end.as_usize().saturating_sub(offset.as_usize()) as u32,
                    );
                    cmp::min(end, offset + dist as usize)
                }
            }
        }
        HorizontalWrap::SkipNewlines => move_skipping_newlines(text, offset, count, direction),
        HorizontalWrap::CrossLine => move_crossing_lines(text, offset, count, direction),
    }
}

fn move_skipping_newlines(
    text: &dyn VimText,
    offset: CharOffset,
    count: u32,
    direction: Direction,
) -> CharOffset {
    let max = CharOffset::from(char_len(text));
    match direction {
        Direction::Backward => {
            let mut seen = 0u32;
            let mut steps = 0usize;
            for c in text.chars_rev_at(offset) {
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
            let mut seen = 0u32;
            let mut steps = 0usize;
            for c in text.chars_at(offset) {
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
    text: &dyn VimText,
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
    text: &dyn VimText,
    offset: CharOffset,
    count: u32,
    word_motion: &WordMotion,
) -> CharOffset {
    let WordMotion {
        direction,
        bound,
        word_type,
    } = word_motion;
    match vim_word_iterator_from_offset(offset, &DynBuf(text), *direction, *bound, *word_type) {
        Ok(iter) => iter.take(count as usize).last().unwrap_or(offset),
        Err(_) => offset,
    }
}

fn move_to_found_char(
    text: &dyn VimText,
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
    text: &dyn VimText,
    offset: CharOffset,
    count: u32,
    direction: Direction,
) -> CharOffset {
    let max = CharOffset::from(char_len(text));
    let buf = DynBuf(text);
    let mut current = offset;
    match direction {
        Direction::Forward => {
            for _ in 0..count {
                current = find_next_paragraph_end(&buf, current).unwrap_or(max);
            }
        }
        Direction::Backward => {
            for _ in 0..count {
                current =
                    find_previous_paragraph_start(&buf, current).unwrap_or(CharOffset::zero());
            }
        }
    }
    current
}

fn jump_to_line_start(text: &dyn VimText, line_number: u32) -> CharOffset {
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

fn jump_to_matching_bracket(text: &dyn VimText, offset: CharOffset) -> CharOffset {
    let mut iter = text.chars_at(offset);
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
    vim_find_matching_bracket(&DynBuf(text), &bracket, start_offset).unwrap_or(offset)
}

fn char_slice_owned(text: &dyn VimText, start: CharOffset, end: CharOffset) -> Option<String> {
    let s = start.as_usize();
    let e = end.as_usize();
    if e < s {
        return None;
    }
    Some(text.chars_at(start).take(e.saturating_sub(s)).collect())
}

#[cfg(test)]
#[path = "motion_tests.rs"]
mod tests;
