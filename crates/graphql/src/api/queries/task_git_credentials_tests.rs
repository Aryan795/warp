use cynic::QueryBuilder;

use super::{
    TaskGitCredentials, TaskGitCredentialsInput, TaskGitCredentialsLegacy,
    TaskGitCredentialsLegacyInput, TaskGitCredentialsLegacyVariables, TaskGitCredentialsVariables,
};
use crate::request_context::{ClientContext, OsContext, RequestContext};

fn request_context() -> RequestContext {
    RequestContext {
        client_context: ClientContext { version: None },
        os_context: OsContext {
            category: None,
            linux_kernel_version: None,
            name: None,
            version: None,
        },
    }
}

#[test]
fn current_query_selects_authority_fields() {
    let operation = TaskGitCredentials::build(TaskGitCredentialsVariables {
        input: TaskGitCredentialsInput {
            task_id: cynic::Id::new("task"),
            workload_token: "token".to_string(),
        },
        request_context: request_context(),
    });

    assert!(operation.query.contains("credentials"));
    for field in [
        "id",
        "instanceUid",
        "installationUid",
        "scheme",
        "host",
        "port",
        "relativeUrlPrefix",
        "projectPaths",
        "username",
        "email",
        "token",
    ] {
        assert!(
            operation.query.contains(field),
            "current query must select {field}"
        );
    }
}

#[test]
fn legacy_query_omits_authority_fields() {
    let operation = TaskGitCredentialsLegacy::build(TaskGitCredentialsLegacyVariables {
        input: TaskGitCredentialsLegacyInput {
            task_id: cynic::Id::new("task"),
            workload_token: "token".to_string(),
        },
        request_context: request_context(),
    });

    assert!(operation.query.contains("credentials"));
    for field in [
        "id",
        "instanceUid",
        "installationUid",
        "scheme",
        "port",
        "relativeUrlPrefix",
        "projectPaths",
    ] {
        assert!(
            !operation.query.contains(field),
            "legacy query must not select {field}"
        );
    }
}
