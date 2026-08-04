use warp_errors::ErrorExt;

use super::AIApiError;

fn error_status(status: http::StatusCode) -> AIApiError {
    AIApiError::ErrorStatus(status, String::new())
}

#[test]
fn no_content_is_neither_recoverable_nor_actionable() {
    // The SSE open rejects anything but 200, so a body-less 204 surfaces as an
    // `ErrorStatus`. The server didn't fail, so re-sending it can't help and it
    // isn't worth reporting to crash reporting.
    let error = error_status(http::StatusCode::NO_CONTENT);

    assert!(!error.is_recoverable());
    assert!(!error.is_actionable());
}

#[test]
fn unexpected_success_statuses_follow_the_same_policy_as_no_content() {
    for status in [
        http::StatusCode::OK,
        http::StatusCode::CREATED,
        http::StatusCode::ACCEPTED,
        http::StatusCode::RESET_CONTENT,
    ] {
        let error = error_status(status);

        assert!(
            !error.is_recoverable(),
            "{status} should not be recoverable"
        );
        assert!(!error.is_actionable(), "{status} should not be actionable");
    }
}

#[test]
fn server_errors_stay_recoverable_and_actionable() {
    for status in [
        http::StatusCode::INTERNAL_SERVER_ERROR,
        http::StatusCode::BAD_GATEWAY,
        http::StatusCode::SERVICE_UNAVAILABLE,
    ] {
        let error = error_status(status);

        assert!(error.is_recoverable(), "{status} should be recoverable");
        assert!(error.is_actionable(), "{status} should be actionable");
    }
}

#[test]
fn client_errors_stay_non_recoverable() {
    for status in [
        http::StatusCode::BAD_REQUEST,
        http::StatusCode::UNAUTHORIZED,
        http::StatusCode::NOT_FOUND,
    ] {
        let error = error_status(status);

        assert!(
            !error.is_recoverable(),
            "{status} should not be recoverable"
        );
        assert!(!error.is_actionable(), "{status} should not be actionable");
    }
}

#[test]
fn timeouts_and_rate_limits_stay_recoverable() {
    for status in [
        http::StatusCode::REQUEST_TIMEOUT,
        http::StatusCode::TOO_MANY_REQUESTS,
    ] {
        let error = error_status(status);

        assert!(error.is_recoverable(), "{status} should be recoverable");
        assert!(error.is_actionable(), "{status} should be actionable");
    }
}
