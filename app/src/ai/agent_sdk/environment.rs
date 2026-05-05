//! twarp 2c-d.3: cloud environments are no longer materialized client-side,
//! so the CLI environment subcommands are stubbed out and report a friendly
//! error rather than performing any work.

use warp_cli::{environment::EnvironmentCommand, GlobalOptions};
use warpui::AppContext;

const REMOVED_ERR: &str = "Warp cloud environments are no longer supported in this build of warp.";

/// Handle environment-related CLI commands. All subcommands return an error
/// since the underlying cloud-environments subsystem has been removed.
pub fn run(
    _ctx: &mut AppContext,
    _global_options: GlobalOptions,
    _command: EnvironmentCommand,
) -> anyhow::Result<()> {
    Err(anyhow::anyhow!(REMOVED_ERR))
}
