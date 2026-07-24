//! Audit distillation: after a successful script run, turn the child primitive
//! sequence recorded under it into a candidate helper, so a proven flow is saved
//! for reuse with no agent writing. This is uniquely enabled by the Phase 1
//! audit hierarchy.

use crate::{
    api::mcp::dispatch::{ARBITRARY_SCRIPT_TOOLS, ToolObserver, ToolObserverContext},
    clock::now_epoch_ms,
    db::audit_log::ToolDispatchRow,
    services::helpers::{self, HelperMeta},
};
use browseros_core::{BrowserSession, PageId};
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::collections::BTreeSet;
use tracing::warn;

/// Observer that distills a successful script run into a candidate helper keyed
/// by the host of the single page the flow drove.
pub fn distill(context: ToolObserverContext<'_>) -> BoxFuture<'_, anyhow::Result<()>> {
    Box::pin(async move {
        if context.result.is_error
            || context.cancelled
            || !ARBITRARY_SCRIPT_TOOLS.contains(&context.call.tool().name)
        {
            return Ok(());
        }
        let (Some(identity), Some(session)) =
            (&context.call.identity, &context.call.browser_session)
        else {
            return Ok(());
        };
        let children = context
            .call
            .state
            .audit_log
            .dispatches_with_parent(context.call.dispatch_id.as_str())
            .await
            .unwrap_or_default();
        let Some((source, page_id)) = distill_source(&children) else {
            return Ok(());
        };
        let Some(host) = page_host(session, page_id).await else {
            return Ok(());
        };
        let steps = source.matches("await browser").count();
        let meta = HelperMeta {
            // Same flow hashes to the same name, so re-running overwrites rather
            // than accumulating a new candidate each time.
            name: format!("candidate-{}", short_hash(&source)),
            host,
            last_verified: now_epoch_ms(),
            agent: identity.agent.slug().to_string(),
            candidate: true,
            deps: format!("distilled from {steps} recorded step(s)"),
        };
        if let Err(error) =
            helpers::save_helper(&context.call.state.config.browserclaw_dir, &meta, &source)
        {
            warn!(error = %error, "distilled candidate helper save failed");
        }
        Ok(())
    })
}

const _: ToolObserver = distill;

/// Synthesizes a single-page action macro from the recorded child sequence, plus
/// the page id it drove. `None` unless at least two recognized actions all target
/// one page (multi-page replays are a follow-up).
#[must_use]
pub(crate) fn distill_source(children: &[ToolDispatchRow]) -> Option<(String, u64)> {
    let mut lines = Vec::new();
    let mut pages = BTreeSet::new();
    for row in children {
        let args: Vec<Value> = row
            .args_json
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_default();
        let Some(page) = args.first().and_then(Value::as_u64) else {
            continue;
        };
        let Some(line) = emit_line(&row.tool_name, &args) else {
            continue;
        };
        pages.insert(page);
        lines.push(line);
    }
    if lines.len() < 2 || pages.len() != 1 {
        return None;
    }
    let page = *pages.iter().next()?;
    let source = format!("async (browser, page) => {{\n{}\n}}", lines.join("\n"));
    Some((source, page))
}

/// Maps one recorded driving primitive to its SDK call line, `page` standing in
/// for the recorded page id. Returns `None` for anything not worth replaying
/// (pure reads, page management, escape hatches).
fn emit_line(tool: &str, args: &[Value]) -> Option<String> {
    let arg = |index: usize| {
        args.get(index)
            .map_or_else(|| "undefined".to_string(), Value::to_string)
    };
    let line = match tool {
        "nav.goto" => format!("  await browser.nav(page).goto({});", arg(1)),
        "nav.back" => "  await browser.nav(page).back();".to_string(),
        "nav.forward" => "  await browser.nav(page).forward();".to_string(),
        "nav.reload" => "  await browser.nav(page).reload();".to_string(),
        "input.click" => format!("  await browser.input(page).click({});", arg(1)),
        "input.fill" => format!("  await browser.input(page).fill({}, {});", arg(1), arg(2)),
        "input.type" => format!("  await browser.input(page).type({});", arg(1)),
        "input.press" => format!("  await browser.input(page).press({});", arg(1)),
        "input.hover" => format!("  await browser.input(page).hover({});", arg(1)),
        "input.selectOption" => {
            format!(
                "  await browser.input(page).selectOption({}, {});",
                arg(1),
                arg(2)
            )
        }
        "input.scroll" => format!(
            "  await browser.input(page).scroll({}, {}, {});",
            arg(1),
            arg(2),
            arg(3)
        ),
        "wait" => format!("  await browser.wait(page, {});", arg(1)),
        _ => return None,
    };
    Some(line)
}

async fn page_host(session: &BrowserSession, page_id: u64) -> Option<String> {
    let page = u32::try_from(page_id).ok()?;
    let info = session.pages.get_info(PageId(page)).await?;
    helpers::host_bucket(&info.url)
}

/// A short, deterministic id for a helper source so identical flows dedupe.
fn short_hash(source: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    format!("{:08x}", hasher.finish() & 0xffff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn child(tool: &str, args: Value) -> ToolDispatchRow {
        ToolDispatchRow {
            id: 0,
            created_at: 0,
            agent_id: String::new(),
            slug: String::new(),
            agent_label: String::new(),
            session_id: String::new(),
            tool_name: tool.to_string(),
            page_id: None,
            tab_id: None,
            target_id: None,
            url: None,
            title: None,
            args_json: Some(args.to_string()),
            result_meta: None,
            duration_ms: None,
            tool_input_token_estimate: 0,
            tool_output_token_estimate: 0,
            token_estimator_version: 0,
            dispatch_id: None,
            parent_dispatch_id: None,
            has_screenshot: false,
        }
    }

    #[test]
    fn distills_a_single_page_action_macro() -> anyhow::Result<()> {
        let children = [
            child("nav.goto", json!([3, "https://example.com/login"])),
            child("observe.snapshot", json!([3])), // pure read, skipped
            child("input.fill", json!([3, "e5", "alice"])),
            child("input.click", json!([3, "e6"])),
        ];
        let (source, page) = distill_source(&children)
            .ok_or_else(|| anyhow::anyhow!("expected a distilled macro"))?;
        assert_eq!(page, 3);
        assert_eq!(
            source,
            "async (browser, page) => {\n  \
             await browser.nav(page).goto(\"https://example.com/login\");\n  \
             await browser.input(page).fill(\"e5\", \"alice\");\n  \
             await browser.input(page).click(\"e6\");\n}"
        );
        Ok(())
    }

    #[test]
    fn skips_multi_page_or_too_short_flows() {
        // Spans two pages: no clean single-page macro.
        let multi = [
            child("input.click", json!([3, "e1"])),
            child("input.click", json!([4, "e2"])),
        ];
        assert!(distill_source(&multi).is_none());
        // Only one recognized action.
        let short = [
            child("nav.goto", json!([3, "https://example.com"])),
            child("read", json!([3, {}])),
        ];
        assert!(distill_source(&short).is_none());
    }
}
