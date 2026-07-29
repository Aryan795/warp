//! Unified child-tracking state machine for orchestration.
//!
//! `OrchestrationChildTracker` is the single entry point for every way a
//! child run can become known — creation-time discovery
//! (`child_agent_started`), lifecycle events, sandbox session links
//! (`run_session_linked`), REST seeds/restore rows, and in-band children
//! registered by the local `StartAgentExecutor`. Owner and viewer processes
//! each hold one tracker per parent family; the only behavioral difference
//! between them is captured by [`ChildTrackingMode`] (see TECH QUALITY-928
//! §7.2).
//!
//! This is the M1 T1 slice: the tracker owns its internal state machine and
//! the classification of signals into placeholder / status / fetch / pane
//! actions. The side effects that require the broader streamer plumbing —
//! creating the persisted placeholder conversation, routing metadata fetches
//! through `AgentConversationsModel`, and materializing the child pane — are
//! stubbed here and wired up by T2 (drain integration) and M2 (unified pane
//! path). The `ctx`-driven pill-bar broadcasts (`ChildSpawned` /
//! `ChildStatusChanged`) are already emitted so downstream views can be
//! developed against them.

use std::collections::{HashMap, HashSet};

use session_sharing_protocol::common::SessionId;
use warp_multi_agent_api as api;
use warpui::ModelContext;

use super::orchestration_event_streamer::{
    OrchestrationEventStreamer, OrchestrationEventStreamerEvent,
    conversation_status_from_lifecycle_event_type,
};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::ambient_agents::{AmbientAgentTask, AmbientAgentTaskId, AmbientAgentTaskState};

/// How this process relates to the orchestrator whose children are tracked.
///
/// Mode is derived from process ownership, not configured: `Owner` iff this
/// process hosts the orchestrator conversation. It captures the only real
/// behavioral differences — inbox consumption and server-cursor authority —
/// which the drain (T2) dispatches on.
pub enum ChildTrackingMode {
    /// This process owns the orchestrator run: it consumes the parent inbox
    /// and authoritatively pushes the server-side event cursor.
    Owner {
        orchestrator_conversation_id: AIConversationId,
    },
    /// Passive view of an orchestrator owned elsewhere: lifecycle only; the
    /// cursor is persisted locally and never pushed to the server, and
    /// server-side status reporting is suppressed on placeholders.
    Viewer {
        placeholder_conversation_id: AIConversationId,
    },
}

/// Every way a child run can become known funnels into
/// [`OrchestrationChildTracker::observe_child`].
pub enum ChildSignal {
    /// `child_agent_started` on the parent run (child run id in `ref_id`).
    Started,
    /// `run_session_linked` on the child run: carries the sandbox session
    /// UUID directly, letting the tracker fill in `session_id` without a
    /// metadata fetch.
    SessionLinked { session_uuid: String },
    /// Any recognised lifecycle event on the child run.
    Lifecycle(api::LifecycleEventType),
    /// A REST seed row (cold-start seed / restore fetch). Boxed because the
    /// task row dwarfs the other variants.
    Seeded(Box<AmbientAgentTask>),
    /// A child created by this process (`run_agents` / `start_agent`): the
    /// executor registers the child it just made with its existing local
    /// conversation, marking it already-represented.
    Registered { conversation_id: AIConversationId },
}

/// Per-child orchestration state, keyed by [`AmbientAgentTaskId`].
pub struct TrackedChild {
    /// The unified placeholder (or, for in-band children, the real local
    /// conversation) representing this child.
    pub conversation_id: AIConversationId,
    /// `None` until execution is claimed and a session is linked.
    pub session_id: Option<SessionId>,
    /// Last observed task state, when known (seeded/refetched rows).
    pub last_state: Option<AmbientAgentTaskState>,
    /// True once pane materialization has been requested for this child.
    pub pane_materialized: bool,
}

/// Owns discovery, placeholder bookkeeping, claim-time metadata refetch, and
/// pane-materialization requests for one parent family, in both owner and
/// viewer modes.
pub struct OrchestrationChildTracker {
    parent_task_id: AmbientAgentTaskId,
    mode: ChildTrackingMode,
    /// Materialized children keyed by task id.
    children: HashMap<AmbientAgentTaskId, TrackedChild>,
    /// Secondary index from stringified `run_id` to task id, kept in sync
    /// with `children`.
    children_by_run_id: HashMap<String, AmbientAgentTaskId>,
    /// In-band children created by this process (`ChildSignal::Registered`).
    /// They already own a real conversation and have their session assigned
    /// by the executor, so the tracker observes them for status only and
    /// never issues a discovery/claim metadata fetch on their behalf.
    in_band_children: HashSet<AmbientAgentTaskId>,
    /// In-flight metadata fetches keyed by `run_id`. Unifies today's
    /// `remote_child_placeholder_fetches` guard and OVM's dispatch guard so a
    /// second discovery signal for a run already being fetched is a no-op.
    metadata_fetches: HashSet<String>,
    /// Session ids delivered by `run_session_linked` before the child's
    /// placeholder exists; applied when the child is created.
    pending_session_ids: HashMap<AmbientAgentTaskId, SessionId>,
    /// Test-only: counts stubbed metadata-fetch dispatches so fetch dedup can
    /// be asserted without the full `AgentConversationsModel` plumbing.
    #[cfg(test)]
    metadata_fetch_dispatch_count: usize,
}

impl OrchestrationChildTracker {
    /// Builds an empty tracker for the given parent family and mode.
    pub fn new(parent_task_id: AmbientAgentTaskId, mode: ChildTrackingMode) -> Self {
        Self {
            parent_task_id,
            mode,
            children: HashMap::new(),
            children_by_run_id: HashMap::new(),
            in_band_children: HashSet::new(),
            metadata_fetches: HashSet::new(),
            pending_session_ids: HashMap::new(),
            #[cfg(test)]
            metadata_fetch_dispatch_count: 0,
        }
    }

    /// The single entry point for all child state changes. Implements the
    /// four-step logic from TECH QUALITY-928 §7.2:
    ///
    /// 0. Drop tombstoned runs (and, in T2, runs owned by a non-placeholder
    ///    local conversation).
    /// 1. Create-or-update the placeholder (`is_remote_child = true`).
    /// 2. Write status through on `Lifecycle` signals (sole status writer).
    /// 3. Refetch metadata while `session_id` is missing or the pane is not
    ///    materialized.
    /// 4. Request pane materialization once `session_id` is known, or a
    ///    transcript view once terminal.
    pub fn observe_child(
        &mut self,
        child_run_id: &str,
        signal: ChildSignal,
        killed_run_ids: &HashSet<String>,
        ctx: &mut ModelContext<OrchestrationEventStreamer>,
    ) {
        // Step 0: tombstone gate. This runs before any placeholder creation
        // or pane request — including across the metadata-fetch await and the
        // cancel-during-spawn race — so a locally killed run cannot be
        // resurrected mid-fetch.
        //
        // T2: also drop runs owned by a non-placeholder local conversation
        // (local in-band children observed for status only), by consulting
        // `BlocklistAIHistoryModel`.
        if killed_run_ids.contains(child_run_id) {
            self.forget_run(child_run_id);
            return;
        }

        let Ok(task_id) = child_run_id.parse::<AmbientAgentTaskId>() else {
            log::warn!(
                "[orch-tracker] signal for malformed run_id={child_run_id:?} \
                 (parent_task_id={}); dropping",
                self.parent_task_id,
            );
            return;
        };

        match signal {
            ChildSignal::Registered { conversation_id } => {
                self.register_in_band_child(task_id, child_run_id, conversation_id, ctx);
            }
            ChildSignal::SessionLinked { session_uuid } => {
                self.apply_session_linked(task_id, &session_uuid);
            }
            ChildSignal::Lifecycle(kind) => {
                self.apply_lifecycle(task_id, child_run_id, kind, ctx);
            }
            ChildSignal::Seeded(task) => {
                self.apply_seeded(*task, ctx);
            }
            ChildSignal::Started => {
                self.apply_started(task_id, child_run_id, ctx);
            }
        }
    }

    /// Discovery via `child_agent_started`. Idempotent: an already-known child
    /// (in-band `Registered`, existing placeholder) or a run with an in-flight
    /// fetch only re-drives step 3/4; the first sighting of a genuinely new
    /// out-of-band run kicks a single metadata fetch to create its
    /// placeholder.
    fn apply_started(
        &mut self,
        task_id: AmbientAgentTaskId,
        run_id: &str,
        ctx: &mut ModelContext<OrchestrationEventStreamer>,
    ) {
        if self.children.contains_key(&task_id) {
            // Already represented; keep hydrating if not yet complete.
            self.refetch_metadata_if_incomplete(task_id, run_id);
            self.maybe_request_pane_materialization(task_id, ctx);
            return;
        }
        // New out-of-band child: start (or dedupe) discovery. The placeholder
        // is created when the fetch completes (T2 wires the completion path).
        self.spawn_metadata_fetch(run_id);
    }

    /// Sole status writer for placeholder children (step 2). Emits the pill-bar
    /// broadcast in both modes; the actual `update_conversation_status` write
    /// is wired in T2 once the tracker owns a history handle. Unknown children
    /// fall back to the discovery path so lifecycle acts as a self-healing
    /// backstop for a missed `child_agent_started`.
    fn apply_lifecycle(
        &mut self,
        task_id: AmbientAgentTaskId,
        run_id: &str,
        kind: api::LifecycleEventType,
        ctx: &mut ModelContext<OrchestrationEventStreamer>,
    ) {
        if self.children.contains_key(&task_id) {
            let status = conversation_status_from_lifecycle_event_type(kind);
            // T2: also write status through `BlocklistAIHistoryModel`.
            ctx.emit(OrchestrationEventStreamerEvent::ChildStatusChanged {
                parent_task_id: self.parent_task_id,
                run_id: run_id.to_string(),
                status,
            });
            self.refetch_metadata_if_incomplete(task_id, run_id);
            self.maybe_request_pane_materialization(task_id, ctx);
            return;
        }
        // Lifecycle for an unknown run: only self-heal a real discovery miss,
        // not a run whose fetch is already in flight.
        if !self.metadata_fetches.contains(run_id) {
            self.spawn_metadata_fetch(run_id);
        }
    }

    /// Registers an in-band child (created by this process) with its existing
    /// local conversation. Marks the run already-represented so later
    /// `Started`/`Lifecycle` signals are idempotent status updates rather than
    /// placeholder creation, replacing the old implicit
    /// `conversation_id_for_agent_id(...).is_none()` guard.
    fn register_in_band_child(
        &mut self,
        task_id: AmbientAgentTaskId,
        run_id: &str,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<OrchestrationEventStreamer>,
    ) {
        // An in-band child has a real conversation, so any speculative fetch
        // for it is moot.
        self.metadata_fetches.remove(run_id);
        self.in_band_children.insert(task_id);
        if let Some(existing) = self.children.get_mut(&task_id) {
            existing.conversation_id = conversation_id;
            return;
        }
        let session_id = self.pending_session_ids.remove(&task_id);
        self.insert_child(
            task_id,
            run_id,
            TrackedChild {
                conversation_id,
                session_id,
                last_state: None,
                pane_materialized: false,
            },
            ctx,
        );
        // If the session link already arrived, hydrate the pane immediately.
        self.maybe_request_pane_materialization(task_id, ctx);
    }

    /// Applies a REST seed / restore row. Creates the placeholder if new and
    /// records the latest known state and session id.
    fn apply_seeded(
        &mut self,
        task: AmbientAgentTask,
        ctx: &mut ModelContext<OrchestrationEventStreamer>,
    ) {
        // The ancestor endpoint includes the parent itself in the response;
        // skip it.
        if task.task_id == self.parent_task_id {
            return;
        }
        let task_id = task.task_id;
        let run_id = task_id.to_string();
        let seed_session_id = task
            .session_id
            .as_deref()
            .and_then(|s| s.parse::<SessionId>().ok());
        let state = task.state.clone();

        self.metadata_fetches.remove(&run_id);

        if let Some(existing) = self.children.get_mut(&task_id) {
            existing.last_state = Some(state);
            if existing.session_id.is_none() {
                existing.session_id = seed_session_id;
            }
            self.maybe_request_pane_materialization(task_id, ctx);
            return;
        }

        // T2 creates the persisted `is_remote_child` placeholder conversation
        // via `BlocklistAIHistoryModel`; T1 records the tracked entry against a
        // placeholder id so the state machine is exercisable. Until that
        // plumbing lands, fall back to any pending session link.
        let session_id = seed_session_id.or_else(|| self.pending_session_ids.remove(&task_id));
        let conversation_id = self.placeholder_conversation_id();
        self.insert_child(
            task_id,
            &run_id,
            TrackedChild {
                conversation_id,
                session_id,
                last_state: Some(state),
                pane_materialized: false,
            },
            ctx,
        );
        self.maybe_request_pane_materialization(task_id, ctx);
    }

    /// Handles `run_session_linked`: fills in `session_id` directly (no
    /// metadata fetch) and requests pane materialization immediately. If the
    /// placeholder does not exist yet, the session id is stashed and applied
    /// when the child is created.
    fn apply_session_linked(&mut self, task_id: AmbientAgentTaskId, session_uuid: &str) {
        let Ok(session_id) = session_uuid.parse::<SessionId>() else {
            log::warn!(
                "[orch-tracker] run_session_linked with malformed session_uuid={session_uuid:?} \
                 for task_id={task_id} (parent_task_id={}); dropping",
                self.parent_task_id,
            );
            return;
        };
        match self.children.get_mut(&task_id) {
            Some(child) => {
                if child.session_id.is_none() {
                    child.session_id = Some(session_id);
                }
                // Request the live pane now that the session is known,
                // bypassing the metadata-fetch round-trip.
                self.request_pane_materialization(task_id);
            }
            None => {
                self.pending_session_ids.insert(task_id, session_id);
            }
        }
    }

    /// Re-drives step 3: refetch while `session_id` is missing or the pane has
    /// not been materialized. No-op once the child is fully hydrated.
    fn refetch_metadata_if_incomplete(&mut self, task_id: AmbientAgentTaskId, run_id: &str) {
        // In-band children are hydrated by the executor, not the tracker.
        if self.in_band_children.contains(&task_id) {
            return;
        }
        let incomplete = self
            .children
            .get(&task_id)
            .is_some_and(|child| child.session_id.is_none() || !child.pane_materialized);
        if incomplete {
            self.spawn_metadata_fetch(run_id);
        }
    }

    /// Step 4: request pane materialization once a `session_id` is known.
    /// No-op stub in T1 — M2 implements the unified pane path.
    fn maybe_request_pane_materialization(
        &mut self,
        task_id: AmbientAgentTaskId,
        _ctx: &mut ModelContext<OrchestrationEventStreamer>,
    ) {
        let should_request = self
            .children
            .get(&task_id)
            .is_some_and(|child| child.session_id.is_some() && !child.pane_materialized);
        if should_request {
            self.request_pane_materialization(task_id);
        }
    }

    /// Marks the child's pane as materialized. T1 records intent only; M2
    /// dispatches the actual live-attach / transcript materialization.
    fn request_pane_materialization(&mut self, task_id: AmbientAgentTaskId) {
        if let Some(child) = self.children.get_mut(&task_id) {
            child.pane_materialized = true;
        }
    }

    /// Records a new tracked child, keeping both indices in sync, and emits
    /// the `ChildSpawned` pill-bar broadcast exactly once.
    fn insert_child(
        &mut self,
        task_id: AmbientAgentTaskId,
        run_id: &str,
        child: TrackedChild,
        ctx: &mut ModelContext<OrchestrationEventStreamer>,
    ) {
        self.children.insert(task_id, child);
        self.children_by_run_id.insert(run_id.to_string(), task_id);
        ctx.emit(OrchestrationEventStreamerEvent::ChildSpawned {
            parent_task_id: self.parent_task_id,
            run_id: run_id.to_string(),
        });
    }

    /// Starts (or dedupes) a metadata fetch for a run. T2 routes this through
    /// `AgentConversationsModel::get_or_async_fetch_task_data` (the one fetch
    /// authority with in-flight dedup, cooldowns, and a shared cache); T1
    /// tracks the in-flight guard and, in tests, counts dispatches so fetch
    /// dedup can be asserted.
    fn spawn_metadata_fetch(&mut self, run_id: &str) {
        if !self.metadata_fetches.insert(run_id.to_string()) {
            // Already in flight: do not issue a second fetch.
            return;
        }
        #[cfg(test)]
        {
            self.metadata_fetch_dispatch_count += 1;
        }
        log::debug!(
            "[orch-tracker] metadata fetch queued for run_id={run_id} \
             (parent_task_id={})",
            self.parent_task_id,
        );
    }

    /// Drops all tracked state for a run (tombstone / kill path).
    fn forget_run(&mut self, run_id: &str) {
        self.metadata_fetches.remove(run_id);
        if let Some(task_id) = self.children_by_run_id.remove(run_id) {
            self.children.remove(&task_id);
            self.in_band_children.remove(&task_id);
            self.pending_session_ids.remove(&task_id);
        }
    }

    /// Test-only: number of metadata-fetch dispatches issued so far. Lets
    /// drain-integration tests in `orchestration_event_streamer_tests.rs`
    /// (a sibling module without access to private fields) assert fetch
    /// dedup.
    #[cfg(test)]
    pub(crate) fn metadata_fetch_dispatch_count(&self) -> usize {
        self.metadata_fetch_dispatch_count
    }

    /// Test-only: whether a metadata fetch is currently in flight for
    /// `run_id`. Used by sibling-module drain tests to assert discovery and
    /// lifecycle signals were routed into the tracker.
    #[cfg(test)]
    pub(crate) fn has_in_flight_fetch(&self, run_id: &str) -> bool {
        self.metadata_fetches.contains(run_id)
    }

    /// Resolves the placeholder conversation id to associate a tracked child
    /// with before T2's per-child placeholder creation lands. Both modes reuse
    /// the mode's conversation id as a stable stand-in.
    fn placeholder_conversation_id(&self) -> AIConversationId {
        match &self.mode {
            ChildTrackingMode::Owner {
                orchestrator_conversation_id,
            } => *orchestrator_conversation_id,
            ChildTrackingMode::Viewer {
                placeholder_conversation_id,
            } => *placeholder_conversation_id,
        }
    }
}

#[cfg(test)]
#[path = "orchestration_child_tracker_tests.rs"]
mod tests;
