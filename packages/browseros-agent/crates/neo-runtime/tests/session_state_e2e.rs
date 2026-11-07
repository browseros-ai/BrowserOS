//! Session state on a real BrowserOS neo (`src/session_state.rs`): what a run sets on its
//! sessions (extra HTTP headers, offline mode, geolocation) ends with it, so the next run on the
//! same tab finds none of it; a cookie it adds is browser data and stays; a permission it grants
//! is reset once it ends, back to what the profile had before.

#[expect(
    dead_code,
    reason = "the shared fixture also provides navigation helpers"
)]
#[path = "support/browser.rs"]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{PageId, RunOutcome, RunRequest, RunStatus, Runtime};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const PAGE: &str =
    "<!doctype html><html><head><title>state</title></head><body>state</body></html>";

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                pages: vec![("/state.html".into(), PAGE.into())],
                ..Fixture::default()
            },
            "/state.html",
        )
        .await,
    )
}

fn request(tab: &Tab, code: &str) -> RunRequest {
    RunRequest {
        code: code.to_string(),
        page: PageId::new(tab.target_id.clone()),
        timeout: Duration::from_secs(30),
        cancel: CancellationToken::new(),
    }
}

fn value(outcome: &RunOutcome) -> &Value {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    outcome.value.as_ref().unwrap_or(&Value::Null)
}

/// The page's geolocation permission state, read over the test's own session.
async fn geolocation_state(tab: &Tab) -> Value {
    let reply = tab
        .cdp
        .send(
            "Runtime.evaluate",
            json!({
                "expression": "navigator.permissions.query({ name: 'geolocation' }).then((status) => status.state)",
                "awaitPromise": true,
                "returnByValue": true,
            }),
            Some(&tab.driver),
        )
        .await
        .unwrap_or_else(|error| panic!("query the permission: {error}"));
    reply["result"]["value"].clone()
}

/// A script's read of where the page is, `"error"` when it learns nothing in a second.
const POSITION: &str = r"page.evaluate(() => new Promise((resolve) => navigator.geolocation.getCurrentPosition(
  (position) => resolve([position.coords.latitude, position.coords.longitude]),
  () => resolve('error'),
  { timeout: 1000 },
)))";

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_session_state_ends_with_the_run_and_cookies_stay() {
    let Some(tab) = open().await else {
        return;
    };
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("Runtime::connect: {error}"));
    let first = runtime
        .run(request(
            &tab,
            &format!(
                r"await context.setExtraHTTPHeaders({{ 'x-neo-run': 'one' }})
const echoed = await page.evaluate(() => fetch('/echo-headers').then((response) => response.text()))
await context.addCookies([{{ name: 'neo_run', value: 'one', url: page.url() }}])
await context.grantPermissions(['geolocation'], {{ origin: page.url() }})
await context.setGeolocation({{ latitude: 10, longitude: 20 }})
const position = await {POSITION}
await context.setOffline(true)
const network = await page.evaluate(() => fetch('/again').then(() => 'online', () => 'offline'))
return [echoed.toLowerCase().includes('x-neo-run: one'), position, network]"
            ),
        ))
        .await;
    assert_eq!(
        value(&first),
        &json!([true, [10, 20], "offline"]),
        "the run's headers, geolocation and offline mode reach its tab"
    );

    let second = runtime
        .run(request(
            &tab,
            &format!(
                r"const echoed = await page.evaluate(() => fetch('/echo-headers').then((response) => response.text()))
const network = await page.evaluate(() => fetch('/again').then(() => 'online', () => 'offline'))
await context.grantPermissions(['geolocation'], {{ origin: page.url() }})
const position = await {POSITION}
const cookies = (await context.cookies(page.url())).filter((cookie) => cookie.name === 'neo_run')
await context.clearCookies({{ name: 'neo_run' }})
const cleared = (await context.cookies(page.url())).filter((cookie) => cookie.name === 'neo_run').length
return [echoed.toLowerCase().includes('x-neo-run'), network, position, cookies.map((cookie) => cookie.value), cleared]"
            ),
        ))
        .await;
    let second = value(&second);
    assert_eq!(
        second.get(0),
        Some(&json!(false)),
        "no header of the first run"
    );
    assert_eq!(second.get(1), Some(&json!("online")), "not offline");
    assert_ne!(
        second.get(2),
        Some(&json!([10, 20])),
        "not the first run's position"
    );
    assert_eq!(
        second.get(3),
        Some(&json!(["one"])),
        "the first run's cookie stayed"
    );
    assert_eq!(
        second.get(4),
        Some(&json!(0)),
        "and a filtered clear removed it"
    );
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_runs_permission_grants_are_reset_when_it_ends() {
    let Some(tab) = open().await else {
        return;
    };
    let before = geolocation_state(&tab).await;
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("Runtime::connect: {error}"));
    let outcome = runtime
        .run(request(
            &tab,
            r"await context.grantPermissions(['geolocation'], { origin: page.url() })
return await page.evaluate(() => navigator.permissions.query({ name: 'geolocation' }).then((status) => status.state))",
        ))
        .await;
    assert_eq!(
        value(&outcome),
        &json!("granted"),
        "granted while the run holds it"
    );
    assert_ne!(
        before,
        json!("granted"),
        "a fresh profile has not granted it"
    );
    assert_eq!(
        geolocation_state(&tab).await,
        before,
        "the profile's own state once the run ended"
    );
    tab.close().await;
}
