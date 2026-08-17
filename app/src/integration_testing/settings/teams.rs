//! Assertions for which sections the settings Teams page painted.
//!
//! A user with no team of their own can land on one of two pages — the create-team form or
//! the workspace page — and the workspace page varies with the user's role and with whether
//! the workspace has teams open to join. Each of these asserts on one section of the frame
//! that was actually drawn, so the state a test seeded is checked against the real render
//! rather than against a restatement of the conditions behind it.

use warpui::integration::TestStep;

use crate::integration_testing::step::assert_element_painted;
use crate::settings_view::{
    CREATE_TEAM_FORM_POSITION_ID, JOIN_A_TEAM_LIST_POSITION_ID,
    WORKSPACE_ADMIN_PANEL_BUTTON_POSITION_ID, WORKSPACE_SECTION_POSITION_ID,
    WORKSPACE_UNRESOLVED_POSITION_ID,
};

/// The "Workspace" section, which only a user in a native workspace sees.
pub fn assert_teams_workspace_section_visible(visible: bool) -> TestStep {
    assert_element_painted(
        WORKSPACE_SECTION_POSITION_ID.to_string(),
        "Teams page workspace section".to_string(),
        visible,
    )
}

/// The "Open admin panel" link, which only a workspace admin sees.
pub fn assert_teams_workspace_admin_panel_visible(visible: bool) -> TestStep {
    assert_element_painted(
        WORKSPACE_ADMIN_PANEL_BUTTON_POSITION_ID.to_string(),
        "Teams page workspace admin panel link".to_string(),
        visible,
    )
}

/// The list of teams open to join, which is absent when the workspace has none.
pub fn assert_teams_join_a_team_list_visible(visible: bool) -> TestStep {
    assert_element_painted(
        JOIN_A_TEAM_LIST_POSITION_ID.to_string(),
        "Teams page join-a-team list".to_string(),
        visible,
    )
}

/// The create-team form, which is withheld inside a native workspace.
pub fn assert_teams_create_team_form_visible(visible: bool) -> TestStep {
    assert_element_painted(
        CREATE_TEAM_FORM_POSITION_ID.to_string(),
        "Teams page create-team form".to_string(),
        visible,
    )
}

/// The placeholder shown while the client cannot yet say which workspace the user is in.
pub fn assert_teams_workspace_unresolved_visible(visible: bool) -> TestStep {
    assert_element_painted(
        WORKSPACE_UNRESOLVED_POSITION_ID.to_string(),
        "Teams page unresolved-workspace placeholder".to_string(),
        visible,
    )
}
