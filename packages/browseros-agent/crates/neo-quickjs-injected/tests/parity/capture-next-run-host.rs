//! Review-only persistent-runtime cancellation probe; launched by the recording browser driver.
use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use neo_runtime::{PageId, RunRequest, RunStatus, Runtime};
use serde_json::json;
use std::env;
use std::fs;
use std::time::Duration;
use tokio::time;
use tokio_util::sync::CancellationToken;

#[tokio::test(flavor = "multi_thread")]
async fn cancel_and_reuse_runtime() {
    let port = env::var("NEO_REVIEW_CDP_PORT")
        .expect("proxy port")
        .parse::<u16>()
        .expect("port");
    let target = env::var("NEO_REVIEW_TARGET").expect("isolated target");
    let marker = env::var("NEO_REVIEW_MARKER").expect("browser effect marker");
    let code = env::var("NEO_REVIEW_CODE").expect("capture code");
    let drop_future = env::var("NEO_REVIEW_DROP").as_deref() == Ok("1");
    let cdp = CdpClient::connect(ConnectOptions {
        reconnect_policy: ReconnectPolicy::KeepTrying,
        ..ConnectOptions::new(port)
    })
    .await
    .expect("proxy connection");
    let runtime = Runtime::connect(cdp.clone()).await.expect("Neo runtime");
    let cancel = CancellationToken::new();
    let task = tokio::spawn({
        let runtime = runtime.clone();
        let cancel = cancel.clone();
        let target = target.clone();
        async move {
            runtime
                .run(RunRequest {
                    origin: Default::default(),
                    code,
                    page: PageId::new(target),
                    timeout: Duration::from_secs(30),
                    cancel,
                    artifacts: None,
                })
                .await
        }
    });
    time::timeout(Duration::from_secs(20), async {
        while !fs::read_to_string(&marker).is_ok_and(|s| s == "held") {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("browser effect entered before cancel");
    if drop_future {
        task.abort();
        let error = task.await.expect_err("aborted");
        assert!(error.is_cancelled());
    } else {
        cancel.cancel();
        let outcome = task.await.expect("run task");
        assert_eq!(outcome.status, RunStatus::Cancelled, "{outcome:#?}");
    }
    let later=runtime.run(RunRequest{code:"const b=await page.screenshot({clip:{x:0,y:0,width:10,height:10}});return {alpha:b[25]===6||b[25]===4}".into(),page:PageId::new(target),timeout:Duration::from_secs(10),cancel:CancellationToken::new(),artifacts:None}).await;
    assert_eq!(later.status, RunStatus::Ok, "{later:#?}");
    time::timeout(Duration::from_secs(10), runtime.drain())
        .await
        .expect("cleanup drained");
    time::sleep(Duration::from_millis(250)).await;
    cdp.disconnect().await;
    println!(
        "NEXT_CAPTURE {}",
        serde_json::to_string(&later.value).expect("capture value")
    );
    assert_eq!(
        later.value,
        Some(json!({"alpha":false})),
        "the next run captures with the default opaque background"
    );
}
