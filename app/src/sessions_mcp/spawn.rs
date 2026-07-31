//! twarp 26d: spawn provenance for `create_chat` (PRODUCT P#16, 22–24, 28).
//!
//! A [`SpawnOrigin`] records who created a session and how deep in a spawn
//! chain it sits. It is minted atomically by
//! [`SessionRegistry::try_reserve_spawn`](super::registry::SessionRegistry::try_reserve_spawn)
//! (cap + depth are validated under the same registry lock the origin is
//! recorded under, so racing `create_chat` calls can never exceed the cap),
//! attached to the pane's view (header provenance chip, P#22), and persisted
//! with the pane snapshot as JSON so the badge survives restore.

use serde::{Deserialize, Serialize};

/// Default cap on concurrently *running* spawned sessions (PRODUCT P#23).
/// Configurable via the `agent.sessions_mcp.spawn_cap` setting.
pub const DEFAULT_SPAWN_CAP: usize = 4;

/// Maximum allowed spawn depth (PRODUCT P#24): user- or external-created
/// sessions are depth 0/1, and a spawn whose depth would reach this value is
/// refused, so a runaway agent-spawning chain halts by construction.
pub const MAX_SPAWN_DEPTH: u8 = 3;

/// Who asked `create_chat` to spawn a session.
#[derive(Clone, Debug)]
pub enum SpawnParent {
    /// An agent inside a twarp pane (the tool's scoped session).
    InPane { session_id: String, title: String },
    /// An external MCP consumer on the token-gated listener. The label is the
    /// consumer's name; "external" when unnamed (PRODUCT P#16).
    External { label: String },
}

/// The provenance recorded on a spawned session (PRODUCT P#16, 22, 24).
/// Serialized as JSON into the pane snapshot (7m persistence payload) so the
/// header badge survives restore.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnOrigin {
    /// The creating session's id, for in-pane parents. `None` for external
    /// consumers.
    pub parent_session_id: Option<String>,
    /// Human-readable creator: the parent session's title, or the external
    /// consumer label ("external" when unnamed).
    pub parent_label: String,
    /// This session's depth in the spawn chain: 1 for a spawn from a
    /// user-created pane or an external consumer, parent depth + 1 otherwise.
    pub depth: u8,
}

impl SpawnOrigin {
    /// The short text the header provenance chip shows (P#22).
    pub fn chip_label(&self) -> String {
        match self.parent_session_id {
            Some(_) => format!("from: {}", self.parent_label),
            None => format!("external: {}", self.parent_label),
        }
    }
}

/// Why a spawn reservation was refused. Each maps to a distinct structured
/// MCP error so an agent can branch on which occurred (PRODUCT P#27).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpawnRefusal {
    /// The cap on concurrently running spawned sessions is reached (P#23,
    /// P#28). Nothing is queued; the error names the limit.
    AtCapacity { limit: usize },
    /// The spawn chain is too deep (P#24).
    DepthExceeded { depth: u8, max: u8 },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The origin must survive the pane-snapshot JSON round trip unchanged
    /// (P#22: the badge persists across restore).
    #[test]
    fn spawn_origin_json_round_trip() {
        for origin in [
            SpawnOrigin {
                parent_session_id: Some("parent-id".to_owned()),
                parent_label: "Fix the tests".to_owned(),
                depth: 2,
            },
            SpawnOrigin {
                parent_session_id: None,
                parent_label: "external".to_owned(),
                depth: 1,
            },
        ] {
            let json = serde_json::to_string(&origin).unwrap();
            let back: SpawnOrigin = serde_json::from_str(&json).unwrap();
            assert_eq!(back, origin);
        }
    }

    #[test]
    fn chip_label_distinguishes_in_pane_from_external() {
        let in_pane = SpawnOrigin {
            parent_session_id: Some("p".to_owned()),
            parent_label: "Refactor".to_owned(),
            depth: 1,
        };
        assert_eq!(in_pane.chip_label(), "from: Refactor");
        let external = SpawnOrigin {
            parent_session_id: None,
            parent_label: "fleet".to_owned(),
            depth: 1,
        };
        assert_eq!(external.chip_label(), "external: fleet");
    }
}
