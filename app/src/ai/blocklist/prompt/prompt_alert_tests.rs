use std::sync::Arc;

use warpui::App;

use super::*;
use crate::ai::credit_availability::AICreditSource;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;

fn initialize_app(app: &mut App) {
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            vec![],
            ctx,
        )
    });
    if app
        .models_of_type::<settings::PrivatePreferences>()
        .is_empty()
    {
        app.update(crate::settings::init_and_register_user_preferences);
    }
    app.update(|ctx| {
        warpui_extras::secure_storage::register_noop("test", ctx);
        ctx.add_singleton_model(ApiKeyManager::new);
    });
    app.add_singleton_model(|_| crate::pricing::PricingInfoModel::new());
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    });
}

fn apply_server_availability(app: &mut App, availability: AICreditAvailability) {
    AIRequestUsageModel::handle(app).update(app, |model, ctx| {
        model.apply_server_availability(Ok(availability), ctx);
    });
}

fn determine_state(app: &mut App) -> PromptAlertState {
    app.read(PromptAlertView::determine_state)
}

#[test]
fn test_server_available_maps_to_no_alert() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        apply_server_availability(
            &mut app,
            AICreditAvailability::available_with_source(Some(AICreditSource::BaseLimit)),
        );
        assert_eq!(determine_state(&mut app), PromptAlertState::NoAlert);
    });
}

#[test]
fn test_server_delinquent_maps_to_delinquency_alert() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        apply_server_availability(
            &mut app,
            AICreditAvailability::unavailable(AICreditDenialReason::Delinquent),
        );
        assert_eq!(
            determine_state(&mut app),
            PromptAlertState::DelinquentDueToPaymentIssue
        );
    });
}

#[test]
fn test_server_spend_limit_reasons_map_to_spend_limit_alert() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        for reason in [
            AICreditDenialReason::EnterpriseTeamSpendLimitHit,
            AICreditDenialReason::EnterprisePerUserSpendLimitHit,
            AICreditDenialReason::EnterpriseWorkspaceSpendLimitHit,
        ] {
            apply_server_availability(&mut app, AICreditAvailability::unavailable(reason));
            assert_eq!(
                determine_state(&mut app),
                PromptAlertState::MonthlyOveragesSpendLimitReached,
                "unexpected alert state for {reason:?}",
            );
        }
    });
}

#[test]
fn test_server_out_of_credits_maps_to_request_limit_reached() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        // With no workspace overage policy in play, an out-of-credits denial
        // falls through to the generic request limit alert.
        for reason in [
            AICreditDenialReason::OutOfCredits,
            AICreditDenialReason::Unknown,
        ] {
            apply_server_availability(&mut app, AICreditAvailability::unavailable(reason));
            assert_eq!(
                determine_state(&mut app),
                PromptAlertState::RequestLimitReached,
                "unexpected alert state for {reason:?}",
            );
        }
    });
}

#[test]
fn test_legacy_fallback_used_before_first_server_response() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        // No server availability applied: the default request limit info has
        // requests remaining, so the legacy derivation reports no alert.
        assert_eq!(determine_state(&mut app), PromptAlertState::NoAlert);
    });
}
