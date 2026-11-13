//! The host side as zuzu and the door meet it: the real neo-http behind the real
//! `host_service` auth on a temp Unix socket, over the fake browser and a real trace store.

use std::error::Error;
use std::future::{self, Future};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use browseros_core::{NativeCapabilities, TabId};
use neo_http::{Settings, SettingsDefaults};
use neo_trace::{Effect, OperationId, RedactionLimits, Redactor, RunId, TraceStore, WriterConfig};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::task::JoinHandle;
use tokio::{fs, time};
use tokio_util::sync::CancellationToken;

use super::{HostApi, HostInputs, TakeoverLock};
use crate::browser::{Adopt, BrowserLink, NeoTab, TabRegistry};
use crate::config::HostApiConfig;
use crate::dispatch::{Caller, ErrorCode, MethodSpec, OperationError, Policy, PolicyCheck};
use crate::test_support::{CLOSED_XVNC, FakeCdp};

type TestResult = Result<(), Box<dyn Error>>;

/// Long enough for the neo-http's 32-byte minimum.
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
    /// The settings the bind decided on, as the app hands them to the janitor.
    settings: Settings,
    fake: Arc<FakeCdp>,
    /// Neo's blank tab, as `GET /v1/tabs` marks it. Nothing opens one until a test asks.
    blank_tab: Arc<NeoTab>,
    socket: PathBuf,
    dir: TempDir,
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
        let tabs = Arc::new(TabRegistry::new());
        let link = BrowserLink::start_with(
            NativeCapabilities::default(),
            Arc::clone(&tabs),
            move || future::ready(connect.connect()),
        );
        let store = TraceStore::open(&dir.path().join("trace.db")).await?;
        let blank_tab = NeoTab::new(Arc::clone(&link), tabs, store.clone());
        let redactor = Redactor::new(RedactionLimits::default());
        let (shots, screenshots) = neo_trace::screenshot_queue();
        let sink = neo_trace::spawn_writer(&store, redactor, WriterConfig::default(), Some(shots));
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
            screenshots,
            xvnc: CLOSED_XVNC,
            // The fake browser has no CDP port, and no test here opens a live view to dial it.
            cdp_port: 0,
            settings_file: dir.path().join("settings.json"),
            settings_defaults: SettingsDefaults::default(),
        };
        let host = HostApi::bind(&config, inputs).await?;
        let settings = host.settings().clone();
        let takeover = Arc::new(TakeoverLock::default());
        let shutdown = CancellationToken::new();
        let serving = tokio::spawn(host.serve(
            link,
            Arc::clone(&takeover),
            Arc::default(),
            Arc::default(),
            Some(Arc::clone(&blank_tab)),
            shutdown.clone(),
        ));
        Ok(Self {
            shutdown,
            serving,
            takeover,
            settings,
            fake,
            blank_tab,
            socket: config.socket,
            dir,
        })
    }

    /// Stops serving, as a killed server does, and hands back the folder with its trace and
    /// saved settings for the next start.
    async fn stop(self) -> Result<TempDir, Box<dyn Error>> {
        self.shutdown.cancel();
        within(self.serving).await??;
        Ok(self.dir)
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

/// The browser restarts a killed server on its own, and no host notices: what the host set must
/// come back from the file before anything reads it.
#[tokio::test]
async fn a_setting_survives_a_restart_and_needs_the_host_api() -> TestResult {
    let host = Host::start(Some(TOKEN)).await?;
    host.until_serving().await?;
    let patch = json!({ "tabs": { "idleCloseSeconds": 300 } });
    let set = request(
        &host.socket,
        "PATCH",
        "/v1/settings",
        Some(TOKEN),
        Some(&patch),
    )
    .await?;
    assert_eq!(
        (
            set.status,
            &set.body["settings"]["tabs"]["idleCloseSeconds"]
        ),
        (200, &json!(300)),
        "{set:?}"
    );
    assert_eq!(
        host.settings.tab_idle_close(),
        Duration::from_secs(300),
        "the janitor's settings are the route's, live"
    );
    let dir = host.stop().await?;

    let again = Host::start_on(FakeCdp::new(), dir, Some(TOKEN)).await?;
    assert_eq!(
        again.settings.tab_idle_close(),
        Duration::from_secs(300),
        "loaded at the bind, before the first run or janitor pass"
    );
    again.until_serving().await?;
    let read = request(&again.socket, "GET", "/v1/settings", Some(TOKEN), None).await?;
    assert_eq!(
        read.body["settings"]["tabs"]["idleCloseSeconds"], 300,
        "{read:?}"
    );
    let dir = again.stop().await?;

    let without = Host::start_on(FakeCdp::new(), dir, None).await?;
    assert_eq!(
        without.settings.tab_idle_close(),
        Duration::from_mins(30),
        "with no host API nobody could see or change a saved value, so the defaults apply"
    );
    let dir = without.stop().await?;
    assert!(
        dir.path().join("settings.json").exists(),
        "a start without a host API leaves the host's file alone"
    );
    Ok(())
}

#[tokio::test]
async fn the_host_api_waits_for_the_watched_listing_before_serving() -> TestResult {
    let dir = tempfile::tempdir()?;
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
        host.fake.count("Browser.getTabs") > 0,
        "the host serves only after the watcher provides the first current listing"
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

/// `GET /v1/tabs` marks Neo's blank tab `owner: "neo"`. Once the owner browses in it, it is
/// theirs and unmarked, and the one new blank tab Neo keeps instead carries the mark.
#[tokio::test]
async fn tabs_mark_neos_blank_tab_and_follow_its_hand_back() -> Result<(), Box<dyn Error>> {
    let fake = FakeCdp::new();
    let launch = fake.open_tab("about:blank", "");
    let host = Host::start_on(fake, tempfile::tempdir()?, Some(TOKEN)).await?;
    let owners = |reply: &Reply| -> Vec<(i64, Option<Value>)> {
        let mut owners: Vec<_> = reply.body["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| Some((item["tabId"].as_i64()?, item.get("owner").cloned())))
            .collect();
        owners.sort_by_key(|(tab, _)| *tab);
        owners
    };

    assert_eq!(
        within(host.blank_tab.ensure(Adopt::Allowed)).await?,
        Some(launch),
        "the launch tab serves"
    );
    let listed = request(&host.socket, "GET", "/v1/tabs", Some(TOKEN), None).await?;
    assert_eq!(listed.status, 200, "{}", listed.body);
    assert_eq!(owners(&listed), vec![(launch.0, Some(json!("neo")))]);

    assert!(
        host.fake
            .navigate_tab(launch, "https://example.com/", "Example")
    );
    let fresh = within(host.blank_tab.ensure(Adopt::Never))
        .await?
        .ok_or("Neo's new blank tab")?;
    let listed = request(&host.socket, "GET", "/v1/tabs", Some(TOKEN), None).await?;
    assert_eq!(
        owners(&listed),
        vec![(launch.0, None), (fresh.0, Some(json!("neo")))],
        "one blank tab of Neo's; the owner's tab has no owner field"
    );
    Ok(())
}
