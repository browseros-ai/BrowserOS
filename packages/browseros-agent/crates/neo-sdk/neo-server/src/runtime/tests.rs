//! The runtime end to end: a real QuickJS realm, the bridge and the real dispatcher on the fake
//! browser, with the trace store read back. Each test drives one script through `run_script`
//! and asserts what the run service reads from it.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use neo_trace::{
    Delivery, ErrorOwner, OperationId, Outcome, SecretSetConfig, SecretSource, SecretValue,
};
use rquickjs::{AsyncContext, AsyncRuntime, Context, Runtime};
use serde_json::{Value, json};
use tokio::time::{self, Instant};

use super::install_web_globals::install_web_globals;
use super::{LogLevel, LogLine, Script, ScriptEnd, ScriptOutcome, run_script};
use crate::config::Defaults;
use crate::dispatch::{CallSite, ErrorCode, OperationError, RunCtx, StopReason};
use crate::test_rig::{TestError, TestRig, on_neo_runtime, within};

/// Far longer than any test here takes, so only the tests about the deadline meet it.
const BUDGET: Duration = Duration::from_mins(1);

async fn run_code(rig: &TestRig, run: &Arc<RunCtx>, code: &str) -> ScriptEnd {
    let script = Script {
        code: code.to_owned(),
        page: None,
        pages: Vec::new(),
    };
    run_script(run, &rig.dispatcher, script).await
}

async fn script(rig: &TestRig, code: &str) -> Result<ScriptEnd, TestError> {
    let (run, _guard) = rig.run(BUDGET, None)?;
    Ok(run_code(rig, &run, code).await)
}

fn returned(end: ScriptEnd) -> Result<Value, TestError> {
    match end.outcome {
        ScriptOutcome::Returned(value) => Ok(value),
        other => Err(format!("expected a returned value, got {other:?}").into()),
    }
}

fn failed(end: ScriptEnd) -> Result<OperationError, TestError> {
    match end.outcome {
        ScriptOutcome::Failed(error) => Ok(error),
        other => Err(format!("expected the script to fail, got {other:?}").into()),
    }
}

/// The globals of a realm with nothing of Neo's in it.
async fn quickjs_globals() -> Result<BTreeSet<String>, TestError> {
    let runtime = AsyncRuntime::new()?;
    let context = AsyncContext::full(&runtime).await?;
    let names: Result<Vec<String>, String> = context
        .async_with(async |ctx| {
            ctx.eval("Object.getOwnPropertyNames(globalThis)")
                .map_err(|error| error.to_string())
        })
        .await;
    Ok(names?.into_iter().collect())
}

#[test]
fn a_fresh_realm_adds_only_the_bridge_bootstrap_and_web_interfaces() -> Result<(), TestError> {
    on_neo_runtime(async {
        let quickjs = quickjs_globals().await?;
        let rig = TestRig::new().await?;
        let names =
            returned(script(&rig, "return Object.getOwnPropertyNames(globalThis);").await?)?;
        let agent: BTreeSet<String> = serde_json::from_value(names)?;
        let added: BTreeSet<&str> = agent
            .iter()
            .map(String::as_str)
            .filter(|name| !quickjs.contains(*name))
            .collect();
        assert_eq!(
            added,
            BTreeSet::from([
                "__neo",
                "__neo_invoke",
                "__neo_log",
                "__neo_sleep",
                "URL",
                "URLSearchParams"
            ]),
            "the agent's API arrives as parameters, so no second native door into Rust appears"
        );
        assert!(
            quickjs.is_subset(&agent),
            "QuickJS's own globals are untouched"
        );
        Ok(())
    })
}

#[test]
fn a_neo_error_carries_its_operations_fields() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = r#"
            try {
              await __neo.invoke("test.fail", {});
            } catch (e) {
              return { name: e.name, neo: e instanceof NeoError, timeout: e instanceof TimeoutError,
                       code: e.code, owner: e.owner, delivery: e.delivery, retryable: e.retryable,
                       operationId: e.operationId, message: e.message };
            }"#;
        let error = returned(run_code(&rig, &run, code).await)?;
        assert_eq!(
            error,
            json!({
                "name": "NeoError", "neo": true, "timeout": false, "code": "cdp_error",
                "owner": "website", "delivery": "not_sent", "retryable": true,
                "operationId": format!("{}.1", run.id()), "message": "the page said no",
            }),
            "the script sees the owner, delivery and op id the trace records"
        );
        Ok(())
    })
}

#[test]
fn a_timed_out_operation_throws_a_timeout_error() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r#"
            try {
              await __neo.invoke("test.slowAnswer", {});
            } catch (e) {
              return [e.name, e instanceof TimeoutError, e instanceof NeoError, e.code];
            }"#;
        assert_eq!(
            returned(script(&rig, code).await?)?,
            json!(["TimeoutError", true, true, "cdp_timeout"]),
            "Playwright's catch (e instanceof TimeoutError) keeps working, and it is still a NeoError"
        );
        Ok(())
    })
}

#[test]
fn an_uncaught_neo_error_fails_the_run_with_its_operations_failure() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let end = run_code(&rig, &run, r#"await __neo.invoke("test.fail", {});"#).await;
        let ScriptOutcome::OperationFailed(failure) = end.outcome else {
            return Err(format!("expected the op's failure, got {:?}", end.outcome).into());
        };
        assert_eq!(
            (failure.error.owner(), failure.error.code()),
            (ErrorOwner::Website, ErrorCode::CDP_ERROR),
            "the run keeps its op's owner instead of blaming the script"
        );
        assert_eq!(failure.operation.run, run.id());
        assert_eq!(failure.delivery, Delivery::NotSent);
        Ok(())
    })
}

#[test]
fn a_script_cannot_pass_its_own_error_off_as_an_operations() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let forged = r#"throw new NeoError({ code: "internal_error", owner: "neo",
            delivery: "done", retryable: false, operationId: "x", message: "forged" });"#;
        let error = failed(script(&rig, forged).await?)?;
        assert_eq!(
            (error.owner(), error.code()),
            (ErrorOwner::Script, ErrorCode::SCRIPT_ERROR),
            "a NeoError the script built is the script's own throw: {error}"
        );

        let edited = r#"
            try { await __neo.invoke("test.fail", {}); }
            catch (e) { e.owner = "neo"; e.code = "internal_error"; throw e; }"#;
        let end = script(&rig, edited).await?;
        let ScriptOutcome::OperationFailed(failure) = end.outcome else {
            return Err(format!("expected the op's failure, got {:?}", end.outcome).into());
        };
        assert_eq!(
            failure.error.owner(),
            ErrorOwner::Website,
            "the run reports what the door decided, not the fields the script edited"
        );
        Ok(())
    })
}

/// Waits out one timer, which yields to Rust: `await null` would resume before the door had
/// answered anything the script sent, since the realm's natives run only once its jobs drain.
const YIELD_TO_RUST: &str = "await new Promise((resolve) => setTimeout(resolve, 0));";

/// The refusal brand `__neo.refuse('page.route', 'method_unavailable', 'page.route: x')` sends.
fn route_refusal() -> Value {
    json!({ "$neo": "refused", "code": "method_unavailable", "reason": "page.route: x" })
}

/// FR/RC C1–C4. A Playwright method Neo does not serve is refused by the door, as a call of its
/// own: thrown uncaught, the refusal fails the run with the failure the door decided, code and op
/// included, never as a `script_error`. The door recorded it under the API the script called.
#[test]
fn an_uncaught_refusal_fails_the_run_with_its_code_and_op() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!(
            "{YIELD_TO_RUST}\nthrow __neo.refuse('page.route', 'method_unavailable', 'page.route: x');"
        );
        let end = run_code(&rig, &run, &code).await;
        let ScriptOutcome::OperationFailed(failure) = end.outcome else {
            return Err(format!("expected the refusal's failure, got {:?}", end.outcome).into());
        };
        assert_eq!(
            (
                failure.error.owner(),
                failure.error.code(),
                failure.error.message()
            ),
            (
                ErrorOwner::Script,
                ErrorCode::METHOD_UNAVAILABLE,
                "page.route: x"
            ),
            "the run keeps the code the prelude refused with"
        );
        assert_eq!(
            (failure.delivery, failure.retryable, failure.operation.run),
            (Delivery::NotSent, true, run.id()),
            "a refusal sends nothing, so it is safe to repeat, and it is this run's op"
        );
        rig.sink.flush().await?;
        let row = rig
            .store
            .operation(failure.operation)
            .await?
            .ok_or("the refusal is on the record")?;
        assert_eq!(
            row.name, "page.route",
            "recorded under the API the script called"
        );
        assert_eq!(
            (row.outcome, row.delivery, row.args),
            (
                Some(Outcome::Error),
                Some(Delivery::NotSent),
                Some(route_refusal())
            ),
            "a refused try, with the brand as its input"
        );
        Ok(())
    })
}

/// FR/RC C3. A refusal thrown as the script's first statement escapes before its door call has
/// even run, so the run waits for the door's answer before it settles.
#[test]
fn a_refusal_thrown_before_any_await_settles_as_its_op() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = "throw __neo.refuse('page.route', 'method_unavailable', 'page.route: x');";
        let end = run_code(&rig, &run, code).await;
        let ScriptOutcome::OperationFailed(failure) = end.outcome else {
            return Err(format!("expected the refusal's failure, got {:?}", end.outcome).into());
        };
        assert_eq!(
            (failure.operation, failure.error.code()),
            (
                OperationId {
                    run: run.id(),
                    seq: 1
                },
                ErrorCode::METHOD_UNAVAILABLE
            ),
            "the run's first op, though the script threw it before it ever yielded to Rust"
        );
        Ok(())
    })
}

/// FR/RC C2. A refusal the script catches is still a try the trace counts, answered and on the
/// record by the time a run that returns has settled.
#[test]
fn a_caught_refusal_is_on_the_record_and_never_left_running() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!(
            "{YIELD_TO_RUST}
            try {{
              throw __neo.refuse('page.route', 'method_unavailable', 'page.route: x');
            }} catch {{}}
            return 1;"
        );
        assert_eq!(returned(run_code(&rig, &run, &code).await)?, json!(1));
        run.stop(StopReason::ScriptReturned);
        let mut ops = run.close_ops();
        within(Duration::from_secs(5), async {
            while ops.join_next().await.is_some() {}
        })
        .await?;
        assert!(
            run.report().not_awaited.is_empty(),
            "the door answered the refusal before the run settled: {:?}",
            run.report().not_awaited
        );
        rig.sink.flush().await?;
        let row = rig
            .store
            .operation(OperationId {
                run: run.id(),
                seq: 1,
            })
            .await?
            .ok_or("the caught refusal is on the record")?;
        assert_eq!(
            (row.name.as_str(), row.outcome, row.delivery),
            ("page.route", Some(Outcome::Error), Some(Delivery::NotSent))
        );
        Ok(())
    })
}

/// FR/RC C2. A run that ends on a throw of the script's own waits on no refusal but the one it
/// throws, so a caught refusal is on the record only because the door admits each call at the
/// call. Raised after the script's first yield, its door call would otherwise never run.
#[test]
fn a_caught_refusal_is_on_the_record_when_the_script_then_throws() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!(
            "{YIELD_TO_RUST}
            try {{
              throw __neo.refuse('page.route', 'method_unavailable', 'page.route: x');
            }} catch {{}}
            throw new Error('after the refusal');"
        );
        let error = failed(run_code(&rig, &run, &code).await)?;
        assert_eq!(
            (error.owner(), error.code()),
            (ErrorOwner::Script, ErrorCode::SCRIPT_ERROR),
            "the run fails with the script's own throw: {error}"
        );
        rig.sink.flush().await?;
        let row = rig
            .store
            .operation(OperationId {
                run: run.id(),
                seq: 1,
            })
            .await?
            .ok_or("the caught refusal is on the record")?;
        assert_eq!(
            (row.name.as_str(), row.outcome, row.delivery),
            ("page.route", Some(Outcome::Error), Some(Delivery::NotSent))
        );
        Ok(())
    })
}

/// FR/RC C2. A caught refusal the script returns as its value is serialized only once the door
/// answered it, so the agent reads the op that links it to its trace row, though the script never
/// yielded to Rust.
#[test]
fn a_caught_refusal_returned_as_the_value_carries_its_operation_id() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = "try {
              throw __neo.refuse('page.route', 'method_unavailable', 'page.route: x');
            } catch (e) {
              return e;
            }";
        let value = returned(run_code(&rig, &run, code).await)?;
        assert_eq!(
            (value.get("code"), value.get("operationId")),
            (
                Some(&json!("method_unavailable")),
                Some(&json!(format!("{}.1", run.id())))
            ),
            "the returned refusal names its op: {value}"
        );
        Ok(())
    })
}

/// FR/RC C1. The door is the source of truth for a refusal's fields: once the script has yielded
/// to Rust, the error it holds carries the op the door recorded and the code the door decided,
/// even where the prelude named a code a refusal may not claim. Its message stays the realm's own
/// text.
#[test]
fn a_refusal_takes_its_fields_from_the_door() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!(
            "const refused = __neo.refuse('page.route', 'method_unavailable', 'page.route: x');
            const spoofed = __neo.refuse('page.route', 'browser_disconnected', 'page.route: y');
            {YIELD_TO_RUST}
            const fields = (e) => ({{ name: e.name, neo: e instanceof NeoError, code: e.code,
              owner: e.owner, delivery: e.delivery, retryable: e.retryable,
              operationId: e.operationId, message: e.message }});
            return {{ refused: fields(refused), spoofed: fields(spoofed) }};"
        );
        let fields = returned(run_code(&rig, &run, &code).await)?;
        let expected = |seq: u32, code: &str, message: &str| {
            json!({
                "name": "NeoError", "neo": true, "code": code, "owner": "script",
                "delivery": "not_sent", "retryable": true,
                "operationId": format!("{}.{seq}", run.id()), "message": message,
            })
        };
        assert_eq!(
            fields,
            json!({
                "refused": expected(1, "method_unavailable", "page.route: x"),
                "spoofed": expected(2, "invalid_input", "page.route: y"),
            }),
            "each error names its op, and carries the code the door gave it"
        );
        Ok(())
    })
}

/// FR/RC C5. A provider may build its parameter asynchronously, as the Playwright prelude's `page`
/// reads its frame tree through an op before the agent's first statement. One that rejects fails
/// the run like any throw. `__neo.run` is what `run_script` calls with the agent's function.
#[test]
fn a_provider_may_return_a_promise() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r#"
            const init = '{"page":null,"pages":[]}';
            __neo.provide('frames', async () =>
              (await __neo.invoke('test.echo', { frames: ['main'] })).frames);
            const built = JSON.parse(
              await __neo.run(async (...args) => ({ frames: args[args.length - 1] }), init));
            __neo.provide('broken', async () => { throw new Error('the page could not be read'); });
            const broken = JSON.parse(await __neo.run(async () => 'ran', init));
            return { built, broken: [broken.thrown?.kind, broken.thrown?.message] };"#;
        assert_eq!(
            returned(script(&rig, code).await?)?,
            json!({
                "built": { "ok": { "frames": ["main"] } },
                "broken": ["error", "the page could not be read"],
            }),
            "the agent's function gets what the promise resolved to, and never runs past a rejection"
        );
        Ok(())
    })
}

/// FR/RC C1. A reply listener that throws on a refusal's answer leaves the realm without the door's
/// fields, so the refusal settles as the script's own throw: never a false op claim, and never Neo's
/// internal error. The door still recorded the try.
#[test]
fn a_refusal_whose_reply_listener_throws_settles_as_a_script_error() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = r"
            __neo.onReply(() => { throw new Error('a reply listener broke'); });
            throw __neo.refuse('page.route', 'method_unavailable', 'page.route: x');";
        let error = failed(run_code(&rig, &run, code).await)?;
        assert_eq!(
            (error.owner(), error.code()),
            (ErrorOwner::Script, ErrorCode::SCRIPT_ERROR),
            "{error}"
        );
        assert!(
            error.message().starts_with("NeoError: page.route: x"),
            "the refusal the script threw, as its own error: {error}"
        );
        rig.sink.flush().await?;
        let row = rig
            .store
            .operation(OperationId {
                run: run.id(),
                seq: 1,
            })
            .await?
            .ok_or("the refusal is on the record")?;
        assert_eq!(row.name, "page.route");
        Ok(())
    })
}

#[test]
fn a_thrown_error_is_a_script_error_at_its_line() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let error = failed(script(&rig, "const value = 1;\nnull.missing;").await?)?;
        assert_eq!(error.code(), ErrorCode::SCRIPT_ERROR);
        assert!(
            error.message().starts_with("TypeError: ") && error.message().contains("(run:2:"),
            "the error names its kind and the script's own line: {error}"
        );

        let thrown = failed(script(&rig, r#"throw "plain text";"#).await?)?;
        assert_eq!(
            thrown.message(),
            "plain text",
            "a thrown non-error is its own text"
        );
        Ok(())
    })
}

/// A value the secret set refuses is masked nowhere, so every run going is lost: what enters its
/// realm from then on is `neo / output_withheld`, in place of what each op returned.
#[test]
fn a_value_the_set_refuses_withholds_what_enters_the_realm() -> Result<(), TestError> {
    on_neo_runtime(async {
        // The set keeps one value, the host's, which nothing pushes out: it refuses the next.
        let rig = TestRig::with_secret_set(SecretSetConfig {
            max_entries: 1,
            ..SecretSetConfig::default()
        })
        .await?;
        let _host = rig.secrets.register_from(
            SecretValue::new("host-secret-7Qx".to_owned()),
            None,
            SecretSource::Host,
        );
        let (run, _guard) = rig.run(BUDGET, None)?;
        rig.hold("agent-pw-7Qx");
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(250)).await;
        let code = r#"
            try {
              return await __neo.invoke("test.echo", { said: "hello" });
            } catch (e) {
              return [e.owner, e.code];
            }"#;
        assert_eq!(
            returned(run_code(&rig, &run, code).await)?,
            json!(["neo", "output_withheld"]),
            "the field went quiet and the set refused its value, so the run held a value \
             nothing masks"
        );
        Ok(())
    })
}

/// Graft G3. A script can hold a value its realm never received, such as a password it typed
/// itself, and throw it. The thrown text is masked with what the run holds too, before it is cut
/// to `TEXT_CHARS`: a cut through the value would leave a prefix that no mask recognizes, in the
/// run's error and its replay.
#[test]
fn a_thrown_errors_held_value_is_masked_before_its_clip() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let value = "typed-pw-7Qx-0123";
        rig.hold(value);
        // The message keeps 4096 characters, so this cut falls inside the value.
        let code = format!("throw new Error('x'.repeat(4090) + '{value}');");
        let error = failed(script(&rig, &code).await?)?;
        let message = error.message();
        let prefix = value.get(..4).unwrap_or(value);
        assert!(
            !message.contains(prefix),
            "no part of the held value survives the cut: {}",
            message
                .get(message.len().saturating_sub(40)..)
                .unwrap_or_default()
        );
        Ok(())
    })
}

#[test]
fn code_that_does_not_compile_is_a_syntax_error() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let error = failed(script(&rig, "const ok = 1;\nreturn (;").await?)?;
        assert_eq!(
            (error.owner(), error.code()),
            (ErrorOwner::Script, ErrorCode::SYNTAX_ERROR)
        );
        assert!(
            error.message().starts_with("SyntaxError: ") && error.message().contains("run:2"),
            "it says where, counting from the script's first line: {error}"
        );
        Ok(())
    })
}

#[test]
fn an_operation_left_running_names_the_line_that_started_it() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let end = run_code(&rig, &run, "\n__neo.invoke(\"test.hang\", {});\nreturn 1;").await;
        assert_eq!(returned(end)?, json!(1));
        run.stop(StopReason::ScriptReturned);
        let mut ops = run.close_ops();
        within(Duration::from_secs(5), async {
            while ops.join_next().await.is_some() {}
        })
        .await?;
        let report = run.report();
        let line = report.not_awaited.first().ok_or("the hang was cut short")?;
        assert_eq!(line.name, "test.hang");
        assert_eq!(
            line.site.map(|site| site.line),
            Some(2),
            "the bootstrap reads the call site from the agent's own frame"
        );
        assert!(
            matches!(line.site, Some(CallSite { column, .. }) if column > 0),
            "with its column"
        );
        Ok(())
    })
}

/// An op reaches the door at the script's call, wherever the script is: one it calls after its
/// first yield to Rust and never awaits is on the record too, cut as not awaited once the script
/// returns. A call that waited for the realm to yield again would never reach the door from a
/// script that returns first.
#[test]
fn an_operation_left_running_after_an_await_is_still_cut_as_not_awaited() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!("{YIELD_TO_RUST}\n__neo.invoke('test.hang', {{}});\nreturn 1;");
        assert_eq!(returned(run_code(&rig, &run, &code).await)?, json!(1));
        run.stop(StopReason::ScriptReturned);
        let mut ops = run.close_ops();
        within(Duration::from_secs(5), async {
            while ops.join_next().await.is_some() {}
        })
        .await?;
        let names: Vec<&str> = run
            .report()
            .not_awaited
            .iter()
            .map(|line| line.name)
            .collect();
        assert_eq!(
            names,
            ["test.hang"],
            "the op the script left running was admitted, then cut"
        );
        Ok(())
    })
}

#[test]
fn a_page_brand_revives_into_one_page_object() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r#"
            const opened = await context.newPage();
            const echoed = await __neo.invoke("test.echo", { page: opened });
            return { same: echoed.page === opened, brand: opened.toJSON(), url: opened.url() };"#;
        let value = returned(script(&rig, code).await?)?;
        assert_eq!(
            value.get("same"),
            Some(&json!(true)),
            "a page handed back is the same object"
        );
        let brand = value.get("brand").ok_or("a brand")?;
        assert_eq!(brand.get("$neo"), Some(&json!("page")));
        assert!(
            brand.get("id").is_some_and(Value::is_u64)
                && brand.get("tab").is_some_and(Value::is_i64),
            "the handle carries core's id and the browser's tab: {brand}"
        );
        assert_eq!(
            value.get("url"),
            Some(&json!("about:blank")),
            "page facts from the reply feed the sync accessors"
        );
        Ok(())
    })
}

/// M1 handoff line 4. neo-playwright carries a result 297 JSON levels deep, and a reply's
/// value is revived on the run's 512 KiB stack. This one is 5,000 objects deep, with a brand at
/// the bottom, and goes to `revive` as a reply's value does: a reply listener swaps it in, so no
/// JSON parse or Rust bound stands between it and `revive`.
#[test]
fn a_result_far_deeper_than_the_js_stack_revives_whole() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r#"
            let deep = { bottom: { $neo: "bytes", base64: "AQI=" } };
            for (let level = 0; level < 5000; level++) deep = { down: deep };
            let feed = true;
            __neo.onReply((envelope) => {
              if (feed) { feed = false; envelope.ok = deep; }
            });
            const revived = await __neo.invoke("test.echo", {});
            let depth = 0;
            let node = revived;
            while (node.down !== undefined) { node = node.down; depth++; }
            const bottom = node.bottom instanceof Uint8Array ? Array.from(node.bottom) : node.bottom;
            return { depth, same: revived === deep, bottom };"#;
        assert_eq!(
            returned(script(&rig, code).await?)?,
            json!({ "depth": 5000, "same": true, "bottom": [1, 2] }),
            "every level is walked, in place, and the brand at the bottom is revived"
        );
        Ok(())
    })
}

/// S3b. An op's input is made plain JSON on the run's 512 KiB stack, where recursion overflowed
/// past 17 nested arrays or 51 nested objects in a debug build (measured here), and a
/// Playwright locator's parsed selector with `:scope` inside `:has()` is 28 levels deep. These go
/// 120 levels deep, of each kind, through a real op and back. The door's JSON parse takes 127
/// levels in all, so that, not the realm's stack, is how deep an input can go.
#[test]
fn an_input_far_deeper_than_the_js_stack_reaches_its_op_whole() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r#"
            const nest = (wrap) => {
              let value = "bottom";
              for (let level = 0; level < 120; level++) value = wrap(value);
              return value;
            };
            const walk = (value, down) => {
              let depth = 0;
              while (typeof value === "object") { value = down(value); depth++; }
              return [depth, value];
            };
            const inObject = (value) => ({ down: value });
            const inArray = (value) => [value];
            const objects = await __neo.invoke("test.echo", { deep: nest(inObject) });
            const arrays = await __neo.invoke("test.echo", { deep: nest(inArray) });
            return {
              objects: walk(objects.deep, (value) => value.down),
              arrays: walk(arrays.deep, (value) => value[0]),
            };"#;
        assert_eq!(
            returned(script(&rig, code).await?)?,
            json!({ "objects": [120, "bottom"], "arrays": [120, "bottom"] }),
            "every level reaches the op and comes back"
        );
        Ok(())
    })
}

/// S3b's review. A Proxy around an array answers whatever `length` its trap chooses, and the
/// input codec counts its items as `Array.prototype.map` did, by that length as a whole number of
/// at least 0: a length in text counts, a fraction is cut and a negative is none. Each item it
/// lacks is a hole, sent as null.
#[test]
fn a_proxy_array_has_as_many_items_as_map_would_count() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r#"
            const answering = (length) => new Proxy([], {
              get: (target, key) => (key === "length" ? length : target[key]),
            });
            const sent = async (length) =>
              (await __neo.invoke("test.echo", { items: answering(length) })).items;
            return { text: await sent("2"), fraction: await sent(1.5), negative: await sent(-1) };"#;
        assert_eq!(
            returned(script(&rig, code).await?)?,
            json!({ "text": [null, null], "fraction": [null], "negative": [] }),
            "the copy has the items map would have made, holes as null"
        );
        Ok(())
    })
}

/// The Playwright prelude's page and element revivers make a `Page` for each tab they meet, so
/// the order `revive` meets brands in is the order of `context.pages()`. It meets them depth
/// first, in each object's key order, and hands each brand to its reviver whole, assigning what
/// comes back in place.
#[test]
fn revive_visits_brands_in_pre_order_and_key_order_and_replaces_each_whole() -> Result<(), TestError>
{
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r#"
            const seen = [];
            __neo.brands.register("probe", (json) => { seen.push(json.n); return { revived: json.n }; });
            const items = [{ $neo: "probe", n: "b" }, { deeper: { $neo: "probe", n: "c" } }];
            // Key order puts integer keys first, ascending: 2, 10, zeta, alpha.
            const value = {
              zeta: items,
              2: { $neo: "probe", n: "a", inner: { $neo: "probe", n: "never" } },
              alpha: { $neo: "probe", n: "d" },
              10: { plain: true },
            };
            let feed = true;
            __neo.onReply((envelope) => {
              if (feed) { feed = false; envelope.ok = value; }
            });
            const revived = await __neo.invoke("test.echo", {});
            return {
              seen,
              inPlace: revived === value && revived.zeta === items,
              whole: revived[2],
              revived: [revived.zeta[0], revived.zeta[1].deeper, revived.alpha],
              untouched: revived[10],
            };"#;
        assert_eq!(
            returned(script(&rig, code).await?)?,
            json!({
                "seen": ["a", "b", "c", "d"],
                "inPlace": true,
                "whole": { "revived": "a" },
                "revived": [{ "revived": "b" }, { "revived": "c" }, { "revived": "d" }],
                "untouched": { "plain": true },
            }),
            "depth first in key order, a brand whole and never what is inside it"
        );
        Ok(())
    })
}

#[test]
fn the_run_page_learns_its_tab_from_the_call_that_opened_it() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r#"
            const before = page.toJSON();
            const touched = await __neo.invoke("test.touch", { page });
            return { before, after: page.toJSON(), touched };"#;
        let value = returned(script(&rig, code).await?)?;
        assert_eq!(
            value.pointer("/before/id"),
            Some(&Value::Null),
            "no tab until a call needs one"
        );
        assert_eq!(
            value.pointer("/after/tab"),
            value.pointer("/touched/tab"),
            "the same `page` object now names the tab the door opened for it"
        );
        Ok(())
    })
}

#[test]
fn bytes_cross_the_bridge_both_ways() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r#"
            const results = [];
            for (let length = 0; length <= 5; length++) {
              const bytes = new Uint8Array(length).map((_, i) => 250 + i);
              const back = await __neo.invoke("test.echo", { bytes });
              results.push(back.bytes instanceof Uint8Array ? Array.from(back.bytes) : back.bytes);
            }
            return results;"#;
        assert_eq!(
            returned(script(&rig, code).await?)?,
            json!([
                [],
                [250],
                [250, 251],
                [250, 251, 252],
                [250, 251, 252, 253],
                [250, 251, 252, 253, 254]
            ]),
            "every padding length survives the bytes brand"
        );
        Ok(())
    })
}

#[test]
fn input_the_codec_refuses_is_refused_and_recorded_by_the_door() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r#"
            try {
              await __neo.invoke("test.echo", { nested: { callback: () => 1 } });
            } catch (e) {
              return { code: e.code, owner: e.owner, delivery: e.delivery, message: e.message, op: e.operationId };
            }"#;
        let refusal = returned(script(&rig, code).await?)?;
        assert_eq!(refusal.get("code"), Some(&json!("invalid_input")));
        assert_eq!(refusal.get("owner"), Some(&json!("script")));
        assert_eq!(refusal.get("delivery"), Some(&json!("not_sent")));
        let message = refusal
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(
            message.contains("input.nested.callback is a function"),
            "the codec's reason, with its path: {message}"
        );
        let operation = refusal
            .get("op")
            .and_then(Value::as_str)
            .ok_or("an op id")?
            .parse()?;
        rig.sink.flush().await?;
        let row = rig
            .store
            .operation(operation)
            .await?
            .ok_or("the refusal is on the record")?;
        assert_eq!(row.outcome, Some(Outcome::Error));
        assert_eq!(row.delivery, Some(Delivery::NotSent));
        Ok(())
    })
}

#[test]
fn the_return_value_is_made_json_safe() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r"
            const loop = { a: 1 };
            loop.self = loop;
            return { big: 10n, fn: () => 1, loop, missing: undefined, bytes: new Uint8Array([1]) };";
        assert_eq!(
            returned(script(&rig, code).await?)?,
            json!({
                "big": "10",
                "loop": { "a": 1, "self": "[Circular]" },
                "bytes": { "$neo": "bytes", "base64": "AQ==" },
            }),
            "a model always gets JSON back, whatever the script returned"
        );
        assert_eq!(
            returned(script(&rig, "const nothing = 1;").await?)?,
            Value::Null,
            "no return is null"
        );
        Ok(())
    })
}

#[test]
fn a_return_value_over_the_limit_is_refused() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let error = failed(script(&rig, "return 'x'.repeat(300 * 1024);").await?)?;
        assert_eq!(
            (error.owner(), error.code()),
            (ErrorOwner::Script, ErrorCode::RESULT_TOO_LARGE)
        );
        Ok(())
    })
}

#[test]
fn console_lines_come_back_with_their_level() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let end = script(
            &rig,
            "console.log('a', 1, { b: 2 }); console.warn('careful'); return 0;",
        )
        .await?;
        let (lines, dropped) = end.logs.into_parts();
        assert_eq!(
            lines,
            vec![
                LogLine {
                    level: LogLevel::Log,
                    text: r#"a 1 {"b":2}"#.to_owned()
                },
                LogLine {
                    level: LogLevel::Warn,
                    text: "careful".to_owned()
                },
            ]
        );
        assert_eq!(dropped, 0);
        Ok(())
    })
}

#[test]
fn a_full_heap_is_out_of_memory() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let end = script(
            &rig,
            "const chunks = []; for (;;) chunks.push(new Array(1e6).fill(7));",
        )
        .await?;
        let error = failed(end)?;
        assert_eq!(
            (error.owner(), error.code()),
            (ErrorOwner::Script, ErrorCode::OUT_OF_MEMORY),
            "the 64 MiB heap cap stops the script, not the process: {error}"
        );
        Ok(())
    })
}

#[test]
fn an_endless_loop_is_a_long_task() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let started = Instant::now();
        let error = failed(script(&rig, "for (;;) {}").await?)?;
        assert_eq!(
            (error.owner(), error.code()),
            (ErrorOwner::Script, ErrorCode::LONG_TASK)
        );
        assert!(
            started.elapsed() < Defaults::JS_SLICE_BUDGET * 2,
            "the slice budget, not the run's deadline, frees the worker"
        );
        let microtasks = failed(script(&rig, "for (;;) { await null; }").await?)?;
        assert_eq!(
            microtasks.code(),
            ErrorCode::LONG_TASK,
            "a microtask loop never yields to tokio either"
        );
        Ok(())
    })
}

/// A script that never yields keeps its worker until the slice budget or the deadline (both
/// read by the interrupt handler itself); one that yields lets its run's stop land, and then
/// the handler ends the JS in the middle of a busy stretch.
#[test]
fn a_stop_interrupts_running_javascript_at_once() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let stopper = Arc::clone(&run);
        tokio::spawn(async move {
            time::sleep(Duration::from_millis(200)).await;
            stopper.stop(StopReason::CallerLeft);
        });
        let started = Instant::now();
        let busy = r"
            for (;;) {
              const end = Date.now() + 50;
              while (Date.now() < end) {}
              await new Promise((resolve) => setTimeout(resolve, 1));
            }";
        let end = run_code(&rig, &run, busy).await;
        assert!(
            matches!(end.outcome, ScriptOutcome::Stopped),
            "{:?}",
            end.outcome
        );
        assert!(
            started.elapsed() < Defaults::JS_SLICE_BUDGET,
            "the interrupt handler reads the stop, well before the slice budget"
        );
        assert_eq!(run.stop_reason(), Some(StopReason::CallerLeft));
        Ok(())
    })
}

#[test]
fn a_script_awaiting_forever_ends_a_grace_after_the_stop() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let stopper = Arc::clone(&run);
        tokio::spawn(async move {
            time::sleep(Duration::from_millis(100)).await;
            stopper.stop(StopReason::Shutdown);
        });
        let started = Instant::now();
        let end = run_code(&rig, &run, "await new Promise(() => {});").await;
        assert!(
            matches!(end.outcome, ScriptOutcome::Stopped),
            "{:?}",
            end.outcome
        );
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Defaults::TEARDOWN_GRACE && elapsed < Defaults::TEARDOWN_GRACE * 2,
            "no JS runs to interrupt, so the grace ends it: {elapsed:?}"
        );
        Ok(())
    })
}

#[test]
fn the_deadline_stops_a_busy_script() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (run, _guard) = rig.run(Duration::from_millis(300), None)?;
        let end = run_code(&rig, &run, "for (;;) {}").await;
        assert!(
            matches!(end.outcome, ScriptOutcome::Stopped),
            "{:?}",
            end.outcome
        );
        assert_eq!(
            run.stop_reason(),
            Some(StopReason::Deadline),
            "the interrupt handler watches the clock, so a busy run still meets its deadline"
        );
        Ok(())
    })
}

#[test]
fn timers_wait_clear_and_stop_at_the_deadline() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r"
            let fired = false;
            const id = setTimeout(() => { fired = true; }, 10);
            clearTimeout(id);
            await new Promise((resolve) => setTimeout(resolve, 50));
            return fired;";
        assert_eq!(
            returned(script(&rig, code).await?)?,
            json!(false),
            "a cleared timer never fires"
        );

        let (run, _guard) = rig.run(Duration::from_millis(400), None)?;
        let started = Instant::now();
        let end = run_code(
            &rig,
            &run,
            "await new Promise((resolve) => setTimeout(resolve, 600000));",
        )
        .await;
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "the deadline caps the timer"
        );
        assert_eq!(
            run.stop_reason(),
            Some(StopReason::Deadline),
            "waking at the deadline stops the run there: {:?}",
            end.outcome
        );
        Ok(())
    })
}

/// Browser-oracle URL/base64 cases, run through the production bootstrap and both preludes.
/// Expectations agree between Node and Bun except iterator object key order, which is not
/// specified (Bun's order here), and the owner's pinned opaque-path/object-URL decisions.
#[test]
fn web_globals_match_the_browser_cases_in_the_real_run_realm() -> Result<(), TestError> {
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        code: String,
        expected: Value,
    }
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/web_globals.json"))?;
        for case in cases {
            let body = serde_json::to_string(&case.code)?;
            let code = format!(
                r"const parts = (u) => [u.href, u.origin, u.protocol, u.username, u.password,
                    u.host, u.hostname, u.port, u.pathname, u.search, u.hash];
                try {{
                    const value = new Function('parts', {body})(parts);
                    return {{ ok: value === undefined ? '<undefined>' : value }};
                }} catch (error) {{ return {{ threw: error.name }}; }}",
            );
            let actual = returned(script(&rig, &code).await?)
                .map_err(|error| format!("{}: {error}", case.name))?;
            assert_eq!(actual, case.expected, "{}", case.name);
        }
        Ok(())
    })
}

/// A same-built bare-context baseline would miss a switch away from `JS_NewContext` that
/// silently dropped QuickJS-ng's base64/DOMException intrinsics. Pin the contract separately.
#[test]
fn web_globals_have_standard_descriptors_arity_and_shadowing() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r"
            const names = ['URL', 'URLSearchParams', 'atob', 'btoa', 'DOMException'];
            return names.map((name) => {
                const d = Object.getOwnPropertyDescriptor(globalThis, name);
                return [typeof d.value, d.value.length, d.writable, d.enumerable, d.configurable];
            });";
        assert_eq!(
            returned(script(&rig, code).await?)?,
            json!([
                ["function", 1, true, false, true],
                ["function", 0, true, false, true],
                ["function", 1, true, false, true],
                ["function", 1, true, false, true],
                ["function", 2, true, false, true]
            ]),
            "the complete production realm exposes the standard interfaces"
        );
        assert_eq!(
            returned(script(&rig, "const URL = 1; let atob = 2; return [URL, atob];").await?)?,
            json!([1, 2]),
            "web interfaces are globals, not extra agent function parameters"
        );
        Ok(())
    })
}

#[test]
fn invalid_web_global_input_is_owned_by_the_script_with_its_call_site() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        for (code, name) in [
            ("new URL('nope');", "TypeError"),
            ("atob('@');", "InvalidCharacterError"),
        ] {
            let error = failed(script(&rig, code).await?)?;
            assert_eq!(
                (error.owner(), error.code()),
                (ErrorOwner::Script, ErrorCode::SCRIPT_ERROR),
                "a local input error is never blamed on the host or Neo"
            );
            assert!(
                error.message().contains(name) && error.message().contains("run:1:"),
                "the agent's call site survives the factory frame: {error}"
            );
        }
        Ok(())
    })
}

#[test]
fn web_globals_preserve_binary_strings_and_empty_search_parameters() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r"
            const bytes = Array.from({ length: 256 }, (_, i) => String.fromCharCode(i)).join('');
            const u = new URL('http://example.test/?a=1');
            const p = u.searchParams;
            u.href = 'https://other.test/?b=2';
            const refreshed = [...p];
            p.delete('b');
            return [atob(btoa(bytes)) === bytes, new URLSearchParams(null).size,
                new URLSearchParams(undefined).size, refreshed, p === u.searchParams, u.href,
                typeof URL.revokeObjectURL];";
        assert_eq!(
            returned(script(&rig, code).await?)?,
            json!([
                true,
                0,
                0,
                [["b", "2"]],
                true,
                "https://other.test/",
                "undefined"
            ]),
            "binary bytes round-trip and URL/searchParams keep one live list across href changes"
        );
        Ok(())
    })
}

#[test]
fn web_idl_search_parameters_convert_once_and_iterators_stay_exhausted() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r"
            let reads = 0;
            const input = { get [Symbol.iterator]() {
                reads++;
                return function* () { yield ['a', '1']; };
            } };
            const params = new URLSearchParams(input);
            const iterator = params.entries();
            iterator.next(); iterator.next();
            params.append('b', '2');
            let conversions = 0;
            const item = { toString() { conversions++; return 'x'; } };
            try { new URLSearchParams([[item, item, item]]); } catch {}
            const record = new URLSearchParams({ '\ud800': 'first', '\ud801': 'second' });
            return [reads, iterator.next().done, conversions, record.toString()];";
        assert_eq!(
            returned(script(&rig, code).await?)?,
            json!([1, true, 3, "%EF%BF%BD=second"]),
            "WebIDL converts a sequence before checking pair length, and normalizes record keys"
        );
        Ok(())
    })
}

#[test]
fn web_parameter_constructor_converts_all_pairs_before_validation() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r"
            function attempt(throwOnConversion) {
                const events = [];
                const lateError = new Error('later');
                const later = { toString() {
                    events.push('convert');
                    if (throwOnConversion) throw lateError;
                    return 'b';
                } };
                function* inner() {
                    try {
                        yield later;
                        yield '2';
                        events.push('inner exhausted');
                    } finally { events.push('inner closed'); }
                }
                function* outer() {
                    try {
                        yield ['a'];
                        yield inner();
                        events.push('outer exhausted');
                    } finally { events.push('outer closed'); }
                }
                try { new URLSearchParams(outer()); }
                catch (e) { return [e === lateError ? 'conversion error' : e.name, events]; }
            }
            return [attempt(true), attempt(false)];";
        assert_eq!(
            returned(script(&rig, code).await?)?,
            json!([
                [
                    "conversion error",
                    ["convert", "inner closed", "outer closed"]
                ],
                [
                    "TypeError",
                    [
                        "convert",
                        "inner exhausted",
                        "inner closed",
                        "outer exhausted",
                        "outer closed"
                    ]
                ]
            ])
        );
        Ok(())
    })
}

#[test]
fn web_base64_keeps_forgiving_decode_and_dom_exception_errors() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r"
            const attempt = (f) => {
                try { return f(); } catch (e) {
                    return [e.name, e.code, e instanceof DOMException, e instanceof Error];
                }
            };
            const invalid = ['Y', 'YQ=', 'YQ===', '@', 'YQ\v=='].map(s => attempt(() => atob(s)));
            const missing = [atob, btoa].map(f => { try { f(); } catch (e) { return e.name; } });
            const u = new URL('http://example.test/');
            u.searchParams.toString = () => 'forged';
            u.searchParams.append('a', '1');
            return [atob(' Y\tQ\n=\f=\r '), atob('YR=='), atob('YQ'), invalid,
                attempt(() => btoa(String.fromCharCode(256))), attempt(() => btoa(String.fromCharCode(0xd800))), missing, u.search];";
        let invalid = json!(["InvalidCharacterError", 5, true, true]);
        assert_eq!(
            returned(script(&rig, code).await?)?,
            json!([
                "a",
                "a",
                "a",
                [&invalid, &invalid, &invalid, &invalid, &invalid],
                &invalid,
                &invalid,
                ["TypeError", "TypeError"],
                "?a=1"
            ]),
            "only Infra's ASCII whitespace is ignored; binary strings reject non-Latin-1 values"
        );
        Ok(())
    })
}

/// A two-byte pair must not buy 48 bytes of Rust storage outside the 64 MiB realm cap.
/// The native's lower-bound scan refuses this result before allocating decoded pairs/output.
#[test]
fn pair_dense_web_parameters_fail_with_the_existing_memory_outcome() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let error = failed(script(&rig, "new URLSearchParams('=&'.repeat(1 << 23));").await?)?;
        assert_eq!(
            (error.owner(), error.code()),
            (ErrorOwner::Script, ErrorCode::OUT_OF_MEMORY),
            "native bounds use QuickJS's allocation exception and preserve resource-limit ownership"
        );
        Ok(())
    })
}

#[test]
fn web_url_setters_apply_after_conversion_to_the_current_live_state() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r"
            const u = new URL('https://a.test/start?a=1');
            const p = u.searchParams;
            u.pathname = { toString() { u.search = '?b=2'; return '/next'; } };
            const linked = [u.href, p.toString(), new URL(u.href).searchParams.toString(), p === u.searchParams];
            u.pathname = { toString() { u.hostname = 'new.test'; return '/last'; } };
            const host = u.href;
            u.search = { toString() { u.pathname = '/changed'; return '?q=1'; } };
            const search = [u.href, p.toString()];
            try { u.href = { toString() { u.search = '?kept=1'; return 'not a URL'; } }; } catch {}
            let coerced = false;
            let illegalReceiver = false;
            const setHref = Object.getOwnPropertyDescriptor(URL.prototype, 'href').set;
            try { setHref.call({}, { toString() { coerced = true; return u.href; } }); }
            catch (e) { illegalReceiver = e instanceof TypeError; }
            return [linked, host, search, u.href, p.toString(), illegalReceiver, coerced];";
        assert_eq!(
            returned(script(&rig, code).await?)?,
            json!([
                ["https://a.test/next?b=2", "b=2", "b=2", true],
                "https://new.test/last?b=2",
                ["https://new.test/changed?q=1", "q=1"],
                "https://new.test/changed?kept=1",
                "kept=1",
                true,
                false
            ]),
            "conversion-time changes survive, and a held parameter list agrees with href"
        );
        Ok(())
    })
}

#[test]
fn web_linked_parameter_append_keeps_both_views_after_caught_oom() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r"
            const u = new URL('https://a.test/?a=1');
            const p = u.searchParams;
            const keys = p.keys();
            const first = keys.next().value;
            let error;
            try { p.append('x', 'a'.repeat(9600000)); }
            catch (e) { error = [e.name, e.message]; }
            const caught = [u.href, p.size, p.get('x'), p.toString(), p === u.searchParams];
            p.append({ toString() { u.search = '?b=2'; return 'c'; } }, '3');
            const added = keys.next().value;
            u.href = 'https://b.test/?x=1&y=2&z=3';
            return [error, caught, first, added, keys.next().value, u.href,
                p.toString(), p === u.searchParams];";
        assert_eq!(
            returned(script(&rig, code).await?)?,
            json!([
                ["InternalError", "out of memory"],
                ["https://a.test/?a=1", 1, null, "a=1", true],
                "a",
                "c",
                "z",
                "https://b.test/?x=1&y=2&z=3",
                "x=1&y=2&z=3",
                true
            ])
        );
        Ok(())
    })
}

#[test]
fn web_linked_parameter_rewrites_keep_both_views_after_caught_oom() -> Result<(), TestError> {
    let runtime = Runtime::new()?;
    let context = Context::full(&runtime)?;
    let code = r"
        (() => {
            // The raw href fits the native budget. Updating it would encode each '%' as
            // '%25', so the setter rejects the longer input even after removing a pair.
            const raw = '%'.repeat(180);
            const u = new URL('https://a.test/?b=' + raw + '&a=1&z=9');
            const p = u.searchParams;
            const href = u.href;
            const cases = [() => p.delete('z'), () => p.sort(), () => p.set('b', raw + '%')];
            const failures = cases.map((mutate) => {
                try { mutate(); } catch (e) {
                    return [e.name, u.href === href, p === u.searchParams, [...p.keys()], p.get('b') === raw];
                }
            });
            p.set('b', '2');
            return JSON.stringify([failures, u.search, p.toString()]);
        })()";
    let result = context.with(|ctx| -> rquickjs::Result<String> {
        install_web_globals(&ctx, 4096)?;
        ctx.eval(code)
    })?;
    let failure = json!(["InternalError", true, true, ["b", "a", "z"], true]);
    assert_eq!(
        serde_json::from_str::<Value>(&result)?,
        json!([
            [&failure, &failure, &failure],
            "?b=2&a=1&z=9",
            "b=2&a=1&z=9"
        ])
    );
    Ok(())
}

#[test]
fn web_parameter_methods_check_receivers_before_coercing_arguments() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r"
            const proto = URLSearchParams.prototype;
            const invalidReceivers = [Object.create(proto), new Proxy(new URLSearchParams(), {})];
            let conversions = 0;
            const arg = { toString() { conversions++; throw new Error('coercion ran'); } };
            const invalid = invalidReceivers.map(receiver =>
                ['append', 'delete', 'get', 'getAll', 'has', 'set', 'forEach'].map(method => {
                    try { Reflect.apply(proto[method], receiver, [arg, arg]); }
                    catch (e) { return e instanceof TypeError; }
                })
            );
            return [invalid, conversions];";
        assert_eq!(
            returned(script(&rig, code).await?)?,
            json!([[vec![true; 7], vec![true; 7]], 0])
        );
        Ok(())
    })
}

#[test]
fn web_parameter_serialization_never_calls_inherited_json_hooks() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r"
            const results = [];
            for (const prototype of [Array.prototype, Object.prototype]) {
                const original = Object.getOwnPropertyDescriptor(prototype, 'toJSON');
                const u = new URL('https://a.test/');
                const p = u.searchParams;
                let calls = 0;
                try {
                    Object.defineProperty(prototype, 'toJSON', {
                        configurable: true,
                        get() { calls++; return () => { throw new Error('internal arrays must never serialize'); }; }
                    });
                    p.append('x', 'y');
                    p.append('a b', '+&');
                    results.push([u.href, p.toString(), calls, p === u.searchParams]);
                } finally {
                    if (original) Object.defineProperty(prototype, 'toJSON', original);
                    else delete prototype.toJSON;
                }
            }
            return results;";
        let expected = json!(["https://a.test/?x=y&a+b=%2B%26", "x=y&a+b=%2B%26", 0, true]);
        assert_eq!(
            returned(script(&rig, code).await?)?,
            json!([&expected, &expected]),
            "private list encoding passes only string pairs; user toJSON getters never run"
        );
        Ok(())
    })
}
