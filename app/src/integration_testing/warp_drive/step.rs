//! Steps that open the Warp Drive sidebar and assert what it painted.
//!
//! What a teamless user is offered there — a "Create a team" section, a "Join a team"
//! section, or neither — depends on their workspace's plan, which the client only learns
//! from the server. Each assertion reads the frame that was actually drawn, so a test
//! checks the render rather than a restatement of the conditions behind it.

use warpui::integration::TestStep;

use crate::drive::index::{
    DRIVE_CREATE_A_TEAM_SECTION_POSITION_ID, DRIVE_JOIN_A_TEAM_OR_POSITION_ID,
    DRIVE_JOIN_A_TEAM_SECTION_POSITION_ID,
};
use crate::integration_testing::step::{
    assert_element_painted, dispatch_workspace_action, new_step_with_default_assertions,
};
use crate::integration_testing::warp_drive::assert_is_left_panel_open;
use crate::workspace::WorkspaceAction;

/// Opens the Warp Drive sidebar and waits until its panel is showing.
///
/// Warp Drive is one of the views the left panel hosts, so "open" is a property of that
/// panel. That the sidebar rendered Warp Drive rather than a sibling view is what the
/// section assertions below establish.
pub fn open_warp_drive() -> TestStep {
    new_step_with_default_assertions("Open Warp Drive")
        .with_action(|app, _, _| {
            dispatch_workspace_action(app, WorkspaceAction::OpenWarpDrive);
        })
        .add_named_assertion("the left panel is open", assert_is_left_panel_open())
}

/// The "Create a team" section, which a member of a native workspace must not be offered.
pub fn assert_drive_create_a_team_section_visible(visible: bool) -> TestStep {
    assert_element_painted(
        DRIVE_CREATE_A_TEAM_SECTION_POSITION_ID.to_string(),
        "Warp Drive create-a-team section".to_string(),
        visible,
    )
}

/// The "Join a team" section, which stays available inside a native workspace.
pub fn assert_drive_join_a_team_section_visible(visible: bool) -> TestStep {
    assert_element_painted(
        DRIVE_JOIN_A_TEAM_SECTION_POSITION_ID.to_string(),
        "Warp Drive join-a-team section".to_string(),
        visible,
    )
}

/// The "Or" that closes the join section. It only ever read as a lead-in to the create
/// button below it, so it must go wherever create-team does.
pub fn assert_drive_join_a_team_or_visible(visible: bool) -> TestStep {
    assert_element_painted(
        DRIVE_JOIN_A_TEAM_OR_POSITION_ID.to_string(),
        "Warp Drive join-a-team \"Or\"".to_string(),
        visible,
    )
}
