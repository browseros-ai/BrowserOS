//! The two test bundles neo-playwright-js's build.ts writes beside the cases, run in QuickJS, the
//! engine the TypeScript renderer runs in. `build.ts --check` fails when either is stale, so they
//! are today's renderer and the vendored printer as the repository holds them.

use std::error::Error;
use std::slice;

use rquickjs::{CatchResultExt, Context, Runtime};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::aria_node::AriaNode;
use crate::call::Call;
use crate::delta::Earlier;
use crate::form::{Form, Passed, TextForm};
use crate::snapshot::Snapshot;

use super::case_file::Pending;
use super::expected::Expected;

/// Today's TypeScript renderer: one script that sets the global `todaysAnswer(call, result)`
/// (neo-playwright-js `scripts/snapshot-cases/answer.ts`), the dispatch the case writer uses.
const TODAYS_RENDERER: &str = include_str!("../../tests/data/todays-renderer.js");

/// Upstream's printer, unmodified: one script that sets the global `renderAriaSnapshotAsYaml`.
const VENDORED_PRINTER: &str = include_str!("../../tests/data/vendored-printer.js");

/// One QuickJS realm holding both bundles. Each sets its one global from inside its own function,
/// so they share nothing else.
pub(crate) struct Bundles(Context);

impl Bundles {
    pub(crate) fn new() -> Result<Self, Box<dyn Error>> {
        let runtime = Runtime::new()?;
        let context = Context::full(&runtime)?;
        context.with(|ctx| -> Result<(), String> {
            for script in [TODAYS_RENDERER, VENDORED_PRINTER] {
                ctx.eval::<(), _>(script)
                    .catch(&ctx)
                    .map_err(|caught| caught.to_string())?;
            }
            Ok(())
        })?;
        Ok(Self(context))
    }

    /// Upstream's own text for `tree`: `renderAriaSnapshotAsYaml(tree)`, its walk recursing.
    pub(crate) fn vendored_text(&self, tree: &[AriaNode]) -> Result<String, Box<dyn Error>> {
        self.run(
            &serde_json::to_string(tree)?,
            "renderAriaSnapshotAsYaml(JSON.parse(input))",
        )
    }

    /// Upstream's text for one node: `renderAriaSnapshotAsYaml([node])`.
    pub(crate) fn vendored_node_text(&self, node: &AriaNode) -> Result<String, Box<dyn Error>> {
        self.vendored_text(slice::from_ref(node))
    }

    /// Today's Playwright-format text for `snapshot`: `todaysAnswer({ playwright: {} }, result)`.
    pub(crate) fn todays_playwright_text(
        &self,
        snapshot: &Snapshot,
    ) -> Result<String, Box<dyn Error>> {
        self.todays(&Call::Playwright, snapshot)?
            .text
            .ok_or_else(|| "Playwright's format always prints a text".into())
    }

    /// Today's answer to the same call `render` takes, as the prelude would send it.
    pub(crate) fn todays(
        &self,
        call: &Call<'_>,
        snapshot: &Snapshot,
    ) -> Result<TodaysAnswer, Box<dyn Error>> {
        self.todays_answer(&call_json(call), snapshot)
    }

    /// Today's answer to `call` (a case's call, `{ page: passed }` and the like) on `snapshot`:
    /// `todaysAnswer(call, result)`, the result holding the tree and every fact as Rust hands
    /// them over today. A lone half of a character in a text, which only the covered block's cut
    /// makes, comes back as U+FFFD, as the committed cases write it.
    pub(crate) fn todays_answer(
        &self,
        call: &Value,
        snapshot: &Snapshot,
    ) -> Result<TodaysAnswer, Box<dyn Error>> {
        let input = json!({ "call": call, "result": result(snapshot) });
        let answer = self.run(
            &input.to_string(),
            r"(() => {
                const { call, result } = JSON.parse(input);
                const answer = todaysAnswer(call, result);
                const whole = (text) => typeof text === 'string'
                    ? text.replace(/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/g, '\uFFFD')
                    : text;
                return JSON.stringify({
                    text: whole(answer.text),
                    pending: answer.pending && { text: whole(answer.pending.text), form: answer.pending.form },
                });
            })()",
        )?;
        Ok(serde_json::from_str(&answer)?)
    }

    /// Evaluates `script` with `input` as the global string `input`, and reads the string it
    /// returns.
    fn run(&self, input: &str, script: &str) -> Result<String, Box<dyn Error>> {
        Ok(self.0.with(|ctx| -> Result<String, String> {
            ctx.globals()
                .set("input", input)
                .catch(&ctx)
                .map_err(|caught| caught.to_string())?;
            ctx.eval::<String, _>(script)
                .catch(&ctx)
                .map_err(|caught| caught.to_string())
        })?)
    }
}

/// What today's renderer answered: the text (the block for a cover, `None` for none) and, for a
/// whole page in Neo's format, what it keeps.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TodaysAnswer {
    pub(crate) text: Option<String>,
    pub(crate) pending: Option<Pending>,
}

impl TodaysAnswer {
    /// The answer as every judge holds `render`'s to it.
    pub(crate) fn expected(&self) -> Expected<'_> {
        Expected {
            text: self.text.as_deref(),
            pending: self.pending.as_ref(),
        }
    }
}

/// `call` as the case writer's `Call` says it (`scripts/snapshot-cases/model.ts`): its one key and
/// its options, each left out when the script left it out.
fn call_json(call: &Call<'_>) -> Value {
    match call {
        Call::Playwright => json!({ "playwright": {} }),
        Call::Full { max_tokens } => {
            let mut options = Map::new();
            if let Some(max_tokens) = max_tokens {
                options.insert("maxTokens".to_owned(), json!(max_tokens.tokens()));
            }
            json!({ "full": options })
        }
        Call::Element(passed) => json!({ "element": passed_json(passed) }),
        Call::Page(passed) => json!({ "page": passed_json(passed) }),
        Call::Delta { passed, earlier } => json!({
            "delta": { "passed": passed_json(passed), "earlier": earlier_json(*earlier) }
        }),
    }
}

/// What a delta found, as the case writer's `Earlier` holds it: `'notRead'`, `'nothing'`, or the
/// stored text and its form, `inert` written only when true, as the TypeScript writes a form.
fn earlier_json(earlier: Earlier<'_>) -> Value {
    match earlier {
        Earlier::NotRead => json!("notRead"),
        Earlier::Nothing => json!("nothing"),
        Earlier::Text(stored) => json!({ "text": stored.text, "form": form_json(&stored.form) }),
    }
}

fn form_json(form: &Form) -> Value {
    let text = match form.text {
        TextForm::Kept => "kept",
        TextForm::LeftOut => "leftOut",
        TextForm::Always => "always",
        TextForm::Never => "never",
    };
    let mut json = Map::new();
    json.insert("text".to_owned(), json!(text));
    json.insert("urls".to_owned(), json!(form.urls));
    json.insert("maxTokens".to_owned(), json!(form.max_tokens.tokens()));
    if form.inert {
        json.insert("inert".to_owned(), json!(true));
    }
    Value::Object(json)
}

/// The snapshot operation's result as Rust hands it to today's renderer: `{ json, partial }`, and
/// `onTop`, `closedRoots` and `unmarked` when the page has them.
fn result(snapshot: &Snapshot) -> Value {
    let partial: Vec<Value> = snapshot
        .partial
        .iter()
        .map(|frame| json!({ "ref": frame.r#ref, "frame": frame.frame, "reason": frame.reason }))
        .collect();
    let mut result = Map::new();
    result.insert("json".to_owned(), json!(snapshot.tree));
    result.insert("partial".to_owned(), Value::Array(partial));
    if !snapshot.layers.is_empty() {
        let layers: Vec<Value> = snapshot
            .layers
            .iter()
            .map(|layer| {
                json!({ "ref": layer.r#ref, "line": layer.line, "wholeView": layer.whole_view })
            })
            .collect();
        result.insert("onTop".to_owned(), Value::Array(layers));
    }
    if let Some(roots) = &snapshot.closed_roots {
        result.insert(
            "closedRoots".to_owned(),
            json!({ "sent": roots.sent, "found": roots.found }),
        );
    }
    if let Some(unmarked) = &snapshot.unmarked {
        result.insert("unmarked".to_owned(), json!({ "reason": unmarked.reason }));
    }
    Value::Object(result)
}

/// A call's options as the prelude sends them: each left out when the script left it out.
fn passed_json(passed: &Passed) -> Value {
    let mut options = Map::new();
    if let Some(text) = passed.text {
        options.insert("text".to_owned(), json!(text));
    }
    if let Some(urls) = passed.urls {
        options.insert("urls".to_owned(), json!(urls));
    }
    if let Some(max_tokens) = passed.max_tokens {
        options.insert("maxTokens".to_owned(), json!(max_tokens.tokens()));
    }
    if let Some(inert) = passed.inert {
        options.insert("inert".to_owned(), json!(inert));
    }
    Value::Object(options)
}
