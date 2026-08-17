//! Integration coverage for the settings Teams page as seen by a user with no team of
//! their own.
//!
//! Which page that user gets is decided by model state the client cannot reach on its own:
//! whether their workspace's plan has native workspaces enabled, whether they administer
//! it, and whether it has teams open to join. Seeding those permutations and asserting on
//! the frame that came out is the only way to pin the routing down end to end — a unit test
//! can reach the copy and the predicate, but not the page they add up to.

use warp::integration_testing::assertions::go_online;
use warp::integration_testing::settings::{
    assert_teams_create_team_form_visible, assert_teams_join_a_team_list_visible,
    assert_teams_workspace_admin_panel_visible, assert_teams_workspace_section_visible,
    open_settings_page,
};
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::user_workspaces::{
    assert_team_creation_is_offered, join_a_native_workspace_as_admin,
    join_a_native_workspace_as_member, leave_every_workspace, set_joinable_teams,
};
use warp::settings_view::SettingsSection;
use warpui_core::integration::TestStep;

use super::{Builder, new_builder};

const WORKSPACE_NAME: &str = "Acme";
const JOINABLE_TEAMS: &[&str] = &["Platform", "Design Systems"];

/// A state of the page, with the steps that seed it and assert what it rendered.
struct TeamlessState {
    /// Doubles as the screenshot file stem in [`test_settings_teams_page_captures`].
    label: &'static str,
    steps: Vec<TestStep>,
}

/// Every page a user with no team of their own can land on, in the order a single app
/// instance can walk them.
fn teamless_states() -> Vec<TeamlessState> {
    vec![
        // A plain member with teams to join: the workspace section explains where they
        // stand, and the join list gives them somewhere to go.
        TeamlessState {
            label: "workspace_member_with_teams_to_join",
            steps: vec![
                join_a_native_workspace_as_member(WORKSPACE_NAME),
                set_joinable_teams(JOINABLE_TEAMS),
                assert_team_creation_is_offered(false),
                assert_teams_workspace_section_visible(true),
                assert_teams_workspace_admin_panel_visible(false),
                assert_teams_join_a_team_list_visible(true),
                assert_teams_create_team_form_visible(false),
            ],
        },
        // The same member with nothing to join. This is the state that used to render
        // near-empty, so the workspace section is the whole reason the page says anything.
        TeamlessState {
            label: "workspace_member_with_nothing_to_join",
            steps: vec![
                set_joinable_teams(&[]),
                assert_teams_workspace_section_visible(true),
                assert_teams_join_a_team_list_visible(false),
                assert_teams_create_team_form_visible(false),
            ],
        },
        // An admin gets the link to the web admin panel, the only surface that can create
        // a team inside an existing workspace, but still not the client's create form.
        TeamlessState {
            label: "workspace_admin",
            steps: vec![
                join_a_native_workspace_as_admin(WORKSPACE_NAME),
                assert_team_creation_is_offered(false),
                assert_teams_workspace_section_visible(true),
                assert_teams_workspace_admin_panel_visible(true),
                assert_teams_create_team_form_visible(false),
            ],
        },
        // Nobody's workspace: the create-team page is untouched by the gate.
        TeamlessState {
            label: "no_workspace",
            steps: vec![
                leave_every_workspace(),
                assert_team_creation_is_offered(true),
                assert_teams_create_team_form_visible(true),
                assert_teams_workspace_section_visible(false),
                assert_teams_workspace_admin_panel_visible(false),
            ],
        },
    ]
}

/// Walks every page a teamless user can land on, asserting what each one rendered.
pub fn test_settings_teams_page_states_for_a_teamless_user() -> Builder {
    let mut builder = new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // The page refuses to render its content while offline.
        .with_step(go_online())
        .with_step(open_settings_page(SettingsSection::Teams));

    for state in teamless_states() {
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
///   cargo run -p integration --bin integration -- test_settings_teams_page_captures
/// ```
pub fn test_settings_teams_page_captures() -> Builder {
    let mut builder = new_builder()
        .with_real_display()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(go_online())
        .with_step(TestStep::new("Start recording").with_start_recording())
        .with_step(open_settings_page(SettingsSection::Teams));

    for state in teamless_states() {
        let label = state.label;
        builder = builder.with_steps(state.steps).with_step(
            TestStep::new(&format!("Capture {label}")).with_take_screenshot(format!("{label}.png")),
        );
    }
    builder.with_step(TestStep::new("Stop recording").with_stop_recording())
}
