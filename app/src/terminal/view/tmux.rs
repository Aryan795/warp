use warpui::ViewContext;

use super::TerminalView;
use crate::features::FeatureFlag;
use crate::terminal::tmux::transport::in_place_tmux_cc_command;

/// Stable tmux session name used so SSH reconnect can attach with `-A`.
const IN_PLACE_TMUX_SESSION: &str = "warp";

impl TerminalView {
    pub(crate) fn create_and_push_tmux_workspace(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::TmuxControlPrototype.is_enabled() {
            log::warn!("tmux control prototype feature flag is disabled");
            return;
        }

        let size = self.size_info();
        let command = in_place_tmux_cc_command(IN_PLACE_TMUX_SESSION, size.columns(), size.rows());
        self.write_to_pty(command.into_bytes(), ctx);
        ctx.notify();
    }
}
