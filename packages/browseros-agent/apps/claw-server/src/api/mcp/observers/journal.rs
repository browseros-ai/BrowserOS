//! Keeps the agent's `run` scripts in the session's run journal while ultrafast skills are on,
//! so saving the session as a skill can compile it. Flow runs and scripts that may hold secrets
//! are never kept.

use crate::{
    api::mcp::dispatch::ToolObserverContext,
    clock::now_epoch_ms,
    services::flow::{
        compose::is_flow_script,
        journal::{self as run_journal, JournalEntry, RETENTION},
    },
};
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::sync::atomic::{AtomicI64, Ordering};

const SWEEP_EVERY_MS: i64 = 60 * 60 * 1000;
static LAST_SWEEP_MS: AtomicI64 = AtomicI64::new(0);

pub fn apply(context: ToolObserverContext<'_>) -> BoxFuture<'_, anyhow::Result<()>> {
    Box::pin(async move {
        let call = context.call;
        if call.tool().name != "run" || context.cancelled || call.identity.is_none() {
            return Ok(());
        }
        if !call.state.agent_settings.ultrafast_enabled() {
            return Ok(());
        }
        let Some(code) = call.raw_args.get("code").and_then(Value::as_str) else {
            return Ok(());
        };
        if is_flow_script(code) || browseros_mcp::tools::sensitive::source_may_hold_secrets(code) {
            return Ok(());
        }
        let dir = call.state.config.browserclaw_dir.clone();
        let entry = JournalEntry::from_run(
            call.dispatch_id.to_string(),
            now_epoch_ms(),
            context.duration_ms,
            code,
            context
                .result
                .structured_content
                .as_ref()
                .unwrap_or(&Value::Null),
            context.result.is_error,
        );
        let session = call.session_id.as_str().to_string();
        let now = now_epoch_ms();
        let sweep = now - LAST_SWEEP_MS.load(Ordering::Relaxed) > SWEEP_EVERY_MS;
        if sweep {
            LAST_SWEEP_MS.store(now, Ordering::Relaxed);
        }
        tokio::task::spawn_blocking(move || {
            if sweep {
                run_journal::sweep(&dir, RETENTION);
            }
            run_journal::append(&dir, &session, &entry)
        })
        .await??;
        Ok(())
    })
}
