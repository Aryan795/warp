use warpui::integration::{AssertionCallback, TestStep};
use warpui::windowing::WindowManager;
use warpui::{App, SingletonEntity, async_assert};

use super::terminal::assert_no_block_executing;
use crate::integration_testing::view_getters::terminal_view;
use crate::workspace::{Workspace, WorkspaceAction};

pub fn new_step_with_default_assertions(name: &str) -> TestStep {
    new_step_with_default_assertions_for_pane(name, 0, 0)
}

pub fn new_step_with_default_assertions_for_pane(
    name: &str,
    tab_index: usize,
    pane_index: usize,
) -> TestStep {
    // Add global assertions here
    TestStep::new(name)
        .add_named_assertion(
            "no pending model events",
            assert_no_pending_model_events_for_pane(tab_index, pane_index),
        )
        .add_named_assertion(
            "no block executing",
            assert_no_block_executing(tab_index, pane_index),
        )
}

pub fn assert_no_pending_model_events() -> AssertionCallback {
    assert_no_pending_model_events_for_pane(0, 0)
}

pub fn assert_no_pending_model_events_for_pane(
    tab_index: usize,
    pane_index: usize,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, tab_index, pane_index);
        terminal_view.read(app, |view, _ctx| {
            let model = view.model.lock();
            log::info!("events pending {}", model.are_any_events_pending());
            async_assert!(
                !model.are_any_events_pending(),
                "Should not be any pending model events",
            )
        })
    })
}

/// Dispatches a [`WorkspaceAction`] against the active window's workspace view.
pub(crate) fn dispatch_workspace_action(app: &mut App, action: WorkspaceAction) {
    let window_id = app.read(|ctx| {
        WindowManager::as_ref(ctx)
            .active_window()
            .expect("no active window")
    });
    let workspace_view_id = app
        .views_of_type::<Workspace>(window_id)
        .and_then(|views| views.first().map(|view| view.id()))
        .expect("no workspace view");
    app.dispatch_typed_action(window_id, &[workspace_view_id], &action);
}

/// Asserts whether the element saved under `position_id` was painted in the
/// most recent frame.
///
/// An element that saves its position for a single frame is only in the
/// position cache while it is currently rendered. Reading visibility this way
/// means the assertion is checking what was actually drawn, rather than
/// re-deriving the conditions behind it and risking drift from `render`.
pub(crate) fn assert_element_painted(
    position_id: String,
    description: String,
    visible: bool,
) -> TestStep {
    TestStep::new(&format!("Assert {description} visible is {visible}")).add_named_assertion(
        format!("{description} visible is {visible}"),
        move |app: &mut App, window_id| {
            let painted = app.presenter(window_id).is_some_and(|presenter| {
                presenter
                    .borrow()
                    .position_cache()
                    .get_position(&position_id)
                    .is_some()
            });
            async_assert!(
                painted == visible,
                "{description} visible should be {visible}, was {painted}"
            )
        },
    )
}
