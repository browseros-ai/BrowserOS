//! Neo: A case's private native process. Bounded stdin/stdout queues feed one hosted runtime run;
//! browser actions still pass through the real SDK and Dispatcher. EOF cancels the run.

use std::env;
use std::fs;
use std::process::ExitCode;
use std::time::Duration;

use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use futures_util::future::BoxFuture;
use neo_runtime::{PageId, RunRequest, Runtime, StartPage, TestHost, TestHostError};
use serde_json::json;
use tokio::io::{self, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;
use url::Url;

/// Neo: bounds the private test transport independently of browser calls.
const FRAME_LIMIT: usize = 16 * 1024 * 1024;

struct Host {
    incoming: Mutex<mpsc::Receiver<String>>,
    outgoing: mpsc::Sender<String>,
}

impl TestHost for Host {
    fn receive(&self) -> BoxFuture<'_, Result<Option<String>, TestHostError>> {
        Box::pin(async { Ok(self.incoming.lock().await.recv().await) })
    }

    fn reply(&self, frame: String) -> Result<(), TestHostError> {
        if frame.len() > FRAME_LIMIT {
            return Err(TestHostError::new("native reply exceeded frame limit"));
        }
        self.outgoing
            .try_send(frame)
            .map_err(|error| TestHostError::new(format!("native reply backpressure: {error}")))
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = io::stderr().write_all(error.as_bytes()).await;
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let [endpoint, page, script] = args.as_slice() else {
        return Err("worker needs endpoint, page and script".into());
    };
    let url = Url::parse(endpoint).map_err(|error| error.to_string())?;
    let port = url.port().ok_or("CDP endpoint has no port")?;
    if url.host_str() != Some("127.0.0.1") && url.host_str() != Some("localhost") {
        return Err("worker only connects to its owned loopback browser".into());
    }
    let code = fs::read_to_string(script).map_err(|error| error.to_string())?;
    let cancel = CancellationToken::new();
    let (incoming_tx, incoming_rx) = mpsc::channel(64);
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<String>(256);
    let reader_cancel = cancel.clone();
    let reader = tokio::spawn(async move {
        let mut stdin = BufReader::new(io::stdin());
        loop {
            let mut frame = String::new();
            let mut limited = (&mut stdin).take((FRAME_LIMIT + 1) as u64);
            let read = tokio::select! {
                () = reader_cancel.cancelled() => break,
                result = limited.read_line(&mut frame) => result,
            };
            match read {
                Ok(length) if length > 0 && length <= FRAME_LIMIT && frame.ends_with('\n') => {
                    if incoming_tx.send(frame).await.is_err() {
                        break;
                    }
                }
                _ => break,
            }
        }
        reader_cancel.cancel();
    });
    let writer_cancel = cancel.clone();
    let writer = tokio::spawn(async move {
        let mut stdout = io::stdout();
        while let Some(frame) = outgoing_rx.recv().await {
            if stdout.write_all(frame.as_bytes()).await.is_err()
                || stdout.write_all(b"\n").await.is_err()
                || stdout.flush().await.is_err()
            {
                writer_cancel.cancel();
                return Err("native stdout closed".to_string());
            }
        }
        Ok(())
    });
    let cdp = CdpClient::connect(ConnectOptions {
        reconnect_policy: ReconnectPolicy::KeepTrying,
        ..ConnectOptions::new(port)
    })
    .await
    .map_err(|error| error.to_string())?;
    let runtime = Runtime::connect(cdp.clone())
        .await
        .map_err(|error| error.to_string())?;
    // Runner timeouts and dynamic slow/setTimeout changes own test cancellation. This is
    // only the finite supervisor guard; hitting it makes evidence incomplete.
    let outcome = runtime
        .run_with_test_host(
            RunRequest {
                code,
                page: StartPage::Page(PageId::new(page.clone())),
                timeout: Duration::from_mins(10),
                cancel: cancel.clone(),
                artifacts: None,
            },
            Box::new(Host {
                incoming: Mutex::new(incoming_rx),
                outgoing: outgoing_tx.clone(),
            }),
        )
        .await;
    // A returned outcome can still own writers and interceptor cleanup. Stop admitting
    // frames, then settle those tasks while CDP is connected. The independent I/O owner
    // bounds shutdown and marks a forced termination as incomplete evidence.
    cancel.cancel();
    reader.abort();
    runtime.drain().await;
    outgoing_tx
        .send(json!({ "event": "outcome", "outcome": outcome }).to_string())
        .await
        .map_err(|error| error.to_string())?;
    drop(outgoing_tx);
    cdp.disconnect().await;
    writer.await.map_err(|error| error.to_string())??;
    Ok(())
}
