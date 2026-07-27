//! Audit distillation: after a successful script run, turn the child primitive
//! sequence recorded under it into a candidate helper, so a proven flow is saved
//! for reuse with no agent writing. This is uniquely enabled by the code-mode
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
        let inputs = source.matches("inputs.field").count();
        let deps = if inputs > 0 {
            format!(
                "distilled from {steps} recorded step(s); expects {inputs} input value(s) in `inputs`"
            )
        } else {
            format!("distilled from {steps} recorded step(s)")
        };
        let meta = HelperMeta {
            // Same flow hashes to the same name, so re-running overwrites rather
            // than accumulating a new candidate each time.
            name: format!("candidate-{}", short_hash(&source)),
            host,
            last_verified: now_epoch_ms(),
            agent: identity.agent.slug().to_string(),
            candidate: true,
            deps,
        };
        let dir = &context.call.state.config.browserclaw_dir;
        if let Err(error) = helpers::save_helper(dir, &meta, &source) {
            warn!(error = %error, "distilled candidate helper save failed");
        }
        // Bound accumulation: keep only the most recent candidates per host.
        helpers::prune_candidates(dir, &meta.host, helpers::MAX_CANDIDATES_PER_HOST);
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
    // Running index for value inputs, so typed values are read from `inputs`
    // rather than embedded (no credentials or personal data in a shared helper).
    let mut inputs = 0usize;
    for row in children {
        // Only distill the agent's own successful actions: skip failures (a
        // caught-and-recovered step) and primitives replayed from inside a
        // hot-loaded helper (a reuse), so a working reuse does not re-distill and
        // a failed reuse distills only the manual repair.
        if row_failed(row) || row_from_helper(row) {
            continue;
        }
        let args: Vec<Value> = row
            .args_json
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_default();
        let Some(page) = args.first().and_then(Value::as_u64) else {
            continue;
        };
        let Some(line) = emit_line(&row.tool_name, &args, &mut inputs) else {
            continue;
        };
        pages.insert(page);
        lines.push(line);
    }
    if lines.len() < 2 || pages.len() != 1 {
        return None;
    }
    let page = *pages.iter().next()?;
    let source = format!(
        "async (browser, page, inputs = {{}}) => {{\n{}\n}}",
        lines.join("\n")
    );
    Some((source, page))
}

/// Whether a child's recorded result marked it an error, read from its
/// `result_meta` summary.
fn row_failed(row: &ToolDispatchRow) -> bool {
    result_meta_flag(row, "isError")
}

/// Whether a child primitive ran inside a hot-loaded helper (a replay), tagged
/// by the script hook in `result_meta`.
fn row_from_helper(row: &ToolDispatchRow) -> bool {
    result_meta_flag(row, "fromHelper")
}

fn result_meta_flag(row: &ToolDispatchRow, key: &str) -> bool {
    row.result_meta
        .as_deref()
        .and_then(|meta| serde_json::from_str::<Value>(meta).ok())
        .and_then(|meta| meta.get(key).and_then(Value::as_bool))
        .unwrap_or(false)
}

/// Maps one recorded driving primitive to its SDK call line, `page` standing in
/// for the recorded page id. Typed values (fill/type/selectOption) are read from
/// `inputs.field<n>` rather than embedded, so a distilled helper never persists a
/// credential or personal data. Returns `None` for anything not worth replaying
/// (pure reads, page management, escape hatches).
fn emit_line(tool: &str, args: &[Value], inputs: &mut usize) -> Option<String> {
    let arg = |index: usize| {
        args.get(index)
            .map_or_else(|| "undefined".to_string(), Value::to_string)
    };
    let mut input = || {
        let field = format!("inputs.field{inputs}");
        *inputs += 1;
        field
    };
    let line = match tool {
        "nav.goto" => format!("  await browser.nav(page).goto({});", arg(1)),
        "nav.back" => "  await browser.nav(page).back();".to_string(),
        "nav.forward" => "  await browser.nav(page).forward();".to_string(),
        "nav.reload" => "  await browser.nav(page).reload();".to_string(),
        "input.click" => format!("  await browser.input(page).click({});", arg(1)),
        "input.fill" => format!("  await browser.input(page).fill({}, {});", arg(1), input()),
        "input.type" => format!("  await browser.input(page).type({});", input()),
        "input.press" => format!("  await browser.input(page).press({});", arg(1)),
        "input.hover" => format!("  await browser.input(page).hover({});", arg(1)),
        "input.selectOption" => {
            format!(
                "  await browser.input(page).selectOption({}, {});",
                arg(1),
                input()
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
        // The fill value is parameterized (inputs.field0), never embedded.
        assert_eq!(
            source,
            "async (browser, page, inputs = {}) => {\n  \
             await browser.nav(page).goto(\"https://example.com/login\");\n  \
             await browser.input(page).fill(\"e5\", inputs.field0);\n  \
             await browser.input(page).click(\"e6\");\n}"
        );
        Ok(())
    }

    #[test]
    fn drops_failed_actions_and_never_embeds_typed_values() -> anyhow::Result<()> {
        let mut failed_click = child("input.click", json!([3, "e9"]));
        // A caught-and-recovered failure must not enter the helper.
        failed_click.result_meta = Some(json!({ "isError": true }).to_string());
        let children = [
            child("nav.goto", json!([3, "https://example.com/login"])),
            failed_click,
            child("input.fill", json!([3, "e5", "s3cr3t-password"])),
            child("input.type", json!([3, "also-secret"])),
            child("input.click", json!([3, "e6"])),
        ];
        let (source, _) = distill_source(&children)
            .ok_or_else(|| anyhow::anyhow!("expected a distilled macro"))?;
        // The failed click is dropped.
        assert!(!source.contains("e9"), "failed action leaked: {source}");
        // Typed values are parameterized, never embedded.
        assert!(
            !source.contains("s3cr3t-password"),
            "secret leaked: {source}"
        );
        assert!(!source.contains("also-secret"), "secret leaked: {source}");
        assert!(source.contains("inputs.field0"));
        assert!(source.contains("inputs.field1"));
        Ok(())
    }

    #[test]
    fn excludes_helper_replays_and_distills_only_the_manual_repair() -> anyhow::Result<()> {
        let tagged = |tool: &str, args: Value, is_error: bool| {
            let mut row = child(tool, args);
            row.result_meta = Some(json!({ "isError": is_error, "fromHelper": true }).to_string());
            row
        };
        let children = [
            // A stale helper replays two fills, then its click fails:
            tagged("input.fill", json!([3, "e3", "old"]), false),
            tagged("input.fill", json!([3, "e5", "old"]), false),
            tagged("input.click", json!([3, "e6"]), true),
            // The agent repairs manually (not from a helper):
            child("input.fill", json!([3, "e3", "repaired"])),
            child("input.click", json!([3, "e7"])),
        ];
        let (source, _) = distill_source(&children)
            .ok_or_else(|| anyhow::anyhow!("expected a distilled macro"))?;
        // Only the manual repair distills; the helper's replayed actions are gone.
        assert!(
            !source.contains("e5"),
            "helper-replayed action leaked: {source}"
        );
        assert!(source.contains("e7"), "repair action missing: {source}");
        assert_eq!(source.matches("await browser").count(), 2);
        Ok(())
    }

    #[test]
    fn a_fully_replayed_reuse_does_not_distill() {
        let helper = |tool: &str, args: Value| {
            let mut row = child(tool, args);
            row.result_meta = Some(json!({ "isError": false, "fromHelper": true }).to_string());
            row
        };
        // Every action came from a helper: nothing new to learn.
        let children = [
            helper("input.fill", json!([3, "e3", "x"])),
            helper("input.click", json!([3, "e6"])),
        ];
        assert!(distill_source(&children).is_none());
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
