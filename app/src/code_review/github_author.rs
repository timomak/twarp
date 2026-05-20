//! twarp 5d: GitHub commits-API lookup for Timeline author info.
//!
//! Gravatar alone returns the identicon fallback for the majority of
//! commits because most authors don't link their email to a Gravatar
//! account. To show real GitHub profile photos and usernames, this
//! module hits `GET /repos/<owner>/<repo>/commits/<sha>` per commit,
//! returns the `author.login` + `author.avatar_url` fields, and the
//! caller caches by SHA so we don't re-fetch the same commit twice
//! within a session.
//!
//! Authentication is optional but recommended — without a token the
//! GitHub API caps at 60 requests/hour, which a single Timeline page
//! load can blow through. Token resolution tries (in order):
//! 1. `GITHUB_TOKEN` env var
//! 2. `gh auth token` (GitHub CLI) — most developers have this
//! 3. unauthenticated (60/hour)
//!
//! Repos that aren't on GitHub (parsing the `origin` URL fails) skip
//! the API entirely; the caller falls back to noreply-email parsing +
//! Gravatar.

use anyhow::{anyhow, Result};
use serde::Deserialize;

const GITHUB_API_URL: &str = "https://api.github.com";

/// Subset of `GET /repos/<owner>/<repo>/commits/<sha>` response we
/// actually render.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GithubAuthor {
    pub login: String,
    pub avatar_url: String,
}

#[derive(Deserialize, Debug)]
struct CommitResponse {
    author: Option<GithubAuthor>,
}

/// `(owner, repo)` extracted from a git remote URL. Handles both
/// HTTPS (`https://github.com/owner/repo[.git][/]`) and SSH
/// (`git@github.com:owner/repo[.git]`) forms. Returns `None` for any
/// non-GitHub remote — those skip the API entirely and the caller
/// falls back to the noreply/Gravatar path.
pub fn parse_github_origin(url: &str) -> Option<(String, String)> {
    let trimmed = url.trim().trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);

    let rest = if let Some(r) = trimmed.strip_prefix("https://github.com/") {
        r
    } else if let Some(r) = trimmed.strip_prefix("http://github.com/") {
        r
    } else if let Some(r) = trimmed.strip_prefix("git@github.com:") {
        r
    } else if let Some(r) = trimmed.strip_prefix("ssh://git@github.com/") {
        r
    } else {
        return None;
    };

    // `owner/repo` — anything more (subgroups, etc.) means it's not a
    // canonical GitHub repo URL.
    let mut parts = rest.splitn(3, '/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    // GitHub usernames/orgs are `[A-Za-z0-9-]`; repo names are
    // similarly restricted plus `.` and `_`. We only reject obvious
    // garbage to avoid bad API URLs.
    if !owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    if !repo
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

/// Try to resolve a GitHub token for authenticated API requests.
/// Returns `None` if neither path produces a non-empty token; callers
/// can still hit the API unauthenticated.
#[cfg(feature = "local_fs")]
pub async fn resolve_github_token(repo_path: &std::path::Path) -> Option<String> {
    if let Ok(env_token) = std::env::var("GITHUB_TOKEN") {
        let trimmed = env_token.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    // `gh auth token` works whether or not a repo is checked out, but
    // setting the cwd matches how the user expects `gh` to behave.
    use command::r#async::Command;
    use command::Stdio;
    let output = Command::new("gh")
        .args(["auth", "token"])
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

#[cfg(not(feature = "local_fs"))]
pub async fn resolve_github_token(_repo_path: &std::path::Path) -> Option<String> {
    None
}

/// Fetch a single commit's author info. Returns `Ok(None)` for 404s
/// (unpushed commits, deleted forks, …) so the caller can cache the
/// negative and not retry. Errors out for transport / rate-limit
/// failures so the caller can log + give up rather than poison the
/// cache.
#[cfg(feature = "local_fs")]
pub async fn fetch_commit_author(
    client: &http_client::Client,
    owner: &str,
    repo: &str,
    sha: &str,
    token: Option<&str>,
) -> Result<Option<GithubAuthor>> {
    let url = format!("{GITHUB_API_URL}/repos/{owner}/{repo}/commits/{sha}");
    let mut req = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "warp-terminal")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(token) = token {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    let response = req
        .send()
        .await
        .map_err(|e| anyhow!("github commits api request failed: {e}"))?;
    let status = response.status();
    if status == 404 {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(anyhow!(
            "github commits api returned {status}: {}",
            response.text().await.unwrap_or_default()
        ));
    }
    let body: CommitResponse = response
        .json()
        .await
        .map_err(|e| anyhow!("github commits api response parse failed: {e}"))?;
    Ok(body.author)
}

#[cfg(not(feature = "local_fs"))]
pub async fn fetch_commit_author(
    _client: &http_client::Client,
    _owner: &str,
    _repo: &str,
    _sha: &str,
    _token: Option<&str>,
) -> Result<Option<GithubAuthor>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_https_url() {
        assert_eq!(
            parse_github_origin("https://github.com/owner/repo.git"),
            Some(("owner".to_string(), "repo".to_string()))
        );
    }

    #[test]
    fn parses_https_url_without_git_suffix() {
        assert_eq!(
            parse_github_origin("https://github.com/owner/repo"),
            Some(("owner".to_string(), "repo".to_string()))
        );
    }

    #[test]
    fn parses_https_url_with_trailing_slash() {
        assert_eq!(
            parse_github_origin("https://github.com/owner/repo/"),
            Some(("owner".to_string(), "repo".to_string()))
        );
    }

    #[test]
    fn parses_ssh_url() {
        assert_eq!(
            parse_github_origin("git@github.com:owner/repo.git"),
            Some(("owner".to_string(), "repo".to_string()))
        );
    }

    #[test]
    fn parses_ssh_url_without_git_suffix() {
        assert_eq!(
            parse_github_origin("git@github.com:owner/repo"),
            Some(("owner".to_string(), "repo".to_string()))
        );
    }

    #[test]
    fn parses_ssh_protocol_url() {
        assert_eq!(
            parse_github_origin("ssh://git@github.com/owner/repo.git"),
            Some(("owner".to_string(), "repo".to_string()))
        );
    }

    #[test]
    fn rejects_non_github_origin() {
        assert_eq!(
            parse_github_origin("https://gitlab.com/owner/repo.git"),
            None
        );
        assert_eq!(
            parse_github_origin("git@bitbucket.org:owner/repo.git"),
            None
        );
    }

    #[test]
    fn rejects_malformed_input() {
        assert_eq!(parse_github_origin(""), None);
        assert_eq!(parse_github_origin("https://github.com/"), None);
        assert_eq!(parse_github_origin("https://github.com/owner"), None);
        assert_eq!(parse_github_origin("https://github.com/owner/"), None);
    }

    #[test]
    fn rejects_garbage_owner_or_repo() {
        // Space in owner — not a real GitHub handle.
        assert_eq!(parse_github_origin("https://github.com/own er/repo"), None);
    }

    #[test]
    fn handles_repo_names_with_dots_underscores() {
        assert_eq!(
            parse_github_origin("https://github.com/owner/my.repo_name"),
            Some(("owner".to_string(), "my.repo_name".to_string()))
        );
    }

    #[test]
    fn ignores_extra_path_segments() {
        // Splitn(3) means a third segment is captured but ignored;
        // only owner+repo are used. This handles cases like a
        // trailing branch path that shouldn't fail parsing.
        assert_eq!(
            parse_github_origin("https://github.com/owner/repo/tree/main"),
            Some(("owner".to_string(), "repo".to_string()))
        );
    }
}
