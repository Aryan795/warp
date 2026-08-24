//! Regression tests for [`TurnUsageView`]'s close handler, following the
//! same `handle_action`-driven pattern as `conversation_usage_view_tests.rs`,
//! plus a layout-alignment regression test for `build_label_value_columns`.

use warp_core::ui::appearance::Appearance;
use warpui::elements::{Flex, ParentElement};
use warpui::platform::WindowStyle;
use warpui::{App, Element, SingletonEntity};

use super::*;

fn placeholder_usage_info() -> TurnUsageInfo {
    TurnUsageInfo {
        models: vec![TurnModelUsage {
            model_id: "auto (cost-efficient)".to_string(),
            tokens: 4,
            cost_in_cents: Some(60.0),
        }],
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

/// Regression test for a layout bug where each section header's
/// value-column placeholder was `Empty` (zero layout height) while its
/// label-column counterpart was a real `Text` (non-zero height), shifting
/// every subsequent row in the value column up relative to its label --
/// once per section header. Verifies row-by-row alignment by checking that
/// the label and value columns produce the same number of text rows (a
/// dropped/misaligned row changes the `Flex::debug_text_content` line
/// count, since `Empty` contributes no line at all while a real `Text`
/// contributes an empty line), and spot-checks specific rows line up.
#[test]
fn build_label_value_columns_keeps_every_row_aligned_across_sections() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);

        let usage_info = TurnUsageInfo {
            models: vec![
                TurnModelUsage {
                    model_id: "claude-sonnet".to_string(),
                    tokens: 100,
                    cost_in_cents: Some(12.0),
                },
                TurnModelUsage {
                    model_id: "gpt-5".to_string(),
                    tokens: 50,
                    cost_in_cents: Some(6.0),
                },
            ],
            context_window_usage: 0.25,
            tool_calls: 3,
            files_changed: 2,
            lines_added: 5,
            lines_removed: 1,
            commands_executed: 4,
        };
        let timing_info = TimingInfo {
            time_to_first_token_ms: 500,
            total_agent_response_time_ms: 1500,
            wall_to_wall_response_time_ms: Some(2000),
        };
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_ctx| {
            TurnUsageView::new(usage_info, Some(timing_info))
        });

        view.read(&app, |view, ctx| {
            let appearance = Appearance::as_ref(ctx);
            let (labels, values) = view.build_label_value_columns(appearance);

            assert_eq!(
                labels.len(),
                values.len(),
                "each label row must have a paired value row"
            );

            let labels_text = Flex::column()
                .with_children(labels)
                .finish()
                .debug_text_content()
                .unwrap_or_default();
            let values_text = Flex::column()
                .with_children(values)
                .finish()
                .debug_text_content()
                .unwrap_or_default();

            let label_lines: Vec<&str> = labels_text.lines().collect();
            let value_lines: Vec<&str> = values_text.lines().collect();

            assert_eq!(
                label_lines.len(),
                value_lines.len(),
                "label column and value column must render the same number of text \
                 rows -- a mismatch here (e.g. a row backed by `Empty` on one side) \
                 causes every subsequent row to shift out of alignment with its \
                 counterpart.\nlabels:\n{labels_text}\n\nvalues:\n{values_text}"
            );

            let model_usage_header_index = label_lines
                .iter()
                .position(|line| *line == "MODEL USAGE")
                .expect("MODEL USAGE header should be present");
            assert_eq!(
                value_lines[model_usage_header_index], "",
                "the MODEL USAGE header's paired value-column row should be an empty \
                 placeholder line, not a real value shifted up from the row below"
            );

            let context_window_index = label_lines
                .iter()
                .position(|line| *line == "Context window usage")
                .expect("Context window usage row should be present");
            assert_eq!(value_lines[context_window_index], "25%");

            let tool_calls_index = label_lines
                .iter()
                .position(|line| *line == "Tool calls")
                .expect("Tool calls row should be present");
            assert_eq!(value_lines[tool_calls_index], "3");
        });
    });
}
