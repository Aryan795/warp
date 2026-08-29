use crate::handler::{
    CaseTransform, VimBufferOps, YankedText, apply_mode_change, apply_operator,
    apply_visual_operator, apply_visual_paste, operand_motion_type,
};
use crate::vim::{
    InsertPosition, ModeTransition, MotionType, TextObjectInclusion, TextObjectType, VimMode,
    VimOperand, VimOperator, VimTextObject, WordBound, WordMotion, WordType,
};

#[derive(Default)]
struct FakeBuffer {
    selected: String,
    deleted: bool,
    inserted: Option<String>,
    smart_indented: bool,
    moved_to_line_start: bool,
    stashed: bool,
    restored: bool,
    collapsed: bool,
    case_transform: Option<CaseTransform>,
    comments_toggled: bool,
    indented: Option<bool>,
    moved_to_first_nonws: bool,
    visual_expanded: Option<(MotionType, bool)>,
    selections_cleared: bool,
    insert_position: Option<InsertPosition>,
    moved_left_exiting_insert: bool,
    line_capped: bool,
    visual_tails_set: bool,
    normal_line_capped: bool,
    supported_operators: Option<Vec<VimOperator>>,
    smart_indent_linewise: bool,
    collapse_text_object_yank: bool,
    last_operand: Option<VimOperand>,
}

impl FakeBuffer {
    fn supporting(operators: Vec<VimOperator>) -> Self {
        Self {
            supported_operators: Some(operators),
            smart_indent_linewise: true,
            collapse_text_object_yank: true,
            ..Self::default()
        }
    }
}

impl VimBufferOps for FakeBuffer {
    type Ctx<'a> = ();

    fn select_for_operand(
        &mut self,
        _operator: &VimOperator,
        _operand_count: u32,
        operand: &VimOperand,
        _ctx: &mut Self::Ctx<'_>,
    ) {
        self.last_operand = Some(operand.clone());
    }

    fn selected_text(&mut self, _ctx: &mut Self::Ctx<'_>) -> String {
        self.selected.clone()
    }

    fn delete_selection(&mut self, _ctx: &mut Self::Ctx<'_>) {
        self.deleted = true;
        self.selected.clear();
    }

    fn insert_text(&mut self, text: &str, _ctx: &mut Self::Ctx<'_>) {
        self.inserted = Some(text.to_owned());
    }

    fn change_line_smart_indent(&mut self, _ctx: &mut Self::Ctx<'_>) {
        self.smart_indented = true;
    }

    fn move_to_line_start(&mut self, _ctx: &mut Self::Ctx<'_>) {
        self.moved_to_line_start = true;
    }

    fn stash_selections(&mut self, _ctx: &mut Self::Ctx<'_>) {
        self.stashed = true;
    }

    fn restore_stashed_selections(&mut self, _ctx: &mut Self::Ctx<'_>) {
        self.restored = true;
    }

    fn collapse_to_selection_start(&mut self, _ctx: &mut Self::Ctx<'_>) {
        self.collapsed = true;
    }

    fn transform_case(&mut self, transform: CaseTransform, _ctx: &mut Self::Ctx<'_>) {
        self.case_transform = Some(transform);
    }

    fn toggle_comments(&mut self, _ctx: &mut Self::Ctx<'_>) {
        self.comments_toggled = true;
    }

    fn indent(&mut self, dedent: bool, _ctx: &mut Self::Ctx<'_>) {
        self.indented = Some(dedent);
    }

    fn move_to_first_nonwhitespace(&mut self, _ctx: &mut Self::Ctx<'_>) {
        self.moved_to_first_nonws = true;
    }

    fn expand_visual_selection(
        &mut self,
        motion_type: MotionType,
        include_newline: bool,
        _ctx: &mut Self::Ctx<'_>,
    ) {
        self.visual_expanded = Some((motion_type, include_newline));
    }

    fn clear_selections(&mut self, _ctx: &mut Self::Ctx<'_>) {
        self.selections_cleared = true;
    }

    fn apply_insert_position(&mut self, position: &InsertPosition, _ctx: &mut Self::Ctx<'_>) {
        self.insert_position = Some(*position);
    }

    fn move_left_exiting_insert(&mut self, _ctx: &mut Self::Ctx<'_>) {
        self.moved_left_exiting_insert = true;
    }

    fn enforce_cursor_line_cap(&mut self, _ctx: &mut Self::Ctx<'_>) {
        self.line_capped = true;
    }

    fn set_visual_tails_to_heads(&mut self, _ctx: &mut Self::Ctx<'_>) {
        self.visual_tails_set = true;
    }

    fn enforce_normal_mode_line_cap(&mut self, _ctx: &mut Self::Ctx<'_>) {
        self.normal_line_capped = true;
    }

    fn supports_operator(&self, operator: &VimOperator) -> bool {
        self.supported_operators
            .as_ref()
            .is_none_or(|ops| ops.contains(operator))
    }

    fn smart_indent_on_linewise_change(&self, _operand: &VimOperand) -> bool {
        self.smart_indent_linewise
    }

    fn collapse_yank_for_text_object(&self) -> bool {
        self.collapse_text_object_yank
    }
}

fn word_operand() -> VimOperand {
    VimOperand::Motion {
        motion_type: MotionType::Charwise,
        motion: crate::vim::VimMotion::Word(WordMotion::new(
            crate::vim::Direction::Forward,
            WordBound::Start,
            WordType::Default,
        )),
    }
}

#[test]
fn operand_motion_type_treats_paragraph_objects_as_linewise() {
    let paragraph = VimOperand::TextObject(VimTextObject {
        inclusion: TextObjectInclusion::Inner,
        object_type: TextObjectType::Paragraph,
    });
    let word = VimOperand::TextObject(VimTextObject {
        inclusion: TextObjectInclusion::Inner,
        object_type: TextObjectType::Word(WordType::Default),
    });

    assert_eq!(operand_motion_type(&paragraph), MotionType::Linewise);
    assert_eq!(operand_motion_type(&word), MotionType::Charwise);
    assert_eq!(operand_motion_type(&VimOperand::Line), MotionType::Linewise);
}

#[test]
fn delete_yanks_then_deletes_selection() {
    let mut buffer = FakeBuffer {
        selected: "abc".into(),
        ..FakeBuffer::default()
    };

    let yanked = apply_operator(
        &mut buffer,
        &VimOperator::Delete,
        1,
        &word_operand(),
        "",
        &mut (),
    );

    assert_eq!(
        yanked,
        Some(YankedText {
            text: "abc".into(),
            motion_type: MotionType::Charwise,
        })
    );
    assert!(buffer.deleted);
    assert!(!buffer.smart_indented);
}

#[test]
fn empty_linewise_delete_yanks_newline_without_deleting() {
    let mut buffer = FakeBuffer::default();

    let yanked = apply_operator(
        &mut buffer,
        &VimOperator::Delete,
        1,
        &VimOperand::Line,
        "",
        &mut (),
    );

    assert_eq!(
        yanked,
        Some(YankedText {
            text: "\n".into(),
            motion_type: MotionType::Linewise,
        })
    );
    assert!(!buffer.deleted);
}

#[test]
fn linewise_change_uses_smart_indent() {
    let mut buffer = FakeBuffer {
        selected: "line\n".into(),
        smart_indent_linewise: true,
        ..FakeBuffer::default()
    };

    apply_operator(
        &mut buffer,
        &VimOperator::Change,
        1,
        &VimOperand::Line,
        "",
        &mut (),
    );

    assert!(buffer.smart_indented);
    assert!(!buffer.deleted);
}

#[test]
fn yank_restores_selection_for_motions_and_collapses_text_objects() {
    let mut motion_buf = FakeBuffer {
        selected: "abc".into(),
        collapse_text_object_yank: true,
        ..FakeBuffer::default()
    };
    apply_operator(
        &mut motion_buf,
        &VimOperator::Yank,
        1,
        &word_operand(),
        "",
        &mut (),
    );
    assert!(motion_buf.stashed);
    assert!(motion_buf.restored);
    assert!(!motion_buf.collapsed);

    let mut object_buf = FakeBuffer {
        selected: "abc".into(),
        collapse_text_object_yank: true,
        ..FakeBuffer::default()
    };
    apply_operator(
        &mut object_buf,
        &VimOperator::Yank,
        1,
        &VimOperand::TextObject(VimTextObject {
            inclusion: TextObjectInclusion::Inner,
            object_type: TextObjectType::Word(WordType::Default),
        }),
        "",
        &mut (),
    );
    assert!(object_buf.collapsed);
    assert!(!object_buf.restored);
}

#[test]
fn unsupported_operators_are_no_ops() {
    let mut buffer = FakeBuffer::supporting(vec![VimOperator::Delete, VimOperator::Yank]);
    buffer.selected = "abc".into();

    let yanked = apply_operator(
        &mut buffer,
        &VimOperator::ToggleComment,
        1,
        &VimOperand::Line,
        "",
        &mut (),
    );

    assert!(yanked.is_none());
    assert!(!buffer.comments_toggled);
    assert!(!buffer.deleted);
    assert!(buffer.last_operand.is_none());
    assert!(!buffer.stashed);
    assert!(!buffer.moved_to_first_nonws);
}

#[test]
fn unsupported_visual_operators_do_not_expand_or_mutate() {
    let mut buffer = FakeBuffer::supporting(vec![VimOperator::Delete, VimOperator::Yank]);
    buffer.selected = "abc".into();

    let yanked = apply_visual_operator(
        &mut buffer,
        &VimOperator::ToggleComment,
        MotionType::Charwise,
        &mut (),
    );

    assert!(yanked.is_none());
    assert!(buffer.visual_expanded.is_none());
    assert!(!buffer.comments_toggled);
    assert!(!buffer.deleted);
    assert!(!buffer.selections_cleared);
}

#[test]
fn visual_delete_expands_then_yanks() {
    let mut buffer = FakeBuffer {
        selected: "vis".into(),
        ..FakeBuffer::default()
    };

    let yanked = apply_visual_operator(
        &mut buffer,
        &VimOperator::Delete,
        MotionType::Charwise,
        &mut (),
    );

    assert_eq!(buffer.visual_expanded, Some((MotionType::Charwise, true)));
    assert!(buffer.deleted);
    assert_eq!(
        yanked,
        Some(YankedText {
            text: "vis".into(),
            motion_type: MotionType::Charwise,
        })
    );
}

#[test]
fn visual_paste_replaces_selection_and_returns_replaced_text() {
    let mut buffer = FakeBuffer {
        selected: "old".into(),
        ..FakeBuffer::default()
    };

    let yanked = apply_visual_paste(
        &mut buffer,
        MotionType::Charwise,
        "new",
        MotionType::Charwise,
        &mut (),
    );

    assert_eq!(buffer.inserted.as_deref(), Some("new"));
    assert_eq!(
        yanked,
        Some(YankedText {
            text: "old".into(),
            motion_type: MotionType::Charwise,
        })
    );
}

#[test]
fn mode_change_applies_insert_exit_and_visual_tails() {
    let mut buffer = FakeBuffer::default();
    apply_mode_change(
        &mut buffer,
        &VimMode::Insert,
        &VimMode::Normal.into(),
        &mut (),
    );
    assert!(buffer.moved_left_exiting_insert);
    assert!(buffer.normal_line_capped);

    let mut buffer = FakeBuffer::default();
    apply_mode_change(
        &mut buffer,
        &VimMode::Normal,
        &ModeTransition {
            mode: VimMode::Insert,
            position: InsertPosition::LineBelow,
        },
        &mut (),
    );
    assert_eq!(buffer.insert_position, Some(InsertPosition::LineBelow));

    let mut buffer = FakeBuffer::default();
    apply_mode_change(
        &mut buffer,
        &VimMode::Normal,
        &ModeTransition {
            mode: VimMode::Visual(MotionType::Charwise),
            position: InsertPosition::AtCursor,
        },
        &mut (),
    );
    assert!(buffer.visual_tails_set);
}
