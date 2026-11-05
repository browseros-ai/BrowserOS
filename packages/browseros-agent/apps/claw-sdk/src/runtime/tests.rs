//! The runtime end to end: a real QuickJS realm, the bridge and the real dispatcher on the fake
//! browser, with the trace store read back. Each test drives one script through `run_script`
//! and asserts what the run service reads from it.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use browseros_trace::{Delivery, ErrorOwner, Outcome};
use rquickjs::{AsyncContext, AsyncRuntime};
use serde_json::{Value, json};
use tokio::time::{self, Instant};

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
fn a_fresh_realm_adds_only_the_natives_and_the_bootstrap_to_quickjs() -> Result<(), TestError> {
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
            BTreeSet::from(["__neo", "__neo_invoke", "__neo_log", "__neo_sleep"]),
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
