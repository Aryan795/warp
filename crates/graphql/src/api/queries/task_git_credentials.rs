use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::schema;

/// A GraphQL query to fetch git credentials for a specific task.
///
/// This query is used by Agent Mode tasks to retrieve fresh provider credentials that
/// the driver uses to configure git and supported provider CLIs, and to refresh those
/// credentials periodically so long-running agents retain repository access.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "TaskGitCredentialsVariables")]
pub struct TaskGitCredentials {
    #[arguments(input: $input, requestContext: $request_context)]
    pub task_git_credentials: TaskGitCredentialsResult,
}

crate::client::define_operation! {
    task_git_credentials(TaskGitCredentialsVariables) -> TaskGitCredentials;
}

#[derive(cynic::QueryVariables, Debug)]
pub struct TaskGitCredentialsVariables {
    pub input: TaskGitCredentialsInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct TaskGitCredentialsInput {
    pub task_id: cynic::Id,
    pub workload_token: String,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum TaskGitCredentialsResult {
    TaskGitCredentialsOutput(TaskGitCredentialsOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct TaskGitCredentialsOutput {
    pub credentials: Vec<TaskGitCredential>,
}

/// Legacy operation for a server that has not deployed the authority fields.
pub mod legacy {
    use crate::error::UserFacingError;
    use crate::request_context::RequestContext;
    use crate::schema;

    #[derive(cynic::QueryFragment, Debug)]
    #[cynic(
        graphql_type = "RootQuery",
        variables = "TaskGitCredentialsLegacyVariables"
    )]
    pub struct TaskGitCredentialsLegacy {
        #[arguments(input: $input, requestContext: $request_context)]
        pub task_git_credentials: TaskGitCredentialsLegacyResult,
    }

    crate::client::define_operation! {
        task_git_credentials_legacy(TaskGitCredentialsLegacyVariables) -> TaskGitCredentialsLegacy;
    }

    #[derive(cynic::QueryVariables, Debug)]
    pub struct TaskGitCredentialsLegacyVariables {
        pub input: TaskGitCredentialsLegacyInput,
        pub request_context: RequestContext,
    }

    #[derive(cynic::InputObject, Debug)]
    #[cynic(graphql_type = "TaskGitCredentialsInput")]
    pub struct TaskGitCredentialsLegacyInput {
        pub task_id: cynic::Id,
        pub workload_token: String,
    }

    #[derive(cynic::InlineFragments, Debug)]
    #[cynic(graphql_type = "TaskGitCredentialsResult")]
    pub enum TaskGitCredentialsLegacyResult {
        TaskGitCredentialsOutput(TaskGitCredentialsLegacyOutput),
        UserFacingError(UserFacingError),
        #[cynic(fallback)]
        Unknown,
    }

    #[derive(cynic::QueryFragment, Debug)]
    #[cynic(graphql_type = "TaskGitCredentialsOutput")]
    pub struct TaskGitCredentialsLegacyOutput {
        pub credentials: Vec<TaskGitCredentialLegacy>,
    }

    #[derive(cynic::QueryFragment, Debug)]
    #[cynic(graphql_type = "TaskGitCredential")]
    pub struct TaskGitCredentialLegacy {
        pub token: String,
        pub username: Option<String>,
        pub email: Option<String>,
        pub host: String,
    }
}

pub use legacy::{
    TaskGitCredentialLegacy, TaskGitCredentialsLegacy, TaskGitCredentialsLegacyInput,
    TaskGitCredentialsLegacyResult, TaskGitCredentialsLegacyVariables,
};

#[derive(cynic::QueryFragment, Debug)]
pub struct TaskGitCredential {
    pub id: cynic::Id,
    pub instance_uid: Option<cynic::Id>,
    pub installation_uid: Option<cynic::Id>,
    pub scheme: String,
    pub host: String,
    pub port: Option<i32>,
    pub relative_url_prefix: String,
    pub project_paths: Vec<String>,
    pub token: String,
    pub username: Option<String>,
    pub email: Option<String>,
}

#[cfg(test)]
#[path = "task_git_credentials_tests.rs"]
mod tests;
