//! Steps that seed [`UserWorkspaces`] directly.
//!
//! Workspace membership, plan policies, and team discovery all arrive from the server, so
//! seeding the model is the only way an integration test can reach a given workspace state.

use warpui::integration::TestStep;
use warpui::{SingletonEntity, async_assert};

use crate::auth::{AuthStateProvider, UserUid};
use crate::workspaces::team::{DiscoverableTeam, MembershipRole};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{
    NativeWorkspacesPolicy, Workspace, WorkspaceMember, WorkspaceMemberUsageInfo,
};

/// Server ids are fixed-width, so these are padded to the 22 characters `ServerId` requires.
const NATIVE_WORKSPACE_UID: &str = "native_workspace_uid00";
const JOINABLE_TEAM_UID_PREFIX: &str = "joinable_team_uid";

/// Puts the signed-in user in a native workspace as a plain member.
pub fn join_a_native_workspace_as_member(workspace_name: &'static str) -> TestStep {
    join_a_native_workspace(workspace_name, MembershipRole::User)
}

/// Puts the signed-in user in a native workspace as one of its admins.
pub fn join_a_native_workspace_as_admin(workspace_name: &'static str) -> TestStep {
    join_a_native_workspace(workspace_name, MembershipRole::Admin)
}

/// Puts the signed-in user in a workspace whose plan has native workspaces turned on, at
/// `role`, with no team of their own.
///
/// The workspace is deliberately seeded with no teams. At the model layer a workspace's
/// `teams` only ever holds the teams the current user belongs to, and `update_workspaces`
/// binds the window to the first of them — a team here would route the Teams page to team
/// management instead of the teamless state. Teams the user could join are seeded
/// separately with [`set_joinable_teams`].
fn join_a_native_workspace(workspace_name: &'static str, role: MembershipRole) -> TestStep {
    TestStep::new(&format!(
        "Join the {workspace_name} native workspace as {role:?}"
    ))
    .with_action(move |app, _, _| {
        let (user_uid, user_email) = app.read(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get();
            (
                auth_state.user_id().unwrap_or_default(),
                auth_state.user_email().unwrap_or_default(),
            )
        });
        UserWorkspaces::handle(app).update(app, |user_workspaces, ctx| {
            let workspace_uid = NATIVE_WORKSPACE_UID.to_string().into();
            let mut workspace = Workspace::from_local_cache(
                workspace_uid,
                workspace_name.to_string(),
                Some(Vec::new()),
            );
            workspace.billing_metadata.tier.native_workspaces_policy =
                Some(NativeWorkspacesPolicy { enabled: true });
            workspace.members = vec![workspace_member(user_uid, user_email, role)];

            user_workspaces.update_workspaces(vec![workspace], ctx);
            user_workspaces.set_current_workspace_uid(workspace_uid, ctx);
        });
    })
    .add_named_assertion(
        format!("{workspace_name} is the current native workspace"),
        move |app, _| {
            UserWorkspaces::handle(app).read(app, |user_workspaces, _| {
                let name = user_workspaces
                    .current_native_workspace()
                    .map(|workspace| workspace.name.clone());
                async_assert!(
                    name.as_deref() == Some(workspace_name),
                    "current native workspace should be {workspace_name:?}, was {name:?}"
                )
            })
        },
    )
    .add_named_assertion("the user has no team of their own", move |app, _| {
        UserWorkspaces::handle(app).read(app, |user_workspaces, _| {
            async_assert!(
                !user_workspaces.has_teams(),
                "the seeded workspace should leave the user teamless"
            )
        })
    })
}

/// Seeds the teams the current user's workspace offers them to join, each accepting
/// invites. An empty slice clears them, which is the state that leaves the page with
/// nothing to join.
pub fn set_joinable_teams(names: &'static [&'static str]) -> TestStep {
    TestStep::new(&format!("Seed {} team(s) open to join", names.len()))
        .with_action(move |app, _, _| {
            UserWorkspaces::handle(app).update(app, |user_workspaces, ctx| {
                let teams = names
                    .iter()
                    .enumerate()
                    .map(|(index, name)| DiscoverableTeam {
                        team_uid: format!("{JOINABLE_TEAM_UID_PREFIX}{index:05}"),
                        num_members: index as i64 + 2,
                        name: (*name).to_string(),
                        team_accepting_invites: true,
                    })
                    .collect();
                user_workspaces.update_joinable_teams(teams, ctx);
            });
        })
        .add_named_assertion(
            format!("{} team(s) are open to join", names.len()),
            move |app, _| {
                UserWorkspaces::handle(app).read(app, |user_workspaces, _| {
                    let actual = user_workspaces.num_joinable_teams();
                    async_assert!(
                        actual == names.len(),
                        "expected {} team(s) open to join, found {actual}",
                        names.len()
                    )
                })
            },
        )
}

/// Drops the user out of every workspace, back to where a solo user starts.
pub fn leave_every_workspace() -> TestStep {
    TestStep::new("Leave every workspace")
        .with_action(move |app, _, _| {
            UserWorkspaces::handle(app).update(app, |user_workspaces, ctx| {
                user_workspaces.update_workspaces(Vec::new(), ctx);
                user_workspaces.update_joinable_teams(Vec::new(), ctx);
            });
        })
        .add_named_assertion("the user is in no workspace", move |app, _| {
            UserWorkspaces::handle(app).read(app, |user_workspaces, _| {
                async_assert!(
                    !user_workspaces.has_workspaces(),
                    "the user should be in no workspace"
                )
            })
        })
}

/// Asserts whether the client's own create-team flow is offered to the current user.
pub fn assert_team_creation_is_offered(offered: bool) -> TestStep {
    TestStep::new(&format!("Assert team creation is offered: {offered}")).add_named_assertion(
        format!("team creation offered is {offered}"),
        move |app, _| {
            let is_logged_in = app.read(|ctx| AuthStateProvider::as_ref(ctx).get().is_logged_in());
            UserWorkspaces::handle(app).read(app, |user_workspaces, _| {
                let actual = user_workspaces.should_offer_team_creation(is_logged_in);
                async_assert!(
                    actual == offered,
                    "team creation offered should be {offered}, was {actual}"
                )
            })
        },
    )
}

fn workspace_member(uid: UserUid, email: String, role: MembershipRole) -> WorkspaceMember {
    WorkspaceMember {
        uid,
        email,
        role,
        usage_info: WorkspaceMemberUsageInfo {
            is_unlimited: false,
            request_limit: 0,
            requests_used_since_last_refresh: 0,
            is_request_limit_prorated: false,
        },
    }
}
