use serde_json::json;
use warp_cli::CliCommand;
use warp_cli::agent::Harness;
use warp_cli::artifact::{
    ArtifactCommand, DownloadArtifactArgs, GetArtifactArgs, UploadArtifactArgs,
};
use warp_cli::task::{MessageCommand, MessageSendArgs, MessageWatchArgs, TaskCommand};
use warp_core::telemetry::TelemetryEvent;
use warp_isolation_platform::IsolationPlatformError;

use super::{
    AgentDriverError, AgentDriverRunner, CommandAuthentication, command_authentication,
    command_requires_auth, command_to_telemetry_event, reconcile_task_harness,
};

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

// ── apply_fetched_git_credentials ───────────────────────────────────────────
//
// These cover `bootstrap_git_credentials_for_task`'s fetch-outcome handling, factored out into
// `apply_fetched_git_credentials` specifically so it can be tested without depending on the
// real, process-wide isolation-platform detection (`warp_isolation_platform::detect()` is
// memoized for the life of the process and reflects the actual host, which in a cloud sandbox
// can itself be an isolation platform). A missing workload token surfaces from the real fetch
// path as `IsolationPlatformError::NoIsolationPlatformDetected`, which these tests construct
// directly. All cases use an empty or erroring credential result, so
// `driver::git_credentials::configure_git_credentials` (which mutates the real git config and
// credential files) is never reached.

fn workload_token_missing_error() -> anyhow::Error {
    IsolationPlatformError::NoIsolationPlatformDetected.into()
}

#[test]
fn missing_workload_token_skips_on_the_isolation_platform_path() {
    let result = AgentDriverRunner::apply_fetched_git_credentials(
        Err(workload_token_missing_error()),
        false,
    );

    assert!(
        result.is_ok(),
        "a missing workload token must be skipped gracefully, not treated as a failure"
    );
}

#[test]
fn missing_workload_token_skips_on_the_gh_configured_path() {
    let result =
        AgentDriverRunner::apply_fetched_git_credentials(Err(workload_token_missing_error()), true);

    assert!(
        result.is_ok(),
        "a missing workload token must be skipped gracefully on the gh-configured path too"
    );
}

#[test]
fn empty_credential_response_is_a_no_op_success() {
    // Represents a successful fetch (e.g. via the env-token fallback) that simply had no
    // credentials to apply.
    let result = AgentDriverRunner::apply_fetched_git_credentials(Ok(Vec::new()), false);

    assert!(
        result.is_ok(),
        "an empty credential response from the server should be a no-op success"
    );
}

#[test]
fn gh_configured_host_tolerates_a_credentials_fetch_failure() {
    // The regression this covers: before this change, a non-isolation-platform host with gh
    // credentials already configured never attempted this fetch. Now that it does, a fetch
    // failure here must not fail a run that is otherwise viable on the gh credentials alone.
    let result = AgentDriverRunner::apply_fetched_git_credentials(
        Err(anyhow::anyhow!("server rejected workload token")),
        true,
    );

    assert!(
        result.is_ok(),
        "a fetch failure must be tolerated when gh credentials are already configured"
    );
}

#[test]
fn isolation_platform_host_still_fails_hard_on_a_credentials_fetch_failure() {
    // The isolation-platform path has no gh fallback, so a fetch failure there must remain
    // fatal; this guards against the task-1 leniency accidentally widening to that path.
    let result = AgentDriverRunner::apply_fetched_git_credentials(
        Err(anyhow::anyhow!("server rejected workload token")),
        false,
    );

    match result {
        Err(AgentDriverError::SkillResolutionFailed(message)) => {
            assert!(message.contains("server rejected workload token"));
        }
        other => panic!("expected a fatal SkillResolutionFailed error, got {other:?}"),
    }
}
