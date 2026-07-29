use std::cell::RefCell;
use std::rc::Rc;

use warp::appearance::Appearance;
use warp::tui_export::{
    AttachmentType, PendingAttachmentSummary, register_tui_session_view_test_singletons,
};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, EntityIdMap};
use warpui_core::elements::MouseStateHandle;
use warpui_core::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiLayoutContext, TuiPaintContext, TuiPaintSurface,
    TuiRect, TuiScreenPosition, TuiSize,
};
use warpui_core::{App, AppContext};

use super::{TuiAttachmentBar, TuiAttachmentBarEvent, render_attachment_snapshot};
use crate::attachment_bar::model::{TuiAttachmentModel, TuiAttachmentSnapshot};
use crate::test_fixtures::{TestHostView, add_test_semantic_selection, add_test_terminal_session};

fn render_lines(ctx: &AppContext, snapshot: TuiAttachmentSnapshot, width: u16) -> Vec<String> {
    let mut element = render_attachment_snapshot(
        snapshot,
        false,
        MouseStateHandle::default(),
        MouseStateHandle::default(),
        MouseStateHandle::default(),
        ctx,
    );
    let mut rendered_views = EntityIdMap::default();
    let mut layout_ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let size = element.layout(
        TuiConstraint::loose(TuiSize::new(width, 1)),
        &mut layout_ctx,
        ctx,
    );
    let area = TuiRect::new(0, 0, size.width, size.height);
    let mut buffer = TuiBuffer::empty(area);
    let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
    let mut surface = TuiPaintSurface::new(&mut buffer);
    element.render(
        TuiScreenPosition::new(i32::from(area.x), i32::from(area.y)),
        &mut surface,
        &mut paint_ctx,
    );
    buffer.to_lines()
}

fn snapshot(file_name: &str, position: usize, count: usize) -> TuiAttachmentSnapshot {
    TuiAttachmentSnapshot {
        selected: Some(PendingAttachmentSummary {
            index: position - 1,
            attachment_type: AttachmentType::Image,
            file_name: file_name.to_owned(),
        }),
        position: Some(position),
        count,
        is_processing: false,
        selected_is_processing: false,
    }
}

#[test]
fn renders_single_attachment_without_carousel_arrows() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let line = render_lines(ctx, snapshot("screenshot.png", 1, 1), 60).remove(0);
            assert!(line.contains("[image]"));
            assert!(line.contains("screenshot.png"));
            assert!(line.contains("1/1"));
            assert!(line.contains('×'));
            assert!(!line.contains('‹'));
            assert!(!line.contains('›'));
        });
    });
}

#[test]
fn renders_carousel_position_and_truncates_at_narrow_width() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let line =
                render_lines(ctx, snapshot("a-very-long-screenshot-name.png", 2, 3), 28).remove(0);
            assert!(line.contains("[image]"));
            assert!(line.contains("2/3"));
            assert!(line.contains('‹'));
            assert!(line.contains('›'));
            assert!(line.contains('×'));
            assert!(line.chars().count() <= 28);
        });
    });
}

#[test]
fn empty_snapshot_does_not_render_loading_placeholder() {
    // Regression: when all attachments are removed, `render_attachment_snapshot`
    // must not paint "loading image…" — it should produce a blank element so
    // stale placeholder text doesn't remain visible above the input prompt.
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let lines = render_lines(
                ctx,
                TuiAttachmentSnapshot {
                    selected: None,
                    position: None,
                    count: 0,
                    is_processing: false,
                    selected_is_processing: false,
                },
                60,
            );
            // The empty snapshot must render nothing (0-height element).
            // Before the fix it rendered a single "loading image…" line.
            // A blank one-row gap (lines = [" ... "]) must not satisfy this check.
            assert!(
                lines.is_empty(),
                "empty snapshot must render nothing, got: {lines:?}"
            );
        });
    });
}

#[test]
fn unfocused_last_removal_emits_return_focus_for_parent_relayout() {
    // Regression for the parent re-layout gap: when the attachment bar is
    // **unfocused** and the model emits `Updated` with `should_render()=false`
    // (e.g. after the last attachment is removed via Backspace on empty input),
    // the bar must emit `ReturnFocus` so the parent `TuiTerminalSessionView`
    // receives an event via `handle_attachment_bar_event`, calls `ctx.notify()`,
    // and re-lays out without the bar child in its element tree.
    //
    // This test specifically fails if the `view.focused` guard is restored on
    // the `ReturnFocus` emission — with the guard, an unfocused bar would NOT
    // emit `ReturnFocus`, leaving the parent's old frame (which still includes
    // the bar child) intact and the stale placeholder visible.
    App::test((), |mut app| async move {
        // Minimal session setup required to construct a TuiAttachmentBar
        // backed by a real TuiAttachmentModel.
        register_tui_session_view_test_singletons(&mut app);
        add_test_semantic_selection(&mut app);
        app.update(crate::autoupdate::TuiAutoupdater::register);
        let window_id = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |_| TestHostView,
            )
            .0
        });
        let (view, _) = add_test_terminal_session(&mut app, window_id);

        // Get the attachment bar from the session view and subscribe to its events.
        let attachment_bar = view.read(&app, |view, _| view.attachment_bar_for_test());
        let events: Rc<RefCell<Vec<TuiAttachmentBarEvent>>> = Rc::new(RefCell::new(Vec::new()));
        let events_for_sub = events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(
                &attachment_bar,
                move |_, event: &TuiAttachmentBarEvent, _| {
                    events_for_sub.borrow_mut().push(event.clone());
                },
            );
        });

        // Bar starts unfocused; should_render() is false with no attachments.
        assert!(
            !attachment_bar.read(&app, |bar: &TuiAttachmentBar, _| bar.focused),
            "bar must start unfocused"
        );
        assert!(
            !attachment_bar.read(&app, |bar: &TuiAttachmentBar, ctx| bar.should_render(ctx)),
            "bar must report should_render=false with no attachments"
        );

        // Drive the model's Updated subscription while the bar is unfocused.
        // This simulates the model update emitted after removing the last
        // attachment (e.g. via sync_from_context on BackspaceAtEmptyInput).
        attachment_bar.update(&mut app, |bar: &mut TuiAttachmentBar, ctx| {
            bar.model
                .update(ctx, |model: &mut TuiAttachmentModel, ctx| {
                    model.emit_updated_for_test(ctx);
                });
        });

        // ReturnFocus must be emitted regardless of focus state so the parent
        // re-lays out and drops the bar from its element tree.
        assert!(
            events
                .borrow()
                .iter()
                .any(|e| matches!(e, TuiAttachmentBarEvent::ReturnFocus)),
            "unfocused model-Updated with should_render=false must emit ReturnFocus \
             so the parent re-layouts and the stale bar child is dropped"
        );

        // The bar must still report should_render=false (next layout omits it).
        assert!(
            !attachment_bar.read(&app, |bar: &TuiAttachmentBar, ctx| bar.should_render(ctx)),
            "bar must remain not renderable after removal"
        );
    });
}

#[test]
fn renders_provisional_filename_while_image_is_loading() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let lines = render_lines(
                ctx,
                TuiAttachmentSnapshot {
                    selected: Some(PendingAttachmentSummary {
                        index: 0,
                        attachment_type: AttachmentType::Image,
                        file_name: "clipboard-image.png".to_owned(),
                    }),
                    position: Some(1),
                    count: 1,
                    is_processing: true,
                    selected_is_processing: true,
                },
                40,
            );
            let line = &lines[0];
            assert!(line.contains("[image]"));
            assert!(line.contains("clipboard-image.png"));
            assert!(line.contains("loading…"));
            assert!(!line.contains('×'));
        });
    });
}
