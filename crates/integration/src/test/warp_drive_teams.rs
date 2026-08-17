//! Integration coverage for the Warp Drive sidebar as seen by a user with no team of their
//! own.
//!
//! The sidebar is the second surface the create-team gate applies to, and what it offers
//! depends on model state the client cannot reach on its own: whether the user's workspace
//! has native workspaces enabled, and whether the workspace has teams open to join. Seeding
//! those permutations and asserting on the frame that came out is the only way to pin the
//! sidebar down end to end — the section list is unit-tested, but the sections it actually
//! paints, and the "Or" that hangs off the join button, are not reachable from there.

use std::time::Duration;

use warp::integration_testing::assertions::go_online;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::user_workspaces::{
    assert_team_creation_is_offered, join_a_native_workspace_as_member, leave_every_workspace,
    set_joinable_teams,
};
use warp::integration_testing::warp_drive::{
    assert_drive_create_a_team_section_visible, assert_drive_join_a_team_or_visible,
    assert_drive_join_a_team_section_visible, open_warp_drive,
};
use warpui_core::integration::TestStep;

use super::{Builder, new_builder};

const WORKSPACE_NAME: &str = "Acme";
const JOINABLE_TEAMS: &[&str] = &["Platform", "Design Systems"];

/// Whoever runs the capture test to look at the result is the same person likely to set
/// `WARPUI_PAUSE_INTEGRATION_TEST_AT_EVERY_STEP`, which adds three seconds per step and
/// pushes the walk past the default two-minute watchdog. That watchdog kills the process
/// before the recording is finalized, so the run loses the video it was for.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(600);

/// A state of the sidebar, with the steps that seed it and assert what it rendered.
struct SidebarState {
    /// Doubles as the screenshot file stem in [`test_warp_drive_teams_sections_captures`].
    label: &'static str,
    steps: Vec<TestStep>,
}

/// The two sidebars a teamless user can get, in the order a single app instance can walk
/// them. They are deliberately adjacent: the only difference between them is the workspace,
/// so together they show what the gate withholds and what it leaves alone.
fn sidebar_states() -> Vec<SidebarState> {
    vec![
        // A plain member of a native workspace. Create-team is withheld, and the "Or" goes
        // with it, since it only ever read as a lead-in to the create button below it.
        SidebarState {
            label: "warp_drive_native_workspace_member",
            steps: vec![
                join_a_native_workspace_as_member(WORKSPACE_NAME),
                set_joinable_teams(JOINABLE_TEAMS),
                assert_team_creation_is_offered(false),
                assert_drive_join_a_team_section_visible(true),
                assert_drive_create_a_team_section_visible(false),
                assert_drive_join_a_team_or_visible(false),
            ],
        },
        // Nobody's workspace, with the same teams to join: the sidebar the gate does not
        // touch, and the one the "Or" was written for.
        SidebarState {
            label: "warp_drive_no_workspace",
            steps: vec![
                leave_every_workspace(),
                set_joinable_teams(JOINABLE_TEAMS),
                assert_team_creation_is_offered(true),
                assert_drive_join_a_team_section_visible(true),
                assert_drive_create_a_team_section_visible(true),
                assert_drive_join_a_team_or_visible(true),
            ],
        },
    ]
}

/// Walks both sidebars a teamless user can get, asserting what each one rendered.
pub fn test_warp_drive_teams_sections_for_a_teamless_user() -> Builder {
    let mut builder = new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // The team sections refuse to render while offline.
        .with_step(go_online())
        .with_step(open_warp_drive());

    for state in sidebar_states() {
        builder = builder.with_steps(state.steps);
    }
    builder
}

/// The same walk, recorded and screenshotted for visual review.
///
/// Ignored in CI because frame capture needs a real display. Run it manually with:
///
/// ```sh
/// WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
///   cargo run -p integration --bin integration -- test_warp_drive_teams_sections_captures
/// ```
pub fn test_warp_drive_teams_sections_captures() -> Builder {
    let mut builder = new_builder()
        .with_real_display()
        .with_timeout(CAPTURE_TIMEOUT)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(go_online())
        .with_step(TestStep::new("Start recording").with_start_recording())
        .with_step(open_warp_drive());

    for state in sidebar_states() {
        let label = state.label;
        builder = builder.with_steps(state.steps).with_step(
            TestStep::new(&format!("Capture {label}")).with_take_screenshot(format!("{label}.png")),
        );
    }
    builder.with_step(TestStep::new("Stop recording").with_stop_recording())
}
