//! The `run` tool's description: what a model reads before it writes a script.

use crate::config::Defaults;
use crate::dispatch::{Listing, Registry};

/// How a script is written and what it gets, true of every build.
const USAGE: &str = "Runs JavaScript against the user's browser and returns what the script \
returns. `code` is the body of an async function: `await` each SDK call and `return` a JSON \
value. The function's parameters are the SDK: `page` (this task's tab; a background tab opens \
on the first call that needs one), `context`, `neo` (any `neo.*` method listed below), `console` \
(its lines come back with the result), `setTimeout`, `clearTimeout`, `NeoError` and \
`TimeoutError`. Every run gets a fresh JavaScript realm; only the browser's tabs carry over from \
one run to the next. The realm is QuickJS, which has no `Intl`.";

/// The error model, which the agent needs before it retries anything.
const ERRORS: &str = "An SDK call that fails throws a `NeoError` with `code`, `owner` (who must \
fix it: neo, script, website or host), `delivery` and `operationId`; a timeout throws its \
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
            "`timeout` is in milliseconds: {} by default, at most {}. The returned value is at most {} KiB of JSON.",
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

    use browseros_trace::Effect;
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
