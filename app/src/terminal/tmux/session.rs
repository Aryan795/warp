use std::collections::HashSet;

use super::parser::PaneId;

/// Tracks tmux pane identities independently of how the `-CC` byte stream is transported.
#[derive(Debug, Default, Clone)]
pub struct PaneRegistry {
    panes: HashSet<PaneId>,
    focused: Option<PaneId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosePlan {
    KillPane,
    TearDownSession,
    DetachClient,
    UnknownPane,
}

/// Network or PTY loss detaches the control client only. Explicit user close of the last
/// pane may tear down the tmux session; transport EOF must not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlClientLoss {
    TransportEof,
    ExplicitClose,
}

impl ControlClientLoss {
    pub fn close_plan(self, last_pane: bool) -> ClosePlan {
        match self {
            Self::TransportEof => ClosePlan::DetachClient,
            Self::ExplicitClose if last_pane => ClosePlan::TearDownSession,
            Self::ExplicitClose => ClosePlan::KillPane,
        }
    }
}

impl PaneRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, pane_id: PaneId) {
        if self.focused.is_none() {
            self.focused = Some(pane_id.clone());
        }
        self.panes.insert(pane_id);
    }

    pub fn contains(&self, pane_id: &PaneId) -> bool {
        self.panes.contains(pane_id)
    }

    pub fn len(&self) -> usize {
        self.panes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.panes.is_empty()
    }

    pub fn focused(&self) -> Option<&PaneId> {
        self.focused.as_ref()
    }

    pub fn focus(&mut self, pane_id: &PaneId) -> bool {
        if !self.panes.contains(pane_id) {
            return false;
        }
        self.focused = Some(pane_id.clone());
        true
    }

    pub fn should_deliver_output(&self, pane_id: &PaneId) -> bool {
        self.panes.contains(pane_id)
    }

    pub fn close_plan(&self, pane_id: &PaneId) -> ClosePlan {
        if !self.panes.contains(pane_id) {
            return ClosePlan::UnknownPane;
        }
        if self.panes.len() <= 1 {
            ClosePlan::TearDownSession
        } else {
            ClosePlan::KillPane
        }
    }

    pub fn unregister(&mut self, pane_id: &PaneId) -> ClosePlan {
        let plan = self.close_plan(pane_id);
        if matches!(plan, ClosePlan::UnknownPane) {
            return plan;
        }
        self.panes.remove(pane_id);
        if self.focused.as_ref() == Some(pane_id) {
            self.focused = self.panes.iter().next().cloned();
        }
        plan
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
