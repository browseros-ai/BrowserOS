//! A run that outlives its context's close (`context-close-scope.test.mjs`): it runs the script
//! at `argv[3]` on the tab `argv[2]` over the CDP port `argv[1]` until the driver writes a line to
//! its stdin, then cancels the run and drains, so the driver can probe the tab after
//! `context.close()` resolved and before the run ends.
use std::time::Duration;
use std::{env, fs, io, thread};

use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use neo_runtime::{PageId, RunRequest, Runtime};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    let port = args[1].parse::<u16>().expect("CDP port");
    let cdp = CdpClient::connect(ConnectOptions {
        reconnect_policy: ReconnectPolicy::KeepTrying,
        ..ConnectOptions::new(port)
    })
    .await
    .expect("CDP");
    let runtime = Runtime::connect(cdp.clone()).await.expect("Runtime");
    let cancel = CancellationToken::new();
    let stopping = cancel.clone();
    thread::spawn(move || {
        let mut line = String::new();
        let _read = io::stdin().read_line(&mut line);
        stopping.cancel();
    });
    let outcome = runtime
        .run(RunRequest {
            origin: Default::default(),
            code: fs::read_to_string(&args[3]).expect("script"),
            page: PageId::new(args[2].clone()),
            timeout: Duration::from_secs(30),
            cancel,
            artifacts: None,
        })
        .await;
    println!("{:?}", outcome.status);
    tokio::time::timeout(Duration::from_secs(10), runtime.drain())
        .await
        .expect("drain");
    drop(runtime);
    cdp.disconnect().await;
}
