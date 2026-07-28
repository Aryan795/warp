use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct DeleteSimpleIntegrationVariables {
    pub input: DeleteSimpleIntegrationInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct DeleteSimpleIntegrationInput {
    pub integration_type: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "DeleteSimpleIntegrationVariables"
)]
pub struct DeleteSimpleIntegration {
    #[arguments(input: $input, requestContext: $request_context)]
    pub delete_simple_integration: DeleteSimpleIntegrationResult,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct DeleteSimpleIntegrationOutput {
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum DeleteSimpleIntegrationResult {
    DeleteSimpleIntegrationOutput(DeleteSimpleIntegrationOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

crate::client::define_operation! {
    DeleteSimpleIntegration(DeleteSimpleIntegrationVariables) -> DeleteSimpleIntegration;
}
