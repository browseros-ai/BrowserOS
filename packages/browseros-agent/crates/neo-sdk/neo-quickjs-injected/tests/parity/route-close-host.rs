//! Runs one script as a Neo run through the CDP port it is given (a proxy in the test), and
//! cancels it when a line arrives on stdin; then drains. The outer test reads the tab while the
//! run is still live (routeCloseSlowReply.test.mjs).
use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use neo_runtime::{PageId, RunRequest, Runtime, StartPage};
use std::env;
use std::fs;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    let port = args[1].parse().expect("port");
    let cdp = CdpClient::connect(ConnectOptions {
        reconnect_policy: ReconnectPolicy::KeepTrying,
        ..ConnectOptions::new(port)
    })
    .await
    .expect("connect");
    let runtime = Runtime::connect(cdp.clone()).await.expect("runtime");
    let cancel = CancellationToken::new();
    let stopping = cancel.clone();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        stopping.cancel();
    });
    let outcome = runtime
        .run(RunRequest {
            code: fs::read_to_string(&args[3]).expect("script"),
            page: StartPage::Page(PageId::new(args[2].clone())),
            timeout: Duration::from_secs(30),
            cancel,
            artifacts: None,
        })
        .await;
    println!("{}", serde_json::json!({ "stage": "outcome", "outcome": outcome }));
    tokio::time::timeout(Duration::from_secs(10), runtime.drain())
        .await
        .expect("drain");
    println!("{}", serde_json::json!({ "stage": "drained" }));
    drop(runtime);
    cdp.disconnect().await;
}
