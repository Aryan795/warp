use settings::{SupportedPlatforms, SyncToCloud};
use warp_core::define_settings_group;

// UI state rather than a user-facing preference: remembers whether the
// "Suggested" section on Settings → Automations is collapsed, so the choice
// survives restarts. Local-only; not synced across devices.
define_settings_group!(LocalAutomationsSettings, settings: [
    suggestions_collapsed: LocalAutomationsSuggestionsCollapsed {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
]);
