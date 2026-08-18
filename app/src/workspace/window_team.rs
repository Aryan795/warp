use warpui::{Entity, ModelContext, SingletonEntity};

use crate::server::ids::ServerId;
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};

/// Events emitted by [`WindowTeam`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowTeamEvent {
    /// The selected team UID changed, or the selected team's metadata may have changed.
    /// Payload-free: consumers re-resolve the current UID and metadata via
    /// [`WindowTeam::uid`] and [`UserWorkspaces`] rather than reading it off the event.
    Changed,
}

/// Tracks which team (if any) a window's [`crate::workspace::Workspace`] is currently scoped
/// to. Owned by `Workspace` and created alongside it, so every window gets its own model
/// rather than sharing one mutable instance.
///
/// Reconciles the selection against [`UserWorkspaces`] whenever team membership or metadata
/// changes:
/// - With no teams, the selection is `None`.
/// - With exactly one team, the selection is always that team.
/// - With multiple teams, a still-valid selection is preserved; otherwise it falls back to
///   [`UserWorkspaces::default_team_uid`].
pub struct WindowTeam {
    team_uid: Option<ServerId>,
}

impl WindowTeam {
    /// Creates a new model seeded with `initial_team_uid`, then immediately reconciles it
    /// against the current team membership and subscribes to further changes.
    pub fn new(initial_team_uid: Option<ServerId>, ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _, event, ctx| {
            if matches!(event, UserWorkspacesEvent::TeamsChanged) {
                me.reconcile(ctx);
            }
        });

        let mut window_team = Self {
            team_uid: initial_team_uid,
        };
        window_team.team_uid = window_team.reconciled_uid(UserWorkspaces::as_ref(ctx));
        window_team
    }

    /// The currently selected team UID, or `None` when the window is scoped to the user's
    /// personal space.
    pub fn uid(&self) -> Option<ServerId> {
        self.team_uid
    }

    /// Reconciles the selection against the latest team membership, then emits
    /// [`WindowTeamEvent::Changed`] unconditionally: the UID itself may not have moved, but the
    /// selected team's metadata may have, and callers cannot cheaply tell the two apart.
    fn reconcile(&mut self, ctx: &mut ModelContext<Self>) {
        self.team_uid = self.reconciled_uid(UserWorkspaces::as_ref(ctx));
        ctx.emit(WindowTeamEvent::Changed);
        ctx.notify();
    }

    fn reconciled_uid(&self, user_workspaces: &UserWorkspaces) -> Option<ServerId> {
        if let Some(sole_team_uid) = user_workspaces.sole_team_uid() {
            Some(sole_team_uid)
        } else if !user_workspaces.has_teams() {
            None
        } else {
            self.team_uid
                .filter(|uid| user_workspaces.team_from_uid(*uid).is_some())
                .or_else(|| user_workspaces.default_team_uid())
        }
    }
}

impl Entity for WindowTeam {
    type Event = WindowTeamEvent;
}

#[cfg(test)]
#[path = "window_team_tests.rs"]
mod tests;
