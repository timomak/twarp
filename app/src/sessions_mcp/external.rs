//! twarp 26d: the token-gated external surface of the sessions MCP server
//! (PRODUCT P#21, 27, 30).
//!
//! A second SSE listener in the sessions MCP runtime on a fixed, configurable
//! port (default 8377), bound to 127.0.0.1 only and off by default. Every
//! request must carry `Authorization: Bearer <token>` matching the token file
//! (owner-only `0600`, under the app-support dir; generated on enable). The
//! token is re-read from disk per request, so a regenerate invalidates the old
//! token immediately (P#21). A bind failure (port conflict) is surfaced as a
//! settings-page error state, not a silent failure (P#30).

use std::{
    io,
    path::{Path, PathBuf},
};

/// Default port for the external listener. Chosen to avoid the common dev
/// server ranges; configurable via `agent.sessions_mcp.external_port`.
pub const DEFAULT_EXTERNAL_PORT: u16 = 8377;

/// The bearer-token file for the external surface: owner-only, under the
/// app-support dir.
pub fn token_file_path() -> PathBuf {
    twarp_core::paths::data_dir().join("sessions_mcp_external_token")
}

/// Read the current token, creating the file (0600) with a fresh token if it
/// does not exist yet ("generated on enable", P#21).
pub fn ensure_token(path: &Path) -> io::Result<String> {
    match std::fs::read_to_string(path) {
        Ok(token) if !token.trim().is_empty() => Ok(token.trim().to_owned()),
        Ok(_) => write_token(path),
        Err(err) if err.kind() == io::ErrorKind::NotFound => write_token(path),
        Err(err) => Err(err),
    }
}

/// Replace the token with a fresh one. The middleware re-reads the file per
/// request, so the old token is rejected from the next request on (P#21).
pub fn regenerate_token(path: &Path) -> io::Result<String> {
    write_token(path)
}

fn write_token(path: &Path) -> io::Result<String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let token = format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    std::fs::write(path, &token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(token)
}

/// Whether an `Authorization` header value grants access given the token
/// file's current contents. Pure, so the accept/reject/regenerate semantics
/// are unit-testable without a listener.
pub fn authorize(authorization: Option<&str>, token_path: &Path) -> bool {
    let Some(header) = authorization else {
        return false;
    };
    let Some(presented) = header.strip_prefix("Bearer ") else {
        return false;
    };
    let Ok(expected) = std::fs::read_to_string(token_path) else {
        return false;
    };
    let expected = expected.trim();
    !expected.is_empty() && presented.trim() == expected
}

/// The axum middleware for the external listener: reject any request whose
/// bearer token doesn't match the token file. Errors are structured JSON so
/// an external agent can branch on the code (P#27).
pub async fn require_bearer_token(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let authorization = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    if authorize(authorization.as_deref(), &token_file_path()) {
        next.run(request).await
    } else {
        (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({
                "error": {
                    "code": "unauthorized",
                    "message": "Missing or invalid bearer token for the twarp sessions external surface",
                }
            })),
        )
            .into_response()
    }
}

/// The current state of the external listener, surfaced on the settings page
/// (P#30: a port conflict is an explicit error state, never silent).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ExternalListenerState {
    #[default]
    Disabled,
    Running {
        port: u16,
    },
    /// Enabling failed (typically a port conflict); the message is shown on
    /// the settings page.
    Failed {
        port: u16,
        error: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_token_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("twarp_sessions_token_test_{name}_{}", uuid::Uuid::new_v4()))
    }

    /// P#21: a valid bearer token is accepted; missing, malformed, and wrong
    /// tokens are rejected.
    #[test]
    fn authorize_accepts_only_the_current_token() {
        let path = temp_token_path("accept");
        let token = ensure_token(&path).unwrap();
        assert!(authorize(Some(&format!("Bearer {token}")), &path));
        assert!(!authorize(None, &path));
        assert!(!authorize(Some(&token), &path)); // no Bearer prefix
        assert!(!authorize(Some("Bearer nope"), &path));
        std::fs::remove_file(&path).unwrap();
        // No token file at all: everything is rejected.
        assert!(!authorize(Some(&format!("Bearer {token}")), &path));
    }

    /// P#21: regenerate invalidates the old token immediately (the middleware
    /// re-reads the file, so no cache can serve the stale token).
    #[test]
    fn regenerate_invalidates_old_token_immediately() {
        let path = temp_token_path("regen");
        let old = ensure_token(&path).unwrap();
        let new = regenerate_token(&path).unwrap();
        assert_ne!(old, new);
        assert!(!authorize(Some(&format!("Bearer {old}")), &path));
        assert!(authorize(Some(&format!("Bearer {new}")), &path));
        std::fs::remove_file(&path).unwrap();
    }

    /// The token file is owner-only (0600).
    #[cfg(unix)]
    #[test]
    fn token_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_token_path("perms");
        ensure_token(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        std::fs::remove_file(&path).unwrap();
    }

    /// `ensure_token` is stable across calls; only `regenerate_token` mints a
    /// new one.
    #[test]
    fn ensure_token_is_idempotent() {
        let path = temp_token_path("idem");
        let first = ensure_token(&path).unwrap();
        let second = ensure_token(&path).unwrap();
        assert_eq!(first, second);
        std::fs::remove_file(&path).unwrap();
    }
}
