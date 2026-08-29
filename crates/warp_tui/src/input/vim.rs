//! [`VimHandler`] implementation for [`TuiInputView`].
//!
//! Wires the TUI prompt's backing [`CodeEditorModel`] into the shared vim
//! dispatch layer (the same pattern [`CodeEditorView`] uses).  Prompt-specific
//! semantics are expressed as explicit no-ops or custom overrides in the trait
//! implementation rather than as arms in a bespoke match:
//!
//! - `find_char` — no-op (single-line prompt; `f`/`F`/`t`/`T` are skipped).
//! - `navigate_paragraph` — no-op (no paragraph structure in a prompt).
//! - `jump_to_*_bracket` — no-op.
//! - `search`, `cycle_search`, `search_word_at_cursor` — no-op.
//! - `visual_paste` — no-op (use the plain `paste` method; the TUI has no register system).
//! - `join_line`, `toggle_case`, `keyword_prg`, `ex_command` — no-op.
//! - Scroll helpers (`center_cursor_vertically`, `scroll_half_page_*`) — no-op.
//!

use vim::handler::{apply_mode_change, apply_operator, apply_visual_operator};
use vim::vim::{
    BracketChar, CharacterMotion, Direction, FindCharMotion, FirstNonWhitespaceMotion,
    InsertPosition, LineMotion, ModeTransition, MotionType, VimHandler, VimMode, VimOperand,
    VimOperator, VimTextObject, WordMotion,
};
use warp::editor::LineBound;
use warp_editor::model::{CoreEditorModel, PlainTextEditorModel};
use warp_editor::selection::{TextDirection, TextUnit};
use warpui_core::ViewContext;

use super::TuiInputView;
const MAX_VIM_PASTE_BYTES: usize = 1024 * 1024;

impl VimHandler for TuiInputView {
    // ── Character insertion ───────────────────────────────────────────────────

    fn insert_char(&mut self, c: char, ctx: &mut ViewContext<Self>) {
        if c == '!'
            && !self.is_shell_mode(ctx)
            && self.is_cursor_at_start(ctx)
            && !self
                .input_mode
                .as_ref(ctx)
                .is_terminal_use_active_or_pending()
        {
            self.enter_shell_mode(ctx);
            ctx.notify();
            return;
        }
        let c_str = c.to_string();
        self.model.update(ctx, |m, ctx| m.user_insert(&c_str, ctx));
        self.follow_cursor(ctx);
        ctx.notify();
    }

    // ── Navigation ────────────────────────────────────────────────────────────

    fn navigate_char(
        &mut self,
        count: u32,
        character_motion: &CharacterMotion,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model.update(ctx, |model, ctx| match character_motion {
            CharacterMotion::Right | CharacterMotion::WrappingRight => {
                model.vim_move_horizontal_by_offset(count, &Direction::Forward, false, true, ctx);
            }
            CharacterMotion::Left | CharacterMotion::WrappingLeft => {
                model.vim_move_horizontal_by_offset(count, &Direction::Backward, false, true, ctx);
            }
            CharacterMotion::Up => {
                model.vim_move_vertical_by_offset(count, TextDirection::Backwards, false, ctx);
            }
            CharacterMotion::Down => {
                model.vim_move_vertical_by_offset(count, TextDirection::Forwards, false, ctx);
            }
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn replace_text(
        &mut self,
        text: &str,
        count: u32,
        already_applied: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let repeat_count = count.saturating_sub(u32::from(already_applied));
        self.model.update(ctx, |model, ctx| {
            if repeat_count > 0 {
                model.vim_replace_text(&text.repeat(repeat_count as usize), ctx);
            }
            if !text.is_empty() {
                model.vim_move_horizontal_by_offset(1, &Direction::Backward, false, true, ctx);
            }
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn navigate_word(&mut self, count: u32, word_motion: &WordMotion, ctx: &mut ViewContext<Self>) {
        let WordMotion {
            direction,
            bound,
            word_type,
        } = word_motion;
        self.model.update(ctx, |model, ctx| {
            model.vim_navigate_word(*direction, *bound, *word_type, count, ctx);
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn navigate_line(&mut self, line_count: u32, motion: &LineMotion, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| match motion {
            LineMotion::Start => model.vim_move_to_line_bound(LineBound::Start, false, ctx),
            LineMotion::FirstNonWhitespace => model.vim_move_to_first_nonwhitespace(false, ctx),
            LineMotion::End => {
                model.vim_move_vertical_by_offset(
                    line_count.saturating_sub(1),
                    TextDirection::Forwards,
                    false,
                    ctx,
                );
                model.vim_move_to_line_bound(LineBound::End, false, ctx);
            }
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn first_nonwhitespace_motion(
        &mut self,
        count: u32,
        motion: &FirstNonWhitespaceMotion,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model.update(ctx, |model, ctx| {
            match motion {
                FirstNonWhitespaceMotion::Up => {
                    model.vim_move_vertical_by_offset(count, TextDirection::Backwards, false, ctx);
                }
                FirstNonWhitespaceMotion::Down => {
                    model.vim_move_vertical_by_offset(count, TextDirection::Forwards, false, ctx);
                }
                FirstNonWhitespaceMotion::DownMinusOne => {
                    model.vim_move_vertical_by_offset(
                        count - 1,
                        TextDirection::Forwards,
                        false,
                        ctx,
                    );
                }
            }
            model.vim_move_to_first_nonwhitespace(false, ctx);
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }

    /// Prompt-specific: `f`/`F`/`t`/`T` are no-ops — single-line prompt
    /// makes find-char useful only when the cursor is at the start of a long
    /// line, and TUI's existing horizontal navigation covers that.
    fn find_char(
        &mut self,
        _occurrence_count: u32,
        _find_char_motion: &FindCharMotion,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.notify();
    }

    /// Prompt-specific: `{` / `}` are no-ops — no paragraph structure.
    fn navigate_paragraph(
        &mut self,
        _count: u32,
        _direction: &Direction,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.notify();
    }

    // ── Operators ─────────────────────────────────────────────────────────────

    fn operation(
        &mut self,
        operator: &VimOperator,
        operand_count: u32,
        operand: &VimOperand,
        _register_name: char,
        replacement_text: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        if !tui_prompt_supports_operator(operator) {
            ctx.notify();
            return;
        }
        let yanked = self.model.update(ctx, |model, ctx| {
            apply_operator(
                model,
                operator,
                operand_count,
                operand,
                replacement_text,
                ctx,
            )
        });
        if let Some(yanked) = yanked {
            self.yank_buffer = yanked.text;
            self.yank_motion_type = yanked.motion_type;
        }
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn replace_char(
        &mut self,
        c: char,
        char_count: u32,
        advance: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model.update(ctx, |model, ctx| {
            if advance {
                model.vim_replace_text(&c.to_string(), ctx);
            } else {
                model.replace_char(c, char_count, ctx);
            }
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }

    /// Prompt-specific: case-toggle is a no-op in the TUI prompt.
    fn toggle_case(&mut self, _char_count: u32, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    // ── Search (no-op in TUI prompt) ──────────────────────────────────────────

    fn search(&mut self, _direction: &Direction, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    fn cycle_search(&mut self, _direction: &Direction, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    fn search_word_at_cursor(&mut self, _direction: &Direction, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    fn ex_command(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    fn keyword_prg(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    // ── Visual mode operators ─────────────────────────────────────────────────

    fn visual_operator(
        &mut self,
        operator: &VimOperator,
        motion_type: MotionType,
        _register_name: char,
        ctx: &mut ViewContext<Self>,
    ) {
        if !tui_prompt_supports_operator(operator) {
            ctx.notify();
            return;
        }
        let yanked = self.model.update(ctx, |model, ctx| {
            apply_visual_operator(model, operator, motion_type, ctx)
        });
        if let Some(yanked) = yanked {
            self.yank_buffer = yanked.text;
            self.yank_motion_type = yanked.motion_type;
        }
        self.follow_cursor(ctx);
        ctx.notify();
    }

    /// Prompt-specific: visual paste is a no-op for TUI; use the plain `paste` method instead.
    fn visual_paste(
        &mut self,
        _motion_type: MotionType,
        _read_register_name: char,
        _write_register_name: char,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.notify();
    }

    /// Prompt-specific: visual text-object selection is a no-op.
    fn visual_text_object(&mut self, _text_object: &VimTextObject, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    // ── Jumps ─────────────────────────────────────────────────────────────────

    fn jump_to_first_line(&mut self, ctx: &mut ViewContext<Self>) {
        self.model
            .update(ctx, |model, ctx| model.jump_to_line_column(0, Some(0), ctx));
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn jump_to_last_line(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.vim_move_to_last_line(ctx);
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }
    fn jump_to_line(&mut self, line_number: u32, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            let last_line = model.content().as_ref(ctx).max_point().row as usize;
            let line = line_number.max(1) as usize;
            model.jump_to_line_column(line.min(last_line), Some(0), ctx);
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }

    /// Prompt-specific: matching-bracket jump is a no-op.
    fn jump_to_matching_bracket(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    /// Prompt-specific: unmatched-bracket jump is a no-op.
    fn jump_to_unmatched_bracket(&mut self, _bracket: &BracketChar, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    // ── Paste ─────────────────────────────────────────────────────────────────

    fn paste(
        &mut self,
        count: u32,
        direction: &Direction,
        _register_name: char,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.yank_buffer.is_empty() {
            ctx.notify();
            return;
        }
        let text = bounded_repeated_text(&self.yank_buffer, count);
        if self.yank_motion_type == MotionType::Linewise {
            let text = text.trim_matches('\n');
            let insertion = match direction {
                Direction::Forward => format!("\n{text}"),
                Direction::Backward => format!("{text}\n"),
            };
            self.model.update(ctx, |model, ctx| {
                model.vim_move_to_line_bound(
                    match direction {
                        Direction::Forward => LineBound::End,
                        Direction::Backward => LineBound::Start,
                    },
                    false,
                    ctx,
                );
                model.user_insert(&insertion, ctx);
            });
        } else {
            match direction {
                Direction::Forward => {
                    self.model.update(ctx, |model, ctx| model.move_right(ctx));
                    self.model
                        .update(ctx, |model, ctx| model.user_insert(&text, ctx));
                }
                Direction::Backward => {
                    self.model
                        .update(ctx, |model, ctx| model.user_insert(&text, ctx));
                }
            }
        }
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn insert_text(
        &mut self,
        text: &str,
        position: &InsertPosition,
        count: u32,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model.update(ctx, |model, ctx| {
            model.vim_insert_text(text, position, count, ctx);
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }

    // ── Miscellaneous ─────────────────────────────────────────────────────────

    fn join_line(&mut self, _count: u32, ctx: &mut ViewContext<Self>) {
        // No-op in TUI prompt.
        ctx.notify();
    }

    fn undo(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |m, ctx| m.undo(ctx));
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn change_mode(&mut self, old: &VimMode, new: &ModeTransition, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            apply_mode_change(model, old, new, ctx);
            // Char-cell `vim_newline(false)` leaves the cursor on the original line.
            if new.mode == VimMode::Insert && new.position == InsertPosition::LineBelow {
                model.move_right(ctx);
            }
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn backspace(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |m, ctx| m.backspace(ctx));
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn delete_forward(&mut self, ctx: &mut ViewContext<Self>) {
        // PlainTextEditorModel::delete is in scope via the import above.
        self.model.update(ctx, |m, ctx| {
            m.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn escape(&mut self, ctx: &mut ViewContext<Self>) {
        // All escape routing (menu dismissal, shell-mode exit, etc.) is handled
        // by `handle_escape` before the keystroke reaches the vim model.
        // By the time this fires the FSA has already consumed the Escape key
        // (e.g. clearing pending showcmd in Normal mode); the dispatch wrapper
        // observes any mode transition and notifies the footer. Nothing more
        // to do here.
        ctx.notify();
    }
}

fn tui_prompt_supports_operator(operator: &VimOperator) -> bool {
    matches!(
        operator,
        VimOperator::Delete | VimOperator::Change | VimOperator::Yank
    )
}

fn bounded_repeated_text(text: &str, count: u32) -> String {
    let max_count = (MAX_VIM_PASTE_BYTES / text.len().max(1)).max(1);
    text.repeat((count as usize).min(max_count))
}
