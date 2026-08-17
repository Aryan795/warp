//! Integration coverage for the settings Teams page as seen by a user with no team of
//! their own.
//!
//! Which page that user gets is decided by model state the client cannot reach on its own:
//! whether their workspace's plan has native workspaces enabled, whether they administer
//! it, and whether it has teams open to join. Seeding those permutations and asserting on
//! the frame that came out is the only way to pin the routing down end to end — a unit test
//! can reach the copy and the predicate, but not the page they add up to.

use std::time::Duration;

use warp::integration_testing::assertions::go_online;
use warp::integration_testing::settings::{
    assert_teams_create_team_form_visible, assert_teams_join_a_team_list_visible,
    assert_teams_workspace_admin_panel_visible, assert_teams_workspace_create_team_form_visible,
    assert_teams_workspace_create_team_in_progress, assert_teams_workspace_section_visible,
    assert_teams_workspace_unresolved_visible, open_settings_page,
};
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::user_workspaces::{
    assert_team_creation_is_offered, join_a_native_workspace_as_admin,
    join_a_native_workspace_as_member, leave_every_workspace, set_joinable_teams,
    simulate_team_created_in_workspace,
};
use warp::settings_view::{
    SettingsSection, WORKSPACE_CREATE_TEAM_BUTTON_POSITION_ID,
    WORKSPACE_CREATE_TEAM_NAME_EDITOR_POSITION_ID,
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

/// Real-time pause held after each step of a recorded flow, so the resulting video shows
/// each state for long enough to actually watch, rather than flashing by in a fraction of
/// a second (the model-simulated steps this crate otherwise uses have no inherent pacing).
const RECORDING_PAUSE: Duration = Duration::from_secs(2);

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
        // An admin gets the in-app form that creates a team inside their own workspace,
        // plus the link to the web admin panel for what that form can't do (color,
        // visibility, member picker). This is not the personal create-team form, which
        // would hand them a second workspace instead of a team in this one.
        TeamlessState {
            label: "workspace_admin",
            steps: vec![
                join_a_native_workspace_as_admin(WORKSPACE_NAME),
                assert_team_creation_is_offered(false),
                assert_teams_workspace_section_visible(true),
                assert_teams_workspace_admin_panel_visible(true),
                assert_teams_workspace_create_team_form_visible(true),
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

/// Types `name` into the in-workspace create-team form and clicks Create, driving the
/// real dispatch path (`TeamsPageAction::CreateTeamInWorkspace` →
/// `TeamsPageView::create_team_in_workspace`) rather than skipping straight to a
/// simulated response. The channel this test runs against answers the resulting
/// `createTeamInWorkspace` request quickly with an error (there's no real server behind
/// it), which this test doesn't wait on; callers follow this with
/// [`simulate_team_created_in_workspace`] to reflect a (simulated) successful response
/// instead.
fn submit_create_team_in_workspace_form(name: &'static str) -> Vec<TestStep> {
    vec![
        TestStep::new(&format!("Type team name {name:?}"))
            .with_click_on_saved_position(WORKSPACE_CREATE_TEAM_NAME_EDITOR_POSITION_ID)
            .with_typed_characters(&[name]),
        TestStep::new("Click Create")
            .with_click_on_saved_position(WORKSPACE_CREATE_TEAM_BUTTON_POSITION_ID),
    ]
}

/// A native-workspace admin creates a team using the in-app form and lands on that
/// team's management page, rather than staying on the (now stale) teamless workspace
/// page. Driving the real button (rather than jumping straight to the simulated
/// response) exercises the real UI interaction; the mutation's own arguments (workspace,
/// trimmed name, visibility, seed) are covered precisely by mock-backed unit tests in
/// `workspaces::update_manager`, since this test's channel has no real server to assert
/// the request against.
pub fn test_settings_teams_page_workspace_admin_creates_a_team_and_lands_on_it() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(go_online())
        .with_step(open_settings_page(SettingsSection::Teams))
        .with_step(join_a_native_workspace_as_admin(WORKSPACE_NAME))
        .with_step(assert_teams_workspace_create_team_form_visible(true))
        .with_step(assert_teams_workspace_create_team_in_progress(false))
        .with_steps(submit_create_team_in_workspace_form("Platform"))
        .with_step(simulate_team_created_in_workspace("Platform"))
        .with_step(assert_teams_workspace_section_visible(false))
        .with_step(assert_teams_workspace_create_team_form_visible(false))
}

/// Walks every page a teamless user can land on, asserting what each one rendered.
pub fn test_settings_teams_page_states_for_a_teamless_user() -> Builder {
    let mut builder = new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // The page refuses to render its content while offline.
        .with_step(go_online())
        .with_step(open_settings_page(SettingsSection::Teams));

    for state in teamless_states() {
        builder = builder
            .with_steps(state.steps)
            .with_step(assert_teams_workspace_unresolved_visible(false));
    }
    builder
}

/// The workspace-admin create flow, recorded and screenshotted for visual review: the
/// admin sees the in-app create form, types a team name, submits it, and lands on the
/// resulting team's management page instead of the (now stale) teamless workspace page.
///
/// The click on the real Create button does dispatch the real `createTeamInWorkspace`
/// request, which this test's channel quickly answers with an error since there's no
/// real server behind it; the step immediately after reflects a (simulated) successful
/// response the same way `test_settings_teams_page_workspace_admin_creates_a_team_and_lands_on_it`
/// does, so the capture doesn't wait on that request.
///
/// Ignored in CI because frame capture needs a real display. Run it manually with:
///
/// ```sh
/// WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
///   cargo run -p integration --bin integration -- test_settings_teams_page_workspace_admin_creates_a_team_captures
/// ```
pub fn test_settings_teams_page_workspace_admin_creates_a_team_captures() -> Builder {
    new_builder()
        .with_real_display()
        .with_timeout(CAPTURE_TIMEOUT)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(go_online())
        .with_step(TestStep::new("Start recording").with_start_recording())
        .with_step(open_settings_page(SettingsSection::Teams))
        .with_step(join_a_native_workspace_as_admin(WORKSPACE_NAME))
        .with_step(assert_teams_workspace_create_team_form_visible(true))
        .with_step(
            TestStep::new("Capture the in-workspace create-team form")
                .with_take_screenshot("workspace_admin_create_team_form.png")
                .set_post_step_pause(RECORDING_PAUSE),
        )
        .with_steps(
            submit_create_team_in_workspace_form("Platform")
                .into_iter()
                .map(|step| step.set_post_step_pause(RECORDING_PAUSE))
                .collect::<Vec<_>>(),
        )
        .with_step(simulate_team_created_in_workspace("Platform"))
        .with_step(
            TestStep::new("Capture landing on the created team")
                .with_take_screenshot("workspace_admin_lands_on_created_team.png")
                .set_post_step_pause(RECORDING_PAUSE),
        )
        .with_step(TestStep::new("Stop recording").with_stop_recording())
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
        .with_timeout(CAPTURE_TIMEOUT)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(go_online())
        .with_step(TestStep::new("Start recording").with_start_recording())
        .with_step(open_settings_page(SettingsSection::Teams));

    for state in teamless_states() {
        let label = state.label;
        builder = builder
            .with_steps(state.steps)
            .with_step(assert_teams_workspace_unresolved_visible(false))
            .with_step(
                TestStep::new(&format!("Capture {label}"))
                    .with_take_screenshot(format!("{label}.png")),
            );
    }
    builder.with_step(TestStep::new("Stop recording").with_stop_recording())
}
