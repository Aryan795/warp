use super::{REFRESHING_CREDENTIALS_MESSAGE, resolve_default_warping_text};
use crate::ai::blocklist::block::view_impl::common::LOAD_OUTPUT_MESSAGE;

/// When the 300 ms delay has elapsed during a request-blocking GEAP refresh,
/// the status bar must show "Refreshing Gemini Enterprise credentials...".
#[test]
fn blocked_credential_refresh_uses_refreshing_text() {
    assert_eq!(
        resolve_default_warping_text(true, None),
        REFRESHING_CREDENTIALS_MESSAGE
    );
}

/// A background refresh (credential_refresh_text_visible = false) must keep
/// the default "Warping..." text so it is never shown for proactive refreshes.
#[test]
fn non_blocking_credential_refresh_keeps_default_warping_text() {
    assert_eq!(
        resolve_default_warping_text(false, None),
        LOAD_OUTPUT_MESSAGE
    );
}

/// The credential-refresh text must take precedence over the fallback-model
/// text ("Warping with Claude…") when both could apply simultaneously.
#[test]
fn credential_refresh_text_takes_precedence_over_fallback_text() {
    assert_eq!(
        resolve_default_warping_text(true, Some("Warping with another model.")),
        REFRESHING_CREDENTIALS_MESSAGE
    );
}
