use std::cmp;

use num_traits::SaturatingSub;
use string_offset::CharOffset;
use vec1::Vec1;
use vim::handler::{CaseTransform, VimBufferOps};
use vim::vim::{
    CharacterMotion, Direction, FindCharMotion, FirstNonWhitespaceMotion, InsertPosition,
    LineMotion, MotionType, VimMotion, VimOperand, VimOperator, WordBound, WordMotion,
};
use vim::{find_next_paragraph_end, find_previous_paragraph_start, vim_find_char_on_line};
use warp_editor::content::buffer::{
    AutoScrollBehavior, BufferEditAction, BufferSelectAction, EditOrigin, SelectionOffsets,
    ToBufferCharOffset, ToBufferPoint,
};
use warp_editor::model::{CoreEditorModel, RichTextEditorModel};
use warp_editor::selection::{TextDirection, TextUnit};
use warpui::ModelContext;
use warpui::text::point::Point;

use super::NotebooksEditorModel;

impl NotebooksEditorModel {
    fn vim_set_selections(
        &mut self,
        selections: Vec1<SelectionOffsets>,
        autoscroll: AutoScrollBehavior,
        ctx: &mut ModelContext<Self>,
    ) {
        self.selection.update(ctx, |selection, ctx| {
            selection.update_selection(
                BufferSelectAction::SetSelectionOffsets { selections },
                autoscroll,
                ctx,
            );
        });
    }

    fn vim_move_horizontal(
        &mut self,
        char_count: u32,
        direction: &Direction,
        keep_selection: bool,
        stop_at_line_boundary: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer = self.content().as_ref(ctx);
        let current_selections = self.selections(ctx);
        let new_selections = current_selections.mapped(|selection| {
            let mut head = selection.head;
            if stop_at_line_boundary {
                let head_point = head.to_buffer_point(buffer);
                let offset_change = match direction {
                    Direction::Backward => u32::min(head_point.column, char_count),
                    Direction::Forward => {
                        let line_len = buffer.line_len(head_point.row);
                        u32::min(line_len.saturating_sub(head_point.column), char_count)
                    }
                };
                head = match direction {
                    Direction::Backward => {
                        head.saturating_sub(&CharOffset::from(offset_change as usize))
                    }
                    Direction::Forward => {
                        cmp::min(buffer.max_charoffset(), head + offset_change as usize)
                    }
                };
            } else {
                let max_offset = buffer.max_charoffset();
                for _ in 0..char_count {
                    match direction {
                        Direction::Forward => {
                            if head >= max_offset {
                                break;
                            }
                            head = cmp::min(max_offset, head + 1);
                        }
                        Direction::Backward => {
                            if head <= CharOffset::from(1) {
                                break;
                            }
                            head = head.saturating_sub(&CharOffset::from(1));
                        }
                    }
                }
            }
            SelectionOffsets {
                head,
                tail: if keep_selection { selection.tail } else { head },
            }
        });
        self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
    }

    fn vim_move_vertical(
        &mut self,
        count: u32,
        direction: TextDirection,
        keep_selection: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer = self.content().as_ref(ctx);
        let current_selections = self.selections(ctx);
        let max_row = buffer.max_point().row;
        let new_selections = current_selections.mapped(|selection| {
            let point = selection.head.to_buffer_point(buffer);
            let target_row = match direction {
                TextDirection::Backwards => point.row.saturating_sub(count),
                TextDirection::Forwards => cmp::min(max_row, point.row.saturating_add(count)),
            };
            let new_col = cmp::min(point.column, buffer.line_len(target_row));
            let new_offset = Point::new(target_row, new_col).to_buffer_char_offset(buffer);
            SelectionOffsets {
                head: new_offset,
                tail: if keep_selection {
                    selection.tail
                } else {
                    new_offset
                },
            }
        });
        self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
    }

    fn vim_move_to_line_bound(
        &mut self,
        start: bool,
        keep_selection: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let current_selections = self.selections(ctx);
        let content = self.content().as_ref(ctx);
        let new_selections = current_selections.mapped(|selection| {
            let point = selection.head.to_buffer_point(content);
            let new_column = if start {
                0
            } else {
                content.line_len(point.row)
            };
            let new_offset = Point::new(point.row, new_column).to_buffer_char_offset(content);
            SelectionOffsets {
                head: new_offset,
                tail: if keep_selection {
                    selection.tail
                } else {
                    new_offset
                },
            }
        });
        self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
    }

    pub(crate) fn vim_navigate_char(
        &mut self,
        count: u32,
        motion: &CharacterMotion,
        keep_selection: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        match motion {
            CharacterMotion::Right => {
                self.vim_move_horizontal(count, &Direction::Forward, keep_selection, true, ctx);
            }
            CharacterMotion::Left => {
                self.vim_move_horizontal(count, &Direction::Backward, keep_selection, true, ctx);
            }
            CharacterMotion::WrappingRight => {
                self.vim_move_horizontal(count, &Direction::Forward, keep_selection, false, ctx);
            }
            CharacterMotion::WrappingLeft => {
                self.vim_move_horizontal(count, &Direction::Backward, keep_selection, false, ctx);
            }
            CharacterMotion::Up => {
                self.vim_move_vertical(count, TextDirection::Backwards, keep_selection, ctx);
            }
            CharacterMotion::Down => {
                self.vim_move_vertical(count, TextDirection::Forwards, keep_selection, ctx);
            }
        }
    }

    pub(crate) fn vim_navigate_word(
        &mut self,
        count: u32,
        motion: &WordMotion,
        ctx: &mut ModelContext<Self>,
    ) {
        match (motion.direction, motion.bound) {
            (Direction::Forward, WordBound::Start | WordBound::End) => {
                for _ in 0..count {
                    self.forward_word(false, ctx);
                }
            }
            (Direction::Backward, _) => {
                for _ in 0..count {
                    self.backward_word(false, ctx);
                }
            }
        }
    }

    pub(crate) fn vim_navigate_line(
        &mut self,
        line_count: u32,
        motion: &LineMotion,
        ctx: &mut ModelContext<Self>,
    ) {
        match motion {
            LineMotion::Start => self.vim_move_to_line_bound(true, false, ctx),
            LineMotion::FirstNonWhitespace => self.vim_move_to_first_nonwhitespace(false, ctx),
            LineMotion::End => {
                self.vim_move_vertical(
                    line_count.saturating_sub(1),
                    TextDirection::Forwards,
                    false,
                    ctx,
                );
                self.vim_move_to_line_bound(false, false, ctx);
            }
        }
    }

    pub(crate) fn vim_first_nonwhitespace_motion(
        &mut self,
        count: u32,
        motion: &FirstNonWhitespaceMotion,
        ctx: &mut ModelContext<Self>,
    ) {
        match motion {
            FirstNonWhitespaceMotion::Up => {
                self.vim_move_vertical(count, TextDirection::Backwards, false, ctx);
            }
            FirstNonWhitespaceMotion::Down => {
                self.vim_move_vertical(count, TextDirection::Forwards, false, ctx);
            }
            FirstNonWhitespaceMotion::DownMinusOne => {
                self.vim_move_vertical(
                    count.saturating_sub(1),
                    TextDirection::Forwards,
                    false,
                    ctx,
                );
            }
        }
        self.vim_move_to_first_nonwhitespace(false, ctx);
    }

    pub(crate) fn vim_find_char(
        &mut self,
        occurrence_count: u32,
        motion: &FindCharMotion,
        keep_selection: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer = self.content().as_ref(ctx);
        let current_selections = self.selections(ctx);
        let new_selections = current_selections.mapped(|selection| {
            let head_point = selection.head.to_buffer_point(buffer);
            let line_start = buffer.containing_line_start(selection.head);
            let line_end = buffer.containing_line_end(selection.head);
            let line_text = buffer.text_in_range(line_start..line_end).into_string();
            let Some(new_column) = vim_find_char_on_line(
                &line_text,
                head_point.column as usize,
                motion,
                occurrence_count,
                keep_selection,
            ) else {
                return selection;
            };
            let new_head =
                Point::new(head_point.row, new_column as u32).to_buffer_char_offset(buffer);
            SelectionOffsets {
                head: new_head,
                tail: if keep_selection {
                    selection.tail
                } else {
                    new_head
                },
            }
        });
        self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
    }

    pub(crate) fn vim_move_by_paragraph(
        &mut self,
        count: u32,
        direction: &Direction,
        keep_selection: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer = self.content().as_ref(ctx);
        let current_selections = self.selections(ctx);
        let max = buffer.max_charoffset();
        let new_selections = current_selections.mapped(|selection| {
            let mut offset = selection.head;
            match direction {
                Direction::Forward => {
                    for _ in 0..count {
                        offset = find_next_paragraph_end(buffer, offset).unwrap_or(max);
                    }
                }
                Direction::Backward => {
                    for _ in 0..count {
                        offset = find_previous_paragraph_start(buffer, offset)
                            .unwrap_or(CharOffset::from(1));
                    }
                }
            }
            SelectionOffsets {
                head: offset,
                tail: if keep_selection {
                    selection.tail
                } else {
                    offset
                },
            }
        });
        self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
    }

    pub(crate) fn vim_jump_to_first_line(&mut self, ctx: &mut ModelContext<Self>) {
        self.cursor_at(CharOffset::from(1), ctx);
    }

    pub(crate) fn vim_jump_to_last_line(&mut self, ctx: &mut ModelContext<Self>) {
        let max = self.content().as_ref(ctx).max_charoffset();
        self.cursor_at(max, ctx);
    }

    pub(crate) fn vim_jump_to_line(&mut self, line_number: u32, ctx: &mut ModelContext<Self>) {
        let buffer = self.content().as_ref(ctx);
        let row = line_number.max(1).min(buffer.max_point().row);
        let offset = Point::new(row, 0).to_buffer_char_offset(buffer);
        self.cursor_at(offset, ctx);
    }

    pub(crate) fn vim_visual_tails(&self) -> &[CharOffset] {
        &self.vim_visual_tails
    }

    fn vim_extend_selection_linewise(
        &mut self,
        include_newline: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer = self.content().as_ref(ctx);
        let current_selections = self.selections(ctx);
        let new_selections = current_selections.mapped(|selection| {
            let start_pos = selection.tail.min(selection.head);
            let end_pos = selection.tail.max(selection.head);
            let start_point = start_pos.to_buffer_point(buffer);
            let end_point = end_pos.to_buffer_point(buffer);
            let line_start = Point::new(start_point.row, 0).to_buffer_char_offset(buffer);
            let start_of_end_line = Point::new(end_point.row, 0).to_buffer_char_offset(buffer);
            let line_end = if include_newline {
                buffer
                    .containing_line_end(start_of_end_line)
                    .min(buffer.max_charoffset())
            } else {
                buffer.containing_line_end(start_of_end_line) - 1
            };
            SelectionOffsets {
                head: line_end,
                tail: line_start,
            }
        });
        self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
    }

    fn vim_select_for_operand(
        &mut self,
        operator: &VimOperator,
        operand_count: u32,
        operand: &VimOperand,
        ctx: &mut ModelContext<Self>,
    ) {
        match operand {
            VimOperand::Motion {
                motion,
                motion_type: _,
            } => match motion {
                VimMotion::Character(m) => self.vim_navigate_char(operand_count, m, true, ctx),
                VimMotion::Word(m) => {
                    let select = true;
                    match m.direction {
                        Direction::Forward => {
                            for _ in 0..operand_count {
                                self.forward_word(select, ctx);
                            }
                        }
                        Direction::Backward => {
                            for _ in 0..operand_count {
                                self.backward_word(select, ctx);
                            }
                        }
                    }
                }
                VimMotion::Line(m) => match m {
                    LineMotion::Start => self.vim_move_to_line_bound(true, true, ctx),
                    LineMotion::FirstNonWhitespace => {
                        self.vim_move_to_first_nonwhitespace(true, ctx)
                    }
                    LineMotion::End => {
                        self.vim_move_vertical(
                            operand_count.saturating_sub(1),
                            TextDirection::Forwards,
                            true,
                            ctx,
                        );
                        self.vim_move_to_line_bound(false, true, ctx);
                    }
                },
                VimMotion::FirstNonWhitespace(m) => {
                    match m {
                        FirstNonWhitespaceMotion::Up => {
                            self.vim_move_vertical(
                                operand_count,
                                TextDirection::Backwards,
                                true,
                                ctx,
                            );
                        }
                        FirstNonWhitespaceMotion::Down => {
                            self.vim_move_vertical(
                                operand_count,
                                TextDirection::Forwards,
                                true,
                                ctx,
                            );
                        }
                        FirstNonWhitespaceMotion::DownMinusOne => {
                            self.vim_move_vertical(
                                operand_count.saturating_sub(1),
                                TextDirection::Forwards,
                                true,
                                ctx,
                            );
                        }
                    }
                    self.vim_move_to_first_nonwhitespace(true, ctx);
                }
                VimMotion::Paragraph(direction) => {
                    self.vim_move_by_paragraph(operand_count, direction, true, ctx);
                }
                VimMotion::JumpToLastLine => self.vim_jump_to_last_line(ctx),
                VimMotion::JumpToFirstLine => self.vim_jump_to_first_line(ctx),
                VimMotion::FindChar(m) => self.vim_find_char(operand_count, m, true, ctx),
                VimMotion::JumpToLine(line_number) => self.vim_jump_to_line(*line_number, ctx),
                VimMotion::JumpToMatchingBracket | VimMotion::JumpToUnmatchedBracket(_) => {}
            },
            VimOperand::Line => {
                if operand_count > 1 {
                    self.vim_move_vertical(operand_count - 1, TextDirection::Forwards, true, ctx);
                }
                self.vim_extend_selection_linewise(operator.includes_trailing_newline(), ctx);
            }
            VimOperand::TextObject(_) => {}
        }
        if let VimOperand::Motion {
            motion_type: MotionType::Linewise,
            ..
        } = operand
        {
            self.vim_extend_selection_linewise(operator.includes_trailing_newline(), ctx);
        }
    }
}

impl VimBufferOps for NotebooksEditorModel {
    type Ctx<'a> = ModelContext<'a, Self>;

    fn select_for_operand(
        &mut self,
        operator: &VimOperator,
        operand_count: u32,
        operand: &VimOperand,
        ctx: &mut Self::Ctx<'_>,
    ) {
        self.vim_select_for_operand(operator, operand_count, operand, ctx);
    }

    fn selected_text(&mut self, ctx: &mut Self::Ctx<'_>) -> String {
        self.content()
            .as_ref(ctx)
            .selected_text_as_plain_text(self.buffer_selection_model().clone(), ctx)
            .into_string()
    }

    fn delete_selection(&mut self, ctx: &mut Self::Ctx<'_>) {
        self.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
    }

    fn insert_text(&mut self, text: &str, ctx: &mut Self::Ctx<'_>) {
        self.insert(text, EditOrigin::UserInitiated, ctx);
    }

    fn move_to_line_start(&mut self, ctx: &mut Self::Ctx<'_>) {
        self.vim_move_to_line_bound(true, false, ctx);
    }

    fn stash_selections(&mut self, ctx: &mut Self::Ctx<'_>) {
        self.vim_selection_stash = Some(self.selections(ctx).clone());
    }

    fn restore_stashed_selections(&mut self, ctx: &mut Self::Ctx<'_>) {
        if let Some(selections) = self.vim_selection_stash.take() {
            self.vim_set_selections(selections, AutoScrollBehavior::None, ctx);
        }
    }

    fn collapse_to_selection_start(&mut self, ctx: &mut Self::Ctx<'_>) {
        let starts = self.selections(ctx).mapped(|selection| {
            let start = selection.head.min(selection.tail);
            SelectionOffsets {
                head: start,
                tail: start,
            }
        });
        self.vim_set_selections(starts, AutoScrollBehavior::None, ctx);
    }

    fn transform_case(&mut self, transform: CaseTransform, ctx: &mut Self::Ctx<'_>) {
        let buffer = self.content().as_ref(ctx);
        let ranges = self
            .buffer_selection_model()
            .as_ref(ctx)
            .selections_to_offset_ranges();
        let edits_vec: Vec<(String, std::ops::Range<CharOffset>)> = ranges
            .iter()
            .map(|range| {
                let original = buffer.text_in_range(range.clone()).into_string();
                (transform.apply_to(&original), range.clone())
            })
            .collect();
        if let Ok(edits) = Vec1::try_from_vec(edits_vec) {
            let selection_model = self.buffer_selection_model().clone();
            self.update_content(
                |mut content, ctx| {
                    content.apply_edit(
                        BufferEditAction::InsertAtCharOffsetRanges { edits: &edits },
                        EditOrigin::UserInitiated,
                        selection_model,
                        ctx,
                    );
                },
                ctx,
            );
        }
        self.clear_selections(ctx);
    }

    fn indent(&mut self, dedent: bool, ctx: &mut Self::Ctx<'_>) {
        let selection_model = self.buffer_selection_model().clone();
        self.update_content(
            |mut content, ctx| {
                content.apply_edit(
                    BufferEditAction::Indent {
                        num_unit: 1,
                        shift: dedent,
                    },
                    EditOrigin::UserInitiated,
                    selection_model,
                    ctx,
                );
            },
            ctx,
        );
    }

    fn move_to_first_nonwhitespace(&mut self, ctx: &mut Self::Ctx<'_>) {
        self.vim_move_to_first_nonwhitespace(false, ctx);
    }

    fn expand_visual_selection(
        &mut self,
        motion_type: MotionType,
        include_newline: bool,
        ctx: &mut Self::Ctx<'_>,
    ) {
        let selection_model = self.buffer_selection_model().as_ref(ctx);
        let buffer = self.content().as_ref(ctx);
        let tails = std::mem::take(&mut self.vim_visual_tails);
        let new_selections = selection_model
            .selection_offsets()
            .iter()
            .zip(tails.iter().chain(std::iter::repeat(&CharOffset::from(1))))
            .map(|(selection, visual_tail)| {
                let mut start = *visual_tail;
                let mut end = selection.head;
                if start > end {
                    std::mem::swap(&mut start, &mut end);
                }
                if end < buffer.max_charoffset() {
                    end += 1;
                }
                if motion_type == MotionType::Linewise {
                    let start_point = start.to_buffer_point(buffer);
                    start = Point::new(start_point.row, 0).to_buffer_char_offset(buffer);
                    let end_point = end.to_buffer_point(buffer);
                    end = Point::new(end_point.row, buffer.line_len(end_point.row))
                        .to_buffer_char_offset(buffer);
                    if include_newline && end < buffer.max_charoffset() {
                        end += 1;
                    }
                }
                SelectionOffsets {
                    head: start,
                    tail: end,
                }
            })
            .collect();
        if let Ok(new_selections) = Vec1::try_from_vec(new_selections) {
            self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
        }
    }

    fn clear_selections(&mut self, ctx: &mut Self::Ctx<'_>) {
        let first = *self.selections(ctx).first();
        self.vim_set_selections(
            Vec1::new(SelectionOffsets {
                head: first.head,
                tail: first.head,
            }),
            AutoScrollBehavior::None,
            ctx,
        );
    }

    fn apply_insert_position(&mut self, position: &InsertPosition, ctx: &mut Self::Ctx<'_>) {
        match position {
            InsertPosition::AtCursor => {}
            InsertPosition::AfterCursor => {
                self.vim_move_horizontal(1, &Direction::Forward, false, true, ctx);
            }
            InsertPosition::LineFirstNonWhitespace => {
                self.vim_move_to_first_nonwhitespace(false, ctx);
            }
            InsertPosition::LineEnd => self.vim_move_to_line_bound(false, false, ctx),
            InsertPosition::LineAbove => {
                self.vim_move_to_line_bound(true, false, ctx);
                self.insert("\n", EditOrigin::UserInitiated, ctx);
            }
            InsertPosition::LineBelow => {
                self.newline(ctx);
            }
        }
    }

    fn move_left_exiting_insert(&mut self, ctx: &mut Self::Ctx<'_>) {
        self.vim_move_horizontal(1, &Direction::Backward, false, true, ctx);
    }

    fn enforce_cursor_line_cap(&mut self, ctx: &mut Self::Ctx<'_>) {
        let buffer = self.content().as_ref(ctx);
        let current_selections = self.selections(ctx);
        let new_selections = current_selections.mapped(|selection| {
            let head_point = selection.head.to_buffer_point(buffer);
            let line_len = buffer.line_len(head_point.row);
            if line_len > 0 && head_point.column >= line_len {
                SelectionOffsets {
                    head: selection.head.saturating_sub(&CharOffset::from(1)),
                    tail: selection.tail.saturating_sub(&CharOffset::from(1)),
                }
            } else {
                selection
            }
        });
        self.vim_set_selections(new_selections, AutoScrollBehavior::None, ctx);
    }

    fn set_visual_tails_to_heads(&mut self, ctx: &mut Self::Ctx<'_>) {
        self.vim_visual_tails = self.selections(ctx).iter().map(|s| s.head).collect();
    }

    fn enforce_normal_mode_line_cap(&mut self, ctx: &mut Self::Ctx<'_>) {
        self.enforce_cursor_line_cap(ctx);
    }

    fn smart_indent_on_linewise_change(&self, _operand: &VimOperand) -> bool {
        false
    }

    fn supports_operator(&self, operator: &VimOperator) -> bool {
        matches!(
            operator,
            VimOperator::Delete
                | VimOperator::Change
                | VimOperator::Yank
                | VimOperator::ToggleCase
                | VimOperator::Uppercase
                | VimOperator::Lowercase
                | VimOperator::Indent
                | VimOperator::Dedent
        )
    }
}
