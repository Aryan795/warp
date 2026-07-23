//! Local automations: personal, user-scoped jobs defined as TOML files under
//! the user's `automations/` data directory (e.g. `~/.warp/automations/` on
//! stable).
//!
//! Supports defining automations on disk, listing them, **Run now**, and
//! in-app cron scheduling while Warp is running (Slice B). Schedules do not
//! fire when Warp is quit; catch-up within 6 hours runs once on wake, older
//! gaps are marked missed.
//!
//! Gated on `FeatureFlag::LocalAutomations`.

pub mod agent_modal;
pub mod list_view;
pub mod local_automation;
pub mod run_state;
pub mod schedule;
pub mod scheduler;
pub mod suggestions;

pub use list_view::LocalAutomationsView;
pub use local_automation::{LocalAutomation, LocalAutomationError, LocalAutomationRunner};
pub use scheduler::{LocalAutomationsScheduler, LocalAutomationsSchedulerEvent};
pub use suggestions::SuggestedAutomation;
use warp_core::paths::home_relative_path;

/// Prompt submitted to a fresh Warp agent conversation by the "New" modal's
/// "Use Warp Agent" option. Defers all mechanics to the create-automation
/// skill.
pub fn new_automation_agent_prompt() -> String {
    "Use the create-automation skill to create a new automation with me.".to_string()
}

/// Creation prompt for the "New" modal's "Copy agent prompt" option, aimed
/// at agents outside Warp (Claude Code, Codex, ...). Assumes the
/// create-automation skill is installed for that agent.
pub fn new_automation_external_prompt() -> String {
    "Use the create-automation skill to create a new automation with me.".to_string()
}

/// Prompt submitted to a fresh Warp agent conversation by the "Move to
/// cloud" modal's "Use Warp Agent" option. Defers all move-to-cloud
/// mechanics (Oz environment selection, schedule mapping, shell → agent
/// rewrites, billing caveats) to the create-automation skill.
pub fn promote_automation_agent_prompt(automation: &LocalAutomation) -> String {
    promote_automation_prompt(automation, "local automation")
}

/// Move-to-cloud prompt for the "Move to cloud" modal's "Copy agent prompt"
/// option, aimed at agents outside Warp (Claude Code, Codex, ...). Assumes
/// the create-automation skill is installed for that agent.
pub fn promote_automation_external_prompt(automation: &LocalAutomation) -> String {
    promote_automation_prompt(automation, "Warp local automation")
}

/// Shared body for the move-to-cloud prompts. Points the agent at the TOML
/// on disk when the source path is known; otherwise embeds the key fields so
/// the prompt is still self-contained.
fn promote_automation_prompt(automation: &LocalAutomation, noun: &str) -> String {
    let name = &automation.name;
    match &automation.source_path {
        Some(path) => format!(
            "Use the create-automation skill to promote my {noun} \"{name}\" to an Oz cloud \
             schedule. Read its config at {} for the existing name, schedule, and runner, and \
             walk me through picking or creating an Oz environment.",
            home_relative_path(path)
        ),
        None => {
            let runner = match &automation.runner {
                LocalAutomationRunner::WarpAgent { prompt } => format!("agent prompt: {prompt}"),
                LocalAutomationRunner::Shell { command } => format!("shell command: {command}"),
            };
            format!(
                "Use the create-automation skill to promote my {noun} \"{name}\" to an Oz cloud \
                 schedule. Its schedule is `{}` and it runs this {runner}. Walk me through \
                 picking or creating an Oz environment.",
                automation.schedule
            )
        }
    }
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
