use std::cmp;

use string_offset::CharOffset;

use crate::find_char::vim_find_char_on_line;
use crate::paragraph_iterator::{find_next_paragraph_end, find_previous_paragraph_start};
use crate::vim::{
    CharacterMotion, Direction, FindCharMotion, FirstNonWhitespaceMotion, InsertPosition,
    LineMotion, ModeTransition, MotionType, TextObjectInclusion, TextObjectType, VimMode,
    VimMotion, VimOperand, VimOperator, VimTextObject, WordMotion,
};
use crate::word_iterator::vim_word_iterator_from_offset;
use crate::{vim_a_word, vim_inner_word};

/// Text yanked by an operator, including whether it should paste linewise.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct YankedText {
    pub text: String,
    pub motion_type: MotionType,
}

/// Case transformation requested by `~` / `gU` / `gu`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaseTransform {
    Toggle,
    Uppercase,
    Lowercase,
}

impl CaseTransform {
    pub fn from_operator(operator: &VimOperator) -> Option<Self> {
        match operator {
            VimOperator::ToggleCase => Some(Self::Toggle),
            VimOperator::Uppercase => Some(Self::Uppercase),
            VimOperator::Lowercase => Some(Self::Lowercase),
            VimOperator::Delete
            | VimOperator::Change
            | VimOperator::Yank
            | VimOperator::ToggleComment
            | VimOperator::Indent
            | VimOperator::Dedent => None,
        }
    }

    pub fn apply_to(&self, input: &str) -> String {
        match self {
            CaseTransform::Toggle => input
                .chars()
                .map(|c| {
                    if c.is_lowercase() {
                        c.to_uppercase().next().unwrap_or(c)
                    } else if c.is_uppercase() {
                        c.to_lowercase().next().unwrap_or(c)
                    } else {
                        c
                    }
                })
                .collect(),
            CaseTransform::Uppercase => input.to_uppercase(),
            CaseTransform::Lowercase => input.to_lowercase(),
        }
    }
}

/// Motion type implied by a pending operator's operand.
pub fn operand_motion_type(operand: &VimOperand) -> MotionType {
    match operand {
        VimOperand::Motion { motion_type, .. } => *motion_type,
        VimOperand::TextObject(VimTextObject {
            object_type: TextObjectType::Paragraph,
            ..
        }) => MotionType::Linewise,
        VimOperand::TextObject(_) => MotionType::Charwise,
        VimOperand::Line => MotionType::Linewise,
    }
}

fn register_text_for_yank(selected_text: &str, motion_type: MotionType) -> Option<String> {
    if selected_text.is_empty() && motion_type == MotionType::Linewise {
        Some("\n".to_owned())
    } else if selected_text.is_empty() {
        None
    } else {
        Some(selected_text.to_owned())
    }
}

/// One caret in buffer coordinates (`CharOffset` 1 is the first character).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VimCaret {
    pub head: CharOffset,
    pub tail: CharOffset,
}

impl VimCaret {
    fn ordered(self) -> (CharOffset, CharOffset) {
        if self.head <= self.tail {
            (self.head, self.tail)
        } else {
            (self.tail, self.head)
        }
    }
}

/// Read-only buffer + caret snapshot. Motions and operand ranges are computed here.
#[derive(Clone, Debug)]
pub struct VimSnapshot {
    /// Buffer plain text. Index 0 is unused so `CharOffset(1)` addresses `chars[1]`.
    chars: Vec<char>,
    pub carets: Vec<VimCaret>,
}

impl VimSnapshot {
    pub fn from_plain_text(text: &str, carets: Vec<VimCaret>) -> Self {
        let mut chars = vec!['\0'];
        chars.extend(text.chars());
        Self { chars, carets }
    }

    fn max_offset(&self) -> CharOffset {
        CharOffset::from(self.chars.len().saturating_sub(1).max(1))
    }

    fn char_at(&self, offset: CharOffset) -> Option<char> {
        self.chars
            .get(offset.as_usize())
            .copied()
            .filter(|_| offset.as_usize() != 0)
    }

    fn point(&self, offset: CharOffset) -> (u32, u32) {
        let mut row = 1u32;
        let mut col = 0u32;
        let target = offset
            .as_usize()
            .min(self.chars.len().saturating_sub(1).max(1));
        for (i, c) in self.chars.iter().enumerate().skip(1) {
            if i >= target {
                break;
            }
            if *c == '\n' {
                row += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (row, col)
    }

    fn offset_at(&self, row: u32, col: u32) -> CharOffset {
        let mut cur_row = 1u32;
        let mut cur_col = 0u32;
        let mut last = CharOffset::from(1);
        for (i, c) in self.chars.iter().enumerate().skip(1) {
            last = CharOffset::from(i);
            if cur_row == row && cur_col == col {
                return last;
            }
            if *c == '\n' {
                if cur_row == row {
                    return last;
                }
                cur_row += 1;
                cur_col = 0;
            } else {
                cur_col += 1;
            }
        }
        last
    }

    fn line_len(&self, row: u32) -> u32 {
        let mut cur_row = 1u32;
        let mut len = 0u32;
        for c in self.chars.iter().skip(1) {
            if cur_row == row {
                if *c == '\n' {
                    return len;
                }
                len += 1;
            } else if *c == '\n' {
                cur_row += 1;
                if cur_row > row {
                    return len;
                }
            }
        }
        if cur_row == row { len } else { 0 }
    }

    fn max_row(&self) -> u32 {
        1 + self.chars.iter().skip(1).filter(|c| **c == '\n').count() as u32
    }

    fn line_start(&self, offset: CharOffset) -> CharOffset {
        let (row, _) = self.point(offset);
        self.offset_at(row, 0)
    }

    fn line_end(&self, offset: CharOffset) -> CharOffset {
        let (row, _) = self.point(offset);
        self.offset_at(row, self.line_len(row))
    }

    fn line_text(&self, offset: CharOffset) -> String {
        let start = self.line_start(offset).as_usize();
        let end = self.line_end(offset).as_usize();
        self.chars[start.max(1)..=end.min(self.chars.len().saturating_sub(1)).max(1)]
            .iter()
            .take_while(|c| **c != '\n')
            .collect()
    }

    fn first_nonwhitespace(&self, offset: CharOffset) -> CharOffset {
        let start = self.line_start(offset);
        let end = self.line_end(offset);
        let mut off = start;
        while off < end {
            match self.char_at(off) {
                Some(c) if c.is_whitespace() && c != '\n' => off += 1,
                _ => return off,
            }
        }
        start
    }

    fn plain_text(&self) -> String {
        self.chars.iter().skip(1).collect()
    }

    fn selected_text(&self) -> String {
        self.carets
            .iter()
            .map(|caret| {
                let (start, end) = caret.ordered();
                let s = start.as_usize().max(1);
                let e = end.as_usize().min(self.chars.len());
                if s >= e {
                    String::new()
                } else {
                    self.chars[s..e].iter().collect()
                }
            })
            .collect()
    }

    fn map_heads(&mut self, f: impl Fn(&Self, CharOffset) -> CharOffset, keep_selection: bool) {
        let new_heads: Vec<CharOffset> = self
            .carets
            .iter()
            .map(|caret| f(self, caret.head))
            .collect();
        for (caret, head) in self.carets.iter_mut().zip(new_heads) {
            caret.head = head;
            if !keep_selection {
                caret.tail = caret.head;
            }
        }
    }

    fn extend_linewise(&mut self, include_newline: bool) {
        let max = self.max_offset();
        let updates: Vec<_> = self
            .carets
            .iter()
            .map(|caret| {
                let start = self.line_start(caret.head.min(caret.tail));
                let end_pos = caret.head.max(caret.tail);
                let mut end = self.line_end(end_pos);
                if include_newline && end < max {
                    end += 1;
                }
                (start, end)
            })
            .collect();
        for (caret, (start, end)) in self.carets.iter_mut().zip(updates) {
            caret.tail = start;
            caret.head = end;
        }
    }
}

/// Editor primitives. Motions, operand ranges, operators, visual behavior, and insert-position
/// policy are implemented on top of these.
pub trait VimBufferOps {
    type Ctx<'a>
    where
        Self: 'a;

    fn snapshot(&self, ctx: &Self::Ctx<'_>) -> VimSnapshot;
    fn set_selections(&mut self, carets: &[VimCaret], ctx: &mut Self::Ctx<'_>);
    fn replace_ranges(
        &mut self,
        edits: &[(CharOffset, CharOffset, String)],
        ctx: &mut Self::Ctx<'_>,
    );

    fn toggle_comments(&mut self, _ctx: &mut Self::Ctx<'_>) {}
    fn indent(&mut self, _dedent: bool, _ctx: &mut Self::Ctx<'_>) {}
    fn open_line(&mut self, above: bool, ctx: &mut Self::Ctx<'_>) {
        let snap = self.snapshot(ctx);
        let caret = snap.carets.first().copied().unwrap_or(VimCaret {
            head: CharOffset::from(1),
            tail: CharOffset::from(1),
        });
        if above {
            let start = snap.line_start(caret.head);
            self.replace_ranges(&[(start, start, "\n".to_owned())], ctx);
            self.set_selections(
                &[VimCaret {
                    head: start,
                    tail: start,
                }],
                ctx,
            );
        } else {
            let end = snap.line_end(caret.head);
            self.replace_ranges(&[(end, end, "\n".to_owned())], ctx);
            let head = end + 1;
            self.set_selections(&[VimCaret { head, tail: head }], ctx);
        }
    }

    fn supports_operator(&self, _operator: &VimOperator) -> bool {
        false
    }

    fn supports_text_objects(&self) -> bool {
        false
    }

    fn smart_indent_on_linewise_change(&self, _operand: &VimOperand) -> bool {
        false
    }

    fn change_line_smart_indent(&mut self, ctx: &mut Self::Ctx<'_>) {
        self.delete_selection(ctx);
        self.open_line(false, ctx);
    }

    fn selected_text(&mut self, ctx: &mut Self::Ctx<'_>) -> String {
        self.snapshot(ctx).selected_text()
    }

    fn delete_selection(&mut self, ctx: &mut Self::Ctx<'_>) {
        let snap = self.snapshot(ctx);
        let edits: Vec<_> = snap
            .carets
            .iter()
            .map(|caret| {
                let (start, end) = caret.ordered();
                (start, end, String::new())
            })
            .collect();
        if !edits.is_empty() {
            self.replace_ranges(&edits, ctx);
        }
    }

    fn insert_text(&mut self, text: &str, ctx: &mut Self::Ctx<'_>) {
        let snap = self.snapshot(ctx);
        let edits: Vec<_> = snap
            .carets
            .iter()
            .map(|caret| {
                let (start, end) = caret.ordered();
                (start, end, text.to_owned())
            })
            .collect();
        if !edits.is_empty() {
            self.replace_ranges(&edits, ctx);
        }
    }

    fn move_to_line_start(&mut self, ctx: &mut Self::Ctx<'_>) {
        let mut snap = self.snapshot(ctx);
        snap.map_heads(|s, head| s.line_start(head), false);
        self.set_selections(&snap.carets, ctx);
    }

    fn collapse_to_selection_start(&mut self, ctx: &mut Self::Ctx<'_>) {
        let mut snap = self.snapshot(ctx);
        for caret in &mut snap.carets {
            let start = caret.head.min(caret.tail);
            *caret = VimCaret {
                head: start,
                tail: start,
            };
        }
        self.set_selections(&snap.carets, ctx);
    }

    fn transform_case(&mut self, transform: CaseTransform, ctx: &mut Self::Ctx<'_>) {
        let snap = self.snapshot(ctx);
        let edits: Vec<_> = snap
            .carets
            .iter()
            .map(|caret| {
                let (start, end) = caret.ordered();
                let original = {
                    let s = start.as_usize().max(1);
                    let e = end.as_usize().min(snap.chars.len());
                    if s >= e {
                        String::new()
                    } else {
                        snap.chars[s..e].iter().collect()
                    }
                };
                (start, end, transform.apply_to(&original))
            })
            .collect();
        if !edits.is_empty() {
            self.replace_ranges(&edits, ctx);
        }
        self.clear_selections(ctx);
    }

    fn move_to_first_nonwhitespace(&mut self, ctx: &mut Self::Ctx<'_>) {
        let mut snap = self.snapshot(ctx);
        snap.map_heads(|s, head| s.first_nonwhitespace(head), false);
        self.set_selections(&snap.carets, ctx);
    }

    fn expand_visual_selection(
        &mut self,
        motion_type: MotionType,
        include_newline: bool,
        ctx: &mut Self::Ctx<'_>,
    ) {
        let mut snap = self.snapshot(ctx);
        let max = snap.max_offset();
        let updates: Vec<_> = snap
            .carets
            .iter()
            .map(|caret| {
                let mut start = caret.tail;
                let mut end = caret.head;
                if start > end {
                    std::mem::swap(&mut start, &mut end);
                }
                if end < max {
                    end += 1;
                }
                if motion_type == MotionType::Linewise {
                    start = snap.line_start(start);
                    end = snap.line_end(end);
                    if include_newline && end < max {
                        end += 1;
                    }
                }
                VimCaret {
                    head: start,
                    tail: end,
                }
            })
            .collect();
        snap.carets = updates;
        self.set_selections(&snap.carets, ctx);
    }

    fn clear_selections(&mut self, ctx: &mut Self::Ctx<'_>) {
        let mut snap = self.snapshot(ctx);
        if let Some(first) = snap.carets.first().copied() {
            snap.carets = vec![VimCaret {
                head: first.head,
                tail: first.head,
            }];
            self.set_selections(&snap.carets, ctx);
        }
    }

    fn apply_insert_position(&mut self, position: &InsertPosition, ctx: &mut Self::Ctx<'_>) {
        match position {
            InsertPosition::AtCursor => {}
            InsertPosition::AfterCursor => {
                let mut snap = self.snapshot(ctx);
                snap.map_heads(
                    |s, head| {
                        let (row, col) = s.point(head);
                        let line_len = s.line_len(row);
                        if col < line_len { head + 1 } else { head }
                    },
                    false,
                );
                self.set_selections(&snap.carets, ctx);
            }
            InsertPosition::LineFirstNonWhitespace => self.move_to_first_nonwhitespace(ctx),
            InsertPosition::LineEnd => {
                let mut snap = self.snapshot(ctx);
                snap.map_heads(|s, head| s.line_end(head), false);
                self.set_selections(&snap.carets, ctx);
            }
            InsertPosition::LineAbove => self.open_line(true, ctx),
            InsertPosition::LineBelow => self.open_line(false, ctx),
        }
    }

    fn move_left_exiting_insert(&mut self, ctx: &mut Self::Ctx<'_>) {
        let mut snap = self.snapshot(ctx);
        snap.map_heads(
            |s, head| {
                let (_, col) = s.point(head);
                if col == 0 {
                    head
                } else {
                    head.as_usize().saturating_sub(1).max(1).into()
                }
            },
            false,
        );
        self.set_selections(&snap.carets, ctx);
    }

    fn enforce_cursor_line_cap(&mut self, ctx: &mut Self::Ctx<'_>) {
        let mut snap = self.snapshot(ctx);
        let updates: Vec<_> = snap
            .carets
            .iter()
            .map(|caret| {
                let (row, col) = snap.point(caret.head);
                let line_len = snap.line_len(row);
                if line_len > 0 && col >= line_len {
                    VimCaret {
                        head: CharOffset::from(caret.head.as_usize().saturating_sub(1).max(1)),
                        tail: CharOffset::from(caret.tail.as_usize().saturating_sub(1).max(1)),
                    }
                } else {
                    *caret
                }
            })
            .collect();
        snap.carets = updates;
        self.set_selections(&snap.carets, ctx);
    }

    fn set_visual_tails_to_heads(&mut self, ctx: &mut Self::Ctx<'_>) {
        let mut snap = self.snapshot(ctx);
        for caret in &mut snap.carets {
            caret.tail = caret.head;
        }
        self.set_selections(&snap.carets, ctx);
    }

    fn enforce_normal_mode_line_cap(&mut self, ctx: &mut Self::Ctx<'_>) {
        self.enforce_cursor_line_cap(ctx);
    }

    fn select_for_operand(
        &mut self,
        operator: &VimOperator,
        operand_count: u32,
        operand: &VimOperand,
        ctx: &mut Self::Ctx<'_>,
    ) {
        let mut snap = self.snapshot(ctx);
        match operand {
            VimOperand::Motion { motion, .. } => match motion {
                VimMotion::Character(m) => move_char_snapshot(&mut snap, operand_count, m, true),
                VimMotion::Word(m) => move_word_snapshot(&mut snap, operand_count, m, true),
                VimMotion::Line(m) => move_line_snapshot(&mut snap, operand_count, m, true),
                VimMotion::FirstNonWhitespace(m) => {
                    move_first_nonwhitespace_snapshot(&mut snap, operand_count, m, true);
                }
                VimMotion::Paragraph(direction) => {
                    move_paragraph_snapshot(&mut snap, operand_count, direction, true);
                }
                VimMotion::JumpToLastLine => jump_last_snapshot(&mut snap),
                VimMotion::JumpToFirstLine => jump_first_snapshot(&mut snap),
                VimMotion::FindChar(m) => find_char_snapshot(&mut snap, operand_count, m, true),
                VimMotion::JumpToLine(line_number) => {
                    let row = (*line_number).max(1).min(snap.max_row());
                    snap.map_heads(|s, _| s.offset_at(row, 0), false);
                }
                VimMotion::JumpToMatchingBracket | VimMotion::JumpToUnmatchedBracket(_) => {}
            },
            VimOperand::Line => {
                if operand_count > 1 {
                    move_vertical_snapshot(&mut snap, operand_count - 1, Direction::Forward, true);
                }
                snap.extend_linewise(operator.includes_trailing_newline());
            }
            VimOperand::TextObject(object) => {
                if self.supports_text_objects() {
                    select_text_object_snapshot(&mut snap, object);
                }
            }
        }
        if let VimOperand::Motion {
            motion_type: MotionType::Linewise,
            ..
        } = operand
        {
            snap.extend_linewise(operator.includes_trailing_newline());
        }
        self.set_selections(&snap.carets, ctx);
    }
}

fn move_char_snapshot(snap: &mut VimSnapshot, count: u32, motion: &CharacterMotion, keep: bool) {
    match motion {
        CharacterMotion::Right => snap.map_heads(
            |s, head| {
                let (row, col) = s.point(head);
                let line_len = s.line_len(row);
                let delta = u32::min(line_len.saturating_sub(col), count);
                head + delta as usize
            },
            keep,
        ),
        CharacterMotion::Left => snap.map_heads(
            |s, head| {
                let (_, col) = s.point(head);
                let delta = u32::min(col, count);
                CharOffset::from(head.as_usize().saturating_sub(delta as usize).max(1))
            },
            keep,
        ),
        CharacterMotion::WrappingRight => snap.map_heads(
            |s, head| {
                let max = s.max_offset();
                let mut h = head;
                for _ in 0..count {
                    if h >= max {
                        break;
                    }
                    h += 1;
                }
                h
            },
            keep,
        ),
        CharacterMotion::WrappingLeft => snap.map_heads(
            |_, head| {
                let mut h = head;
                for _ in 0..count {
                    if h <= CharOffset::from(1) {
                        break;
                    }
                    h = CharOffset::from(h.as_usize() - 1);
                }
                h
            },
            keep,
        ),
        CharacterMotion::Up => move_vertical_snapshot(snap, count, Direction::Backward, keep),
        CharacterMotion::Down => move_vertical_snapshot(snap, count, Direction::Forward, keep),
    }
}

fn move_vertical_snapshot(snap: &mut VimSnapshot, count: u32, direction: Direction, keep: bool) {
    let max_row = snap.max_row();
    snap.map_heads(
        |s, head| {
            let (row, col) = s.point(head);
            let target = match direction {
                Direction::Backward => row.saturating_sub(count).max(1),
                Direction::Forward => cmp::min(max_row, row.saturating_add(count)),
            };
            let new_col = cmp::min(col, s.line_len(target));
            s.offset_at(target, new_col)
        },
        keep,
    );
}

fn move_word_snapshot(snap: &mut VimSnapshot, count: u32, motion: &WordMotion, keep: bool) {
    let text = snap.plain_text();
    snap.map_heads(
        |_, head| {
            let zero = CharOffset::from(head.as_usize().saturating_sub(1));
            let Ok(iter) = vim_word_iterator_from_offset(
                zero,
                text.as_str(),
                motion.direction,
                motion.bound,
                motion.word_type,
            ) else {
                return head;
            };
            iter.take(count as usize)
                .last()
                .map(|off| CharOffset::from(off.as_usize() + 1))
                .unwrap_or(head)
        },
        keep,
    );
}

fn move_line_snapshot(snap: &mut VimSnapshot, line_count: u32, motion: &LineMotion, keep: bool) {
    match motion {
        LineMotion::Start => snap.map_heads(|s, head| s.line_start(head), keep),
        LineMotion::FirstNonWhitespace => {
            snap.map_heads(|s, head| s.first_nonwhitespace(head), keep)
        }
        LineMotion::End => {
            move_vertical_snapshot(snap, line_count.saturating_sub(1), Direction::Forward, keep);
            snap.map_heads(|s, head| s.line_end(head), keep);
        }
    }
}

fn move_first_nonwhitespace_snapshot(
    snap: &mut VimSnapshot,
    count: u32,
    motion: &FirstNonWhitespaceMotion,
    keep: bool,
) {
    match motion {
        FirstNonWhitespaceMotion::Up => {
            move_vertical_snapshot(snap, count, Direction::Backward, keep);
        }
        FirstNonWhitespaceMotion::Down => {
            move_vertical_snapshot(snap, count, Direction::Forward, keep);
        }
        FirstNonWhitespaceMotion::DownMinusOne => {
            move_vertical_snapshot(snap, count.saturating_sub(1), Direction::Forward, keep);
        }
    }
    snap.map_heads(|s, head| s.first_nonwhitespace(head), keep);
}

fn move_paragraph_snapshot(snap: &mut VimSnapshot, count: u32, direction: &Direction, keep: bool) {
    let text = snap.plain_text();
    let max = snap.max_offset();
    snap.map_heads(
        |_, head| {
            let zero = CharOffset::from(head.as_usize().saturating_sub(1));
            let mut offset = zero;
            match direction {
                Direction::Forward => {
                    for _ in 0..count {
                        offset = find_next_paragraph_end(text.as_str(), offset)
                            .unwrap_or(CharOffset::from(text.chars().count()));
                    }
                    CharOffset::from(offset.as_usize() + 1).min(max)
                }
                Direction::Backward => {
                    for _ in 0..count {
                        offset = find_previous_paragraph_start(text.as_str(), offset)
                            .unwrap_or(CharOffset::from(0));
                    }
                    CharOffset::from(offset.as_usize() + 1).max(CharOffset::from(1))
                }
            }
        },
        keep,
    );
}

fn jump_first_snapshot(snap: &mut VimSnapshot) {
    snap.map_heads(|_, _| CharOffset::from(1), false);
}

fn jump_last_snapshot(snap: &mut VimSnapshot) {
    let max = snap.max_offset();
    snap.map_heads(|_, _| max, false);
}

fn find_char_snapshot(
    snap: &mut VimSnapshot,
    occurrence_count: u32,
    motion: &FindCharMotion,
    keep: bool,
) {
    snap.map_heads(
        |s, head| {
            let (_, col) = s.point(head);
            let line = s.line_text(head);
            vim_find_char_on_line(&line, col as usize, motion, occurrence_count, keep)
                .map(|new_col| {
                    let (row, _) = s.point(head);
                    s.offset_at(row, new_col as u32)
                })
                .unwrap_or(head)
        },
        keep,
    );
}

fn select_text_object_snapshot(snap: &mut VimSnapshot, object: &VimTextObject) {
    let text = snap.plain_text();
    for caret in &mut snap.carets {
        let zero = CharOffset::from(caret.head.as_usize().saturating_sub(1));
        let range = match object.object_type {
            TextObjectType::Word(word_type) => match object.inclusion {
                TextObjectInclusion::Inner => vim_inner_word(text.as_str(), zero, word_type),
                TextObjectInclusion::Around => vim_a_word(text.as_str(), zero, word_type),
            },
            _ => None,
        };
        if let Some(range) = range {
            caret.tail = CharOffset::from(range.start.as_usize() + 1);
            caret.head = CharOffset::from(range.end.as_usize() + 1);
        }
    }
}

/// Shared caret navigation used by view `VimHandler` impls.
pub fn move_char<B: VimBufferOps>(
    buffer: &mut B,
    count: u32,
    motion: &CharacterMotion,
    ctx: &mut B::Ctx<'_>,
) {
    let mut snap = buffer.snapshot(ctx);
    move_char_snapshot(&mut snap, count, motion, false);
    buffer.set_selections(&snap.carets, ctx);
}

pub fn move_word<B: VimBufferOps>(
    buffer: &mut B,
    count: u32,
    motion: &WordMotion,
    ctx: &mut B::Ctx<'_>,
) {
    let mut snap = buffer.snapshot(ctx);
    move_word_snapshot(&mut snap, count, motion, false);
    buffer.set_selections(&snap.carets, ctx);
}

pub fn move_line<B: VimBufferOps>(
    buffer: &mut B,
    count: u32,
    motion: &LineMotion,
    ctx: &mut B::Ctx<'_>,
) {
    let mut snap = buffer.snapshot(ctx);
    move_line_snapshot(&mut snap, count, motion, false);
    buffer.set_selections(&snap.carets, ctx);
}

pub fn move_first_nonwhitespace<B: VimBufferOps>(
    buffer: &mut B,
    count: u32,
    motion: &FirstNonWhitespaceMotion,
    ctx: &mut B::Ctx<'_>,
) {
    let mut snap = buffer.snapshot(ctx);
    move_first_nonwhitespace_snapshot(&mut snap, count, motion, false);
    buffer.set_selections(&snap.carets, ctx);
}

pub fn move_paragraph<B: VimBufferOps>(
    buffer: &mut B,
    count: u32,
    direction: &Direction,
    ctx: &mut B::Ctx<'_>,
) {
    let mut snap = buffer.snapshot(ctx);
    move_paragraph_snapshot(&mut snap, count, direction, false);
    buffer.set_selections(&snap.carets, ctx);
}

pub fn find_char<B: VimBufferOps>(
    buffer: &mut B,
    occurrence_count: u32,
    motion: &FindCharMotion,
    ctx: &mut B::Ctx<'_>,
) {
    let mut snap = buffer.snapshot(ctx);
    find_char_snapshot(&mut snap, occurrence_count, motion, false);
    buffer.set_selections(&snap.carets, ctx);
}

pub fn jump_to_first_line<B: VimBufferOps>(buffer: &mut B, ctx: &mut B::Ctx<'_>) {
    let mut snap = buffer.snapshot(ctx);
    jump_first_snapshot(&mut snap);
    buffer.set_selections(&snap.carets, ctx);
}

pub fn jump_to_last_line<B: VimBufferOps>(buffer: &mut B, ctx: &mut B::Ctx<'_>) {
    let mut snap = buffer.snapshot(ctx);
    jump_last_snapshot(&mut snap);
    buffer.set_selections(&snap.carets, ctx);
}

pub fn jump_to_line<B: VimBufferOps>(buffer: &mut B, line_number: u32, ctx: &mut B::Ctx<'_>) {
    let mut snap = buffer.snapshot(ctx);
    let row = line_number.max(1).min(snap.max_row());
    snap.map_heads(|s, _| s.offset_at(row, 0), false);
    buffer.set_selections(&snap.carets, ctx);
}

/// Apply a normal-mode operator (`d`/`c`/`y`/case/comment/indent) to `operand`.
pub fn apply_operator<B: VimBufferOps>(
    buffer: &mut B,
    operator: &VimOperator,
    operand_count: u32,
    operand: &VimOperand,
    replacement_text: &str,
    ctx: &mut B::Ctx<'_>,
) -> Option<YankedText> {
    if !buffer.supports_operator(operator) {
        return None;
    }

    let motion_type = operand_motion_type(operand);

    match operator {
        VimOperator::Delete | VimOperator::Change => {
            buffer.select_for_operand(operator, operand_count, operand, ctx);
            let selected_text = buffer.selected_text(ctx);
            let yanked = register_text_for_yank(&selected_text, motion_type)
                .map(|text| YankedText { text, motion_type });
            if !selected_text.is_empty() {
                if *operator == VimOperator::Change
                    && motion_type == MotionType::Linewise
                    && buffer.smart_indent_on_linewise_change(operand)
                {
                    buffer.change_line_smart_indent(ctx);
                } else {
                    buffer.delete_selection(ctx);
                    if *operator == VimOperator::Change && !replacement_text.is_empty() {
                        buffer.insert_text(replacement_text, ctx);
                    }
                    if motion_type == MotionType::Linewise {
                        buffer.move_to_line_start(ctx);
                    }
                }
            }
            yanked
        }
        VimOperator::Yank => {
            let original = buffer.snapshot(ctx).carets;
            buffer.select_for_operand(operator, operand_count, operand, ctx);
            let selected_text = buffer.selected_text(ctx);
            let yanked = register_text_for_yank(&selected_text, motion_type)
                .map(|text| YankedText { text, motion_type });
            if matches!(operand, VimOperand::TextObject(_)) {
                buffer.collapse_to_selection_start(ctx);
            } else {
                buffer.set_selections(&original, ctx);
            }
            yanked
        }
        VimOperator::ToggleCase | VimOperator::Uppercase | VimOperator::Lowercase => {
            buffer.select_for_operand(operator, operand_count, operand, ctx);
            if let Some(transform) = CaseTransform::from_operator(operator) {
                buffer.transform_case(transform, ctx);
            }
            None
        }
        VimOperator::ToggleComment => {
            let original = buffer.snapshot(ctx).carets;
            buffer.select_for_operand(operator, operand_count, operand, ctx);
            buffer.toggle_comments(ctx);
            if motion_type == MotionType::Linewise {
                buffer.move_to_first_nonwhitespace(ctx);
            } else {
                buffer.set_selections(&original, ctx);
            }
            None
        }
        VimOperator::Indent | VimOperator::Dedent => {
            buffer.select_for_operand(operator, operand_count, operand, ctx);
            buffer.indent(*operator == VimOperator::Dedent, ctx);
            buffer.move_to_first_nonwhitespace(ctx);
            None
        }
    }
}

/// Apply a visual-mode operator to the current visual selection.
pub fn apply_visual_operator<B: VimBufferOps>(
    buffer: &mut B,
    operator: &VimOperator,
    motion_type: MotionType,
    ctx: &mut B::Ctx<'_>,
) -> Option<YankedText> {
    if !buffer.supports_operator(operator) {
        return None;
    }

    buffer.expand_visual_selection(motion_type, operator.includes_trailing_newline(), ctx);

    let yanked = if matches!(
        operator,
        VimOperator::Delete | VimOperator::Change | VimOperator::Yank
    ) {
        let selected_text = buffer.selected_text(ctx);
        if selected_text.is_empty() {
            None
        } else {
            Some(YankedText {
                text: selected_text,
                motion_type,
            })
        }
    } else {
        None
    };

    match operator {
        VimOperator::Delete | VimOperator::Change => {
            buffer.delete_selection(ctx);
            if *operator == VimOperator::Change && motion_type == MotionType::Linewise {
                buffer.change_line_smart_indent(ctx);
            }
        }
        VimOperator::ToggleCase | VimOperator::Uppercase | VimOperator::Lowercase => {
            if let Some(transform) = CaseTransform::from_operator(operator) {
                buffer.transform_case(transform, ctx);
            }
        }
        VimOperator::Yank => buffer.clear_selections(ctx),
        VimOperator::ToggleComment => {
            buffer.toggle_comments(ctx);
            if motion_type == MotionType::Linewise {
                buffer.move_to_first_nonwhitespace(ctx);
            } else {
                buffer.clear_selections(ctx);
            }
        }
        VimOperator::Indent | VimOperator::Dedent => {
            buffer.indent(*operator == VimOperator::Dedent, ctx);
            buffer.move_to_first_nonwhitespace(ctx);
        }
    }

    yanked
}

/// Replace the visual selection with `paste_text`.
pub fn apply_visual_paste<B: VimBufferOps>(
    buffer: &mut B,
    motion_type: MotionType,
    paste_text: &str,
    yanked_motion_type: MotionType,
    ctx: &mut B::Ctx<'_>,
) -> Option<YankedText> {
    let include_newline =
        motion_type == MotionType::Linewise && yanked_motion_type == MotionType::Linewise;
    buffer.expand_visual_selection(motion_type, include_newline, ctx);
    let selected_text = buffer.selected_text(ctx);
    let yanked = if selected_text.is_empty() {
        None
    } else {
        Some(YankedText {
            text: selected_text,
            motion_type,
        })
    };
    buffer.insert_text(paste_text, ctx);
    if motion_type == MotionType::Linewise {
        buffer.move_to_line_start(ctx);
    }
    yanked
}

/// Apply cursor policy for a vim mode transition.
pub fn apply_mode_change<B: VimBufferOps>(
    buffer: &mut B,
    old: &VimMode,
    new: &ModeTransition,
    ctx: &mut B::Ctx<'_>,
) {
    match new.mode {
        VimMode::Normal => {
            if *old == VimMode::Insert {
                buffer.move_left_exiting_insert(ctx);
            }
            buffer.enforce_normal_mode_line_cap(ctx);
        }
        VimMode::Insert => buffer.apply_insert_position(&new.position, ctx),
        VimMode::Visual(_) => buffer.set_visual_tails_to_heads(ctx),
        VimMode::Replace => buffer.enforce_cursor_line_cap(ctx),
    }
}

#[cfg(test)]
#[path = "handler_tests.rs"]
mod tests;
