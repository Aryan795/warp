use serde::Serialize;
use warpui::Element;
use warpui::elements::MouseStateHandle;
use warpui::notification::NotificationSendError;

use super::{
    InlineBannerButtonState, InlineBannerCloseButton, InlineBannerContent, InlineBannerCopyButton,
    InlineBannerStyle, InlineBannerTextButton, InlineBannerTextButtonVariant,
    render_inline_block_list_banner,
};
use crate::appearance::Appearance;
use crate::terminal::view::{InlineBannerId, TerminalAction};

/// Position-cache id for the copy button, used to click it in integration tests.
pub const NOTIFICATIONS_ERROR_COPY_BUTTON_POSITION_ID: &str =
    "notifications_error_banner:copy_button";

#[derive(Clone, Copy, Debug, Serialize)]
pub enum NotificationsErrorBannerAction {
    SetPermissions,
    Troubleshoot,
    /// Copy the banner's error text to the clipboard.
    Copy,
    Close,
}

#[derive(Default)]
pub struct NotificationsErrorBannerMouseStates {
    pub troubleshoot: MouseStateHandle,
    pub copy: MouseStateHandle,
    pub close: MouseStateHandle,
    pub set_permissions: MouseStateHandle,
}

/// State necessary to render the (singleton) notifications error banner.
pub struct NotificationsErrorBannerState {
    pub banner_id: InlineBannerId,
    pub mouse_states: NotificationsErrorBannerMouseStates,
}

pub fn render_inline_notifications_error_banner(
    title: &str,
    state: &NotificationsErrorBannerState,
    error: &Option<NotificationSendError>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let active_ui_text_color = appearance.theme().active_ui_text_color().into_solid();

    let mut buttons: Vec<InlineBannerTextButton> = vec![];

    // If permissions haven't been granted or denied, add a button to set the permissions.
    if matches!(error, Some(NotificationSendError::PermissionsNotYetGranted)) {
        buttons.push(InlineBannerTextButton {
            text: "Set permissions".to_string(),
            text_color: active_ui_text_color,
            button_state: InlineBannerButtonState {
                on_click_event: TerminalAction::NotificationsErrorBanner(
                    NotificationsErrorBannerAction::SetPermissions,
                ),
                mouse_state_handle: state.mouse_states.set_permissions.clone(),
            },
            font: Default::default(),
            position_id: None,
            variant: InlineBannerTextButtonVariant::Primary,
        });
    }

    buttons.push(InlineBannerTextButton {
        text: "Troubleshoot".to_string(),
        text_color: active_ui_text_color,
        button_state: InlineBannerButtonState {
            on_click_event: TerminalAction::NotificationsErrorBanner(
                NotificationsErrorBannerAction::Troubleshoot,
            ),
            mouse_state_handle: state.mouse_states.troubleshoot.clone(),
        },
        font: Default::default(),
        position_id: None,
        variant: InlineBannerTextButtonVariant::Secondary,
    });

    // Copy button so the user can copy the full error text instead of screenshotting it.
    let copy_button = InlineBannerCopyButton {
        button_state: InlineBannerButtonState {
            on_click_event: TerminalAction::NotificationsErrorBanner(
                NotificationsErrorBannerAction::Copy,
            ),
            mouse_state_handle: state.mouse_states.copy.clone(),
        },
        position_id: Some(NOTIFICATIONS_ERROR_COPY_BUTTON_POSITION_ID.to_string()),
    };

    let close_button = InlineBannerCloseButton(InlineBannerButtonState {
        on_click_event: TerminalAction::NotificationsErrorBanner(
            NotificationsErrorBannerAction::Close,
        ),
        mouse_state_handle: state.mouse_states.close.clone(),
    });

    render_inline_block_list_banner(
        InlineBannerStyle::LowPriority,
        appearance,
        InlineBannerContent {
            title: title.into(),
            buttons,
            copy_button: Some(copy_button),
            close_button: Some(close_button),
            ..Default::default()
        },
    )
}
