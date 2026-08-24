use std::time::Duration;

use warp_core::channel::ChannelState;
use warpui::App;

use super::*;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::server::server_api::ServerApiProvider;

/// A logged-in `AuthStateProvider` makes the model's constructor take the "already
/// authenticated at startup" path immediately, without needing a real `AuthComplete` event.
fn initialize_logged_in_app(app: &mut App) {
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
}

async fn settle() {
    warpui::r#async::Timer::after(Duration::from_millis(100)).await;
}

#[test]
#[serial_test::serial]
fn fetches_eagerly_and_resolves_allowed() {
    let mock = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/factory/access")
            .with_status(200)
            .with_body(r#"{"allowed":true}"#)
            .create()
    };
    App::test((), |mut app| async move {
        initialize_logged_in_app(&mut app);
        let model = app.add_singleton_model(FactoryAccessModel::new);
        settle().await;

        model.read(&app, |model, _| {
            assert_eq!(model.access(), FactoryAccess::Allowed);
        });
    });
    mock.assert();
}

#[test]
#[serial_test::serial]
fn resolves_denied_for_an_allowed_false_response() {
    let mock = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/factory/access")
            .with_status(200)
            .with_body(r#"{"allowed":false}"#)
            .create()
    };
    App::test((), |mut app| async move {
        initialize_logged_in_app(&mut app);
        let model = app.add_singleton_model(FactoryAccessModel::new);
        settle().await;

        model.read(&app, |model, _| {
            assert_eq!(model.access(), FactoryAccess::Denied);
        });
    });
    mock.assert();
}

#[test]
#[serial_test::serial]
fn http_failure_resolves_unknown_and_is_not_retried() {
    let mock = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/factory/access")
            .with_status(500)
            .with_body(r#"{"error":"internal"}"#)
            .expect(1)
            .create()
    };
    App::test((), |mut app| async move {
        initialize_logged_in_app(&mut app);
        let model = app.add_singleton_model(FactoryAccessModel::new);
        settle().await;

        model.read(&app, |model, _| {
            assert_eq!(model.access(), FactoryAccess::Unknown);
        });

        // A later call (mirroring a subsequent AuthComplete, e.g. a token refresh) must not
        // issue a second request: the session holds the first outcome, success or failure.
        model.update(&mut app, |model, ctx| model.request_if_needed(ctx));
        settle().await;
    });
    mock.assert();
}

#[test]
#[serial_test::serial]
fn malformed_response_body_resolves_unknown() {
    let mock = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/factory/access")
            .with_status(200)
            .with_body("not json")
            .create()
    };
    App::test((), |mut app| async move {
        initialize_logged_in_app(&mut app);
        let model = app.add_singleton_model(FactoryAccessModel::new);
        settle().await;

        model.read(&app, |model, _| {
            assert_eq!(model.access(), FactoryAccess::Unknown);
        });
    });
    mock.assert();
}

#[test]
#[serial_test::serial]
fn reset_allows_a_fresh_request_on_the_next_authenticated_session() {
    let mock = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/factory/access")
            .with_status(200)
            .with_body(r#"{"allowed":true}"#)
            .expect(2)
            .create()
    };
    App::test((), |mut app| async move {
        initialize_logged_in_app(&mut app);
        let model = app.add_singleton_model(FactoryAccessModel::new);
        settle().await;
        model.read(&app, |model, _| {
            assert_eq!(model.access(), FactoryAccess::Allowed);
        });

        // Simulates `auth::log_out`'s explicit reset call.
        model.update(&mut app, |model, _| model.reset());
        model.read(&app, |model, _| {
            assert_eq!(model.access(), FactoryAccess::Unknown);
        });

        // The next authenticated session (a real AuthComplete after re-login) starts a new
        // request rather than reusing the stale result.
        model.update(&mut app, |model, ctx| model.request_if_needed(ctx));
        settle().await;
        model.read(&app, |model, _| {
            assert_eq!(model.access(), FactoryAccess::Allowed);
        });
    });
    mock.assert();
}
