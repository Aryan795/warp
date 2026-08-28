pub mod docker_sandbox;
pub mod terminal_manager;
mod terminal_view_adaptor;

pub use terminal_manager::{TerminalManager, get_shell_starter};
#[cfg(feature = "tui")]
pub use terminal_manager::{TerminalManagerInit, TerminalSurfaceInit, TerminalSurfaceResult};
#[cfg(windows)]
pub use terminal_view_adaptor::shutdown_all_pty_event_loops;
#[cfg(all(feature = "local_tty", not(feature = "remote_tty")))]
pub(crate) use terminal_view_adaptor::{
    TerminalViewSurfaceConfig, create_terminal_view_surface, terminal_view_restored_blocks,
};
pub use warp_terminal::local_tty::*;

#[cfg(unix)]
pub fn run_terminal_server(args: &warp_cli::TerminalServerArgs) {
    warp_terminal::local_tty::server::run_terminal_server(
        args,
        crate::features::init_feature_flags,
        crate::terminal::platform::init,
    );
}

impl event_loop::ActiveTerminal for crate::terminal::TerminalModel {
    fn exit(&mut self, reason: crate::terminal::model::terminal_model::ExitReason) {
        crate::terminal::TerminalModel::exit(self, reason);
    }

    fn on_tmux_control_mode(&mut self, active: bool) {
        self.set_tmux_control_mode(active);
        #[cfg(not(feature = "remote_tty"))]
        if !active {
            crate::terminal::tmux::bridge::TmuxRuntime::global().clear_session();
        }
    }

    fn on_tmux_pane_output(
        &mut self,
        pane_id: &crate::terminal::tmux::parser::PaneId,
        bytes: &[u8],
    ) {
        #[cfg(not(feature = "remote_tty"))]
        crate::terminal::tmux::bridge::TmuxRuntime::global().deliver_output(pane_id, bytes);
        #[cfg(feature = "remote_tty")]
        {
            let _ = (pane_id, bytes);
        }
    }

    fn on_tmux_focus(&mut self, pane_id: &crate::terminal::tmux::parser::PaneId) {
        self.set_tmux_focused_pane(Some(pane_id.as_str().to_owned()));
    }

    fn on_tmux_layout(
        &mut self,
        window_id: &crate::terminal::tmux::parser::WindowId,
        layout: &str,
        visible_layout: Option<&str>,
        flags: Option<&str>,
    ) {
        self.push_tmux_event(
            crate::terminal::model::terminal_model::TmuxClientEvent::LayoutChange {
                window_id: window_id.as_str().to_owned(),
                layout: layout.to_owned(),
                visible_layout: visible_layout.map(str::to_owned),
                flags: flags.map(str::to_owned),
            },
        );
    }

    fn on_tmux_window_add(&mut self, window_id: &crate::terminal::tmux::parser::WindowId) {
        self.push_tmux_event(
            crate::terminal::model::terminal_model::TmuxClientEvent::WindowAdd {
                window_id: window_id.as_str().to_owned(),
            },
        );
    }

    fn on_tmux_window_close(&mut self, window_id: &crate::terminal::tmux::parser::WindowId) {
        self.push_tmux_event(
            crate::terminal::model::terminal_model::TmuxClientEvent::WindowClose {
                window_id: window_id.as_str().to_owned(),
            },
        );
    }

    fn on_tmux_window_renamed(
        &mut self,
        window_id: &crate::terminal::tmux::parser::WindowId,
        name: &str,
    ) {
        self.push_tmux_event(
            crate::terminal::model::terminal_model::TmuxClientEvent::WindowRenamed {
                window_id: window_id.as_str().to_owned(),
                name: name.to_owned(),
            },
        );
    }

    fn on_tmux_session_window_changed(
        &mut self,
        window_id: &crate::terminal::tmux::parser::WindowId,
    ) {
        self.push_tmux_event(
            crate::terminal::model::terminal_model::TmuxClientEvent::SessionWindowChanged {
                window_id: window_id.as_str().to_owned(),
            },
        );
    }

    fn on_tmux_command_end(&mut self, number: u64, error: bool, payload: &[String]) {
        self.push_tmux_event(
            crate::terminal::model::terminal_model::TmuxClientEvent::CommandEnd {
                number,
                error,
                payload: payload.to_vec(),
            },
        );
    }
}
