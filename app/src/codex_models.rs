//! Model catalog for the Codex model dropdown.
//!
//! The Codex CLI fetches its own model list from the backend and caches it at
//! `~/.codex/models_cache.json` (written by every `codex` run, including the
//! headless app-server twarp spawns). Reading that cache keeps twarp's dropdown
//! in sync with whatever models the account can actually pick in the Codex TUI
//! — no twarp rebuild needed when OpenAI ships a new model. If the cache is
//! missing or unreadable (Codex never run, format change), the dropdown falls
//! back to [`FALLBACK_MODELS`].

use std::sync::OnceLock;

/// Known `-m` values the Codex CLI accepts, shown only when the CLI's own
/// model cache can't be read. `(slug, display name)` pairs, best first.
pub const FALLBACK_MODELS: &[(&str, &str)] = &[
    ("gpt-5.6-sol", "GPT-5.6-Sol"),
    ("gpt-5.6-terra", "GPT-5.6-Terra"),
    ("gpt-5.6-luna", "GPT-5.6-Luna"),
    ("gpt-5.5", "GPT-5.5"),
    ("gpt-5.4", "GPT-5.4"),
    ("gpt-5.4-mini", "GPT-5.4-Mini"),
];

/// One selectable model from the Codex CLI's cache.
#[derive(Debug, Clone)]
pub struct CodexModel {
    /// Model slug (e.g. `gpt-5.6-sol`), passed verbatim as `-m`/`model`.
    pub id: String,
    /// Human-readable name (e.g. "GPT-5.6-Sol") for the dropdown row.
    pub display_name: String,
}

static DISCOVERED: OnceLock<Option<Vec<CodexModel>>> = OnceLock::new();

/// The models from `~/.codex/models_cache.json`, in the CLI's own order, or
/// `None` if the cache is missing/unreadable. Read once per app run.
pub fn discovered() -> Option<&'static [CodexModel]> {
    DISCOVERED.get_or_init(load_cache).as_deref()
}

fn load_cache() -> Option<Vec<CodexModel>> {
    let home = std::env::var("HOME").ok()?;
    let path = std::path::Path::new(&home).join(".codex/models_cache.json");
    let text = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let models: Vec<CodexModel> = json
        .get("models")?
        .as_array()?
        .iter()
        .filter_map(|model| {
            // Internal models (e.g. codex-auto-review) carry visibility
            // "hide"; the TUI's picker shows only "list".
            if model.get("visibility").and_then(|v| v.as_str()) != Some("list") {
                return None;
            }
            let id = model.get("slug")?.as_str()?.to_owned();
            let display_name = model
                .get("display_name")
                .and_then(|name| name.as_str())
                .unwrap_or(&id)
                .to_owned();
            Some(CodexModel { id, display_name })
        })
        .collect();
    (!models.is_empty()).then_some(models)
}
