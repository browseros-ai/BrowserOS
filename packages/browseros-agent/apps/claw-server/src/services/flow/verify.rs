//! The gates a submitted flow must pass before it is marked verified. Each exists because a
//! hand-written flow broke on a real page: an unmatched wait that cost 8 to 10 s, a parser
//! written from one product that failed on the next, a literal ref that was stale a day later.

use super::{
    FlowExec, check_inputs,
    compose::{Mode, SOFT_DEADLINE_MS},
    outcome::Outcome,
    store::{Effects, FlowDoc, MAX_STEP_BYTES},
};
use regex::Regex;
use serde_json::{Map, Value};
use std::{
    sync::LazyLock,
    time::{Duration, Instant},
};

pub const MAX_SUBMITS: i64 = 3;
const MIN_EXAMPLES: usize = 3;
const MAX_PAUSE_MS: u64 = 1_000;
/// The whole verification has to fit inside one agent tool call.
const TOTAL_BUDGET: Duration = Duration::from_secs(60);
const WRITE_METHODS: &str =
    "click|fill|selectOption|check|uncheck|press|type|insertText|upload|drag|clickAt|typeAt|dragAt";

static LITERAL_REF: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"['"`]e\d+['"`]"#).ok());
static PAUSE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?:\bsleep|\.waitForTime)\(\s*(\d+)|setTimeout\([^,()]*,\s*(\d+)").ok()
});
static BARE_WAIT: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*await\s+[\w.$\[\]]+\.waitFor(?:Selector|Text)\(").ok());
static GENERATED_CLASS: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"\.(?:css|sc|jsx|emotion)-[a-z0-9]{5,}|\.[a-z]{1,3}-([a-z0-9]{4,})\b").ok()
});
static WRITE_CALL: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(&format!(r"([\w.$\]\[]+)\s*\.\s*(?:{WRITE_METHODS})\s*\(")).ok());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Static,
    Parse,
    Replay,
    Variety,
    Budget,
}

impl Gate {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Static => "G1 static",
            Self::Parse => "G1 parse",
            Self::Replay => "G2 replay",
            Self::Variety => "G3 variety",
            Self::Budget => "G5 budget",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub gate: Gate,
    pub example: Option<Map<String, Value>>,
    pub step: Option<u32>,
    pub message: String,
    pub view: Option<String>,
}

impl Failure {
    fn new(gate: Gate, step: Option<u32>, message: impl Into<String>) -> Self {
        Self {
            gate,
            example: None,
            step,
            message: message.into(),
            view: None,
        }
    }

    /// What the agent reads to fix the flow.
    #[must_use]
    pub fn describe(&self) -> String {
        let mut out = format!("failing {}", self.gate.label());
        if let Some(example) = &self.example {
            out.push_str(&format!(" on {}", Value::Object(example.clone())));
        }
        if let Some(step) = self.step {
            out.push_str(&format!(" at step {step}"));
        }
        out.push_str(&format!(": {}", self.message));
        if let Some(view) = &self.view {
            out.push_str("\nThe page at that point:\n");
            out.push_str(view);
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pass {
    pub runs: usize,
    pub p50_ms: i64,
}

/// G1: rules a flow must follow before it is worth running.
pub fn static_check(doc: &FlowDoc, labels: &[String]) -> Result<(), Failure> {
    let fail = |step: Option<u32>, message: String| Err(Failure::new(Gate::Static, step, message));
    if doc.steps.is_empty() {
        return fail(None, "the flow has no steps".to_string());
    }
    let expected: Vec<u32> = if labels.is_empty() {
        (1..=u32::try_from(doc.steps.len()).unwrap_or(u32::MAX)).collect()
    } else {
        (1..=u32::try_from(labels.len()).unwrap_or(u32::MAX)).collect()
    };
    let numbers: Vec<u32> = doc.steps.iter().map(|step| step.n).collect();
    if numbers != expected {
        return fail(
            None,
            format!(
                "the flow must have exactly one step per numbered step of the skill ({expected:?}), got {numbers:?}"
            ),
        );
    }
    let commits: Vec<u32> = doc.steps.iter().filter(|s| s.commit).map(|s| s.n).collect();
    match (doc.meta.effects, commits.len()) {
        (Effects::Write, 1) | (Effects::Read, 0) => {}
        (Effects::Write, _) => {
            return fail(
                None,
                format!("a write flow marks exactly one step commit: true, got {commits:?}"),
            );
        }
        (Effects::Read, _) => {
            return fail(
                None,
                "a read flow has no commit step; use effects: \"write\" if it changes something"
                    .to_string(),
            );
        }
    }
    if doc.meta.examples.len() < MIN_EXAMPLES {
        return fail(
            None,
            format!(
                "give at least {MIN_EXAMPLES} examples (this session's input plus {} more), got {}",
                MIN_EXAMPLES - 1,
                doc.meta.examples.len()
            ),
        );
    }
    for example in &doc.meta.examples {
        if let Err(message) = check_inputs(doc, &Value::Object(example.input.clone())) {
            return fail(
                None,
                format!(
                    "example {}: {message}",
                    Value::Object(example.input.clone())
                ),
            );
        }
    }
    if !doc
        .steps
        .iter()
        .any(|step| step.src.contains("state.page =") || step.src.contains("state.page="))
    {
        return fail(
            None,
            "keep the working page in state.page (state.page = await browser.open(...))"
                .to_string(),
        );
    }
    for step in &doc.steps {
        check_step(step.n, &step.src, step.commit)?;
    }
    Ok(())
}

fn check_step(n: u32, src: &str, commit: bool) -> Result<(), Failure> {
    let fail = |message: String| Err(Failure::new(Gate::Static, Some(n), message));
    if src.len() > MAX_STEP_BYTES {
        return fail(format!("the step is over {MAX_STEP_BYTES} bytes"));
    }
    if let Some(found) = LITERAL_REF.as_ref().and_then(|re| re.find(src)) {
        return fail(format!(
            "{} is a literal ref; refs change between page loads, so find the element from a snapshot inside the flow",
            found.as_str()
        ));
    }
    for captures in PAUSE
        .as_ref()
        .into_iter()
        .flat_map(|re| re.captures_iter(src))
    {
        let ms = captures
            .get(1)
            .or_else(|| captures.get(2))
            .and_then(|m| m.as_str().parse::<u64>().ok())
            .unwrap_or(0);
        if ms > MAX_PAUSE_MS {
            return fail(format!(
                "a fixed pause of {ms} ms; wait on the thing itself with waitForSelector or waitForText instead"
            ));
        }
    }
    if BARE_WAIT.as_ref().is_some_and(|re| re.is_match(src)) {
        return fail(
            "a waitForSelector or waitForText result is ignored; it does not throw on timeout, so check matched and handBack when it is false".to_string(),
        );
    }
    for captures in GENERATED_CLASS
        .as_ref()
        .into_iter()
        .flat_map(|re| re.captures_iter(src))
    {
        let generated = match captures.get(1) {
            None => true,
            Some(suffix) => {
                let suffix = suffix.as_str();
                suffix.chars().any(|c| c.is_ascii_digit())
                    && suffix.chars().any(|c| c.is_ascii_alphabetic())
            }
        };
        if generated {
            return fail(format!(
                "{} looks like a generated CSS class, which changes between deploys; find the element by role and name from a snapshot",
                captures.get(0).map_or("", |m| m.as_str())
            ));
        }
    }
    if commit {
        for captures in WRITE_CALL
            .as_ref()
            .into_iter()
            .flat_map(|re| re.captures_iter(src))
        {
            let receiver = captures.get(1).map_or("", |m| m.as_str());
            if receiver != "state.page" {
                return fail(format!(
                    "the commit step acts through {receiver}; it must act only through state.page so verification can stop before the write"
                ));
            }
        }
    }
    Ok(())
}

fn type_matches(kind: &str, value: &Value) -> bool {
    match kind {
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        _ => value.is_string(),
    }
}

fn check_output(doc: &FlowDoc, output: &Value) -> Result<(), String> {
    if doc.meta.output.is_empty() {
        return Ok(());
    }
    let Some(map) = output.as_object() else {
        return Err(format!(
            "the output should be an object with {:?}, got {output}",
            doc.meta.output.keys().collect::<Vec<_>>()
        ));
    };
    for (key, kind) in &doc.meta.output {
        match map.get(key) {
            Some(value) if type_matches(kind, value) => {}
            Some(value) => return Err(format!("output `{key}` should be a {kind}, got {value}")),
            None => return Err(format!("output `{key}` is missing")),
        }
    }
    Ok(())
}

/// Runs every gate. Static rules first, then each example in the browser.
pub async fn verify(
    exec: &dyn FlowExec,
    doc: &FlowDoc,
    labels: &[String],
) -> Result<Pass, Failure> {
    static_check(doc, labels)?;
    let started = Instant::now();
    let first = doc.meta.examples[0].input.clone();
    match exec.exec(doc, &first, Mode::Parse, None).await {
        Outcome::Parsed => {}
        Outcome::NeedsAgent(handover) => {
            return Err(Failure::new(Gate::Parse, handover.step, handover.message()));
        }
        Outcome::RunFailed(message) => return Err(Failure::new(Gate::Parse, None, message)),
        other => {
            return Err(Failure::new(
                Gate::Parse,
                None,
                format!("unexpected {other:?}"),
            ));
        }
    }
    let mut durations = Vec::new();
    for (index, example) in doc.meta.examples.iter().enumerate() {
        let gate = if index == 0 {
            Gate::Replay
        } else {
            Gate::Variety
        };
        let run_started = Instant::now();
        let outcome = exec.exec(doc, &example.input, Mode::Verify, None).await;
        let elapsed = run_started.elapsed();
        let with_example = |mut failure: Failure| {
            failure.example = Some(example.input.clone());
            failure
        };
        match outcome {
            Outcome::Done { output, .. } if doc.meta.effects == Effects::Read => {
                check_output(doc, &output)
                    .map_err(|message| with_example(Failure::new(gate, None, message)))?;
            }
            Outcome::VerifiedToCommit { .. } if doc.meta.effects == Effects::Write => {}
            Outcome::NeedsAgent(handover)
                if index > 0
                    && example.expect == "either"
                    && matches!(
                        handover.kind.as_str(),
                        "not_found" | "wait_timeout" | "parse_failed"
                    ) => {}
            Outcome::NeedsAgent(handover) => {
                let mut failure = Failure::new(gate, handover.step, handover.message());
                failure.view = handover.view.clone();
                return Err(with_example(failure));
            }
            Outcome::RunFailed(message) => {
                return Err(with_example(Failure::new(
                    gate,
                    None,
                    format!("the run failed: {message}"),
                )));
            }
            other => {
                return Err(with_example(Failure::new(
                    gate,
                    None,
                    format!("unexpected outcome {other:?}"),
                )));
            }
        }
        if elapsed > Duration::from_millis(SOFT_DEADLINE_MS) {
            return Err(with_example(Failure::new(
                Gate::Budget,
                None,
                format!(
                    "the run took {} ms; keep each run under {SOFT_DEADLINE_MS} ms",
                    elapsed.as_millis()
                ),
            )));
        }
        durations.push(i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX));
        if started.elapsed() > TOTAL_BUDGET {
            return Err(Failure::new(
                Gate::Budget,
                None,
                format!(
                    "verification passed {} s; use fewer or faster examples",
                    TOTAL_BUDGET.as_secs()
                ),
            ));
        }
    }
    durations.sort_unstable();
    Ok(Pass {
        runs: durations.len(),
        p50_ms: durations.get(durations.len() / 2).copied().unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::flow::{
        outcome::{Committed, Handover},
        store::{Example, FlowMeta, FlowStep},
    };
    use futures_util::future::BoxFuture;
    use serde_json::json;
    use std::{collections::BTreeMap, sync::Mutex};

    fn input(query: &str) -> Map<String, Value> {
        json!({ "query": query })
            .as_object()
            .cloned()
            .unwrap_or_default()
    }

    fn example(query: &str, source: &str) -> Example {
        Example {
            input: input(query),
            source: source.to_string(),
            expect: "done".to_string(),
        }
    }

    fn doc(effects: Effects, steps: Vec<(&str, bool)>) -> FlowDoc {
        FlowDoc {
            meta: FlowMeta {
                name: "neo-x".to_string(),
                flow_version: 0,
                effects,
                hosts: vec![],
                inputs: BTreeMap::from([("query".to_string(), "string".to_string())]),
                output: BTreeMap::from([("price".to_string(), "string".to_string())]),
                examples: vec![
                    example("sensodyne", "original"),
                    example("colgate", "agent"),
                    example("dove", "agent"),
                ],
                source_session: None,
            },
            steps: steps
                .into_iter()
                .enumerate()
                .map(|(index, (src, commit))| FlowStep {
                    n: u32::try_from(index + 1).unwrap_or(1),
                    commit,
                    src: src.to_string(),
                })
                .collect(),
        }
    }

    const OPEN: &str = "async ({ browser, inputs, state }) => { state.page = await browser.open('http://shop/s?k=' + inputs.query) }";
    const READ: &str = "async ({ state, handBack }) => { const ok = await state.page.waitForSelector('[data-result]'); if (!ok.matched) handBack('wait_timeout'); return { price: '1' } }";
    const CLICK: &str = "async ({ state }) => { await state.page.click(state.card.ref) }";

    fn labels(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("step {i}")).collect()
    }

    fn static_message(doc: &FlowDoc, n: usize) -> String {
        static_check(doc, &labels(n))
            .err()
            .map(|f| f.message)
            .unwrap_or_default()
    }

    #[test]
    fn good_flows_pass_the_static_rules() {
        assert!(
            static_check(
                &doc(Effects::Read, vec![(OPEN, false), (READ, false)]),
                &labels(2)
            )
            .is_ok()
        );
        assert!(
            static_check(
                &doc(Effects::Write, vec![(OPEN, false), (CLICK, true)]),
                &labels(2)
            )
            .is_ok()
        );
    }

    #[test]
    fn each_static_rule_catches_its_mistake() {
        let literal = doc(
            Effects::Read,
            vec![
                (OPEN, false),
                (
                    "async ({ state }) => { await state.page.click('e12') }",
                    false,
                ),
            ],
        );
        assert!(static_message(&literal, 2).contains("literal ref"));
        let pause = doc(
            Effects::Read,
            vec![(OPEN, false), ("async () => { await sleep(4000) }", false)],
        );
        assert!(static_message(&pause, 2).contains("fixed pause of 4000"));
        let bare = doc(
            Effects::Read,
            vec![
                (OPEN, false),
                (
                    "async ({ state }) => {\n  await state.page.waitForText('Added')\n}",
                    false,
                ),
            ],
        );
        assert!(static_message(&bare, 2).contains("ignored"));
        let generated = doc(
            Effects::Read,
            vec![
                (OPEN, false),
                (
                    "async ({ state }) => { const ok = await state.page.waitForSelector('.a-x7f3'); return ok }",
                    false,
                ),
            ],
        );
        assert!(static_message(&generated, 2).contains("generated CSS class"));
        let stable = doc(
            Effects::Read,
            vec![
                (OPEN, false),
                (
                    "async ({ state, handBack }) => { const ok = await state.page.waitForSelector('.s-result-item'); if (!ok.matched) handBack('x'); return { price: '1' } }",
                    false,
                ),
            ],
        );
        assert!(static_check(&stable, &labels(2)).is_ok());
        let outside = doc(
            Effects::Write,
            vec![
                (OPEN, false),
                (
                    "async ({ state }) => { await state.raw.click(state.ref) }",
                    true,
                ),
            ],
        );
        assert!(static_message(&outside, 2).contains("only through state.page"));
        let two_commits = doc(Effects::Write, vec![(OPEN, true), (CLICK, true)]);
        assert!(static_message(&two_commits, 2).contains("exactly one step"));
        let missing = doc(Effects::Read, vec![(OPEN, false)]);
        assert!(static_message(&missing, 2).contains("one step per numbered step"));
        let mut few = doc(Effects::Read, vec![(OPEN, false), (READ, false)]);
        few.meta.examples.truncate(1);
        assert!(static_message(&few, 2).contains("at least 3 examples"));
    }

    /// Plays back one outcome per call: parse first, then one per example.
    struct Script(Mutex<Vec<Outcome>>);

    impl FlowExec for Script {
        fn exec<'a>(
            &'a self,
            _doc: &'a FlowDoc,
            _inputs: &'a Map<String, Value>,
            _mode: Mode,
            _stop_after: Option<u32>,
        ) -> BoxFuture<'a, Outcome> {
            let next = self
                .0
                .lock()
                .ok()
                .and_then(|mut queue| (!queue.is_empty()).then(|| queue.remove(0)))
                .unwrap_or(Outcome::RunFailed("no more outcomes".to_string()));
            Box::pin(async move { next })
        }
    }

    fn done(price: Value) -> Outcome {
        Outcome::Done {
            output: json!({ "price": price }),
            done: json!([]),
            duration_ms: Some(500),
        }
    }

    fn handed_back(kind: &str) -> Outcome {
        Outcome::NeedsAgent(Handover {
            kind: kind.to_string(),
            step: Some(2),
            data: json!({ "message": "-70% S ₹961 did not parse" }),
            done: json!([]),
            committed: Committed::No,
            page: Value::Null,
            view: Some("- link \"₹961\"".to_string()),
            duration_ms: None,
        })
    }

    #[tokio::test]
    async fn a_flow_that_works_on_every_example_passes() {
        let flow = doc(Effects::Read, vec![(OPEN, false), (READ, false)]);
        let script = Script(Mutex::new(vec![
            Outcome::Parsed,
            done(json!("1")),
            done(json!("2")),
            done(json!("3")),
        ]));
        let pass = verify(&script, &flow, &labels(2)).await;
        assert_eq!(pass.map(|p| p.runs), Ok(3));
    }

    #[tokio::test]
    async fn a_parser_that_misses_another_product_fails_variety() {
        let flow = doc(Effects::Read, vec![(OPEN, false), (READ, false)]);
        let script = Script(Mutex::new(vec![
            Outcome::Parsed,
            done(json!("1")),
            handed_back("parse_failed"),
        ]));
        let failure = verify(&script, &flow, &labels(2)).await.err();
        assert_eq!(failure.as_ref().map(|f| f.gate), Some(Gate::Variety));
        assert!(
            failure
                .as_ref()
                .is_some_and(|f| f.describe().contains("colgate") && f.describe().contains("₹961"))
        );
    }

    #[tokio::test]
    async fn wrong_output_types_fail_replay() {
        let flow = doc(Effects::Read, vec![(OPEN, false), (READ, false)]);
        let script = Script(Mutex::new(vec![Outcome::Parsed, done(json!(463))]));
        let failure = verify(&script, &flow, &labels(2)).await.err();
        assert_eq!(failure.map(|f| f.gate), Some(Gate::Replay));
    }

    #[tokio::test]
    async fn a_write_flow_must_reach_its_commit() {
        let flow = doc(Effects::Write, vec![(OPEN, false), (CLICK, true)]);
        let reached = || Outcome::VerifiedToCommit {
            dry: json!([{ "method": "click" }]),
            duration_ms: Some(400),
        };
        let script = Script(Mutex::new(vec![
            Outcome::Parsed,
            reached(),
            reached(),
            reached(),
        ]));
        assert!(verify(&script, &flow, &labels(2)).await.is_ok());
        let script = Script(Mutex::new(vec![Outcome::Parsed, done(json!("1"))]));
        assert_eq!(
            verify(&script, &flow, &labels(2))
                .await
                .err()
                .map(|f| f.gate),
            Some(Gate::Replay)
        );
    }

    #[tokio::test]
    async fn a_syntax_error_names_its_step() {
        let flow = doc(Effects::Read, vec![(OPEN, false), (READ, false)]);
        let mut syntax = handed_back("syntax");
        if let Outcome::NeedsAgent(handover) = &mut syntax {
            handover.data = json!({ "step": 2, "message": "SyntaxError: unexpected token" });
        }
        let script = Script(Mutex::new(vec![syntax]));
        let failure = verify(&script, &flow, &labels(2)).await.err();
        assert_eq!(
            failure.as_ref().map(|f| (f.gate, f.step)),
            Some((Gate::Parse, Some(2)))
        );
    }
}
