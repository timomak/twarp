//! Model catalog for the Claude Code pane's model dropdown (feature 07).
//!
//! The dropdown used to be a hardcoded alias list, which meant every new
//! Anthropic model launch needed a twarp rebuild before it could be selected.
//! This module makes the list self-updating: once per app run it fetches
//! `GET /v1/models` from the Anthropic API and caches the result, and the
//! dropdown renders whatever the account can actually see. Discovery needs an
//! API key (the `claude` CLI's subscription OAuth token is not accepted by the
//! Models API — verified 2026-07-02); without one the dropdown falls back to
//! [`FALLBACK_MODEL_ALIASES`], which the CLI resolves itself.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

/// `--model` values the `claude` CLI accepts, shown when the Models API is
/// unreachable or no API key is configured (subscription OAuth can't call it,
/// so this is the common case). `(value, display name)` pairs: the tier
/// aliases first (per `claude --help` they always track the latest model in
/// their tier), then the exact model IDs current as of 2026-07 so a specific
/// version (Opus 4.8 vs Opus 5, …) is directly selectable. The exact-ID tail
/// needs a refresh when Anthropic ships new models.
pub const FALLBACK_MODELS: &[(&str, &str)] = &[
    ("fable", "Fable (latest)"),
    ("opus", "Opus (latest)"),
    ("sonnet", "Sonnet (latest)"),
    ("haiku", "Haiku (latest)"),
    ("claude-fable-5", "Claude Fable 5"),
    ("claude-opus-5", "Claude Opus 5"),
    ("claude-opus-4-8", "Claude Opus 4.8"),
    ("claude-opus-4-7", "Claude Opus 4.7"),
    ("claude-opus-4-6", "Claude Opus 4.6"),
    ("claude-sonnet-5", "Claude Sonnet 5"),
    ("claude-sonnet-4-6", "Claude Sonnet 4.6"),
    ("claude-haiku-4-5", "Claude Haiku 4.5"),
];

/// One selectable model from `GET /v1/models`.
#[derive(Debug, Clone)]
pub struct DiscoveredModel {
    /// Full model ID (e.g. `claude-fable-5`), passed verbatim as `--model`.
    pub id: String,
    /// Human-readable name (e.g. "Claude Fable 5") for the dropdown row.
    pub display_name: String,
}

static DISCOVERED: OnceLock<Vec<DiscoveredModel>> = OnceLock::new();
static FETCH_CLAIMED: AtomicBool = AtomicBool::new(false);

/// The models discovered this app run, newest first, or `None` if discovery
/// hasn't succeeded (yet).
pub fn discovered() -> Option<&'static [DiscoveredModel]> {
    DISCOVERED.get().map(Vec::as_slice)
}

/// Claim the one-per-app-run fetch. Returns `true` for the first caller only,
/// so concurrently-opened panes don't race duplicate requests.
pub fn claim_fetch() -> bool {
    !FETCH_CLAIMED.swap(true, Ordering::SeqCst)
}

/// Store a successful fetch result. First writer wins; later calls are no-ops.
pub fn store(models: Vec<DiscoveredModel>) {
    let _ = DISCOVERED.set(models);
}

/// Fetch the account's model list from the Anthropic Models API. Best-effort:
/// any failure (no key, network, non-2xx, unexpected shape) yields `None` and
/// the dropdown keeps its alias fallback.
pub async fn fetch() -> Option<Vec<DiscoveredModel>> {
    let key = api_key()?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(15))
        .build()
        .ok()?;
    let response = client
        .get("https://api.anthropic.com/v1/models")
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        log::info!(
            "claude model discovery: /v1/models returned {}; keeping alias fallback",
            response.status()
        );
        return None;
    }
    let body: serde_json::Value = response.json().await.ok()?;
    let models: Vec<DiscoveredModel> = body
        .get("data")?
        .as_array()?
        .iter()
        .filter_map(|model| {
            let id = model.get("id")?.as_str()?.to_owned();
            let display_name = model
                .get("display_name")
                .and_then(|name| name.as_str())
                .unwrap_or(&id)
                .to_owned();
            Some(DiscoveredModel { id, display_name })
        })
        .collect();
    (!models.is_empty()).then_some(models)
}

/// Resolve an Anthropic API key: the process environment first (covers
/// terminal launches), then the `env` block of `~/.claude/settings.json`
/// (covers Finder/Dock launches, where the GUI process has no shell exports).
fn api_key() -> Option<String> {
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        let key = key.trim();
        if !key.is_empty() {
            return Some(key.to_owned());
        }
    }
    let home = std::env::var("HOME").ok()?;
    let settings = std::path::Path::new(&home).join(".claude/settings.json");
    let text = std::fs::read_to_string(settings).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let key = json.get("env")?.get("ANTHROPIC_API_KEY")?.as_str()?.trim();
    (!key.is_empty()).then(|| key.to_owned())
}
