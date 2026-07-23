//! Suggested automations shown in Settings → Automations.
//!
//! Each suggestion is a recipe: "Set up" opens a new tab with a Warp agent
//! conversation seeded with a recipe-specific prompt. The section renders
//! beneath the user's automations (including when the list is empty) and can
//! be collapsed; the collapse state persists via
//! `crate::settings::LocalAutomationsSettings`.

/// A suggested automation recipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestedAutomation {
    StaleBranchCleanup,
    PrBabysitter,
    MiniSoftwareFactory,
}

impl SuggestedAutomation {
    pub const ALL: [Self; 3] = [
        Self::StaleBranchCleanup,
        Self::PrBabysitter,
        Self::MiniSoftwareFactory,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::StaleBranchCleanup => "Stale branch cleanup",
            Self::PrBabysitter => "PR babysitter",
            Self::MiniSoftwareFactory => "Mini software factory",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::StaleBranchCleanup => {
                "Prune merged and abandoned branches in a repo you pick, on a schedule."
            }
            Self::PrBabysitter => {
                "Keep your open PRs moving: address review feedback and rebase stale branches."
            }
            Self::MiniSoftwareFactory => {
                "Agents triage new issues, implement on label, and review PRs via GitHub triggers."
            }
        }
    }

    /// Prompt seeded into an agent conversation ("Set up → Create with Warp
    /// Agent") or copied to the clipboard for another agent. Defers all
    /// mechanics to the create-automation skill; states only the recipe.
    pub fn prompt(self) -> &'static str {
        match self {
            Self::StaleBranchCleanup => {
                "Use the create-automation skill to set up a stale branch cleanup \
                 automation: on a schedule, in a repo I pick, find local branches that \
                 are merged or idle for 30+ days, delete only fully merged ones, and \
                 report anything skipped. It must never force-delete unmerged work or \
                 touch the current branch."
            }
            Self::PrBabysitter => {
                "Use the create-automation skill to set up a \"PR babysitter\" \
                 automation for a repo I pick: on each run, list my open PRs, address \
                 unresolved review feedback where the fix is clear, rebase branches that \
                 are behind their base, push the updates, and summarize what changed. It \
                 must never merge PRs or force-push over other people's commits."
            }
            Self::MiniSoftwareFactory => {
                "Use the create-automation skill to set up a mini software factory on a \
                 GitHub repository I pick: GitHub-triggered agents that triage new \
                 issues, implement issues when labeled ready, and review pull requests. \
                 Walk me through the requirements and check with me before any billable \
                 runs."
            }
        }
    }
}
