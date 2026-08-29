use settings::Setting as _;
use vim::vim::VimMode;
use warp_util::user_input::UserInput;
use warpui::{App, SingletonEntity, TypedActionView, UpdateModel, ViewHandle};

use super::tests::initialize_editor;
use super::{EditorViewAction, RichTextEditorView};
use crate::editor::InteractionState;
use crate::settings::AppEditorSettings;
use crate::vim_registers::VimRegisters;

fn enable_vim(editor: &ViewHandle<RichTextEditorView>, app: &mut App) {
    app.add_singleton_model(|_| VimRegisters::new());
    app.update_model(
        &AppEditorSettings::handle(app),
        |settings: &mut AppEditorSettings, ctx| {
            settings.vim_mode.set_value(true, ctx).unwrap();
        },
    );
    editor.update(app, |view, ctx| {
        ctx.focus_self();
        view.set_interaction_state(InteractionState::Editable, ctx);
        view.reset_with_markdown("hello world\nsecond line", ctx);
        view.handle_action(&EditorViewAction::VimEscape, ctx);
    });
}

fn vim_type(editor: &ViewHandle<RichTextEditorView>, text: &str, app: &mut App) {
    editor.update(app, |view, ctx| {
        ctx.focus_self();
        view.handle_action(
            &EditorViewAction::VimUserTyped(UserInput::new(text.to_string())),
            ctx,
        );
    });
}

fn vim_escape(editor: &ViewHandle<RichTextEditorView>, app: &mut App) {
    editor.update(app, |view, ctx| {
        ctx.focus_self();
        view.handle_action(&EditorViewAction::VimEscape, ctx);
    });
}

fn markdown(editor: &ViewHandle<RichTextEditorView>, app: &App) -> String {
    editor.read(app, |view, ctx| view.markdown(ctx))
}

fn vim_mode(editor: &ViewHandle<RichTextEditorView>, app: &App) -> Option<VimMode> {
    editor.read(app, |view, ctx| view.vim_mode(ctx))
}

#[test]
fn notebook_vim_starts_in_normal_mode() {
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim(&editor, &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn notebook_vim_insert_and_escape() {
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim(&editor, &mut app);

        vim_type(&editor, "i", &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));
        vim_type(&editor, "x", &mut app);
        vim_escape(&editor, &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
        let text = markdown(&editor, &app);
        assert!(text.contains('x') || text.contains("hello"));
    });
}

#[test]
fn notebook_vim_delete_line() {
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim(&editor, &mut app);

        vim_type(&editor, "dd", &mut app);
        let _text = markdown(&editor, &app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn notebook_vim_hjkl_motions_stay_in_normal_mode() {
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim(&editor, &mut app);

        vim_type(&editor, "llj", &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn notebook_vim_search_opens_find_bar() {
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim(&editor, &mut app);

        vim_type(&editor, "/", &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn notebook_vim_unsupported_text_object_is_a_noop() {
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim(&editor, &mut app);
        let before = markdown(&editor, &app);

        vim_type(&editor, "diw", &mut app);
        assert_eq!(markdown(&editor, &app), before);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}
