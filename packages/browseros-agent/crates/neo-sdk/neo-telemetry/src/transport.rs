//! Where every OTLP byte goes out, and the one place that owns export time.
//!
//! ```text
//!  SDK batch-processor thread ──┐ export() runs under futures' block_on: it spawns the
//!  SDK periodic-reader thread ──┤ request onto the I/O runtime and awaits its JoinHandle
//!                               ▼
//!  neo-telemetry-io thread: current-thread tokio, a reqwest 0.12 client per endpoint
//!    (rustls for https, `ClientBuilder::unix_socket` for http+unix)
//!    each request: select!(send, retried for spans | min(start + timeout, shutdown deadline))
//! ```
//!
//! Three decisions live here:
//! - I/O runs on the transport's own thread and runtime, never the server's: no
//!   `Handle::current()` panic outside a runtime, no starving behind busy server workers.
//! - Every request ends by `min(start + OTEL_EXPORTER_OTLP_TIMEOUT, deadline)`.
//!   [`Transport::arm_deadline`] cuts requests already in flight too, so the SDK's threads come
//!   back in time whatever the collector does. That is what bounds shutdown, even though the
//!   SDK's periodic reader ignores its own shutdown timeout and waits up to 5 s.
//! - A failed export is logged here, at a bounded rate, and the exporter is told it went
//!   through. The SDK logs every failed export twice (a WARN from the OTLP retry loop, an ERROR
//!   from the processor or reader), so a collector that stays down would write two lines per
//!   batch for as long as Neo runs. Nothing else reads an export's result: the SDK neither
//!   retries nor backs off on it.

use std::error::Error;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::{Duration, Instant as StdInstant};
use std::{future, io, mem, thread};

use async_trait::async_trait;
use bytes::Bytes;
use http::header::{CONTENT_ENCODING, CONTENT_TYPE, HeaderMap, USER_AGENT};
use http::{Request, Response, StatusCode};
use opentelemetry_http::{HttpClient, HttpError};
use reqwest::redirect::Policy;
use tokio::runtime::{Builder, Handle};
use tokio::sync::{oneshot, watch};
use tokio::time::{self, Instant};
use tracing::{info, warn};

use crate::TelemetryError;
use crate::config::{Headers, SignalPush};

/// The cap opentelemetry-http's own clients put on a collector's answer. An answer is a few bytes
/// (an empty body or a partial-success note); anything bigger is not a collector.
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// While exports keep failing, at most one warning per signal this often.
const REPORT_EVERY: Duration = Duration::from_secs(60);

/// The push door's I/O: the `neo-telemetry-io` thread and the shutdown deadline. Built only when
/// a signal pushes, and owned by `otlp::Push`. The clients it hands out hold the thread too, so it
/// stops only once the last exporter is gone.
#[derive(Debug)]
pub(crate) struct Transport {
    io: Arc<IoThread>,
    /// Unarmed (`None`) until shutdown starts. Every client's requests watch it.
    deadline: watch::Sender<Option<Instant>>,
}

impl Transport {
    /// Spawns the I/O thread and waits for its runtime, so a failure is a startup error rather
    /// than a panic on the first export. Callable inside or outside a tokio runtime.
    pub(crate) fn start() -> Result<Self, TelemetryError> {
        let io = IoThread::spawn().map_err(|error| TelemetryError::Pipeline {
            stage: "start",
            reason: format!("the telemetry I/O thread did not start: {error}"),
        })?;
        Ok(Self {
            io: Arc::new(io),
            deadline: watch::Sender::new(None),
        })
    }

    /// The HTTP client `signal`'s OTLP exporter sends through, connected to its endpoint: over
    /// TCP (rustls for `https://`) or to its Unix socket, with its headers and timeout. Redirects
    /// are not followed (they could carry the headers elsewhere) and `HTTP(S)_PROXY` is not used:
    /// telemetry goes straight to its collector.
    pub(crate) fn client(
        &self,
        signal: Signal,
        push: &SignalPush,
    ) -> Result<ExportClient, TelemetryError> {
        let builder = reqwest::Client::builder()
            .redirect(Policy::none())
            .no_proxy();
        let builder = match push.endpoint.socket() {
            Some(socket) => builder.unix_socket(socket),
            None => builder,
        };
        let http = builder.build().map_err(|error| TelemetryError::Pipeline {
            stage: "start",
            reason: format!(
                "the OTLP {} client could not be built: {}",
                signal.as_str(),
                causes(&error)
            ),
        })?;
        Ok(ExportClient {
            signal,
            http,
            io: Arc::clone(&self.io),
            deadline: self.deadline.subscribe(),
            headers: push.headers.clone(),
            timeout: push.timeout,
            failures: FailureLog::default(),
        })
    }

    /// From now on, every request, in flight or new, ends within `budget`. Shutdown calls this
    /// before it shuts the providers down, so the SDK's export threads return by then. Only the
    /// first call counts: shutdown is idempotent.
    pub(crate) fn arm_deadline(&self, budget: Duration) {
        // From std's clock, not tokio's: a caller on a test runtime with a paused clock must not
        // move the deadline the I/O runtime, on real time, enforces.
        let deadline = StdInstant::now().checked_add(budget).map(Instant::from_std);
        self.deadline.send_if_modified(|armed| {
            if armed.is_some() || deadline.is_none() {
                return false;
            }
            *armed = deadline;
            true
        });
    }
}

/// Which signal an exporter pushes. It decides the retries and names the signal in warnings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Signal {
    /// Spans, `/v1/traces`.
    Traces,
    /// Metrics, `/v1/metrics`.
    Metrics,
}

impl Signal {
    fn as_str(self) -> &'static str {
        match self {
            Self::Traces => "traces",
            Self::Metrics => "metrics",
        }
    }

    /// A span batch that fails is lost, so a failed span request is sent up to twice more. A
    /// metric push carries cumulative totals that the next push repeats, so it is never resent.
    fn retries(self) -> u32 {
        match self {
            Self::Traces => 2,
            Self::Metrics => 0,
        }
    }
}

/// The `neo-telemetry-io` thread's runtime. It goes with the last [`Arc`] (the transport's and
/// every client's): the stop sender drops, the thread leaves `block_on`, and the runtime ends
/// with `shutdown_background`, which waits for nothing, neither a request an SDK thread gave up
/// on nor a DNS lookup still blocking.
#[derive(Debug)]
struct IoThread {
    handle: Handle,
    /// Held only to be dropped: that is the thread's stop signal.
    _stop: oneshot::Sender<()>,
}

impl IoThread {
    fn spawn() -> io::Result<Self> {
        let (ready, started) = mpsc::sync_channel(1);
        let (stop, stopped) = oneshot::channel::<()>();
        thread::Builder::new()
            .name("neo-telemetry-io".to_owned())
            .spawn(move || {
                let runtime = match Builder::new_current_thread().enable_all().build() {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        // A send error means `spawn` is no longer waiting; nothing to tell.
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                let _ = ready.send(Ok(runtime.handle().clone()));
                // A current-thread runtime runs its tasks only inside `block_on`, so this is
                // where every request runs. It returns once the stop sender is dropped.
                let _ = runtime.block_on(stopped);
                runtime.shutdown_background();
            })?;
        let handle = started.recv().map_err(io::Error::other)??;
        Ok(Self {
            handle,
            _stop: stop,
        })
    }
}

/// One signal's HTTP client, handed to its OTLP exporter as opentelemetry-http's `HttpClient`.
#[derive(Debug)]
pub(crate) struct ExportClient {
    signal: Signal,
    http: reqwest::Client,
    io: Arc<IoThread>,
    deadline: watch::Receiver<Option<Instant>>,
    headers: Headers,
    timeout: Duration,
    failures: FailureLog,
}

#[async_trait]
impl HttpClient for ExportClient {
    /// Called on an SDK thread, under `futures_executor::block_on`: the request runs on the I/O
    /// runtime and this awaits its `JoinHandle`, which needs no runtime of its own.
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let request = self.outgoing(request)?;
        let exchange = exchange(
            self.http.clone(),
            request,
            self.signal.retries(),
            self.deadline.clone(),
            self.timeout,
        );
        let outcome = match self.io.handle.spawn(exchange).await {
            Ok(outcome) => outcome,
            Err(error) => Err(ExportError::IoThread(error.to_string())),
        };
        match outcome {
            Ok(response) => {
                self.failures.delivered(self.signal);
                Ok(response)
            }
            Err(error) => {
                let phase = if self.deadline.borrow().is_some() {
                    Phase::ShuttingDown
                } else {
                    Phase::Running
                };
                self.failures.failed(self.signal, &error, phase);
                Ok(absorbed())
            }
        }
    }
}

impl ExportClient {
    /// The exporter's request as it goes on the wire: its URL, body, content type and encoding,
    /// this signal's configured headers, and no other header. The exporter also reads header
    /// variables from the process environment on its own; dropping what it adds keeps what is
    /// sent equal to what the config parsed and checked once.
    fn outgoing(&self, request: Request<Bytes>) -> Result<reqwest::Request, reqwest::Error> {
        let (mut parts, body) = request.into_parts();
        let mut headers = HeaderMap::new();
        if let Some(agent) = parts.headers.get(USER_AGENT) {
            headers.insert(USER_AGENT, agent.clone());
        }
        for (name, value) in self.headers.iter() {
            headers.insert(name, value.clone());
        }
        // Last, so no configured header can change how the collector reads the body.
        for name in [CONTENT_TYPE, CONTENT_ENCODING] {
            match parts.headers.get(&name) {
                Some(value) => headers.insert(name, value.clone()),
                None => headers.remove(name),
            };
        }
        parts.headers = headers;
        reqwest::Request::try_from(Request::from_parts(parts, body))
    }
}

/// What the exporter hears about an export that failed (see the module doc): an empty `204`.
fn absorbed() -> Response<Bytes> {
    let mut response = Response::new(Bytes::new());
    *response.status_mut() = StatusCode::NO_CONTENT;
    response
}

/// One export, from first byte to answer, on the I/O runtime. It ends by
/// `min(start + timeout, deadline)` whatever the collector does, retries included.
async fn exchange(
    http: reqwest::Client,
    request: reqwest::Request,
    retries: u32,
    mut deadline: watch::Receiver<Option<Instant>>,
    timeout: Duration,
) -> Result<Response<Bytes>, ExportError> {
    let start = Instant::now();
    let shutdown = deadline.clone();
    tokio::select! {
        // Polled first, so a request that starts after the deadline never connects.
        biased;
        cut = expiry(&mut deadline, start, timeout) => Err(cut),
        answer = send(&http, request, retries, &shutdown) => answer,
    }
}

/// Resolves when a request started at `start` must end: at `start + timeout`, or at the shutdown
/// deadline if that comes first, including a deadline armed while the request runs.
async fn expiry(
    deadline: &mut watch::Receiver<Option<Instant>>,
    start: Instant,
    timeout: Duration,
) -> ExportError {
    let timed_out_at = start.checked_add(timeout);
    loop {
        let armed = *deadline.borrow_and_update();
        let (until, limit) = match (armed, timed_out_at) {
            (Some(armed), Some(timed_out_at)) if timed_out_at <= armed => {
                (Some(timed_out_at), ExportError::TimedOut(timeout))
            }
            (Some(armed), _) => (Some(armed), ExportError::Deadline),
            (None, timed_out_at) => (timed_out_at, ExportError::TimedOut(timeout)),
        };
        tokio::select! {
            () = sleep_until(until) => return limit,
            changed = deadline.changed() => {
                if changed.is_err() {
                    // The transport is gone, so the deadline can no longer move.
                    sleep_until(until).await;
                    return limit;
                }
            }
        }
    }
}

async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => time::sleep_until(at).await,
        None => future::pending().await,
    }
}

/// Sends `request`, and again up to `retries` times (100 ms, then 400 ms later) when the
/// collector could not be reached or answered 502, 503 or 504. `exchange`'s limit covers the
/// retries too. Once shutdown has armed the deadline nothing is resent: the batches still
/// queued behind this one get the time left, so a dead collector does not eat the budget.
async fn send(
    http: &reqwest::Client,
    mut request: reqwest::Request,
    retries: u32,
    shutdown: &watch::Receiver<Option<Instant>>,
) -> Result<Response<Bytes>, ExportError> {
    let mut attempt = 0;
    loop {
        let again = if attempt < retries {
            request.try_clone()
        } else {
            None
        };
        let answer = send_once(http, request).await;
        let Some(next) = again else {
            return answer;
        };
        let shutting_down = shutdown.borrow().is_some();
        if shutting_down || !answer.as_ref().is_err_and(ExportError::is_transient) {
            return answer;
        }
        time::sleep(Duration::from_millis(100 * 4_u64.pow(attempt))).await;
        request = next;
        attempt += 1;
    }
}

/// One attempt: a `2xx` answer with its body, read up to [`MAX_RESPONSE_BYTES`].
async fn send_once(
    http: &reqwest::Client,
    request: reqwest::Request,
) -> Result<Response<Bytes>, ExportError> {
    let mut response = http
        .execute(request)
        .await
        .map_err(|error| ExportError::Send(causes(&error.without_url())))?;
    let status = response.status();
    if !status.is_success() {
        return Err(ExportError::Status(status));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| ExportError::Read(causes(&error.without_url())))?
    {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(ExportError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    let mut answer = Response::new(Bytes::from(body));
    *answer.status_mut() = status;
    Ok(answer)
}

/// Why an export did not reach the collector, for the transport's warnings. It names no header
/// and no URL.
#[derive(Debug, thiserror::Error)]
enum ExportError {
    #[error("the collector did not answer within {0:?} (OTEL_EXPORTER_OTLP_TIMEOUT)")]
    TimedOut(Duration),
    #[error("cut off by the shutdown deadline")]
    Deadline,
    #[error("could not reach the collector: {0}")]
    Send(String),
    #[error("the collector answered {0}")]
    Status(StatusCode),
    #[error("could not read the collector's answer: {0}")]
    Read(String),
    #[error("the collector's answer is over {MAX_RESPONSE_BYTES} bytes")]
    TooLarge,
    #[error("the telemetry I/O thread dropped the request: {0}")]
    IoThread(String),
}

impl ExportError {
    /// Worth sending again: no answer came, or the collector said it is briefly unavailable.
    fn is_transient(&self) -> bool {
        match self {
            Self::Send(_) => true,
            Self::Status(status) => matches!(status.as_u16(), 502..=504),
            Self::TimedOut(_)
            | Self::Deadline
            | Self::Read(_)
            | Self::TooLarge
            | Self::IoThread(_) => false,
        }
    }
}

/// `error` and its sources on one line: reqwest's own message ("error sending request") says
/// little until its causes ("tcp connect error: Connection refused") follow.
fn causes(error: &dyn Error) -> String {
    let mut line = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let _ = write!(line, ": {cause}");
        source = cause.source();
    }
    line
}

/// One signal's export failures, logged at a bounded rate: at most one line per
/// [`REPORT_EVERY`] whatever the collector does, flapping included, each line counting the batches
/// dropped since the one before. A failure is logged at once when no line came in the last
/// minute; failures that follow fold into the next line; once exports go through again, the next
/// line due says so. Once shutdown has armed the deadline every failure is logged: those batches
/// are the last.
#[derive(Debug, Default)]
struct FailureLog(Mutex<Outage>);

#[derive(Debug, Default)]
struct Outage {
    /// Batches dropped since the last line.
    dropped: u64,
    /// An export failed, and no line has said exports go through again since.
    open: bool,
    /// When the last line was written. Kept when exports recover, so a collector that flaps
    /// still gets one line a minute, not two per flap.
    reported_at: Option<Instant>,
}

/// Where the transport is in its life. Once shutdown starts, every failure is logged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Running,
    ShuttingDown,
}

/// What one export's outcome logs, decided apart from the logging so it can be tested.
#[derive(Debug, PartialEq, Eq)]
enum Report {
    /// Exports fail: `dropped` batches since the last line, this one included.
    Failing { dropped: u64 },
    /// Exports go through again, `dropped` batches after the last line.
    Recovered { dropped: u64 },
    /// Nothing now; the next line counts it.
    Quiet,
}

impl FailureLog {
    fn failed(&self, signal: Signal, error: &ExportError, phase: Phase) {
        if let Report::Failing { dropped } = self.note_failure(Instant::now(), phase) {
            warn!(
                signal = signal.as_str(),
                %error,
                dropped,
                "OTLP push failing, batches dropped; at most one line a minute while it lasts"
            );
        }
    }

    fn delivered(&self, signal: Signal) {
        if let Report::Recovered { dropped } = self.note_delivery(Instant::now()) {
            info!(
                signal = signal.as_str(),
                dropped, "OTLP push goes through again"
            );
        }
    }

    fn note_failure(&self, now: Instant, phase: Phase) -> Report {
        let mut outage = self.lock();
        outage.dropped += 1;
        outage.open = true;
        if phase == Phase::Running && !outage.due(now) {
            return Report::Quiet;
        }
        outage.reported_at = Some(now);
        Report::Failing {
            dropped: mem::take(&mut outage.dropped),
        }
    }

    fn note_delivery(&self, now: Instant) -> Report {
        let mut outage = self.lock();
        if !outage.open || !outage.due(now) {
            return Report::Quiet;
        }
        outage.open = false;
        outage.reported_at = Some(now);
        Report::Recovered {
            dropped: mem::take(&mut outage.dropped),
        }
    }

    /// A poisoned lock is still a consistent `Outage`: every update is a plain field write.
    fn lock(&self) -> MutexGuard<'_, Outage> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Outage {
    /// Whether a line may be written now: none yet, or the last one is [`REPORT_EVERY`] old.
    fn due(&self, now: Instant) -> bool {
        self.reported_at
            .is_none_or(|reported_at| now.duration_since(reported_at) >= REPORT_EVERY)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use http::HeaderValue;

    use super::*;
    use crate::OtlpConfig;

    #[test]
    fn a_failing_signal_logs_at_most_a_line_a_minute_even_when_its_collector_flaps() {
        let log = FailureLog::default();
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        assert_eq!(
            log.note_failure(start, Phase::Running),
            Report::Failing { dropped: 1 },
            "the first failure is logged at once"
        );
        for second in 1..60 {
            let report = if second % 2 == 0 {
                log.note_failure(at(second), Phase::Running)
            } else {
                log.note_delivery(at(second))
            };
            assert_eq!(
                report,
                Report::Quiet,
                "second {second}: a collector flapping every second folds into the next line"
            );
        }
        assert_eq!(
            log.note_failure(at(60), Phase::Running),
            Report::Failing { dropped: 30 },
            "a minute after the last line, one line counts every batch dropped since"
        );
        assert_eq!(log.note_delivery(at(61)), Report::Quiet);
        assert_eq!(
            log.note_delivery(at(120)),
            Report::Recovered { dropped: 0 },
            "the first delivery a line is due for says exports go through again"
        );
        assert_eq!(log.note_delivery(at(200)), Report::Quiet, "said once");
        assert_eq!(
            log.note_failure(at(300), Phase::Running),
            Report::Failing { dropped: 1 },
            "a failure after a quiet minute is logged at once"
        );
        assert_eq!(
            log.note_failure(at(301), Phase::ShuttingDown),
            Report::Failing { dropped: 1 },
            "during shutdown every failure is logged: those batches are the last"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_ends_at_its_timeout_or_at_a_deadline_armed_later()
    -> Result<(), Box<dyn Error>> {
        let (arm, deadline) = watch::channel(None);
        let start = Instant::now();
        let mut timed = deadline.clone();
        assert!(matches!(
            expiry(&mut timed, start, Duration::from_secs(10)).await,
            ExportError::TimedOut(_)
        ));
        assert_eq!(
            Instant::now() - start,
            Duration::from_secs(10),
            "unarmed, a request runs to its own timeout"
        );

        let start = Instant::now();
        let mut cut = deadline.clone();
        let waiting = tokio::spawn(async move {
            let limit = expiry(&mut cut, start, Duration::from_secs(10)).await;
            (limit, Instant::now() - start)
        });
        time::sleep(Duration::from_secs(2)).await;
        arm.send_replace(Some(Instant::now() + Duration::from_secs(1)));
        let (limit, after) = waiting.await?;
        assert!(matches!(limit, ExportError::Deadline));
        assert_eq!(
            after,
            Duration::from_secs(3),
            "a deadline armed while the request runs cuts it at the deadline, not at its timeout"
        );

        let start = Instant::now();
        let mut late = deadline.clone();
        assert!(matches!(
            expiry(&mut late, start, Duration::from_secs(10)).await,
            ExportError::Deadline
        ));
        assert_eq!(
            Instant::now(),
            start,
            "a request that starts after the deadline ends at once"
        );
        Ok(())
    }

    #[test]
    fn the_wire_carries_the_configured_headers_and_the_exporters_encoding_only()
    -> Result<(), Box<dyn Error>> {
        let traces = OtlpConfig::from_env(&|name| match name {
            "OTEL_EXPORTER_OTLP_ENDPOINT" => Some("http://127.0.0.1:9".to_owned()),
            "OTEL_EXPORTER_OTLP_HEADERS" => Some(
                "authorization=Bearer%20token,content-type=text/plain,user-agent=neo".to_owned(),
            ),
            _ => None,
        })?
        .traces
        .ok_or("the endpoint must turn traces on")?;
        let client = Transport::start()?.client(Signal::Traces, &traces)?;
        let request = Request::builder()
            .method("POST")
            .uri("http://127.0.0.1:9/v1/traces")
            .header(CONTENT_TYPE, "application/x-protobuf")
            .header(CONTENT_ENCODING, "gzip")
            .header(USER_AGENT, "OTel-OTLP-Exporter-Rust")
            .header("x-from-process-env", "1")
            .body(Bytes::from_static(b"body"))?;
        let sent = client.outgoing(request)?;
        let headers = sent.headers();
        assert_eq!(
            headers.get("authorization"),
            Some(&HeaderValue::from_static("Bearer token"))
        );
        assert_eq!(
            headers.get(CONTENT_TYPE),
            Some(&HeaderValue::from_static("application/x-protobuf")),
            "a configured header never overrides how the body is encoded"
        );
        assert_eq!(
            headers.get(CONTENT_ENCODING),
            Some(&HeaderValue::from_static("gzip"))
        );
        assert_eq!(
            headers.get(USER_AGENT),
            Some(&HeaderValue::from_static("neo")),
            "a configured user agent replaces the exporter's"
        );
        assert!(
            headers.get("x-from-process-env").is_none(),
            "a header the exporter picked up from the process environment is not sent"
        );
        assert_eq!(sent.url().as_str(), "http://127.0.0.1:9/v1/traces");
        assert_eq!(
            sent.body().and_then(reqwest::Body::as_bytes),
            Some(&b"body"[..])
        );
        Ok(())
    }
}
