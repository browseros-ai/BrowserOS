//! A provider stream that goes quiet is not cut by a timer of the model layer's own: only the
//! turn's deadline bounds a model call, as before Rig (whose default ends a stream after five
//! minutes without an event).
//!
//! Rig times the stall with `futures-timer`, a real-time timer thread that tokio's paused clock
//! does not move, so this waits five minutes of real time and is run on demand:
//! `cargo test -p browseros-server --test model_idle -- --ignored`.

#[path = "support/fake_provider.rs"]
mod fake_provider;

use std::time::Duration;

use browseros_server::store::{Credentials, ProviderKind};
use fake_provider::{models, provider, request};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

#[tokio::test]
#[ignore = "waits five minutes of real time; run with --ignored"]
async fn a_quiet_stream_waits_past_five_minutes() -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}/v1", listener.local_addr()?);
    let (sent, first_event) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let mut request = vec![0; 64 * 1024];
        let _ = socket.read(&mut request).await?;
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n\
                  data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"thinking\"}}]}\n\n",
            )
            .await?;
        let _ = sent.send(());
        // Then nothing, with the connection held open.
        std::future::pending::<()>().await;
        anyhow::Ok(())
    });
    let (_dir, models) = models().await?;
    let model = models
        .resolve(
            provider(ProviderKind::OpenaiCompatible, base, Credentials::None),
            Uuid::new_v4(),
        )
        .await?;
    let request = request(&[]);
    let mut events = Vec::new();
    let step = model.step(&request, &mut events);
    tokio::pin!(step);
    tokio::select! {
        result = &mut step => anyhow::bail!("the step ended before the stream went quiet: {:?}", result.err()),
        sent = first_event => sent?,
    }
    // Rig's default would end the stream 300 s after its last event.
    let quiet = tokio::time::timeout(Duration::from_secs(310), &mut step).await;
    assert!(
        quiet.is_err(),
        "the step ended after a quiet stream: {:?}",
        quiet.ok().and_then(Result::err)
    );
    server.abort();
    Ok(())
}
