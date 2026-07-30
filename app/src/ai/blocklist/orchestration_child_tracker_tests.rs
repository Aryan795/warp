//! Tests for [`OrchestrationChildTracker`]'s internal state machine.
//!
//! These exercise `observe_child` against a real
//! `ModelContext<OrchestrationEventStreamer>` (so the pill-bar broadcasts
//! have somewhere to go) but assert only on the tracker's own state — the
//! persisted placeholder write, metadata-fetch dispatch, and pane
//! materialization are exercised through the tracker's in-memory bookkeeping
//! (including the unified `is_remote_child` intent), so no history / network
//! plumbing is required.

use std::collections::HashSet;
use std::sync::Arc;

use warp_multi_agent_api as api;
use warpui::App;

use super::*;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::ambient_agents::{AmbientAgentTask, AmbientAgentTaskId, AmbientAgentTaskState};
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::{AIClient, MockAIClient};

const PARENT_RUN_ID: &str = "11111111-1111-1111-1111-111111111111";
const CHILD_A_RUN_ID: &str = "22222222-2222-2222-2222-222222222222";
const SESSION_A: &str = "44444444-4444-4444-4444-444444444444";

fn task_id(s: &str) -> AmbientAgentTaskId {
    s.parse().expect("hardcoded task id parses")
}

/// Builds a minimal child task row for `ChildSignal::Seeded`, parented under
/// `PARENT_RUN_ID` so `apply_seeded` treats it as a real child rather than
/// the parent's own row.
fn child_task(task_id: AmbientAgentTaskId) -> AmbientAgentTask {
    use chrono::Utc;
    AmbientAgentTask {
        task_id,
        parent_run_id: Some(PARENT_RUN_ID.to_string()),
        title: "child".to_string(),
        state: AmbientAgentTaskState::InProgress,
        prompt: "prompt".to_string(),
        created_at: Utc::now(),
        started_at: Some(Utc::now()),
        updated_at: Utc::now(),
        run_time: None,
        status_message: None,
        source: None,
        session_id: None,
        session_link: None,
        creator: None,
        executor: None,
        conversation_id: None,
        request_usage: None,
        agent_config_snapshot: None,
        artifacts: vec![],
        is_sandbox_running: false,
        last_event_sequence: None,
        children: vec![],
    }
}

/// Installs the singletons `OrchestrationEventStreamer` depends on and
/// returns the streamer handle. Mirrors the setup in
/// `orchestration_event_streamer_tests.rs`.
fn install_streamer(app: &mut App) -> warpui::ModelHandle<OrchestrationEventStreamer> {
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], vec![], &[]));
    let ai_client: Arc<dyn AIClient> = Arc::new(MockAIClient::new());
    let server_api = ServerApiProvider::new_for_test().get();
    app.add_singleton_model(|ctx| {
        OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
    })
}

fn observer_tracker() -> OrchestrationChildTracker {
    OrchestrationChildTracker::new(
        task_id(PARENT_RUN_ID),
        OrchestrationEventConsumer::Observer {
            placeholder_conversation_id: AIConversationId::new(),
        },
    )
}

#[test]
fn started_creates_pending_entry_and_is_idempotent() {
    App::test((), |mut app| async move {
        let streamer = install_streamer(&mut app);
        streamer.update(&mut app, |_streamer, ctx| {
            let mut tracker = observer_tracker();
            let killed = HashSet::new();

            tracker.observe_child(CHILD_A_RUN_ID, ChildSignal::Started, &killed, ctx);

            // A pending entry is an in-flight metadata fetch; the placeholder
            // itself is created once the fetch returns (T2).
            assert!(
                tracker.metadata_fetches.contains(CHILD_A_RUN_ID),
                "first Started must record an in-flight fetch"
            );
            assert!(
                tracker.children.is_empty(),
                "no placeholder is created before the fetch completes"
            );
            assert_eq!(tracker.metadata_fetch_dispatch_count, 1);

            // Second Started for the same run id is a no-op.
            tracker.observe_child(CHILD_A_RUN_ID, ChildSignal::Started, &killed, ctx);
            assert_eq!(
                tracker.metadata_fetch_dispatch_count, 1,
                "a repeat Started must not dispatch another fetch"
            );
        });
    });
}

#[test]
fn lifecycle_for_tombstoned_run_is_noop() {
    App::test((), |mut app| async move {
        let streamer = install_streamer(&mut app);
        streamer.update(&mut app, |_streamer, ctx| {
            let mut tracker = observer_tracker();
            let mut killed = HashSet::new();
            killed.insert(CHILD_A_RUN_ID.to_string());

            tracker.observe_child(
                CHILD_A_RUN_ID,
                ChildSignal::Lifecycle(api::LifecycleEventType::InProgress),
                &killed,
                ctx,
            );

            assert!(
                tracker.children.is_empty(),
                "tombstoned run must not create a placeholder"
            );
            assert!(
                tracker.metadata_fetches.is_empty(),
                "tombstoned run must not dispatch a metadata fetch"
            );
            assert_eq!(tracker.metadata_fetch_dispatch_count, 0);
        });
    });
}

#[test]
fn registered_prevents_placeholder_creation() {
    App::test((), |mut app| async move {
        let streamer = install_streamer(&mut app);
        streamer.update(&mut app, |_streamer, ctx| {
            let mut tracker = observer_tracker();
            let killed = HashSet::new();
            let conversation_id = AIConversationId::new();

            tracker.observe_child(
                CHILD_A_RUN_ID,
                ChildSignal::Registered { conversation_id },
                &killed,
                ctx,
            );

            let entry = tracker
                .children
                .get(&task_id(CHILD_A_RUN_ID))
                .expect("registered child is tracked immediately");
            assert_eq!(
                entry.conversation_id, conversation_id,
                "the executor-supplied conversation id is stored on the entry"
            );
            assert!(
                !entry.is_remote_child,
                "an in-band child owns a real local conversation, not an is_remote_child placeholder"
            );
            assert!(
                tracker.metadata_fetches.is_empty(),
                "an in-band child needs no discovery fetch"
            );
            assert_eq!(tracker.metadata_fetch_dispatch_count, 0);

            // A later Started for the same run id must be an idempotent no-op:
            // no placeholder creation, no metadata fetch.
            tracker.observe_child(CHILD_A_RUN_ID, ChildSignal::Started, &killed, ctx);
            assert_eq!(
                tracker.metadata_fetch_dispatch_count, 0,
                "Started for an already-registered run must not fetch"
            );
            assert!(
                tracker.children.contains_key(&task_id(CHILD_A_RUN_ID)),
                "the registered entry survives a subsequent Started"
            );
        });
    });
}

#[test]
fn session_linked_fills_session_id_and_requests_pane_without_fetch() {
    App::test((), |mut app| async move {
        let streamer = install_streamer(&mut app);
        streamer.update(&mut app, |_streamer, ctx| {
            let mut tracker = observer_tracker();
            let killed = HashSet::new();

            // Establish a tracked child first (no session id yet).
            tracker.observe_child(
                CHILD_A_RUN_ID,
                ChildSignal::Registered {
                    conversation_id: AIConversationId::new(),
                },
                &killed,
                ctx,
            );

            tracker.observe_child(
                CHILD_A_RUN_ID,
                ChildSignal::SessionLinked {
                    session_uuid: SESSION_A.to_string(),
                },
                &killed,
                ctx,
            );

            let entry = tracker
                .children
                .get(&task_id(CHILD_A_RUN_ID))
                .expect("child is tracked");
            assert_eq!(
                entry.session_id,
                Some(SESSION_A.parse().unwrap()),
                "SessionLinked fills in the session id directly"
            );
            assert!(
                entry.pane_materialized,
                "SessionLinked requests pane materialization immediately"
            );
            assert_eq!(
                tracker.metadata_fetch_dispatch_count, 0,
                "SessionLinked must not trigger a metadata fetch"
            );
        });
    });
}

#[test]
fn two_started_signals_issue_one_metadata_fetch() {
    App::test((), |mut app| async move {
        let streamer = install_streamer(&mut app);
        streamer.update(&mut app, |_streamer, ctx| {
            let mut tracker = observer_tracker();
            let killed = HashSet::new();

            tracker.observe_child(CHILD_A_RUN_ID, ChildSignal::Started, &killed, ctx);
            tracker.observe_child(CHILD_A_RUN_ID, ChildSignal::Started, &killed, ctx);

            assert_eq!(
                tracker.metadata_fetch_dispatch_count, 1,
                "two Started signals for the same run id must dedupe to one fetch"
            );
            assert!(tracker.metadata_fetches.contains(CHILD_A_RUN_ID));
        });
    });
}

#[test]
fn seeded_child_placeholder_is_remote_child_in_viewer_mode() {
    // Validation criterion 4 (TECH QUALITY-928 §7.4): a child placeholder
    // materialized by the tracker uses the single unified `is_remote_child`
    // marker even in viewer mode. The tracker never sets
    // `is_viewing_shared_session` on a child — that flavor stays reserved for
    // the parent viewer placeholder — so a viewer-created child persists as an
    // `is_remote_child` row and survives restart.
    App::test((), |mut app| async move {
        let streamer = install_streamer(&mut app);
        streamer.update(&mut app, |_streamer, ctx| {
            let mut tracker = observer_tracker();
            let killed = HashSet::new();

            tracker.observe_child(
                CHILD_A_RUN_ID,
                ChildSignal::Seeded(Box::new(child_task(task_id(CHILD_A_RUN_ID)))),
                &killed,
                ctx,
            );

            let entry = tracker
                .children
                .get(&task_id(CHILD_A_RUN_ID))
                .expect("seeded child placeholder is tracked immediately");
            assert!(
                entry.is_remote_child,
                "viewer-created child placeholders use the unified is_remote_child marker"
            );
        });
    });
}
