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
        description: "How long a shared session can be inactive before you're warned it's about to end, in seconds.",
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
        description: "How long a shared session can be inactive before edit access is automatically revoked from everyone you're sharing with, in seconds.",
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

impl SharedSessionSettings {
    /// Returns time between showing the inactivity warning modal and ending the session.
    pub fn inactivity_period_between_warning_and_ending_session(&self) -> Duration {
        *self.inactivity_period_before_ending_session.value()
            - *self.inactivity_period_before_warning.value()
    }

    /// Returns time between revoking roles and showing the inactivity warning modal.
    pub fn inactivity_period_between_revoking_roles_and_warning(&self) -> Duration {
        *self.inactivity_period_before_warning.value()
            - *self.inactivity_period_before_revoking_roles.value()
    }
}
