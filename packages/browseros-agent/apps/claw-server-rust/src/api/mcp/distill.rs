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
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tracing::warn;

/// A distilled flow with fewer than this many actions is not worth a helper; more
/// than the ceiling signals exploration or a retry loop rather than a crisp macro.
const MIN_ACTIONS: usize = 2;
const MAX_ACTIONS: usize = 8;

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
        let Some((source, target)) = distill_source(&children) else {
            return Ok(());
        };
        let host = match target {
            DistillTarget::Host(host) => host,
            DistillTarget::Page(page_id) => match page_host(session, page_id).await {
                Some(host) => host,
                None => return Ok(()),
            },
        };
        // Quality gate: skip exploration and retry loops so only crisp, proven
        // flows become helpers.
        if !is_distillable(&source) {
            return Ok(());
        }
        let steps = source.matches("await browser").count();
        let input_count = source.matches("inputs.field").count();
        // An opened-page macro has the `(browser, inputs)` signature; a search-by-URL
        // one additionally reads a field into the navigation via encodeURIComponent.
        let opens_page = source.starts_with("async (browser, inputs");
        let is_search = opens_page && source.contains("encodeURIComponent(inputs.field");
        let inputs = build_inputs(&source, input_count);
        let description = describe(&host, steps, opens_page, is_search);
        // A search-by-URL flow gets one canonical descriptive name per host; other
        // flows dedupe across sessions by normalized shape, so the same flow
        // re-derived elsewhere supersedes rather than piling up.
        let name = if is_search {
            format!("search-{host}")
        } else {
            format!("candidate-{}", short_hash(&normalize_shape(&source)))
        };
        let dir = &context.call.state.config.browserclaw_dir;
        let session = identity.session.convo_id().as_str().to_string();
        // Hard per-session cap: a single session contributes at most one candidate
        // per host, the richest qualifying flow it produced. If it already has an
        // equal-or-richer candidate here, keep that one.
        let rank = (u8::from(opens_page), steps);
        let mine: Vec<HelperMeta> = helpers::list_helper_meta(dir, &host)
            .into_iter()
            .filter(|meta| meta.candidate && meta.session == session)
            .collect();
        if mine
            .iter()
            .any(|meta| candidate_rank(dir, &host, meta) >= rank)
        {
            return Ok(());
        }
        let meta = HelperMeta {
            name: name.clone(),
            host: host.clone(),
            last_verified: now_epoch_ms(),
            agent: identity.agent.slug().to_string(),
            candidate: true,
            opens_page,
            inputs,
            description,
            session,
        };
        if let Err(error) = helpers::save_helper(dir, &meta, &source) {
            warn!(error = %error, "distilled candidate helper save failed");
            return Ok(());
        }
        // Enforce the per-session cap: drop this session's now-superseded candidates.
        for old in mine {
            if old.name != name {
                helpers::remove_helper(dir, &host, &old.name);
            }
        }
        // Bound accumulation across sessions: keep only the most recent candidates.
        helpers::prune_candidates(dir, &host, helpers::MAX_CANDIDATES_PER_HOST);
        Ok(())
    })
}

const _: ToolObserver = distill;

/// How the distilled helper's host is resolved: directly from an opening
/// navigation URL, or from the page id the flow drove.
pub(crate) enum DistillTarget {
    Host(String),
    Page(u64),
}

/// Synthesizes a single-page action macro from the recorded child sequence, plus
/// how to resolve its host. Handles two shapes: a flow that opens its own page
/// (`newPage(url)`, common for search-by-URL) becomes
/// `async (browser, inputs) => { const page = await browser.pages.newPage(...); ...; return page; }`;
/// a flow that acts on an existing page keeps `async (browser, page, inputs) => { ... }`.
/// `None` unless at least two recognized actions target one page (multi-page
/// replays are a follow-up).
#[must_use]
pub(crate) fn distill_source(children: &[ToolDispatchRow]) -> Option<(String, DistillTarget)> {
    let mut lines = Vec::new();
    let mut pages = BTreeSet::new();
    // Running index for value inputs, so typed values (and URL query terms) are
    // read from `inputs` rather than embedded (no personal data in a shared helper).
    let mut inputs = 0usize;
    // The opening navigation, if the flow creates its own page: (url JS expression, host bucket).
    let mut opened: Option<(String, Option<String>)> = None;
    let mut new_pages = 0usize;
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
        // Never distill a flow that navigated to a credential-bearing URL: it
        // would bake an OAuth code, token, session id, or secret into a reusable
        // helper that readHelper exposes and later runs replay.
        let nav_url = match row.tool_name.as_str() {
            "pages.newPage" => args.first(),
            "nav.goto" => args.get(1),
            _ => None,
        };
        if nav_url
            .and_then(Value::as_str)
            .is_some_and(url_has_sensitive_param)
        {
            return None;
        }
        if row.tool_name == "pages.newPage" {
            new_pages += 1;
            let Some(url) = args.first().and_then(Value::as_str) else {
                continue;
            };
            opened = Some((
                parameterize_url(url, &mut inputs),
                helpers::host_bucket(url),
            ));
            continue;
        }
        let Some(page) = args.first().and_then(Value::as_u64) else {
            continue;
        };
        let Some(line) = emit_line(&row.tool_name, &args, &mut inputs) else {
            continue;
        };
        pages.insert(page);
        lines.push(line);
    }
    // At most one opened page; page-scoped actions all target a single page.
    if new_pages > 1 || pages.len() > 1 {
        return None;
    }
    let action_count = lines.len() + usize::from(opened.is_some());
    if action_count < 2 {
        return None;
    }
    match opened {
        Some((url_expr, host)) => {
            let mut body = vec![format!(
                "  const page = await browser.pages.newPage({url_expr});"
            )];
            body.extend(lines);
            body.push("  return page;".to_string());
            let source = format!(
                "async (browser, inputs = {{}}) => {{\n{}\n}}",
                body.join("\n")
            );
            let target = match host {
                Some(host) => DistillTarget::Host(host),
                None => DistillTarget::Page(*pages.iter().next()?),
            };
            Some((source, target))
        }
        None => {
            let page = *pages.iter().next()?;
            let source = format!(
                "async (browser, page, inputs = {{}}) => {{\n{}\n}}",
                lines.join("\n")
            );
            Some((source, DistillTarget::Page(page)))
        }
    }
}

/// Rewrites a URL's search-query values as `encodeURIComponent(inputs.field<n>)`
/// so a search-by-URL navigation becomes a reusable, parameterized macro without
/// baking in the specific query. Returns a JS string expression. URLs with no
/// recognized search parameter are emitted as a literal.
fn parameterize_url(url: &str, inputs: &mut usize) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return json_string(url);
    };
    let mut fragments: Vec<String> = Vec::new();
    let mut literal = format!("{base}?");
    let mut parameterized = false;
    for (index, pair) in query.split('&').enumerate() {
        if index > 0 {
            literal.push('&');
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if is_search_key(key) && !value.is_empty() {
            parameterized = true;
            literal.push_str(key);
            literal.push('=');
            fragments.push(json_string(&literal));
            literal.clear();
            fragments.push(format!("encodeURIComponent(inputs.field{inputs})"));
            *inputs += 1;
        } else {
            literal.push_str(pair);
        }
    }
    if !parameterized {
        return json_string(url);
    }
    if !literal.is_empty() {
        fragments.push(json_string(&literal));
    }
    fragments.join(" + ")
}

/// Common search-query parameter names across sites (amazon `k`, google `q`, ...).
fn is_search_key(key: &str) -> bool {
    matches!(
        key,
        "q" | "k" | "query" | "search" | "s" | "term" | "keyword" | "keywords" | "field-keywords"
    )
}

/// Whether a URL carries a credential-bearing query parameter (an OAuth code,
/// token, session id, signature, or secret) that must never be baked into a
/// reusable helper. Case-insensitive on the parameter name; over-skips rather
/// than risk persisting a secret.
fn url_has_sensitive_param(url: &str) -> bool {
    let Some((_, query)) = url.split_once('?') else {
        return false;
    };
    query.split(['&', ';']).any(|pair| {
        let key = pair
            .split('=')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        matches!(
            key.as_str(),
            "token"
                | "access_token"
                | "id_token"
                | "refresh_token"
                | "code"
                | "state"
                | "session"
                | "sid"
                | "sig"
                | "signature"
                | "auth"
                | "authorization"
                | "secret"
                | "client_secret"
                | "password"
                | "passwd"
                | "pwd"
                | "api_key"
                | "apikey"
                | "jwt"
                | "otp"
                | "nonce"
        ) || key.ends_with("_token")
            || key.ends_with("_secret")
    })
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
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
    let line = match tool {
        "nav.goto" => format!(
            "  await browser.nav(page).goto({});",
            url_arg(args, 1, inputs)
        ),
        "nav.back" => "  await browser.nav(page).back();".to_string(),
        "nav.forward" => "  await browser.nav(page).forward();".to_string(),
        "nav.reload" => "  await browser.nav(page).reload();".to_string(),
        "input.click" => format!("  await browser.input(page).click({});", arg(1)),
        "input.fill" => format!(
            "  await browser.input(page).fill({}, {});",
            arg(1),
            next_input(inputs)
        ),
        "input.type" => format!("  await browser.input(page).type({});", next_input(inputs)),
        "input.press" => format!("  await browser.input(page).press({});", arg(1)),
        "input.hover" => format!("  await browser.input(page).hover({});", arg(1)),
        "input.selectOption" => {
            format!(
                "  await browser.input(page).selectOption({}, {});",
                arg(1),
                next_input(inputs)
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

/// The next `inputs.field<n>` placeholder, advancing the counter.
fn next_input(inputs: &mut usize) -> String {
    let field = format!("inputs.field{inputs}");
    *inputs += 1;
    field
}

/// A URL argument, parameterized when it is a string with a search query, else
/// emitted as its JSON literal.
fn url_arg(args: &[Value], index: usize, inputs: &mut usize) -> String {
    match args.get(index).and_then(Value::as_str) {
        Some(url) => parameterize_url(url, inputs),
        None => args
            .get(index)
            .map_or_else(|| "undefined".to_string(), Value::to_string),
    }
}

async fn page_host(session: &BrowserSession, page_id: u64) -> Option<String> {
    let page = u32::try_from(page_id).ok()?;
    let info = session.pages.get_info(PageId(page)).await?;
    helpers::host_bucket(&info.url)
}

/// Builds the `field<n> -> description` input map from the generated source: a
/// field read into a URL via `encodeURIComponent` is a search query; the rest are
/// generic input values. Drives the documented call form.
fn build_inputs(source: &str, count: usize) -> BTreeMap<String, String> {
    let mut labeled_search = false;
    (0..count)
        .map(|index| {
            let field = format!("field{index}");
            // Only the first field read into a URL is the search query; the rest
            // are generic input values.
            let is_search =
                !labeled_search && source.contains(&format!("encodeURIComponent(inputs.{field})"));
            if is_search {
                labeled_search = true;
            }
            let desc = if is_search {
                "search query"
            } else {
                "input value"
            };
            (field, desc.to_string())
        })
        .collect()
}

/// Whether a distilled flow is worth saving as a helper: a sane action count, and
/// no more than one search navigation (multiple is a re-search loop, not a macro).
fn is_distillable(source: &str) -> bool {
    let actions = source.matches("await browser").count();
    if !(MIN_ACTIONS..=MAX_ACTIONS).contains(&actions) {
        return false;
    }
    source.matches("encodeURIComponent(inputs.field").count() <= 1
}

/// The (opensPage, action-count) rank of an existing candidate, read from its
/// source. A search helper (opensPage) outranks a passed-page one; among the
/// same kind, more actions wins.
fn candidate_rank(dir: &Path, host: &str, meta: &HelperMeta) -> (u8, usize) {
    let actions = helpers::read_helper_source(dir, host, &meta.name)
        .map(|source| source.matches("await browser").count())
        .unwrap_or(0);
    (u8::from(meta.opens_page), actions)
}

/// A structural skeleton of a distilled source: string literals (refs, URLs,
/// values) collapsed to a placeholder and input indices normalized, so the same
/// flow re-derived with different specifics hashes the same and supersedes rather
/// than accumulating.
fn normalize_shape(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars();
    while let Some(character) = chars.next() {
        if character == '"' {
            // Skip the string body; refs, URLs, and values all live in quotes.
            while let Some(inner) = chars.next() {
                match inner {
                    '\\' => {
                        chars.next();
                    }
                    '"' => break,
                    _ => {}
                }
            }
            out.push_str("\"S\"");
        } else {
            out.push(character);
        }
    }
    collapse_input_indices(&out)
}

/// Replaces `inputs.field<n>` with `inputs.fieldN`, so the number of inputs does
/// not fork the shape.
fn collapse_input_indices(source: &str) -> String {
    let needle = "inputs.field";
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(pos) = rest.find(needle) {
        out.push_str(&rest[..pos]);
        out.push_str(needle);
        out.push('N');
        rest = rest[pos + needle.len()..].trim_start_matches(|c: char| c.is_ascii_digit());
    }
    out.push_str(rest);
    out
}

/// A one-line human summary of the distilled flow for the helper's frontmatter.
fn describe(host: &str, steps: usize, opens_page: bool, is_search: bool) -> String {
    if is_search {
        format!("Opens {host} search for a query and returns the results page")
    } else if opens_page {
        format!("Opens a {host} page and returns it")
    } else {
        format!("Replays {steps} step(s) on {host}")
    }
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
        let (source, target) = distill_source(&children)
            .ok_or_else(|| anyhow::anyhow!("expected a distilled macro"))?;
        assert!(matches!(target, DistillTarget::Page(3)));
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
    fn distills_a_search_by_url_flow_into_a_parameterized_macro() -> anyhow::Result<()> {
        // The natural real-agent pattern: search by URL, then read (not type + click).
        let children = [
            child(
                "pages.newPage",
                json!(["https://www.amazon.in/s?k=wireless+earbuds"]),
            ),
            child("wait", json!([10, { "text": "results", "timeout": 20000 }])),
            child("read", json!([10, {}])), // pure read, skipped
        ];
        let (source, target) = distill_source(&children)
            .ok_or_else(|| anyhow::anyhow!("expected a distilled macro"))?;
        // Host comes straight from the navigation URL.
        assert!(matches!(&target, DistillTarget::Host(host) if host == "amazon.in"));
        // The query term is parameterized (never embedded); the macro opens its own page.
        assert!(!source.contains("wireless"), "query leaked: {source}");
        assert!(
            source.contains("async (browser, inputs = {}) =>"),
            "shape: {source}"
        );
        assert!(
            source.contains(
                "const page = await browser.pages.newPage(\"https://www.amazon.in/s?k=\" + encodeURIComponent(inputs.field0));"
            ),
            "got: {source}"
        );
        assert!(source.contains("return page;"));
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
    fn build_inputs_labels_search_query_field_and_describes_flow() {
        let search = "const page = await browser.pages.newPage(\"https://www.amazon.in/s?k=\" + encodeURIComponent(inputs.field0));";
        let inputs = build_inputs(search, 1);
        assert_eq!(
            inputs.get("field0").map(String::as_str),
            Some("search query")
        );
        assert_eq!(
            describe("amazon.in", 2, true, true),
            "Opens amazon.in search for a query and returns the results page"
        );
        // A form fill (not a URL query) is a generic input value.
        let fill = "await browser.input(page).fill(\"e5\", inputs.field0);";
        assert_eq!(
            build_inputs(fill, 1).get("field0").map(String::as_str),
            Some("input value")
        );
        assert_eq!(
            describe("example.com", 3, false, false),
            "Replays 3 step(s) on example.com"
        );
    }

    #[test]
    fn is_distillable_rejects_loops_and_oversized_flows() {
        let clean = "async (browser, inputs = {}) => {\n  const page = await browser.pages.newPage(\"x\" + encodeURIComponent(inputs.field0));\n  await browser.wait(page, {});\n}";
        assert!(is_distillable(clean));
        // A re-search loop (more than one search navigation) is rejected.
        let loop_src = "await browser a encodeURIComponent(inputs.field0) await browser b encodeURIComponent(inputs.field1)";
        assert!(!is_distillable(loop_src));
        // Too many actions signals exploration.
        assert!(!is_distillable(&"await browser x ".repeat(12)));
        // Too few is rejected.
        assert!(!is_distillable("await browser x"));
    }

    #[test]
    fn normalize_shape_is_stable_across_refs_and_values() {
        let a = "async (browser, page, inputs = {}) => {\n  await browser.input(page).fill(\"e5\", inputs.field0);\n  await browser.input(page).click(\"e6\");\n}";
        let b = "async (browser, page, inputs = {}) => {\n  await browser.input(page).fill(\"e9\", inputs.field0);\n  await browser.input(page).click(\"e42\");\n}";
        assert_eq!(normalize_shape(a), normalize_shape(b));
        // A structurally different flow differs.
        let c = "async (browser, page, inputs = {}) => {\n  await browser.input(page).click(\"e6\");\n}";
        assert_ne!(normalize_shape(a), normalize_shape(c));
    }

    #[test]
    fn build_inputs_labels_only_the_first_search_field() {
        let two_search = "newPage(\"a\" + encodeURIComponent(inputs.field0)); nav(page).goto(\"b\" + encodeURIComponent(inputs.field1));";
        let inputs = build_inputs(two_search, 2);
        assert_eq!(
            inputs.get("field0").map(String::as_str),
            Some("search query")
        );
        assert_eq!(
            inputs.get("field1").map(String::as_str),
            Some("input value")
        );
    }

    #[test]
    fn does_not_distill_a_flow_that_navigates_to_a_credentialed_url() {
        // An OAuth-callback-style newPage carries code + state; distilling would
        // bake the secret into the helper.
        let oauth = [
            child(
                "pages.newPage",
                json!(["https://example.com/callback?code=abc123&state=xyz"]),
            ),
            child("wait", json!([3, { "text": "ok", "timeout": 5000 }])),
        ];
        assert!(distill_source(&oauth).is_none());
        // A token on a nav.goto is caught too.
        let reset = [
            child(
                "nav.goto",
                json!([3, "https://example.com/reset?token=SECRET"]),
            ),
            child("input.click", json!([3, "e5"])),
        ];
        assert!(distill_source(&reset).is_none());
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
