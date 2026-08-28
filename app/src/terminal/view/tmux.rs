use warpui::ViewContext;

use super::TerminalView;
use crate::features::FeatureFlag;
use crate::terminal::tmux::transport::{TmuxCommandError, tmux_cc_shell_command};

/// Stable tmux session name used so SSH reconnect can attach with `-A`.
const IN_PLACE_TMUX_SESSION: &str = "warp";

impl TerminalView {
    pub(crate) fn create_and_push_tmux_workspace(
        &mut self,
        args: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::TmuxControlPrototype.is_enabled() {
            log::warn!("tmux control prototype feature flag is disabled");
            return;
        }

        let size = self.size_info();
        let command = match tmux_cc_shell_command(
            args,
            Some(IN_PLACE_TMUX_SESSION),
            size.columns(),
            size.rows(),
        ) {
            Ok(command) => command,
            Err(TmuxCommandError::IsolatedSocketOverride) => {
                log::warn!(
                    "/tmux refuses -L/-S; managed sessions use the dedicated Warp tmux server"
                );
                return;
            }
        };
        self.write_to_pty(command.into_bytes(), ctx);
        ctx.notify();
    }
}
