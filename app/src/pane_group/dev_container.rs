use std::path::PathBuf;

use warpui::{SingletonEntity, ViewContext};

use super::{Direction, PaneGroup, PaneId};
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
}
