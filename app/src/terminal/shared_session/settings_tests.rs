use super::*;

// ---------------------------------------------------------------------------
// Zero-disables-a-phase matrix (APP-5313 follow-up)
// ---------------------------------------------------------------------------

/// Builds a `SharedSessionSettings` group directly with the given (revoke, warn, end)
/// durations, bypassing storage/registration entirely so the ladder-gating matrix can be
/// tested as pure logic.
fn settings_with(revoke: Duration, warn: Duration, end: Duration) -> SharedSessionSettings {
    SharedSessionSettings {
        onboarding_block_shown: SessionSharingOnboardingBlockShown::new(None),
        inactivity_period_before_ending_session: InactivityPeriodBeforeEndingSession::new(Some(
            end,
        )),
        inactivity_period_before_warning: InactivityPeriodBeforeWarning::new(Some(warn)),
        inactivity_period_before_revoking_roles: InactivityPeriodBeforeRevokingRoles::new(Some(
            revoke,
        )),
        viewer_driven_sizing_enabled: ViewerDrivenSizingEnabled::new(None),
    }
}

const SECS_10: Duration = Duration::from_secs(10);
const SECS_25: Duration = Duration::from_secs(25);
const SECS_30: Duration = Duration::from_secs(30);

#[test]
fn all_zero_arms_nothing() {
    let settings = settings_with(Duration::ZERO, Duration::ZERO, Duration::ZERO);
    assert_eq!(settings.next_inactivity_phase(), None);
}

#[test]
fn full_ladder_unaffected_when_nothing_is_zero() {
    let settings = settings_with(SECS_10, SECS_25, SECS_30);
    assert_eq!(
        settings.next_inactivity_phase(),
        Some((InactivityPhase::RevokeEditorRoles, SECS_10))
    );
    assert_eq!(
        settings.next_phase_after_revoke(),
        Some((InactivityPhase::ShowWarning, SECS_25 - SECS_10))
    );
}

#[test]
fn revoke_disabled_jumps_straight_to_warning() {
    let settings = settings_with(Duration::ZERO, SECS_25, SECS_30);
    assert_eq!(
        settings.next_inactivity_phase(),
        Some((InactivityPhase::ShowWarning, SECS_25)),
        "with revoke off, the first armed phase should be the full warning duration, not an \
         offset from a skipped revoke"
    );
}

#[test]
fn revoke_and_warning_disabled_jumps_straight_to_end() {
    let settings = settings_with(Duration::ZERO, Duration::ZERO, SECS_30);
    assert_eq!(
        settings.next_inactivity_phase(),
        Some((InactivityPhase::EndSession, SECS_30))
    );
}

#[test]
fn end_disabled_folds_the_warning_phase_off_too() {
    // Warn has a non-zero value of its own, but end=0 means there's nothing to warn about.
    let settings = settings_with(SECS_10, SECS_25, Duration::ZERO);
    assert!(!settings.is_warning_phase_enabled());
    assert_eq!(
        settings.next_phase_after_revoke(),
        None,
        "warning is disabled (end=0) and end is disabled, so nothing should arm after revoke"
    );
}

#[test]
fn revoke_only_enabled_stays_read_only_indefinitely() {
    // revoke on, warning and end both off: after revoking, nothing further should arm.
    let settings = settings_with(SECS_10, Duration::ZERO, Duration::ZERO);
    assert_eq!(
        settings.next_inactivity_phase(),
        Some((InactivityPhase::RevokeEditorRoles, SECS_10))
    );
    assert_eq!(settings.next_phase_after_revoke(), None);
}

#[test]
fn revoke_enabled_with_only_end_enabled_skips_the_warning() {
    let settings = settings_with(SECS_10, Duration::ZERO, SECS_30);
    assert_eq!(
        settings.next_phase_after_revoke(),
        Some((InactivityPhase::EndSession, SECS_30 - SECS_10)),
        "warning is disabled by its own zero value, so end should arm directly after revoke"
    );
}

#[test]
fn derived_intervals_never_panic_on_out_of_order_values() {
    // Directly construct an inconsistent group to prove the derived-interval helpers are
    // defensive regardless of how a bad ordering arises (e.g. a hand-edited settings file),
    // since nothing enforces the ordering outside of the settings UI's own clamping.
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
