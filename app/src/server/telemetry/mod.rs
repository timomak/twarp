//! twarp: de-cloud — the RudderStack telemetry pipeline has been deleted.
//!
//! The `TelemetryEvent` definitions and the recording macros are kept so the
//! hundreds of call sites compile unchanged; recorded events land in the
//! bounded in-memory queue (`twarpui::telemetry`) and are dropped — nothing
//! is ever sent off-device.

pub mod context_provider;
pub mod events;
mod macros;

pub use events::*;
