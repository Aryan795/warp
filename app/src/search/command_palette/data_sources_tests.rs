use std::sync::Arc;

use chrono::Utc;
use cloud_object_client::MockObjectClient;
use settings::manager::SettingsManager;
use warpui::{App, SingletonEntity, WindowId};

use super::*;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::model::view::CloudViewModel;
use crate::cloud_object::{
    Owner, Revision, ServerMetadata, ServerNotebook, ServerPermissions, ServerWorkflow,
};
use crate::network::NetworkStatus;
use crate::notebooks::manager::NotebookManager;
use crate::notebooks::{CloudNotebookModel, NotebookId};
use crate::search::data_source::Query;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::ServerId;
use crate::server::ids::SyncId::{self};
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::sync_queue::SyncQueue;
use crate::settings::AISettings;
use crate::system::SystemStats;
use crate::workflows::workflow::Workflow;
use crate::workflows::{CloudWorkflowModel, WorkflowId};
use crate::workspaces::team::Team;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::Workspace;

fn mock_server_metadata() -> ServerMetadata {
    ServerMetadata {
        uid: ServerId::default(),
        revision: Revision::now(),
        metadata_last_updated_ts: Utc::now().into(),
        trashed_ts: None,
        folder_id: None,
        is_welcome_object: false,
        creator_uid: None,
        last_editor_uid: None,
        current_editor_uid: None,
    }
}

fn mock_server_permissions(owner: Owner) -> ServerPermissions {
    ServerPermissions {
        space: owner,
        guests: Vec::new(),
        anyone_link_sharing: None,
        permissions_last_updated_ts: Utc::now().into(),
    }
}

fn mock_server_workflow(id: WorkflowId, owner: Owner) -> ServerWorkflow {
    ServerWorkflow::new(
        SyncId::ServerId(id.into()),
        CloudWorkflowModel::new(Workflow::new(format!("foo{id}"), format!("bar{id}"))),
        mock_server_metadata(),
        mock_server_permissions(owner),
    )
}

fn mock_server_notebook(id: NotebookId, owner: Owner) -> ServerNotebook {
    ServerNotebook::new(
        SyncId::ServerId(id.into()),
        CloudNotebookModel {
            title: format!("foo{id}"),
            data: format!("bar{id}"),
            ai_document_id: None,
            conversation_id: None,
        },
        mock_server_metadata(),
        mock_server_permissions(owner),
    )
}

fn team_for_test(uid: i64, name: &str) -> Team {
    Team {
        uid: uid.into(),
        name: name.to_owned(),
        color: None,
        invite_code: None,
        members: vec![],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
    }
}

fn workspace_for_test(teams: Vec<Team>) -> Workspace {
    Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "test".to_string(),
        stripe_customer_id: None,
        teams,
        billing_metadata: Default::default(),
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: None,
        has_billing_history: false,
        settings: Default::default(),
        invite_code: None,
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        members: vec![],
        total_requests_used_since_last_refresh: 0,
    }
}

fn initialize_app(app: &mut App, workspaces: Vec<Workspace>) {
    // Add the necessary singleton models to the App
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| SystemStats::new());
    let mock_team_client = Arc::new(MockTeamClient::new());
    let mock_workspace_client = Arc::new(MockWorkspaceClient::new());
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            mock_team_client.clone(),
            mock_workspace_client.clone(),
            workspaces,
            ctx,
        )
    });
    app.add_singleton_model(TeamTesterStatus::new);
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|ctx| UpdateManager::new(None, Arc::new(MockObjectClient::new()), ctx));
    app.add_singleton_model(|_| UserProfiles::new(Vec::new()));
    app.add_singleton_model(CloudViewModel::new);
    app.add_singleton_model(NotebookManager::mock);
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| SettingsManager::default());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.update(crate::settings::init_and_register_user_preferences);
    app.update(AISettings::register_and_subscribe_to_events);
}

#[test]
fn test_drive_data_source_correctly_filters_drive_filter() {
    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![]);
        // Initialize CloudModel
        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.upsert_from_server_notebook(
                mock_server_notebook(1.into(), Owner::mock_current_user()),
                ctx,
            );
            model.upsert_from_server_workflow(
                mock_server_workflow(2.into(), Owner::mock_current_user()),
                ctx,
            )
        });

        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle =
            app.add_model(|ctx| warp_drive::DataSource::new(WindowId::new(), ctx));
        mixer.update(&mut app, |mixer, ctx| {
            // Add the drive data source with the relevant filters
            mixer.add_sync_source(
                data_source_handle,
                [
                    QueryFilter::Drive,
                    QueryFilter::Notebooks,
                    QueryFilter::Workflows,
                ],
            );

            // Run the query with the drive filter
            mixer.run_query(
                Query {
                    filters: HashSet::from([QueryFilter::Drive]),
                    text: "foo".into(),
                },
                ctx,
            );
        });

        app.read(|app| {
            let results = mixer.as_ref(app).results();

            // Expect both of the results to be included
            assert_eq!(results.len(), 2);
        });
    })
}

#[test]
fn test_drive_data_source_correctly_filters_no_filter() {
    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![]);
        // Initialize CloudModel
        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.upsert_from_server_notebook(
                mock_server_notebook(1.into(), Owner::mock_current_user()),
                ctx,
            );
            model.upsert_from_server_workflow(
                mock_server_workflow(2.into(), Owner::mock_current_user()),
                ctx,
            )
        });
        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle =
            app.add_model(|ctx| warp_drive::DataSource::new(WindowId::new(), ctx));
        mixer.update(&mut app, |mixer, ctx| {
            // Add the drive data source with the relevant filters
            mixer.add_sync_source(
                data_source_handle,
                [
                    QueryFilter::Drive,
                    QueryFilter::Notebooks,
                    QueryFilter::Workflows,
                ],
            );

            // Run the query with no filter
            mixer.run_query(
                Query {
                    filters: HashSet::new(),
                    text: "foo".into(),
                },
                ctx,
            );
        });

        app.read(|app| {
            let results = mixer.as_ref(app).results();

            // Expect both of the results to be included
            assert_eq!(results.len(), 2);
        });
    })
}

#[test]
fn test_drive_data_source_correctly_filters_workflow_filter() {
    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![]);
        // Initialize CloudModel
        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.upsert_from_server_notebook(
                mock_server_notebook(1.into(), Owner::mock_current_user()),
                ctx,
            );
            model.upsert_from_server_workflow(
                mock_server_workflow(2.into(), Owner::mock_current_user()),
                ctx,
            )
        });
        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle =
            app.add_model(|ctx| warp_drive::DataSource::new(WindowId::new(), ctx));
        mixer.update(&mut app, |mixer, ctx| {
            // Add the drive data source with the relevant filters
            mixer.add_sync_source(
                data_source_handle,
                [
                    QueryFilter::Drive,
                    QueryFilter::Notebooks,
                    QueryFilter::Workflows,
                ],
            );

            // Run the query with no filter
            mixer.run_query(
                Query {
                    filters: HashSet::from([QueryFilter::Workflows]),
                    text: "foo".into(),
                },
                ctx,
            );
        });

        app.read(|app| {
            let results = mixer.as_ref(app).results();

            // Expect only the workflow result to be included
            assert_eq!(results.len(), 1);

            assert!(results[0].accessibility_label().starts_with("Workflow:"));
        });
    })
}

#[test]
fn test_drive_data_source_correctly_filters_notebook_filter() {
    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![]);
        // Initialize CloudModel
        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.upsert_from_server_notebook(
                mock_server_notebook(1.into(), Owner::mock_current_user()),
                ctx,
            );
            model.upsert_from_server_workflow(
                mock_server_workflow(2.into(), Owner::mock_current_user()),
                ctx,
            )
        });
        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle =
            app.add_model(|ctx| warp_drive::DataSource::new(WindowId::new(), ctx));
        mixer.update(&mut app, |mixer, ctx| {
            // Add the drive data source with the relevant filters
            mixer.add_sync_source(
                data_source_handle,
                [
                    QueryFilter::Drive,
                    QueryFilter::Notebooks,
                    QueryFilter::Workflows,
                ],
            );

            // Run the query with no filter
            mixer.run_query(
                Query {
                    filters: HashSet::from([QueryFilter::Notebooks]),
                    text: "foo".into(),
                },
                ctx,
            );
        });

        app.read(|app| {
            let results = mixer.as_ref(app).results();

            // Expect only the workflow result to be included
            assert_eq!(results.len(), 1);

            assert!(results[0].accessibility_label().starts_with("Notebook:"));
        });
    })
}

#[test]
fn test_drive_data_source_only_returns_objects_visible_in_the_window() {
    let selected_team = team_for_test(123, "selected");
    let other_team = team_for_test(456, "other");
    let workspace = workspace_for_test(vec![selected_team.clone(), other_team.clone()]);

    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![workspace]);

        let selected_team_workflow_id: WorkflowId = 1.into();
        let other_team_workflow_id: WorkflowId = 2.into();
        let personal_workflow_id: WorkflowId = 3.into();
        CloudModel::handle(&app).update(&mut app, |model, ctx| {
            model.upsert_from_server_workflow(
                mock_server_workflow(
                    selected_team_workflow_id,
                    Owner::Team {
                        team_uid: selected_team.uid,
                    },
                ),
                ctx,
            );
            model.upsert_from_server_workflow(
                mock_server_workflow(
                    other_team_workflow_id,
                    Owner::Team {
                        team_uid: other_team.uid,
                    },
                ),
                ctx,
            );
            model.upsert_from_server_workflow(
                mock_server_workflow(personal_workflow_id, Owner::mock_current_user()),
                ctx,
            );
        });

        let window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_id, selected_team.uid, ctx);
        });

        let mixer = app.add_model(|_| CommandPaletteMixer::new());
        let data_source_handle = app.add_model(|ctx| warp_drive::DataSource::new(window_id, ctx));
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_sync_source(data_source_handle, [QueryFilter::Workflows]);

            mixer.run_query(
                Query {
                    filters: HashSet::from([QueryFilter::Workflows]),
                    text: "foo".into(),
                },
                ctx,
            );
        });

        app.read(|app| {
            let mut labels = mixer
                .as_ref(app)
                .results()
                .iter()
                .map(|result| result.accessibility_label())
                .collect::<Vec<_>>();
            labels.sort();

            // The window is switched to `selected_team`, so only that team's workflow and the
            // user's personal workflow are searchable; `other_team`'s workflow is not.
            let mut expected = vec![
                format!("Workflow: foo{selected_team_workflow_id}"),
                format!("Workflow: foo{personal_workflow_id}"),
            ];
            expected.sort();
            assert_eq!(labels, expected);
        });
    })
}
