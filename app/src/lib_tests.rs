use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::anyhow;
use warp_core::channel::IapConfig;
use warp_server_client::iap::PathResolver;

use super::*;
use crate::server::server_api::auth::{MockAuthClient, UserAuthenticationError};

#[test]
fn app_api_key_requires_validation() {
    let app = LaunchMode::App {
        args: Default::default(),
        api_key: Some("app-api-key".to_owned()),
    };

    assert!(matches!(
        app.auth_initialization(),
        AuthInitialization::PendingApiKey(api_key) if api_key == "app-api-key"
    ));
}

#[test]
fn tui_api_key_requires_validation() {
    let tui = LaunchMode::Tui {
        entrypoint: TuiEntryPoint::Interactive {
            mount: Box::new(|_| {}),
            api_key: Some("tui-api-key".to_owned()),
        },
    };

    assert!(matches!(
        tui.auth_initialization(),
        AuthInitialization::PendingApiKey(api_key) if api_key == "tui-api-key"
    ));
}

#[test]
fn command_line_api_key_requires_validation() {
    let command_line = LaunchMode::CommandLine {
        command: CliCommand::Whoami,
        global_options: GlobalOptions {
            api_key: Some("cli-api-key".to_owned()),
            ..Default::default()
        },
        debug: false,
        is_sandboxed: false,
        computer_use_override: None,
    };

    assert!(matches!(
        command_line.auth_initialization(),
        AuthInitialization::PendingApiKey(api_key) if api_key == "cli-api-key"
    ));
}

#[test]
fn startup_without_api_key_loads_persisted_auth() {
    let app = LaunchMode::App {
        args: Default::default(),
        api_key: None,
    };

    assert!(matches!(
        app.auth_initialization(),
        AuthInitialization::Persisted
    ));
}

#[test]
fn tui_uses_distinct_secure_storage_service_name() {
    let launch_mode = LaunchMode::Tui {
        entrypoint: TuiEntryPoint::Interactive {
            mount: Box::new(|_| {}),
            api_key: None,
        },
    };
    assert!(matches!(
        &launch_mode,
        LaunchMode::Tui {
            entrypoint: TuiEntryPoint::Interactive { .. }
        }
    ));

    assert_eq!(
        launch_mode.secure_storage_service_name("dev.warp.Warp-Dev"),
        "dev.warp.Warp-Dev.tui"
    );
}

#[test]
fn app_keeps_default_secure_storage_service_name() {
    let launch_mode = LaunchMode::App {
        args: Default::default(),
        api_key: None,
    };

    assert_eq!(
        launch_mode.secure_storage_service_name("dev.warp.Warp-Dev"),
        "dev.warp.Warp-Dev"
    );
}

#[test]
fn startup_auth_is_non_blocking_for_gui_and_tui() {
    // The GUI and TUI front-ends both skip the startup IAP wait: blocking here is
    // what deadlocks a cloud sandbox once its bootstrap JWT expires (see
    // warpdotdev/warp#15342). Every other launch mode keeps the blocking
    // behavior so this scope can't widen without a deliberate decision.
    let non_blocking_modes = [
        LaunchMode::App {
            args: Default::default(),
            api_key: None,
        },
        LaunchMode::Tui {
            entrypoint: TuiEntryPoint::Interactive {
                mount: Box::new(|_| {}),
                api_key: None,
            },
        },
    ];
    for mode in non_blocking_modes {
        assert!(
            startup_auth_is_non_blocking(&mode),
            "{} must not block startup auth on IAP",
            mode.as_str_for_tracing()
        );
    }

    let blocking_modes = [
        LaunchMode::CommandLine {
            command: CliCommand::Whoami,
            global_options: GlobalOptions::default(),
            debug: false,
            is_sandboxed: false,
            computer_use_override: None,
        },
        LaunchMode::Test {
            driver: Box::new(None),
            is_integration_test: false,
        },
        LaunchMode::RemoteServerProxy,
        LaunchMode::RemoteServerDaemon {
            identity_key: "test".to_owned(),
        },
    ];
    for mode in blocking_modes {
        assert!(
            !startup_auth_is_non_blocking(&mode),
            "{} must block startup auth on IAP",
            mode.as_str_for_tracing()
        );
    }
}

#[test]
fn retry_gate_fires_when_iap_ready_arrives_before_attempt_settles() {
    // Reproduces the reported race: IAP becomes ready while the optimistic
    // attempt is still in flight. The retry must wait for the attempt to
    // settle rather than firing immediately.
    let mut gate = StartupAuthRetryGate::default();
    assert!(!gate.on_iap_token_ready());
    assert!(gate.on_first_attempt_settled(false));
}

#[test]
fn retry_gate_fires_when_attempt_settles_before_iap_ready() {
    let mut gate = StartupAuthRetryGate::default();
    assert!(!gate.on_first_attempt_settled(false));
    assert!(gate.on_iap_token_ready());
}

#[test]
fn retry_gate_does_not_fire_when_first_attempt_already_authenticated() {
    for iap_ready_first in [true, false] {
        let mut gate = StartupAuthRetryGate::default();
        if iap_ready_first {
            assert!(!gate.on_iap_token_ready());
            assert!(!gate.on_first_attempt_settled(true));
        } else {
            assert!(!gate.on_first_attempt_settled(true));
            assert!(!gate.on_iap_token_ready());
        }
    }
}

#[test]
fn retry_gate_fires_at_most_once() {
    let mut gate = StartupAuthRetryGate::default();
    assert!(!gate.on_first_attempt_settled(false));
    assert!(gate.on_iap_token_ready());
    // A later proactive refresh can report `StateChanged` again; the retry
    // must not fire a second time.
    assert!(!gate.on_iap_token_ready());
}

#[test]
fn retry_gate_never_fires_if_iap_never_becomes_ready() {
    // Mirrors what happens once `IapManager` exhausts its retries: the gate
    // just sits idle rather than retrying or panicking.
    let mut gate = StartupAuthRetryGate::default();
    assert!(!gate.on_first_attempt_settled(false));
    assert!(!gate.retried);
}

/// Exercises the real production wiring in `authenticate_user_after_iap_access`'s
/// non-blocking branch - the actual `AuthManager` and `IapManager` subscriptions,
/// not just `StartupAuthRetryGate` in isolation. A real `AuthManager` (backed by a
/// mocked `AuthClient`) and a real `IapManager` (with its background gcloud refresh
/// left permanently pending, so it can't race with the test) fire an `AuthFailed`
/// and an IAP-ready `StateChanged` back to back. The mocked auth client resolves on
/// a background executor, so which of the two the wiring observes first isn't
/// pinned down here (that ordering is covered deterministically by the
/// `StartupAuthRetryGate` unit tests above) - but the wiring must produce exactly
/// one retry regardless of the order. `MockAuthClient::times(2)` is the regression
/// signal: it fails the test if the wiring never retries, or double-fires.
#[test]
fn non_blocking_startup_auth_retries_exactly_once_through_real_wiring() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());

        let fetch_user_calls = Arc::new(AtomicUsize::new(0));
        let fetch_user_calls_for_mock = fetch_user_calls.clone();
        let mut mock_auth_client = MockAuthClient::new();
        mock_auth_client
            .expect_fetch_user()
            .times(2)
            .returning(move |_, _| {
                fetch_user_calls_for_mock.fetch_add(1, Ordering::SeqCst);
                Err(UserAuthenticationError::Unexpected(anyhow!(
                    "blocked by IAP challenge"
                )))
            });

        let server_api = app.read(|ctx| ServerApiProvider::as_ref(ctx).get());
        app.add_singleton_model(move |ctx| {
            AuthManager::new(server_api, Arc::new(mock_auth_client), ctx)
        });

        let iap_config = IapConfig {
            audiences: "test-audience".into(),
            service_account_email: "test-sa@example.com".into(),
        };
        let iap_state = Arc::new(IapState::new(&iap_config));
        let iap_state_for_test = iap_state.clone();
        app.add_singleton_model(move |ctx| {
            // Never resolves, so `IapManager`'s own background gcloud-refresh
            // attempt (which would fail anyway, since gcloud isn't installed)
            // can never reach far enough to race with this test's manual
            // `set_valid_token_for_test` + `StateChanged` below.
            let path_resolver: PathResolver =
                Box::new(|_ctx: &mut AppContext| Box::pin(futures::future::pending()));
            IapManager::new(Some(iap_state), path_resolver, None, ctx)
        });

        app.update(|ctx| {
            authenticate_user_after_iap_access(
                StartupUserAuthentication::ApiKey("fake-key".to_owned()),
                true,
                ctx,
            );
        });

        // Fire the IAP-ready signal right away, without waiting for the
        // optimistic `fetch_user` to settle first - this is what reproduces
        // the reported race instead of just the already-settled case.
        iap_state_for_test.set_valid_token_for_test("fake-iap-token");
        app.update(|ctx| {
            IapManager::handle(ctx).update(ctx, |_, ctx| ctx.emit(IapManagerEvent::StateChanged));
        });

        // Let the optimistic `fetch_user` resolve (on whichever side of the
        // `StateChanged` emit above it actually settles); its `AuthFailed`
        // should result in the deferred retry firing exactly once either way.
        warpui::r#async::Timer::after(Duration::from_millis(200)).await;
        assert_eq!(
            fetch_user_calls.load(Ordering::SeqCst),
            2,
            "expected exactly one retry (2 total fetch_user calls)"
        );

        // A later `StateChanged` (e.g. a proactive refresh) must not retry again.
        app.update(|ctx| {
            IapManager::handle(ctx).update(ctx, |_, ctx| ctx.emit(IapManagerEvent::StateChanged));
        });
        warpui::r#async::Timer::after(Duration::from_millis(200)).await;
        assert_eq!(
            fetch_user_calls.load(Ordering::SeqCst),
            2,
            "no further retries should fire"
        );
    });
}

#[test]
fn launch_modes_select_expected_logging_frontend() {
    let tui = LaunchMode::Tui {
        entrypoint: TuiEntryPoint::Interactive {
            mount: Box::new(|_| {}),
            api_key: None,
        },
    };
    let app = LaunchMode::App {
        args: Default::default(),
        api_key: None,
    };
    let test = LaunchMode::Test {
        driver: Box::new(None),
        is_integration_test: false,
    };

    assert_eq!(tui.log_frontend(), LogFrontend::Tui);
    assert_eq!(app.log_frontend(), LogFrontend::Gui);
    assert_eq!(test.log_frontend(), LogFrontend::Gui);
    assert_eq!(
        LaunchMode::RemoteServerProxy.log_frontend(),
        LogFrontend::Cli
    );
    assert_eq!(
        LaunchMode::RemoteServerDaemon {
            identity_key: "test".to_owned(),
        }
        .log_frontend(),
        LogFrontend::Cli
    );
}
