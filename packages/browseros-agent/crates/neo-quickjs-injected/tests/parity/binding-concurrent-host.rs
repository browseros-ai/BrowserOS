//! Two runs of one Runtime on one tab (`binding-concurrent-review.test.mjs`): run A runs the
//! script at `argv[3]` on the tab `argv[2]` over the CDP port `argv[1]`; once the driver writes
//! `startB`, run B runs `argv[4]` on the same tab. `stopA` and `stopB` cancel each, and the host
//! prints `outcome <index> <status>` as each ends, then drains.
use std::time::Duration;
use std::{env, fs, io, thread};

use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use neo_runtime::{PageId, RunRequest, Runtime};
use tokio::sync::mpsc;
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
    let (lines, mut commands) = mpsc::unbounded_channel();
    thread::spawn(move || {
        loop {
            let mut line = String::new();
            if io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            if lines.send(line.trim().to_string()).is_err() {
                break;
            }
        }
    });
    let cancels = [CancellationToken::new(), CancellationToken::new()];
    let mut runs = Vec::new();
    for index in 0..2 {
        if index == 1 {
            assert_eq!(commands.recv().await.as_deref(), Some("startB"));
        }
        let runtime = runtime.clone();
        let page = PageId::new(args[2].clone());
        let code = fs::read_to_string(&args[3 + index]).expect("script");
        let cancel = cancels[index].clone();
        runs.push(tokio::spawn(async move {
            let outcome = runtime
                .run(RunRequest {
                    code,
                    page,
                    timeout: Duration::from_secs(30),
                    cancel,
                    artifacts: None,
                    session: None,
                })
                .await;
            println!("outcome {index} {:?}", outcome.status);
        }));
    }
    assert_eq!(commands.recv().await.as_deref(), Some("stopA"));
    cancels[0].cancel();
    assert_eq!(commands.recv().await.as_deref(), Some("stopB"));
    cancels[1].cancel();
    for run in runs {
        run.await.expect("run task");
    }
    tokio::time::timeout(Duration::from_secs(10), runtime.drain())
        .await
        .expect("drain");
    drop(runtime);
    cdp.disconnect().await;
}
