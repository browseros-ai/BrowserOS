//! The push door through the public facade: a collector that never answers, over TCP or a Unix socket, cannot hold `Telemetry::shutdown` past its budget, and the flush's requests are cut there.

use std::error::Error;
use std::io::{self, Read};
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::time::{Duration, Instant};

use neo_telemetry::label::RunStatus;
use neo_telemetry::{RunFacts, RunVerdict, Telemetry, TelemetryConfig};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use tokio::time;

/// The server's exit flush budget (`Defaults::SHUTDOWN_FLUSH`).
const BUDGET: Duration = Duration::from_secs(1);

/// Telemetry pushing to `endpoint` records one finished run; returns how long
/// `shutdown(BUDGET)` then took.
async fn shutdown_time(endpoint: &str) -> Result<Duration, Box<dyn Error>> {
    let config = TelemetryConfig::from_env(&|name| match name {
        "OTEL_EXPORTER_OTLP_ENDPOINT" => Some(endpoint.to_owned()),
        // Nothing leaves on a timer first, so the shutdown flush carries the run.
        "OTEL_BSP_SCHEDULE_DELAY" | "OTEL_METRIC_EXPORT_INTERVAL" => Some("600000".to_owned()),
        _ => None,
    })?;
    let telemetry = Telemetry::new(&config)?;
    telemetry
        .start_run(RunFacts {
            run_id: &"run-before-exit",
            task_id: None,
            parent: None,
        })
        .finish(RunVerdict {
            status: RunStatus::Ok,
            error: None,
        });
    let start = Instant::now();
    // Only the time is under test: a flush its collector never answered may well report that.
    let _ = telemetry.shutdown(BUDGET).await;
    Ok(start.elapsed())
}

/// Reads a connection the flush left at the collector (non-blocking) until it ends, which it
/// does only once the client dropped the request: the shutdown deadline cut it. Left in flight,
/// the request would hold the connection for its 10 s timeout, past this wait.
async fn assert_cut(mut connection: impl Read) -> Result<(), Box<dyn Error>> {
    let waiting = Instant::now();
    let mut buffer = [0; 4096];
    loop {
        match connection.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::ConnectionReset => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if waiting.elapsed() > BUDGET {
                    return Err("a flush request is still in flight after the deadline".into());
                }
                time::sleep(Duration::from_millis(10)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn assert_in_budget(took: Duration) {
    assert!(
        took < Duration::from_millis(1200),
        "a collector that never answers held shutdown({BUDGET:?}) for {took:?}"
    );
    assert!(
        took >= Duration::from_millis(900),
        "the flush must have waited on the collector until the budget, so push really ran: \
         {took:?}"
    );
}

#[tokio::test]
async fn a_hung_collector_over_tcp_cannot_hold_shutdown_past_its_budget()
-> Result<(), Box<dyn Error>> {
    // Bound and never accepted: the kernel takes the connection and the request; nobody answers.
    let collector = TcpListener::bind("127.0.0.1:0")?;
    let took = shutdown_time(&format!("http://{}", collector.local_addr()?)).await?;
    assert_in_budget(took);
    collector.set_nonblocking(true)?;
    let mut cut = 0;
    while let Ok((connection, _)) = collector.accept() {
        connection.set_nonblocking(true)?;
        assert_cut(connection).await?;
        cut += 1;
    }
    assert!(
        cut >= 2,
        "the span and the metric flush each reached the collector and were cut: {cut}"
    );
    Ok(())
}

#[tokio::test]
async fn a_hung_collector_over_a_unix_socket_cannot_hold_shutdown_past_its_budget()
-> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let socket = dir.path().join("collector.sock");
    let collector = UnixListener::bind(&socket)?;
    let path = socket.to_string_lossy();
    let endpoint = format!(
        "http+unix://{}",
        utf8_percent_encode(&path, NON_ALPHANUMERIC)
    );
    let took = shutdown_time(&endpoint).await?;
    assert_in_budget(took);
    collector.set_nonblocking(true)?;
    let mut cut = 0;
    while let Ok((connection, _)) = collector.accept() {
        connection.set_nonblocking(true)?;
        assert_cut(connection).await?;
        cut += 1;
    }
    assert!(
        cut >= 2,
        "the span and the metric flush each reached the collector and were cut: {cut}"
    );
    Ok(())
}
