//! A held screenshot cleanup must not block another tab or a closed tab's queued capture.
use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use neo_runtime::{PageId, RunRequest, RunStatus, Runtime};
use std::env;
use std::fs;
use std::time::Duration;
use tokio::time;
use tokio_util::sync::CancellationToken;

async fn marker(path: &str) {
    time::timeout(Duration::from_secs(20), async {
        while !fs::read_to_string(path).is_ok_and(|v| v == "ready") {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("driver marker");
}

#[tokio::test(flavor = "multi_thread")]
async fn held_cleanup_keeps_other_calls_live() {
    let port = env::var("NEO_REVIEW_CDP_PORT")
        .expect("port")
        .parse()
        .expect("number");
    let target = env::var("NEO_REVIEW_TARGET").expect("target");
    let other = env::var("NEO_REVIEW_OTHER").expect("other");
    let mode = env::var("NEO_REVIEW_MODE").expect("mode");
    let held = env::var("NEO_REVIEW_HELD").expect("held");
    let done = env::var("NEO_REVIEW_DONE").expect("done");
    let release = env::var("NEO_REVIEW_RELEASE").expect("release");
    let cdp = CdpClient::connect(ConnectOptions {
        reconnect_policy: ReconnectPolicy::KeepTrying,
        ..ConnectOptions::new(port)
    })
    .await
    .expect("CDP");
    let runtime = Runtime::connect(cdp.clone()).await.expect("Runtime");
    let cancel = CancellationToken::new();
    let first = tokio::spawn({
        let runtime = runtime.clone();
        let target = target.clone();
        let cancel = cancel.clone();
        async move {
            runtime
                .run(RunRequest {
                    code: "await page.screenshot({omitBackground:true,timeout:0})".into(),
                    page: PageId::new(target),
                    timeout: Duration::from_secs(30),
                    cancel,
                    artifacts: None,
                })
                .await
        }
    });
    marker(&held).await;
    cancel.cancel();
    let first = first.await.expect("first task");
    assert_eq!(first.status, RunStatus::Cancelled, "{first:?}");
    let code = if mode == "independent" {
        "const b=await page.screenshot({clip:{x:0,y:0,width:10,height:10},timeout:0});return {png:b[0]===137&&b[1]===80,alpha:b[25]===6||b[25]===4}"
    } else {
        "await page.evaluate(()=>{document.documentElement.dataset.turnQueued='yes'});return await page.screenshot({timeout:0}).then(()=>({captured:true}),error=>({closed:error.name==='Error'&&/closed|crashed/i.test(error.message)}))"
    };
    let second = runtime
        .run(RunRequest {
            code: code.into(),
            page: PageId::new(if mode == "independent" { other } else { target }),
            timeout: Duration::from_secs(20),
            cancel: CancellationToken::new(),
            artifacts: None,
        })
        .await;
    assert_eq!(second.status, RunStatus::Ok, "{second:?}");
    let value = second.value.as_ref().expect("value");
    if mode == "independent" {
        assert_eq!(value["png"], true, "{second:?}");
        assert_eq!(value["alpha"], false, "{second:?}");
    } else {
        assert_eq!(value["closed"], true, "{second:?}");
    }
    println!("TURN_RESULT {mode} {value}");
    fs::write(done, "ready").expect("done marker");
    marker(&release).await;
    time::timeout(Duration::from_secs(15), runtime.drain())
        .await
        .expect("cleanup drain");
    cdp.disconnect().await;
}
