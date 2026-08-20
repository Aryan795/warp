use std::sync::Arc;

use ai::api_keys::ApiKeyManager;
use warp_core::channel::{Channel, ChannelState};
use warpui::{App, SingletonEntity as _};

use super::{RequestParams, ServerConversationToken};
use crate::ai::agent::ServerOutputId;
use crate::ai::llms::LLMProvider;
use crate::auth::AuthStateProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::workspaces::team::Team;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{
    BillingMetadata, ManagedByokByoePolicy, TeamByoSettings, TeamSettings, Tier, Workspace,
};

#[test]
fn debugging_payload_is_link_on_dogfood_channels() {
    let token = ServerConversationToken::new("conversation-token".to_owned());
    let request_id = ServerOutputId::new("request-id".to_owned());
    let expected_link = format!(
        "{}/debug/maa/conversation-token",
        ChannelState::server_root_url()
    );

    for channel in [Channel::Dev, Channel::Local] {
        assert_eq!(
            token.debugging_payload_for_channel(None, channel),
            expected_link
        );
        assert_eq!(
            token.debugging_payload_for_channel(Some(&request_id), channel),
            format!("{expected_link}?request=request-id")
        );
    }
}

#[test]
fn debugging_payload_is_id_on_non_dogfood_channels() {
    let token = ServerConversationToken::new("conversation-token".to_owned());
    let request_id = ServerOutputId::new("request-id".to_owned());

    for channel in [
        Channel::Stable,
        Channel::Preview,
        Channel::Integration,
        Channel::Oss,
    ] {
        assert_eq!(
            token.debugging_payload_for_channel(None, channel),
            "{\"conversation_id\":\"conversation-token\"}"
        );
        assert_eq!(
            token.debugging_payload_for_channel(Some(&request_id), channel),
            "{\"request_id\":\"request-id\",\"conversation_id\":\"conversation-token\"}"
        );
    }
}

fn team_byo_settings(allow_user_keys: bool, allow_user_endpoints: bool) -> TeamByoSettings {
    TeamByoSettings {
        first_party_enabled: true,
        endpoints_enabled: true,
        allow_user_keys,
        allow_user_endpoints,
        first_party_keys: vec![],
        endpoints: vec![],
    }
}

/// Two teams on one workspace with identical workspace-level BYO entitlement (derived from
/// team A's billing metadata by `Workspace::from_local_cache`) but opposite `team_byo`
/// policies, so only the team's own policy - not plan entitlement - can explain a difference
/// in behavior between them.
fn workspace_with_two_teams_of_opposing_byo_policy() -> (Team, Team, Workspace) {
    let team_a = Team::from_local_cache(
        111.into(),
        "team-a".to_string(),
        Some(TeamSettings {
            team_byo: Some(team_byo_settings(true, true)),
            ..Default::default()
        }),
        Some(BillingMetadata {
            tier: Tier {
                managed_byok_byoe_policy: Some(ManagedByokByoePolicy { enabled: true }),
                ..Default::default()
            },
            ..Default::default()
        }),
        None,
    );
    let team_b = Team::from_local_cache(
        222.into(),
        "team-b".to_string(),
        Some(TeamSettings {
            team_byo: Some(team_byo_settings(false, false)),
            ..Default::default()
        }),
        None,
        None,
    );
    let workspace = Workspace::from_local_cache(
        "workspace_uid123456789".to_string().into(),
        "test".to_string(),
        Some(vec![team_a.clone(), team_b.clone()]),
    );
    (team_a, team_b, workspace)
}

#[test]
fn apply_team_byo_policy_gates_member_credentials_by_team_policy() {
    let (team_a, team_b, workspace) = workspace_with_two_teams_of_opposing_byo_policy();

    App::test((), |mut app| async move {
        app.update(|ctx| {
            warpui_extras::secure_storage::register_noop("test", ctx);
            warp_core::telemetry::testing::MockTelemetryContextProvider::register(ctx);
        });
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(|ctx| {
            UserWorkspaces::mock(
                Arc::new(MockTeamClient::new()),
                Arc::new(MockWorkspaceClient::new()),
                vec![workspace],
                ctx,
            )
        });
        let api_key_manager = app.add_singleton_model(ApiKeyManager::new);
        api_key_manager
            .update(&mut app, |manager, ctx| {
                manager.persist_provider_key(
                    LLMProvider::Anthropic,
                    Some("sk-ant-test".to_owned()),
                    ctx,
                )
            })
            .expect("no-op secure storage should accept the provider key");

        let mut request_params = RequestParams::new_for_test();
        request_params.api_keys =
            app.read(|ctx| ApiKeyManager::as_ref(ctx).api_keys_for_request(true, false, None));
        assert!(
            request_params.api_keys.is_some(),
            "test setup should attach a BYO key for the policy check to gate"
        );

        app.read(|ctx| {
            let mut allowed = request_params.clone();
            allowed.apply_team_byo_policy(Some(team_a.uid), ctx);
            assert!(
                allowed.api_keys.is_some(),
                "team A's policy allows members to use their own keys"
            );

            let mut disallowed = request_params.clone();
            disallowed.apply_team_byo_policy(Some(team_b.uid), ctx);
            assert!(
                disallowed.api_keys.is_none(),
                "team B's policy disallows members from using their own keys"
            );
        });
    });
}
