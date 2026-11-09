//! Real Runtime cancellation during init-script setup; the outer browser driver probes the next document.
use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use neo_runtime::{PageId, RunRequest, RunStatus, Runtime};
use std::env;
use std::fs;
use std::time::Duration;
use tokio::time;
use tokio_util::sync::CancellationToken;

async fn marker(path: &str) {
    time::timeout(Duration::from_secs(20), async {
        while !fs::read_to_string(path).is_ok_and(|s| s == "ready") {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("driver marker");
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_during_setup() {
    let port = env::var("NEO_REVIEW_CDP_PORT")
        .expect("port")
        .parse::<u16>()
        .expect("number");
    let target = env::var("NEO_REVIEW_TARGET").expect("target");
    let held = env::var("NEO_REVIEW_HELD").expect("held marker");
    let returned = env::var("NEO_REVIEW_RETURNED").expect("returned marker");
    let release = env::var("NEO_REVIEW_RELEASE").expect("release marker");
    let cdp = CdpClient::connect(ConnectOptions {
        reconnect_policy: ReconnectPolicy::KeepTrying,
        ..ConnectOptions::new(port)
    })
    .await
    .expect("CDP");
    let runtime = Runtime::connect(cdp.clone()).await.expect("Runtime");
    let cancel = CancellationToken::new();
    let run = tokio::spawn({
        let runtime = runtime.clone();
        let cancel = cancel.clone();
        async move {
            runtime.run(RunRequest {
            code: "await page.context().addInitScript(()=>{window.oldRunInit=17});await page.evaluate(()=>0);await new Promise(()=>{})".into(),
            page: PageId::new(target), timeout: Duration::from_secs(30), cancel, artifacts: None,
            hooks: None,
        }).await
        }
    });
    marker(&held).await;
    cancel.cancel();
    let outcome = run.await.expect("run task");
    assert_eq!(outcome.status, RunStatus::Cancelled, "{outcome:?}");
    fs::write(returned, "ready").expect("returned marker");
    marker(&release).await;
    time::timeout(Duration::from_secs(15), runtime.drain())
        .await
        .expect("drain cleanup");
    cdp.disconnect().await;
}
