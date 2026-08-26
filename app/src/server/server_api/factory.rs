use anyhow::{Result, anyhow};
use async_trait::async_trait;
use cynic::{MutationBuilder, QueryBuilder};
#[cfg(test)]
use mockall::automock;
use warp_graphql::mutations::delete_runner::{
    DeleteRunner, DeleteRunnerInput, DeleteRunnerResult, DeleteRunnerVariables,
};
use warp_graphql::mutations::upsert_runner::{
    UpsertRunner, UpsertRunnerInput, UpsertRunnerResult, UpsertRunnerVariables,
};
use warp_graphql::queries::get_runner::{
    GetRunner, GetRunnerResult, GetRunnerVariables, RunnerSelector,
};
use warp_graphql::queries::get_runners::{
    GetRunners, GetRunnersResult, GetRunnersVariables, Runner, RunnerSortBy,
};

use super::ServerApi;
use crate::server::graphql::{get_request_context, get_user_facing_error_message};

/// The result of upserting a runner: the resulting [`Runner`] plus whether the
/// operation updated an existing runner (vs. creating a new one).
// `upsert_runner`/`delete_runner` back CLI commands that aren't built for wasm, so
// this type is unused there while `get_runners` still powers the runner picker.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub struct UpsertedRunner {
    pub runner: Runner,
    pub is_update: bool,
}

/// Client for the Factory GraphQL surface (runner CRUD).
#[cfg_attr(test, automock)]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait FactoryClient: 'static + Send + Sync {
    /// Fetch all runners visible to the caller, optionally sorted.
    async fn get_runners(&self, sort_by: Option<RunnerSortBy>) -> Result<Vec<Runner>>;

    /// Resolve a single runner by UID or name, without fetching every
    /// accessible runner. `selector.uid` takes precedence; `selector.name` is
    /// used as a fallback if no runner matches the uid.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    async fn get_runner(&self, selector: RunnerSelector) -> Result<Runner>;

    /// Create or update a runner. `input.uid` is `None` for a create and
    /// `Some(_)` for an update; this single method backs both CLI commands.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    async fn upsert_runner(&self, input: UpsertRunnerInput) -> Result<UpsertedRunner>;

    /// Delete a runner by UID, returning the deleted UID on success.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    async fn delete_runner(&self, uid: String) -> Result<String>;
}

/// True when a GraphQL error indicates the server doesn't recognize the
/// `getRunner` query.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
fn is_missing_get_runner_query_error(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    message.contains("getRunner") && message.contains("Cannot query field")
}

/// Resolves a runner via [`FactoryClient::get_runner`], with a fallback to a
/// uid match against [`FactoryClient::get_runners`].
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub async fn get_runner_with_fallback(
    factory: &dyn FactoryClient,
    selector: RunnerSelector,
) -> Result<Runner> {
    let uid = selector.uid.clone();
    match factory.get_runner(selector).await {
        Ok(runner) => Ok(runner),
        Err(err) if is_missing_get_runner_query_error(&err) => {
            let Some(uid) = uid else {
                return Err(err);
            };
            let uid = uid.inner().to_string();
            factory
                .get_runners(None)
                .await?
                .into_iter()
                .find(|runner| runner.uid.inner() == uid)
                .ok_or_else(|| anyhow!("runner {uid} not found"))
        }
        Err(err) => Err(err),
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl FactoryClient for ServerApi {
    async fn get_runners(&self, sort_by: Option<RunnerSortBy>) -> Result<Vec<Runner>> {
        let operation = GetRunners::build(GetRunnersVariables {
            request_context: get_request_context(),
            sort_by,
        });
        let response = self.send_graphql_request(operation, None).await?;
        match response.get_runners {
            GetRunnersResult::GetRunnersOutput(output) => Ok(output.runners),
            GetRunnersResult::UserFacingError(e) => Err(anyhow!(get_user_facing_error_message(e))),
            GetRunnersResult::Unknown => Err(anyhow!("failed to list runners")),
        }
    }

    async fn get_runner(&self, selector: RunnerSelector) -> Result<Runner> {
        let operation = GetRunner::build(GetRunnerVariables {
            request_context: get_request_context(),
            selector,
        });
        let response = self.send_graphql_request(operation, None).await?;
        match response.get_runner {
            GetRunnerResult::GetRunnerOutput(output) => Ok(output.runner),
            GetRunnerResult::UserFacingError(e) => Err(anyhow!(get_user_facing_error_message(e))),
            GetRunnerResult::Unknown => Err(anyhow!("failed to resolve runner")),
        }
    }

    async fn upsert_runner(&self, input: UpsertRunnerInput) -> Result<UpsertedRunner> {
        let operation = UpsertRunner::build(UpsertRunnerVariables {
            input,
            request_context: get_request_context(),
        });
        let response = self.send_graphql_request(operation, None).await?;
        match response.upsert_runner {
            UpsertRunnerResult::UpsertRunnerOutput(output) => Ok(UpsertedRunner {
                runner: output.runner,
                is_update: output.is_update,
            }),
            UpsertRunnerResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            UpsertRunnerResult::Unknown => Err(anyhow!("failed to upsert runner")),
        }
    }

    async fn delete_runner(&self, uid: String) -> Result<String> {
        let operation = DeleteRunner::build(DeleteRunnerVariables {
            input: DeleteRunnerInput {
                uid: cynic::Id::new(uid),
            },
            request_context: get_request_context(),
        });
        let response = self.send_graphql_request(operation, None).await?;
        match response.delete_runner {
            DeleteRunnerResult::DeleteRunnerOutput(output) => {
                Ok(output.deleted_uid.inner().to_string())
            }
            DeleteRunnerResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            DeleteRunnerResult::Unknown => Err(anyhow!("failed to delete runner")),
        }
    }
}
