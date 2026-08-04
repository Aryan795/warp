use warp::integration_testing::clipboard::assert_clipboard_contains_string;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::single_terminal_view;
use warp::terminal::view::inline_banner::NOTIFICATIONS_ERROR_COPY_BUTTON_POSITION_ID;
use warpui_core::notification::NotificationSendError;

use super::{Builder, new_builder};

/// Regression coverage for APP-5153. In-session error text rendered by the notifications error
/// banner used to be un-copyable (custom-drawn, non-selectable text with only hyperlink handlers),
/// forcing users to screenshot it. The banner now exposes a copy button; clicking it must write the
/// full, plain error text to the clipboard so it can be pasted into Slack or an editor.
///
/// This exercises the shared inline-banner copy affordance (`InlineBannerContent::copy_button`)
/// end to end: surface the banner, click its copy button by saved position, and assert the
/// clipboard contains the exact error text.
pub fn test_notifications_error_banner_copy_button_copies_error_text() -> Builder {
    let error = NotificationSendError::Other {
        error_message: "notification daemon unavailable".to_string(),
    };
    // The copy button writes the banner's user-visible title, which is derived from the error.
    let expected_text = error.notifications_error_banner_title().to_string();

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Surface the notifications error banner").with_action(
                move |app, window_id, _| {
                    let terminal_view = single_terminal_view(app, window_id);
                    terminal_view.update(app, |view, ctx| {
                        view.show_notification_error(error.clone(), ctx);
                    });
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions("Click the copy button and verify clipboard contents")
                .with_click_on_saved_position(NOTIFICATIONS_ERROR_COPY_BUTTON_POSITION_ID)
                .add_named_assertion(
                    "clipboard contains the full error text",
                    assert_clipboard_contains_string(expected_text),
                ),
        )
}
