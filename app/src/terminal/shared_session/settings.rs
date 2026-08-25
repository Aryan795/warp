use std::time::Duration;

use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, Setting, SupportedPlatforms, SyncToCloud};

define_settings_group!(SharedSessionSettings, settings: [
    onboarding_block_shown: SessionSharingOnboardingBlockShown {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
    inactivity_period_before_ending_session: InactivityPeriodBeforeEndingSession {
        type: Duration,
        // After a total of 30 min of inactivity, we will end the session
        default: Duration::from_secs(1800),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "session_sharing.inactivity.end_session_after_secs",
        description: "How long a shared session can be inactive before it is automatically ended, in seconds.",
    },
    inactivity_period_before_warning: InactivityPeriodBeforeWarning {
        type: Duration,
        // After a total of 25 min of inactivity, we will show a warning modal
        default: Duration::from_secs(1500),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "session_sharing.inactivity.warning_after_secs",
        description: "How long to wait before warning that a shared session will end due to inactivity, in seconds",
    },
    inactivity_period_before_revoking_roles: InactivityPeriodBeforeRevokingRoles {
        type: Duration,
        // After a total of 10 min of inactivity, we will revoke all executor roles
        default: Duration::from_secs(600),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "session_sharing.inactivity.revoke_edit_access_after_secs",
        description: "Idle period before shared sessions are made read-only",
    },
    // Killswitch: when false, the sharer ignores viewer terminal size reports.
    viewer_driven_sizing_enabled: ViewerDrivenSizingEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
]);

/// A phase of the sharer inactivity ladder, in the order it can fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InactivityPhase {
    RevokeEditorRoles,
    ShowWarning,
    EndSession,
}

impl SharedSessionSettings {
    /// Returns time between showing the inactivity warning modal and ending the session.
    ///
    /// Uses `saturating_sub` as defense-in-depth: these three durations are cumulative
    /// time-since-last-activity (each setting's default doc comment says "after a total of
    /// N min"), which requires `revoke <= warn <= end` for a meaningful ladder, but nothing
    /// actively enforces that ordering outside of the settings UI's own clamping (see
    /// `app/src/settings_view/features_page.rs`). A hand-edited settings file or a cloud
    /// sync could still hand this an inconsistent triple; `saturating_sub` degrades that to
    /// an immediately-firing phase instead of a panic. Callers must only reach this once
    /// they've confirmed (via [`Self::next_inactivity_phase`] or
    /// [`Self::next_phase_after_revoke`]) that both the warning and end phases are enabled
    /// -- a zero `end` disables the warning phase entirely, so this is never a meaningful
    /// duration to compute in that case.
    pub fn inactivity_period_between_warning_and_ending_session(&self) -> Duration {
        self.inactivity_period_before_ending_session
            .value()
            .saturating_sub(*self.inactivity_period_before_warning.value())
    }

    /// Returns time between revoking roles and showing the inactivity warning modal.
    ///
    /// See [`Self::inactivity_period_between_warning_and_ending_session`] for why this
    /// uses `saturating_sub` and must only be called once the warning phase is confirmed
    /// enabled.
    pub fn inactivity_period_between_revoking_roles_and_warning(&self) -> Duration {
        self.inactivity_period_before_warning
            .value()
            .saturating_sub(*self.inactivity_period_before_revoking_roles.value())
    }

    /// Whether the warning phase is enabled: it needs both a non-zero warning duration of
    /// its own, and a non-zero end duration -- a countdown to an end that will never come
    /// would be misleading, so disabling the end phase disables the warning too.
    pub fn is_warning_phase_enabled(&self) -> bool {
        !self.inactivity_period_before_warning.value().is_zero()
            && !self
                .inactivity_period_before_ending_session
                .value()
                .is_zero()
    }

    /// Determines which phase of the inactivity ladder should be armed next, and how long
    /// from *now* (the point activity was last observed) it should fire after, skipping any
    /// disabled (zero-duration) phase. Returns `None` when every phase is disabled, meaning
    /// no idle timeout should be armed at all.
    pub fn next_inactivity_phase(&self) -> Option<(InactivityPhase, Duration)> {
        let revoke = *self.inactivity_period_before_revoking_roles.value();
        if !revoke.is_zero() {
            return Some((InactivityPhase::RevokeEditorRoles, revoke));
        }
        self.next_phase_after_revoke()
    }

    /// Determines which phase should be armed after the revoke phase has already happened
    /// (or was itself disabled), and how long from *that point* it should fire after.
    /// Returns `None` when both the warning and end phases are disabled, meaning the ladder
    /// should stop advancing (the session stays shared, permanently read-only if roles were
    /// revoked, until the sharer changes these settings or ends it explicitly).
    pub fn next_phase_after_revoke(&self) -> Option<(InactivityPhase, Duration)> {
        let revoke = *self.inactivity_period_before_revoking_roles.value();
        let end = *self.inactivity_period_before_ending_session.value();
        if self.is_warning_phase_enabled() {
            let warn = *self.inactivity_period_before_warning.value();
            return Some((InactivityPhase::ShowWarning, warn.saturating_sub(revoke)));
        }
        if !end.is_zero() {
            return Some((InactivityPhase::EndSession, end.saturating_sub(revoke)));
        }
        None
    }
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod settings_tests;
