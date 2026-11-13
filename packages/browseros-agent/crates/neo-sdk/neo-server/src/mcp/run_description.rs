//! The `run` tool's description: what a model reads before it writes a script.

use crate::config::Defaults;
use crate::dispatch::{Listing, Registry};

/// How a script is written and what it gets, true of every build.
const USAGE: &str = "Runs JavaScript against the user's browser and returns what the script \
returns. `code` is the body of an async function: `await` each SDK call and `return` a JSON \
value. Each run starts with fresh JavaScript. With no task, built-in `page` opens a new tab \
on first use; `context.newPage()` opens another. Neither reuses a tab or creates a task.\n\n\
To keep tabs across runs, create a task and return its id. First run: \
`const task = await neo.createTask('hn top 5'); return task.id`. Start later scripts with \
`await neo.task('<id>')`; then `page` uses the task's last-used open tab (else another open \
tab, else a new one), and `context.newPage()` opens in the task. The latest task pick wins. \
Use `task.pages()` (oldest first), `task.page(tabId)` or `task.newPage()` as needed. \
When finished: `await task.close()`. Popups join their opener's task. `context.pages()` \
lists every tab with `page.tabId` and `page.taskId` (null without a task); use it to recover \
a lost task id.\n\n\
The function's parameters are the SDK: Playwright's `page`, `context`, `browser`, `expect` and \
`playwright` (`selectors.setTestIdAttribute`, `errors.TimeoutError`), Node's `Buffer` \
(`screenshot` and `pdf` resolve to one, and a `setInputFiles` payload takes one), then `neo` \
(the `neo.*` methods listed below), `console` (its lines come back with the result), \
`setTimeout`, `clearTimeout`, `NeoError` and `TimeoutError`. The realm is QuickJS, which has no \
`Intl`.";

/// The error model, which the agent needs before it retries anything.
const ERRORS: &str = "An SDK call that fails throws a `NeoError` with `code`, `owner` (who must \
fix it: neo, script, website or host), `delivery`, `retryable` and `operationId`; a timeout throws its \
subclass `TimeoutError`. `delivery: \"unknown\"` means a write may have happened: look at the \
page before repeating it.";

/// sdk-playwright's ref hint, shown once its snapshot method is registered. The prelude's v1
/// text is the default (`neo-playwright-js/src/prelude/snapshot-v1.ts`): refs only on what can
/// be acted on and on sections, page text only while it is short. So the hint names the option
/// that adds the text, and the two ways to reach an element printed without a ref: a locator,
/// or `mode: 'full'`, Playwright's own text with a ref on each visible element
/// (`neo-playwright-js/src/prelude/snapshot.ts`). Every agent follows this text literally, so it
/// names one section: a whole page in that mode is 9k tokens at the median of 31 live pages and
/// 52k on a big one, which the mode's `maxTokens` cap cuts, while one section fits in 196 of 212
/// cases. It names that mode, not `ariaSnapshotJSON`: the JSON has no cap and no clickable
/// marks, so it lacks refs the snapshot prints.
const REF_HINT: &str = "`page.ariaSnapshot()` lists what you can act on, with refs; \
`page.getByRef('e5')` acts on one; `{ text: true }` adds the page's text. An element without a \
ref is reached with a locator (`page.getByText('…')`). \
A field printed without a ref takes no click and no `fill` (it has no size on the \
page, or is disabled): click the element drawn for it, the line beside it, then \
`page.keyboard.type('…')`. \
`page.getByRef('e12').ariaSnapshot({ mode: 'full' })` prints all of one section as Playwright \
does, with its text and a ref on each visible element, up to `maxTokens`; a whole page that way \
is several times larger.";
const SNAPSHOT_METHOD: &str = "page.ariaSnapshot";

/// Usage, limits, the error model, then the registry's listed methods: each `Own` line, and
/// each `Grouped` line once (the Playwright API is one line, not 120). Built once at startup;
/// the registry is frozen, so it never changes while Neo runs.
#[must_use]
pub(crate) fn run_description(registry: &Registry) -> String {
    let listed = registry.listed();
    let mut lines = vec![
        USAGE.to_owned(),
        format!(
            "`timeout` is in milliseconds: {} by default, at most {}. The returned value is at most {} KiB of JSON; \
             bytes in it (a `Buffer`, such as a `screenshot` or `pdf`) that would take it over come back as \
             `{{\"$neo\":\"blob\",\"id\",\"size\"}}`, which the host reads, so return large text as `Buffer.from(text)`.",
            Defaults::RUN_TIMEOUT.as_millis(),
            Defaults::RUN_TIMEOUT_MAX.as_millis(),
            Defaults::MAX_RESULT_BYTES / 1024
        ),
        ERRORS.to_owned(),
    ];
    if listed.iter().any(|spec| spec.name == SNAPSHOT_METHOD) {
        lines.push(REF_HINT.to_owned());
    }
    let mut methods: Vec<&str> = Vec::new();
    for spec in &listed {
        match spec.listing {
            Listing::Own(line) => methods.push(line),
            Listing::Grouped(line) if !methods.contains(&line) => methods.push(line),
            Listing::Grouped(_) | Listing::Hidden => {}
        }
    }
    if !methods.is_empty() {
        lines.push("SDK methods:".to_owned());
        lines.extend(methods.into_iter().map(|line| format!("- {line}")));
    }
    lines.join("\n\n")
}

#[cfg(test)]
mod tests {
    use std::future::{self, Future};
    use std::time::Duration;

    use neo_trace::Effect;
    use serde_json::Value;

    use super::run_description;
    use crate::dispatch::{
        LeafError, Listing, Method, MethodSpec, OpCtx, PageTarget, RegistryBuilder, RegistryError,
    };
    use crate::runtime::{Script, ScriptOutcome, run_script};
    use crate::test_rig::{TestError, TestRig, on_neo_runtime};

    /// A method that is only its spec.
    struct Named(MethodSpec);

    impl Method for Named {
        type Input = Value;
        type Output = Value;

        fn spec(&self) -> MethodSpec {
            self.0
        }

        fn target(_input: &Value) -> Option<&PageTarget> {
            None
        }

        fn call(
            &self,
            _op: &OpCtx<Self>,
            _input: Value,
        ) -> impl Future<Output = Result<Value, LeafError>> + Send {
            future::ready(Ok(Value::Null))
        }
    }

    fn named(name: &'static str, listing: Listing) -> Named {
        Named(MethodSpec {
            listing,
            ..MethodSpec::new(name, Effect::Read)
        })
    }

    #[test]
    fn grouped_methods_share_one_line_and_hidden_ones_stay_out() -> Result<(), RegistryError> {
        let mut builder = RegistryBuilder::default();
        builder.define(named("page.goto", Listing::Grouped("the page API")))?;
        builder.define(named("page.title", Listing::Grouped("the page API")))?;
        builder.define(named(
            "neo.solveCaptcha",
            Listing::Own("neo.solveCaptcha(page)"),
        ))?;
        builder.define(named("neo.internal", Listing::Hidden))?;
        let description = run_description(&builder.build());
        assert_eq!(
            description.matches("- the page API").count(),
            1,
            "one line for the group"
        );
        assert!(description.contains("- neo.solveCaptcha(page)"));
        assert!(!description.contains("neo.internal"), "hidden stays hidden");
        assert!(
            !description.contains("ariaSnapshot"),
            "no ref hint before the method it names exists"
        );
        Ok(())
    }

    #[test]
    fn the_ref_hint_appears_with_the_snapshot_method() -> Result<(), RegistryError> {
        let mut builder = RegistryBuilder::default();
        builder.define(named(
            "page.ariaSnapshot",
            Listing::Grouped("Playwright's Page API"),
        ))?;
        let description = run_description(&builder.build());
        assert!(description.contains("`page.ariaSnapshot()` lists what you can act on"));
        assert!(description.contains("page.getByRef('e5')"));
        assert!(
            description.contains(
                "An element without a ref is reached with a locator (`page.getByText('…')`). \
                 A field printed without a ref takes no click and no `fill` (it has no size on the \
                 page, or is disabled): click the element drawn for it, the line beside it, then \
                 `page.keyboard.type('…')`. \
                 `page.getByRef('e12').ariaSnapshot({ mode: 'full' })` prints all of one section \
                 as Playwright does, with its text and a ref on each visible element, up to \
                 `maxTokens`; a whole page that way is several times larger."
            ),
            "v1 prints refs only on controls and sections, so the model learns how to reach the \
             rest, one section at a time: {description}"
        );
        assert!(
            !description.contains("ariaSnapshotJSON"),
            "the hint no longer sends the model to the uncapped, unmarked tree: {description}"
        );
        Ok(())
    }

    #[test]
    fn the_usage_teaches_task_selection_and_plain_playwright() -> Result<(), TestError> {
        let description = run_description(&RegistryBuilder::default().build());
        for sentence in [
            "returns what the script returns.",
            "Each run starts with fresh JavaScript.",
            "With no task, built-in `page` opens a new tab on first use; `context.newPage()` opens another.",
            "Neither reuses a tab or creates a task.",
            "To keep tabs across runs, create a task and return its id.",
            "First run: `const task = await neo.createTask('hn top 5'); return task.id`.",
            "Start later scripts with `await neo.task('<id>')`",
            "`page` uses the task's last-used open tab (else another open tab, else a new one)",
            "`context.newPage()` opens in the task.",
            "The latest task pick wins.",
            "When finished: `await task.close()`.",
            "Popups join their opener's task.",
            "`context.pages()` lists every tab with `page.tabId` and `page.taskId` (null without a task)",
        ] {
            assert!(
                description.contains(sentence),
                "the model writes its first script from this text, so it must learn the task \
                 recipe and the built-in page rule here: {sentence}"
            );
        }
        let recipe = description
            .find("To keep tabs across runs")
            .ok_or("missing task recipe")?;
        let first_script = description
            .find("`const task =")
            .ok_or("missing first script")?;
        assert!(
            recipe < first_script,
            "the model learns how to keep tabs before copying its first script"
        );
        assert!(
            !description.contains(concat!("neo.", "tabs")),
            "tab discovery uses Playwright's context"
        );
        Ok(())
    }

    /// The prose is hand-written, so the realm checks it: a parameter a prelude adds, or an
    /// `Intl` a QuickJS bump brings, fails here until the description says so.
    #[test]
    fn the_usage_names_every_parameter_and_the_missing_intl() -> Result<(), TestError> {
        on_neo_runtime(async {
            let rig = TestRig::new().await?;
            let description = run_description(rig.dispatcher.registry());
            let (run, _guard) = rig.run(Duration::from_mins(1), None)?;
            let script = Script {
                code: "return [__neo.parameters(), typeof Intl];".to_owned(),
                page: None,
                pages: Vec::new(),
                observed: None,
            };
            let ScriptOutcome::Returned(value) =
                run_script(&run, &rig.dispatcher, script).await.outcome
            else {
                return Err("the probe script returns".into());
            };
            let (parameters, intl): (String, String) = serde_json::from_value(value)?;
            for name in parameters.split(", ") {
                assert!(
                    description.contains(&format!("`{name}`")),
                    "the description names the parameter `{name}`: {description}"
                );
            }
            assert_eq!(intl, "undefined");
            assert!(description.contains("no `Intl`"));
            Ok(())
        })
    }
}
