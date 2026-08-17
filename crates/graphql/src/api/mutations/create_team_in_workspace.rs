use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;
use crate::workspace::{MembershipRole, Team, TeamVisibility};

/*
mutation CreateTeamInWorkspace($input: CreateTeamInWorkspaceInput!, $requestContext: RequestContext!) {
  createTeamInWorkspace(input: $input, requestContext: $requestContext) {
    ... on CreateTeamInWorkspaceOutput {
      team {
        uid
        name
        members {
          uid
          email
          role
        }
        settings {
          ...
        }
        color
        inviteLink
      }
      responseContext {
        serverVersion
      }
    }
    ... on UserFacingError {
      error {
        message
      }
      responseContext {
        serverVersion
      }
    }
  }
}
*/

#[derive(cynic::QueryVariables, Debug)]
pub struct CreateTeamInWorkspaceVariables {
    pub input: CreateTeamInWorkspaceInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct CreateTeamInWorkspaceInput {
    pub workspace_uid: cynic::Id,
    pub name: String,
    pub visibility: TeamVisibility,
    pub color: Option<String>,
    pub members: Vec<CreateTeamInWorkspaceMemberInput>,
}

#[derive(cynic::InputObject, Debug)]
pub struct CreateTeamInWorkspaceMemberInput {
    pub user_uid: cynic::Id,
    pub role: MembershipRole,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "CreateTeamInWorkspaceVariables"
)]
pub struct CreateTeamInWorkspace {
    #[arguments(input: $input, requestContext: $request_context)]
    pub create_team_in_workspace: CreateTeamInWorkspaceResult,
}
crate::client::define_operation! {
    create_team_in_workspace(CreateTeamInWorkspaceVariables) -> CreateTeamInWorkspace;
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum CreateTeamInWorkspaceResult {
    CreateTeamInWorkspaceOutput(CreateTeamInWorkspaceOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct CreateTeamInWorkspaceOutput {
    pub team: Team,
    pub response_context: ResponseContext,
}
