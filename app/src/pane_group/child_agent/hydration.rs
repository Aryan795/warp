use session_sharing_protocol::common::SessionId;
use uuid::Uuid;
use warp_errors::report_error;
use warpui::{SingletonEntity, ViewContext};

use super::materialization::{
    ChildPaneMaterialization, ChildPaneMaterializationMode, decide_child_pane_materialization,
};
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::history_model::CloudConversationData;
use crate::pane_group::{
    AmbientAgentViewModelHandleExt, PaneGroup, PaneId, TerminalPane, TerminalViewResources,
};
use crate::terminal::view::load_ai_conversation::{
    RestoreConversationEntryBehavior, RestoredAIConversation,
};

impl PaneGroup {
    /// Single dispatch for every placeholder-child pane — the `is_remote_child`
    /// (owner) and `is_viewing_shared_session` (viewer) branches of
    /// [`Self::create_hidden_child_agent_pane`] both funnel here.
    ///
    /// Fetches the child's [`AmbientAgentTask`](crate::ai::ambient_agents::AmbientAgentTask)
    /// and dispatches on [`decide_child_pane_materialization`], which makes the
    /// same live / transcript / pending choice for both modes. `mode` selects
    /// only the pane *construction* strategy.
    ///
    /// Idempotent: skipped when the placeholder already has a live tracked
    /// pane, so repeat calls from `restore_missing_child_agent_panes_for_parent`
    /// don't create a duplicate pane and orphan the first.
    pub(in crate::pane_group) fn materialize_child_placeholder_pane(
        &mut self,
        child_conversation: AIConversation,
        parent_pane_id: PaneId,
        mode: ChildPaneMaterializationMode,
        ctx: &mut ViewContext<Self>,
    ) {
        let child_id = child_conversation.id();

        // Idempotency guard — see fn doc.
        if let Some(existing_pane_id) = self.child_agent_panes.get(&child_id).copied()
            && self.has_pane_id(existing_pane_id)
        {
            return;
        }

        let task_id = child_conversation.task_id();
        let materialization = task_id
            .and_then(|task_id| {
                AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
                    model.get_or_async_fetch_task_data(&task_id, ctx)
                })
            })
            .as_ref()
            .map(decide_child_pane_materialization);

        match mode {
            ChildPaneMaterializationMode::Owner => {
                self.materialize_owner_child_pane(
                    child_conversation,
                    parent_pane_id,
                    task_id,
                    materialization,
                    ctx,
                );
            }
            ChildPaneMaterializationMode::Viewer => {
                self.materialize_viewer_child_pane(
                    child_conversation,
                    parent_pane_id,
                    task_id,
                    materialization,
                    ctx,
                );
            }
        }
    }

    /// Owner-mode arm of [`Self::materialize_child_placeholder_pane`]. Always
    /// creates the hidden ambient pane and registers it in `child_agent_panes`
    /// (so the orchestration pill can reveal it), then dispatches:
    /// `AttachLive` joins the live session, `LoadTranscript` merges the cloud
    /// transcript, and `Pending` (or missing task data) leaves the bare
    /// ambient shell for the tracker to re-drive on the next lifecycle /
    /// session-linked event.
    fn materialize_owner_child_pane(
        &mut self,
        child_conversation: AIConversation,
        parent_pane_id: PaneId,
        task_id: Option<AmbientAgentTaskId>,
        materialization: Option<ChildPaneMaterialization>,
        ctx: &mut ViewContext<Self>,
    ) {
        let child_id = child_conversation.id();
        let Some(task_id) = task_id else {
            log::warn!("Cannot restore remote child conversation {child_id:?} without a task ID");
            return;
        };

        let Some(pane_id) =
            self.create_hidden_ambient_child_pane(child_conversation, parent_pane_id, ctx)
        else {
            return;
        };

        match materialization {
            Some(ChildPaneMaterialization::AttachLive { session_id }) => {
                self.attach_child_session(
                    child_id,
                    session_id,
                    ChildPaneMaterializationMode::Owner,
                    ctx,
                );
            }
            Some(ChildPaneMaterialization::LoadTranscript { server_token }) => {
                self.hydrate_child_transcript_in_place(
                    pane_id,
                    child_id,
                    task_id,
                    server_token,
                    ctx,
                );
            }
            Some(ChildPaneMaterialization::Pending) | None => {
                // Pending: the bare ambient shell stays un-materialized; the
                // tracker re-drives on the next lifecycle / session-linked
                // signal (an async task fetch was already kicked above).
            }
        }
    }

    /// Viewer-mode arm of [`Self::materialize_child_placeholder_pane`].
    /// `AttachLive` creates a dedicated shared-session viewer pane;
    /// `LoadTranscript` loads the cloud transcript into a hidden ambient pane
    /// (a terminal child has no live session to join, so both modes load the
    /// transcript identically — the server ACL prerequisite grants viewers
    /// access); `Pending` leaves a loading placeholder that
    /// `OrchestrationViewerModel` re-drives via
    /// `EnsureSharedSessionViewerChildPane` once a session id surfaces.
    fn materialize_viewer_child_pane(
        &mut self,
        child_conversation: AIConversation,
        parent_pane_id: PaneId,
        task_id: Option<AmbientAgentTaskId>,
        materialization: Option<ChildPaneMaterialization>,
        ctx: &mut ViewContext<Self>,
    ) {
        let child_id = child_conversation.id();
        match (materialization, task_id) {
            (Some(ChildPaneMaterialization::AttachLive { session_id }), _) => {
                self.attach_child_session(
                    child_id,
                    session_id,
                    ChildPaneMaterializationMode::Viewer,
                    ctx,
                );
            }
            (Some(ChildPaneMaterialization::LoadTranscript { server_token }), Some(task_id)) => {
                let Some(pane_id) =
                    self.create_hidden_ambient_child_pane(child_conversation, parent_pane_id, ctx)
                else {
                    return;
                };
                self.hydrate_child_transcript_in_place(
                    pane_id,
                    child_id,
                    task_id,
                    server_token,
                    ctx,
                );
            }
            _ => {
                // Pending / no task data yet: render a loading placeholder. The
                // real pane is swapped in by `attach_child_session` (viewer
                // arm) once `OrchestrationViewerModel` surfaces a session id.
                self.create_viewer_loading_child_placeholder(child_conversation, ctx);
            }
        }
    }

    /// Converged live-session attach for both modes (replaces the owner's old
    /// in-place ambient attach and the viewer's dedicated-pane creation).
    ///
    /// Owner mode attaches the already-created hidden ambient pane to the live
    /// session; viewer mode materializes a dedicated shared-session viewer
    /// pane with its own `Network` + `BlocklistAIController`.
    pub(in crate::pane_group) fn attach_child_session(
        &mut self,
        child_id: AIConversationId,
        session_id: SessionId,
        mode: ChildPaneMaterializationMode,
        ctx: &mut ViewContext<Self>,
    ) {
        match mode {
            ChildPaneMaterializationMode::Owner => {
                let Some(pane_id) = self
                    .child_agent_panes
                    .get(&child_id)
                    .copied()
                    .filter(|pane_id| self.has_pane_id(*pane_id))
                else {
                    return;
                };
                let Some(task_id) = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&child_id)
                    .and_then(|conversation| conversation.task_id())
                else {
                    return;
                };
                self.apply_existing_ambient_task_to_pane(pane_id, child_id, task_id, ctx);
            }
            ChildPaneMaterializationMode::Viewer => {
                self.attach_viewer_child_session(child_id, session_id, ctx);
            }
        }
    }

    /// Creates a hidden cloud-mode ambient pane for a child placeholder,
    /// restores the placeholder conversation into it, enters agent view, and
    /// registers the pane in `child_agent_panes` keyed by the placeholder's
    /// local `AIConversationId`. Returns the new pane id, or `None` if pane or
    /// view creation failed.
    fn create_hidden_ambient_child_pane(
        &mut self,
        child_conversation: AIConversation,
        parent_pane_id: PaneId,
        ctx: &mut ViewContext<Self>,
    ) -> Option<PaneId> {
        let child_id = child_conversation.id();
        let new_pane_id =
            self.insert_ambient_agent_pane_hidden_for_child_agent(parent_pane_id, ctx);

        let Some(new_terminal_view) = self.terminal_view_from_pane_id(new_pane_id, ctx) else {
            report_error!(
                "Failed to get terminal view for remote child agent pane",
                extra: { "child_id" => ?child_id }
            );
            self.discard_pane(new_pane_id.into(), ctx);
            return None;
        };

        // Restore the placeholder so the pane has parent linkage + agent name
        // before materialization runs.
        let mut restored = false;
        new_terminal_view.update(ctx, |terminal_view, ctx| {
            terminal_view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(child_conversation),
                true,
                RestoreConversationEntryBehavior::PreserveAgentViewState,
                ctx,
            );
            terminal_view.enter_agent_view(
                None,
                Some(child_id),
                AgentViewEntryOrigin::CloudAgent,
                ctx,
            );
            restored = terminal_view
                .ambient_agent_view_model()
                .into_optional_handle()
                .is_some();
        });

        if !restored {
            report_error!(
                "Failed to restore remote child agent pane: missing ambient agent view model",
                extra: { "child_id" => ?child_id }
            );
            self.discard_pane(new_pane_id.into(), ctx);
            return None;
        }

        // Placeholder's local id stays the canonical `child_agent_panes` key
        // across live-attach and transcript hydration.
        self.child_agent_panes.insert(child_id, new_pane_id.into());
        Some(new_pane_id.into())
    }

    /// Attaches the hidden child pane's ambient agent view model to the live
    /// ambient session for `task_id`. Wrapper around
    /// `AmbientAgentViewModel::enter_viewing_existing_session` that also sets
    /// the active conversation id.
    fn apply_existing_ambient_task_to_pane(
        &mut self,
        pane_id: PaneId,
        child_id: AIConversationId,
        task_id: AmbientAgentTaskId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(terminal_view) = self.terminal_view_from_pane_id(pane_id, ctx) else {
            return;
        };
        terminal_view.update(ctx, |terminal_view, ctx| {
            let Some(ambient_agent_view_model) = terminal_view
                .ambient_agent_view_model()
                .into_optional_handle()
                .cloned()
            else {
                return;
            };
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.set_conversation_id(Some(child_id));
                model.enter_viewing_existing_session(task_id, ctx);
            });
        });
    }

    /// Fetches the cloud transcript identified by `server_token`, hydrates the
    /// placeholder via
    /// `hydrate_remote_child_placeholder_with_cloud_transcript`, and
    /// re-restores the merged conversation into the pane. Used for terminal
    /// (completed) children in both owner and viewer modes; a completed run
    /// always ends with the conversation-ended tombstone.
    fn hydrate_child_transcript_in_place(
        &mut self,
        pane_id: PaneId,
        child_id: AIConversationId,
        task_id: AmbientAgentTaskId,
        server_token: ServerConversationToken,
        ctx: &mut ViewContext<Self>,
    ) {
        let history_handle = BlocklistAIHistoryModel::handle(ctx);
        let future = history_handle.update(ctx, |history_model, ctx| {
            history_model.load_conversation_by_server_token(&server_token, ctx)
        });
        ctx.spawn(future, move |group, conversation, ctx| {
            // Guard against a stale target while the fetch was in flight: the
            // pane id must still be the canonical one for `child_id` AND the
            // pane's terminal view must still be displaying it.
            let still_canonical = group
                .child_agent_panes
                .get(&child_id)
                .copied()
                .is_some_and(|p| p == pane_id && group.has_pane_id(p));
            if !still_canonical {
                return;
            }
            let terminal_view_active_conversation = group
                .terminal_view_from_pane_id(pane_id, ctx)
                .and_then(|tv| tv.as_ref(ctx).active_conversation_id(ctx));
            if terminal_view_active_conversation != Some(child_id) {
                return;
            }

            match conversation {
                Some(CloudConversationData::Oz(cloud)) => {
                    let tasks: Vec<warp_multi_agent_api::Task> = cloud
                        .all_tasks()
                        .filter_map(|task| task.source().cloned())
                        .collect();
                    let cloud_conversation = *cloud;
                    let merge_result =
                        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _| {
                            history.hydrate_remote_child_placeholder_with_cloud_transcript(
                                child_id,
                                tasks,
                                cloud_conversation,
                            )
                        });
                    match merge_result {
                        Ok(merged) => {
                            if let Some(terminal_view) =
                                group.terminal_view_from_pane_id(pane_id, ctx)
                            {
                                terminal_view.update(ctx, |view, ctx| {
                                    view.restore_conversation_after_view_creation(
                                        RestoredAIConversation::new(merged),
                                        true,
                                        RestoreConversationEntryBehavior::PreserveAgentViewState,
                                        ctx,
                                    );
                                });
                            }
                        }
                        Err(err) => {
                            log::warn!(
                                "hydrate_remote_child_placeholder_with_cloud_transcript failed for {child_id:?}: {err:#}"
                            );
                        }
                    }
                }
                Some(CloudConversationData::CLIAgent(_)) | None => {
                    // Non-Oz transcript or fetch failure — the post-match call
                    // still attaches and inserts the ended tombstone.
                }
            }

            // Uniform post-match step: attach the (ended) ambient session and
            // insert the conversation-ended tombstone. `LoadTranscript` is only
            // chosen for terminal runs, so the tombstone always applies.
            group.apply_existing_ambient_task_to_pane(pane_id, child_id, task_id, ctx);
            if let Some(terminal_view) = group.terminal_view_from_pane_id(pane_id, ctx) {
                terminal_view.update(ctx, |view, ctx| {
                    view.insert_conversation_ended_tombstone_with_resolved_cta(ctx);
                });
            }
        });
    }

    /// Renders a loading placeholder pane for a viewer-side child that was
    /// clicked / restored before `OrchestrationViewerModel` surfaced a
    /// `session_id`. The real pane gets swapped in by
    /// [`Self::attach_child_session`] (viewer arm).
    fn create_viewer_loading_child_placeholder(
        &mut self,
        child_conversation: AIConversation,
        ctx: &mut ViewContext<Self>,
    ) {
        let child_id = child_conversation.id();
        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };
        let view_size = Self::estimated_view_bounds(ctx).size();
        let (loading_view, loading_manager) = Self::create_loading_terminal_manager_and_view(
            resources,
            view_size,
            ctx.window_id(),
            ctx,
        );
        let pane_data = TerminalPane::new(
            Uuid::new_v4().as_bytes().to_vec(),
            loading_manager,
            loading_view.clone(),
            self.model_event_sender.clone(),
            ctx,
        );
        let new_pane_id = pane_data.terminal_pane_id();
        if self
            .attach_child_pane_off_tree(Box::new(pane_data), ctx)
            .is_none()
        {
            report_error!(
                "create_viewer_loading_child_placeholder: failed to attach loading placeholder for \
                 viewer-side child",
                extra: { "child_id" => ?child_id }
            );
            return;
        }

        // Restore the conversation and enter agent view so the pill bar renders
        // (its gate requires `is_fullscreen()`). The output area stays a loading
        // spinner because the loading view's
        // `ConversationTranscriptViewerStatus::Loading` short-circuits the
        // block list render in `TerminalView::render`.
        loading_view.update(ctx, |terminal_view, ctx| {
            terminal_view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(child_conversation),
                true,
                RestoreConversationEntryBehavior::PreserveAgentViewState,
                ctx,
            );
            terminal_view.enter_agent_view(
                None,
                Some(child_id),
                AgentViewEntryOrigin::SharedSessionSelection,
                ctx,
            );
        });

        self.child_agent_panes.insert(child_id, new_pane_id.into());
    }

    /// Viewer arm of [`Self::attach_child_session`]: materializes a dedicated
    /// hidden shared-session viewer pane for a viewer-discovered child agent.
    /// Triggered from the unified dispatch (`AttachLive`) and from
    /// `Event::EnsureSharedSessionViewerChildPane`, which
    /// `OrchestrationViewerModel` emits the first time it observes a
    /// `session_id` for a child. The new pane gets its own
    /// `BlocklistAIController` and viewer-side `Network` so child traffic
    /// doesn't cross the parent's single-stream state.
    fn attach_viewer_child_session(
        &mut self,
        child_conversation_id: AIConversationId,
        child_session_id: SessionId,
        ctx: &mut ViewContext<Self>,
    ) {
        // Race recovery: a pill click / restore before materialization had a
        // `session_id` falls through to the viewer loading placeholder, which
        // leaves an entry in `child_agent_panes`. The emission gate in
        // `OrchestrationViewerModel` guarantees the viewer attach runs at most
        // once per child per model lifetime, so any existing entry must be that
        // fallback — safe to discard.
        let fallback_was_swapped_anchor = if let Some(prior_pane_id) = self
            .child_agent_panes
            .get(&child_conversation_id)
            .copied()
            .filter(|pane_id| self.has_pane_id(*pane_id))
        {
            let anchor = self.panes.original_pane_for_replacement(prior_pane_id);
            self.discard_child_agent_pane_for_conversation(child_conversation_id, ctx);
            anchor
        } else {
            None
        };

        let Some(child_conversation) = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&child_conversation_id)
            .cloned()
        else {
            log::warn!(
                "attach_viewer_child_session: no local conversation {child_conversation_id:?}"
            );
            return;
        };
        let child_task_id = child_conversation.task_id();

        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };
        let view_size = Self::estimated_view_bounds(ctx).size();
        // Per-child viewer: parent's model already discovers descendants, and
        // hidden child viewers aren't snapshotted, so `is_cloud_mode` stays
        // `false` (no `ambient_agent_view_model` needed for snapshot round-trip).
        let (new_terminal_view, terminal_manager) = Self::create_shared_session_viewer(
            child_session_id,
            resources,
            view_size,
            false, // enable_orchestration_polling
            false, // is_ambient_agent
            ctx,
        );

        let pane_data = TerminalPane::new(
            Uuid::new_v4().as_bytes().to_vec(),
            terminal_manager,
            new_terminal_view.clone(),
            self.model_event_sender.clone(),
            ctx,
        );
        let new_pane_id = pane_data.terminal_pane_id();
        if self
            .attach_child_pane_off_tree(Box::new(pane_data), ctx)
            .is_none()
        {
            report_error!(
                "attach_viewer_child_session: failed to attach pane",
                extra: { "child_conversation_id" => ?child_conversation_id }
            );
            return;
        }

        new_terminal_view.update(ctx, |terminal_view, ctx| {
            terminal_view.suppress_initial_conversation_details_panel_auto_open();
            terminal_view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(child_conversation),
                true,
                RestoreConversationEntryBehavior::PreserveAgentViewState,
                ctx,
            );
            terminal_view.enter_agent_view(
                None,
                Some(child_conversation_id),
                AgentViewEntryOrigin::SharedSessionSelection,
                ctx,
            );
            // Shared-session viewer is `is_cloud_mode=false`, so
            // `ambient_agent_view_model()` is typically `None`. Update
            // opportunistically; the network's `JoinedSuccessfully` is the
            // authoritative source for ambient agent state.
            if let Some(ambient_agent_view_model) = terminal_view
                .ambient_agent_view_model()
                .into_optional_handle()
                .cloned()
            {
                ambient_agent_view_model.update(ctx, |model, ctx| {
                    model.set_conversation_id(Some(child_conversation_id));
                    if let Some(task_id) = child_task_id {
                        model.enter_viewing_existing_session(task_id, ctx);
                    }
                });
            }
        });

        self.child_agent_panes
            .insert(child_conversation_id, new_pane_id.into());
        // If the discarded fallback was occupying a tree slot via temporary
        // replacement, re-swap so the user lands on the new pane.
        if let Some(anchor) = fallback_was_swapped_anchor {
            self.swap_active_pane_to_conversation(anchor, child_conversation_id, ctx);
        }
    }
}
