use chrono::Utc;
use persistence::model::ConversationUsageMetadata;
use warpui::{App, AppContext, EntityId, SingletonEntity, View};

use super::AgentInputFooter;
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{AIAgentHarness, ServerAIConversationMetadata};
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::ambient_agents::task::TaskPrincipalInfo;
use crate::ai::ambient_agents::{AmbientAgentTask, AmbientAgentTaskId, AmbientAgentTaskState};
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::auth::user::TEST_USER_UID;
use crate::cloud_object::{Owner, Revision, ServerMetadata, ServerPermissions};
use crate::features::FeatureFlag;
use crate::server::ids::ServerId;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
    CLIAgentSessionsModel,
};
use crate::terminal::shared_session::{SharedSessionSource, SharedSessionStatus};
use crate::terminal::{CLIAgent, TerminalView};
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

const CONVERSATION_TOKEN: &str = "server-conversation-token";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloudRoutingIndicatorKind {
    LiveSession,
    NewCloudVm,
}

fn ambient_task_id() -> AmbientAgentTaskId {
    "550e8400-e29b-41d4-a716-000000000001".parse().unwrap()
}

fn claude_cli_session() -> CLIAgentSession {
    CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        remote_host: None,
        draft_text: None,
        custom_command_prefix: None,
        received_rich_notification: false,
    }
}

fn register_claude_cli_session(view: &TerminalView, ctx: &mut warpui::ViewContext<TerminalView>) {
    let view_id = view.id();
    CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
        sessions.set_session(view_id, claude_cli_session(), ctx);
    });
}

fn footer_indicator_kind(
    footer: &AgentInputFooter,
    app: &AppContext,
) -> Option<CloudRoutingIndicatorKind> {
    let terminal_model = footer.terminal_model.lock();
    let button = footer.cloud_routing_indicator_button(&terminal_model, app)?;
    if std::ptr::eq(button, &footer.live_session_indicator) {
        Some(CloudRoutingIndicatorKind::LiveSession)
    } else if std::ptr::eq(button, &footer.new_cloud_vm_indicator) {
        Some(CloudRoutingIndicatorKind::NewCloudVm)
    } else {
        panic!("cloud routing indicator returned an unexpected button");
    }
}

fn assert_cli_footer_indicator(
    view: &TerminalView,
    app: &AppContext,
    expected: Option<CloudRoutingIndicatorKind>,
) {
    let footer = view.input().as_ref(app).agent_input_footer().clone();
    footer.read(app, |footer, app| {
        assert!(
            footer.is_cli_agent_session_active(app),
            "precondition: CLI rich-input footer path should be active"
        );
        assert_eq!(footer_indicator_kind(footer, app), expected);
        let _ = footer.render(app);
    });
}

fn ambient_agent_task(task_id: AmbientAgentTaskId) -> AmbientAgentTask {
    let now = Utc::now();
    AmbientAgentTask {
        task_id,
        parent_run_id: None,
        title: "Task".to_string(),
        state: AmbientAgentTaskState::Succeeded,
        prompt: "test".to_string(),
        created_at: now,
        started_at: Some(now),
        updated_at: now,
        run_time: Some("PT1S".parse().unwrap()),
        status_message: None,
        source: None,
        execution_location: None,
        session_id: None,
        session_link: None,
        creator: Some(TaskPrincipalInfo {
            creator_type: "USER".to_string(),
            uid: TEST_USER_UID.to_string(),
            display_name: None,
        }),
        executor: None,
        conversation_id: Some(CONVERSATION_TOKEN.to_string()),
        request_usage: None,
        is_sandbox_running: false,
        agent_config_snapshot: None,
        artifacts: vec![],
        last_event_sequence: None,
        children: vec![],
    }
}

fn server_conversation_metadata(
    harness: AIAgentHarness,
    task_id: AmbientAgentTaskId,
) -> ServerAIConversationMetadata {
    ServerAIConversationMetadata {
        title: "Conversation".to_string(),
        working_directory: None,
        harness,
        usage: ConversationUsageMetadata {
            was_summarized: false,
            context_window_usage: 0.0,
            credits_spent: 0.0,
            platform_credits_spent: 0.0,
            total_provider_cost_in_cents: None,
            credits_spent_for_last_block: None,
            charged_usage_for_last_block: None,
            total_charged_usage: None,
            token_usage: vec![],
            tool_usage_metadata: Default::default(),
            context_window_segments: Vec::new(),
        },
        metadata: ServerMetadata {
            uid: ServerId::default(),
            revision: Revision::now(),
            metadata_last_updated_ts: Utc::now().into(),
            trashed_ts: None,
            folder_id: None,
            is_welcome_object: false,
            creator_uid: Some(TEST_USER_UID.to_string()),
            last_editor_uid: None,
            current_editor_uid: None,
        },
        creator: None,
        permissions: ServerPermissions {
            space: Owner::mock_current_user(),
            guests: vec![],
            anyone_link_sharing: None,
            permissions_last_updated_ts: Utc::now().into(),
        },
        ambient_agent_task_id: Some(task_id),
        server_conversation_token: ServerConversationToken::new(CONVERSATION_TOKEN.to_string()),
        artifacts: vec![],
    }
}

fn seed_owned_disconnected_cloud_task(
    app: &mut App,
    terminal_view_id: EntityId,
    task_id: AmbientAgentTaskId,
    harness: AIAgentHarness,
) {
    let _agent_management_guard = FeatureFlag::AgentManagementView.override_enabled(false);
    AgentConversationsModel::handle(app).update(app, |model, _| {
        model.insert_task_for_test(ambient_agent_task(task_id));
    });
    BlocklistAIHistoryModel::handle(app).update(app, |model, ctx| {
        let conversation_id =
            model.start_new_conversation(terminal_view_id, false, false, false, ctx);
        model.set_server_conversation_token_for_conversation(
            conversation_id,
            CONVERSATION_TOKEN.to_string(),
        );
        model.set_server_metadata_for_conversation(
            conversation_id,
            server_conversation_metadata(harness, task_id),
            ctx,
        );
    });
}

#[test]
fn cli_footer_shows_live_indicator_for_cloud_third_party_session() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let task_id = ambient_task_id();

        terminal.update(&mut app, |view, ctx| {
            register_claude_cli_session(view, ctx);
            {
                let mut model = view.model.lock();
                model.set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                    task_id.to_string(),
                )));
                model.set_shared_session_status(SharedSessionStatus::executor());
            }
        });

        terminal.read(&app, |view, app| {
            assert_cli_footer_indicator(view, app, Some(CloudRoutingIndicatorKind::LiveSession));
        });
    });
}

#[test]
fn cli_footer_shows_new_cloud_vm_indicator_for_disconnected_third_party_session() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let task_id = ambient_task_id();
        let terminal_view_id = terminal.id();
        seed_owned_disconnected_cloud_task(
            &mut app,
            terminal_view_id,
            task_id,
            AIAgentHarness::ClaudeCode,
        );

        terminal.update(&mut app, |view, ctx| {
            register_claude_cli_session(view, ctx);
            {
                let mut model = view.model.lock();
                model.set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                    task_id.to_string(),
                )));
                model.set_shared_session_status(SharedSessionStatus::NotShared);
            }
        });

        terminal.read(&app, |view, app| {
            assert_cli_footer_indicator(view, app, Some(CloudRoutingIndicatorKind::NewCloudVm));
        });
    });
}

#[test]
fn cli_footer_omits_cloud_indicators_for_local_cli_session() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            register_claude_cli_session(view, ctx);
        });

        terminal.read(&app, |view, app| {
            assert_cli_footer_indicator(view, app, None);
        });
    });
}

#[test]
fn cli_footer_omits_live_indicator_for_shared_local_session_viewer() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            register_claude_cli_session(view, ctx);
            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::executor());
        });

        terminal.read(&app, |view, app| {
            assert_cli_footer_indicator(view, app, None);
        });
    });
}
