use super::*;

#[test]
fn native_workspace_admin_can_create_a_team() {
    let presentation = CreateTeamPagePresentation::new(true, Some(true));

    assert!(presentation.show_create_team_section);
    assert!(presentation.show_discovery_separator);
    assert_eq!(presentation.discovery_header, JOIN_TEAM_HEADER_WITH_CREATE);
}

#[test]
fn native_workspace_non_admin_can_only_join_a_team() {
    let presentation = CreateTeamPagePresentation::new(true, Some(false));

    assert!(!presentation.show_create_team_section);
    assert!(!presentation.show_discovery_separator);
    assert_eq!(
        presentation.discovery_header,
        JOIN_TEAM_HEADER_WITHOUT_CREATE
    );
}

#[test]
fn native_workspace_without_a_resolved_user_can_only_join_a_team() {
    let presentation = CreateTeamPagePresentation::new(true, None);

    assert!(!presentation.show_create_team_section);
    assert!(!presentation.show_discovery_separator);
    assert_eq!(
        presentation.discovery_header,
        JOIN_TEAM_HEADER_WITHOUT_CREATE
    );
}

#[test]
fn non_native_workspace_preserves_team_creation_for_non_admins() {
    let presentation = CreateTeamPagePresentation::new(false, Some(false));

    assert!(presentation.show_create_team_section);
    assert!(presentation.show_discovery_separator);
    assert_eq!(presentation.discovery_header, JOIN_TEAM_HEADER_WITH_CREATE);
}
