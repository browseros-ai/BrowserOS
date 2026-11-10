//! Neo: Exercise the logging adapter against the production runtime and a fake BrowserOS.
//! The captured subscriber text is the same one-line event format the local host writes.

#[path = "../../neo-runtime/tests/support/fake_browser.rs"]
#[expect(dead_code, reason = "the trace tests need only the basic fake browser")]
mod fake_browser;

use std::error::Error;
use std::future::Future;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fake_browser::{FakeBrowser, Replies, Reply};
use neo_runtime::RunStatus;
use tokio::runtime::Builder;
use tokio::time;
use tokio_util::sync::CancellationToken;
use tracing::instrument::WithSubscriber;
use tracing::subscriber;

#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl Write for Log {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("log lock poisoned"))?
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture<F: Future>(future: F) -> Result<(F::Output, String), Box<dyn Error>> {
    let log = Log::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::new("neo_trace=info"))
        .with_writer(move || writer.clone())
        .finish();
    let runtime = Builder::new_multi_thread().enable_all().build()?;
    let result = subscriber::with_default(subscriber, || runtime.block_on(future));
    let text = String::from_utf8(log.0.lock().map_err(|_| "log lock poisoned")?.clone())?;
    Ok((result, text))
}

#[test]
fn a_call_is_visible_before_a_blocked_synchronous_getter_is_released() -> Result<(), Box<dyn Error>>
{
    let log = Log::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::new("neo_trace=info"))
        .with_writer(move || writer.clone())
        .finish();
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()?;
    subscriber::with_default(subscriber, || {
        runtime.block_on(async {
            let browser = FakeBrowser::start(Replies {
                history: Reply {
                    answers: 1,
                    ..Reply::default()
                },
                ..Replies::default()
            })
            .await;
            let neo = browser.runtime().await;
            let cancel = CancellationToken::new();
            let request = fake_browser::run_request(
                "const pending = page.waitForTimeout(10000); page.url(); await pending;",
                Duration::from_secs(10),
                cancel.clone(),
            );
            // Put QuickJS on the sole async worker. The second history read never replies,
            // so only cancellation can release its synchronous getter after the assertion.
            let run = tokio::spawn(
                async move { neo_trace::run(&neo, request, None).await }.with_current_subscriber(),
            );
            let blocked = time::timeout(Duration::from_secs(2), async {
                loop {
                    let history: Vec<_> = browser
                        .exchanges()
                        .into_iter()
                        .filter(|exchange| exchange.method() == "Page.getNavigationHistory")
                        .collect();
                    if history.len() == 2 && history[1].reply.is_none() {
                        break;
                    }
                    time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .is_ok();
            let visible = time::timeout(Duration::from_secs(1), async {
                loop {
                    if String::from_utf8_lossy(&log.0.lock().expect("test log lock"))
                        .contains("call.start")
                    {
                        break;
                    }
                    time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .is_ok();
            let still_running = !run.is_finished();
            // Always release the stalled run, including on the old implementation's failure.
            cancel.cancel();
            let (id, outcome) = time::timeout(Duration::from_secs(5), run)
                .await
                .expect("cancel releases the getter")
                .expect("run task");
            assert!(
                blocked,
                "the synchronous getter reached the controlled barrier"
            );
            assert!(
                still_running,
                "the observation was made before releasing the getter"
            );
            assert!(
                visible,
                "call.start must be visible before cancellation releases the getter"
            );
            assert_eq!(outcome.status, RunStatus::Cancelled);
            let text =
                String::from_utf8(log.0.lock().expect("test log lock").clone()).expect("UTF-8 log");
            assert!(
                text.lines().all(|line| line.contains(&format!("run={id}"))),
                "subscriber context crosses the collector thread"
            );
            assert!(
                text.lines().last().expect("last event").contains("run.end"),
                "run.end remains last"
            );
        });
    });
    Ok(())
}

#[test]
fn records_code_calls_errors_and_final_status_as_single_lines() -> Result<(), Box<dyn Error>> {
    let ((id, outcome), text) = capture(async {
        let browser = FakeBrowser::start(Replies::default()).await;
        neo_trace::run(&browser.runtime().await, fake_browser::run_request(
            "await page.click('#go');\nawait page.click('#missing', {timeout: 5}).catch(() => {});\nthrow new Error('broken\\nline');",
            Duration::from_secs(10), CancellationToken::new(),
        ), None).await
    })?;
    assert_eq!(outcome.status, RunStatus::Error);
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(
        lines.len(),
        6,
        "each event occupies exactly one physical line: {text}"
    );
    assert!(
        lines.iter().all(|line| line.contains(&format!("run={id}"))),
        "{text}"
    );
    assert!(
        lines[0].contains("run.start") && lines[0].contains("\\n"),
        "{text}"
    );
    assert!(
        lines[1].contains("call.start") && lines[1].contains("line=1") && lines[1].contains("#go"),
        "{text}"
    );
    assert!(
        lines[2].contains("call.end")
            && lines[2].contains("result=\"ok\"")
            && !lines[2].contains(" log="),
        "{text}"
    );
    assert!(
        lines[4].contains("TimeoutError") && lines[4].contains(" log="),
        "{text}"
    );
    assert!(
        lines[5].contains("run.end") && lines[5].contains("broken\\nline"),
        "{text}"
    );
    assert_eq!(id.to_string().len(), 26);
    assert_eq!(
        serde_json::to_value(id).expect("id serialization"),
        id.to_string()
    );
    Ok(())
}

#[tokio::test]
async fn the_current_thread_runtime_still_returns_its_normal_error() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let (_, outcome) = neo_trace::run(
        &browser.runtime().await,
        fake_browser::run_request("return 1", Duration::from_secs(5), CancellationToken::new()),
        None,
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Error);
    assert!(
        outcome
            .error
            .expect("runtime flavor error")
            .message
            .contains("multi-threaded runtime"),
        "collector scheduling must preserve the runtime's existing flavor check"
    );
    assert!(
        outcome.calls.is_empty(),
        "unsupported runs make no browser calls"
    );
}

#[test]
fn concurrent_runs_have_distinct_correlated_timelines() -> Result<(), Box<dyn Error>> {
    let ((left, right), text) = capture(async {
        let browser = FakeBrowser::start(Replies::default()).await;
        let runtime = browser.runtime().await;
        let request = || {
            fake_browser::run_request(
                "await page.title()",
                Duration::from_secs(10),
                CancellationToken::new(),
            )
        };
        tokio::join!(
            neo_trace::run(&runtime, request(), None),
            neo_trace::run(&runtime, request(), None)
        )
    })?;
    assert_ne!(left.0, right.0);
    for (id, outcome) in [left, right] {
        assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
        let lines: Vec<_> = text
            .lines()
            .filter(|line| line.contains(&format!("run={id}")))
            .collect();
        assert_eq!(lines.len(), 4, "{text}");
        assert!(lines[0].contains("run.start"), "{text}");
        assert!(
            lines[1].contains("call.start") && lines[1].contains("call=1"),
            "{text}"
        );
        assert!(
            lines[2].contains("call.end") && lines[2].contains("call=1"),
            "{text}"
        );
        assert!(lines[3].contains("run.end"), "{text}");
    }
    Ok(())
}

#[test]
fn clips_multibyte_code_params_and_failure_logs_without_changing_the_outcome()
-> Result<(), Box<dyn Error>> {
    let code = format!(
        "// {}\nawait page.click('{}', {{timeout: 5}}).catch(() => {{}})",
        "é".repeat(32768),
        "x".repeat(5000)
    );
    let ((_, outcome), text) = capture(async {
        let browser = FakeBrowser::start(Replies::default()).await;
        neo_trace::run(
            &browser.runtime().await,
            fake_browser::run_request(&code, Duration::from_secs(10), CancellationToken::new()),
            None,
        )
        .await
    })?;
    assert_eq!(outcome.status, RunStatus::Ok);
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 4, "one physical line per event");
    assert!(
        lines[0].contains(&format!("…[+{} bytes]", code.len() - 65535)),
        "code clips before the split UTF-8 character"
    );
    assert!(lines[1].contains("…[+"), "large params are clipped");
    assert!(lines[2].contains("…[+"), "large failure logs are clipped");
    assert_eq!(
        outcome.calls[0].params["selector"].as_str().map(str::len),
        Some(5000)
    );
    assert!(outcome.calls[0].log.iter().any(|line| line.len() > 4096));
    Ok(())
}

#[test]
fn cancellation_still_writes_run_end_last() -> Result<(), Box<dyn Error>> {
    let ((_, outcome), text) = capture(async {
        let browser = FakeBrowser::start(Replies::default()).await;
        let runtime = browser.runtime().await;
        let cancel = CancellationToken::new();
        let cancellation = async {
            time::sleep(Duration::from_millis(100)).await;
            cancel.cancel();
        };
        let (result, ()) = tokio::join!(
            neo_trace::run(
                &runtime,
                fake_browser::run_request(
                    "await page.click('#missing')",
                    Duration::from_secs(10),
                    cancel.clone(),
                ),
                None,
            ),
            cancellation,
        );
        result
    })?;
    assert_eq!(outcome.status, RunStatus::Cancelled);
    assert!(
        text.lines()
            .last()
            .expect("last event")
            .contains("status=\"cancelled\""),
        "{text}"
    );
    assert!(
        text.lines().last().expect("last event").contains("run.end"),
        "{text}"
    );
    Ok(())
}

#[test]
fn persistence_failure_warns_without_changing_the_run() -> Result<(), Box<dyn Error>> {
    let ((status, value), text) = capture(async {
        let directory = tempfile::TempDir::new().expect("temporary state");
        let store = neo_trace::TraceStore::open(&directory.path().join("traces.db"))
            .await
            .expect("open");
        store.close().await;
        let browser = FakeBrowser::start(Replies::default()).await;
        let (_, outcome) = neo_trace::run(
            &browser.runtime().await,
            fake_browser::run_request(
                "return 42",
                Duration::from_secs(5),
                CancellationToken::new(),
            ),
            Some(&store),
        )
        .await;
        (outcome.status, outcome.value)
    })?;
    assert_eq!(status, RunStatus::Ok);
    assert_eq!(value, Some(serde_json::json!(42)));
    assert!(
        text.contains("trace store unavailable or queue full"),
        "{text}"
    );
    assert_eq!(
        text.matches("trace store unavailable or queue full")
            .count(),
        1,
        "one warning for repeated rejected commands"
    );
    Ok(())
}
