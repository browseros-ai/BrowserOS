//! The `run` tool's description: what a model reads before it writes a script.

use crate::config::Defaults;
use crate::dispatch::{Listing, Registry};

/// How a script is written and what it gets, true of every build.
const USAGE: &str = "Runs JavaScript against the user's browser and returns what the script \
returns. `code` is the body of an async function: `await` each SDK call and `return` a JSON \
value. Use one task per user request. On the first run, call `neo.createTask(name)` and return \
`task.id`; keep that id. Start later runs with `const task = await neo.task(id)`. Open tabs with \
`await task.newPage()`, list them with `task.pages()` (oldest first), or pick one with \
`task.page(tabId)`. When done, call `await task.close()`. See SDK methods below.\n\n\
Without a host task, the first use of built-in `page` opens a background tab in a new unnamed \
task on every run; the reply names it. After `neo.createTask` or `neo.task`, built-in `page` \
is refused if it has no tab yet; an existing tab keeps working. Declare your own \
`const page = task.page(tabId)` or `const page = await task.newPage()`. After fetching a task, \
`context.newPage()` is refused; use `task.newPage()`. A host's `taskId` keeps its default page \
rule: that task's last-used tab, or a new tab on first use.\n\n\
Every reply that ran a script, including a failed one, ends with one line per created or \
fetched task: `[task <id> · tabs: <tabId> <url>, ...]`, `[task <id> · tabs: none]`, \
`[new task <id> was created for this script's built-in page · tabs: ...]`, or \
`[task <id> closed · its tabs were closed]`. Host tasks appear only if fetched with `neo.task`.\n\n\
The function's parameters are the SDK: Playwright's `page`, `context`, `browser`, `expect` and `playwright` \
(`selectors.setTestIdAttribute`, `errors.TimeoutError`), Node's `Buffer` (`screenshot` and `pdf` \
resolve to one, and a `setInputFiles` payload takes one), then `neo` (any \
`neo.*` method listed below), `console` (its lines come back with the result), `setTimeout`, \
`clearTimeout`, `NeoError` and `TimeoutError`. Every run gets a fresh JavaScript realm; only the \
browser's tabs carry over from one run to the next. The realm is QuickJS, which has no `Intl`.";

/// The error model, which the agent needs before it retries anything.
const ERRORS: &str = "An SDK call that fails throws a `NeoError` with `code`, `owner` (who must \
fix it: neo, script, website or host), `delivery`, `retryable` and `operationId`; a timeout throws its \
subclass `TimeoutError`. `delivery: \"unknown\"` means a write may have happened: look at the \
page before repeating it.";

/// sdk-playwright's ref hint, shown once its snapshot method is registered.
const REF_HINT: &str =
    "`page.ariaSnapshot({ mode: 'ai' })` gives refs, and `page.getByRef('e5')` acts on one.";
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
        assert!(run_description(&builder.build()).contains("page.getByRef('e5')"));
        Ok(())
    }

    #[test]
    fn the_usage_explains_how_tasks_keep_tabs_across_runs() {
        let description = run_description(&RegistryBuilder::default().build());
        for sentence in [
            "Use one task per user request.",
            "On the first run, call `neo.createTask(name)` and return `task.id`; keep that id.",
            "Start later runs with `const task = await neo.task(id)`.",
            "Open tabs with `await task.newPage()`, list them with `task.pages()` (oldest first), or pick one with `task.page(tabId)`.",
            "When done, call `await task.close()`.",
        ] {
            assert!(
                description.contains(sentence),
                "the agent needs the task lifecycle before writing a script: {sentence}"
            );
        }
    }

    #[test]
    fn the_usage_explains_default_pages_and_task_recovery_on_failure() {
        let description = run_description(&RegistryBuilder::default().build());
        for sentence in [
            "Without a host task, the first use of built-in `page` opens a background tab in a new unnamed task on every run; the reply names it.",
            "After `neo.createTask` or `neo.task`, built-in `page` is refused if it has no tab yet; an existing tab keeps working.",
            "Declare your own `const page = task.page(tabId)` or `const page = await task.newPage()`.",
            "After fetching a task, `context.newPage()` is refused; use `task.newPage()`.",
            "A host's `taskId` keeps its default page rule: that task's last-used tab, or a new tab on first use.",
            "Every reply that ran a script, including a failed one, ends with one line per created or fetched task:",
            "[task <id> · tabs: <tabId> <url>, ...]",
            "[task <id> · tabs: none]",
            "[new task <id> was created for this script's built-in page · tabs: ...]",
            "[task <id> closed · its tabs were closed]",
            "Host tasks appear only if fetched with `neo.task`.",
        ] {
            assert!(
                description.contains(sentence),
                "the agent must know which tab it gets and how to recover the task id: {sentence}"
            );
        }
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
