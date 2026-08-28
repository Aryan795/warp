use warpui::{SingletonEntity, ViewContext};

use super::TerminalView;
use crate::features::FeatureFlag;
use crate::pane_group::TerminalViewResources;
use crate::server::server_api::ServerApiProvider;
use crate::terminal::tmux::protocol::resolve_tmux_binary;
use crate::terminal::tmux::terminal_manager::TmuxTerminalManager;
use crate::view_components::ToastFlavor;

impl TerminalView {
    pub(crate) fn create_and_push_tmux_workspace(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::TmuxControlPrototype.is_enabled() {
            log::warn!("tmux control prototype feature flag is disabled");
            return;
        }
        if resolve_tmux_binary().is_none() {
            self.show_persistent_toast(
                "tmux was not found on PATH".to_owned(),
                ToastFlavor::Error,
                ctx,
            );
            return;
        }

        let Some(pane_stack) = self
            .pane_stack
            .as_ref()
            .and_then(|stack| stack.upgrade(ctx))
        else {
            log::warn!("Pane stack not available, cannot create tmux workspace");
            return;
        };

        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: ServerApiProvider::as_ref(ctx).get(),
            model_event_sender: self.model_event_sender.clone(),
        };
        let pane_configuration = self.pane_configuration().clone();
        let init = TmuxTerminalManager::create_model(
            resources,
            self.size_info().pane_size_px(),
            self.model_event_sender.clone(),
            ctx.window_id(),
            ctx,
        );
        init.view.update(ctx, |view, _| {
            view.set_pane_configuration(pane_configuration);
        });
        pane_stack.update(ctx, |stack, ctx| {
            stack.push(init.manager, init.view, ctx);
        });
        ctx.notify();
    }
}
