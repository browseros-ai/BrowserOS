//! Neo: Runtime admission, host cancellation and delivery through the real QuickJS bridge.
//! A gated host makes ownership observable without paid providers or timing-based ordering.

#[expect(
    dead_code,
    reason = "The shared browser fixture also supports other integration suites"
)]
#[path = "support/fake_browser.rs"]
mod fake_browser;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use fake_browser::{FakeBrowser, FrameReplies, Replies};
use futures_util::future::BoxFuture;
use neo_runtime::{CaptchaHost, Delivery, PageId, RunStatus, SolveReply};
use serde_json::json;
use tokio::sync::{Notify, Semaphore};
use tokio::time;
use tokio_util::sync::CancellationToken;

#[expect(
    clippy::expect_used,
    reason = "A failed test synchronization must stop the fixture"
)]
async fn wait(notification: &Notify) {
    time::timeout(Duration::from_secs(5), notification.notified())
        .await
        .expect("host notification");
}

struct Host {
    starts: Arc<AtomicUsize>,
    started: Arc<Notify>,
    stopped: Arc<Notify>,
    stop_seen: Arc<AtomicBool>,
    gate: Arc<Semaphore>,
    input_settled: bool,
    refused_closed: bool,
}
impl Host {
    fn new(input_settled: bool) -> Arc<Self> {
        Arc::new(Self {
            starts: Arc::default(),
            started: Arc::default(),
            stopped: Arc::default(),
            stop_seen: Arc::default(),
            gate: Arc::new(Semaphore::new(0)),
            input_settled,
            refused_closed: false,
        })
    }
}
impl CaptchaHost for Host {
    #[expect(
        clippy::expect_used,
        reason = "The test owns this gate for the whole fake-host lifetime"
    )]
    fn solve(&self, _page: PageId, stop: CancellationToken) -> BoxFuture<'static, SolveReply> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        let (gate, stopped, seen, settled, refused) = (
            Arc::clone(&self.gate),
            Arc::clone(&self.stopped),
            Arc::clone(&self.stop_seen),
            self.input_settled,
            self.refused_closed,
        );
        Box::pin(async move {
            tokio::select! {
                () = stop.cancelled() => { seen.store(true, Ordering::SeqCst); stopped.notify_one(); let _permit = gate.acquire().await.expect("host gate"); }
                permit = gate.acquire() => { let _permit = permit.expect("host gate"); }
            }
            if stop.is_cancelled() {
                return SolveReply::Stopped;
            }
            if refused {
                return SolveReply::Refused {
                    closed: true,
                    message: "closed".to_owned(),
                };
            }
            SolveReply::Value {
                value: json!({"verdict":"already_clear","paidTasks":0}),
                log: vec!["captcha test event".to_owned()],
                input_settled: settled,
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captcha_unavailable_and_bad_page_are_unsent() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let outcome = runtime
        .run(fake_browser::run_request(
            "return await neo.solveCaptcha(page)",
            Duration::from_secs(5),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(outcome.status, RunStatus::Error);
    assert_eq!(outcome.calls[0].delivery, Some(Delivery::NotSent));
    assert!(
        outcome
            .error
            .expect("unavailable")
            .message
            .contains("unavailable")
    );
    let outcome = runtime
        .run(fake_browser::run_request(
            "return await neo.solveCaptcha({})",
            Duration::from_secs(5),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(outcome.error.expect("type error").name, "TypeError");
    assert!(outcome.calls.is_empty());
    runtime.drain().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captcha_value_delivery_is_done_only_for_input_settled() {
    for settled in [true, false] {
        let browser = FakeBrowser::start(Replies::default()).await;
        let host = Host::new(settled);
        host.gate.add_permits(1);
        let runtime = browser.runtime().await.with_captcha(host);
        let outcome = runtime
            .run(fake_browser::run_request(
                "return await neo.solveCaptcha(page)",
                Duration::from_secs(5),
                CancellationToken::new(),
            ))
            .await;
        assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
        assert_eq!(outcome.calls[0].method, "neo:solveCaptcha");
        assert_eq!(outcome.calls[0].params, json!({}));
        assert_eq!(
            outcome.calls[0].delivery,
            Some(if settled {
                Delivery::Done
            } else {
                Delivery::Unknown
            })
        );
        assert!(
            serde_json::to_string(&outcome.calls)
                .expect("records")
                .contains("captcha test event")
        );
        runtime.drain().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captcha_claim_refuses_same_run_and_cross_run_input_but_allows_reads() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let host = Host::new(true);
    let runtime = browser
        .runtime()
        .await
        .with_captcha(Arc::<Host>::clone(&host));
    let running = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime.run(fake_browser::run_request("return await Promise.allSettled([neo.solveCaptcha(page), page.click('#go'), neo.solveCaptcha(page), page.title()])", Duration::from_secs(10), CancellationToken::new())).await
        })
    };
    wait(&host.started).await;
    let blocked = runtime
        .run(fake_browser::run_request(
            "return await page.click('#go')",
            Duration::from_secs(5),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(blocked.status, RunStatus::Error);
    assert_eq!(blocked.calls[0].delivery, Some(Delivery::NotSent));
    assert!(
        blocked
            .error
            .expect("busy")
            .message
            .contains("neo.solveCaptcha is working")
    );
    let other = runtime
        .run(fake_browser::run_request(
            "const other=await context.newPage();await other.click('#go');return true",
            Duration::from_secs(5),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(
        other.status,
        RunStatus::Ok,
        "another tab has its own input turn: {other:?}"
    );
    assert!(
        other
            .calls
            .iter()
            .any(|call| call.delivery == Some(Delivery::Done))
    );
    host.gate.add_permits(1);
    let outcome = running.await.expect("run");
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(host.starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        outcome
            .calls
            .iter()
            .filter(|call| call.delivery == Some(Delivery::NotSent))
            .count(),
        2
    );
    assert!(outcome.calls.iter().any(|call| call.method == "title"));
    runtime.drain().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captcha_cancel_keeps_claim_until_the_host_finishes_draining() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let host = Host::new(true);
    let runtime = browser
        .runtime()
        .await
        .with_captcha(Arc::<Host>::clone(&host));
    let cancel = CancellationToken::new();
    let running = {
        let runtime = runtime.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            runtime
                .run(fake_browser::run_request(
                    "return await neo.solveCaptcha(page)",
                    Duration::from_secs(10),
                    cancel,
                ))
                .await
        })
    };
    wait(&host.started).await;
    cancel.cancel();
    wait(&host.stopped).await;
    let blocked = runtime
        .run(fake_browser::run_request(
            "return await neo.solveCaptcha(page)",
            Duration::from_secs(5),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(blocked.calls[0].delivery, Some(Delivery::NotSent));
    host.gate.add_permits(1);
    let outcome = running.await.expect("run");
    assert_eq!(outcome.status, RunStatus::Cancelled);
    assert_eq!(outcome.calls[0].delivery, Some(Delivery::Unknown));
    assert!(host.stop_seen.load(Ordering::SeqCst));
    // A fresh ordinary click proves the claim and input turn have both been released.
    let next = runtime
        .run(fake_browser::run_request(
            "await page.click('#go'); return true",
            Duration::from_secs(5),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(next.status, RunStatus::Ok, "{next:?}");
    runtime.drain().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captcha_refused_closed_preserves_not_sent() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let mut host = Host::new(true);
    Arc::get_mut(&mut host).expect("sole host").refused_closed = true;
    host.gate.add_permits(1);
    let runtime = browser.runtime().await.with_captcha(host);
    let outcome = runtime
        .run(fake_browser::run_request(
            "return await neo.solveCaptcha(page)",
            Duration::from_secs(5),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(outcome.status, RunStatus::Error);
    assert_eq!(outcome.calls[0].delivery, Some(Delivery::NotSent));
    runtime.drain().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captcha_file_navigation_refusals_send_no_navigation() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await.with_captcha(Host::new(true));
    for url in [
        "file:///x",
        "FILE:///x",
        " file:///x",
        "fi\tle:///x",
        "view-source:file:///x",
        "filesystem:file:///x",
    ] {
        let code = format!("return await page.goto({})", json!(url));
        let outcome = runtime
            .run(fake_browser::run_request(
                &code,
                Duration::from_secs(5),
                CancellationToken::new(),
            ))
            .await;
        assert_eq!(outcome.status, RunStatus::Error);
        assert!(
            outcome
                .error
                .expect("refusal")
                .message
                .contains("Neo refuses file:")
        );
    }
    assert!(
        !browser
            .exchanges()
            .iter()
            .any(|exchange| exchange.method() == "Page.navigate")
    );
    runtime.drain().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captcha_reload_and_both_history_directions_refuse_local_urls_before_writing() {
    for (code, urls, index) in [
        ("return await page.reload()", &["file:///key"][..], 0),
        (
            "return await page.goBack()",
            &["file:///key", "https://example.test"][..],
            1,
        ),
        (
            "return await page.goForward()",
            &["https://example.test", "view-source:file:///key"][..],
            0,
        ),
    ] {
        let browser = FakeBrowser::start(Replies {
            history_urls: Some(urls),
            history_index: index,
            ..Default::default()
        })
        .await;
        let runtime = browser.runtime().await.with_captcha(Host::new(true));
        let outcome = runtime
            .run(fake_browser::run_request(
                code,
                Duration::from_secs(5),
                CancellationToken::new(),
            ))
            .await;
        assert_eq!(outcome.status, RunStatus::Error, "{outcome:?}");
        assert!(
            outcome
                .error
                .expect("refusal")
                .message
                .contains("Neo refuses file:")
        );
        assert!(!browser.exchanges().iter().any(|exchange| matches!(
            exchange.method(),
            "Page.reload" | "Page.navigateToHistoryEntry"
        )));
        runtime.drain().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captcha_without_host_preserves_file_navigation_parity() {
    let browser = FakeBrowser::start(Replies {
        navigation: Some(fake_browser::NavigationScript {
            reply_after: Duration::ZERO,
            commit_after: Duration::ZERO,
            same_document: false,
            stop_loading: true,
            restored_from_bfcache: false,
        }),
        ..Default::default()
    })
    .await;
    let runtime = browser.runtime().await;
    let outcome = runtime
        .run(fake_browser::run_request(
            "await page.goto('file:///key', {waitUntil:'commit'}); return true",
            Duration::from_secs(5),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert!(
        browser
            .exchanges()
            .iter()
            .any(|exchange| exchange.method() == "Page.navigate")
    );
    runtime.drain().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captcha_waits_for_an_already_pressed_click_to_release_before_host_input() {
    let browser = FakeBrowser::start(Replies {
        mouse_press_delay: Some(Duration::from_millis(300)),
        ..Default::default()
    })
    .await;
    let host = Host::new(true);
    let runtime = browser
        .runtime()
        .await
        .with_captcha(Arc::<Host>::clone(&host));
    let click = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .run(fake_browser::run_request(
                    "await page.click('#go'); return true",
                    Duration::from_secs(5),
                    CancellationToken::new(),
                ))
                .await
        })
    };
    time::timeout(Duration::from_secs(5), async {
        loop {
            if browser.exchanges().iter().any(|exchange| {
                exchange.method() == "Input.dispatchMouseEvent"
                    && exchange.command["params"]["type"] == "mousePressed"
            }) {
                break;
            }
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("click pressed");
    let solve = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .run(fake_browser::run_request(
                    "return await neo.solveCaptcha(page)",
                    Duration::from_secs(5),
                    CancellationToken::new(),
                ))
                .await
        })
    };
    wait(&host.started).await;
    assert!(
        browser
            .exchanges()
            .iter()
            .any(|exchange| exchange.method() == "Input.dispatchMouseEvent"
                && exchange.command["params"]["type"] == "mouseReleased"
                && exchange.reply.is_some())
    );
    host.gate.add_permits(1);
    assert_eq!(click.await.expect("click task").status, RunStatus::Ok);
    assert_eq!(solve.await.expect("solve task").status, RunStatus::Ok);
    runtime.drain().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captcha_run_deadline_cancels_host_but_does_not_drop_its_cleanup() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let host = Host::new(true);
    let runtime = browser
        .runtime()
        .await
        .with_captcha(Arc::<Host>::clone(&host));
    let running = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .run(fake_browser::run_request(
                    "return await neo.solveCaptcha(page)",
                    Duration::from_secs(1),
                    CancellationToken::new(),
                ))
                .await
        })
    };
    wait(&host.started).await;
    wait(&host.stopped).await;
    let blocked = runtime
        .run(fake_browser::run_request(
            "return await page.click('#go')",
            Duration::from_secs(5),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(blocked.calls[0].delivery, Some(Delivery::NotSent));
    host.gate.add_permits(1);
    let outcome = running.await.expect("run");
    assert_eq!(outcome.status, RunStatus::Timeout);
    assert_eq!(outcome.calls[0].delivery, Some(Delivery::Unknown));
    runtime.drain().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captcha_awaited_value_releases_admission_before_the_script_continues() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let host = Host::new(true);
    host.gate.add_permits(1);
    let runtime = browser
        .runtime()
        .await
        .with_captcha(Arc::<Host>::clone(&host));
    let outcome = runtime.run(fake_browser::run_request("await neo.solveCaptcha(page); await page.click('#go'); return await neo.solveCaptcha(page)", Duration::from_secs(5), CancellationToken::new())).await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(host.starts.load(Ordering::SeqCst), 2);
    assert!(
        outcome
            .calls
            .iter()
            .all(|call| call.delivery == Some(Delivery::Done))
    );
    runtime.drain().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captcha_tab_close_cancels_the_host_without_abandoning_its_input_turn() {
    let browser = FakeBrowser::start(Replies {
        emit_close_events: true,
        ..Replies::default()
    })
    .await;
    let host = Host::new(true);
    let runtime = browser
        .runtime()
        .await
        .with_captcha(Arc::<Host>::clone(&host));
    let running = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .run(fake_browser::run_request(
                    "return await neo.solveCaptcha(page)",
                    Duration::from_secs(10),
                    CancellationToken::new(),
                ))
                .await
        })
    };
    wait(&host.started).await;
    let closing = runtime
        .run(fake_browser::run_request(
            "await page.close();return true",
            Duration::from_secs(5),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(closing.status, RunStatus::Ok, "{closing:?}");
    wait(&host.stopped).await;
    assert!(host.stop_seen.load(Ordering::SeqCst));
    host.gate.add_permits(1);
    let outcome = running.await.expect("run");
    assert!(matches!(
        outcome.status,
        RunStatus::Error | RunStatus::Cancelled
    ));
    assert_eq!(outcome.calls[0].delivery, Some(Delivery::Unknown));
    runtime.drain().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captcha_child_frame_goto_refuses_file_before_any_navigation() {
    let browser = FakeBrowser::start(Replies::default()).await;
    browser.page.lock().expect("page").frames = Some(FrameReplies {
        trees: [(fake_browser::PAGE.to_owned(), json!({"frameTree":{
            "frame":{"id":fake_browser::PAGE,"url":fake_browser::PAGE_URL},
            "childFrames":[{"frame":{"id":"child","parentId":fake_browser::PAGE,"name":"child","url":"https://example.test/child"}}]
        }}))].into(),
        ..Default::default()
    });
    let runtime = browser.runtime().await.with_captcha(Host::new(true));
    let outcome = runtime.run(fake_browser::run_request("const child=page.frames().find(f=>f.name()==='child');return await child.goto('file:///key')", Duration::from_secs(5), CancellationToken::new())).await;
    assert_eq!(outcome.status, RunStatus::Error, "{outcome:?}");
    assert!(
        outcome
            .error
            .as_ref()
            .expect("refusal")
            .message
            .contains("Neo refuses file:"),
        "{outcome:?}"
    );
    assert!(
        !browser
            .exchanges()
            .iter()
            .any(|exchange| exchange.method() == "Page.navigate")
    );
    runtime.drain().await;
}
