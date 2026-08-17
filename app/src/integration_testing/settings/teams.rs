//! Assertions for which sections the settings Teams page painted.
//!
//! A user with no team of their own can land on one of two pages — the create-team form or
//! the workspace page — and the workspace page varies with the user's role and with whether
//! the workspace has teams open to join. Each of these asserts on one section of the frame
//! that was actually drawn, so the state a test seeded is checked against the real render
//! rather than against a restatement of the conditions behind it.

use warpui::async_assert;
use warpui::integration::TestStep;

use crate::integration_testing::step::assert_element_painted;
use crate::integration_testing::view_getters::teams_page_view;
use crate::settings_view::{
    CREATE_TEAM_FORM_POSITION_ID, JOIN_A_TEAM_LIST_POSITION_ID, TeamsPageView,
    WORKSPACE_ADMIN_PANEL_BUTTON_POSITION_ID, WORKSPACE_CREATE_TEAM_FORM_POSITION_ID,
    WORKSPACE_SECTION_POSITION_ID, WORKSPACE_UNRESOLVED_POSITION_ID,
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

/// The in-workspace create-team form, which only a workspace admin sees, targeting their
/// own workspace rather than creating a brand-new one.
pub fn assert_teams_workspace_create_team_form_visible(visible: bool) -> TestStep {
    assert_element_painted(
        WORKSPACE_CREATE_TEAM_FORM_POSITION_ID.to_string(),
        "Teams page in-workspace create-team form".to_string(),
        visible,
    )
}

/// Whether an in-workspace create-team submission is currently in flight. Reading the
/// view's own state directly (rather than a saved position) proves the create button
/// actually dispatched, since a broken dispatch would leave this false even though the
/// button and form are still painted.
pub fn assert_teams_workspace_create_team_in_progress(in_progress: bool) -> TestStep {
    TestStep::new(&format!(
        "Assert in-workspace create-team submission in progress: {in_progress}"
    ))
    .add_named_assertion(
        format!("in-workspace create-team submission in progress is {in_progress}"),
        move |app, window_id| {
            let actual = teams_page_view(app, window_id)
                .read(app, |view: &TeamsPageView, _| {
                    view.is_creating_team_in_workspace()
                });
            async_assert!(
                actual == in_progress,
                "in-workspace create-team submission in progress should be {in_progress}, was {actual}"
            )
        },
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
