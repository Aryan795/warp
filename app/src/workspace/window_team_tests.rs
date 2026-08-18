use warpui::{App, SingletonEntity};

use super::*;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::workspaces::team::{Team, TeamVisibility};
use crate::workspaces::workspace::Workspace as UserWorkspace;

fn team_for_test(uid: i64, name: &str) -> Team {
    Team {
        uid: uid.into(),
        name: name.to_owned(),
        color: None,
        invite_link: None,
        members: vec![],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
        visibility: TeamVisibility::Open,
    }
}

fn workspace_for_test(teams: Vec<Team>) -> UserWorkspace {
    UserWorkspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "test".to_string(),
        stripe_customer_id: None,
        teams,
        billing_metadata: Default::default(),
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: None,
        has_billing_history: false,
        settings: Default::default(),
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        members: vec![],
        total_requests_used_since_last_refresh: 0,
    }
}

fn initialize_app(app: &mut App, teams: Vec<Team>) {
    app.add_singleton_model(crate::settings::PrivacySettings::mock);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            std::sync::Arc::new(MockTeamClient::new()),
            std::sync::Arc::new(MockWorkspaceClient::new()),
            if teams.is_empty() {
                vec![]
            } else {
                vec![workspace_for_test(teams)]
            },
            ctx,
        )
    });
}

#[test]
fn no_teams_reconciles_to_none() {
    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![]);

        let window_team = app.add_model(|ctx| WindowTeam::new(None, ctx));
        app.read(|ctx| {
            assert_eq!(window_team.as_ref(ctx).uid(), None);
        });
    })
}

#[test]
fn sole_team_is_always_selected_even_if_a_different_initial_uid_was_provided() {
    let team = team_for_test(123, "solo");
    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![team.clone()]);

        let window_team = app.add_model(|ctx| WindowTeam::new(Some(456.into()), ctx));
        app.read(|ctx| {
            assert_eq!(window_team.as_ref(ctx).uid(), Some(team.uid));
        });
    })
}

#[test]
fn multi_team_preserves_a_still_valid_selection() {
    let first = team_for_test(123, "first");
    let second = team_for_test(456, "second");
    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![first.clone(), second.clone()]);

        let window_team = app.add_model(|ctx| WindowTeam::new(Some(second.uid), ctx));
        app.read(|ctx| {
            assert_eq!(window_team.as_ref(ctx).uid(), Some(second.uid));
        });
    })
}

#[test]
fn multi_team_falls_back_to_default_when_selection_becomes_invalid() {
    let first = team_for_test(123, "first");
    let second = team_for_test(456, "second");
    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![first.clone(), second.clone()]);

        let window_team = app.add_model(|ctx| WindowTeam::new(Some(789.into()), ctx));
        app.read(|ctx| {
            assert_eq!(window_team.as_ref(ctx).uid(), Some(first.uid));
        });
    })
}

#[test]
fn removing_all_but_one_team_reconciles_the_selection_on_teams_changed() {
    let first = team_for_test(123, "first");
    let second = team_for_test(456, "second");
    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![first.clone(), second.clone()]);

        let window_team = app.add_model(|ctx| WindowTeam::new(Some(second.uid), ctx));
        app.read(|ctx| {
            assert_eq!(window_team.as_ref(ctx).uid(), Some(second.uid));
        });

        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.update_workspaces(vec![workspace_for_test(vec![first.clone()])], ctx);
        });

        app.read(|ctx| {
            assert_eq!(window_team.as_ref(ctx).uid(), Some(first.uid));
        });
    })
}

#[test]
fn metadata_only_change_preserves_the_selected_uid_and_still_emits_changed() {
    let team = team_for_test(123, "solo");
    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![team.clone()]);

        let window_team = app.add_model(|ctx| WindowTeam::new(Some(team.uid), ctx));

        let (sender, receiver) = async_channel::unbounded();
        app.update(|ctx| {
            ctx.subscribe_to_model(&window_team, move |_, event: &WindowTeamEvent, _| {
                if matches!(event, WindowTeamEvent::Changed) {
                    let _ = sender.try_send(());
                }
            });
        });

        // Rename the team without changing its uid or membership; this is purely a metadata
        // change but must still surface as a `TeamsChanged`-triggered `Changed` event.
        let mut renamed_team = team.clone();
        renamed_team.name = "renamed".to_string();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.update_workspaces(vec![workspace_for_test(vec![renamed_team])], ctx);
        });

        receiver
            .try_recv()
            .expect("expected WindowTeamEvent::Changed to be emitted for a metadata-only change");
        app.read(|ctx| {
            assert_eq!(window_team.as_ref(ctx).uid(), Some(team.uid));
        });
    })
}
