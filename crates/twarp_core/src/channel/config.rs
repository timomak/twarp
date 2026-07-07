use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::AppId;

#[derive(Debug, Deserialize, Serialize)]
pub struct ChannelConfig {
    /// The application ID for this channel.
    pub app_id: AppId,

    /// The name of the file to which logs should be written.
    pub logfile_name: Cow<'static, str>,

    /// Configuration for talking to server APIs.
    pub server_config: WarpServerConfig,
    /// Configuration for Oz/ambient agents.
    pub oz_config: OzConfig,
    // twarp: de-cloud — telemetry_config (RudderStack) deleted; unknown keys in
    // externally generated configs are ignored by serde.
    /// Configuration for statically-bundled MCP OAuth credentials.
    pub mcp_static_config: Option<McpStaticConfig>,
}

impl ChannelConfig {
    /// Removes upstream Warp service destinations from externally generated channel configs.
    ///
    /// Twarp should not inherit production Warp auth/cloud or telemetry
    /// destinations by accident. Local/test server overrides are preserved so
    /// development builds can still point at explicitly configured non-Warp services.
    pub fn without_upstream_warp_services(mut self) -> Self {
        if self.server_config.has_upstream_warp_endpoint() {
            self.server_config = WarpServerConfig::disabled();
        }
        if self.oz_config.has_upstream_warp_endpoint() {
            self.oz_config = OzConfig::disabled();
        }

        self
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WarpServerConfig {
    /// The root URL for the standard server pool.
    pub server_root_url: Cow<'static, str>,
    /// The URL for the RTC server, which serves real-time updates for Warp Drive objects.
    pub rtc_server_url: Cow<'static, str>,
    /// The URL for the session sharing server, or [`None`] if session sharing is not
    /// supported.
    pub session_sharing_server_url: Option<Cow<'static, str>>,
    /// The API key to use when making requests to Firebase Authentication endpoints.
    pub firebase_auth_api_key: Cow<'static, str>,
}

impl WarpServerConfig {
    pub fn disabled() -> Self {
        Self {
            firebase_auth_api_key: "".into(),
            // Use an IP in the IANA testing range, with the TCP discard port, to
            // black-hole accidental server traffic.
            server_root_url: "http://192.0.2.0:9".into(),
            rtc_server_url: "ws://192.0.2.0:9/graphql/v2".into(),
            session_sharing_server_url: None,
        }
    }

    fn has_upstream_warp_endpoint(&self) -> bool {
        is_upstream_warp_url(&self.server_root_url)
            || is_upstream_warp_url(&self.rtc_server_url)
            || self
                .session_sharing_server_url
                .as_deref()
                .is_some_and(is_upstream_warp_url)
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct OzConfig {
    /// Root URL for the Oz (ambient agent management) dashboard.
    pub oz_root_url: Cow<'static, str>,

    /// URL to use as the audience when issuing workload identity tokens. If [`None`], falls back
    /// to [`WarpServerConfig::server_root_url`]. This exists so the audience is not overridden
    /// when a custom server root URL is provided (e.g. an ngrok URL for local development).
    pub workload_audience_url: Option<Cow<'static, str>>,
}

impl OzConfig {
    pub fn disabled() -> Self {
        Self {
            // Use an IP in the IANA testing range, with the TCP discard port, to
            // black-hole accidental Oz traffic.
            oz_root_url: "http://192.0.2.0:9".into(),
            workload_audience_url: None,
        }
    }

    fn has_upstream_warp_endpoint(&self) -> bool {
        is_upstream_warp_url(&self.oz_root_url)
            || self
                .workload_audience_url
                .as_deref()
                .is_some_and(is_upstream_warp_url)
    }
}

fn is_upstream_warp_url(url: &str) -> bool {
    Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned))
        .is_some_and(|host| host == "warp.dev" || host.ends_with(".warp.dev"))
}

/// Configuration for statically-bundled MCP OAuth credentials.
///
/// These are credentials for OAuth providers where dynamic client registration
/// is not supported and we instead ship pre-registered client IDs and secrets.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct McpStaticConfig {
    /// Per-provider OAuth credentials.
    pub providers: Vec<McpOAuthProviderConfig>,
}

/// A single OAuth provider's credentials for MCP authentication.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct McpOAuthProviderConfig {
    /// The issuer URL of the OAuth provider (e.g. `https://github.com/login/oauth`).
    pub issuer: Cow<'static, str>,
    /// The OAuth client ID registered for this channel.
    pub client_id: Cow<'static, str>,
    /// The OAuth client secret registered for this channel.
    pub client_secret: Cow<'static, str>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_service_destinations(
        server_config: WarpServerConfig,
        oz_config: OzConfig,
    ) -> ChannelConfig {
        ChannelConfig {
            app_id: AppId::new("dev", "twarp", "TwarpLocal"),
            logfile_name: "twarp.log".into(),
            server_config,
            oz_config,
            mcp_static_config: None,
        }
    }

    fn upstream_url(scheme: &str, host_prefix: &str, path: &str) -> Cow<'static, str> {
        format!("{scheme}://{host_prefix}.{}{path}", "warp.dev").into()
    }

    #[test]
    fn disabled_server_config_uses_blackhole_destinations() {
        let server_config = WarpServerConfig::disabled();
        let oz_config = OzConfig::disabled();

        assert_eq!(server_config.server_root_url, "http://192.0.2.0:9");
        assert_eq!(server_config.rtc_server_url, "ws://192.0.2.0:9/graphql/v2");
        assert!(server_config.session_sharing_server_url.is_none());
        assert!(server_config.firebase_auth_api_key.is_empty());
        assert_eq!(oz_config.oz_root_url, "http://192.0.2.0:9");
    }

    #[test]
    fn sanitizing_generated_config_disables_upstream_warp_services() {
        let config = config_with_service_destinations(
            WarpServerConfig {
                server_root_url: upstream_url("https", "app", ""),
                rtc_server_url: upstream_url("wss", "rtc.app", "/graphql/v2"),
                session_sharing_server_url: Some(upstream_url("wss", "sessions.app", "")),
                firebase_auth_api_key: "upstream-firebase-key".into(),
            },
            OzConfig {
                oz_root_url: upstream_url("https", "oz", ""),
                workload_audience_url: Some(upstream_url("https", "app", "")),
            },
        )
        .without_upstream_warp_services();

        assert_eq!(config.server_config.server_root_url, "http://192.0.2.0:9");
        assert_eq!(
            config.server_config.rtc_server_url,
            "ws://192.0.2.0:9/graphql/v2"
        );
        assert!(config.server_config.session_sharing_server_url.is_none());
        assert!(config.server_config.firebase_auth_api_key.is_empty());
        assert_eq!(config.oz_config.oz_root_url, "http://192.0.2.0:9");
        assert!(config.oz_config.workload_audience_url.is_none());
    }

    #[test]
    fn sanitizing_generated_config_preserves_explicit_local_servers() {
        let config = config_with_service_destinations(
            WarpServerConfig {
                server_root_url: "http://localhost:8080".into(),
                rtc_server_url: "ws://localhost:8081/graphql/v2".into(),
                session_sharing_server_url: Some("ws://localhost:8082".into()),
                firebase_auth_api_key: "local-key".into(),
            },
            OzConfig {
                oz_root_url: "http://localhost:8083".into(),
                workload_audience_url: Some("http://localhost:8080".into()),
            },
        )
        .without_upstream_warp_services();

        assert_eq!(
            config.server_config.server_root_url,
            "http://localhost:8080"
        );
        assert_eq!(
            config.server_config.rtc_server_url,
            "ws://localhost:8081/graphql/v2"
        );
        assert_eq!(
            config.server_config.session_sharing_server_url.as_deref(),
            Some("ws://localhost:8082")
        );
        assert_eq!(config.server_config.firebase_auth_api_key, "local-key");
        assert_eq!(config.oz_config.oz_root_url, "http://localhost:8083");
        assert_eq!(
            config.oz_config.workload_audience_url.as_deref(),
            Some("http://localhost:8080")
        );
    }
}
