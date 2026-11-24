//! Today's renderer: the TypeScript the prelude runs, as neo-snapshot's differential runs it. The
//! bundle `tests/data/todays-renderer.js` (built by neo-playwright-js `scripts/build.ts`, kept
//! current by `build.ts --check`) sets the global `todaysAnswer(call, result)`, the same dispatch
//! as `Frame._ariaSnapshot`. It runs in QuickJS with a run's limits from neo-server's
//! `config/defaults.rs`: a 128 MiB heap and a 512 KiB stack.
//!
//! One runtime per row, so a row that runs out of memory leaves nothing behind for the next.

use std::time::{Duration, Instant};

use neo_snapshot::{Call, Earlier, Form, Passed, Snapshot, TextForm};
use rquickjs::{CatchResultExt, Context, Runtime};
use serde_json::{Map, Value, json};

use crate::counting;

const TODAYS_RENDERER: &str = include_str!("../../../neo-snapshot/tests/data/todays-renderer.js");

/// `Defaults::JS_MEMORY_BYTES` in neo-server: one run's QuickJS heap.
pub const MEMORY_LIMIT_BYTES: usize = 128 * 1024 * 1024;

/// `Defaults::JS_STACK_BYTES` in neo-server.
const STACK_BYTES: usize = 512 * 1024;

/// One call in the old engine, timed in two steps as the tree reaches it today: the result as
/// JSON parsed into the script, then the render.
pub struct Sample {
    /// `JSON.parse` of the snapshot operation's result.
    pub parse: Duration,
    /// `todaysAnswer(call, result)`: the render, from the parsed tree to the text.
    pub render: Duration,
    /// QuickJS's own count after the render, the text still alive and nothing collected: the
    /// count its 128 MiB limit is checked against.
    pub own_count: usize,
    /// The most bytes the runtime held at once during the parse and the render, by this binary's
    /// counting allocator: the bundle, the result's JSON, the tree as objects, every line.
    pub peak: usize,
    /// The text it printed.
    pub text: String,
}

/// A QuickJS runtime holding today's renderer.
pub struct OldEngine {
    runtime: Runtime,
    context: Context,
}

impl OldEngine {
    pub fn new() -> Result<Self, String> {
        let runtime = Runtime::new().map_err(|error| error.to_string())?;
        runtime.set_memory_limit(MEMORY_LIMIT_BYTES);
        runtime.set_max_stack_size(STACK_BYTES);
        let context = Context::full(&runtime).map_err(|error| error.to_string())?;
        context.with(|ctx| {
            ctx.eval::<(), _>(TODAYS_RENDERER)
                .catch(&ctx)
                .map_err(|caught| caught.to_string())
        })?;
        Ok(Self { runtime, context })
    }

    /// Runs `call` on `snapshot` once. An error is the engine's own, out of memory included.
    pub fn answer(&self, call: &Call<'_>, snapshot: &Snapshot) -> Result<Sample, String> {
        let input = result(snapshot).to_string();
        let call = call_json(call).to_string();
        // The Rust strings stay alive until the count is read, so the peak above `before` is the
        // runtime's alone: the result's JSON as a script string, the tree parsed from it, the
        // render.
        let before = counting::reset_peak();
        self.eval(|ctx| {
            ctx.globals().set("input", input.as_str())?;
            ctx.globals().set("callInput", call.as_str())?;
            Ok(())
        })?;
        self.script("globalThis.call = JSON.parse(callInput); callInput = undefined; undefined")?;

        let started = Instant::now();
        self.script("globalThis.result = JSON.parse(input); input = undefined; undefined")?;
        let parse = started.elapsed();

        let started = Instant::now();
        let rendered = self.script("globalThis.answer = todaysAnswer(call, result); undefined");
        let render = started.elapsed();
        let peak = counting::peak().saturating_sub(before);
        let own_count = usize::try_from(self.runtime.memory_usage().malloc_size).unwrap_or(0);
        let text = rendered.and_then(|()| {
            self.context.with(|ctx| {
                ctx.eval::<String, _>("answer.text")
                    .catch(&ctx)
                    .map_err(|caught| caught.to_string())
            })
        });
        self.script("result = undefined; answer = undefined; call = undefined; undefined")?;
        self.runtime.run_gc();
        drop((input, call));
        Ok(Sample {
            parse,
            render,
            own_count,
            peak,
            text: text?,
        })
    }

    fn script(&self, source: &str) -> Result<(), String> {
        self.context.with(|ctx| {
            ctx.eval::<(), _>(source)
                .catch(&ctx)
                .map_err(|caught| caught.to_string())
        })
    }

    fn eval(
        &self,
        work: impl for<'js> FnOnce(&rquickjs::Ctx<'js>) -> rquickjs::Result<()>,
    ) -> Result<(), String> {
        self.context
            .with(|ctx| work(&ctx).map_err(|error| error.to_string()))
    }
}

/// `call` as the case writer's `Call` says it (`scripts/snapshot-cases/model.ts`), the same
/// encoding neo-snapshot's judges send: one key and the options the script passed.
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
        Call::Delta { passed, earlier } => {
            let earlier = match earlier {
                Earlier::NotRead => json!("notRead"),
                Earlier::Nothing => json!("nothing"),
                Earlier::Text(stored) => {
                    json!({ "text": stored.text, "form": form_json(&stored.form) })
                }
            };
            json!({ "delta": { "passed": passed_json(passed), "earlier": earlier } })
        }
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

/// The snapshot operation's result as the old path hands it to the prelude: `{ json, partial }`.
/// The bench's trees carry no layer, closed root or unfinished check.
fn result(snapshot: &Snapshot) -> Value {
    json!({ "json": snapshot.tree, "partial": [] })
}
