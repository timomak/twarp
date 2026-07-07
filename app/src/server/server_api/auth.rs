// twarp: de-cloud (2b) — the Firebase auth client was deleted. Everything that
// exchanged, refreshed, or minted Firebase tokens (fetch_user, anonymous-user
// creation, custom-token minting, the OAuth2 device-auth flow, and the
// warp-server login URL builders) is gone. The remaining `AuthClient` surface
// serves API-key/test credentials and the user-settings/API-key GraphQL calls
// that other kept code still references.

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use cynic::{MutationBuilder, QueryBuilder};
use instant::Duration;
#[cfg(test)]
use mockall::{automock, predicate::*};
use thiserror::Error;
use twarp_core::errors::{AnyhowErrorExt, ErrorExt};
use twarp_graphql::mutations::expire_api_key::{
    ExpireApiKey, ExpireApiKeyResult, ExpireApiKeyVariables,
};
use twarp_graphql::mutations::generate_api_key::{
    GenerateApiKey, GenerateApiKeyInput, GenerateApiKeyResult, GenerateApiKeyVariables,
};
use twarp_graphql::mutations::update_user_settings::{
    UpdateUserSettings, UpdateUserSettingsInput, UpdateUserSettingsResult,
    UpdateUserSettingsVariables,
};
use twarp_graphql::queries::api_keys::{
    ApiKeyProperties, ApiKeyPropertiesResult, ApiKeys, ApiKeysVariables,
};
use twarp_graphql::queries::get_conversation_usage::{
    ConversationUsage, GetConversationUsage, GetConversationUsageVariables, UserResult,
};
use twarp_graphql::queries::get_user_settings::{GetUserSettings, GetUserSettingsVariables};

use crate::auth::credentials::AuthToken;
use crate::server::graphql::{get_request_context, get_user_facing_error_message};
use crate::server::ids::ApiKeyUid;
use crate::server::server_api::register_error;
use crate::settings::PrivacySettingsSnapshot;

use super::ServerApi;

/// Header key for the ambient workload token attached to multi-agent requests.
pub const AMBIENT_WORKLOAD_TOKEN_HEADER: &str = "X-Warp-Ambient-Workload-Token";

/// Header key for the cloud agent task ID attached to requests from ambient agents.
pub const CLOUD_AGENT_ID_HEADER: &str = "X-Warp-Cloud-Agent-ID";

/// Duration for which the ambient workload token is valid (3 hours).
const AMBIENT_WORKLOAD_TOKEN_DURATION: Duration = Duration::from_secs(3 * 60 * 60);

/// User settings that are currently 'synced' (e.g. stored server-side) on a per-user basis.
#[derive(Copy, Clone, Debug, Default)]
pub struct SyncedUserSettings {
    pub is_cloud_conversation_storage_enabled: bool,
}

#[cfg_attr(test, automock)]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait AuthClient: 'static + Send + Sync {
    /// Returns the auth token for the current credentials, if any.
    /// twarp: de-cloud — Firebase token refresh is gone; API keys are returned
    /// as-is and all other credential types carry no Authorization header.
    async fn get_or_refresh_access_token(&self) -> Result<AuthToken>;

    /// Upon success, returns an `Option` containing the user's settings retrieved from the server,
    /// if any. The user may not have server-side settings if they onboarded prior to the launch
    /// of telemetry opt-out, have not logged in since the launch, and have never changed defaults
    /// for any of the settings in [`SyncedUserSettings`]. If the fetched settings object exists
    /// but is missing required fields, or if the request itself failed, returns an error.
    async fn get_user_settings(&self) -> Result<Option<SyncedUserSettings>>;

    /// Returns conversation usage history for the current user over the past n days.
    /// If last_updated_end_timestamp is provided, only conversations with
    /// lastUpdated earlier than this timestamp are returned.
    async fn get_conversation_usage_history(
        &self,
        days: Option<i32>,
        limit: Option<i32>,
        last_updated_end_timestamp: Option<twarp_graphql::scalars::Time>,
    ) -> Result<Vec<ConversationUsage>>;

    async fn set_is_cloud_conversation_storage_enabled(&self, value: bool) -> Result<()>;

    /// Sends a request to update the user's settings on the server with values contained in the
    /// given `settings_snapshot`.
    async fn update_user_settings(&self, settings_snapshot: PrivacySettingsSnapshot) -> Result<()>;

    // API Keys
    async fn list_api_keys(&self) -> Result<Vec<ApiKeyProperties>>;

    async fn create_api_key(
        &self,
        name: String,
        team_id: Option<cynic::Id>,
        expires_at: Option<twarp_graphql::scalars::Time>,
    ) -> Result<GenerateApiKeyResult>;

    async fn expire_api_key(&self, key_uid: &ApiKeyUid) -> Result<ExpireApiKeyResult>;

    /// Returns a cached ambient workload token, or issues a new one if not present or expired.
    ///
    /// Returns `Ok(None)` if not running in an isolation platform (e.g., Namespace) or on WASM.
    async fn get_or_create_ambient_workload_token(&self) -> Result<Option<String>>;
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl AuthClient for ServerApi {
    async fn get_or_refresh_access_token(&self) -> Result<AuthToken> {
        if cfg!(feature = "skip_login") {
            bail!("skip_login enabled; failing all authenticated requests");
        }

        let Some(credentials) = self.auth_state.credentials() else {
            bail!("Attempted to retrieve access token when user is logged out");
        };

        Ok(credentials.bearer_token())
    }

    async fn get_user_settings(&self) -> Result<Option<SyncedUserSettings>> {
        let variables = GetUserSettingsVariables {
            request_context: get_request_context(),
        };
        let operation = GetUserSettings::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.user {
            twarp_graphql::queries::get_user_settings::UserResult::UserOutput(user_output) => {
                match user_output.user.settings {
                    Some(user_settings) => Ok(Some(SyncedUserSettings {
                        is_cloud_conversation_storage_enabled: user_settings
                            .is_cloud_conversation_storage_enabled,
                    })),
                    None => Ok(None),
                }
            }
            twarp_graphql::queries::get_user_settings::UserResult::Unknown => {
                Err(anyhow!("Unable to fetch user settings"))
            }
        }
    }

    // Returns a history of the current user's conversation usage over the past n days.
    async fn get_conversation_usage_history(
        &self,
        days: Option<i32>,
        limit: Option<i32>,
        last_updated_end_timestamp: Option<twarp_graphql::scalars::Time>,
    ) -> Result<Vec<ConversationUsage>> {
        let operation = GetConversationUsage::build(GetConversationUsageVariables {
            request_context: get_request_context(),
            days,
            limit,
            last_updated_end_timestamp,
        });
        let response = self.send_graphql_request(operation, None).await?;
        match response.user {
            UserResult::UserOutput(out) => Ok(out.user.conversation_usage),
            UserResult::Unknown => Err(anyhow!("Unable to fetch conversation usage")),
        }
    }

    async fn set_is_cloud_conversation_storage_enabled(&self, value: bool) -> Result<()> {
        let variables = UpdateUserSettingsVariables {
            input: UpdateUserSettingsInput {
                cloud_conversation_storage_enabled: Some(value),
                ..Default::default()
            },
            request_context: get_request_context(),
        };

        let operation = UpdateUserSettings::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .update_user_settings;

        match result {
            UpdateUserSettingsResult::UpdateUserSettingsOutput(_) => Ok(()),
            UpdateUserSettingsResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            UpdateUserSettingsResult::Unknown => {
                Err(anyhow!("failed to set cloud conversation storage enabled"))
            }
        }
    }

    async fn update_user_settings(&self, settings_snapshot: PrivacySettingsSnapshot) -> Result<()> {
        let variables = UpdateUserSettingsVariables {
            input: UpdateUserSettingsInput {
                // twarp: de-cloud — telemetry deleted; never touch the server-side flag.
                telemetry_enabled: None,
                cloud_conversation_storage_enabled: settings_snapshot
                    .cloud_conversation_storage_enabled(),
            },
            request_context: get_request_context(),
        };

        let operation = UpdateUserSettings::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .update_user_settings;

        match result {
            UpdateUserSettingsResult::UpdateUserSettingsOutput(_) => Ok(()),
            UpdateUserSettingsResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            UpdateUserSettingsResult::Unknown => Err(anyhow!("failed to update user settings")),
        }
    }

    // API Keys
    async fn list_api_keys(&self) -> Result<Vec<ApiKeyProperties>> {
        let variables = ApiKeysVariables {
            request_context: get_request_context(),
        };
        let operation = ApiKeys::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        match response.api_keys {
            ApiKeyPropertiesResult::ApiKeyPropertiesOutput(output) => Ok(output.api_keys),
            ApiKeyPropertiesResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            ApiKeyPropertiesResult::Unknown => Err(anyhow!("failed to fetch API keys")),
        }
    }

    async fn create_api_key(
        &self,
        name: String,
        team_id: Option<cynic::Id>,
        expires_at: Option<twarp_graphql::scalars::Time>,
    ) -> Result<GenerateApiKeyResult> {
        let variables = GenerateApiKeyVariables {
            input: GenerateApiKeyInput {
                name,
                team_id,
                expires_at,
            },
            request_context: get_request_context(),
        };
        let operation = GenerateApiKey::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        Ok(response.generate_api_key)
    }
    async fn expire_api_key(&self, key_uid: &ApiKeyUid) -> Result<ExpireApiKeyResult> {
        let variables = ExpireApiKeyVariables {
            key_uid: key_uid.into(),
            request_context: get_request_context(),
        };
        let op = ExpireApiKey::build(variables);
        let res = self.send_graphql_request(op, None).await?;
        Ok(res.expire_api_key)
    }

    async fn get_or_create_ambient_workload_token(&self) -> Result<Option<String>> {
        if cfg!(target_family = "wasm") {
            return Ok(None);
        }

        // Check if we have a cached token that's still valid (with 5 minute buffer).
        // Tokens without an expiration time are always considered valid.
        {
            let cached = self.ambient_workload_token.lock();
            if let Some(ref token) = *cached {
                let is_valid = token.expires_at.is_none_or(|expires_at| {
                    chrono::Utc::now() + chrono::Duration::minutes(5) < expires_at
                });
                if is_valid {
                    return Ok(Some(token.token.clone()));
                }
            }
        }

        // Issue a new token.
        let workload_token = match twarp_isolation_platform::issue_workload_token(Some(
            AMBIENT_WORKLOAD_TOKEN_DURATION,
        ))
        .await
        {
            Ok(token) => token,
            Err(twarp_isolation_platform::IsolationPlatformError::NoIsolationPlatformDetected) => {
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        };

        let token_str = workload_token.token.clone();

        {
            let mut cached = self.ambient_workload_token.lock();
            *cached = Some(workload_token);
        }

        Ok(Some(token_str))
    }
}

#[derive(Error, Debug)]
/// Error type for authentication failures.
/// twarp: de-cloud — trimmed to the shape the sync queue relies on (it stops
/// dequeueing when any error in a chain is a `UserAuthenticationError`).
pub enum UserAuthenticationError {
    /// The server denied the request's credentials.
    #[error("authentication was denied: {0}")]
    DeniedAccessToken(String),
    #[error("unexpected error occurred during authentication: {0:#}")]
    Unexpected(#[from] anyhow::Error),
}

impl ErrorExt for UserAuthenticationError {
    fn is_actionable(&self) -> bool {
        match self {
            UserAuthenticationError::DeniedAccessToken(err) => {
                // If a request to our server failed because the user's credentials
                // were rejected, there's no value in reporting this back to us.
                log::info!("ignoring denied access token error: {err:#}");
                false
            }
            UserAuthenticationError::Unexpected(err) => err.is_actionable(),
        }
    }
}
register_error!(UserAuthenticationError);
