use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation CreateTeamInWorkspace($input: CreateTeamInWorkspaceInput!, $request_context: RequestContext!) {
  createTeamInWorkspace(input: $input, requestContext: $request_context) {
    ... on CreateTeamInWorkspaceOutput {
      team {
        uid
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

/// Fields with server-side defaults (visibility, members) and the optional color
/// are omitted; the client currently only creates open, member-less teams.
#[derive(cynic::InputObject, Debug)]
pub struct CreateTeamInWorkspaceInput {
    pub workspace_uid: cynic::Id,
    pub name: String,
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
pub enum CreateTeamInWorkspaceResult {
    CreateTeamInWorkspaceOutput(CreateTeamInWorkspaceOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct CreateTeamInWorkspaceOutput {
    pub team: CreateTeamInWorkspaceTeam,
    pub response_context: ResponseContext,
}

/// Minimal selection on `Team`: the caller refetches full workspaces metadata
/// after a successful create, so only the uid is needed here.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Team")]
pub struct CreateTeamInWorkspaceTeam {
    pub uid: cynic::Id,
}
