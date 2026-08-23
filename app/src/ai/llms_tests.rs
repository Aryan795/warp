use std::cell::Cell;
use std::rc::Rc;

use warpui::elements::Empty;
use warpui::platform::WindowStyle;
use warpui::{App, Element, TypedActionView, View, ViewContext, WindowId};

use super::*;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
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
use crate::terminal::input::models::query_model_picker_choices;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::team::Team;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{
    BillingMetadata, ByoApiKeyPolicy, ManagedByokByoePolicy, TeamByoSettings, TeamSettings, Tier,
    Workspace,
};
use crate::{LaunchMode, TuiEntryPoint};

// -- DisableReason::should_clear_preference tests --

#[test]
fn should_clear_preference_admin_disabled() {
    // AdminDisabled always clears, regardless of BYOK status.
    assert!(DisableReason::AdminDisabled.should_clear_preference(false));
    assert!(DisableReason::AdminDisabled.should_clear_preference(true));
}

#[test]
fn should_clear_preference_unavailable() {
    assert!(DisableReason::Unavailable.should_clear_preference(false));
    assert!(DisableReason::Unavailable.should_clear_preference(true));
}

#[test]
fn should_not_clear_preference_out_of_requests() {
    // Transient — never clears.
    assert!(!DisableReason::OutOfRequests.should_clear_preference(false));
    assert!(!DisableReason::OutOfRequests.should_clear_preference(true));
}

#[test]
fn should_not_clear_preference_provider_outage() {
    // Transient — never clears.
    assert!(!DisableReason::ProviderOutage.should_clear_preference(false));
    assert!(!DisableReason::ProviderOutage.should_clear_preference(true));
}

#[test]
fn should_clear_preference_requires_upgrade_without_byok() {
    // No BYOK key → server will reject → clear.
    assert!(DisableReason::RequiresUpgrade.should_clear_preference(false));
}

#[test]
fn should_not_clear_preference_requires_upgrade_with_byok() {
    // BYOK key present → server allows → keep.
    assert!(!DisableReason::RequiresUpgrade.should_clear_preference(true));
}

#[test]
fn llm_info_deserializes_without_base_model_name() {
    let raw = r#"{
            "display_name": "gpt-4o",
            "id": "gpt-4o",
            "usage_metadata": {
                "request_multiplier": 1,
                "credit_multiplier": null
            },
            "description": null,
            "disable_reason": null,
            "vision_supported": false,
            "spec": null,
            "provider": "Unknown"
        }"#;

    let info: LLMInfo = serde_json::from_str(raw).expect("should deserialize");
    assert_eq!(info.display_name, "gpt-4o");
    assert_eq!(info.base_model_name, "gpt-4o");
}

#[test]
fn llm_info_deserializes_host_configs_as_vec() {
    // Wire format from server: host_configs is a Vec
    let raw = r#"{
            "display_name": "gpt-4o",
            "id": "gpt-4o",
            "usage_metadata": { "request_multiplier": 1, "credit_multiplier": null },
            "provider": "OpenAI",
            "host_configs": [
                { "enabled": true, "model_routing_host": "DirectApi" },
                { "enabled": false, "model_routing_host": "AwsBedrock" }
            ]
        }"#;

    let info: LLMInfo = serde_json::from_str(raw).expect("should deserialize vec format");
    assert_eq!(info.display_name, "gpt-4o");
    assert_eq!(info.host_configs.len(), 2);
    assert!(
        info.host_configs
            .get(&LLMModelHost::DirectApi)
            .unwrap()
            .enabled
    );
    assert!(
        !info
            .host_configs
            .get(&LLMModelHost::AwsBedrock)
            .unwrap()
            .enabled
    );
}

#[test]
fn llm_info_round_trip_serializes_and_deserializes() {
    // Start with wire format (Vec)
    let wire_json = r#"{
            "display_name": "claude-3",
            "base_model_name": "claude-3",
            "id": "claude-3",
            "usage_metadata": { "request_multiplier": 2, "credit_multiplier": 1.5 },
            "description": "A powerful model",
            "vision_supported": true,
            "provider": "Anthropic",
            "host_configs": [
                { "enabled": true, "model_routing_host": "DirectApi" }
            ]
        }"#;

    // Deserialize from wire format
    let info: LLMInfo = serde_json::from_str(wire_json).expect("should deserialize");

    // Serialize (produces HashMap format)
    let serialized = serde_json::to_string(&info).expect("should serialize");

    // Deserialize again (from HashMap format)
    let round_tripped: LLMInfo =
        serde_json::from_str(&serialized).expect("should deserialize after round trip");

    assert_eq!(info, round_tripped);
}

#[test]
fn host_icon_visibility_requires_enabled_credentials_and_model_host() {
    let mut info = server_llm("gemini-test", None);
    info.host_configs.insert(
        LLMModelHost::GeminiEnterprise,
        RoutingHostConfig {
            enabled: true,
            model_routing_host: LLMModelHost::GeminiEnterprise,
        },
    );

    assert!(should_show_host_icon_for_model(
        &info,
        &LLMModelHost::GeminiEnterprise,
        true,
    ));
    assert!(!should_show_host_icon_for_model(
        &info,
        &LLMModelHost::GeminiEnterprise,
        false,
    ));
    assert!(!should_show_host_icon_for_model(
        &info,
        &LLMModelHost::AwsBedrock,
        true,
    ));

    info.host_configs
        .get_mut(&LLMModelHost::GeminiEnterprise)
        .expect("Gemini Enterprise host should exist")
        .enabled = false;
    assert!(!should_show_host_icon_for_model(
        &info,
        &LLMModelHost::GeminiEnterprise,
        true,
    ));
}

#[test]
fn auto_models_show_the_agent_glyph_instead_of_a_host_logo() {
    // The server reports host availability for auto models from host-level org
    // settings, without checking whether the auto variant's routing table can
    // actually reach that host. Badging the row with a host logo would promise a
    // destination the classifier may never pick, so auto models stay generic.
    let llm = server_llm("auto-open", None);

    for flags in [
        ModelIconFlags {
            is_auto: true,
            is_using_bedrock: true,
            ..Default::default()
        },
        ModelIconFlags {
            is_auto: true,
            is_using_gemini_enterprise: true,
            ..Default::default()
        },
        ModelIconFlags {
            is_auto: true,
            is_using_bedrock: true,
            is_using_gemini_enterprise: true,
            ..Default::default()
        },
    ] {
        assert_eq!(model_leading_icon(&llm, flags), Icon::Agent);
    }
}

#[test]
fn non_auto_models_keep_their_host_logo() {
    let llm = server_llm("claude-test", None);

    assert_eq!(
        model_leading_icon(
            &llm,
            ModelIconFlags {
                is_using_bedrock: true,
                ..Default::default()
            }
        ),
        Icon::Aws
    );
    assert_eq!(
        model_leading_icon(
            &llm,
            ModelIconFlags {
                is_using_gemini_enterprise: true,
                ..Default::default()
            }
        ),
        Icon::GeminiEnterpriseAgentPlatform
    );
    // Bedrock wins when both hosts are available, matching the server's
    // AWS_BEDROCK -> GEMINI_ENTERPRISE fallback priority.
    assert_eq!(
        model_leading_icon(
            &llm,
            ModelIconFlags {
                is_using_bedrock: true,
                is_using_gemini_enterprise: true,
                ..Default::default()
            }
        ),
        Icon::Aws
    );
}

#[test]
fn custom_routers_keep_the_dataflow_icon() {
    // `is_auto` is a name/id substring match, so a router called "Auto Router"
    // trips it. The custom-router branch is checked first so those rows keep
    // their own icon.
    let llm = server_llm("Auto Router", None);

    assert_eq!(
        model_leading_icon(
            &llm,
            ModelIconFlags {
                is_custom_router: true,
                is_auto: true,
                ..Default::default()
            }
        ),
        Icon::Dataflow
    );
}

#[test]
fn models_without_a_host_fall_back_to_the_provider_icon() {
    let mut llm = server_llm("gpt-test", None);
    llm.provider = LLMProvider::OpenAI;
    assert_eq!(
        model_leading_icon(&llm, ModelIconFlags::default()),
        Icon::OpenAILogo
    );

    // Providers with no logo of their own land on the agent glyph.
    llm.provider = LLMProvider::Unknown;
    assert_eq!(
        model_leading_icon(&llm, ModelIconFlags::default()),
        Icon::Agent
    );
}

// -- build_custom_llm_infos / display label tests --

fn endpoint(
    name: &str,
    url: &str,
    api_key: &str,
    models: Vec<CustomEndpointModel>,
) -> CustomEndpoint {
    CustomEndpoint {
        name: name.into(),
        url: url.into(),
        api_key: api_key.into(),
        schema: Default::default(),
        models,
    }
}

fn disabled_agent_llm(id: &str, display_name: &str) -> LLMInfo {
    LLMInfo {
        disable_reason: Some(DisableReason::Unavailable),
        ..agent_llm(id, display_name)
    }
}

fn model(name: &str, alias: Option<&str>, config_key: &str) -> CustomEndpointModel {
    CustomEndpointModel {
        name: name.into(),
        alias: alias.map(|s| s.into()),
        config_key: config_key.into(),
    }
}

#[test]
fn custom_llm_infos_built_from_endpoints() {
    let keys = ai::api_keys::ApiKeys {
        custom_endpoints: vec![endpoint(
            "My Endpoint",
            "https://x.io",
            "k",
            vec![
                model("gpt-4", Some("fast"), "uuid-1"),
                model("llama", None, "uuid-2"),
            ],
        )],
        ..Default::default()
    };
    let infos = build_custom_llm_infos(&keys);
    assert_eq!(infos.len(), 2);
    assert_eq!(infos[0].display_name, "fast");
    assert_eq!(infos[0].id.as_str(), "uuid-1");
    assert_eq!(
        infos[0].description.as_deref(),
        Some("Custom · My Endpoint")
    );
    assert_eq!(infos[1].display_name, "llama");
    assert_eq!(infos[1].id.as_str(), "uuid-2");
}

#[test]
fn custom_llm_display_name_uses_alias_when_present() {
    let keys = ai::api_keys::ApiKeys {
        custom_endpoints: vec![endpoint(
            "ep",
            "https://a.io",
            "k",
            vec![model("raw-name", Some("My Alias"), "uuid-a")],
        )],
        ..Default::default()
    };
    let infos = build_custom_llm_infos(&keys);
    assert_eq!(infos[0].display_name, "My Alias");
}

#[test]
fn custom_llm_display_name_falls_back_to_name_when_alias_missing() {
    let keys = ai::api_keys::ApiKeys {
        custom_endpoints: vec![endpoint(
            "ep",
            "https://a.io",
            "k",
            vec![model("raw-name", None, "uuid-a")],
        )],
        ..Default::default()
    };
    let infos = build_custom_llm_infos(&keys);
    assert_eq!(infos[0].display_name, "raw-name");
}

#[test]
fn custom_endpoint_usage_display_label_resolves_alias_name_and_generic_fallback() {
    let keys = ai::api_keys::ApiKeys {
        custom_endpoints: vec![endpoint(
            "ep",
            "https://a.io",
            "k",
            vec![
                model("raw-alias", Some("Alias"), "uuid-alias"),
                model("raw-name", None, "uuid-name"),
                model("raw~name", None, "uuid-tilde-name"),
            ],
        )],
        ..Default::default()
    };
    let preferences = LLMPreferences {
        models_by_feature: ModelsByFeature::default(),
        agent_mode_models_unavailable: false,
        last_update: None,
        base_llm_for_terminal_view: HashMap::new(),
        custom_llms: build_custom_llm_infos(&keys),
        custom_model_routers: Vec::new(),
    };

    assert_eq!(
        preferences.custom_endpoint_usage_display_label("uuid-alias"),
        "Alias"
    );
    assert_eq!(
        preferences.custom_endpoint_usage_display_label("uuid-name"),
        "raw-name"
    );
    assert_eq!(
        preferences.custom_endpoint_usage_display_label("uuid-tilde-name"),
        "raw~name"
    );
    assert_eq!(
        preferences.custom_endpoint_usage_display_label("unknown"),
        CUSTOM_ENDPOINT_USAGE_FALLBACK_LABEL
    );
}

#[test]
fn custom_llm_infos_skip_endpoints_with_empty_api_key() {
    let keys = ai::api_keys::ApiKeys {
        custom_endpoints: vec![
            endpoint("bad", "https://a.io", "", vec![model("m", None, "uuid-x")]),
            endpoint(
                "good",
                "https://b.io",
                "k",
                vec![model("m", None, "uuid-y")],
            ),
        ],
        ..Default::default()
    };
    let infos = build_custom_llm_infos(&keys);
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].id.as_str(), "uuid-y");
}

#[test]
fn custom_llm_infos_skip_models_without_config_key() {
    let keys = ai::api_keys::ApiKeys {
        custom_endpoints: vec![endpoint(
            "ep",
            "https://a.io",
            "k",
            vec![
                model("unconfigured", None, ""),
                model("ready", None, "uuid-a"),
            ],
        )],
        ..Default::default()
    };
    let infos = build_custom_llm_infos(&keys);
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].display_name, "ready");
}

#[test]
fn removing_model_row_purges_from_custom_llms() {
    let before = ai::api_keys::ApiKeys {
        custom_endpoints: vec![endpoint(
            "ep",
            "https://a.io",
            "k",
            vec![model("a", None, "uuid-a"), model("b", None, "uuid-b")],
        )],
        ..Default::default()
    };
    assert_eq!(build_custom_llm_infos(&before).len(), 2);

    let after = ai::api_keys::ApiKeys {
        custom_endpoints: vec![endpoint(
            "ep",
            "https://a.io",
            "k",
            vec![model("b", None, "uuid-b")],
        )],
        ..Default::default()
    };
    let infos = build_custom_llm_infos(&after);
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].id.as_str(), "uuid-b");
    assert!(infos.iter().all(|i| i.id.as_str() != "uuid-a"));
}

#[test]
fn removing_endpoint_purges_all_its_models_from_custom_llms() {
    let before = ai::api_keys::ApiKeys {
        custom_endpoints: vec![
            endpoint(
                "keep",
                "https://a.io",
                "k",
                vec![model("k1", None, "uuid-k1")],
            ),
            endpoint(
                "goner",
                "https://b.io",
                "k",
                vec![model("g1", None, "uuid-g1"), model("g2", None, "uuid-g2")],
            ),
        ],
        ..Default::default()
    };
    assert_eq!(build_custom_llm_infos(&before).len(), 3);

    let after = ai::api_keys::ApiKeys {
        custom_endpoints: vec![endpoint(
            "keep",
            "https://a.io",
            "k",
            vec![model("k1", None, "uuid-k1")],
        )],
        ..Default::default()
    };
    let infos = build_custom_llm_infos(&after);
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].id.as_str(), "uuid-k1");
}

// -- is_cloud_runnable_oz_model_id tests --

#[test]
fn is_cloud_runnable_oz_model_id_classifies_ids() {
    // A custom-endpoint (BYOK) model whose id is a bare `config_key` UUID —
    // this is the id that triggered the reported handoff failure.
    let keys = ai::api_keys::ApiKeys {
        custom_endpoints: vec![endpoint(
            "ep",
            "https://a.io",
            "k",
            vec![model("gpt", None, "52941f14-1b74-4afa-8f02-cdd5243b5aa9")],
        )],
        ..Default::default()
    };
    let preferences = LLMPreferences {
        models_by_feature: ModelsByFeature::default(),
        agent_mode_models_unavailable: false,
        last_update: None,
        base_llm_for_terminal_view: HashMap::new(),
        custom_llms: build_custom_llm_infos(&keys),
        custom_model_routers: Vec::new(),
    };

    // Custom-endpoint (BYOK) UUID id — not cloud-runnable.
    assert!(
        !preferences
            .is_cloud_runnable_oz_model_id(&LLMId::from("52941f14-1b74-4afa-8f02-cdd5243b5aa9"))
    );
    // Local custom router — not cloud-runnable.
    assert!(
        !preferences.is_cloud_runnable_oz_model_id(&LLMId::from("custom-router:local:my-router"))
    );
    // Cloud/team custom router — cloud-runnable: the server accepts the
    // `custom-router:cloud:` prefix at spawn and resolves it server-side.
    assert!(
        preferences.is_cloud_runnable_oz_model_id(&LLMId::from("custom-router:cloud:team-router"))
    );
    // Warp Oz slugs — cloud-runnable.
    assert!(preferences.is_cloud_runnable_oz_model_id(&LLMId::from("auto")));
    assert!(preferences.is_cloud_runnable_oz_model_id(&LLMId::from("auto-genius")));
    // A server-provided (non-custom, non-local-router) id is treated as
    // runnable; only definitively non-runnable ids are downgraded.
    assert!(preferences.is_cloud_runnable_oz_model_id(&LLMId::from("claude-4-opus")));
}

// -- Disable-aware default fallback tests --

fn server_llm(id: &str, disable_reason: Option<DisableReason>) -> LLMInfo {
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
        disable_reason,
        vision_supported: false,
        spec: None,
        provider: LLMProvider::Unknown,
        host_configs: HashMap::new(),
        discount_percentage: None,
        context_window: LLMContextWindow::default(),
    }
}

fn available(default_id: &str, choices: Vec<LLMInfo>) -> AvailableLLMs {
    AvailableLLMs {
        default_id: default_id.into(),
        choices,
        preferred_codex_model_id: None,
    }
}

#[test]
fn deserialized_available_llms_with_missing_default_does_not_panic() {
    // `AvailableLLMs::new()` guarantees `default_id` is one of `choices`, but
    // deserialization (e.g. a stale persisted cache or a server payload)
    // bypasses `new()`. Build such a struct, round-trip it through serde, and
    // confirm `default_llm_info()` falls back to the first choice instead of
    // panicking (Sentry: "Default LLM ID must be present in choices").
    let original = available(
        "missing-default",
        vec![server_llm("gpt-x", None), server_llm("gpt-y", None)],
    );
    let json = serde_json::to_string(&original).expect("should serialize");
    let deserialized: AvailableLLMs = serde_json::from_str(&json).expect("should deserialize");

    assert_eq!(deserialized.default_id.as_str(), "missing-default");
    assert_eq!(deserialized.default_llm_info().id.as_str(), "gpt-x");
}

#[test]
fn active_models_fall_back_to_usable_choice_or_custom_endpoint_when_default_disabled() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| NetworkStatus::new());
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.add_singleton_model(CloudModel::mock);
        app.add_singleton_model(TeamTesterStatus::mock);
        app.add_singleton_model(SyncQueue::mock);
        app.add_singleton_model(UpdateManager::mock);
        app.add_singleton_model(|_| TemplatableMCPServerManager::default());

        app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        let llm_preferences = app.add_singleton_model(LLMPreferences::new);

        let custom_model_id = LLMId::from("custom-config-key");
        ApiKeyManager::handle(&app).update(&mut app, |api_key_manager, ctx| {
            api_key_manager.add_custom_endpoint(
                ai::api_keys::CustomEndpointParams {
                    name: "local".to_string(),
                    url: "https://example.com/v1".to_string(),
                    api_key: "test-key".to_string(),
                    models: vec![(
                        "custom-model".to_string(),
                        None,
                        Some(custom_model_id.to_string()),
                    )],
                    schema: ai::api_keys::CustomEndpointSchema::default(),
                },
                ctx,
            );
        });

        // The base/coding default is admin-disabled but another hosted choice
        // is usable; every hosted CLI agent choice is admin-disabled.
        let models = ModelsByFeature {
            agent_mode: available(
                "auto",
                vec![
                    server_llm("auto", Some(DisableReason::AdminDisabled)),
                    server_llm("gpt-x", None),
                ],
            ),
            coding: available(
                "auto",
                vec![
                    server_llm("auto", Some(DisableReason::AdminDisabled)),
                    server_llm("gpt-x", None),
                ],
            ),
            cli_agent: Some(available(
                "cli-agent-auto",
                vec![server_llm(
                    "cli-agent-auto",
                    Some(DisableReason::AdminDisabled),
                )],
            )),
            computer_use: None,
        };
        llm_preferences.update(&mut app, |preferences, ctx| {
            preferences.update_feature_model_choices(Ok(models), ctx);
        });

        llm_preferences.read(&app, |preferences, app| {
            // Falls back to the first usable hosted choice.
            assert_eq!(
                preferences.get_active_base_model(app, None).id.as_str(),
                "gpt-x"
            );
            assert_eq!(
                preferences.get_active_coding_model(app, None).id.as_str(),
                "gpt-x"
            );
            // No usable hosted CLI choice → falls back to the custom endpoint.
            assert_eq!(
                preferences.get_active_cli_agent_model(app, None).id,
                custom_model_id
            );
        });
    });
}

/// Runs picker-query assertions with searchable, selectable, and disabled model fixtures plus
/// the app singletons consulted by model eligibility logic.
fn with_model_picker_query_test_context(f: impl FnOnce(&LLMPreferences, &AppContext) + 'static) {
    App::test((), |app| async move {
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.read(|app_ctx| {
            let agent_mode = AvailableLLMs::new(
                "auto".into(),
                vec![
                    agent_llm("auto", "auto (cost-efficient)"),
                    agent_llm("gpt-5", "GPT 5"),
                    disabled_agent_llm("disabled-gpt", "GPT Disabled"),
                ],
                None,
            )
            .expect("choices are non-empty");
            let preferences = LLMPreferences {
                models_by_feature: ModelsByFeature {
                    agent_mode,
                    ..Default::default()
                },
                agent_mode_models_unavailable: false,
                last_update: None,
                base_llm_for_terminal_view: HashMap::new(),
                custom_llms: Vec::new(),
                custom_model_routers: Vec::new(),
            };
            f(&preferences, app_ctx);
        });
    });
}

#[test]
fn active_models_use_default_when_usable() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| NetworkStatus::new());
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.add_singleton_model(CloudModel::mock);
        app.add_singleton_model(TeamTesterStatus::mock);
        app.add_singleton_model(SyncQueue::mock);
        app.add_singleton_model(UpdateManager::mock);
        app.add_singleton_model(|_| TemplatableMCPServerManager::default());

        app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        let llm_preferences = app.add_singleton_model(LLMPreferences::new);

        let models = ModelsByFeature {
            agent_mode: available(
                "auto",
                vec![server_llm("auto", None), server_llm("gpt-x", None)],
            ),
            coding: available("auto", vec![server_llm("auto", None)]),
            cli_agent: Some(available(
                "cli-agent-auto",
                vec![server_llm("cli-agent-auto", None)],
            )),
            computer_use: None,
        };
        llm_preferences.update(&mut app, |preferences, ctx| {
            preferences.update_feature_model_choices(Ok(models), ctx);
        });

        llm_preferences.read(&app, |preferences, app| {
            assert_eq!(
                preferences.get_active_base_model(app, None).id.as_str(),
                "auto"
            );
            assert_eq!(
                preferences
                    .get_active_cli_agent_model(app, None)
                    .id
                    .as_str(),
                "cli-agent-auto"
            );
        });
    });
}

#[test]
fn reconcile_preserves_custom_models_saved_on_execution_profile() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| NetworkStatus::new());
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.add_singleton_model(CloudModel::mock);
        app.add_singleton_model(TeamTesterStatus::mock);
        app.add_singleton_model(SyncQueue::mock);
        app.add_singleton_model(UpdateManager::mock);
        app.add_singleton_model(|_| TemplatableMCPServerManager::default());

        let profiles_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        let llm_preferences = app.add_singleton_model(LLMPreferences::new);

        let custom_model_id = LLMId::from("custom-model-config-key");
        ApiKeyManager::handle(&app).update(&mut app, |api_key_manager, ctx| {
            api_key_manager.add_custom_endpoint(
                ai::api_keys::CustomEndpointParams {
                    name: "local".to_string(),
                    url: "https://example.com/v1".to_string(),
                    api_key: "test-key".to_string(),
                    models: vec![(
                        "custom-model".to_string(),
                        Some("Custom Model".to_string()),
                        Some(custom_model_id.to_string()),
                    )],
                    schema: ai::api_keys::CustomEndpointSchema::default(),
                },
                ctx,
            );
        });

        let default_profile_id =
            profiles_model.read(&app, |profiles, _| profiles.default_profile_id());
        profiles_model.update(&mut app, |profiles, ctx| {
            profiles.set_base_model(&default_profile_id, Some(custom_model_id.clone()), ctx);
            profiles.set_coding_model(&default_profile_id, Some(custom_model_id.clone()), ctx);
            profiles.set_cli_agent_model(&default_profile_id, Some(custom_model_id.clone()), ctx);
        });

        llm_preferences.update(&mut app, |preferences, ctx| {
            preferences.update_feature_model_choices(Ok(ModelsByFeature::default()), ctx);
        });

        profiles_model.read(&app, |profiles, ctx| {
            let profile = profiles.default_profile(ctx);
            assert_eq!(profile.data().base_model.as_ref(), Some(&custom_model_id));
            assert_eq!(profile.data().coding_model.as_ref(), Some(&custom_model_id));
            assert_eq!(
                profile.data().cli_agent_model.as_ref(),
                Some(&custom_model_id)
            );
        });
    });
}

#[test]
fn reconcile_preserves_custom_endpoint_models_not_configured_locally() {
    // Regression test for QUALITY-866: a profile whose model was set to a custom
    // endpoint on device A should NOT be reset when device B syncs that profile
    // but does not have the corresponding custom endpoint configured.
    //
    // Before the fix, `reconcile_disabled_model_preferences` would clear any model
    // ID that couldn't be resolved locally, causing the profile to revert to Auto
    // and syncing that change back to cloud — erasing the user's setting on device A.
    //
    // The `context_window_limit` clear is a separately-guarded branch in
    // `reconcile_disabled_model_preferences` (gated on
    // `preferred_base_model_is_recognized`), so this test also sets a limit and
    // asserts it is preserved for the unrecognized custom endpoint ID.
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| NetworkStatus::new());
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.add_singleton_model(CloudModel::mock);
        app.add_singleton_model(TeamTesterStatus::mock);
        app.add_singleton_model(SyncQueue::mock);
        app.add_singleton_model(UpdateManager::mock);
        app.add_singleton_model(|_| TemplatableMCPServerManager::default());

        let profiles_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        let llm_preferences = app.add_singleton_model(LLMPreferences::new);

        // Simulate a model ID from a custom endpoint on another device.
        // This device (device B) does NOT have the endpoint configured locally.
        let remote_custom_model_id = LLMId::from("a1b2c3d4-5e6f-7890-abcd-ef1234567890");
        // Intentionally skip adding the endpoint to ApiKeyManager.

        let default_profile_id =
            profiles_model.read(&app, |profiles, _| profiles.default_profile_id());
        // Also set a context window limit so the separately-guarded
        // `context_window_limit` clear branch in `reconcile_disabled_model_preferences`
        // is exercised: it must NOT clear the limit for an unrecognized model ID.
        let preserved_context_window_limit: u32 = 200_000;
        profiles_model.update(&mut app, |profiles, ctx| {
            profiles.set_base_model(
                &default_profile_id,
                Some(remote_custom_model_id.clone()),
                ctx,
            );
            profiles.set_coding_model(
                &default_profile_id,
                Some(remote_custom_model_id.clone()),
                ctx,
            );
            profiles.set_cli_agent_model(
                &default_profile_id,
                Some(remote_custom_model_id.clone()),
                ctx,
            );
            profiles.set_context_window_limit(
                &default_profile_id,
                Some(preserved_context_window_limit),
                ctx,
            );
        });

        // Trigger a model list refresh (as happens on login, network reconnect, etc.).
        llm_preferences.update(&mut app, |preferences, ctx| {
            preferences.update_feature_model_choices(Ok(ModelsByFeature::default()), ctx);
        });

        // The model IDs should be PRESERVED even though no matching custom endpoint
        // is configured on this device.
        profiles_model.read(&app, |profiles, ctx| {
            let profile = profiles.default_profile(ctx);
            assert_eq!(
                profile.data().base_model.as_ref(),
                Some(&remote_custom_model_id),
                "base_model must be preserved for unknown custom endpoint IDs (cross-device sync)"
            );
            assert_eq!(
                profile.data().coding_model.as_ref(),
                Some(&remote_custom_model_id),
                "coding_model must be preserved for unknown custom endpoint IDs (cross-device sync)"
            );
            assert_eq!(
                profile.data().cli_agent_model.as_ref(),
                Some(&remote_custom_model_id),
                "cli_agent_model must be preserved for unknown custom endpoint IDs (cross-device sync)"
            );
            assert_eq!(
                profile.data().context_window_limit,
                Some(preserved_context_window_limit),
                "context_window_limit must be preserved for unknown custom endpoint IDs (cross-device sync)"
            );
        });
    });
}

#[test]
fn reconcile_preserves_custom_router_models_not_configured_locally() {
    // Regression test for QUALITY-1308: a profile whose model was set to a local
    // custom router on device A should NOT be reset when device B syncs that profile
    // but does not have the corresponding router configured locally.
    //
    // Before the fix, `reconcile_stale_custom_router_selection` called
    // `set_base_model(None)` / `set_coding_model(None)` for any
    // `custom-router:local:…` id absent from the loaded registry, causing the
    // preference to be cleared and synced back — wiping device A's setting.
    //
    // The fix: profile (persisted/synced) preferences are never cleared for
    // unrecognized local router ids. The display fallback already handles
    // rendering the default model when the router cannot be resolved locally.
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| NetworkStatus::new());
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.add_singleton_model(CloudModel::mock);
        app.add_singleton_model(TeamTesterStatus::mock);
        app.add_singleton_model(SyncQueue::mock);
        app.add_singleton_model(UpdateManager::mock);
        app.add_singleton_model(|_| TemplatableMCPServerManager::default());

        let profiles_model = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        let llm_preferences = app.add_singleton_model(LLMPreferences::new);

        // Simulate a local custom-router id from another device.
        // This device (device B) has NO local routers configured in its registry.
        let remote_router_id = LLMId::from("custom-router:local:my-special-router");

        let default_profile_id =
            profiles_model.read(&app, |profiles, _| profiles.default_profile_id());
        profiles_model.update(&mut app, |profiles, ctx| {
            profiles.set_base_model(&default_profile_id, Some(remote_router_id.clone()), ctx);
            profiles.set_coding_model(&default_profile_id, Some(remote_router_id.clone()), ctx);
        });

        // Call reconcile_stale_custom_router_selection directly.
        // self.custom_model_routers is empty (device B has no local routers),
        // so valid_local = {} and the old code would have cleared both fields.
        llm_preferences.update(&mut app, |preferences, ctx| {
            preferences.reconcile_stale_custom_router_selection(ctx);
        });

        // The model IDs must be PRESERVED — no profile clear should be synced back.
        profiles_model.read(&app, |profiles, ctx| {
            let profile = profiles.default_profile(ctx);
            assert_eq!(
                profile.data().base_model.as_ref(),
                Some(&remote_router_id),
                "base_model must be preserved for unknown custom-router:local:* IDs (cross-device sync)"
            );
            assert_eq!(
                profile.data().coding_model.as_ref(),
                Some(&remote_router_id),
                "coding_model must be preserved for unknown custom-router:local:* IDs (cross-device sync)"
            );
        });
    });
}

// -- execution-profile model selection tests --

fn agent_llm(id: &str, display_name: &str) -> LLMInfo {
    LLMInfo {
        display_name: display_name.to_owned(),
        base_model_name: display_name.to_owned(),
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
        provider: LLMProvider::Unknown,
        host_configs: HashMap::new(),
        discount_percentage: None,
        context_window: LLMContextWindow::default(),
    }
}

/// Preferences whose agent-mode models are a server-style list with an
/// `"auto"` default plus one concrete model.
fn preferences_for_profile_model_tests() -> LLMPreferences {
    let agent_mode = AvailableLLMs::new(
        "auto".into(),
        vec![
            agent_llm("auto", "auto (cost-efficient)"),
            agent_llm("claude-opus", "Opus"),
        ],
        None,
    )
    .expect("choices are non-empty");
    LLMPreferences {
        models_by_feature: ModelsByFeature {
            agent_mode,
            ..Default::default()
        },
        agent_mode_models_unavailable: false,
        last_update: None,
        base_llm_for_terminal_view: HashMap::new(),
        custom_llms: Vec::new(),
        custom_model_routers: Vec::new(),
    }
}

#[test]
fn shared_model_picker_query_orders_filters_and_marks_disabled_choices() {
    with_model_picker_query_test_context(|preferences, app| {
        let all = query_model_picker_choices(
            preferences,
            preferences.get_base_llm_choices_for_agent_mode(app),
            "",
            |_, _| false,
            app,
        );
        assert_eq!(
            all.first().map(|choice| choice.llm.id.as_str()),
            Some("auto")
        );
        assert_eq!(
            all.last().map(|choice| choice.llm.id.as_str()),
            Some("disabled-gpt")
        );
        assert!(!all.last().expect("disabled choice").is_selectable());

        let filtered = query_model_picker_choices(
            preferences,
            preferences.get_base_llm_choices_for_agent_mode(app),
            "gpt 5",
            |_, _| false,
            app,
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].llm.id.as_str(), "gpt-5");
        assert!(filtered[0].name_match_result.is_some());
        assert!(filtered[0].is_selectable());
    });
}

#[test]
fn updating_active_profile_base_model_persists_and_updates_resolution() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| NetworkStatus::new());
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.add_singleton_model(CloudModel::mock);
        app.add_singleton_model(TeamTesterStatus::mock);
        app.add_singleton_model(SyncQueue::mock);
        app.add_singleton_model(UpdateManager::mock);
        app.add_singleton_model(|_| TemplatableMCPServerManager::default());
        let profiles = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(
                &LaunchMode::Tui {
                    entrypoint: TuiEntryPoint::Interactive {
                        mount: Box::new(|_| {}),
                        api_key: None,
                    },
                },
                ctx,
            )
        });
        let preferences = app.add_singleton_model(|_| preferences_for_profile_model_tests());
        let surface_id = EntityId::new();
        let profile_id = profiles.read(&app, |profiles, ctx| {
            profiles.active_profile(Some(surface_id), ctx).id().clone()
        });
        profiles.update(&mut app, |profiles, ctx| {
            profiles.set_context_window_limit(&profile_id, Some(123), ctx);
        });

        let persisted = preferences.update(&mut app, |preferences, ctx| {
            preferences.update_active_profile_base_model(
                &LLMId::from("claude-opus"),
                Some(surface_id),
                ctx,
            )
        });

        assert!(persisted);
        profiles.read(&app, |profiles, ctx| {
            let profile = profiles
                .get_profile_by_id(&profile_id, ctx)
                .expect("active profile should exist");
            assert_eq!(
                profile.data().base_model.as_ref().map(LLMId::as_str),
                Some("claude-opus")
            );
            assert_eq!(profile.data().context_window_limit, None);
        });
        preferences.read(&app, |preferences, ctx| {
            assert_eq!(
                preferences
                    .get_active_base_model(ctx, Some(surface_id))
                    .id
                    .as_str(),
                "claude-opus"
            );
        });
    });
}

#[test]
fn selecting_a_custom_profile_default_clears_the_session_override() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| NetworkStatus::new());
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.add_singleton_model(CloudModel::mock);
        app.add_singleton_model(TeamTesterStatus::mock);
        app.add_singleton_model(SyncQueue::mock);
        app.add_singleton_model(UpdateManager::mock);
        app.add_singleton_model(|_| TemplatableMCPServerManager::default());
        let profiles = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        let custom_model_id = LLMId::from("custom-endpoint");
        let preferences = app.add_singleton_model(|_| {
            let mut preferences = preferences_for_profile_model_tests();
            preferences
                .custom_llms
                .push(agent_llm(custom_model_id.as_str(), "Custom Endpoint"));
            preferences
        });
        let surface_id = EntityId::new();
        let profile_id = profiles.read(&app, |profiles, ctx| {
            profiles.active_profile(Some(surface_id), ctx).id().clone()
        });
        profiles.update(&mut app, |profiles, ctx| {
            profiles.set_base_model(&profile_id, Some(custom_model_id.clone()), ctx);
        });
        preferences.update(&mut app, |preferences, ctx| {
            preferences.set_agent_mode_llm_override(surface_id, LLMId::from("claude-opus"), ctx);
            preferences.update_preferred_agent_mode_llm(&custom_model_id, surface_id, ctx);
        });

        preferences.read(&app, |preferences, _| {
            assert_eq!(
                preferences.base_llm_for_terminal_view.get(&surface_id),
                None
            );
        });
        profiles.update(&mut app, |profiles, ctx| {
            profiles.set_base_model(&profile_id, Some(LLMId::from("auto")), ctx);
        });
        preferences.read(&app, |preferences, ctx| {
            assert_eq!(
                preferences
                    .get_active_base_model(ctx, Some(surface_id))
                    .id
                    .as_str(),
                "auto"
            );
        });
    });
}

#[test]
fn explicit_child_model_pin_preserves_gui_behavior_and_only_emits_for_effective_changes() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| NetworkStatus::new());
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.add_singleton_model(CloudModel::mock);
        app.add_singleton_model(TeamTesterStatus::mock);
        app.add_singleton_model(SyncQueue::mock);
        app.add_singleton_model(UpdateManager::mock);
        app.add_singleton_model(|_| TemplatableMCPServerManager::default());
        let profiles = app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        let preferences = app.add_singleton_model(|_| preferences_for_profile_model_tests());
        let active_model_events = Rc::new(Cell::new(0));
        let captured_events = active_model_events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&preferences, move |_, event, _| {
                if matches!(event, LLMPreferencesEvent::UpdatedActiveAgentModeLLM) {
                    captured_events.set(captured_events.get() + 1);
                }
            });
        });

        let surface_id = EntityId::new();
        preferences.update(&mut app, |preferences, ctx| {
            preferences.set_agent_mode_llm_override(surface_id, LLMId::from("auto"), ctx);
        });
        assert_eq!(active_model_events.get(), 0);
        preferences.read(&app, |preferences, ctx| {
            assert_eq!(
                preferences
                    .get_active_base_model(ctx, Some(surface_id))
                    .id
                    .as_str(),
                "auto"
            );
            assert_eq!(
                preferences
                    .base_llm_for_terminal_view
                    .get(&surface_id)
                    .map(LLMId::as_str),
                Some("auto")
            );
        });

        profiles.update(&mut app, |profiles, ctx| {
            let profile_id = profiles.active_profile(Some(surface_id), ctx).id().clone();
            profiles.set_base_model(&profile_id, Some(LLMId::from("claude-opus")), ctx);
        });
        preferences.read(&app, |preferences, ctx| {
            assert_eq!(
                preferences
                    .get_active_base_model(ctx, Some(surface_id))
                    .id
                    .as_str(),
                "auto"
            );
        });

        preferences.update(&mut app, |preferences, ctx| {
            preferences.set_agent_mode_llm_override(surface_id, LLMId::from("claude-opus"), ctx);
        });
        assert_eq!(active_model_events.get(), 1);
        preferences.update(&mut app, |preferences, ctx| {
            preferences.set_agent_mode_llm_override(surface_id, LLMId::from("claude-opus"), ctx);
        });
        assert_eq!(active_model_events.get(), 1);
    });
}

// -- Half B: `byo_key_source_for_model` / icon scoping to the window's team --

/// A team whose plan manages BYOK/BYOE centrally, with `team_byo` set from `allow_member_credentials`.
fn team_with_byo_policy(uid: i64, allow_member_credentials: bool) -> Team {
    Team::from_local_cache(
        uid.into(),
        format!("team-{uid}"),
        Some(TeamSettings {
            team_byo: Some(TeamByoSettings {
                first_party_enabled: true,
                endpoints_enabled: true,
                allow_user_keys: allow_member_credentials,
                allow_user_endpoints: allow_member_credentials,
                first_party_keys: vec![],
                endpoints: vec![],
            }),
            ..Default::default()
        }),
        Some(BillingMetadata {
            tier: Tier {
                byo_api_key_policy: Some(ByoApiKeyPolicy { enabled: true }),
                managed_byok_byoe_policy: Some(ManagedByokByoePolicy { enabled: true }),
                ..Default::default()
            },
            ..Default::default()
        }),
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

/// The Half B fix: `byo_key_source_for_model` used to read `current_workspace().settings`,
/// one arbitrarily-chosen team's `team_byo` for a user on more than one team. Scoped, it must
/// follow the requesting window's own team instead -- with the same BYO key configured, one
/// window's read must show it in use while the other's must not.
#[test]
fn byo_key_source_for_model_follows_each_windows_own_team() {
    let permissive_team = team_with_byo_policy(111, true);
    let restrictive_team = team_with_byo_policy(222, false);
    let workspace = workspace_with_teams(vec![permissive_team.clone(), restrictive_team.clone()]);
    let mut llm = agent_llm("gpt-5", "GPT 5");
    llm.provider = LLMProvider::OpenAI;

    App::test((), |mut app| async move {
        register_user_workspaces_for_test(&mut app, workspace);
        app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        app.add_singleton_model(LLMPreferences::new);
        ApiKeyManager::handle(&app)
            .update(&mut app, |manager, ctx| {
                manager.persist_provider_key(LLMProvider::OpenAI, Some("sk-test".to_owned()), ctx)
            })
            .expect("no-op secure storage should accept the provider key");

        let permissive_window = WindowId::new();
        let restrictive_window = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(permissive_window, permissive_team.uid, ctx);
            user_workspaces.set_team_for_window(restrictive_window, restrictive_team.uid, ctx);
        });

        app.read(|ctx| {
            let user_workspaces = UserWorkspaces::as_ref(ctx);
            let permissive_scope =
                user_workspaces.team_context_for_window_for_test(permissive_window);
            let restrictive_scope =
                user_workspaces.team_context_for_window_for_test(restrictive_window);

            assert_eq!(
                byo_key_source_for_model(&llm, &permissive_scope, ctx),
                Some(ByoKeySource::UserProvided),
                "the permissive team's window should report the member's own key in use"
            );
            assert_eq!(
                byo_key_source_for_model(&llm, &restrictive_scope, ctx),
                None,
                "the restrictive team's window must not report the same member key as usable, \
                 even though it's the same workspace and the same configured key"
            );
        });
    });
}

/// A window with no team resolves to the same scope a teamless user gets: the workspace's own
/// `team_byo`, not "no policy restricts this" and not the union over every team.
#[test]
fn byo_key_source_for_model_for_a_window_with_no_team_reads_the_workspace() {
    let team = team_with_byo_policy(111, false);
    let mut workspace = workspace_with_teams(vec![team]);
    workspace.teams.clear();
    workspace.settings.team_byo = Some(TeamByoSettings {
        first_party_enabled: true,
        endpoints_enabled: true,
        allow_user_keys: false,
        allow_user_endpoints: false,
        first_party_keys: vec![],
        endpoints: vec![],
    });

    App::test((), |mut app| async move {
        register_user_workspaces_for_test(&mut app, workspace);
        app.add_singleton_model(LLMPreferences::new);

        let window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.register_window(window_id, None, ctx);
        });

        app.read(|ctx| {
            let user_workspaces = UserWorkspaces::as_ref(ctx);
            let scope = user_workspaces.team_context_for_window_for_test(window_id);
            assert_eq!(scope.team_uid(), None);
            assert!(
                !user_workspaces.are_member_byo_keys_allowed_for_scope(&scope),
                "a teamless window must read the workspace's own restrictive team_byo"
            );
        });
    });
}

/// Captures whether the key icon would be shown for `llm` at the moment this view is
/// constructed, using its own handle -- before that handle is registered in `view_to_window`
/// (registration happens only after the view's constructor returns; see
/// [`UserWorkspaces::team_context`]'s docs). This is the concrete case, alongside a `warp_tui`
/// session whose window exists but has not completed its own workspaces-metadata registration,
/// where a view's window is genuinely *unknown* rather than resolved-to-no-team.
struct LlmsScopeTestView {
    key_icon_shown_during_construction: bool,
}

impl Entity for LlmsScopeTestView {
    type Event = ();
}

impl LlmsScopeTestView {
    fn new(llm: LLMInfo, ctx: &mut ViewContext<Self>) -> Self {
        let handle = ctx.handle();
        Self {
            key_icon_shown_during_construction: should_show_key_icon_for_model(&llm, &handle, ctx),
        }
    }
}

impl View for LlmsScopeTestView {
    fn ui_name() -> &'static str {
        "LlmsScopeTestView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for LlmsScopeTestView {
    type Action = ();
}

/// A view cannot resolve its own team while it is still under construction, which is exactly
/// the shape of an *unknown* scope (Rule 3): distinct from a resolved window with no team,
/// this is a window [`UserWorkspaces`] cannot name at all. [`should_show_key_icon_for_model`]
/// must not fall back to the workspace's own `team_byo` for that case -- `team_byo` is
/// team-specific by construction, so for a user who does belong to a team, that fallback would
/// read one arbitrarily-chosen team's configuration, exactly the defect this migration exists
/// to remove. The workspace is configured permissively enough, and a real key is registered,
/// that the wrong (workspace-copy) answer and the correct (no policy) answer differ.
#[test]
fn should_show_key_icon_for_model_reports_no_icon_while_the_views_window_is_unknown() {
    let team = team_with_byo_policy(111, true);
    let mut workspace = workspace_with_teams(vec![team]);
    workspace.settings.team_byo = Some(TeamByoSettings {
        first_party_enabled: true,
        endpoints_enabled: true,
        allow_user_keys: true,
        allow_user_endpoints: true,
        first_party_keys: vec![],
        endpoints: vec![],
    });
    let mut llm = agent_llm("gpt-5", "GPT 5");
    llm.provider = LLMProvider::OpenAI;

    App::test((), |mut app| async move {
        register_user_workspaces_for_test(&mut app, workspace);
        app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        app.add_singleton_model(LLMPreferences::new);
        ApiKeyManager::handle(&app)
            .update(&mut app, |manager, ctx| {
                manager.persist_provider_key(LLMProvider::OpenAI, Some("sk-test".to_owned()), ctx)
            })
            .expect("no-op secure storage should accept the provider key");

        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            LlmsScopeTestView::new(llm, ctx)
        });

        app.read(|ctx| {
            assert!(
                !view.as_ref(ctx).key_icon_shown_during_construction,
                "a view that cannot yet resolve its window must not read the workspace's \
                 permissive team_byo as if it were that view's own team's policy"
            );
        });
    });
}

/// [`should_show_key_icon_for_model`]'s other unknown-window case (REV-2205): unlike
/// [`should_show_key_icon_for_model_reports_no_icon_while_the_views_window_is_unknown`], where
/// the *view* resolves no window at all, here the window exists (the view is fully
/// constructed and registered) but `UserWorkspaces` has no `window_team_uids` entry for it --
/// e.g. a `warp_tui` window created before its workspaces-metadata response registers a team.
/// The pre-fix `team_context` cannot tell that apart from a registered teamless window, so it
/// would read the workspace's permissive `team_byo` here too.
#[test]
fn should_show_key_icon_for_model_reports_no_icon_for_a_resolved_window_unknown_to_user_workspaces()
{
    let team = team_with_byo_policy(111, true);
    let mut workspace = workspace_with_teams(vec![team]);
    workspace.settings.team_byo = Some(TeamByoSettings {
        first_party_enabled: true,
        endpoints_enabled: true,
        allow_user_keys: true,
        allow_user_endpoints: true,
        first_party_keys: vec![],
        endpoints: vec![],
    });
    let mut llm = agent_llm("gpt-5", "GPT 5");
    llm.provider = LLMProvider::OpenAI;

    App::test((), |mut app| async move {
        register_user_workspaces_for_test(&mut app, workspace);
        app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        app.add_singleton_model(LLMPreferences::new);
        ApiKeyManager::handle(&app)
            .update(&mut app, |manager, ctx| {
                manager.persist_provider_key(LLMProvider::OpenAI, Some("sk-test".to_owned()), ctx)
            })
            .expect("no-op secure storage should accept the provider key");

        // The construction-time read (see the other test) reads `false` regardless, since the
        // handle can't resolve a window yet either way; what this test exercises is the read
        // made *after* construction, once the view -- and therefore its window -- is real.
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, {
            let llm = llm.clone();
            move |ctx| LlmsScopeTestView::new(llm, ctx)
        });
        let weak_view = view.downgrade();

        app.read(|ctx| {
            assert!(
                !should_show_key_icon_for_model(&llm, &weak_view, ctx),
                "a resolved window UserWorkspaces has never registered must not read the \
                 workspace's permissive team_byo as if it were a genuinely teamless window's \
                 fallback"
            );
        });
    });
}

struct LlmsViewContextScopeTestView {
    key_icon_shown_during_construction: bool,
}

impl Entity for LlmsViewContextScopeTestView {
    type Event = ();
}

impl LlmsViewContextScopeTestView {
    /// When `team_uid` is `Some`, assigns it to this constructor's own window -- using
    /// [`ViewContext::window_id`], valid from the first line of construction -- making the
    /// window *known* (registered, even if that registration were teamless). When `None`, the
    /// window is left genuinely *unknown*: no entry at all in `UserWorkspaces`. Either way,
    /// immediately reads the key icon flag through [`should_show_key_icon_for_model_for_view`]
    /// before returning. This is the construction-time path itself, not a proxy for it.
    fn new(llm: LLMInfo, team_uid: Option<ServerId>, ctx: &mut ViewContext<Self>) -> Self {
        if let Some(team_uid) = team_uid {
            let window_id = ctx.window_id();
            UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                user_workspaces.set_team_for_window(window_id, team_uid, ctx);
            });
        }
        Self {
            key_icon_shown_during_construction: should_show_key_icon_for_model_for_view(&llm, ctx),
        }
    }
}

impl View for LlmsViewContextScopeTestView {
    fn ui_name() -> &'static str {
        "LlmsViewContextScopeTestView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for LlmsViewContextScopeTestView {
    type Action = ();
}

/// [`should_show_key_icon_for_model_for_view`] exists precisely because a view's own
/// constructor cannot resolve a [`WeakViewHandle`] (see the previous test), but
/// [`ViewContext::window_id`] is valid throughout construction. Pins that it resolves the
/// *correct* window's team during construction, not merely a safe default: two windows
/// assigned to opposing teams, each queried from inside its own view's constructor, must
/// disagree with each other, matching each team's own policy.
#[test]
fn should_show_key_icon_for_model_for_view_resolves_the_constructing_views_own_window() {
    let permissive_team = team_with_byo_policy(111, true);
    let restrictive_team = team_with_byo_policy(222, false);
    let workspace = workspace_with_teams(vec![permissive_team.clone(), restrictive_team.clone()]);
    let mut llm = agent_llm("gpt-5", "GPT 5");
    llm.provider = LLMProvider::OpenAI;

    App::test((), |mut app| async move {
        register_user_workspaces_for_test(&mut app, workspace);
        app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        app.add_singleton_model(LLMPreferences::new);
        ApiKeyManager::handle(&app)
            .update(&mut app, |manager, ctx| {
                manager.persist_provider_key(LLMProvider::OpenAI, Some("sk-test".to_owned()), ctx)
            })
            .expect("no-op secure storage should accept the provider key");

        let permissive_llm = llm.clone();
        let permissive_team_uid = permissive_team.uid;
        let (_permissive_window, permissive_view) =
            app.add_window(WindowStyle::NotStealFocus, move |ctx| {
                LlmsViewContextScopeTestView::new(permissive_llm, Some(permissive_team_uid), ctx)
            });

        let restrictive_team_uid = restrictive_team.uid;
        let (_restrictive_window, restrictive_view) = app
            .add_window(WindowStyle::NotStealFocus, move |ctx| {
                LlmsViewContextScopeTestView::new(llm, Some(restrictive_team_uid), ctx)
            });

        app.read(|ctx| {
            assert!(
                permissive_view
                    .as_ref(ctx)
                    .key_icon_shown_during_construction,
                "the window under construction on the permissive team should see its own \
                 team's key in use, resolved during construction"
            );
            assert!(
                !restrictive_view
                    .as_ref(ctx)
                    .key_icon_shown_during_construction,
                "the window under construction on the restrictive team must not inherit the \
                 permissive team's key, even though construction can't use a WeakViewHandle"
            );
        });
    });
}

/// The other half of `should_show_key_icon_for_model_for_view`'s contract (REV-2205): a window
/// *known* to `UserWorkspaces` with no team (exercised above, folded into the restrictive
/// case's absence of team-specific data) is not the same state as a window
/// `UserWorkspaces` has never heard of at all. A window's own constructor knows its
/// `WindowId` (`ViewContext::window_id`), but that alone does not mean `UserWorkspaces` has a
/// `window_team_uids` entry for it -- registration is a separate step this test deliberately
/// skips. The workspace's own `team_byo` is configured permissively enough, with a real key
/// registered, that reading it (the pre-fix bug) and correctly reading "no policy" would
/// disagree.
#[test]
fn should_show_key_icon_for_model_for_view_reports_no_icon_for_a_window_unknown_to_user_workspaces()
{
    let team = team_with_byo_policy(111, true);
    let mut workspace = workspace_with_teams(vec![team]);
    workspace.settings.team_byo = Some(TeamByoSettings {
        first_party_enabled: true,
        endpoints_enabled: true,
        allow_user_keys: true,
        allow_user_endpoints: true,
        first_party_keys: vec![],
        endpoints: vec![],
    });
    let mut llm = agent_llm("gpt-5", "GPT 5");
    llm.provider = LLMProvider::OpenAI;

    App::test((), |mut app| async move {
        register_user_workspaces_for_test(&mut app, workspace);
        app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        app.add_singleton_model(LLMPreferences::new);
        ApiKeyManager::handle(&app)
            .update(&mut app, |manager, ctx| {
                manager.persist_provider_key(LLMProvider::OpenAI, Some("sk-test".to_owned()), ctx)
            })
            .expect("no-op secure storage should accept the provider key");

        // No `set_team_for_window` call at all: this window is genuinely unknown to
        // `UserWorkspaces`, not merely resolved-to-no-team.
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
            LlmsViewContextScopeTestView::new(llm, None, ctx)
        });

        app.read(|ctx| {
            assert!(
                !view.as_ref(ctx).key_icon_shown_during_construction,
                "a window UserWorkspaces has never registered must not read the workspace's \
                 permissive team_byo as if it were a genuinely teamless window's fallback"
            );
        });
    });
}
