use crate::vim::{
    InsertPosition, ModeTransition, MotionType, TextObjectType, VimMode, VimOperand, VimOperator,
    VimTextObject,
};

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

/// Buffer-level primitives used by shared operator / visual / mode glue.
///
/// Implement this on editor models. Views stay thin: they run the shared glue
/// inside a model update and handle registers, search, and other view hooks.
pub trait VimBufferOps {
    type Ctx<'a>
    where
        Self: 'a;

    fn select_for_operand(
        &mut self,
        operator: &VimOperator,
        operand_count: u32,
        operand: &VimOperand,
        ctx: &mut Self::Ctx<'_>,
    );

    fn selected_text(&mut self, ctx: &mut Self::Ctx<'_>) -> String;

    fn delete_selection(&mut self, ctx: &mut Self::Ctx<'_>);

    fn insert_text(&mut self, text: &str, ctx: &mut Self::Ctx<'_>);

    fn change_line_smart_indent(&mut self, _ctx: &mut Self::Ctx<'_>) {}

    fn move_to_line_start(&mut self, _ctx: &mut Self::Ctx<'_>) {}

    fn stash_selections(&mut self, _ctx: &mut Self::Ctx<'_>) {}

    fn restore_stashed_selections(&mut self, _ctx: &mut Self::Ctx<'_>) {}

    fn collapse_to_selection_start(&mut self, _ctx: &mut Self::Ctx<'_>) {}

    fn transform_case(&mut self, _transform: CaseTransform, _ctx: &mut Self::Ctx<'_>) {}

    fn toggle_comments(&mut self, _ctx: &mut Self::Ctx<'_>) {}

    fn indent(&mut self, _dedent: bool, _ctx: &mut Self::Ctx<'_>) {}

    fn move_to_first_nonwhitespace(&mut self, _ctx: &mut Self::Ctx<'_>) {}

    fn expand_visual_selection(
        &mut self,
        _motion_type: MotionType,
        _include_newline: bool,
        _ctx: &mut Self::Ctx<'_>,
    ) {
    }

    fn clear_selections(&mut self, _ctx: &mut Self::Ctx<'_>) {}

    fn apply_insert_position(&mut self, _position: &InsertPosition, _ctx: &mut Self::Ctx<'_>) {}

    fn move_left_exiting_insert(&mut self, _ctx: &mut Self::Ctx<'_>) {}

    fn enforce_cursor_line_cap(&mut self, _ctx: &mut Self::Ctx<'_>) {}

    fn set_visual_tails_to_heads(&mut self, _ctx: &mut Self::Ctx<'_>) {}

    fn enforce_normal_mode_line_cap(&mut self, _ctx: &mut Self::Ctx<'_>) {}

    fn supports_operator(&self, _operator: &VimOperator) -> bool {
        true
    }

    fn smart_indent_on_linewise_change(&self, _operand: &VimOperand) -> bool {
        true
    }

    fn collapse_yank_for_text_object(&self) -> bool {
        true
    }
}

/// Apply a normal-mode operator (`d`/`c`/`y`/case/comment/indent) to `operand`.
///
/// Returns yanked text when the operator writes a register.
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
            buffer.stash_selections(ctx);
            buffer.select_for_operand(operator, operand_count, operand, ctx);
            let selected_text = buffer.selected_text(ctx);
            let yanked = register_text_for_yank(&selected_text, motion_type)
                .map(|text| YankedText { text, motion_type });
            if matches!(operand, VimOperand::TextObject(_))
                && buffer.collapse_yank_for_text_object()
            {
                buffer.collapse_to_selection_start(ctx);
            } else {
                buffer.restore_stashed_selections(ctx);
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
            buffer.stash_selections(ctx);
            buffer.select_for_operand(operator, operand_count, operand, ctx);
            buffer.toggle_comments(ctx);
            if motion_type == MotionType::Linewise {
                buffer.move_to_first_nonwhitespace(ctx);
            } else {
                buffer.restore_stashed_selections(ctx);
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

/// Replace the visual selection with `paste_text`, returning the replaced text to write back
/// to a register.
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
