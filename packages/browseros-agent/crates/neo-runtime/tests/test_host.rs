//! Neo: The optional test transport through the public runtime API and a real QuickJS realm.
//! The existing fake browser owns CDP replies; bounded channels carry opaque test frames and
//! expose whether a parked receive ended when its owning run did.

#![cfg(feature = "test-host")]

#[path = "support/fake_browser.rs"]
mod fake_browser;

use std::time::Duration;

use fake_browser::{FakeBrowser, PAGE_URL, Replies};
use futures_util::future::BoxFuture;
use neo_runtime::{CallResult, Delivery, RunEnd, RunStatus, TestHost, TestHostError};
use serde_json::json;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::time;
use tokio_util::sync::CancellationToken;

/// Neo: bounds fake-browser test teardown without changing runtime deadlines.
const SETTLE: Duration = Duration::from_secs(5);

/// One run's bounded transport. Reply never waits for the host test to consume a frame.
struct ChannelHost {
    incoming: Mutex<Receiver<String>>,
    outgoing: Sender<String>,
    receiving: CancellationToken,
}

/// The independent host side can deliver commands while QuickJS is parked in receive.
struct HostControl {
    commands: Sender<String>,
    replies: Receiver<String>,
    receiving: CancellationToken,
}

fn host() -> (Box<dyn TestHost>, HostControl) {
    let (commands, incoming) = mpsc::channel(8);
    let (outgoing, replies) = mpsc::channel(8);
    let receiving = CancellationToken::new();
    (
        Box::new(ChannelHost {
            incoming: Mutex::new(incoming),
            outgoing,
            receiving: receiving.clone(),
        }),
        HostControl {
            commands,
            replies,
            receiving,
        },
    )
}

impl TestHost for ChannelHost {
    fn receive(&self) -> BoxFuture<'_, Result<Option<String>, TestHostError>> {
        Box::pin(async move {
            self.receiving.cancel();
            Ok(self.incoming.lock().await.recv().await)
        })
    }

    fn reply(&self, frame: String) -> Result<(), TestHostError> {
        self.outgoing
            .try_send(frame)
            .map_err(|error| TestHostError::new(error.to_string()))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ordinary_runs_have_no_test_host_even_after_a_hosted_run() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let code = "return ['__neo_test_host' in globalThis, typeof __neo_test_host, page.url()]";
    for hosted_first in [false, true] {
        if hosted_first {
            let (host, _control) = host();
            let outcome = runtime
                .run_with_test_host(
                    fake_browser::run_request(
                        "return typeof __neo_test_host.receive",
                        SETTLE,
                        CancellationToken::new(),
                    ),
                    host,
                )
                .await;
            assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
            assert_eq!(outcome.value, Some(json!("function")), "{outcome:?}");
        }
        let outcome = runtime
            .clone()
            .run(fake_browser::run_request(
                code,
                SETTLE,
                CancellationToken::new(),
            ))
            .await;
        assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
        assert_eq!(
            outcome.value,
            Some(json!([false, "undefined", PAGE_URL])),
            "{outcome:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn frames_are_opaque_reply_is_synchronous_and_eof_is_null() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let (host, control) = host();
    let HostControl {
        commands,
        mut replies,
        ..
    } = control;
    let frame = "an opaque frame\nwith \"quotes\", \u{0} and \u{1f980}";
    commands
        .try_send(frame.to_string())
        .expect("one frame fits");
    drop(commands);

    let outcome = browser
        .runtime()
        .await
        .run_with_test_host(
            fake_browser::run_request(
                "const frame = await __neo_test_host.receive();
                 const result = __neo_test_host.reply(frame);
                 return [frame, await __neo_test_host.receive(), result === undefined, page.url()]",
                SETTLE,
                CancellationToken::new(),
            ),
            host,
        )
        .await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        outcome.value,
        Some(json!([frame, null, true, PAGE_URL])),
        "{outcome:?}"
    );
    assert_eq!(replies.try_recv().as_deref(), Ok(frame));
    assert!(
        outcome.calls.is_empty(),
        "transport frames do not become browser calls: {outcome:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_ends_a_parked_receive_without_an_external_frame() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let (host, control) = host();
    let cancel = CancellationToken::new();
    let request = fake_browser::run_request(
        "await __neo_test_host.receive(); return 'received'",
        Duration::from_secs(30),
        cancel.clone(),
    );
    let outcome = time::timeout(SETTLE, async {
        let cancel_when_parked = async {
            control.receiving.cancelled().await;
            cancel.cancel();
        };
        let (outcome, ()) = tokio::join!(
            runtime.run_with_test_host(request, host),
            cancel_when_parked
        );
        outcome
    })
    .await
    .expect("cancellation must end the parked receive");

    assert_eq!(outcome.status, RunStatus::Cancelled, "{outcome:?}");
    assert!(
        control.commands.is_closed(),
        "the run releases its host and pending receive before returning"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_native_deadline_ends_a_parked_receive_without_an_external_frame() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let (host, control) = host();
    let outcome = time::timeout(
        SETTLE,
        browser.runtime().await.run_with_test_host(
            fake_browser::run_request(
                "await __neo_test_host.receive(); return 'received'",
                Duration::from_millis(500),
                CancellationToken::new(),
            ),
            host,
        ),
    )
    .await
    .expect("the native deadline must end the parked receive");

    assert!(
        control.receiving.is_cancelled(),
        "the script reached the host before its deadline: {outcome:?}"
    );
    assert_eq!(outcome.status, RunStatus::Timeout, "{outcome:?}");
    assert!(
        control.commands.is_closed(),
        "the timed-out run releases its host and pending receive"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_runs_on_runtime_clones_keep_their_hosts_separate() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let other_runtime = runtime.clone();
    let (first_host, mut first) = host();
    let (second_host, mut second) = host();
    let code = "const frame = await __neo_test_host.receive();
                __neo_test_host.reply(frame); return frame";
    let request = || fake_browser::run_request(code, SETTLE, CancellationToken::new());
    let (first_outcome, second_outcome) = time::timeout(SETTLE, async {
        let deliver = async {
            tokio::join!(first.receiving.cancelled(), second.receiving.cancelled());
            first
                .commands
                .try_send("first run".to_string())
                .expect("the first host has capacity");
            second
                .commands
                .try_send("second run".to_string())
                .expect("the second host has capacity");
        };
        let (first, second, ()) = tokio::join!(
            runtime.run_with_test_host(request(), first_host),
            other_runtime.run_with_test_host(request(), second_host),
            deliver,
        );
        (first, second)
    })
    .await
    .expect("both hosts receive independently while both runs are parked");

    for (outcome, expected) in [
        (&first_outcome, "first run"),
        (&second_outcome, "second run"),
    ] {
        assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
        assert_eq!(outcome.value, Some(json!(expected)), "{outcome:?}");
    }
    assert_eq!(first.replies.try_recv().as_deref(), Ok("first run"));
    assert_eq!(second.replies.try_recv().as_deref(), Ok("second run"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finish_frame_uses_normal_pending_input_cancellation_and_delivery_records() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let (host, control) = host();
    let request = fake_browser::run_request(
        "page.click('#go', { delay: 60_000 });
         const frame = await __neo_test_host.receive(); return frame",
        Duration::from_secs(15),
        CancellationToken::new(),
    );
    let outcome = time::timeout(SETTLE, async {
        let finish_after_press = async {
            control.receiving.cancelled().await;
            loop {
                let pressed = browser.exchanges().iter().any(|exchange| {
                    exchange.method() == "Input.dispatchMouseEvent"
                        && exchange.command["params"]["type"] == "mousePressed"
                        && exchange.reply.is_some()
                });
                if pressed {
                    break;
                }
                time::sleep(Duration::from_millis(10)).await;
            }
            control
                .commands
                .try_send("finish".to_string())
                .expect("the pending receive accepts finish");
        };
        let (outcome, ()) = tokio::join!(
            runtime.run_with_test_host(request, host),
            finish_after_press,
        );
        outcome
    })
    .await
    .expect("finish cuts short the unawaited input delay");

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.value, Some(json!("finish")), "{outcome:?}");
    let [call] = outcome.calls.as_slice() else {
        panic!("one pending native click leaves one record: {outcome:?}");
    };
    assert_eq!(call.method, "click", "{call:?}");
    assert_eq!(
        call.result,
        CallResult::Unfinished {
            reason: RunEnd::Settled
        },
        "{call:?}"
    );
    assert_eq!(call.delivery, Some(Delivery::Done), "{call:?}");
    assert_eq!(browser.armed_interceptor(), None);
    assert_eq!(browser.page_clicks(), 1);
    assert!(
        control.commands.is_closed(),
        "a normal finish also drops the per-run test host"
    );
}
