use std::path::PathBuf;

use warpui::{SingletonEntity, ViewContext};

use super::{Direction, PaneGroup, PaneId, TerminalPane, TerminalViewResources};
use crate::terminal::view::dev_container::operation::DevContainerBuildOperation;
use crate::terminal::view::dev_container::registry::{
    DevContainerBuildClaim, DevContainerBuildKey, DevContainerBuildLocator,
    DevContainerBuildRegistry,
};
use crate::view_components::DismissibleToast;
use crate::workspace::ToastStack;

impl PaneGroup {
    pub(crate) fn start_dev_container_build(
        &mut self,
        originating_pane_id: PaneId,
        workspace_folder: PathBuf,
        config_file: PathBuf,
        ctx: &mut ViewContext<Self>,
    ) {
        let key = DevContainerBuildKey {
            workspace_folder,
            config_file,
        };

        let existing_live = DevContainerBuildRegistry::handle(ctx).read(ctx, |registry, ctx| {
            registry
                .get(&key)
                .and_then(|entry| entry.locator.is_live(ctx).then(|| entry.locator.pane_id))
        });
        if let Some(pane_id) = existing_live {
            self.focus_pane_by_id(pane_id, ctx);
            return;
        }

        let pane_id =
            self.add_loading_conversation_pane(Direction::Right, Some(originating_pane_id), ctx);
        if !self.has_pane(pane_id) {
            let window_id = ctx.window_id();
            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                toast_stack.add_ephemeral_toast(
                    DismissibleToast::error("Couldn't open a Dev Container build pane.".to_owned()),
                    window_id,
                    ctx,
                );
            });
            return;
        }

        let Some(terminal_view) = self.terminal_view_from_pane_id(pane_id, ctx) else {
            return;
        };

        let operation = ctx.add_model(|_| DevContainerBuildOperation::new(key.clone()));
        let operation_id = operation.read(ctx, |operation, _| operation.operation_id());
        let locator = DevContainerBuildLocator {
            window_id: ctx.window_id(),
            pane_group: ctx.handle(),
            pane_id,
        };
        let claim = DevContainerBuildRegistry::handle(ctx).update(ctx, |registry, ctx| {
            registry.claim(key, locator, operation_id, ctx)
        });
        match claim {
            DevContainerBuildClaim::Existing { locator, .. } => {
                self.close_pane(pane_id, ctx);
                self.focus_pane_by_id(locator.pane_id, ctx);
            }
            DevContainerBuildClaim::Claimed { .. } => {
                terminal_view.update(ctx, |view, ctx| {
                    view.bind_dev_container_build(operation, ctx);
                });
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn replace_dev_container_build_pane(
        &mut self,
        build_pane_id: PaneId,
        workspace_folder: PathBuf,
        docker_path: PathBuf,
        container_id: String,
        remote_user: Option<String>,
        remote_workspace_folder: String,
        sandbox_id: String,
        session_id: warp_core::SessionId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(build_view) = self.terminal_view_from_pane_id(build_pane_id, ctx) else {
            return;
        };
        let pane_configuration = build_view.read(ctx, |view, _| view.pane_configuration().clone());
        let model_event_sender = build_view.read(ctx, |view, _| view.model_event_sender());
        let size = build_view.read(ctx, |view, _| view.size_info().pane_size_px());
        let key = build_view.read(ctx, |view, ctx| view.dev_container_build_key(ctx));

        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };
        let uuid = uuid::Uuid::new_v4();
        let (terminal_view, terminal_manager) =
            crate::terminal::view::dev_container::create_dev_container_view(
                resources,
                size,
                model_event_sender,
                workspace_folder,
                docker_path,
                container_id,
                remote_user,
                remote_workspace_folder,
                sandbox_id,
                session_id,
                ctx,
            );
        terminal_view.update(ctx, |view, _| {
            view.set_pane_configuration(pane_configuration);
        });
        let pane_data = TerminalPane::new(
            uuid.as_bytes().to_vec(),
            terminal_manager,
            terminal_view,
            self.model_event_sender.clone(),
            ctx,
        );
        let _ = self.replace_pane(build_pane_id, pane_data, false, ctx);
        if let Some(key) = key {
            DevContainerBuildRegistry::handle(ctx).update(ctx, |registry, _| {
                registry.remove(&key);
            });
        }
    }
}
