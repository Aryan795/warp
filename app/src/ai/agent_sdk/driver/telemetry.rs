use serde::Serialize;
use serde_json::{Value, json};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

/// Telemetry emitted at the `wait_for_events` action's execution boundary once
/// its warm wait episode has resolved. Carries the fields QUALITY-1759's
/// TECH.md calls for, so the hibernation rollout (server stamp vs. fallback
/// use, checkpoint success) can be tracked via a dashboard rather than log
/// search.
#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub(crate) enum WaitForEventsTelemetryEvent {
    EpisodeResolved(WaitForEventsEpisodeResolvedEvent),
}

/// How a `wait_for_events` warm wait episode ended.
///
/// Only [`Self::Timeout`] is reachable today: this event fires from the
/// driver's yield path, which only runs after the warm wait watchdog fires.
/// The path where a relevant event arrives first cancels the watchdog and
/// resumes the live execution without yielding the action, so it never
/// reaches this boundary.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WaitForEventsOutcome {
    Timeout,
}

/// Whether the final checkpoint upload succeeded before the run exited.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WaitForEventsCheckpointOutcome {
    Succeeded,
    Failed,
}

impl WaitForEventsCheckpointOutcome {
    pub(crate) fn from_succeeded(checkpoint_succeeded: bool) -> Self {
        if checkpoint_succeeded {
            Self::Succeeded
        } else {
            Self::Failed
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct WaitForEventsEpisodeResolvedEvent {
    /// Serialized as `null` (not omitted) when absent, so every emitted
    /// event carries the full fixed eight-field payload for schema-based
    /// dashboard queries.
    pub task_id: Option<String>,
    pub execution_id: String,
    pub server_idle_timeout_seconds: i32,
    pub used_fallback: bool,
    pub resolved_watchdog_seconds: u64,
    pub hibernate_on_first_timeout_enabled: bool,
    pub wait_outcome: WaitForEventsOutcome,
    pub checkpoint_outcome: WaitForEventsCheckpointOutcome,
}

impl TelemetryEvent for WaitForEventsTelemetryEvent {
    fn name(&self) -> &'static str {
        WaitForEventsTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            Self::EpisodeResolved(event) => Some(json!(event)),
        }
    }

    fn description(&self) -> &'static str {
        WaitForEventsTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        WaitForEventsTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        match self {
            Self::EpisodeResolved(_) => false,
        }
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for WaitForEventsTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::EpisodeResolved => "AmbientAgents.WaitForEvents.EpisodeResolved",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::EpisodeResolved => {
                "A wait_for_events warm wait episode resolved at the driver's action \
                 execution boundary, after the final checkpoint upload attempt. Contains \
                 no message or checkpoint content."
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            Self::EpisodeResolved => EnablementState::Always,
        }
    }
}

warp_core::register_telemetry_event!(WaitForEventsTelemetryEvent);

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;
