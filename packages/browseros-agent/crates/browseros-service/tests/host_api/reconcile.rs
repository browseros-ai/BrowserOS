//! A reconcile as the app runs it when the browser link reconnects: a fence, a listing of the
//! open tabs, then `Service::retain_tabs`, with a host changing a tab that opened after the
//! listing through the routes in between. And a lock or flag a host asks for on a tab that
//! closes while the route waits on a reconcile, or before the route answers.

use crate::support::{self, BoxError, Harness, Op, Reply, Start, TestResult};
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use browseros_core::TabId;
use browseros_takeover::LockTarget;
use browseros_trace::{
    Delivery, Effect, OperationId, PrivacyRecord, PrivacyStore, TraceError, TraceSink,
};
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;

fn agent_may_write(h: &Harness, tab: i64) -> bool {
    h.service
        .locks()
        .check(LockTarget::Tab(TabId(tab)), Effect::Write)
        .is_ok()
}

async fn post_ok(h: &Harness, uri: &str, body: &str) -> Result<Value, BoxError> {
    let reply = h.post(uri, body).await?;
    assert_eq!(reply.status, StatusCode::OK, "{uri}: {}", reply.body);
    Ok(reply.body)
}

async fn lock(h: &Harness, tab: i64) -> Result<Value, BoxError> {
    let body = format!(r#"{{"tabId":{tab},"ttlSeconds":60}}"#);
    post_ok(h, "/v1/locks", &body).await
}

async fn make_private(h: &Harness, tab: i64) -> Result<Value, BoxError> {
    post_ok(h, &format!("/v1/tabs/{tab}/private"), "").await
}

/// Whether the host reads `lock` as ended.
async fn released(h: &Harness, lock: &Value) -> Result<bool, BoxError> {
    let id = lock["id"].as_str().ok_or("a lock without an id")?;
    Ok(h.get(&format!("/v1/locks/{id}")).await?.body["releasedAt"].is_i64())
}

/// `POST uri` in a task of its own, for a request that may wait on something the test holds.
fn spawn_post(h: &Harness, uri: &str, body: &str) -> JoinHandle<Result<Reply, BoxError>> {
    use tower::ServiceExt;

    let router = h.router.clone();
    let request = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()));
    tokio::spawn(async move { Reply::read(router.oneshot(request?).await?).await })
}

#[tokio::test]
async fn a_reconcile_keeps_a_lock_and_flag_a_host_set_on_a_tab_newer_than_its_listing() -> TestResult
{
    let h = support::harness().await?;
    h.browser.open_tab(5);
    h.browser.open_tab(3);
    let before = lock(&h, 3).await?;
    make_private(&h, 3).await?;
    h.browser.close_tab(3);
    let fence = h.service.tab_fence();
    let listing = [TabId(5)];
    h.browser.open_tab(77);
    let newer = lock(&h, 77).await?;
    make_private(&h, 77).await?;

    h.service.retain_tabs(&listing, fence).await;

    assert!(
        !agent_may_write(&h, 77),
        "77's lock still refuses the agent"
    );
    assert!(
        !released(&h, &newer).await?,
        "the host reads 77's lock as held"
    );
    assert!(
        h.privacy.is_private(TabId(77)).is_some(),
        "77 stays private"
    );
    assert!(agent_may_write(&h, 3), "3 closed before the listing");
    assert!(released(&h, &before).await?, "3's lock ended with it");
    assert!(
        h.privacy.is_private(TabId(3)).is_none(),
        "3's flag is forgotten"
    );
    Ok(())
}

#[tokio::test]
async fn the_next_reconcile_ends_that_state_when_its_listing_still_lacks_the_tab() -> TestResult {
    let h = support::harness().await?;
    h.browser.open_tab(5);
    let fence = h.service.tab_fence();
    h.browser.open_tab(77);
    let newer = lock(&h, 77).await?;
    make_private(&h, 77).await?;
    h.service.retain_tabs(&[TabId(5)], fence).await;
    assert!(!agent_may_write(&h, 77), "the first reconcile keeps 77");

    h.browser.close_tab(77);
    let next = h.service.tab_fence();
    h.service.retain_tabs(&[TabId(5)], next).await;
    assert!(
        agent_may_write(&h, 77),
        "77's stamp is older than this fence, and the listing lacks 77"
    );
    assert!(released(&h, &newer).await?);
    assert!(h.privacy.is_private(TabId(77)).is_none());
    Ok(())
}

#[tokio::test]
async fn a_reconcile_keeps_a_fit_and_a_takeover_a_host_began_on_tabs_newer_than_its_listing()
-> TestResult {
    let h = support::harness().await?;
    h.browser.open_tab(5);
    let fence = h.service.tab_fence();
    h.browser.open_tab(78);
    h.browser.open_tab(79);
    let fit = post_ok(&h, "/v1/tabs/78/fit", r#"{"width":390,"height":844}"#).await?;
    let takeover = post_ok(&h, "/v1/takeovers", r#"{"tabId":79}"#).await?;

    h.service.retain_tabs(&[TabId(5)], fence).await;

    let fit = h
        .get(&format!(
            "/v1/fits/{}",
            fit["id"].as_str().ok_or("a fit id")?
        ))
        .await?;
    assert!(fit.body.get("restoredAt").is_none(), "{}", fit.body);
    let takeover = h
        .get(&format!(
            "/v1/takeovers/{}",
            takeover["id"].as_str().ok_or("a takeover id")?
        ))
        .await?;
    assert_eq!(takeover.body["state"], "active", "{}", takeover.body);
    assert!(!agent_may_write(&h, 79), "the takeover's lock holds");
    Ok(())
}

/// Which of a tab's two privacy writes a `HeldStore` holds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Held {
    /// The commit of its flag.
    Commit,
    /// The delete of its flag.
    Forget,
}

/// The trace store as a `PrivacyStore`, except that the `held` write on `tab` waits until the
/// test opens `gate`. `began` reads true once it waits.
struct HeldStore {
    store: TraceSink,
    tab: TabId,
    held: Held,
    began: watch::Sender<bool>,
    gate: watch::Receiver<bool>,
}

impl HeldStore {
    /// Returns at once, unless `write` on `tab` is the held one: then once the gate opens.
    async fn pass(&self, tab: TabId, write: Held) {
        if tab != self.tab || write != self.held {
            return;
        }
        self.began.send_replace(true);
        let mut gate = self.gate.clone();
        // The test keeps the gate's sender, so the wait ends only when it opens.
        let _ = gate.wait_for(|open| *open).await;
    }
}

impl PrivacyStore for HeldStore {
    fn persist_private(
        &self,
        tab: TabId,
        record: PrivacyRecord,
    ) -> BoxFuture<'_, Result<(), TraceError>> {
        Box::pin(async move {
            self.pass(tab, Held::Commit).await;
            self.store.persist_private(tab, record).await
        })
    }

    fn forget_private(&self, tab: TabId) -> BoxFuture<'_, Result<(), TraceError>> {
        Box::pin(async move {
            self.pass(tab, Held::Forget).await;
            self.store.forget_private(tab).await
        })
    }
}

#[tokio::test]
async fn a_lock_a_host_asks_for_while_a_reconcile_hands_its_tabs_to_the_parts_is_kept() -> TestResult
{
    use futures_util::FutureExt;

    let (open, gate) = watch::channel(false);
    let (began, mut commit_began) = watch::channel(false);
    // A takeover start on 3 holds the takeovers' start mutex while its commit waits, and a
    // reconcile's first part waits for that mutex.
    let h = support::harness_with(move |sink| {
        Arc::new(HeldStore {
            store: sink.clone(),
            tab: TabId(3),
            held: Held::Commit,
            began,
            gate,
        })
    })
    .await?;
    h.browser.open_tab(3);
    let fence = h.service.tab_fence();
    let listing = [TabId(3)];
    h.browser.open_tab(77);
    let start = spawn_post(&h, "/v1/takeovers", r#"{"tabId":3}"#);
    tokio::time::timeout(
        Duration::from_secs(5),
        commit_began.wait_for(|began| *began),
    )
    .await??;

    let mut reconcile = pin!(h.service.retain_tabs(&listing, fence));
    assert!(
        reconcile.as_mut().now_or_never().is_none(),
        "the takeover start on 3 holds the reconcile at its first part"
    );
    let lock = spawn_post(&h, "/v1/locks", r#"{"tabId":77,"ttlSeconds":60}"#);
    // Time for the lock to reach the locks' table, unless something makes it wait.
    tokio::time::sleep(Duration::from_millis(200)).await;
    open.send_replace(true);
    tokio::time::timeout(Duration::from_secs(10), reconcile).await?;

    let lock = tokio::time::timeout(Duration::from_secs(5), lock).await???;
    assert_eq!(lock.status, StatusCode::OK, "{}", lock.body);
    let takeover = tokio::time::timeout(Duration::from_secs(5), start).await???;
    assert_eq!(takeover.status, StatusCode::OK, "{}", takeover.body);
    assert!(!agent_may_write(&h, 77), "77's lock outlived the reconcile");
    assert!(!released(&h, &lock.body).await?);
    assert_eq!(takeover.body["state"], "active");
    Ok(())
}

#[tokio::test]
async fn a_tab_that_closes_while_its_lock_and_flag_wait_on_a_reconcile_gets_neither() -> TestResult
{
    use futures_util::FutureExt;

    let (open, gate) = watch::channel(false);
    let (began, mut forget_began) = watch::channel(false);
    let h = support::harness_with(move |sink| {
        Arc::new(HeldStore {
            store: sink.clone(),
            tab: TabId(9),
            held: Held::Forget,
            began,
            gate,
        })
    })
    .await?;
    h.browser.open_tab(9);
    h.browser.open_tab(77);
    make_private(&h, 9).await?;
    // 9 closes while no socket is watched, so the reconcile's last part forgets its flag, and
    // the reconcile holds the stamp table until the test lets that delete through.
    h.browser.close_tab(9);
    let fence = h.service.tab_fence();
    let listing = [TabId(77)];
    let service = h.service.clone();
    let reconcile = tokio::spawn(async move { service.retain_tabs(&listing, fence).await });
    tokio::time::timeout(
        Duration::from_secs(5),
        forget_began.wait_for(|began| *began),
    )
    .await??;

    let lock = spawn_post(&h, "/v1/locks", r#"{"tabId":77,"ttlSeconds":60}"#);
    let private = spawn_post(&h, "/v1/tabs/77/private", "");
    // Time for both requests to reach the stamp table, where they wait.
    tokio::time::sleep(Duration::from_millis(200)).await;
    h.browser.close_tab(77);
    let mut closed = pin!(h.service.tab_closed(TabId(77)));
    assert!(
        closed.as_mut().now_or_never().is_none(),
        "77's close waits on the stamp table the reconcile holds"
    );
    open.send_replace(true);
    tokio::time::timeout(Duration::from_secs(10), reconcile).await??;
    let lock = tokio::time::timeout(Duration::from_secs(5), lock).await???;
    let private = tokio::time::timeout(Duration::from_secs(5), private).await???;
    tokio::time::timeout(Duration::from_secs(5), closed).await?;

    assert!(
        agent_may_write(&h, 77),
        "77 closed while both requests waited, so no lock outlives it"
    );
    assert!(
        h.privacy.is_private(TabId(77)).is_none(),
        "and no private flag outlives it"
    );
    assert!(
        !h.store.private_tabs().await?.contains_key(&TabId(77)),
        "not even on disk, where the next tab 77 could inherit it"
    );
    lock.assert_error(StatusCode::NOT_FOUND, "tab_not_found");
    private.assert_error(StatusCode::NOT_FOUND, "tab_not_found");
    Ok(())
}

#[tokio::test]
async fn a_tab_that_closes_while_its_lock_and_flag_are_made_keeps_neither() -> TestResult {
    let (open, gate) = watch::channel(false);
    let (began, mut commit_began) = watch::channel(false);
    let h = support::harness_with(move |sink| {
        Arc::new(HeldStore {
            store: sink.clone(),
            tab: TabId(77),
            held: Held::Commit,
            began,
            gate,
        })
    })
    .await?;
    h.browser.open_tab(77);
    // An agent's click on 77 is in flight, so a lock on 77 answers only once the click ends.
    let run = support::start_run(&h.sink, Start::default());
    let click = OperationId::new(run, 1);
    support::op_started(&h.sink, click, Op::write(77, 1));
    let lock = spawn_post(&h, "/v1/locks", r#"{"tabId":77,"ttlSeconds":60}"#);
    let private = spawn_post(&h, "/v1/tabs/77/private", "");
    tokio::time::timeout(
        Duration::from_secs(5),
        commit_began.wait_for(|began| *began),
    )
    .await??;
    tokio::time::timeout(Duration::from_secs(5), async {
        while agent_may_write(&h, 77) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;

    // Both changes are in place, and 77 closes before either route answers. The browser link
    // has not reported the close yet.
    h.browser.close_tab(77);
    support::op_ended(&h.sink, click, Delivery::Done, None);
    open.send_replace(true);
    let lock = tokio::time::timeout(Duration::from_secs(5), lock).await???;
    let private = tokio::time::timeout(Duration::from_secs(5), private).await???;

    assert!(
        agent_may_write(&h, 77),
        "77 was gone when the lock route checked again, so it ended its lock"
    );
    assert!(
        h.privacy.is_private(TabId(77)).is_none(),
        "and the private route forgot its flag"
    );
    assert!(
        !h.store.private_tabs().await?.contains_key(&TabId(77)),
        "on disk too, where the next tab 77 could inherit it"
    );
    lock.assert_error(StatusCode::NOT_FOUND, "tab_not_found");
    private.assert_error(StatusCode::NOT_FOUND, "tab_not_found");
    Ok(())
}
