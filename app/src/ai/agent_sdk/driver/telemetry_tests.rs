use serde_json::json;
use warp_core::telemetry::TelemetryEvent as _;

use super::*;

#[test]
fn episode_resolved_reports_stamp_and_checkpoint_fields() {
    let event = WaitForEventsTelemetryEvent::EpisodeResolved(WaitForEventsEpisodeResolvedEvent {
        task_id: Some("task-123".to_string()),
        execution_id: "exec-456".to_string(),
        server_idle_timeout_seconds: 1800,
        used_fallback: false,
        resolved_watchdog_seconds: 1770,
        hibernate_on_first_timeout_enabled: true,
        wait_outcome: WaitForEventsOutcome::Timeout,
        checkpoint_outcome: WaitForEventsCheckpointOutcome::Succeeded,
    });

    assert_eq!(event.name(), "AmbientAgents.WaitForEvents.EpisodeResolved");
    assert!(!event.contains_ugc());
    assert_eq!(
        event.payload(),
        Some(json!({
            "task_id": "task-123",
            "execution_id": "exec-456",
            "server_idle_timeout_seconds": 1800,
            "used_fallback": false,
            "resolved_watchdog_seconds": 1770,
            "hibernate_on_first_timeout_enabled": true,
            "wait_outcome": "timeout",
            "checkpoint_outcome": "succeeded",
        }))
    );
}

#[test]
fn episode_resolved_omits_task_id_when_absent_and_reports_failed_checkpoint() {
    let event = WaitForEventsTelemetryEvent::EpisodeResolved(WaitForEventsEpisodeResolvedEvent {
        task_id: None,
        execution_id: "exec-456".to_string(),
        server_idle_timeout_seconds: 0,
        used_fallback: true,
        resolved_watchdog_seconds: 1770,
        hibernate_on_first_timeout_enabled: true,
        wait_outcome: WaitForEventsOutcome::Timeout,
        checkpoint_outcome: WaitForEventsCheckpointOutcome::from_succeeded(false),
    });

    let payload = event.payload().unwrap();
    assert!(!payload.as_object().unwrap().contains_key("task_id"));
    assert_eq!(payload["used_fallback"], json!(true));
    assert_eq!(payload["checkpoint_outcome"], json!("failed"));
}
