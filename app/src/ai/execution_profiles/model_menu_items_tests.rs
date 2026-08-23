use std::collections::HashMap;
use std::sync::Arc;

use warpui::elements::Empty;
use warpui::platform::WindowStyle;
use warpui::{App, AppContext, Element, SingletonEntity, TypedActionView, View, ViewContext};

use super::*;
use crate::LaunchMode;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::llms::{
    LLMContextWindow, LLMModelHost, LLMPreferences, LLMProvider, LLMUsageMetadata,
    RoutingHostConfig,
};
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::cloud_object::model::persistence::CloudModel;
use crate::network::NetworkStatus;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::ServerId;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::sync_queue::SyncQueue;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings::PrivacySettings;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::team::Team;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{
    HostEnablementSetting, LlmHostSettings, LlmSettings, TeamSettings, Workspace,
};

fn bedrock_llm(id: &str) -> LLMInfo {
    let mut host_configs = HashMap::new();
    host_configs.insert(
        LLMModelHost::AwsBedrock,
        RoutingHostConfig {
            enabled: true,
            model_routing_host: LLMModelHost::AwsBedrock,
        },
    );
    LLMInfo {
        display_name: id.to_string(),
        base_model_name: id.to_string(),
        id: id.into(),
        reasoning_level: None,
        usage_metadata: LLMUsageMetadata {
            request_multiplier: 1,
            credit_multiplier: None,
        },
        description: None,
        disable_reason: None,
        vision_supported: false,
        spec: None,
        // `Unknown` has no provider logo of its own, so the fallback branch of
        // `model_leading_icon` lands on the generic agent glyph -- distinct from the AWS
        // glyph the Bedrock branch produces, so the two outcomes can't be confused.
        provider: LLMProvider::Unknown,
        host_configs,
        discount_percentage: None,
        context_window: LLMContextWindow::default(),
    }
}

/// A team whose workspace-level LLM settings enable AWS Bedrock outright (`Enforce`), so the
/// icon decision doesn't depend on the user's own `AISettings` toggle.
fn team_with_bedrock_enabled(uid: i64, bedrock_enabled: bool) -> Team {
    let mut host_configs = HashMap::new();
    if bedrock_enabled {
        host_configs.insert(
            LLMModelHost::AwsBedrock,
            LlmHostSettings {
                enabled: true,
                enablement_setting: HostEnablementSetting::Enforce,
                gcp_audience: None,
                gcp_sa_email: None,
            },
        );
    }
    Team::from_local_cache(
        uid.into(),
        format!("team-{uid}"),
        Some(TeamSettings {
            llm_settings: LlmSettings {
                enabled: true,
                host_configs,
            },
            ..Default::default()
        }),
        None,
        None,
    )
}

fn workspace_with_teams(teams: Vec<Team>) -> Workspace {
    Workspace::from_local_cache(
        "workspace_uid123456789".to_string().into(),
        "test".to_string(),
        Some(teams),
    )
}

fn register_user_workspaces_for_test(app: &mut App, workspace: Workspace) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(|_| TemplatableMCPServerManager::default());
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            vec![workspace],
            ctx,
        )
    });
}

/// Captures the leading icon `available_model_menu_items` resolves for a single non-custom
/// (server) model at the moment this view is constructed -- the exact path
/// `ProfileModelSelector::new` -> `refresh_state` -> `refresh_model_menu` takes, and the one
/// finding 1 identified as still broken: `make_item_fields` used to mint its own
/// `WeakViewHandle` via `ctx.handle()`, which cannot resolve a window until construction
/// returns, so every server-model row lost its host/key icons on the first render.
struct ModelMenuIconTestView {
    leading_icon: Option<Icon>,
}

impl Entity for ModelMenuIconTestView {
    type Event = ();
}

impl ModelMenuIconTestView {
    fn new(llm: LLMInfo, team_uid: ServerId, ctx: &mut ViewContext<Self>) -> Self {
        let window_id = ctx.window_id();
        UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_id, team_uid, ctx);
        });
        let items = available_model_menu_items(vec![&llm], |_| (), None, None, false, false, ctx);
        let leading_icon = items.into_iter().find_map(|item| match item {
            MenuItem::Item(fields) => fields.icon(),
            _ => None,
        });
        Self { leading_icon }
    }
}

impl View for ModelMenuIconTestView {
    fn ui_name() -> &'static str {
        "ModelMenuIconTestView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for ModelMenuIconTestView {
    type Action = ();
}

/// Pins finding 1's fix: a non-custom model's leading icon must reflect the *constructing*
/// view's own window team, not a `WeakViewHandle` minted from `ctx.handle()` (which resolves
/// nothing until construction returns and would leave every row on the generic fallback
/// icon). Two windows on opposing Bedrock policies, each queried from inside its own view's
/// constructor, must disagree with each other.
#[test]
fn available_model_menu_items_resolves_the_constructing_views_own_window_bedrock_policy() {
    let bedrock_team = team_with_bedrock_enabled(111, true);
    let plain_team = team_with_bedrock_enabled(222, false);
    let workspace = workspace_with_teams(vec![bedrock_team.clone(), plain_team.clone()]);
    let llm = bedrock_llm("claude-test");

    App::test((), |mut app| async move {
        register_user_workspaces_for_test(&mut app, workspace);
        app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        app.add_singleton_model(LLMPreferences::new);

        let bedrock_team_uid = bedrock_team.uid;
        let bedrock_llm_clone = llm.clone();
        let (_bedrock_window, bedrock_view) = app
            .add_window(WindowStyle::NotStealFocus, move |ctx| {
                ModelMenuIconTestView::new(bedrock_llm_clone, bedrock_team_uid, ctx)
            });

        let plain_team_uid = plain_team.uid;
        let (_plain_window, plain_view) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
            ModelMenuIconTestView::new(llm, plain_team_uid, ctx)
        });

        app.read(|ctx| {
            assert_eq!(
                bedrock_view.as_ref(ctx).leading_icon,
                Some(Icon::Aws),
                "the window under construction on the Bedrock-enabled team should badge the \
                 model with the AWS icon, resolved during construction"
            );
            assert_ne!(
                plain_view.as_ref(ctx).leading_icon,
                Some(Icon::Aws),
                "the window under construction on the team without Bedrock must not inherit \
                 the other team's host icon, even though construction can't use a \
                 WeakViewHandle"
            );
        });
    });
}
