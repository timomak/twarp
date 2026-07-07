//! Representation of Warp user credentials.
//!
//! twarp: de-cloud — the Firebase login/refresh-token machinery was deleted.
//! The only remaining credentials are API keys, ambient session cookies
//! (wasm), and test credentials. In practice the app runs permanently
//! logged out.
use twarp_graphql::object_permissions::OwnerType;

/// Represents the different ways a user can authenticate with Warp.
#[derive(Clone, Debug)]
pub enum Credentials {
    /// API key for direct server authentication.
    ApiKey {
        key: String,
        /// The owner type for this API key. Only set after user info is fetched from the server.
        owner_type: Option<OwnerType>,
    },
    /// Authentication derived from an ambient browser session cookie.
    SessionCookie,
    /// Test credentials used in unit tests, integration tests, and skip_login builds.
    #[cfg(any(test, feature = "integration_tests", feature = "skip_login"))]
    Test,
}

impl Credentials {
    /// Returns the API key string if this is an API key credential.
    pub fn as_api_key(&self) -> Option<&str> {
        match self {
            Credentials::ApiKey { key, .. } => Some(key),
            Credentials::SessionCookie => None,
            #[cfg(any(test, feature = "integration_tests", feature = "skip_login"))]
            Credentials::Test => None,
        }
    }

    /// Returns the owner type if this is an API key credential.
    pub fn api_key_owner_type(&self) -> Option<OwnerType> {
        match self {
            Credentials::ApiKey { owner_type, .. } => *owner_type,
            Credentials::SessionCookie => None,
            #[cfg(any(test, feature = "integration_tests", feature = "skip_login"))]
            Credentials::Test => None,
        }
    }

    /// Returns the short-lived token to use in HTTP requests to the server.
    pub fn bearer_token(&self) -> AuthToken {
        match self {
            Credentials::ApiKey { key, .. } => AuthToken::ApiKey(key.clone()),
            Credentials::SessionCookie => AuthToken::NoAuth,
            #[cfg(any(test, feature = "integration_tests", feature = "skip_login"))]
            Credentials::Test => AuthToken::NoAuth,
        }
    }
}

/// Represents different types of authentication tokens.
#[derive(Debug, Clone)]
pub enum AuthToken {
    /// API key for direct server authentication.
    ApiKey(String),
    /// No authentication token available (e.g. session cookie auth or test credentials).
    NoAuth,
}

impl AuthToken {
    /// Returns the token string to use in an Authorization header, or `None` if auth is not
    /// header-based (e.g. session cookie) or there is no auth.
    pub fn as_bearer_token(&self) -> Option<&str> {
        match self {
            AuthToken::ApiKey(key) => Some(key),
            AuthToken::NoAuth => None,
        }
    }

    /// Returns the bearer token as an owned string, or `None` if auth is not header-based.
    pub fn bearer_token(&self) -> Option<String> {
        self.as_bearer_token().map(ToOwned::to_owned)
    }
}
