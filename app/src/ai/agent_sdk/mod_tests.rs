use serde_json::json;
use warp_cli::CliCommand;
use warp_cli::agent::Harness;
use warp_cli::artifact::{
    ArtifactCommand, DownloadArtifactArgs, GetArtifactArgs, UploadArtifactArgs,
};
use warp_cli::task::{MessageCommand, MessageSendArgs, MessageWatchArgs, TaskCommand};
use warp_core::telemetry::TelemetryEvent;

use super::{
    CommandAuthentication, command_authentication, command_requires_auth,
    command_to_telemetry_event, credentials_from_complete_response, reconcile_task_harness,
};
use crate::ai::agent_sdk::driver::AgentDriverError;
use crate::server::server_api::ai::{GitCredential, TaskGitCredentialsResponse};

const TASK_ID: &str = "00000000-0000-0000-0000-000000000001";

#[test]
fn logout_does_not_require_auth() {
    assert!(!command_requires_auth(&CliCommand::Logout));
}

#[test]
fn login_does_not_require_auth() {
    assert!(!command_requires_auth(&CliCommand::Login));
}

#[test]
fn pending_api_key_is_selected_for_command_authentication() {
    assert_eq!(
        command_authentication(Some("api-key".to_owned()), false),
        Some(CommandAuthentication::PendingApiKey("api-key".to_owned()))
    );
}

#[test]
fn pending_api_key_takes_precedence_over_persisted_auth() {
    assert_eq!(
        command_authentication(Some("api-key".to_owned()), true),
        Some(CommandAuthentication::PendingApiKey("api-key".to_owned()))
    );
}

#[test]
fn persisted_auth_is_refreshed_without_pending_api_key() {
    assert_eq!(
        command_authentication(None, true),
        Some(CommandAuthentication::RefreshUser)
    );
}

#[test]
fn logged_out_command_has_no_authentication_source() {
    assert_eq!(command_authentication(None, false), None);
}

#[test]
fn artifact_download_requires_auth() {
    assert!(command_requires_auth(&CliCommand::Artifact(
        ArtifactCommand::Download(DownloadArtifactArgs {
            artifact_uid: "artifact-123".to_string(),
            out: None,
        },)
    )));
}

#[test]
fn run_message_send_requires_auth() {
    assert!(command_requires_auth(&CliCommand::Run(
        TaskCommand::Message(MessageCommand::Send(MessageSendArgs {
            to: vec!["run-456".to_string()],
            subject: "subject".to_string(),
            body: "body".to_string(),
            sender_run_id: "run-123".to_string(),
        }),)
    )));
}

#[test]
fn artifact_get_requires_auth() {
    assert!(command_requires_auth(&CliCommand::Artifact(
        ArtifactCommand::Get(GetArtifactArgs {
            artifact_uid: "artifact-123".to_string(),
        },)
    )));
}

#[test]
fn artifact_upload_requires_auth() {
    assert!(command_requires_auth(&CliCommand::Artifact(
        ArtifactCommand::Upload(UploadArtifactArgs {
            path: "artifact.txt".into(),
            run_id: Some("run-123".to_string()),
            conversation_id: None,
            description: None,
        },)
    )));
}

#[test]
#[serial_test::serial]
fn run_message_send_telemetry_uses_canonical_harness_from_env() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("OZ_HARNESS", "  CLAUDE  ") };
    let event = command_to_telemetry_event(&CliCommand::Run(TaskCommand::Message(
        MessageCommand::Send(MessageSendArgs {
            to: vec!["run-456".to_string()],
            subject: "subject".to_string(),
            body: "body".to_string(),
            sender_run_id: "run-123".to_string(),
        }),
    )));
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("OZ_HARNESS") };

    assert_eq!(event.payload(), Some(json!({ "harness": "claude" })));
}

#[test]
#[serial_test::serial]
fn run_message_send_telemetry_supports_claude_code_alias() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("OZ_HARNESS", "CLAUDE_CODE") };
    let event = command_to_telemetry_event(&CliCommand::Run(TaskCommand::Message(
        MessageCommand::Send(MessageSendArgs {
            to: vec!["run-456".to_string()],
            subject: "subject".to_string(),
            body: "body".to_string(),
            sender_run_id: "run-123".to_string(),
        }),
    )));
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("OZ_HARNESS") };

    assert_eq!(event.payload(), Some(json!({ "harness": "claude" })));
}

#[test]
#[serial_test::serial]
fn run_message_send_telemetry_supports_opencode_harness() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("OZ_HARNESS", "opencode") };
    let event = command_to_telemetry_event(&CliCommand::Run(TaskCommand::Message(
        MessageCommand::Send(MessageSendArgs {
            to: vec!["run-456".to_string()],
            subject: "subject".to_string(),
            body: "body".to_string(),
            sender_run_id: "run-123".to_string(),
        }),
    )));
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("OZ_HARNESS") };

    assert_eq!(event.payload(), Some(json!({ "harness": "opencode" })));
}

#[test]
#[serial_test::serial]
fn run_message_send_telemetry_defaults_to_unknown_harness() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("OZ_HARNESS") };
    let event = command_to_telemetry_event(&CliCommand::Run(TaskCommand::Message(
        MessageCommand::Send(MessageSendArgs {
            to: vec!["run-456".to_string()],
            subject: "subject".to_string(),
            body: "body".to_string(),
            sender_run_id: "run-123".to_string(),
        }),
    )));

    assert_eq!(event.payload(), Some(json!({ "harness": "unknown" })));
}

#[test]
fn reconcile_task_harness_adopts_task_harness_when_cli_uses_default() {
    let mut selected_harness = Harness::Oz;
    let harness = reconcile_task_harness(TASK_ID, &mut selected_harness, Harness::Claude)
        .expect("default harness should adopt task harness");

    assert_eq!(selected_harness, Harness::Claude);
    assert_eq!(harness.harness(), Harness::Claude);
}

#[test]
fn reconcile_task_harness_allows_matching_explicit_harness() {
    let mut selected_harness = Harness::Claude;
    let harness = reconcile_task_harness(TASK_ID, &mut selected_harness, Harness::Claude)
        .expect("matching harness should succeed");

    assert_eq!(selected_harness, Harness::Claude);
    assert_eq!(harness.harness(), Harness::Claude);
}

#[test]
fn reconcile_task_harness_rejects_explicit_mismatch() {
    let mut selected_harness = Harness::Gemini;
    let err = reconcile_task_harness(TASK_ID, &mut selected_harness, Harness::Claude)
        .expect_err("mismatched harness should fail");

    assert_eq!(selected_harness, Harness::Gemini);
    assert!(err.to_string().contains("Task"));
    assert!(err.to_string().contains("--harness gemini"));
    assert!(err.to_string().contains("claude"));
}

#[test]
#[serial_test::serial]
fn run_message_watch_telemetry_defaults_to_unknown_harness() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("OZ_HARNESS") };
    let event = command_to_telemetry_event(&CliCommand::Run(TaskCommand::Message(
        MessageCommand::Watch(MessageWatchArgs {
            run_id: "run-123".to_string(),
            since_sequence: 0,
        }),
    )));

    assert_eq!(event.payload(), Some(json!({ "harness": "unknown" })));
}

fn github_credential() -> GitCredential {
    GitCredential {
        token: "github-token".to_string(),
        username: None,
        email: None,
        host: "github.com".to_string(),
    }
}

#[test]
fn bootstrap_rejects_partial_credentials_before_environment_prep() {
    let result = credentials_from_complete_response(TaskGitCredentialsResponse {
        credentials: vec![github_credential()],
        failed_hosts: vec!["gitlab.com".to_string()],
    });
    let Err(err) = result else {
        panic!("a mixed success/failure result must fail bootstrap");
    };

    let mapped = AgentDriverError::SkillResolutionFailed(format!(
        "Failed to fetch git credentials before skill resolution: {err:#}"
    ));
    assert!(mapped.to_string().contains("gitlab.com"));
    assert!(
        mapped
            .to_string()
            .contains("refusing to prepare the environment")
    );
}

#[test]
fn bootstrap_accepts_complete_credentials() {
    let result = credentials_from_complete_response(TaskGitCredentialsResponse {
        credentials: vec![github_credential()],
        failed_hosts: vec![],
    });
    let Ok(credentials) = result else {
        panic!("a complete response must proceed to credential configuration");
    };

    assert_eq!(credentials.len(), 1);
    assert_eq!(credentials[0].host, "github.com");
}
