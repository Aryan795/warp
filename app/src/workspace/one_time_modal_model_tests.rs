use std::sync::Arc;

use futures::FutureExt;
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warpui::{App, SingletonEntity};

use super::{
    AISettings, AuthManager, AuthManagerEvent, AuthStateProvider, FEATURE_INTROS, FeatureIntroId,
    FreeAiRemovalModalDecision, OneTimeModalModel, ServerApiProvider,
    free_ai_removal_modal_decision,
};
use crate::server::experiments::ServerExperiments;
use crate::server::server_api::auth::MockAuthClient;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::CustomerType;

#[test]
fn wait_until_auto_handoff_sleep_modal_closed_tracks_modal_state() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                // Resolves immediately while the modal is closed.
                assert!(
                    model
                        .wait_until_auto_handoff_sleep_modal_closed()
                        .now_or_never()
                        .is_some()
                );

                // The auto-resume path creates its wait future before the
                // modal opens (e.g. while offline during sleep); it must
                // still observe the modal that opens later.
                let pending_probe = model.wait_until_auto_handoff_sleep_modal_closed();
                let resolving_waiter = model.wait_until_auto_handoff_sleep_modal_closed();

                model.set_auto_handoff_sleep_modal_open(true, ctx);

                // Pending while the modal is open, because the future reads
                // live modal state at poll time.
                assert!(pending_probe.now_or_never().is_none());

                model.mark_auto_handoff_sleep_modal_dismissed(ctx);

                // An existing waiter resolves once the modal closes.
                assert!(resolving_waiter.now_or_never().is_some());
            });
        });
    });
}

#[test]
fn test_free_ai_removal_modal_decision_matrix() {
    struct Case {
        name: &'static str,
        customer_type: Option<CustomerType>,
        is_warp_ai_enabled: bool,
        has_byok_or_byoe: bool,
        completed_new_onboarding: bool,
        has_zero_base_credits: bool,
        workspaces_fetched: bool,
        expected: FreeAiRemovalModalDecision,
    }

    let cases = [
        Case {
            name: "free user with AI enabled and no base credits sees the modal",
            customer_type: Some(CustomerType::Free),
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: false,
            expected: FreeAiRemovalModalDecision::Show,
        },
        Case {
            name: "free user who still receives base credits defers (ICP)",
            customer_type: Some(CustomerType::Free),
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: false,
            workspaces_fetched: false,
            expected: FreeAiRemovalModalDecision::Defer,
        },
        Case {
            name: "free user with AI disabled is marked seen silently",
            customer_type: Some(CustomerType::Free),
            is_warp_ai_enabled: false,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: false,
            expected: FreeAiRemovalModalDecision::MarkSeenSilently,
        },
        Case {
            name: "free user with a BYO key or endpoint is marked seen silently",
            customer_type: Some(CustomerType::Free),
            is_warp_ai_enabled: true,
            has_byok_or_byoe: true,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: true,
            expected: FreeAiRemovalModalDecision::MarkSeenSilently,
        },
        Case {
            name: "free user who completed the new onboarding is marked seen silently",
            customer_type: Some(CustomerType::Free),
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: true,
            has_zero_base_credits: true,
            workspaces_fetched: true,
            expected: FreeAiRemovalModalDecision::MarkSeenSilently,
        },
        Case {
            name: "paid (Build) user is marked seen silently",
            customer_type: Some(CustomerType::Build),
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: false,
            expected: FreeAiRemovalModalDecision::MarkSeenSilently,
        },
        Case {
            name: "paid (BuildMax) user is marked seen silently",
            customer_type: Some(CustomerType::BuildMax),
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: true,
            expected: FreeAiRemovalModalDecision::MarkSeenSilently,
        },
        Case {
            name: "enterprise user is marked seen silently",
            customer_type: Some(CustomerType::Enterprise),
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: true,
            expected: FreeAiRemovalModalDecision::MarkSeenSilently,
        },
        Case {
            name: "legacy paid (Prosumer) user is marked seen silently",
            customer_type: Some(CustomerType::Prosumer),
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: true,
            expected: FreeAiRemovalModalDecision::MarkSeenSilently,
        },
        Case {
            name: "unknown customer type defers until billing data resolves",
            customer_type: Some(CustomerType::Unknown),
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: true,
            expected: FreeAiRemovalModalDecision::Defer,
        },
        Case {
            name: "missing workspace defers before the first server fetch",
            customer_type: None,
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: false,
            expected: FreeAiRemovalModalDecision::Defer,
        },
        Case {
            name: "missing workspace after a server fetch with no base credits is a solo free user",
            customer_type: None,
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: true,
            expected: FreeAiRemovalModalDecision::Show,
        },
        Case {
            name: "solo user who still receives base credits defers (ICP)",
            customer_type: None,
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: false,
            workspaces_fetched: true,
            expected: FreeAiRemovalModalDecision::Defer,
        },
        Case {
            name: "missing workspace with AI disabled is marked seen silently",
            customer_type: None,
            is_warp_ai_enabled: false,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: true,
            expected: FreeAiRemovalModalDecision::MarkSeenSilently,
        },
    ];

    for case in cases {
        assert_eq!(
            free_ai_removal_modal_decision(
                case.customer_type,
                case.is_warp_ai_enabled,
                case.has_byok_or_byoe,
                case.completed_new_onboarding,
                case.has_zero_base_credits,
                case.workspaces_fetched,
            ),
            case.expected,
            "case failed: {}",
            case.name,
        );
    }
}

#[test]
fn feature_intro_triggers_for_unseen_feature() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let key = FeatureIntroId::CustomModelRouter.as_key();
            let window_id = ctx.window_id();
            let active_window = ctx.windows().active_window();

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert!(!AISettings::as_ref(ctx).is_feature_intro_seen(key));
                // Simulate the startup race where the modal queue runs before
                // on_active_window_changed has assigned a target window.
                model.target_window_id = None;

                let shown = model.check_and_trigger_feature_intro_modal(ctx);

                // The feature is marked seen up front, whether or not it is shown on
                // the current channel.
                assert!(AISettings::as_ref(ctx).is_feature_intro_seen(key));
                if shown {
                    assert_eq!(
                        model.active_feature_intro,
                        Some(FeatureIntroId::CustomModelRouter)
                    );
                    // Prefer binding to the focused window immediately. If the
                    // window manager has not yet reported an active window, the
                    // intro stays pending until `update_target_window_id`.
                    if active_window.is_some() {
                        assert_eq!(model.target_window_id, Some(window_id));
                        assert_eq!(
                            model.active_feature_intro(),
                            Some(FeatureIntroId::CustomModelRouter)
                        );
                    } else {
                        assert_eq!(model.target_window_id, None);
                        assert_eq!(model.active_feature_intro(), None);
                    }
                }

                // It is shown at most once: a second check is a no-op.
                assert!(!model.check_and_trigger_feature_intro_modal(ctx));
            });
        });
    });
}

#[test]
fn feature_intro_becomes_visible_when_target_window_is_assigned() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let window_id = ctx.window_id();

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                // Intro selected before any window is active (no active window
                // available to bind yet).
                model.target_window_id = None;
                model.active_feature_intro = Some(FeatureIntroId::CustomModelRouter);
                assert_eq!(model.active_feature_intro(), None);

                model.update_target_window_id(window_id, ctx);

                assert_eq!(model.target_window_id, Some(window_id));
                assert_eq!(
                    model.active_feature_intro(),
                    Some(FeatureIntroId::CustomModelRouter)
                );
            });
        });
    });
}

#[test]
fn agent_cli_launch_modal_shows_at_most_once() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let _flag = FeatureFlag::AgentCliLaunchModal.override_enabled(true);

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert!(!*AISettings::as_ref(ctx).did_check_to_trigger_agent_cli_launch_modal);

                let shown = model.check_and_trigger_agent_cli_launch_modal(ctx);

                // The seen marker is written up front, whether or not the modal
                // is shown on the current channel.
                assert!(*AISettings::as_ref(ctx).did_check_to_trigger_agent_cli_launch_modal);
                assert_eq!(model.is_agent_cli_launch_modal_open, shown);

                // A second check is a no-op, so the modal is never shown twice.
                assert!(!model.check_and_trigger_agent_cli_launch_modal(ctx));

                model.mark_agent_cli_launch_modal_dismissed(ctx);
                assert!(!model.is_agent_cli_launch_modal_open);
                assert!(!model.check_and_trigger_agent_cli_launch_modal(ctx));
            });
        });
    });
}

#[test]
fn agent_cli_launch_modal_pre_dismissed_for_new_users_on_auth_complete() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            // Building the model installs the AuthComplete subscription under test.
            let _model = OneTimeModalModel::handle(ctx);

            // A user who hasn't completed onboarding is a fresh signup.
            AuthStateProvider::as_ref(ctx).get().set_is_onboarded(false);
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(false)
            );
            assert!(!*AISettings::as_ref(ctx).did_check_to_trigger_agent_cli_launch_modal);

            AuthManager::handle(ctx).update(ctx, |_, ctx| {
                ctx.emit(AuthManagerEvent::AuthComplete);
            });
        });

        // Without this pre-dismissal a new signup would be shown the modal on
        // their second startup, right after onboarding.
        app.read(|ctx| {
            assert!(*AISettings::as_ref(ctx).did_check_to_trigger_agent_cli_launch_modal);
        });
    });
}

#[test]
fn agent_cli_launch_modal_skipped_when_flag_disabled() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let _flag = FeatureFlag::AgentCliLaunchModal.override_enabled(false);

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert!(!model.check_and_trigger_agent_cli_launch_modal(ctx));
                // The seen marker stays untouched so the modal can still be
                // shown once the flag is turned on.
                assert!(!*AISettings::as_ref(ctx).did_check_to_trigger_agent_cli_launch_modal);
            });
        });
    });
}

#[test]
fn feature_intro_ineligible_entry_is_skipped_without_being_marked_seen() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let _flag = FeatureFlag::FactoriesLaunchModal.override_enabled(false);

            // Mark every intro ahead of FactoriesLaunch as seen so it's next in line.
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                for intro in FEATURE_INTROS {
                    if intro.id == FeatureIntroId::FactoriesLaunch {
                        break;
                    }
                    settings.mark_feature_intro_seen(intro.id.as_key(), ctx);
                }
            });

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert!(!model.check_and_trigger_feature_intro_modal(ctx));
                assert!(
                    !AISettings::as_ref(ctx)
                        .is_feature_intro_seen(FeatureIntroId::FactoriesLaunch.as_key()),
                    "an ineligible intro must not be consumed, so it can still show once eligible"
                );
            });
        });
    });
}

#[test]
fn feature_intro_skipped_when_all_seen() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                // Mirror the new-user pre-dismissal: mark every registered intro seen.
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    for intro in FEATURE_INTROS {
                        settings.mark_feature_intro_seen(intro.id.as_key(), ctx);
                    }
                });
                for intro in FEATURE_INTROS {
                    assert!(AISettings::as_ref(ctx).is_feature_intro_seen(intro.id.as_key()));
                }

                assert!(!model.check_and_trigger_feature_intro_modal(ctx));
                assert_eq!(model.active_feature_intro, None);
            });
        });
    });
}

#[test]
fn maybe_check_and_trigger_feature_intro_modal_respects_guards() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let key = FeatureIntroId::CustomModelRouter.as_key();
            let window_id = ctx.window_id();

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                // `is_any_modal_open` also requires a target window; bind one so
                // opening a modal below is actually observed as "open".
                model.update_target_window_id(window_id, ctx);

                // Before the initial modal-check pass has completed, a recheck
                // must not act on the (possibly stale) seen markers.
                model.maybe_check_and_trigger_feature_intro_modal(ctx);
                assert!(!AISettings::as_ref(ctx).is_feature_intro_seen(key));

                model.has_completed_initial_modal_checks = true;

                // While a higher-priority modal is open, a recheck is deferred.
                model.set_oz_launch_modal_open(true, ctx);
                model.maybe_check_and_trigger_feature_intro_modal(ctx);
                assert!(!AISettings::as_ref(ctx).is_feature_intro_seen(key));

                // Once it closes and both guards are satisfied, the recheck runs.
                model.set_oz_launch_modal_open(false, ctx);
                model.maybe_check_and_trigger_feature_intro_modal(ctx);
                assert!(AISettings::as_ref(ctx).is_feature_intro_seen(key));
            });
        });
    });
}

#[test]
fn higher_priority_modal_dismissal_rechecks_feature_intro() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let key = FeatureIntroId::CustomModelRouter.as_key();
            let window_id = ctx.window_id();

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                model.update_target_window_id(window_id, ctx);
                model.has_completed_initial_modal_checks = true;
                model.set_oz_launch_modal_open(true, ctx);
                assert!(model.is_any_modal_open());
                assert!(!AISettings::as_ref(ctx).is_feature_intro_seen(key));

                // Dismissing a modal that sits ahead of feature intros in
                // `check_and_trigger_all_modals` must give them a chance to show,
                // rather than waiting for the next full app-level check.
                model.mark_oz_launch_modal_dismissed(ctx);

                assert!(!model.is_oz_launch_modal_open);
                assert!(AISettings::as_ref(ctx).is_feature_intro_seen(key));
            });
        });
    });
}

#[test]
fn factories_launch_modal_requires_validated_cta_url() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let _flag = FeatureFlag::FactoriesLaunchModal.override_enabled(true);
            let intro = FEATURE_INTROS
                .iter()
                .find(|intro| intro.id == FeatureIntroId::FactoriesLaunch)
                .unwrap();

            // No server-configured CTA URL yet: the eligibility check must fail
            // closed rather than fall back to the generic Contact Sales link.
            assert!(!UserWorkspaces::as_ref(ctx).has_validated_factories_launch_modal_cta_url());
            assert!(!(intro.eligible)(ctx));

            // A weak byte-inequality check would accept any of these; a real
            // safety gate in front of `ctx.open_url` must not.
            let malformed_or_fallback = [
                "",
                "   ",
                "not a url",
                "http://cal.com/warp",
                "javascript:alert(1)",
                "/relative/path",
                "https://www.warp.dev/contact-sales",
                "https://www.warp.dev/contact-sales/",
                "  https://www.warp.dev/contact-sales  ",
            ];
            for value in malformed_or_fallback {
                UserWorkspaces::handle(ctx).update(ctx, |workspaces, _ctx| {
                    workspaces.set_factories_launch_modal_cta_url(Some(value.to_string()));
                });
                assert!(
                    !UserWorkspaces::as_ref(ctx).has_validated_factories_launch_modal_cta_url(),
                    "expected {value:?} to be rejected"
                );
                assert!(
                    !(intro.eligible)(ctx),
                    "expected {value:?} to be ineligible"
                );
            }

            UserWorkspaces::handle(ctx).update(ctx, |workspaces, _ctx| {
                workspaces
                    .set_factories_launch_modal_cta_url(Some("https://cal.com/warp".to_string()));
            });

            assert!(UserWorkspaces::as_ref(ctx).has_validated_factories_launch_modal_cta_url());
            assert!((intro.eligible)(ctx));
        });
    });
}

#[test]
fn factories_launch_modal_eligible_with_the_real_configured_booking_url() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let _flag = FeatureFlag::FactoriesLaunchModal.override_enabled(true);
            let intro = FEATURE_INTROS
                .iter()
                .find(|intro| intro.id == FeatureIntroId::FactoriesLaunch)
                .unwrap();

            // The exact production value configured in warp-server's
            // features.factories_launch_modal_cta_url, proving the positive path
            // (not just synthetic examples or rejection cases) opens the gate.
            UserWorkspaces::handle(ctx).update(ctx, |workspaces, _ctx| {
                workspaces.set_factories_launch_modal_cta_url(Some(
                    "https://warp-dev.chilipiper.com/round-robin/factories-warp-intro".to_string(),
                ));
            });

            assert!(UserWorkspaces::as_ref(ctx).has_validated_factories_launch_modal_cta_url());
            assert!((intro.eligible)(ctx));
        });
    });
}

#[test]
fn custom_model_router_requires_ai_enabled_but_factories_launch_does_not() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let _flag = FeatureFlag::FactoriesLaunchModal.override_enabled(true);
            UserWorkspaces::handle(ctx).update(ctx, |workspaces, _ctx| {
                workspaces
                    .set_factories_launch_modal_cta_url(Some("https://cal.com/warp".to_string()));
            });
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                let _ = settings.is_any_ai_enabled.set_value(false, ctx);
            });

            let custom_model_router = FEATURE_INTROS
                .iter()
                .find(|intro| intro.id == FeatureIntroId::CustomModelRouter)
                .unwrap();
            let factories_launch = FEATURE_INTROS
                .iter()
                .find(|intro| intro.id == FeatureIntroId::FactoriesLaunch)
                .unwrap();

            // The generic AI-enabled gate now lives only on CustomModelRouter's
            // own eligibility; FactoriesLaunch's eligibility is purely server-driven.
            assert!(!(custom_model_router.eligible)(ctx));
            assert!((factories_launch.eligible)(ctx));
        });
    });
}

/// Marks every FEATURE_INTROS entry ahead of FactoriesLaunch as seen and
/// configures a validated CTA URL, so FactoriesLaunch is the next eligible,
/// unseen intro. Callers must separately hold
/// `FeatureFlag::FactoriesLaunchModal.override_enabled(true)` for the
/// duration of the test.
fn prepare_factories_launch_as_next_intro(ctx: &mut warpui::AppContext) {
    UserWorkspaces::handle(ctx).update(ctx, |workspaces, _ctx| {
        workspaces.set_factories_launch_modal_cta_url(Some("https://cal.com/warp".to_string()));
    });
    AISettings::handle(ctx).update(ctx, |settings, ctx| {
        for intro in FEATURE_INTROS {
            if intro.id == FeatureIntroId::FactoriesLaunch {
                break;
            }
            settings.mark_feature_intro_seen(intro.id.as_key(), ctx);
        }
    });
}

#[test]
fn factories_launch_feature_intro_requires_winning_the_server_claim() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let _flag = FeatureFlag::FactoriesLaunchModal.override_enabled(true);

        terminal.update(&mut app, |_, ctx| {
            prepare_factories_launch_as_next_intro(ctx);

            // Simulate another device having already claimed the impression.
            let mut auth_client = MockAuthClient::new();
            auth_client
                .expect_claim_feature_intro_impression()
                .withf(|intro_key| intro_key == FeatureIntroId::FactoriesLaunch.as_key())
                .times(1)
                .return_once(|_| Ok(false));
            ServerApiProvider::handle(ctx).update(ctx, |provider, _ctx| {
                provider.set_auth_client_for_test(Arc::new(auth_client));
            });

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                let handled = model.check_and_trigger_feature_intro_modal(ctx);

                // The seen marker must NOT be written until the claim resolves:
                // writing it eagerly (and then treating a lost/failed claim as
                // "shown") would burn the user's only impression before we even
                // know whether another device actually won it.
                assert!(
                    !AISettings::as_ref(ctx)
                        .is_feature_intro_seen(FeatureIntroId::FactoriesLaunch.as_key())
                );
                assert!(handled);
                // The popover must not open until the claim resolves.
                assert_eq!(model.active_feature_intro, None);
            });
        });

        // Let the spawned claim future resolve.
        warpui::r#async::Timer::after(std::time::Duration::from_millis(100)).await;

        terminal.update(&mut app, |_, ctx| {
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert_eq!(
                    model.active_feature_intro, None,
                    "a lost claim must not show the popover on this device"
                );
                assert!(
                    AISettings::as_ref(ctx)
                        .is_feature_intro_seen(FeatureIntroId::FactoriesLaunch.as_key()),
                    "a genuinely lost claim (another device won) is now known, so it's safe \
                     to mark seen"
                );
                assert_eq!(
                    model.pending_claim_intro_id, None,
                    "the in-flight guard must clear once the claim resolves"
                );
            });
        });
    });
}

#[test]
fn factories_launch_feature_intro_network_error_does_not_burn_the_impression() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let _flag = FeatureFlag::FactoriesLaunchModal.override_enabled(true);

        terminal.update(&mut app, |_, ctx| {
            prepare_factories_launch_as_next_intro(ctx);

            // Simulate an offline device / transient server error.
            let mut auth_client = MockAuthClient::new();
            auth_client
                .expect_claim_feature_intro_impression()
                .withf(|intro_key| intro_key == FeatureIntroId::FactoriesLaunch.as_key())
                .times(1)
                .return_once(|_| Err(anyhow::anyhow!("network unreachable")));
            ServerApiProvider::handle(ctx).update(ctx, |provider, _ctx| {
                provider.set_auth_client_for_test(Arc::new(auth_client));
            });

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert!(model.check_and_trigger_feature_intro_modal(ctx));
                assert!(
                    !AISettings::as_ref(ctx)
                        .is_feature_intro_seen(FeatureIntroId::FactoriesLaunch.as_key())
                );
            });
        });

        warpui::r#async::Timer::after(std::time::Duration::from_millis(100)).await;

        terminal.update(&mut app, |_, ctx| {
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert_eq!(
                    model.active_feature_intro, None,
                    "a failed claim must not show the popover on this device"
                );
                // The critical assertion: a request error must NOT burn the
                // user's only (globally-synced) impression. Being offline or
                // hitting a transient failure must leave the intro eligible
                // to retry, not permanently suppress it on every device.
                assert!(
                    !AISettings::as_ref(ctx)
                        .is_feature_intro_seen(FeatureIntroId::FactoriesLaunch.as_key()),
                    "a request error must leave the intro unseen so it can retry"
                );
                assert_eq!(model.pending_claim_intro_id, None);

                // A later recheck (e.g. once connectivity resumes) must retry
                // the claim rather than treating the intro as already handled.
                let mut retry_client = MockAuthClient::new();
                retry_client
                    .expect_claim_feature_intro_impression()
                    .withf(|intro_key| intro_key == FeatureIntroId::FactoriesLaunch.as_key())
                    .times(1)
                    .return_once(|_| Ok(true));
                ServerApiProvider::handle(ctx).update(ctx, |provider, _ctx| {
                    provider.set_auth_client_for_test(Arc::new(retry_client));
                });
                assert!(model.check_and_trigger_feature_intro_modal(ctx));
            });
        });

        warpui::r#async::Timer::after(std::time::Duration::from_millis(100)).await;

        terminal.update(&mut app, |_, ctx| {
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert_eq!(
                    model.active_feature_intro,
                    Some(FeatureIntroId::FactoriesLaunch),
                    "the retry should succeed once connectivity is restored"
                );
                assert!(
                    AISettings::as_ref(ctx)
                        .is_feature_intro_seen(FeatureIntroId::FactoriesLaunch.as_key())
                );
            });
        });
    });
}

#[test]
fn factories_launch_feature_intro_shows_after_winning_the_server_claim() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let _flag = FeatureFlag::FactoriesLaunchModal.override_enabled(true);
            UserWorkspaces::handle(ctx).update(ctx, |workspaces, _ctx| {
                workspaces
                    .set_factories_launch_modal_cta_url(Some("https://cal.com/warp".to_string()));
            });
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                for intro in FEATURE_INTROS {
                    if intro.id == FeatureIntroId::FactoriesLaunch {
                        break;
                    }
                    settings.mark_feature_intro_seen(intro.id.as_key(), ctx);
                }
            });

            let mut auth_client = MockAuthClient::new();
            auth_client
                .expect_claim_feature_intro_impression()
                .withf(|intro_key| intro_key == FeatureIntroId::FactoriesLaunch.as_key())
                .times(1)
                .return_once(|_| Ok(true));
            ServerApiProvider::handle(ctx).update(ctx, |provider, _ctx| {
                provider.set_auth_client_for_test(Arc::new(auth_client));
            });

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert!(model.check_and_trigger_feature_intro_modal(ctx));
                assert_eq!(model.active_feature_intro, None, "still awaiting the claim");
            });
        });

        warpui::r#async::Timer::after(std::time::Duration::from_millis(100)).await;

        terminal.update(&mut app, |_, ctx| {
            OneTimeModalModel::handle(ctx).update(ctx, |model, _ctx| {
                assert_eq!(
                    model.active_feature_intro,
                    Some(FeatureIntroId::FactoriesLaunch)
                );
            });
        });
    });
}

#[test]
fn feature_intro_recheck_on_experiments_updated() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let key = FeatureIntroId::CustomModelRouter.as_key();

        terminal.update(&mut app, |_, ctx| {
            OneTimeModalModel::handle(ctx).update(ctx, |model, _ctx| {
                model.has_completed_initial_modal_checks = true;
            });
        });
        app.read(|ctx| {
            assert!(!AISettings::as_ref(ctx).is_feature_intro_seen(key));
        });

        // A fresh experiments fetch (e.g. after the initial modal-check
        // pass already ran) must re-trigger the feature-intro check.
        terminal.update(&mut app, |_, ctx| {
            ServerExperiments::handle(ctx).update(ctx, |experiments, ctx| {
                experiments.apply_latest_state(vec![], ctx);
            });
        });

        app.read(|ctx| {
            assert!(AISettings::as_ref(ctx).is_feature_intro_seen(key));
        });
    });
}
