//! The host side as zuzu and the door meet it: the real browseros-service behind the real
//! `host_service` auth on a temp Unix socket, over the fake browser and a real trace store.

use std::collections::HashMap;
use std::error::Error;
use std::future::{self, Future};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use browseros_core::{NativeCapabilities, TabId};
use browseros_trace::{
    Effect, EpochMillis, OperationId, Privacy, PrivateReason, RedactionLimits, Redactor, RunId,
    SecretSet, SecretSetConfig, TraceStore, WriterConfig,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::task::JoinHandle;
use tokio::{fs, time};
use tokio_util::sync::CancellationToken;

use super::{FlagRecovery, HostApi, HostInputs, TakeoverLock};
use crate::browser::{BrowserLink, TabRegistry};
use crate::config::HostApiConfig;
use crate::dispatch::{Caller, ErrorCode, MethodSpec, OperationError, Policy, PolicyCheck};
use crate::test_support::{CLOSED_XVNC, FakeCdp};

type TestResult = Result<(), Box<dyn Error>>;

/// Long enough for the browseros-service's 32-byte minimum.
const TOKEN: &str = "host-test-token-0123456789-abcdefghij";

/// One reply off the socket. A plain data bag.
#[derive(Debug)]
struct Reply {
    status: u16,
    body: Value,
}

/// A host API started the way app.rs starts it, over the fake browser. Fields drop in order:
/// the serving task is stopped before the directory holding its socket goes.
struct Host {
    shutdown: CancellationToken,
    serving: JoinHandle<()>,
    takeover: Arc<TakeoverLock>,
    fake: Arc<FakeCdp>,
    store: TraceStore,
    socket: PathBuf,
    _dir: TempDir,
}

impl Host {
    /// Binds with `token_file` (none when `None`) and starts serving, over a fresh browser and
    /// trace store.
    async fn start(token: Option<&str>) -> Result<Self, Box<dyn Error>> {
        Self::start_on(FakeCdp::new(), tempfile::tempdir()?, token).await
    }

    /// Like [`Host::start`], over `fake` as the test set it up and the trace store in `dir`
    /// (`trace.db`, created when absent). The fake links in the background, as Chromium does.
    async fn start_on(
        fake: Arc<FakeCdp>,
        dir: TempDir,
        token: Option<&str>,
    ) -> Result<Self, Box<dyn Error>> {
        let connect = Arc::clone(&fake);
        let link = BrowserLink::start_with(
            NativeCapabilities::default(),
            Arc::new(TabRegistry::new()),
            move || future::ready(connect.connect()),
        );
        let store = TraceStore::open(&dir.path().join("trace.db")).await?;
        let secrets = Arc::new(SecretSet::new(SecretSetConfig::default()));
        let redactor = Redactor::new(Arc::clone(&secrets), RedactionLimits::default());
        let (shots, screenshots) = browseros_trace::screenshot_queue();
        let sink =
            browseros_trace::spawn_writer(&store, redactor, WriterConfig::default(), Some(shots));
        let privacy = Privacy::new(secrets, Arc::new(sink.clone()), store.private_tabs().await?);
        let token_file = match token {
            Some(token) => {
                let path = dir.path().join("token");
                fs::write(&path, format!("{token}\n")).await?;
                Some(path)
            }
            None => None,
        };
        let config = HostApiConfig {
            socket: dir.path().join("host.sock"),
            token_file,
            tcp: None,
        };
        let inputs = HostInputs {
            store: store.clone(),
            sink,
            privacy,
            screenshots,
            xvnc: CLOSED_XVNC,
            // The fake browser has no CDP port, and no test here opens a live view to dial it.
            cdp_port: 0,
            recovery: FlagRecovery::none(),
        };
        let host = HostApi::bind(&config, inputs).await?;
        let takeover = Arc::new(TakeoverLock::default());
        let shutdown = CancellationToken::new();
        let serving = tokio::spawn(host.serve(link, Arc::clone(&takeover), shutdown.clone()));
        Ok(Self {
            shutdown,
            serving,
            takeover,
            fake,
            store,
            socket: config.socket,
            _dir: dir,
        })
    }

    /// `POST /v1/locks` on `tab` for a minute, as a host takes the tab for its owner.
    async fn take_lock(&self, tab: TabId) -> Result<Reply, Box<dyn Error>> {
        let take = json!({ "tabId": tab.0, "ttlSeconds": 60 });
        request(&self.socket, "POST", "/v1/locks", Some(TOKEN), Some(&take)).await
    }

    /// Waits until the fake answered `method` more than `before` times.
    async fn until_called(&self, method: &str, before: usize) -> TestResult {
        within(async {
            while self.fake.count(method) <= before {
                time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
    }

    /// Waits until the service answers on the socket: it starts once the browser linked.
    async fn until_serving(&self) -> TestResult {
        within(async {
            loop {
                let served = request(&self.socket, "GET", "/v1/runs", Some(TOKEN), None).await;
                if served.is_ok_and(|reply| reply.status == 200) {
                    return;
                }
                time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
    }

    /// Whether the door would let the agent's `effect` on `tab` through now.
    fn agent_may(&self, tab: TabId, effect: Effect) -> Result<(), OperationError> {
        let method = MethodSpec::new("test.op", effect);
        self.takeover.check(&PolicyCheck {
            method: &method,
            caller: Caller::Script,
            tab: Some(tab),
            operation: OperationId::new(RunId::new(), 1),
        })
    }
}

/// One HTTP/1.1 request on the Unix socket, read to the end (`Connection: close`).
async fn request(
    socket: &Path,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&Value>,
) -> Result<Reply, Box<dyn Error>> {
    let mut stream = UnixStream::connect(socket).await?;
    let body = body.map(Value::to_string).unwrap_or_default();
    let auth = token.map_or_else(String::new, |token| {
        format!("Authorization: Bearer {token}\r\n")
    });
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| format!("no HTTP head in {response:?}"))?;
    let status: u16 = head.split(' ').nth(1).ok_or("a status line")?.parse()?;
    let body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(body)?
    };
    Ok(Reply { status, body })
}

/// `work`, or an error after 10 s: a hang fails the test instead of stalling it.
async fn within<T>(work: impl Future<Output = T>) -> Result<T, Box<dyn Error>> {
    let limit = Duration::from_secs(10);
    time::timeout(limit, work)
        .await
        .map_err(|error| format!("not within {limit:?}: {error}").into())
}

/// Commits `tab`'s private flag into the trace store in `dir`, as an earlier Neo did before it
/// stopped.
async fn seed_private(dir: &TempDir, tab: TabId) -> TestResult {
    let store = TraceStore::open(&dir.path().join("trace.db")).await?;
    let secrets = Arc::new(SecretSet::new(SecretSetConfig::default()));
    let redactor = Redactor::new(Arc::clone(&secrets), RedactionLimits::default());
    let sink = browseros_trace::spawn_writer(&store, redactor, WriterConfig::default(), None);
    let privacy = Privacy::new(secrets, Arc::new(sink.clone()), HashMap::new());
    privacy
        .make_private(tab, PrivateReason::Host, EpochMillis::now())
        .await?;
    sink.flush().await?;
    Ok(())
}

#[tokio::test]
async fn the_host_api_serves_behind_the_token_and_its_locks_reach_the_door() -> TestResult {
    let host = Host::start(Some(TOKEN)).await?;
    let tab = host.fake.open_tab("https://example.com/", "Owner");
    host.until_serving().await?;

    let refused = request(&host.socket, "GET", "/v1/runs", None, None).await?;
    assert_eq!(
        (refused.status, refused.body["code"].as_str()),
        (401, Some("unauthorized")),
        "no bearer token, no host API: {refused:?}"
    );
    let wrong = request(
        &host.socket,
        "GET",
        "/v1/runs",
        Some("not-the-token-0123456789-abcdefghij"),
        None,
    )
    .await?;
    assert_eq!(wrong.status, 401, "only the configured token: {wrong:?}");
    let runs = request(&host.socket, "GET", "/v1/runs", Some(TOKEN), None).await?;
    assert_eq!(
        runs.status, 200,
        "the real router, behind the auth: {runs:?}"
    );

    let lock = host.take_lock(tab).await?;
    assert_eq!(lock.status, 200, "{lock:?}");
    let refusal = host
        .agent_may(tab, Effect::Write)
        .err()
        .ok_or("a host's lock refuses the agent's write on its tab")?;
    assert_eq!(refusal.code(), ErrorCode::OWNER_HAS_CONTROL);
    assert!(
        host.agent_may(tab, Effect::Read).is_err(),
        "reads block by default: the owner may be typing a password"
    );
    assert!(
        host.agent_may(TabId(tab.0 + 1), Effect::Write).is_ok(),
        "another tab is not the owner's"
    );

    assert!(host.fake.close_tab(tab), "the owner closes the tab");
    within(async {
        while host.takeover.is_locked(tab) {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|error| format!("the tab feed ends a closed tab's lock: {error}"))?;
    let id = lock.body["id"].as_str().ok_or("a lock has an id")?;
    let ended = request(
        &host.socket,
        "GET",
        &format!("/v1/locks/{id}"),
        Some(TOKEN),
        None,
    )
    .await?;
    assert!(
        ended.body["releasedAt"].is_i64(),
        "the host reads the lock as ended: {ended:?}"
    );

    host.shutdown.cancel();
    within(host.serving).await??;
    assert!(
        !host.socket.exists(),
        "a clean stop removes the socket file"
    );
    Ok(())
}

#[tokio::test]
async fn without_a_usable_token_there_is_no_host_socket() -> TestResult {
    for token in [None, Some("too-short")] {
        let host = Host::start(token).await?;
        assert!(
            !host.socket.exists(),
            "{token:?}: never a host API behind a missing or weak token"
        );
        host.shutdown.cancel();
        within(host.serving).await??;
    }
    Ok(())
}

#[tokio::test]
async fn the_host_api_serves_only_once_closed_tabs_lost_their_state() -> TestResult {
    let dir = tempfile::tempdir()?;
    let gone = TabId(999);
    seed_private(&dir, gone).await?;
    let fake = FakeCdp::new();
    fake.open_tab("https://example.com/", "Owner");
    // No listing until target discovery answers, so the tabs cannot be reconciled yet.
    fake.hold_next("Target.setDiscoverTargets");
    let host = Host::start_on(fake, dir, Some(TOKEN)).await?;
    host.until_called("Target.setDiscoverTargets", 0).await?;

    let early = time::timeout(
        Duration::from_millis(300),
        request(&host.socket, "GET", "/v1/runs", Some(TOKEN), None),
    )
    .await;
    assert!(
        early.is_err(),
        "no request is answered before the tab state is reconciled: {early:?}"
    );

    host.fake.release_held();
    host.until_serving().await?;
    assert!(
        !host.store.private_tabs().await?.contains_key(&gone),
        "a private flag an earlier Neo left on a tab that is gone ended before the first request"
    );
    host.shutdown.cancel();
    within(host.serving).await??;
    Ok(())
}

#[tokio::test]
async fn a_reconnect_never_ends_a_lock_on_a_tab_newer_than_the_listing() -> TestResult {
    let fake = FakeCdp::new();
    fake.open_tab("https://example.com/", "Owner");
    let unwatched = fake.open_tab("https://example.com/gone", "Closed while unwatched");
    let host = Host::start_on(fake, tempfile::tempdir()?, Some(TOKEN)).await?;
    host.until_serving().await?;
    host.until_called("Browser.getTabs", 0).await?;
    let lock = host.take_lock(unwatched).await?;
    assert_eq!(lock.status, 200, "{lock:?}");

    // The listing on the new socket answers late, as it stood before the next tab opened, and
    // that tab's own target event is late too, so nothing lists the tabs again meanwhile.
    host.fake.hold_events();
    host.fake.hold_next("Browser.getTabs");
    let listed = host.fake.count("Browser.getTabs");
    host.fake.drop_connection();
    assert!(
        host.fake.close_tab(unwatched),
        "closed while the socket is down, so only a reconciliation can end its lock"
    );
    host.fake.reconnect();
    host.until_called("Browser.getTabs", listed).await?;
    let newer = host.fake.open_tab("https://example.org/", "Newer");
    let lock = host.take_lock(newer).await?;
    assert_eq!(lock.status, 200, "{lock:?}");

    host.fake.release_held();
    within(async {
        while host.takeover.is_locked(unwatched) {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|error| format!("the reconnect is reconciled: {error}"))?;
    assert!(
        host.takeover.is_locked(newer),
        "the same reconciliation keeps the lock on a tab that opened after its listing"
    );

    host.shutdown.cancel();
    within(host.serving).await??;
    Ok(())
}

#[tokio::test]
async fn a_lock_on_a_tab_no_listing_had_ends_when_the_tab_closes() -> TestResult {
    let fake = FakeCdp::new();
    fake.open_tab("https://example.com/", "Owner");
    let host = Host::start_on(fake, tempfile::tempdir()?, Some(TOKEN)).await?;
    host.until_serving().await?;

    // The tab opens and closes before Neo hears of it, and both target events then arrive back
    // to back: any listing taken for them already lacks the tab, so no two listings differ by it.
    host.fake.hold_events();
    let brief = host.fake.open_tab("https://example.org/", "Brief");
    let lock = host.take_lock(brief).await?;
    assert_eq!(
        lock.status, 200,
        "the host saw the tab open and locked it: {lock:?}"
    );
    assert!(host.fake.close_tab(brief), "the owner closes the tab");
    host.fake.release_events();

    within(async {
        while host.takeover.is_locked(brief) {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|error| format!("the close ends the lock, though no listing had the tab: {error}"))?;
    let id = lock.body["id"].as_str().ok_or("a lock has an id")?;
    let ended = request(
        &host.socket,
        "GET",
        &format!("/v1/locks/{id}"),
        Some(TOKEN),
        None,
    )
    .await?;
    assert!(
        ended.body["releasedAt"].is_i64(),
        "the host reads the lock as ended: {ended:?}"
    );
    host.shutdown.cancel();
    within(host.serving).await??;
    Ok(())
}
