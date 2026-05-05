use super::ServerApi;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use cynic::{MutationBuilder, QueryBuilder};

#[cfg(test)]
use mockall::automock;

use crate::server::graphql::{get_request_context, get_user_facing_error_message};
use warp_graphql::mutations::create_simple_integration::{
    CreateSimpleIntegration, CreateSimpleIntegrationOutput, CreateSimpleIntegrationResult,
    CreateSimpleIntegrationVariables, SimpleIntegrationConfig,
};
use warp_graphql::queries::get_oauth_connect_tx_status::{
    GetOAuthConnectTxStatus, GetOAuthConnectTxStatusInput, GetOAuthConnectTxStatusResult,
    GetOAuthConnectTxStatusVariables, OauthConnectTxStatus,
};
use warp_graphql::queries::get_simple_integrations::{
    SimpleIntegrations, SimpleIntegrationsInput, SimpleIntegrationsOutput,
    SimpleIntegrationsResult, SimpleIntegrationsVariables,
};
use warp_graphql::queries::user_repo_auth_status::{
    RepoInput as UserRepoAuthStatusRepoInput, UserRepoAuthStatus, UserRepoAuthStatusInput,
    UserRepoAuthStatusOutput, UserRepoAuthStatusResult, UserRepoAuthStatusVariables,
};

#[cfg(not(target_family = "wasm"))]
pub trait IntegrationsClientBounds: Send + Sync {}

#[cfg(not(target_family = "wasm"))]
impl<T: 'static + Send + Sync> IntegrationsClientBounds for T {}

#[cfg(target_family = "wasm")]
pub trait IntegrationsClientBounds {}

#[cfg(target_family = "wasm")]
impl<T: 'static> IntegrationsClientBounds for T {}

#[cfg_attr(test, automock)]
#[cfg_attr(target_family = "wasm", allow(dead_code))]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
pub trait IntegrationsClient: 'static + IntegrationsClientBounds {
    /// Checks the user's GitHub authorization status for the given repositories.
    ///
    /// Returns a list of statuses for each repo, indicating whether the user has
    /// access to the repo, and an optional auth URL for the user to authorize.
    async fn check_user_repo_auth_status(
        &self,
        repos: Vec<(String, String)>,
    ) -> Result<UserRepoAuthStatusOutput>;

    /// Creates or updates a simple integration on the server.
    ///
    /// # Arguments
    /// * `integration_type` - The type of integration (e.g. "github", "linear", "slack")
    /// * `is_update` - Whether this is an update to an existing integration
    /// * `environment_uid` - The UID of the environment to associate with this integration
    /// * `base_prompt` - Optional base prompt for the integration
    /// * `model_id` - Optional model ID for the integration
    /// * `mcp_servers_json` - Optional JSON string encoding a map[string]MCPServerConfig (ambient agent spec)
    /// * `remove_mcp_server_names` - Optional list of MCP server names to remove (applies on update)
    /// * `worker_host` - Optional worker host ID for self-hosted workers
    /// * `enabled` - Whether the integration should be enabled on creation
    #[allow(clippy::too_many_arguments)]
    async fn create_or_update_simple_integration(
        &self,
        integration_type: String,
        is_update: bool,
        environment_uid: Option<String>,
        base_prompt: Option<String>,
        model_id: Option<String>,
        mcp_servers_json: Option<String>,
        remove_mcp_server_names: Option<Vec<String>>,
        worker_host: Option<String>,
        enabled: bool,
    ) -> Result<CreateSimpleIntegrationOutput>;

    /// Lists simple integrations for a fixed set of provider slugs.
    ///
    /// The server will return one SimpleIntegration entry per requested provider,
    /// regardless of whether the connection or integration currently exists.
    async fn list_simple_integrations(
        &self,
        providers: Vec<String>,
    ) -> Result<SimpleIntegrationsOutput>;

    /// Polls the status of an OAuth connect transaction.
    ///
    /// # Arguments
    /// * `tx_id` - The transaction ID returned from create_simple_integration
    ///
    /// # Returns
    /// * `Ok(OauthConnectTxStatus)` - The current status of the transaction
    /// * `Err` - If the transaction is not found or polling fails
    async fn poll_oauth_connect_status(&self, tx_id: String) -> Result<OauthConnectTxStatus>;
}

#[cfg_attr(target_family = "wasm", async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
impl IntegrationsClient for ServerApi {
    async fn check_user_repo_auth_status(
        &self,
        repos: Vec<(String, String)>,
    ) -> Result<UserRepoAuthStatusOutput> {
        let repo_inputs: Vec<UserRepoAuthStatusRepoInput> = repos
            .into_iter()
            .map(|(owner, repo)| UserRepoAuthStatusRepoInput { owner, repo })
            .collect();

        let variables = UserRepoAuthStatusVariables {
            request_context: get_request_context(),
            input: UserRepoAuthStatusInput { repos: repo_inputs },
        };

        let operation = UserRepoAuthStatus::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.user_repo_auth_status {
            UserRepoAuthStatusResult::UserRepoAuthStatusOutput(output) => Ok(output),
            UserRepoAuthStatusResult::Unknown => Err(anyhow::anyhow!(
                "Failed to check GitHub auth status: unknown response"
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_or_update_simple_integration(
        &self,
        integration_type: String,
        is_update: bool,
        environment_uid: Option<String>,
        base_prompt: Option<String>,
        model_id: Option<String>,
        mcp_servers_json: Option<String>,
        remove_mcp_server_names: Option<Vec<String>>,
        worker_host: Option<String>,
        enabled: bool,
    ) -> Result<CreateSimpleIntegrationOutput> {
        let variables = CreateSimpleIntegrationVariables {
            config: SimpleIntegrationConfig {
                base_prompt,
                environment_uid,
                model_id,
                mcp_servers_json,
                remove_mcp_server_names,
                worker_host,
            },
            enabled,
            integration_type,
            is_update,
            request_context: get_request_context(),
        };

        let operation = CreateSimpleIntegration::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        match response.create_simple_integration {
            CreateSimpleIntegrationResult::CreateSimpleIntegrationOutput(output) => Ok(output),
            CreateSimpleIntegrationResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            CreateSimpleIntegrationResult::Unknown => {
                Err(anyhow!("Unknown error while creating integration"))
            }
        }
    }

    async fn list_simple_integrations(
        &self,
        providers: Vec<String>,
    ) -> Result<SimpleIntegrationsOutput> {
        let variables = SimpleIntegrationsVariables {
            request_context: get_request_context(),
            input: SimpleIntegrationsInput { providers },
        };

        let operation = SimpleIntegrations::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.simple_integrations {
            SimpleIntegrationsResult::SimpleIntegrationsOutput(output) => Ok(output),
            SimpleIntegrationsResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            SimpleIntegrationsResult::Unknown => {
                Err(anyhow!("Unknown error while listing simple integrations"))
            }
        }
    }

    async fn poll_oauth_connect_status(&self, tx_id: String) -> Result<OauthConnectTxStatus> {
        let variables = GetOAuthConnectTxStatusVariables {
            request_context: get_request_context(),
            input: GetOAuthConnectTxStatusInput {
                tx_id: cynic::Id::new(tx_id),
            },
        };

        let operation = GetOAuthConnectTxStatus::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.get_oauth_connect_tx_status {
            GetOAuthConnectTxStatusResult::GetOAuthConnectTxStatusOutput(output) => {
                Ok(output.status)
            }
            GetOAuthConnectTxStatusResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            GetOAuthConnectTxStatusResult::Unknown => {
                Err(anyhow!("Unknown error while polling OAuth status"))
            }
        }
    }
}
