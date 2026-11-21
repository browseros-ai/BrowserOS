//! The engine child: this executable started a second time as `browseros_server __neo-engine
//! --config <file>`, whose body is neo-server's own `main`. The parent half supervises it (a
//! stdin lifeline, a reserved port, readiness, restart with backoff), writes its settings file
//! and speaks raw JSON-RPC to its loopback `/mcp`. Chunk `browser` fills this file, following
//! the unmerged `origin/ft/claw-server-with-neo-sdk` branch (design section 3.5).

use std::process::ExitCode;

/// Runs the engine in this process until it stops. Not built yet: the child exits 1, which the
/// parent reads as a failed start and answers by running with no browser.
pub(super) fn engine_main(_config: &std::path::Path) -> ExitCode {
    ExitCode::FAILURE
}
