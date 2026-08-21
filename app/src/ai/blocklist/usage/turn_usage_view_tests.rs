//! Regression tests for [`TurnUsageView`]'s collapse/expand and close
//! handlers, following the same `handle_action`-driven pattern as
//! `conversation_usage_view_tests.rs`.

use warp_core::ui::appearance::Appearance;
use warpui::App;
use warpui::platform::WindowStyle;

use super::*;

fn placeholder_usage_info() -> TurnUsageInfo {
    TurnUsageInfo {
        model_id: "auto (cost-efficient)".to_string(),
        tokens: 4,
        cost_in_cents: Some(60.0),
        context_window_usage: 0.001,
        tool_calls: 2,
        files_changed: 1,
        lines_added: 4,
        lines_removed: 1,
        commands_executed: 1,
    }
}

fn initialize_test_app(app: &mut App) {
    app.add_singleton_model(|_| Appearance::mock());
}

fn build_view(_ctx: &mut warpui::ViewContext<TurnUsageView>) -> TurnUsageView {
    TurnUsageView::new(placeholder_usage_info(), None)
}

#[test]
fn sections_start_expanded_and_toggle_independently() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, build_view);

        view.read(&app, |view, _| {
            assert!(view.model_usage_expanded);
            assert!(view.tool_call_summary_expanded);
            assert!(view.response_time_expanded);
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(&TurnUsageViewAction::ToggleModelUsageExpanded, ctx);
        });
        view.read(&app, |view, _| {
            assert!(!view.model_usage_expanded, "model usage should collapse");
            assert!(
                view.tool_call_summary_expanded,
                "toggling one section must not affect others"
            );
            assert!(view.response_time_expanded);
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(&TurnUsageViewAction::ToggleToolCallSummaryExpanded, ctx);
        });
        view.read(&app, |view, _| {
            assert!(!view.tool_call_summary_expanded);
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(&TurnUsageViewAction::ToggleResponseTimeExpanded, ctx);
        });
        view.read(&app, |view, _| {
            assert!(!view.response_time_expanded);
        });

        // Toggling back should re-expand.
        view.update(&mut app, |view, ctx| {
            view.handle_action(&TurnUsageViewAction::ToggleModelUsageExpanded, ctx);
        });
        view.read(&app, |view, _| {
            assert!(view.model_usage_expanded);
        });
    });
}

#[test]
fn close_action_emits_close_requested_event() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, build_view);

        let received = std::rc::Rc::new(std::cell::Cell::new(false));
        let received_clone = received.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&view, move |_, event, _| {
                if matches!(event, TurnUsageViewEvent::CloseRequested) {
                    received_clone.set(true);
                }
            });
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(&TurnUsageViewAction::Close, ctx);
        });

        assert!(
            received.get(),
            "Close action should emit TurnUsageViewEvent::CloseRequested"
        );
    });
}
