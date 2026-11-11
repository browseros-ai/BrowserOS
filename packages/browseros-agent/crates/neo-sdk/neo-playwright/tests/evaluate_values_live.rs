//! `page.evaluate` and `page.waitForFunction` as agents write them, run by the real prelude in
//! QuickJS (Runner N, `tests/support/js_runner.rs`) through `Playwright::invoke` on a real
//! browser: values come back as Playwright 1.64.1 gives them, a field's value in whatever shape
//! a script gives it, and the main world behaves as upstream's does (user activation, a
//! predicate that keeps state). One test drives the op directly, to read what the crate asked
//! of its host around each send.
//!
//! Needs a throwaway BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005):
//! `cargo test -p neo-playwright --test evaluate_values_live -- --ignored`.
//! Every test opens its own tab and closes it.

#[path = "support/fixture_server.rs"]
#[expect(
    dead_code,
    reason = "the nested-frame fixture and scripts serve frames_live"
)]
mod fixture_server;
#[path = "support/js_runner.rs"]
mod js_runner;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "interceptions, clicks and adopted tabs serve other live tests"
)]
mod live_run;

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use fixture_server::{FixtureServer, Page};
use js_runner::{OpHost, run_script};
use live_run::{EnvCall, LiveRun, TestPage, TestResult, brand};
use serde_json::{Value, json};

/// Two form fields, empty until a test types into them.
const LOGIN: &str = r#"<!doctype html><html><head><title>Login</title></head><body>
<label>Password <input id="pw" type="password"></label>
<label>Code <input id="code" type="password"></label>
</body></html>"#;

const POPUP: &str = "<!doctype html><title>Popup</title><p>popup</p>";

/// Describes a value as JavaScript sees it, `[typeof, Object.prototype.toString, String]`, with
/// arrays and plain objects described item by item, a Date by its ISO text, `-0` as itself and
/// a cycle as `"cycle"`, so the description is JSON whatever the value is.
const DESCRIBE: &str = r"const describe = (value, seen = new Set()) => {
  if (value !== null && typeof value === 'object') {
    if (seen.has(value)) return 'cycle'
    seen.add(value)
  }
  const tag = Object.prototype.toString.call(value)
  const text = Object.is(value, -0) ? '-0' : tag === '[object Date]' ? value.toISOString() : String(value)
  if (Array.isArray(value)) return [typeof value, tag, value.map((item) => describe(item, seen))]
  if (tag === '[object Object]')
    return [typeof value, tag, Object.fromEntries(Object.entries(value).map(([key, item]) => [key, describe(item, seen)]))]
  return [typeof value, tag, text]
}";

fn pages() -> HashMap<String, Page> {
    HashMap::from([
        ("/login.html".to_owned(), Page::html(LOGIN)),
        ("/popup.html".to_owned(), Page::html(POPUP)),
    ])
}

/// A live run with a tab on `/login.html`, handed to `body` and closed whatever happened.
async fn on_login<F, Fut>(body: F) -> TestResult
where
    F: FnOnce(Arc<LiveRun>, TestPage) -> Fut,
    Fut: Future<Output = TestResult>,
{
    let run = Arc::new(LiveRun::connect().await?);
    let server = FixtureServer::start(pages()).await?;
    let page = run.new_page().await?;
    let opened = run
        .invoke(
            "page.goto",
            json!({ "call": { "timeoutMs": 30_000 }, "page": brand(&page),
                    "url": server.url("127.0.0.1", "/login.html") }),
        )
        .await;
    let result = match opened {
        Ok(_) => body(Arc::clone(&run), page.clone()).await,
        Err(error) => Err(error.into()),
    };
    run.close_page(&page).await;
    result
}

/// Runs `body` as an agent's script on `page`, with [`DESCRIBE`] in scope.
async fn script(run: &Arc<LiveRun>, page: &TestPage, body: &str) -> Result<Value, String> {
    let host: Arc<dyn OpHost> = Arc::<LiveRun>::clone(run);
    let init = json!({ "page": brand(page) });
    run_script(&host, &init, &format!("{DESCRIBE}\n{body}")).await
}

/// Types `value` into the field `id`, from the page's own world, as the user would.
async fn type_into(run: &LiveRun, page: &TestPage, id: &str, value: &str) -> TestResult {
    run.eval(
        page,
        &format!("document.getElementById({id:?}).value = {value:?}"),
    )
    .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_page_values_come_back_as_playwright_gives_them() -> TestResult {
    on_login(|run, page| async move {
        let described = script(
            &run,
            &page,
            "const values = await page.evaluate(() => [undefined, null, NaN, -0, Infinity, 10n, \
               new Date(0), /a+b/gi, new TypeError('no'), Uint8Array.of(1, 2), new Map([[1, 2]]), \
               'text', 42])\n\
             const cycle = await page.evaluate(() => { const o = { n: 1 }; o.self = o; return o })\n\
             const nothing = await page.evaluate(() => undefined)\n\
             return { values: values.map((value) => describe(value)), cycle: describe(cycle), \
               same: cycle.self === cycle, nothing: describe(nothing) }",
        )
        .await?;
        assert_eq!(
            described,
            json!({
                "values": [
                    ["undefined", "[object Undefined]", "undefined"],
                    ["object", "[object Null]", "null"],
                    ["number", "[object Number]", "NaN"],
                    ["number", "[object Number]", "-0"],
                    ["number", "[object Number]", "Infinity"],
                    ["bigint", "[object BigInt]", "10"],
                    ["object", "[object Date]", "1970-01-01T00:00:00.000Z"],
                    ["object", "[object RegExp]", "/a+b/gi"],
                    ["object", "[object Error]", "TypeError: no"],
                    ["object", "[object Uint8Array]", "1,2"],
                    ["object", "[object Object]", {}],
                    ["string", "[object String]", "text"],
                    ["number", "[object Number]", "42"],
                ],
                "cycle": ["object", "[object Object]", {
                    "n": ["number", "[object Number]", "1"],
                    "self": "cycle",
                }],
                "same": true,
                "nothing": ["undefined", "[object Undefined]", "undefined"],
            }),
            "every value as upstream's serializer carries it; a Map has no own keys, so it is {{}}"
        );
        Ok(())
    })
    .await
}

/// A field's value comes back in the shape the script gave it, as upstream returns it: as a
/// key, in an Error, as a number, a `BigInt`, a Date's time and inside a string.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_typed_value_comes_back_in_the_shape_the_script_gives_it() -> TestResult {
    on_login(|run, page| async move {
        type_into(&run, &page, "pw", "hunter2-typed").await?;
        type_into(&run, &page, "code", "482913").await?;
        let got = script(
            &run,
            &page,
            "const got = await page.evaluate(() => {\n\
               const pw = document.getElementById('pw').value\n\
               const code = document.getElementById('code').value\n\
               return { [pw]: 'a key', error: new Error('bad ' + pw), number: Number(code), \
                 big: BigInt(code), date: new Date(Number(code)), text: 'you typed ' + pw }\n\
             })\n\
             return { keys: Object.keys(got), error: describe(got.error), number: describe(got.number), \
               big: describe(got.big), date: describe(got.date), text: got.text }",
        )
        .await?;
        assert_eq!(
            got,
            json!({
                "keys": ["hunter2-typed", "error", "number", "big", "date", "text"],
                "error": ["object", "[object Error]", "Error: bad hunter2-typed"],
                "number": ["number", "[object Number]", "482913"],
                "big": ["bigint", "[object BigInt]", "482913"],
                "date": ["object", "[object Date]", "1970-01-01T00:08:02.913Z"],
                "text": "you typed hunter2-typed",
            }),
            "a number stays a number, a BigInt a BigInt and a Date a Date"
        );
        Ok(())
    })
    .await
}

/// A typed array carries a code as its elements, and comes back as the same typed array, for
/// the integer, float and 64-bit kinds alike.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_code_in_a_typed_array_comes_back_as_that_array() -> TestResult {
    on_login(|run, page| async move {
        type_into(&run, &page, "code", "482913").await?;
        let got = script(
            &run,
            &page,
            "const got = await page.evaluate(() => {\n\
               const code = document.getElementById('code').value\n\
               return [Int32Array.of(1, Number(code)), Float64Array.of(Number(code)), \
                 BigInt64Array.of(BigInt(code))]\n\
             })\n\
             return got.map((value) => describe(value))",
        )
        .await?;
        assert_eq!(
            got,
            json!([
                ["object", "[object Int32Array]", "1,482913"],
                ["object", "[object Float64Array]", "482913"],
                ["object", "[object BigInt64Array]", "482913"],
            ]),
            "an Int32Array, a Float64Array and a BigInt64Array element holding the code"
        );
        Ok(())
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_window_open_inside_evaluate_opens_a_popup() -> TestResult {
    on_login(|run, page| async move {
        let opened = script(
            &run,
            &page,
            "const [popup] = await Promise.all([\n\
               page.waitForEvent('popup', { timeout: 5000 }),\n\
               page.evaluate(() => { window.open('/popup.html') }),\n\
             ])\n\
             const url = popup.url()\n\
             await popup.close()\n\
             return url",
        )
        .await?;
        assert!(
            opened
                .as_str()
                .is_some_and(|url| url.ends_with("/popup.html")),
            "the script runs with user activation, as upstream's does: {opened}"
        );
        Ok(())
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_predicate_keeping_state_in_its_argument_resolves() -> TestResult {
    on_login(|run, page| async move {
        let polls = script(
            &run,
            &page,
            "const handle = await page.waitForFunction((state) => ++state.polls >= 3 && state.polls, \
               { polls: 0 }, { timeout: 5000 })\n\
             return await handle.jsonValue()",
        )
        .await?;
        assert_eq!(
            polls,
            json!(3),
            "the argument is parsed once and the predicate called with it on every poll"
        );
        Ok(())
    })
    .await
}

/// A page value shaped like one of Neo's handle brands is the plain object it is: Playwright's
/// format writes an object's keys as data (`{k: "$neo"}`), so the bootstrap's reviver, which
/// turns a `{"$neo": ...}` object in a result into a handle, never meets one.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_value_shaped_like_a_handle_stays_a_plain_object() -> TestResult {
    on_login(|run, page| async move {
        let got = script(
            &run,
            &page,
            "const value = await page.evaluate(() => ({ $neo: 'page', tab: 1 }))\n\
             return [Object.getPrototypeOf(value) === Object.prototype, Object.keys(value), \
               typeof value.goto]",
        )
        .await?;
        assert_eq!(got, json!([true, ["$neo", "tab"], "undefined"]));
        Ok(())
    })
    .await
}

/// upstream's server normalizes the function before anything reaches the page; one it cannot
/// read is refused with its message and its prefix, and the op is never marked sent.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_function_upstream_cannot_read_is_refused_with_nothing_sent() -> TestResult {
    on_login(|run, page| async move {
        run.clear_calls();
        let refused = script(
            &run,
            &page,
            "const getter = Object.getOwnPropertyDescriptor({ get x() { return 1 } }, 'x').get\n\
             try { await page.evaluate(getter) } catch (e) { return [e.message, e.code] }",
        )
        .await?;
        assert_eq!(
            refused,
            json!([
                "page.evaluate: Passed function is not well-serializable!",
                "invalid_input"
            ])
        );
        assert!(
            !run.calls().contains(&EnvCall::MarkSent),
            "nothing was sent: {:?}",
            run.calls()
        );
        Ok(())
    })
    .await
}

/// `page.evaluate` as the prelude sends it, its argument `undefined`.
fn evaluate_input(page: &TestPage, function: &str) -> Value {
    json!({ "page": brand(page), "script": { "run": {
        "expression": function, "isFunction": true, "arg": r#"{"v":"undefined"}"#,
    } } })
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_each_send_is_rechecked_then_marked_with_nothing_between() -> TestResult {
    on_login(|run, page| async move {
        run.clear_calls();
        let one = run
            .invoke("page.evaluate", evaluate_input(&page, "() => 1"))
            .await?;
        assert_eq!(one, json!(1));
        assert_eq!(
            run.calls(),
            [EnvCall::Recheck, EnvCall::MarkSent],
            "one script, one recheck, then the mark"
        );
        run.eval(&page, "window.polls = 0").await?;
        run.clear_calls();
        let mut input = evaluate_input(&page, "() => ++window.polls >= 3");
        input["call"] = json!({ "timeoutMs": 5000 });
        let waited = run.invoke("page.waitForFunction", input).await?;
        assert_eq!(waited["value"], json!(true));
        assert_eq!(
            run.calls(),
            [
                EnvCall::Recheck,
                EnvCall::MarkSent,
                EnvCall::Recheck,
                EnvCall::MarkSent,
                EnvCall::Recheck,
                EnvCall::MarkSent,
            ],
            "every poll is a send of its own"
        );
        Ok(())
    })
    .await
}
