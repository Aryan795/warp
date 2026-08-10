//! Builders for a shared-session **viewer** pane.
//!
//! A viewer pane is only interesting when its `TerminalView`, its `Network`, and the
//! `TerminalManager` subscriptions between them are all wired together, because that is the path
//! a submitted prompt actually travels. Constructing that by hand is verbose enough that tests
//! tend to poke internal state instead; these builders exist so they don't have to.

use std::sync::Arc;

use parking_lot::FairMutex;
use session_sharing_protocol::common::AgentPromptRequest;
use session_sharing_protocol::viewer::{DownstreamMessage, UpstreamMessage};
use warpui::{App, ModelHandle, SingletonEntity, ViewHandle};

use super::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::context_chips::prompt_type::PromptType;
use crate::settings::WarpPromptSeparator;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::session::SessionId as TerminalSessionId;
use crate::terminal::shared_session::SharedSessionStatus;
use crate::terminal::shared_session::shared_handlers::RemoteUpdateGuard;
use crate::terminal::shared_session::viewer::TerminalManager;
use crate::terminal::shared_session::viewer::network::{Network, Stage};
use crate::terminal::{TerminalModel, TerminalView};
use crate::workspace::ToastStack;

/// What the viewer is allowed to do in the shared session. Only an executor may submit prompts,
/// so the reader variant exists to assert that ineligible viewers are turned away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerRole {
    Executor,
    Reader,
}

impl ViewerRole {
    fn status(self) -> SharedSessionStatus {
        match self {
            ViewerRole::Executor => SharedSessionStatus::executor(),
            ViewerRole::Reader => SharedSessionStatus::reader(),
        }
    }
}

/// A viewer pane with its network attached and the manager subscriptions installed.
pub struct ViewerPane {
    pub view: ViewHandle<TerminalView>,
    pub conversation_id: AIConversationId,
    pub network: ModelHandle<Network>,
    /// The slot the manager reads the live network from. Swapping it models the network
    /// replacement that `attach_execution_session` performs on a fatal disconnect.
    pub current_network: Arc<FairMutex<Option<ModelHandle<Network>>>>,
    pub model: Arc<FairMutex<TerminalModel>>,
}

impl ViewerPane {
    /// Replaces the live network with `network`, as a fatal disconnect followed by a new
    /// execution session would. Events from the previous network must then be ignored.
    pub fn set_current_network(&self, network: Option<ModelHandle<Network>>) {
        *self.current_network.lock() = network;
    }
}

/// Builds an executor viewer pane whose network is in `stage`.
pub fn viewer_pane(app: &mut App, stage: Stage) -> ViewerPane {
    viewer_pane_with_role(app, stage, ViewerRole::Executor)
}

/// Builds a viewer pane in `stage` with an explicit `role`.
pub fn viewer_pane_with_role(app: &mut App, stage: Stage, role: ViewerRole) -> ViewerPane {
    initialize_app_for_terminal_view(app);
    app.add_singleton_model(|_| ToastStack);

    let view = add_window_with_terminal(app, None);
    let terminal_view_id = view.id();
    let model = view.read(app, |view, _| view.model.clone());
    {
        let mut model = model.lock();
        model.block_list_mut().set_bootstrapped();
        model
            .block_list_mut()
            .active_block_for_test()
            .set_session_id(TerminalSessionId::from(0));
        model.set_shared_session_status(role.status());
    }

    // Entering agent view is what makes a conversation *selected*, which is how the submission
    // path resolves the queue that owns a fallback row.
    let conversation_id = view.update(app, |view, ctx| {
        view.agent_view_controller().update(ctx, |controller, ctx| {
            controller
                .try_enter_agent_view(
                    None,
                    AgentViewEntryOrigin::Input {
                        was_prompt_autodetected: false,
                    },
                    ctx,
                )
                .expect("the pane can enter agent view")
        })
    });
    BlocklistAIHistoryModel::handle(app).update(app, |history, ctx| {
        history.set_active_conversation_id(conversation_id, terminal_view_id, ctx);
    });

    let network = attach_network(app, &view, stage);
    let current_network = Arc::new(FairMutex::new(Some(network.clone())));
    app.update(|ctx| {
        TerminalManager::handle_view_events(
            current_network.clone(),
            &view,
            model.clone(),
            RemoteUpdateGuard::new(),
            ctx,
        );
    });
    subscribe_network_events(app, &view, &model, &current_network, &network);

    ViewerPane {
        view,
        conversation_id,
        network,
        current_network,
        model,
    }
}

/// Installs the manager's inbound subscription for `network`, so a message injected into it
/// reaches the view the same way a real server message would.
///
/// Called for the pane's initial network, and again by the test for a replacement network so a
/// stale event from the old one can be shown to be ignored.
pub fn subscribe_network_events(
    app: &mut App,
    view: &ViewHandle<TerminalView>,
    model: &Arc<FairMutex<TerminalModel>>,
    current_network: &Arc<FairMutex<Option<ModelHandle<Network>>>>,
    network: &ModelHandle<Network>,
) {
    let prompt_type =
        app.add_model(|_| PromptType::new_static(vec![], false, WarpPromptSeparator::None));
    app.update(|ctx| {
        TerminalManager::handle_network_events(
            network,
            view,
            model.clone(),
            current_network.clone(),
            prompt_type,
            RemoteUpdateGuard::new(),
            Arc::new(FairMutex::new(None)),
            /* enable_orchestration_polling */ false,
            ctx,
        );
    });
}

/// Builds an additional `Network` for `view` without installing it as the live one. Used to model
/// the replacement session created after a fatal disconnect.
pub fn attach_network(
    app: &mut App,
    view: &ViewHandle<TerminalView>,
    stage: Stage,
) -> ModelHandle<Network> {
    let model = view.read(app, |view, _| view.model.clone());
    let channel_event_proxy = ChannelEventListener::new_for_test();
    let (_write_to_pty_tx, write_to_pty_rx) = async_channel::unbounded();
    let network = app.add_model(|ctx| {
        Network::new_for_test(
            channel_event_proxy,
            view.downgrade(),
            model,
            write_to_pty_rx,
            RemoteUpdateGuard::new(),
            ctx,
        )
    });
    network.update(app, |network, _| {
        network.stage = stage;
    });
    network
}

/// A network stage midway through a reconnect: the pane still reports itself an active viewer,
/// but nothing can actually be sent.
pub fn reconnecting_stage() -> Stage {
    let (abort_handle, _registration) = futures_util::stream::AbortHandle::new_pair();
    Stage::Reconnecting { abort_handle }
}

/// Types `prompt` and submits it through the real routing path, then lets the resulting events
/// propagate. The submission crosses `Input` -> `TerminalView` -> `TerminalManager`, and each hop
/// is delivered on an effect flush.
pub fn submit_viewer_prompt(app: &mut App, view: &ViewHandle<TerminalView>, prompt: &str) {
    let input = view.read(app, |view, _| view.input().clone());
    input.update(app, |input, ctx| {
        input.replace_buffer_content(prompt, ctx);
    });
    input.update(app, |input, ctx| {
        input.maybe_route_ai_query_to_remote_target(ctx);
    });
    flush(app);
}

/// Drives a server message through the real inbound path on `network`.
pub fn inject_downstream(
    app: &mut App,
    network: &ModelHandle<Network>,
    message: DownstreamMessage,
) {
    network.update(app, |network, ctx| {
        network.inject_downstream_message_for_test(message, ctx);
    });
    flush(app);
}

/// Runs pending effects so queued emissions are delivered.
pub fn flush(app: &mut App) {
    app.update(|_| ());
    app.update(|_| ());
}

/// Every agent prompt that reached `network`'s outbound channel, draining it. The channel also
/// carries CRDT input updates for the same submission, so prompts have to be picked out rather
/// than assumed to be first.
pub fn drain_agent_prompts(app: &App, network: &ModelHandle<Network>) -> Vec<AgentPromptRequest> {
    let ws_proxy_rx = network.read(app, |network, _| network.ws_proxy_rx.clone());
    let mut requests = Vec::new();
    while let Ok(message) = ws_proxy_rx.try_recv() {
        if let UpstreamMessage::SendAgentPrompt(request) = message {
            requests.push(request);
        }
    }
    requests
}

/// The single agent prompt that reached `network`, asserting there is exactly one.
pub fn sent_agent_prompt(app: &App, network: &ModelHandle<Network>) -> AgentPromptRequest {
    let mut requests = drain_agent_prompts(app, network);
    assert_eq!(
        requests.len(),
        1,
        "expected exactly one agent prompt to reach the network"
    );
    requests.remove(0)
}
