use settings::{PrivatePreferences, PublicPreferences, Setting, SettingsManager};
use warp_core::features::FeatureFlag;
use warp_core::user_preferences::GetUserPreferences as _;
use warpui::{App, AppContext, SingletonEntity};
use warpui_extras::user_preferences;

use super::*;

fn init_test_app(ctx: &mut AppContext) {
    ctx.add_singleton_model(move |_| {
        PublicPreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
    });
    ctx.add_singleton_model(move |_| -> PrivatePreferences {
        PrivatePreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
    });
    ctx.add_singleton_model(|_| SettingsManager::default());
}

fn write_public(ctx: &AppContext, key: &str, duration: Duration) {
    // Any of the three (now-public) settings routes to the same PublicPreferences backend;
    // `preferences_for_setting` is the public API for reaching it from outside the
    // `settings` crate.
    InactivityPeriodBeforeRevokingRoles::preferences_for_setting(ctx)
        .write_value(key, serde_json::to_string(&duration).unwrap())
        .unwrap();
}

fn write_private(ctx: &AppContext, key: &str, duration: Duration) {
    ctx.private_user_preferences()
        .write_value(key, serde_json::to_string(&duration).unwrap())
        .unwrap();
}

// ---------------------------------------------------------------------------
// Legacy private -> public migration (review finding 1)
// ---------------------------------------------------------------------------

#[test]
fn legacy_private_value_survives_migration_even_when_settings_file_marker_already_set() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        app.update(init_test_app);

        // Simulate a pre-existing user for whom the general native->TOML migration already
        // ran and recorded its completion marker, before these three settings became public.
        app.update(|ctx| {
            ctx.private_user_preferences()
                .write_value("SettingsFileMigrationComplete", "true".to_owned())
                .unwrap();
            write_private(
                ctx,
                InactivityPeriodBeforeRevokingRoles::storage_key(),
                Duration::from_secs(900),
            );
        });

        app.update(|ctx| {
            SharedSessionSettings::register_and_enforce_inactivity_ordering(ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                *SharedSessionSettings::as_ref(ctx)
                    .inactivity_period_before_revoking_roles
                    .value(),
                Duration::from_secs(900),
                "legacy private-store value should survive the flip to a public setting"
            );
        });
    });
}

#[test]
fn migration_does_not_overwrite_already_set_public_value() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        app.update(init_test_app);

        app.update(|ctx| {
            // The user already has an explicit value in the new public location...
            write_public(
                ctx,
                InactivityPeriodBeforeRevokingRoles::storage_key(),
                Duration::from_secs(120),
            );
            // ...while a stale legacy private-store value also happens to exist.
            write_private(
                ctx,
                InactivityPeriodBeforeRevokingRoles::storage_key(),
                Duration::from_secs(999),
            );
        });

        app.update(|ctx| {
            SharedSessionSettings::register_and_enforce_inactivity_ordering(ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                *SharedSessionSettings::as_ref(ctx)
                    .inactivity_period_before_revoking_roles
                    .value(),
                Duration::from_secs(120),
                "migration must not clobber a value already explicitly set in the public location"
            );
        });
    });
}

#[test]
fn migration_is_one_time_via_its_own_marker() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        app.update(init_test_app);

        app.update(|ctx| {
            write_private(
                ctx,
                InactivityPeriodBeforeRevokingRoles::storage_key(),
                Duration::from_secs(900),
            );
        });

        // Register once, then run the migration explicitly (simulating a launch with a
        // pre-existing private-store value).
        app.update(|ctx| {
            SharedSessionSettings::register(ctx);
        });
        app.update(migrate_legacy_private_inactivity_settings);

        app.read(|ctx| {
            assert_eq!(
                ctx.private_user_preferences()
                    .read_value(LEGACY_INACTIVITY_SETTINGS_MIGRATED_KEY)
                    .unwrap()
                    .as_deref(),
                Some("true"),
                "migration should record its own completion marker"
            );
            assert_eq!(
                *SharedSessionSettings::as_ref(ctx)
                    .inactivity_period_before_revoking_roles
                    .value(),
                Duration::from_secs(900)
            );
        });

        // The user then explicitly clears the migrated value (e.g. removing it from their
        // settings file / resetting to default).
        app.update(|ctx| {
            SharedSessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .inactivity_period_before_revoking_roles
                    .clear_value(ctx)
                    .unwrap();
            });
        });

        // Running the migration again (simulating a second launch) must be a no-op: its own
        // marker is already set, so it must not re-copy the stale legacy value and clobber
        // the user's explicit reset.
        app.update(migrate_legacy_private_inactivity_settings);

        app.read(|ctx| {
            assert_eq!(
                *SharedSessionSettings::as_ref(ctx)
                    .inactivity_period_before_revoking_roles
                    .value(),
                InactivityPeriodBeforeRevokingRoles::default_value(),
                "migration must not re-run once its own marker is set"
            );
        });
    });
}

// ---------------------------------------------------------------------------
// Ordering enforcement at the authoritative boundary (review finding 2)
// ---------------------------------------------------------------------------

#[test]
fn register_corrects_out_of_order_values_from_storage() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        app.update(init_test_app);

        // Simulate a hand-edited settings file with revoke > warn.
        app.update(|ctx| {
            write_public(
                ctx,
                InactivityPeriodBeforeRevokingRoles::storage_key(),
                Duration::from_secs(1000),
            );
            write_public(
                ctx,
                InactivityPeriodBeforeWarning::storage_key(),
                Duration::from_secs(500),
            );
        });

        app.update(|ctx| {
            SharedSessionSettings::register_and_enforce_inactivity_ordering(ctx);
        });

        app.read(|ctx| {
            let settings = SharedSessionSettings::as_ref(ctx);
            let revoke = *settings.inactivity_period_before_revoking_roles.value();
            let warn = *settings.inactivity_period_before_warning.value();
            let end = *settings.inactivity_period_before_ending_session.value();
            assert!(
                revoke <= warn && warn <= end,
                "ordering must hold after loading an inconsistent file: \
                 revoke={revoke:?} warn={warn:?} end={end:?}"
            );
            assert_eq!(warn, revoke, "warn should be pulled up to revoke's value");
        });
    });
}

#[test]
fn cloud_sync_update_producing_bad_ordering_gets_corrected() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        app.update(init_test_app);

        app.update(|ctx| {
            SharedSessionSettings::register_and_enforce_inactivity_ordering(ctx);
        });

        // A cloud-synced update sets `end` below the current `warn` (defaults: revoke=600s,
        // warn=1500s, end=1800s).
        app.update(|ctx| {
            SharedSessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .inactivity_period_before_ending_session
                    .set_value_from_cloud_sync(Duration::from_secs(100), ctx)
                    .unwrap();
            });
        });

        app.read(|ctx| {
            let settings = SharedSessionSettings::as_ref(ctx);
            let revoke = *settings.inactivity_period_before_revoking_roles.value();
            let warn = *settings.inactivity_period_before_warning.value();
            let end = *settings.inactivity_period_before_ending_session.value();
            assert!(
                revoke <= warn && warn <= end,
                "ordering must hold after a bad cloud sync update: \
                 revoke={revoke:?} warn={warn:?} end={end:?}"
            );
            assert_eq!(
                end, warn,
                "end should be pulled back up to warn's value rather than left below it"
            );
        });
    });
}

#[test]
fn derived_intervals_never_panic_on_out_of_order_values() {
    // Directly construct an inconsistent group (bypassing the ordering enforcement entirely)
    // to prove the derived-interval helpers are defensive regardless of how a bad ordering
    // arises, not just against the paths this change actively guards.
    let settings = SharedSessionSettings {
        onboarding_block_shown: SessionSharingOnboardingBlockShown::new(None),
        inactivity_period_before_ending_session: InactivityPeriodBeforeEndingSession::new(Some(
            Duration::from_secs(10),
        )),
        inactivity_period_before_warning: InactivityPeriodBeforeWarning::new(Some(
            Duration::from_secs(500),
        )),
        inactivity_period_before_revoking_roles: InactivityPeriodBeforeRevokingRoles::new(Some(
            Duration::from_secs(600),
        )),
        viewer_driven_sizing_enabled: ViewerDrivenSizingEnabled::new(None),
    };

    assert_eq!(
        settings.inactivity_period_between_warning_and_ending_session(),
        Duration::ZERO
    );
    assert_eq!(
        settings.inactivity_period_between_revoking_roles_and_warning(),
        Duration::ZERO
    );
}
