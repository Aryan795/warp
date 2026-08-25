pub mod inactivity_modal;
use async_channel::Sender;
use inactivity_modal::InactivityModal;
use warpui::r#async::SpawnedFutureHandle;
use warpui::elements::MouseStateHandle;
use warpui::{ViewContext, ViewHandle};

use crate::terminal::TerminalView;
use crate::terminal::shared_session::settings::InactivityLadderSnapshot;

pub struct Sharer {
    pub(super) activity_tx: Sender<()>,
    pub(super) revoke_all_mouse_state_handle: MouseStateHandle,
    pub(super) inactivity_timer_abort_handle: Option<SpawnedFutureHandle>,
    pub(super) is_inactivity_warning_modal_open: bool,
    pub(super) inactivity_modal: ViewHandle<InactivityModal>,
    /// The inactivity durations in effect for the idle period currently being timed, if
    /// any -- see [`InactivityLadderSnapshot`]. Captured fresh by
    /// `TerminalView::reset_sharer_inactivity_timer` each time a new idle period begins,
    /// and consulted (rather than the live settings) by every later phase transition within
    /// that same period.
    pub(super) inactivity_snapshot: Option<InactivityLadderSnapshot>,
}

impl Sharer {
    pub(super) fn new(activity_tx: Sender<()>, ctx: &mut ViewContext<TerminalView>) -> Self {
        let inactivity_modal = ctx.add_view(InactivityModal::new);
        ctx.subscribe_to_view(&inactivity_modal, |me, _, event, ctx| {
            me.handle_inactivity_modal_event(event, ctx)
        });

        Self {
            activity_tx,
            revoke_all_mouse_state_handle: Default::default(),
            inactivity_timer_abort_handle: None,
            is_inactivity_warning_modal_open: false,
            inactivity_modal,
            inactivity_snapshot: None,
        }
    }

    pub fn activity_tx(&self) -> &Sender<()> {
        &self.activity_tx
    }

    pub fn is_inactivity_warning_modal_open(&self) -> bool {
        self.is_inactivity_warning_modal_open
    }

    /// Opens inactivity warning modal and resets the timer, using the snapshot captured for
    /// the current idle period (see [`InactivityLadderSnapshot`]) rather than live settings,
    /// so this stays consistent with whichever durations the rest of this idle period's
    /// ladder is using.
    pub fn open_inactivity_warning_modal(&mut self, ctx: &mut ViewContext<TerminalView>) {
        let duration = self
            .inactivity_snapshot
            .map(|snapshot| snapshot.period_between_warning_and_ending_session())
            .unwrap_or_default();
        self.is_inactivity_warning_modal_open = true;
        self.inactivity_modal.update(ctx, |modal, ctx| {
            modal.reset_timer(duration, ctx);
        });
        ctx.focus(&self.inactivity_modal);
    }

    /// Closes the inactivity warning modal, including stopping its internal countdown --
    /// not just the flag that gates whether it renders. Without this, a countdown already
    /// in flight keeps ticking (and can still fire `TimedOut`) even once the modal is no
    /// longer shown. Safe to call even when the modal's own body already stopped its
    /// countdown itself (e.g. via a button click); stopping an already-stopped countdown
    /// is a no-op.
    pub fn close_inactivity_warning_modal(&mut self, ctx: &mut ViewContext<TerminalView>) {
        self.is_inactivity_warning_modal_open = false;
        self.inactivity_modal.update(ctx, |modal, ctx| {
            modal.stop_countdown(ctx);
        });
    }

    pub fn inactivity_modal(&self) -> &ViewHandle<InactivityModal> {
        &self.inactivity_modal
    }

    pub fn revoke_all_mouse_state_handle(&self) -> &MouseStateHandle {
        &self.revoke_all_mouse_state_handle
    }
}
