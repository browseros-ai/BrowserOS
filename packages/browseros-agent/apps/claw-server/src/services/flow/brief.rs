//! The trace brief `save_skill` hands back on its first call: what the session actually ran,
//! which runs led to the result, where time was lost, and how to write the flow. It does the
//! mechanical analysis so the agent only makes the judgement calls.

use crate::services::run_journal::JournalEntry;
use regex::Regex;
use serde_json::{Value, json};
use std::sync::LazyLock;

const MAX_BRIEF_BYTES: usize = 16 * 1024;
const MAX_GOLDEN_CODE_BYTES: usize = 6 * 1024;
const WRITE_METHODS: &[&str] = &["click", "fill", "selectOption", "press", "check", "type"];
/// The `wait` tool's own default when a script passes no timeout.
const DEFAULT_WAIT_TIMEOUT_MS: i64 = 10_000;

/// One primitive a run made, from its child audit row.
#[derive(Clone, Debug, PartialEq)]
pub struct CallTrace {
    pub method: String,
    pub ms: i64,
    pub error: bool,
    pub wait_timeout_ms: Option<i64>,
    /// What a wait waited for, e.g. `text "Added to cart"`.
    pub waited_for: Option<String>,
}

impl CallTrace {
    /// Reads a child audit row's arguments: a JSON array whose object element holds the
    /// options, e.g. `[7, { "for": "text", "timeout": 3000, "value": "Added to cart" }]`.
    #[must_use]
    pub fn from_row(method: String, ms: i64, error: bool, args_json: Option<&str>) -> Self {
        let options = args_json
            .and_then(|args| serde_json::from_str::<Value>(args).ok())
            .and_then(|args| match args {
                Value::Array(items) => items.into_iter().find(Value::is_object),
                object @ Value::Object(_) => Some(object),
                _ => None,
            })
            .unwrap_or(Value::Null);
        let waited_for = options.get("value").map(|value| {
            let kind = options.get("for").and_then(Value::as_str).unwrap_or("for");
            format!("{kind} {value}")
        });
        Self {
            method,
            ms,
            error,
            wait_timeout_ms: options.get("timeout").and_then(Value::as_i64),
            waited_for,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RunTrace {
    pub entry: JournalEntry,
    pub calls: Vec<CallTrace>,
}

pub const TEMPLATE: &str = "Answer with save_skill { name, flow, inputs, output, effects, examples }.\n\
flow: one entry per numbered step above, { step: N, fn: \"<function source>\" }, plus commit: true on the single irreversible step of a write flow (the click that adds, submits, sends or buys).\n\
Each fn is async ({ browser, inputs, state, handBack, firstOf }) => { ... }:\n\
- state is shared across steps. Open the page in step 1 and keep it in state.page; the commit step must act only through state.page.\n\
- handBack(kind, data) stops the flow cleanly and hands the page to the agent: use not_found, wait_timeout or parse_failed.\n\
- firstOf(a, b, ...) tries strategies in order and returns the first non-empty result.\n\
- The last step's return value is the output.\n\
Rules: refs come from a snapshot taken inside the flow, never a literal like \"e12\"; check matched on every waitForSelector or waitForText and handBack when it is false; no sleep or waitForTime above 1000 ms; no generated CSS class names; reach pages by URL when the site has one.\n\
inputs and output map names to \"string\", \"number\" or \"boolean\". effects is \"read\" or \"write\".\n\
examples: at least 3 input objects. The first is this session's own input; add 2 more realistic ones (other products, other names) so the flow is checked beyond the one page it was written from. Writes are never performed while verifying.";

static LITERAL_REF: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"['"`](e\d+)['"`]"#).ok());
static VALUE_TOKEN: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"\be\d+\b|https?://[^\s"'`)]+"#).ok());
static OPEN_URL: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"(?:browser\.open|\.goto)\(\s*['"`]([^'"`]+)['"`]"#).ok());

fn writes_in(code: &str) -> Vec<&'static str> {
    WRITE_METHODS
        .iter()
        .filter(|method| code.contains(&format!(".{method}(")))
        .copied()
        .collect()
}

/// Which runs fed the result: a run is golden when a later run used a ref or URL it returned,
/// or when it holds the session's last write.
fn golden_runs(runs: &[RunTrace]) -> Vec<bool> {
    let last_write = runs
        .iter()
        .rposition(|run| run.entry.ok && !writes_in(&run.entry.code).is_empty());
    runs.iter()
        .enumerate()
        .map(|(index, run)| {
            if Some(index) == last_write {
                return true;
            }
            let produced: Vec<String> = run
                .entry
                .value
                .as_deref()
                .zip(VALUE_TOKEN.as_ref())
                .map(|(value, token)| {
                    token
                        .find_iter(value)
                        .map(|m| m.as_str().to_string())
                        .collect()
                })
                .unwrap_or_default();
            run.entry.ok
                && runs[index + 1..].iter().any(|later| {
                    produced
                        .iter()
                        .any(|token| later.entry.code.contains(token.as_str()))
                })
        })
        .collect()
}

fn hints(runs: &[RunTrace], golden: &[bool]) -> Vec<String> {
    let mut hints = Vec::new();
    for run in runs {
        for call in &run.calls {
            if !call.method.contains("wait") {
                continue;
            }
            let timeout = call.wait_timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS);
            if call.ms + 150 >= timeout {
                let what = call
                    .waited_for
                    .as_deref()
                    .map_or_else(String::new, |what| format!(" for {what}"));
                hints.push(format!(
                    "a {} call{what} ran for {} ms, its full timeout, so it most likely never matched: wait on something that does appear instead",
                    call.method, call.ms
                ));
            }
        }
    }
    for (run, is_golden) in runs.iter().zip(golden) {
        if !is_golden {
            continue;
        }
        let literals = LITERAL_REF
            .as_ref()
            .map(|re| re.captures_iter(&run.entry.code));
        for literal in literals.into_iter().flatten() {
            hints.push(format!(
                "ref {} is written as a literal: find the element again from a snapshot inside the flow",
                &literal[1]
            ));
        }
        let urls = OPEN_URL
            .as_ref()
            .map(|re| re.captures_iter(&run.entry.code));
        for url in urls.into_iter().flatten() {
            hints.push(format!(
                "the session reached {} by URL: build it from the inputs in step 1",
                &url[1]
            ));
        }
    }
    if let Some(run) = runs
        .iter()
        .rev()
        .find(|run| run.entry.ok && !writes_in(&run.entry.code).is_empty())
    {
        hints.push(format!(
            "the last write was {}: if it adds, submits, sends or buys something, put it alone in the step marked commit: true",
            writes_in(&run.entry.code).join(", ")
        ));
    }
    hints.dedup();
    hints
}

fn cut(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n/* ... cut ... */", &text[..end])
}

/// Builds the brief. `goal` is the session's name when it has one.
#[must_use]
pub fn build(goal: Option<&str>, steps: &[String], runs: &[RunTrace]) -> Value {
    let golden = golden_runs(runs);
    let mut run_values: Vec<Value> = runs
        .iter()
        .zip(&golden)
        .enumerate()
        .map(|(index, (run, is_golden))| {
            let calls: Vec<Value> = run
                .calls
                .iter()
                .map(|call| json!({ "method": call.method, "ms": call.ms, "error": call.error }))
                .collect();
            let mut value = json!({
                "i": index + 1,
                "ok": run.entry.ok,
                "ms": run.entry.duration_ms,
                "tag": if *is_golden { "golden" } else { "explore" },
                "calls": calls,
            });
            if *is_golden {
                value["code"] = json!(cut(&run.entry.code, MAX_GOLDEN_CODE_BYTES));
                if let Some(returned) = &run.entry.value {
                    value["returned"] = json!(cut(returned, 1024));
                }
            } else if let Some(error) = &run.entry.error {
                value["error"] = json!(cut(error, 300));
            }
            value
        })
        .collect();
    let numbered: Vec<Value> = steps
        .iter()
        .enumerate()
        .map(|(index, text)| json!({ "step": index + 1, "text": text }))
        .collect();
    let mut brief = json!({
        "goal": goal,
        "steps": numbered,
        "journal": if runs.is_empty() { "missing" } else { "ok" },
        "runs": run_values,
        "hints": hints(runs, &golden),
        "template": TEMPLATE,
    });
    // Keep under the cap by dropping explore runs first, oldest first.
    while brief.to_string().len() > MAX_BRIEF_BYTES {
        let Some(index) = run_values
            .iter()
            .position(|run| run["tag"] == "explore")
            .or_else(|| (!run_values.is_empty()).then_some(0))
        else {
            break;
        };
        run_values.remove(index);
        brief["runs"] = json!(run_values);
    }
    brief
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(code: &str, value: Option<&str>, calls: Vec<CallTrace>) -> RunTrace {
        RunTrace {
            entry: JournalEntry {
                dispatch_id: "d".into(),
                at: 0,
                duration_ms: 100,
                ok: true,
                code: code.into(),
                value: value.map(str::to_string),
                error: None,
                logs: None,
            },
            calls,
        }
    }

    #[test]
    fn the_brief_tags_golden_runs_and_points_at_lost_time() {
        let runs = vec![
            run(
                "const p = await browser.open('http://shop/s?k=x'); return p.id",
                Some("7"),
                vec![],
            ),
            run(
                "const p = browser.page(7); const s = await p.snapshot(); return s.text",
                Some("- button \"Add to cart\" [ref=e182]"),
                vec![],
            ),
            run("return 'poking around'", None, vec![]),
            run(
                "const p = browser.page(7); await p.click('e182'); return await p.waitForText('Added to cart')",
                Some("{\"matched\":false}"),
                vec![CallTrace::from_row(
                    "wait".into(),
                    3_004,
                    false,
                    Some(r#"[7,{"for":"text","timeout":3000,"value":"Added to cart"}]"#),
                )],
            ),
        ];
        let brief = build(Some("shop add"), &["Open".into(), "Add".into()], &runs);
        let tags: Vec<&str> = brief["runs"]
            .as_array()
            .map(|runs| runs.iter().filter_map(|run| run["tag"].as_str()).collect())
            .unwrap_or_default();
        assert_eq!(tags, ["explore", "golden", "explore", "golden"]);
        let hints = brief["hints"].to_string();
        assert!(hints.contains("for text \\\"Added to cart\\\" ran for 3004 ms"));
        assert!(hints.contains("ref e182 is written as a literal"));
        assert!(hints.contains("last write was click"));
        assert_eq!(brief["journal"], "ok");
        assert_eq!(brief["steps"][1]["step"], 2);
    }

    #[test]
    fn an_empty_journal_still_explains_the_format() {
        let brief = build(None, &["Open".into()], &[]);
        assert_eq!(brief["journal"], "missing");
        assert!(
            brief["template"]
                .as_str()
                .is_some_and(|t| t.contains("commit: true"))
        );
    }

    #[test]
    fn the_brief_stays_under_its_cap() {
        let runs: Vec<RunTrace> = (0..40)
            .map(|_| run(&"x".repeat(2_000), None, vec![]))
            .collect();
        assert!(build(None, &[], &runs).to_string().len() <= MAX_BRIEF_BYTES);
    }
}
