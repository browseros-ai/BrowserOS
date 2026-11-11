//! The main-world scripts on a real BrowserOS, through MCP `run` (S4, S5): each API that runs the
//! agent's script or HTML in the page behaves as Playwright 1.64.1's, the frame forms in a child
//! frame included, an owner's lock that blocks reads withholds what a script that was already
//! running read, of its own page or across an opener link, a popup's script is sent only while no
//! one holds its opener, which the script can act on, and a lock taken on the opener while the
//! popup's script runs is not settled.
//!
//! ```text
//!  page.evaluate, frame.evaluate ──────────────────────────────────▶ page.evaluate
//!  page.waitForFunction, frame.waitForFunction ────────────────────▶ page.waitForFunction
//!  locator.evaluate, elementHandle.evaluate, $eval (page, frame, handle) ─▶ locator.evaluate
//!  locator.evaluateAll, $$eval (page, frame, handle) ──────────────▶ locator.evaluateAll
//!  page.setContent, frame.setContent ──────────────────────────────▶ page.setContent
//!    each op: the crate's evaluate guard sweeps, sends, sweeps, masks ─ the door: the rules again
//!    as a read of the tab ─▶ the realm, and the trace the host reads back
//! ```
//!
//! Each case's script returns what it saw, as the realm saw it, and the test compares that with
//! what Playwright gives. Like `e2e_browseros.rs`, it runs only when `CLAW_SDK_E2E_BROWSER` names
//! a BrowserOS binary.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::error::Error;
use std::future::Future;
use std::net::Ipv4Addr;
use std::pin::Pin;
use std::time::Duration;
use std::{fs, io};

use browseros_rig::{CaptchaKey, HOST_TOKEN, PATIENCE, Rig, TestResult};
use rmcp::model::CallToolResult;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::time;

/// The whole drive; see `e2e_browseros.rs` for why the budget sits inside the test.
const TEST_BUDGET: Duration = Duration::from_secs(240);

/// The page every case loads into the run's own `page`: a field, two items, a same-origin child
/// frame, and a flag the page sets a moment after load.
const PAGE: &str = r#"
const html = `<title>Main world</title>
<input id="user" value="ada">
<div id="list"><p class="item">one</p><p class="item">two</p></div>
<iframe srcdoc="<p id=inner>in frame</p>"></iframe>
<script>setTimeout(() => { window.ready = 'yes' }, 300)</script>`;
await page.goto('data:text/html,' + encodeURIComponent(html));
await page.waitForLoadState('load');
// What a failing call threw, as a script reads it: the first line of its message, since the
// rest is upstream's stack or call log.
const failed = async (call) => {
  try {
    await call();
    return 'resolved';
  } catch (error) {
    const { name, owner, code, delivery } = error;
    return { name, owner, code, delivery, message: String(error.message).split('\n')[0] };
  }
};
"#;

#[tokio::test(flavor = "multi_thread")]
async fn page_evaluate_behaves_as_playwright_on_browseros() -> TestResult {
    on_browseros(|rig| Box::pin(page_evaluate(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_function_behaves_as_playwright_on_browseros() -> TestResult {
    on_browseros(|rig| Box::pin(wait_for_function(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn locator_evaluate_and_dollar_eval_behave_as_playwright_on_browseros() -> TestResult {
    on_browseros(|rig| Box::pin(locator_evaluate(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn evaluate_all_and_dollar_dollar_eval_behave_as_playwright_on_browseros() -> TestResult {
    on_browseros(|rig| Box::pin(evaluate_all(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn set_content_behaves_as_playwright_on_browseros() -> TestResult {
    on_browseros(|rig| Box::pin(set_content(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn an_owner_lock_that_blocks_reads_withholds_what_a_running_script_read() -> TestResult {
    on_browseros(|rig| Box::pin(lock_while_a_script_runs(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_popups_script_runs_only_while_no_one_holds_its_opener() -> TestResult {
    on_browseros(|rig| Box::pin(popup_reaching_its_opener(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn an_owner_lock_on_an_opener_withholds_what_a_running_popup_script_read_there() -> TestResult
{
    on_browseros(|rig| Box::pin(lock_an_opener_while_its_popup_reads(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "waits for sdk-traces' operation_reach recording and settle (op-reach)"]
async fn an_owner_lock_on_an_opener_is_not_settled_while_its_popups_script_runs() -> TestResult {
    on_browseros(|rig| Box::pin(settle_an_opener_while_its_popup_runs(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_popup_a_click_opened_takes_a_script_at_once_from_either_end() -> TestResult {
    on_browseros(|rig| Box::pin(popup_scripts_at_once(rig))).await
}

/// `drive` on a fresh BrowserOS within the test's budget, with both processes' logs on failure.
async fn on_browseros(
    drive: impl for<'a> FnOnce(&'a Rig) -> Pin<Box<dyn Future<Output = TestResult> + 'a>>,
) -> TestResult {
    let Some(browser) = browseros_rig::browser_or_skip() else {
        return Ok(());
    };
    let rig = Rig::start(&browser, CaptchaKey::Absent)?;
    let failure = match time::timeout(TEST_BUDGET, drive(&rig)).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

/// One run of [`PAGE`] then `script` for `task`, on a fresh client: what the script returned.
async fn observed(rig: &Rig, task: &str, script: &str) -> Result<Value, Box<dyn Error>> {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let result = browseros_rig::run(
        &client,
        &format!("{PAGE}\n{script}"),
        &json!({ "taskId": task }),
    )
    .await?;
    client.cancel().await?;
    returned(&result)
}

/// What a run returned, once it ended `ok`.
fn returned(result: &CallToolResult) -> Result<Value, Box<dyn Error>> {
    let meta = browseros_rig::outcome(result)?;
    if meta.get("status") != Some(&json!("ok")) {
        return Err(format!("the run ends ok: {result:?}").into());
    }
    result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .cloned()
        .ok_or_else(|| format!("the script returns what it saw: {result:?}").into())
}

/// An SDK call's failure as [`PAGE`]'s `failed` reports it: Playwright's error class, Neo's
/// owner, code and delivery, and the message's first line.
fn threw(name: &str, owner: &str, code: &str, delivery: &str, message: &str) -> Value {
    json!({ "name": name, "owner": owner, "code": code, "delivery": delivery, "message": message })
}

/// `page.evaluate`, and `frame.evaluate`, which sends it: Playwright's value transport both ways,
/// an expression, element handles as arguments, upstream's error with its prefix, Neo's bound on
/// a result's depth, and a script in a same-site child frame, which runs there and reaches its
/// parent.
async fn page_evaluate(rig: &Rig) -> TestResult {
    let script = r"
        const got = await page.evaluate(() => {
          const cycle = { name: 'cycle' };
          cycle.self = cycle;
          return {
            u: undefined, nan: NaN, negZero: -0, inf: -Infinity, big: 10n, date: new Date(0),
            re: /a+/g, url: new URL('https://a.example/x?y=1#z'), typed: new Uint8Array([1, 2, 3]),
            err: new Error('inner'), cycle,
          };
        });
        const [field, item] = [await page.$('#user'), await page.$('.item')];
        const nested = (levels) => page.evaluate((levels) => {
          let value = 'bottom';
          for (let level = 0; level < levels; level++) value = { down: value };
          return value;
        }, levels);
        const depth = (value) => {
          let levels = 0;
          for (; typeof value === 'object'; levels++) value = value.down;
          return levels;
        };
        return {
          values: {
            undefined: 'u' in got && got.u === undefined,
            nan: Number.isNaN(got.nan),
            negZero: Object.is(got.negZero, -0),
            infinity: got.inf === -Infinity,
            bigint: got.big === 10n,
            date: got.date instanceof Date && got.date.getTime(),
            regexp: got.re instanceof RegExp && `${got.re.source}/${got.re.flags}`,
            url: [got.url.href, got.url.hostname, got.url instanceof Object],
            typed: got.typed instanceof Uint8Array && Array.from(got.typed),
            error: got.err instanceof Error && got.err.message,
            cycle: got.cycle.self === got.cycle,
          },
          argument: await page.evaluate(
            (a) => [a.d instanceof Date && a.d.getTime(), typeof a.b, a.u === undefined, a.n.x],
            { d: new Date(5), b: 7n, u: undefined, n: { x: [1] } },
          ),
          expression: await page.evaluate('1 + 2'),
          handles: await page.evaluate(([f, i]) => `${f.value}:${i.textContent}`, [field, item]),
          mainFrame: await page.mainFrame().evaluate(() => document.title),
          thrown: await failed(() => page.evaluate(() => { throw new Error('boom'); })),
          nested: [depth(await nested(40)), await failed(() => nested(80))],
          childFrame: await page.frames()[1].evaluate(() => [
            document.getElementById('inner').textContent, parent.document.title, window === top,
          ]),
        };";
    let value = observed(rig, "evaluate", script).await?;
    assert_eq!(
        value,
        json!({
            "values": {
                "undefined": true, "nan": true, "negZero": true, "infinity": true, "bigint": true,
                "date": 0, "regexp": "a+/g",
                // The realm has no `URL`, so a URL comes back URL-like: its fields, no class.
                "url": ["https://a.example/x?y=1#z", "a.example", true],
                "typed": [1, 2, 3], "error": "inner", "cycle": true,
            },
            "argument": [5, "bigint", true, [1]],
            "expression": 3,
            "handles": "ada:one",
            "mainFrame": "Main world",
            "thrown": threw("Error", "website", "page_error", "done", "page.evaluate: Error: boom"),
            // The realm's stack bounds how deep a result it reads, where Playwright reads 98
            // levels; the op answered, so its error carries no owner, code or delivery.
            "nested": [
                40,
                { "name": "Error", "message": "page.evaluate: Maximum call stack size exceeded" },
            ],
            "childFrame": ["in frame", "Main world", false],
        }),
        "page.evaluate as Playwright runs it"
    );
    Ok(())
}

/// `page.waitForFunction`, and `frame.waitForFunction`, which sends it: a wait on the page's own
/// change, an argument, a polling interval, upstream's timeout and errors, a wait that polls in a
/// child frame, and Neo's difference, a handle that holds the value.
async fn wait_for_function(rig: &Rig) -> TestResult {
    let script = r"
        const ready = await page.waitForFunction(() => window.ready);
        const counted = await page.waitForFunction(
          (n) => document.querySelectorAll('.item').length >= n && 'enough', 2, { polling: 50 },
        );
        const object = await page.waitForFunction(() => ({ a: 1 }));
        return {
          ready: [await ready.jsonValue(), String(ready)],
          argument: await counted.jsonValue(),
          snapshot: [await object.jsonValue(), await failed(() => object.evaluate((o) => o.a))],
          timeout: await failed(() => page.waitForFunction(() => false, null, { timeout: 300 })),
          defaultTimeout: await failed(() => page.waitForFunction(() => false)),
          polling: await failed(() => page.waitForFunction(() => true, null, { polling: 'mutation' })),
          thrown: await failed(() => page.waitForFunction(() => { throw new Error('nope'); })),
          childFrame: await page.frames()[1]
            .waitForFunction(() => document.getElementById('inner')?.textContent)
            .then((handle) => handle.jsonValue()),
        };";
    let value = observed(rig, "wait", script).await?;
    assert_eq!(
        value,
        json!({
            "ready": ["yes", "yes"],
            "argument": "enough",
            "snapshot": [
                { "a": 1 },
                // A method Neo lacks, refused through the door, which lets a prelude claim only
                // the script's two codes.
                threw(
                    "Error", "script", "method_unavailable", "not_sent",
                    "jsHandle.evaluate: Neo's waitForFunction handle holds the value, not the page's object; read it with jsonValue()",
                ),
            ],
            "timeout": threw(
                "TimeoutError", "website", "wait_timeout", "unknown",
                "page.waitForFunction: Timeout 300ms exceeded.",
            ),
            // Neo's default for a call that waits, which the run description states.
            "defaultTimeout": threw(
                "TimeoutError", "website", "wait_timeout", "unknown",
                "page.waitForFunction: Timeout 5000ms exceeded.",
            ),
            "polling": threw(
                "Error", "script", "invalid_input", "not_sent", "Unknown polling option: mutation"
            ),
            "thrown": threw(
                "Error", "website", "page_error", "done", "page.waitForFunction: Error: nope"
            ),
            "childFrame": "in frame",
        }),
        "page.waitForFunction as Playwright runs it, but for Neo's handle"
    );
    Ok(())
}

/// `locator.evaluate`, and what sends it: `elementHandle.evaluate` and `$eval` on page, frame and
/// handle, a child frame included, since a locator reaches into it. An argument, an element handle
/// as one, and upstream's errors with their prefixes.
async fn locator_evaluate(rig: &Rig) -> TestResult {
    let script = r"
        const [list, item, field] = [await page.$('#list'), await page.$('.item'), await page.$('#user')];
        return {
          locator: await page.locator('#user').evaluate((el, suffix) => el.value + suffix, '!'),
          handleArgument: await page.locator('#list').evaluate((el, i) => el.contains(i), item),
          elementHandle: await field.evaluate((el) => el.id),
          pageDollarEval: await page.$eval('#user', (el) => el.value),
          frameDollarEval: await page.mainFrame().$eval('#user', (el) => el.value),
          childFrameDollarEval: await page.frames()[1].$eval('#inner', (el) => el.textContent),
          handleDollarEval: await list.$eval('.item', (el) => el.textContent),
          missing: await failed(() => page.$eval('#missing', () => 1)),
          strict: await failed(() => page.locator('.item').evaluate(() => 1)),
          thrown: await failed(() => page.locator('#user').evaluate(() => { throw new Error('bad element'); })),
          timeout: await failed(() => page.locator('#missing').evaluate(() => 1, null, { timeout: 300 })),
        };";
    let value = observed(rig, "locator-evaluate", script).await?;
    assert_eq!(
        value,
        json!({
            "locator": "ada!",
            "handleArgument": true,
            "elementHandle": "user",
            "pageDollarEval": "ada",
            "frameDollarEval": "ada",
            "childFrameDollarEval": "in frame",
            "handleDollarEval": "one",
            "missing": threw(
                "Error", "script", "invalid_input", "not_sent",
                "page.$eval: Failed to find element matching selector \"#missing\"",
            ),
            "strict": threw(
                "Error", "script", "strict_mode_violation", "not_sent",
                "locator.evaluate: Error: strict mode violation: locator('.item') resolved to 2 elements:",
            ),
            "thrown": threw(
                "Error", "website", "page_error", "done", "locator.evaluate: Error: bad element"
            ),
            "timeout": threw(
                "TimeoutError", "script", "no_match", "not_sent",
                "locator.evaluate: Timeout 300ms exceeded.",
            ),
        }),
        "locator.evaluate and every API that sends it, as Playwright runs them"
    );
    Ok(())
}

/// `locator.evaluateAll`, and what sends it: `$$eval` on page, frame and handle, a child frame
/// included. An argument, an element handle as one, no match, and upstream's error.
async fn evaluate_all(rig: &Rig) -> TestResult {
    let script = r"
        const list = await page.$('#list');
        return {
          locator: await page.locator('.item').evaluateAll((els, sep) => els.map((el) => el.textContent).join(sep), '+'),
          none: await page.locator('.none').evaluateAll((els) => els.length),
          pageDollarDollarEval: await page.$$eval('.item', (els) => els.map((el) => el.textContent)),
          frameDollarDollarEval: await page.mainFrame().$$eval('.item', (els) => els.length),
          childFrame: await page.frames()[1].$$eval('p', (els) => els.map((el) => el.textContent)),
          handle: await list.$$eval('.item', (els) => els.length),
          handleArgument: await page.$$eval('.item', (els, l) => els.every((el) => l.contains(el)), list),
          thrown: await failed(() => page.$$eval('.item', () => { throw new Error('bad elements'); })),
        };";
    let value = observed(rig, "evaluate-all", script).await?;
    assert_eq!(
        value,
        json!({
            "locator": "one+two",
            "none": 0,
            "pageDollarDollarEval": ["one", "two"],
            "frameDollarDollarEval": 2,
            "childFrame": ["in frame"],
            "handle": 2,
            "handleArgument": true,
            "thrown": threw(
                "Error", "website", "page_error", "done", "page.$$eval: Error: bad elements"
            ),
        }),
        "locator.evaluateAll and every API that sends it, as Playwright runs them"
    );
    Ok(())
}

/// The password [`set_content`]'s HTML gives its field.
const WRITTEN_SECRET: &str = "setc-pw-7Rb3";

/// `page.setContent`, and `frame.setContent`, which sends it, on a child frame and on the main
/// frame: each writes its own frame's document, the HTML's inline script runs once, and Neo
/// refuses `networkidle`, which it cannot observe after a write. A password field the HTML writes
/// reads masked through every read the realm has, a child frame's script reaching its parent
/// included: the crate's seal and its evaluate guard mask it.
async fn set_content(rig: &Rig) -> TestResult {
    let script = format!(
        r#"
        const SECRET = {secret};
        const child = page.frames()[1];
        await child.setContent('<title>Child</title><p id="c">child written</p>');
        const childWrite = [
          await child.title(), await child.locator('#c').textContent(), await child.content(),
          await page.title(),
        ];
        const written = await page.setContent(`<title>Written</title>
          <input type="password" id="pw" value="${{SECRET}}"><p id="w">written</p>
          <iframe srcdoc="<p id=inner>inner</p>"></iframe>
          <script>window.runs = (window.runs || 0) + 1;</script>`);
        const seen = (value) => [value, value.includes(SECRET)];
        const content = await page.content();
        return {{
          childWrite,
          written: [written === undefined, await page.title(), await page.locator('#w').textContent()],
          runs: await page.evaluate(() => window.runs),
          content: seen(content.match(/<input[^>]*id="pw"[^>]*>/)?.[0] ?? content),
          inputValue: seen(await page.locator('#pw').inputValue()),
          evaluate: seen(await page.evaluate(() => document.getElementById('pw').value)),
          childFrame: seen(await page.frames()[1].evaluate(() => parent.document.getElementById('pw').value)),
          mainFrame: await page.mainFrame().setContent('<title>Again</title>').then(() => page.title()),
          networkidle: await failed(() => page.setContent('<p>x</p>', {{ waitUntil: 'networkidle' }})),
        }};"#,
        secret = json!(WRITTEN_SECRET)
    );
    rig.until_ready().await?;
    let client = rig.client().await?;
    let result = browseros_rig::run(
        &client,
        &format!("{PAGE}\n{script}"),
        &json!({ "taskId": "set-content" }),
    )
    .await?;
    client.cancel().await?;
    // The main world reads the field's value as `[private]`, and the sealed world shows it as
    // `••••`, as an aria snapshot does.
    let masked = json!(["[private]", false]);
    let sealed = json!(["••••", false]);
    assert_eq!(
        returned(&result)?,
        json!({
            "childWrite": [
                "Child", "child written",
                "<html><head><title>Child</title></head><body><p id=\"c\">child written</p></body></html>",
                "Main world",
            ],
            "written": [true, "Written", "written"],
            "runs": 1,
            "content": [r#"<input type="password" id="pw" value="••••">"#, false],
            "inputValue": sealed,
            "evaluate": masked,
            "childFrame": masked,
            "mainFrame": "Again",
            "networkidle": threw(
                "Error", "neo", "unsupported", "not_sent",
                "page.setContent: waitUntil \"networkidle\" cannot be observed after setContent in Neo; use \"load\"",
            ),
        }),
        "setContent as Playwright runs it, but for networkidle, with the written password masked"
    );
    set_content_recorded(&run_record(rig, &result).await?)
}

/// [`set_content`]'s run as the host reads it back: one row per write.
fn set_content_recorded(recorded: &Value) -> TestResult {
    let writes: Vec<&Value> = operations(recorded)?
        .iter()
        .filter(|row| row.get("name") == Some(&json!("page.setContent")))
        .collect();
    let tries: Vec<(Option<&str>, Option<&str>)> = writes
        .iter()
        .map(|row| {
            (
                row.pointer("/args/call/api").and_then(Value::as_str),
                row.get("delivery").and_then(Value::as_str),
            )
        })
        .collect();
    assert_eq!(
        tries,
        [
            (Some("frame.setContent"), Some("done")),
            (Some("page.setContent"), Some("done")),
            (Some("frame.setContent"), Some("done")),
            (Some("page.setContent"), Some("not_sent")),
        ],
        "each write is one row, as the crate op both APIs share: {recorded}"
    );
    Ok(())
}

/// G2 (decision 35c) on the real surface. A script reads the page 15 s after it starts; the host
/// takes the tab with reads blocked meanwhile, and the page changes under the owner. The script
/// ran, so its op is `done`, but what it read is withheld, from the realm and from the record.
/// The lock waits for the script's own mark in the page, [`READING`], since the op's record
/// starts before its pre-send sweep: a lock taken in that sweep refuses the op unsent instead.
async fn lock_while_a_script_runs(rig: &Rig) -> TestResult {
    // What the owner types into a field no seal masks, built from parts in the page, so that no
    // script or URL holds it whole.
    let typed = "4 Elm St";
    rig.until_ready().await?;
    let client = rig.client().await?;
    let opened = browseros_rig::run(
        &client,
        r"
        await page.goto('data:text/html,' + encodeURIComponent(`<title>Address</title>
          <input id='address'>
          <script>setTimeout(() => { document.getElementById('address').value = ['4 Elm', ' St'].join('') }, 3000)</script>`));
        return page;",
        &json!({ "taskId": "lock" }),
    )
    .await?;
    let tab = returned(&opened)?
        .get("tab")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("a page leaves a run as its brand, with its tab: {opened:?}"))?;
    let reading = r"
        try {
          const read = await page.evaluate(() => {
            document.title = 'the script is reading';
            return new Promise((resolve) => setTimeout(() => resolve(document.getElementById('address').value), 15000));
          });
          return { released: read };
        } catch (error) {
          return { withheld: [error.owner, error.code, error.delivery] };
        }";
    let meta = json!({ "taskId": "lock" });
    let (lock, read) = tokio::join!(
        lock_once_the_script_runs(rig, tab, "block"),
        browseros_rig::run(&client, reading, &meta),
    );
    let (lock, read) = (lock?, read?);
    client.cancel().await?;
    let recorded = run_record(rig, &read).await?;
    let script_tab = operations(&recorded)?
        .iter()
        .find(|row| row.get("name") == Some(&json!("page.evaluate")))
        .and_then(|row| row.get("tabId"))
        .and_then(Value::as_i64);
    assert_eq!(
        script_tab,
        Some(tab),
        "the script ran on the tab the host locked: {lock}\n{recorded}"
    );
    assert_eq!(
        returned(&read)?,
        json!({ "withheld": ["host", "owner_has_control", "done"] }),
        "the script ran, and what it read of the owner's tab is withheld: {recorded}"
    );
    assert_eq!(
        lock.get("settled"),
        Some(&json!(false)),
        "the lock found the script in flight, so it was taken while the script ran: {lock}\n\
         {recorded}"
    );
    assert!(
        !recorded.to_string().contains(typed),
        "the record keeps none of it either: {recorded}"
    );
    let lock_id = lock
        .get("id")
        .and_then(Value::as_str)
        .ok_or("a lock has an id")?;
    let released = rig
        .host(
            "DELETE",
            &format!("/v1/locks/{lock_id}"),
            Some(HOST_TOKEN),
            None,
        )
        .await?;
    assert_eq!(released.status, 200, "{}", released.text());
    Ok(())
}

/// The title the opener's script gives its popup, by which each later run finds the popup.
const POPUP_TITLE: &str = "Reach popup";

/// A popup's script reaches its opener through `window.opener`, so Neo runs its rules on the
/// opener too before it sends a script to the popup (`OpEnv::recheck_reach`), as a write there.
/// With no lock the script runs and returns Playwright's value. With the host holding the
/// opener, even with reads allowed, it is refused unsent, and it never ran in the popup.
async fn popup_reaching_its_opener(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let meta = json!({ "taskId": "reach" });
    let open = format!(
        r"
        await page.goto('data:text/html,' + encodeURIComponent('<title>Opener</title>'));
        await page.evaluate((title) => {{ window.open('', 'reach').document.title = title; }}, {POPUP_TITLE:?});
        return page;"
    );
    let opened = browseros_rig::run(&client, &open, &meta).await?;
    let opener = returned(&opened)?
        .get("tab")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("a page leaves a run as its brand, with its tab: {opened:?}"))?;
    // Each run finds the popup among every tab the browser has, and reports what its script did.
    let in_popup = |script: &str| {
        format!(
            r"
            const tabs = context.pages();
            const titles = await Promise.all(tabs.map((tab) => tab.title().catch(() => null)));
            const popup = tabs[titles.indexOf({POPUP_TITLE:?})];
            try {{
              return {{ value: await popup.evaluate({script}) }};
            }} catch (error) {{
              return {{ threw: [error.owner, error.code, error.delivery], message: error.message }};
            }}"
        )
    };
    let free = browseros_rig::run(
        &client,
        &in_popup("() => [window.opener.document.title, 6 * 7]"),
        &meta,
    )
    .await?;
    let free = returned(&free)?;
    assert_eq!(
        free.get("value"),
        Some(&json!(["Opener", 42])),
        "with no one holding the opener, the popup's script runs and reaches it: {free}"
    );

    let take = json!({ "tabId": opener, "reads": "allow", "ttlSeconds": 600 });
    let taken = rig
        .host("POST", "/v1/locks", Some(HOST_TOKEN), Some(&take))
        .await?;
    assert_eq!(
        taken.status,
        200,
        "the host locks the opener: {}",
        taken.text()
    );
    let held =
        browseros_rig::run(&client, &in_popup("() => { window.ran = true; }"), &meta).await?;
    let held = returned(&held)?;
    assert_eq!(
        held.get("threw"),
        Some(&json!(["host", "owner_has_control", "not_sent"])),
        "the popup's script could act on the opener the owner holds, so Neo sent none of it: \
         {held}"
    );

    let lock = taken.json()?;
    let lock_id = lock
        .get("id")
        .and_then(Value::as_str)
        .ok_or("a lock has an id")?;
    let released = rig
        .host(
            "DELETE",
            &format!("/v1/locks/{lock_id}"),
            Some(HOST_TOKEN),
            None,
        )
        .await?;
    assert_eq!(released.status, 200, "{}", released.text());
    let after = browseros_rig::run(&client, &in_popup("() => window.ran ?? false"), &meta).await?;
    client.cancel().await?;
    let after = returned(&after)?;
    assert_eq!(
        after.get("value"),
        Some(&json!(false)),
        "the refused script never ran in the popup: {after}"
    );
    Ok(())
}

/// What [`lock_an_opener_while_its_popup_reads`]'s opener shows. Its page builds it from parts, so
/// that no script or URL holds it whole: only a script that read the opener can carry it to the
/// realm or the trace store.
const OPENERS_TEXT: &str = "opener-text-3Wd8";

/// G2 across an opener link. A popup's script reads its opener through `window.opener` 15 s after
/// it starts; the host takes the opener with reads blocked meanwhile, once the script marks the
/// popup [`READING`]. No one holds the popup, the script's own tab, yet what it read of the opener
/// is withheld: the op is `done`, refused `owner_has_control`, and neither the realm, the run's
/// record nor the trace store's files get the opener's text.
async fn lock_an_opener_while_its_popup_reads(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let meta = json!({ "taskId": "reach-lock" });
    let open = format!(
        r"
        await page.goto('data:text/html,' + encodeURIComponent(`<title>Opener</title><p id='text'></p>
          <script>document.getElementById('text').textContent = ['opener', '-text', '-3Wd8'].join('')</script>`));
        await page.evaluate((title) => {{ window.open('', 'reader').document.title = title; }}, {POPUP_TITLE:?});
        return page;"
    );
    let opened = browseros_rig::run(&client, &open, &meta).await?;
    let opener = returned(&opened)?
        .get("tab")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("a page leaves a run as its brand, with its tab: {opened:?}"))?;
    let reading = format!(
        r"
        const tabs = context.pages();
        const titles = await Promise.all(tabs.map((tab) => tab.title().catch(() => null)));
        const popup = tabs[titles.indexOf({POPUP_TITLE:?})];
        try {{
          const read = await popup.evaluate((reading) => {{
            document.title = reading;
            return new Promise((resolve) => setTimeout(() => resolve(window.opener.document.body.innerText), 15000));
          }}, {READING:?});
          return {{ released: read }};
        }} catch (error) {{
          return {{ withheld: [error.owner, error.code, error.delivery], message: error.message }};
        }}"
    );
    let (lock, read) = tokio::join!(
        lock_once_the_script_runs(rig, opener, "block"),
        browseros_rig::run(&client, &reading, &meta),
    );
    let read = read?;
    let lock = lock.map_err(|error| format!("{error}; the run returned {read:?}"))?;
    client.cancel().await?;
    let recorded = run_record(rig, &read).await?;
    let script_tab = operations(&recorded)?
        .iter()
        .find(|row| row.get("name") == Some(&json!("page.evaluate")))
        .and_then(|row| row.get("tabId"))
        .and_then(Value::as_i64);
    assert!(
        script_tab.is_some_and(|tab| tab != opener),
        "the script ran in the popup, a tab the host never took: {lock}\n{recorded}"
    );
    let read = returned(&read)?;
    assert_eq!(
        read.get("withheld"),
        Some(&json!(["host", "owner_has_control", "done"])),
        "the script ran, and what it read of the opener the host took is withheld: {read}\n\
         {recorded}"
    );
    assert!(
        !read.to_string().contains(OPENERS_TEXT),
        "the realm gets none of the opener's text: {read}"
    );
    assert!(
        !recorded.to_string().contains(OPENERS_TEXT),
        "the run's record keeps none of it: {recorded}"
    );
    assert!(
        !holds(&trace_files(rig)?, OPENERS_TEXT),
        "trace.db and its WAL keep no copy of it"
    );
    let lock_id = lock
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the lock answered with its id: {lock}"))?;
    let released = rig
        .host(
            "DELETE",
            &format!("/v1/locks/{lock_id}"),
            Some(HOST_TOKEN),
            None,
        )
        .await?;
    assert_eq!(released.status, 200, "{}", released.text());
    Ok(())
}

/// The settle across an opener link (sdk-traces' `OperationReached`). A takeover lock's `settled`
/// says whether the agent's writes on the tab ended before the owner got it, and a popup's script
/// can act on its opener through `window.opener`. So a lock the host takes on the opener while the
/// popup's script runs finds a write in flight there and answers `settled: false`, though the
/// script's own tab is the popup.
///
/// The script marks the popup [`READING`] once it runs, then waits on a fetch that the test's
/// server answers only after the lock answered, so the op stays open past the lock's settle budget
/// however loaded the machine is. Both pages are the server's, so the popup shares its opener's
/// origin. The lock lets reads through, so the release rules pass and the script's answer, read
/// from the opener, goes out. A lock taken once the op ended answers `settled: true`.
async fn settle_an_opener_while_its_popup_runs(rig: &Rig) -> TestResult {
    let (release, released) = watch::channel(false);
    let origin = serve_opener(released).await?;
    rig.until_ready().await?;
    let client = rig.client().await?;
    let meta = json!({ "taskId": "reach-settle" });
    let open = format!(
        r"
        await page.goto({opener:?});
        await page.evaluate((title) => {{ window.open('', 'settle').document.title = title; }}, {POPUP_TITLE:?});
        return page;",
        opener = format!("{origin}/opener"),
    );
    let opened = browseros_rig::run(&client, &open, &meta).await?;
    let opener = returned(&opened)?
        .get("tab")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("a page leaves a run as its brand, with its tab: {opened:?}"))?;
    let running = format!(
        r"
        const tabs = context.pages();
        const titles = await Promise.all(tabs.map((tab) => tab.title().catch(() => null)));
        const popup = tabs[titles.indexOf({POPUP_TITLE:?})];
        try {{
          const read = await popup.evaluate(async ({{ reading, release }}) => {{
            document.title = reading;
            await fetch(release);
            return window.opener.document.title;
          }}, {{ reading: {READING:?}, release: {release_url:?} }});
          return {{ released: read }};
        }} catch (error) {{
          return {{ threw: [error.owner, error.code, error.delivery], message: error.message }};
        }}",
        release_url = format!("{origin}/release"),
    );
    let (lock, ran) = tokio::join!(
        async {
            let lock = lock_once_the_script_runs(rig, opener, "allow").await;
            // Whatever the lock answered, so that the run ends either way.
            release.send_replace(true);
            lock
        },
        browseros_rig::run(&client, &running, &meta),
    );
    let ran = ran?;
    let lock = lock.map_err(|error| format!("{error}; the run returned {ran:?}"))?;
    let recorded = run_record(rig, &ran).await?;
    let ran = returned(&ran)?;
    assert_eq!(
        lock.get("settled"),
        Some(&json!(false)),
        "the popup's script could act on the opener when the host took it: {lock}\n{ran}\n\
         {recorded}"
    );
    assert_eq!(
        ran.get("released"),
        Some(&json!("Opener")),
        "the lock lets reads through, so what the script read of the opener goes out: {ran}"
    );
    release_lock(rig, &lock).await?;

    let after = take_lock(rig, opener, "allow").await?;
    client.cancel().await?;
    assert_eq!(
        after.get("settled"),
        Some(&json!(true)),
        "the popup's script ended before this lock: {after}\n{recorded}"
    );
    release_lock(rig, &after).await
}

/// A web server on loopback for [`settle_an_opener_while_its_popup_runs`]: `/release` answers only
/// once `released` turns true, and every other path is a page titled `Opener`. Returns its origin;
/// it serves until the test's runtime ends.
async fn serve_opener(released: watch::Receiver<bool>) -> Result<String, Box<dyn Error>> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let origin = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut released = released.clone();
            tokio::spawn(async move {
                let mut request = [0_u8; 4096];
                let read = stream.read(&mut request).await.unwrap_or(0);
                let head = String::from_utf8_lossy(request.get(..read).unwrap_or_default());
                let body = if head.split(' ').nth(1) == Some("/release") {
                    let _ = released.wait_for(|&released| released).await;
                    "released"
                } else {
                    "<title>Opener</title>"
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    Ok(origin)
}

/// The page [`popup_scripts_at_once`] loads: each click on its button opens a popup linked to
/// it, numbered in its title, and keeps the window `window.open` returned.
const POPUP_OPENER: &str = r"
await page.goto('data:text/html,' + encodeURIComponent(`<title>Opener</title>
  <button id='open'>Open</button>
  <script>
    let opened = 0;
    document.getElementById('open').onclick = () => {
      opened += 1;
      const popup = window.open('', 'popup-' + opened);
      popup.document.write('<title>Popup ' + opened + '</title>');
      popup.document.close();
      window.popup = popup;
    };
  </script>`));
";

/// The flow an agent writes for a popup: a click opens it, `waitForEvent('popup')` hands it over,
/// and a script runs in it at once. Neo checks its rules on every tab a main-world script can
/// reach before it sends one, and refuses a tab its browser link has not listed yet
/// (`OpCtx::recheck_reach`), so a popup the link lists late would be refused here. Each script
/// runs in a popup of its own, at once: in the popup after the event, in the opener after the
/// event, and in the opener right after the click, with no wait for the event. The popup's
/// script reaches only its opener, listed long before; the opener's reaches the new popup.
/// Measured on BrowserOS at a load of 50 to 150: the link listed each popup within 45 ms of the
/// browser announcing its target, and at least 24 ms before the door checked it.
async fn popup_scripts_at_once(rig: &Rig) -> TestResult {
    let script = format!(
        r"{POPUP_OPENER}
        const attempt = async (call) => {{
          try {{
            return await call();
          }} catch (error) {{
            return {{ threw: [error.owner, error.code, error.delivery], message: error.message }};
          }}
        }};
        let opened = page.waitForEvent('popup');
        await page.locator('#open').click();
        let popup = await opened;
        const inPopup = await attempt(() => popup.evaluate(() => window.opener.document.title));
        await popup.close();

        opened = page.waitForEvent('popup');
        await page.locator('#open').click();
        popup = await opened;
        const inOpenerAfterEvent = await attempt(() => page.evaluate(() => window.popup.document.title));
        await popup.close();

        opened = page.waitForEvent('popup');
        await page.locator('#open').click();
        const inOpenerAfterClick = await attempt(() => page.evaluate(() => window.popup.document.title));
        await (await opened).close();
        return {{ inPopup, inOpenerAfterEvent, inOpenerAfterClick }};"
    );
    rig.until_ready().await?;
    let client = rig.client().await?;
    let result =
        browseros_rig::run(&client, &script, &json!({ "taskId": "popup-at-once" })).await?;
    client.cancel().await?;
    assert_eq!(
        returned(&result)?,
        json!({
            "inPopup": "Opener",
            "inOpenerAfterEvent": "Popup 2",
            "inOpenerAfterClick": "Popup 3",
        }),
        "each script crosses the opener link the moment the popup is there"
    );
    Ok(())
}

/// The bytes of the trace store's database and its WAL, as they are on disk now. SQLite drops the
/// WAL when its last connection closes, so a missing one holds nothing.
fn trace_files(rig: &Rig) -> Result<Vec<u8>, Box<dyn Error>> {
    let db = rig.trace_path();
    let mut stored = fs::read(&db)?;
    match fs::read(db.with_extension("db-wal")) {
        Ok(wal) => stored.extend(wal),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(stored)
}

/// Whether `stored` holds `value`'s bytes anywhere.
fn holds(stored: &[u8], value: &str) -> bool {
    stored
        .windows(value.len())
        .any(|bytes| bytes == value.as_bytes())
}

/// The title the scripts of [`lock_while_a_script_runs`], [`lock_an_opener_while_its_popup_reads`]
/// and [`settle_an_opener_while_its_popup_runs`] give their page from inside the page, once Neo
/// sent them and they run.
const READING: &str = "the script is reading";

/// Takes `tab` with `reads` (`block` or `allow`), as a host does, once a script runs in its page:
/// the browser lists a tab titled [`READING`]. Returns the lock.
async fn lock_once_the_script_runs(
    rig: &Rig,
    tab: i64,
    reads: &str,
) -> Result<Value, Box<dyn Error>> {
    time::timeout(PATIENCE, async {
        while !browseros_rig::get(rig.cdp_port(), "/json/list")
            .await?
            .contains(READING)
        {
            time::sleep(Duration::from_millis(100)).await;
        }
        Ok::<(), Box<dyn Error>>(())
    })
    .await
    .map_err(|_elapsed| format!("the script was not seen running within {PATIENCE:?}"))??;
    take_lock(rig, tab, reads).await
}

/// Takes `tab` with `reads`, as a host does: `POST /v1/locks`, which answers once the lock settled.
async fn take_lock(rig: &Rig, tab: i64, reads: &str) -> Result<Value, Box<dyn Error>> {
    let take = json!({ "tabId": tab, "reads": reads, "ttlSeconds": 600 });
    let taken = rig
        .host("POST", "/v1/locks", Some(HOST_TOKEN), Some(&take))
        .await?;
    if taken.status != 200 {
        return Err(format!("the host locks the tab: {}", taken.text()).into());
    }
    taken.json()
}

/// Gives `lock` back, as a host does.
async fn release_lock(rig: &Rig, lock: &Value) -> TestResult {
    let id = lock
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the lock answered with its id: {lock}"))?;
    let released = rig
        .host("DELETE", &format!("/v1/locks/{id}"), Some(HOST_TOKEN), None)
        .await?;
    assert_eq!(released.status, 200, "{}", released.text());
    Ok(())
}

/// The run of `result` as the host reads it back from the store, once it is complete.
async fn run_record(rig: &Rig, result: &CallToolResult) -> Result<Value, Box<dyn Error>> {
    let id = browseros_rig::outcome(result)?
        .get("id")
        .and_then(Value::as_str)
        .ok_or("the run has an id")?;
    let read = rig
        .host(
            "GET",
            &format!("/v1/runs/{id}?wait=30"),
            Some(HOST_TOKEN),
            None,
        )
        .await?;
    if read.status != 200 {
        return Err(format!("the host reads the run back: {}", read.text()).into());
    }
    read.json()
}

/// A run read's operations.
fn operations(recorded: &Value) -> Result<&Vec<Value>, Box<dyn Error>> {
    recorded
        .get("operations")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("a run read lists its ops: {recorded}").into())
}
