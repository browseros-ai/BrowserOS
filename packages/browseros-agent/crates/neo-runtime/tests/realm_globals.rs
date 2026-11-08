//! The standard globals a run's realm defines (`realm.rs` `HOST_GLOBALS` and neo-sdk's
//! `globals.ts`), through `Runtime::run` against the fake browser in `support/fake_browser.rs`:
//! what is defined and how, `URL` resolving and writing through Rust's parser, `crypto`'s bytes
//! from Rust, and `setInterval` within the run's deadline and cancellation. No browser needed;
//! `neo-sdk/tests/parity/globals.test.mjs` compares the values with Node's.

#[path = "support/fake_browser.rs"]
#[expect(dead_code, reason = "the fake page's readers serve the click tests")]
mod fake_browser;

use std::time::{Duration, Instant};

use fake_browser::{FakeBrowser, Replies};
use neo_runtime::{RunOutcome, RunStatus};
use serde_json::{Value, json};
use tokio::time;
use tokio_util::sync::CancellationToken;

async fn run(code: &str, timeout: Duration) -> RunOutcome {
    let browser = FakeBrowser::start(Replies::default()).await;
    browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            code,
            timeout,
            CancellationToken::new(),
        ))
        .await
}

async fn value(code: &str) -> Value {
    let outcome = run(code, Duration::from_secs(10)).await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    outcome.value.unwrap_or(Value::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_standard_globals_are_defined_as_node_defines_them() {
    let code = "const names = ['URL', 'URLSearchParams', 'Buffer', 'TextEncoder', 'TextDecoder',
        'structuredClone', 'crypto', 'setInterval', 'clearInterval', 'Intl', 'fetch']
      return {
        types: names.map((name) => typeof globalThis[name]),
        hidden: names.slice(0, 7).every((name) =>
          !Object.getOwnPropertyDescriptor(globalThis, name).enumerable),
        buffer: Buffer.from('hi').toString('base64'),
      }";
    assert_eq!(
        value(code).await,
        json!({
            "types": ["function", "function", "function", "function", "function", "function",
                "object", "function", "function", "undefined", "undefined"],
            "hidden": true,
            "buffer": "aGk=",
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn url_resolves_and_writes_through_rusts_parser() {
    let code = "const url = new URL('../q?a=1#h', 'https://h.test/p/r/s')
      url.searchParams.append('b', '2 3')
      const appended = url.href
      url.pathname = '/x y'
      url.port = '8443'
      let invalid
      try { new URL('/relative') } catch (error) { invalid = error.name }
      return [appended, url.href, url.origin, url instanceof URL, invalid]";
    assert_eq!(
        value(code).await,
        json!([
            "https://h.test/p/q?a=1&b=2+3#h",
            "https://h.test:8443/x%20y?a=1&b=2+3#h",
            "https://h.test:8443",
            true,
            "TypeError",
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn crypto_draws_fresh_bytes_from_rust() {
    let code = "const v4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/
      const ids = [crypto.randomUUID(), crypto.randomUUID()]
      const words = crypto.getRandomValues(new Uint32Array(8))
      let quota
      try { crypto.getRandomValues(new Uint8Array(65537)) } catch (error) { quota = error.name }
      return [ids.every((id) => v4.test(id)), ids[0] !== ids[1], words.some((w) => w !== 0), quota]";
    assert_eq!(
        value(code).await,
        json!([true, true, true, "QuotaExceededError"])
    );
}

/// The interval's own callback reports its third tick, so the count never depends on how fast
/// the machine is; the wait after `clearInterval` then spans many more periods, which a live
/// interval would have used.
#[tokio::test(flavor = "multi_thread")]
async fn an_interval_repeats_with_its_arguments_until_cleared() {
    let code = "const seen = []
      let id
      await new Promise((resolve) => {
        id = setInterval((step) => {
          seen.push(step)
          if (seen.length === 3) { clearInterval(id); resolve() }
        }, 5, 'tick')
      })
      await new Promise((resolve) => setTimeout(resolve, 100))
      let zero = 0
      const fast = setInterval(() => { zero++ }, 0)
      await new Promise((resolve) => setTimeout(resolve, 30))
      clearTimeout(fast)
      const stopped = zero
      await new Promise((resolve) => setTimeout(resolve, 30))
      return { seen, clearedByClearTimeout: zero === stopped, ticked: stopped > 0 }";
    assert_eq!(
        value(code).await,
        json!({
            "seen": ["tick", "tick", "tick"],
            "clearedByClearTimeout": true,
            "ticked": true,
        })
    );
}

/// Node exits on a timer callback's throw (an uncaught exception); the run ends with that error
/// at once, a parked script included, and the throwing interval stops.
#[tokio::test(flavor = "multi_thread")]
async fn a_timer_callback_that_throws_ends_the_run_with_its_error() {
    for code in [
        "setTimeout(() => { throw new RangeError('from a timeout') }, 1); await new Promise(() => {})",
        "let n = 0
         setInterval(() => { n++; throw new RangeError('from an interval') }, 1)
         await new Promise(() => {})",
    ] {
        let started = Instant::now();
        let outcome = run(code, Duration::from_secs(30)).await;
        assert_eq!(outcome.status, RunStatus::Error, "{outcome:?}");
        let error = outcome.error.expect("the callback's error");
        assert_eq!(error.name, "RangeError", "{error:?}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the throw ends the run, not its deadline: {:?}",
            started.elapsed()
        );
    }
    let refused = value(
        "const kinds = []
         for (const start of [() => setTimeout('x', 1), () => setInterval(42, 1)])
           try { start() } catch (error) { kinds.push(error.name) }
         return kinds",
    )
    .await;
    assert_eq!(refused, json!(["TypeError", "TypeError"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_live_interval_never_holds_a_run_past_its_end() {
    let started = Instant::now();
    let returned = run(
        "setInterval(() => {}, 1); return 'done'",
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(returned.value, Some(json!("done")), "{returned:?}");

    let parked = run(
        "setInterval(() => {}, 1); await new Promise(() => {})",
        Duration::from_millis(300),
    )
    .await;
    assert_eq!(parked.status, RunStatus::Timeout, "{parked:?}");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "both runs end at once with their interval still armed, took {:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_ends_a_run_with_a_live_interval() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let cancel = CancellationToken::new();
    let request = fake_browser::run_request(
        "let n = 0; setInterval(() => { n++ }, 1); await new Promise(() => {})",
        Duration::from_secs(30),
        cancel.clone(),
    );
    let canceller = tokio::spawn(async move {
        time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();
    });
    let outcome = runtime.run(request).await;
    canceller.await.unwrap();

    assert_eq!(outcome.status, RunStatus::Cancelled, "{outcome:?}");
}
