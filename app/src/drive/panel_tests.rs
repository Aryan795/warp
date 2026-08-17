use std::sync::Arc;

use warp_core::ui::appearance::Appearance;
use warpui::platform::WindowStyle;
use warpui::{App, SingletonEntity, ViewHandle};

use super::DrivePanel;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::cloud_object::Space;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::model::view::CloudViewModel;
use crate::drive::index::DriveIndexSection;
use crate::network::NetworkStatus;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::sync_queue::SyncQueue;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings::PrivacySettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::terminal::resizable_data::ResizableData;
use crate::terminal::shared_session::permissions_manager::SessionPermissionsManager;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{NativeWorkspacesPolicy, Workspace, WorkspaceUid};
use crate::{ASSETS, ObjectActions};

fn initialize_app(app: &mut App) {
    initialize_app_with_workspaces(app, vec![]);
}

/// Seeds `workspaces` as the local cache, which is what the client starts from before the
/// server has described them. Tests that need the authoritative answer must also call
/// [`apply_workspaces_metadata`].
fn initialize_app_with_workspaces(app: &mut App, workspaces: Vec<Workspace>) {
    initialize_settings_for_tests(app);

    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            workspaces,
            ctx,
        )
    });
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(|_| ResizableData::default());
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(CloudViewModel::mock);
    app.add_singleton_model(|_| ObjectActions::new(Vec::new()));
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(SessionPermissionsManager::new);
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);
}

/// Applies `workspaces` as an authoritative workspaces-metadata response, the way a
/// successful poll does.
fn apply_workspaces_metadata(app: &mut App, workspaces: Vec<Workspace>) {
    UserWorkspaces::handle(app).update(app, |user_workspaces, ctx| {
        user_workspaces.update_workspaces(workspaces, ctx);
    });
}

/// A cached workspace whose plan the server has since described as native.
fn native_workspace() -> Workspace {
    let mut workspace = Workspace::from_local_cache(
        WorkspaceUid::from("workspace_uid123456789".to_string()),
        "Test Workspace".to_string(),
        None,
    );
    workspace.billing_metadata.tier.native_workspaces_policy =
        Some(NativeWorkspacesPolicy { enabled: true });
    workspace
}

fn drive_index_sections(app: &App, panel: &ViewHandle<DrivePanel>) -> Vec<DriveIndexSection> {
    let index = panel.read(app, |panel, _| panel.index_view.clone());
    index.read(app, |index, _| index.sections().to_vec())
}

#[test]
fn test_warp_drive_sections_with_no_team() {
    App::test(ASSETS, |mut app| async move {
        initialize_app(&mut app);
        apply_workspaces_metadata(&mut app, vec![]);

        // Instead of being in the panel module and depending on DrivePanel, this test should be in the index module.
        // It happens to be here for the time being because `DriveIndex` depends on `DrivePanel` calling the `initialize_section_states` method.
        // Ideally, the constructor should handle the necessary initialization but for now this functional test asserts that the drive index is setup.
        let (_, panel) = app.add_window(WindowStyle::NotStealFocus, DrivePanel::new);

        assert_eq!(
            drive_index_sections(&app, &panel),
            [
                DriveIndexSection::CreateATeam,
                DriveIndexSection::Space(Space::Personal)
            ]
        );
    })
}

#[test]
fn test_warp_drive_sections_omit_create_a_team_for_a_teamless_native_workspace_member() {
    App::test(ASSETS, |mut app| async move {
        initialize_app_with_workspaces(&mut app, vec![native_workspace()]);
        apply_workspaces_metadata(&mut app, vec![native_workspace()]);

        let (_, panel) = app.add_window(WindowStyle::NotStealFocus, DrivePanel::new);

        assert_eq!(
            drive_index_sections(&app, &panel),
            [DriveIndexSection::Space(Space::Personal)],
            "the create-team section should be absent, not present-but-empty"
        );
    })
}

#[test]
fn test_warp_drive_sections_omit_create_a_team_until_workspace_metadata_arrives() {
    App::test(ASSETS, |mut app| async move {
        // The launch state of a native-workspace member: the workspace is restored from
        // SQLite, which carries no plan, so the client cannot yet tell it apart from a
        // solo user's and must not offer a form that would create a second workspace.
        let cached = Workspace::from_local_cache(
            WorkspaceUid::from("workspace_uid123456789".to_string()),
            "Test Workspace".to_string(),
            None,
        );
        initialize_app_with_workspaces(&mut app, vec![cached]);

        let (_, panel) = app.add_window(WindowStyle::NotStealFocus, DrivePanel::new);

        assert_eq!(
            drive_index_sections(&app, &panel),
            [DriveIndexSection::Space(Space::Personal)],
            "create-team must stay withheld until the server describes the workspace"
        );
    })
}

#[test]
fn test_warp_drive_sections_omit_create_a_team_when_workspace_metadata_never_arrives() {
    App::test(ASSETS, |mut app| async move {
        let cached = Workspace::from_local_cache(
            WorkspaceUid::from("workspace_uid123456789".to_string()),
            "Test Workspace".to_string(),
            None,
        );
        initialize_app_with_workspaces(&mut app, vec![cached]);
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.note_workspaces_metadata_unavailable(ctx);
        });

        let (_, panel) = app.add_window(WindowStyle::NotStealFocus, DrivePanel::new);

        assert_eq!(
            drive_index_sections(&app, &panel),
            [DriveIndexSection::Space(Space::Personal)],
            "exhausted retries leave the workspace unknown, so create-team stays withheld"
        );
    })
}
