//! Isolated-world facts no op can observe, against a real browser: where a page's seqs start,
//! that concurrent first uses share one install, that a navigation retires a seq, that the
//! probe reads a field as the page has it, that a strict CSP does not stop the install, that a second
//! `Playwright` re-keys a world, that `page.setContent` keeps the seq while an old ref fails,
//! that with `Runtime` off the install takes every kind of frame's world by number, and that
//! while a `Runtime` lease has the session report its contexts the install takes a new world by
//! its unique id and fails `Gone` when the document changes renderer inside its window.
//!
//! Needs BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005):
//! `cargo test -p neo-playwright --lib world::live_tests -- --ignored`. Each test opens
//! and closes its own tabs and serves its pages on 127.0.0.1 and localhost.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{CdpConnection, FrameId, ProtocolSession, SessionId};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};
use tokio::time;

use crate::op::Op;
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{
    self, PROBE_PROTOCOL, ParsedSelector, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget,
    Read, Want,
};
use crate::target::{CallHeader, Strictness};
use crate::test_support::fixture_server::{FixtureServer, Page};
use crate::test_support::{LiveBrowser, Tab, TestResult, count_op};
use crate::world::isolated_world::ENGINE_WORLD;
use crate::world::{
    ContextWatch, Endpoint, FrameSeq, IsolatedWorld, ParentLink, Pin, Place, SeqLookup,
};

const PASSWORD_PAGE: &str = r#"<!doctype html><title>pw</title>
<label>Password <input id="pw" type="password" value="hunter2-attribute"></label>
<label>Name <input id="name" value="Ada"></label>
<iframe id="frame" src="/child.html" width="300" height="80"></iframe>"#;

/// A parsed CSS chunk, as the vendored parser emits it.
fn css(selector: &str) -> TestResult<ParsedSelector> {
    let parsed = json!({ "parts": [{
        "name": "css",
        "body": [{ "simples": [{ "selector": { "css": selector, "functions": [] }, "combinator": "" }] }],
        "source": selector,
    }] });
    Ok(serde_json::from_value(parsed)?)
}

fn resolve(target: ParsedSelector, want: Want) -> ProbeRequest {
    ProbeRequest::Resolve {
        target: ProbeTarget::Chunk(target),
        strictness: Strictness::Strict,
        want,
    }
}

/// `resolve { want: element }`, as an op sends it.
async fn resolve_element(
    world: &IsolatedWorld,
    target: ParsedSelector,
) -> TestResult<Result<ProbeReply, ProbeFailure>> {
    Ok(probe::call_for_tests(world, &resolve(target, Want::Element)).await)
}

async fn fixtures() -> TestResult<FixtureServer> {
    let pages = HashMap::from([
        ("/password.html".to_owned(), Page::html(PASSWORD_PAGE)),
        (
            "/child.html".to_owned(),
            Page::html("<!doctype html><title>child</title><p>child frame</p>"),
        ),
        (
            "/csp.html".to_owned(),
            Page::html(&format!(
                "{PASSWORD_PAGE}<script src=\"/page.js\"></script><script>window.inlineRan = true</script>"
            ))
            .with_header("content-security-policy", "script-src 'self'"),
        ),
        (
            "/page.js".to_owned(),
            Page::script("window.pageScriptRan = true"),
        ),
        // A same-process child and, on the other site, an OOPIF.
        (
            "/frames.html".to_owned(),
            Page::html(
                "<!doctype html><title>frames</title><iframe id=\"same\" src=\"/child.html\"></iframe>\
                 <iframe id=\"oop\"></iframe><script>document.getElementById('oop').src = \
                 'http://localhost:' + location.port + '/child.html'</script>",
            ),
        ),
        // Many child frames, so small context ids name a page-owned main world here.
        (
            "/kids.html".to_owned(),
            Page::html(&format!(
                "<!doctype html><title>kids</title>{}",
                "<iframe src=\"/kid.html\"></iframe>".repeat(8)
            )),
        ),
        (
            "/kid.html".to_owned(),
            Page::html("<!doctype html><script>window.pageOwned = true</script>"),
        ),
    ]);
    Ok(FixtureServer::start(pages).await?)
}

/// The live connection, recording what goes through it and, once armed, moving the tab to
/// another site right before it sends a given command: the window between the world's create
/// and the evaluate that takes its handle.
struct Recorder {
    inner: Arc<dyn CdpConnection>,
    sent: Mutex<Vec<(String, Value)>>,
    swap: Mutex<Option<(&'static str, String)>>,
}

impl Recorder {
    fn over(live: &LiveBrowser) -> Arc<Self> {
        Arc::new(Self {
            inner: live.browser.connection(),
            sent: Mutex::default(),
            swap: Mutex::default(),
        })
    }

    /// Before the next `command`, navigate the session's tab to `url` and let it load.
    fn swap_before(&self, command: &'static str, url: String) {
        *lock(&self.swap) = Some((command, url));
    }

    /// The params of every `method` sent so far, and forgets them.
    fn take(&self, method: &str) -> Vec<Value> {
        let mut sent = lock(&self.sent);
        let (taken, kept) = sent.drain(..).partition(|(name, _)| name == method);
        *sent = kept;
        taken.into_iter().map(|(_, params)| params).collect()
    }

    fn session(self: &Arc<Self>, of: &ProtocolSession) -> TestResult<ProtocolSession> {
        let id = of.session_id().ok_or("a target session")?.clone();
        Ok(ProtocolSession::for_session(
            Arc::clone(self) as Arc<dyn CdpConnection>,
            id,
        ))
    }

    fn watch(&self) -> ContextWatch {
        ContextWatch::new(self.inner.events())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl CdpConnection for Recorder {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            let armed = {
                let mut swap = lock(&self.swap);
                match swap.take() {
                    Some((command, url)) if command == method => Some(url),
                    other => {
                        *swap = other;
                        None
                    }
                }
            };
            if let Some(url) = armed {
                self.inner
                    .send("Page.navigate", json!({ "url": url }), session)
                    .await?;
                time::sleep(Duration::from_millis(1500)).await;
            }
            lock(&self.sent).push((method.to_owned(), params.clone()));
            self.inner.send(method, params, session).await
        })
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        self.inner.send_raw_json(method, params_json, session)
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.inner.events()
    }

    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        self.inner.events_targeted(method)
    }

    fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }

    fn connection_epoch(&self) -> u64 {
        self.inner.connection_epoch()
    }
}

/// Whether the session reports its contexts while an install runs: not at all, as Neo leaves
/// every session, or under a `Runtime` lease, as while a run listens for console messages.
#[derive(Clone, Copy, Debug)]
enum Reporting {
    Off,
    Leased,
}

/// One install as `FrameWorlds` runs it (the watch, then the install), over `recorder`, with
/// the session reporting as `reporting` says.
async fn install_over(
    live: &LiveBrowser,
    recorder: &Arc<Recorder>,
    at: (&ProtocolSession, &str),
    seq: u32,
    reporting: Reporting,
) -> TestResult<Result<IsolatedWorld, ProbeFailure>> {
    let (session, frame) = at;
    let session = recorder.session(session)?;
    let _lease = match reporting {
        Reporting::Off => None,
        Reporting::Leased => Some(
            live.browser
                .page_signals
                .runtime_leases()
                .acquire(&session, 0.0)
                .await?,
        ),
    };
    Ok(IsolatedWorld::install(
        session,
        FrameId(frame.to_owned()),
        FrameSeq(seq),
        live.playwright.epoch(),
        recorder.watch(),
    )
    .await
    .map(|installed| installed.world))
}

/// The URL the world's own `location` reads, through its handle.
async fn world_url(world: &IsolatedWorld) -> TestResult<Value> {
    let read: Value = world
        .session()
        .send(
            "Runtime.callFunctionOn",
            json!({ "objectId": world.global(), "functionDeclaration": "function () { return [location.href, window.pageOwned === true] }", "returnByValue": true }),
        )
        .await?;
    Ok(read["result"]["value"].clone())
}

/// The frame of the same-process `<iframe>` the probe pinned in `world`.
async fn same_process_frame(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    pin: Pin,
) -> TestResult<FrameId> {
    let iframe = probe::pinned_object(scope, page, world, pin)
        .await?
        .ok_or("the pinned iframe comes back by handle")?;
    let described: Value = world
        .session()
        .send(
            "DOM.describeNode",
            json!({ "objectId": iframe, "depth": 1 }),
        )
        .await?;
    assert!(
        described["node"]["contentDocument"].is_object(),
        "a same-process iframe carries its document: {described}"
    );
    let frame = described["node"]["frameId"]
        .as_str()
        .ok_or("a same-process iframe names its frame")?;
    Ok(FrameId(frame.to_owned()))
}

/// Boots `world`'s probe as another `Playwright` would: under `epoch`, numbering it `seq`.
async fn boot_as(world: &IsolatedWorld, seq: FrameSeq, epoch: &str) -> TestResult {
    world
        .session()
        .send::<_, Value>(
            "Runtime.callFunctionOn",
            json!({
                "objectId": world.global(),
                "functionDeclaration": "function (options) { return globalThis.__neoProbe.boot(options) }",
                "arguments": [{ "value": { "frameSeq": seq, "epoch": epoch } }],
                "returnByValue": true,
            }),
        )
        .await?;
    Ok(())
}

/// Loads a new document into `#frame` (`/child.html` with `search`) and waits for it.
async fn navigate_frame(tab: &Tab, search: &str) -> TestResult {
    tab.main_world(&format!(
        "document.getElementById('frame').src = '/child.html{search}'"
    ))
    .await?;
    let loaded = format!(
        "document.getElementById('frame').contentWindow.location.search === '{search}' && \
         document.getElementById('frame').contentDocument.readyState === 'complete'"
    );
    for _ in 0..100 {
        if tab.main_world(&loaded).await? == json!(true) {
            return Ok(());
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    Err(format!("#frame did not load /child.html{search} within 5 s").into())
}

/// The main frame's world, as an op reaches it.
async fn main_world(
    live: &LiveBrowser,
    scope: &OpScope<'_>,
    tab: &Tab,
) -> TestResult<IsolatedWorld> {
    let page = OpPage::admit(&live.playwright, scope, tab.id).await?;
    Ok(page.world(&page.main_endpoint(), Place::Main).await?)
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_new_page_starts_at_seq_0_and_concurrent_first_uses_share_one_install()
-> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = fixtures().await?;
    let tab = live
        .open(&server.url("127.0.0.1", "/password.html"))
        .await?;
    let env = live.env();
    let scope = OpScope::new(&env, count_op()?, CallHeader::default());
    let page = OpPage::admit(&live.playwright, &scope, tab.id).await?;
    let endpoint = page.main_endpoint();
    let (first, second) = tokio::join!(
        page.world(&endpoint, Place::Main),
        page.world(&endpoint, Place::Main)
    );
    let (first, second) = (first?, second?);
    assert_eq!(
        first.seq(),
        FrameSeq(0),
        "a tab first seen after the instance started prints bare refs"
    );
    assert!(
        first.same_install(&second),
        "both first uses got the one install's handle"
    );
    let described = probe::call_for_tests(&first, &ProbeRequest::Describe).await;
    assert!(
        matches!(
            described,
            Ok(ProbeReply::Described {
                seq: FrameSeq(0),
                ..
            })
        ),
        "{described:?}"
    );
    live.close(&tab).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_navigation_retires_the_seq_even_across_processes() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = fixtures().await?;
    let tab = live
        .open(&server.url("127.0.0.1", "/password.html"))
        .await?;
    let env = live.env();
    let scope = OpScope::new(&env, count_op()?, CallHeader::default());
    let first = main_world(&live, &scope, &tab).await?;

    // Another site: the document moves to another renderer, where context ids start over.
    tab.navigate(&server.url("localhost", "/password.html"))
        .await?;
    let stale = probe::call_for_tests(&first, &ProbeRequest::Describe).await;
    assert!(
        matches!(stale, Err(ProbeFailure::Gone(_))),
        "the old handle must fail, never answer from another context: {stale:?}"
    );
    let page = OpPage::admit(&live.playwright, &scope, tab.id).await?;
    page.frames().invalidate(&first);
    let second = page.world(&page.main_endpoint(), Place::Main).await?;
    // Which seq is not fixed: every frame that attached before the commit spent one first, as
    // upstream numbers them (the browser sometimes adds short-lived about:blank frames), and
    // the watcher may log the commit only after this install. What is fixed is that the old
    // page's seq is never handed out again.
    assert_eq!(first.seq(), FrameSeq(0));
    assert!(
        second.seq() > first.seq(),
        "the new document takes a seq after the old page's: {:?}",
        second.seq()
    );
    assert!(matches!(
        page.frames().by_seq(first.seq()),
        SeqLookup::Retired
    ));
    assert!(matches!(
        page.frames().by_seq(second.seq()),
        SeqLookup::Live { .. }
    ));
    // Child frames reserve seqs when they attach, even before their worlds install, so the
    // unknown case uses a seq no page reaches.
    assert!(matches!(
        page.frames().by_seq(FrameSeq(u32::MAX)),
        SeqLookup::Unknown
    ));

    // The same site again: the context is destroyed in the same renderer.
    tab.navigate(&server.url("localhost", "/password.html?again"))
        .await?;
    let stale = probe::call_for_tests(&second, &ProbeRequest::Describe).await;
    assert!(matches!(stale, Err(ProbeFailure::Gone(_))), "{stale:?}");
    live.close(&tab).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_the_probe_reads_a_password_as_the_page_has_it() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = fixtures().await?;
    let tab = live
        .open(&server.url("127.0.0.1", "/password.html"))
        .await?;
    tab.main_world("document.getElementById('pw').value = 'typed-secret'")
        .await?;
    let env = live.env();
    let scope = OpScope::new(&env, count_op()?, CallHeader::default());
    let world = main_world(&live, &scope, &tab).await?;

    let resolved = resolve_element(&world, css("#pw")?).await?;
    let Ok(ProbeReply::Element { preview, .. }) = resolved else {
        return Err(format!("the password field did not resolve: {resolved:?}").into());
    };
    assert!(
        preview.contains("value=\"hunter2-attribute\""),
        "the preview is upstream's, of the page's markup: {preview}"
    );
    let raw = tab
        .main_world("document.getElementById('pw').value")
        .await?;
    assert_eq!(
        raw,
        json!("typed-secret"),
        "the page's own world is untouched"
    );
    live.close(&tab).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_strict_csp_page_still_installs() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = fixtures().await?;
    let tab = live.open(&server.url("127.0.0.1", "/csp.html")).await?;
    let enforced = tab
        .main_world("[window.pageScriptRan === true, window.inlineRan === true]")
        .await?;
    assert_eq!(
        enforced,
        json!([true, false]),
        "the page's CSP is in force: its own script ran, its inline script did not"
    );
    let env = live.env();
    let scope = OpScope::new(&env, count_op()?, CallHeader::default());
    let world = main_world(&live, &scope, &tab).await?;
    let resolved = resolve_element(&world, css("#name")?).await?;
    assert!(
        matches!(resolved, Ok(ProbeReply::Element { .. })),
        "{resolved:?}"
    );
    live.close(&tab).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_second_playwright_rekeys_the_world_so_old_refs_fail() -> TestResult {
    let first = LiveBrowser::connect().await?;
    let server = fixtures().await?;
    let tab = first
        .open(&server.url("127.0.0.1", "/password.html"))
        .await?;
    let env = first.env();
    let scope = OpScope::new(&env, count_op()?, CallHeader::default());
    let world = main_world(&first, &scope, &tab).await?;
    // A frame hop's key stands in for a snapshot ref: the probe forgets both on a re-key.
    let frame = probe::call_for_tests(&world, &resolve(css("#frame")?, Want::Frame)).await;
    let Ok(ProbeReply::Frame { key, pin, .. }) = frame else {
        return Err(format!("the iframe did not resolve: {frame:?}").into());
    };
    let by_key = resolve_element(&world, ParsedSelector::frame_key(&key)?).await?;
    assert!(
        matches!(by_key, Ok(ProbeReply::Element { .. })),
        "{by_key:?}"
    );
    let first_page = OpPage::admit(&first.playwright, &scope, tab.id).await?;
    let child_frame = same_process_frame(&scope, &first_page, &world, pin).await?;
    let child = first_page
        .world(
            &Endpoint {
                session: world.session().clone(),
                frame: child_frame.clone(),
            },
            Place::Child(ParentLink {
                parent: world.seq(),
                iframe_key: key.clone(),
            }),
        )
        .await?;

    // A restarted claw-sdk: another connection, another instance, the same tab and world.
    let second = LiveBrowser::connect().await?;
    let reopened = second.tab(&tab.target).await?;
    let other_env = second.env();
    let other_scope = OpScope::new(&other_env, count_op()?, CallHeader::default());
    let rekeyed = main_world(&second, &other_scope, &reopened).await?;
    assert!(
        (100_000..1_000_000).contains(&rekeyed.seq().0),
        "a tab open before the instance started takes a seq from the random base: {:?}",
        rekeyed.seq()
    );
    let other_page = OpPage::admit(&second.playwright, &other_scope, reopened.id).await?;
    assert!(
        matches!(other_page.frames().by_seq(world.seq()), SeqLookup::Retired),
        "the re-key reported the seq the first instance printed refs under, so it is retired"
    );

    // An instance before the first whose numbering overlapped the second's: it gave the child
    // document the seq the second would hand out after its next one.
    let overlapping = FrameSeq(rekeyed.seq().0 + 2);
    boot_as(&child, overlapping, "an older claw-sdk").await?;
    let child_place = Place::Child(ParentLink {
        parent: rekeyed.seq(),
        iframe_key: key.clone(),
    });
    let child_endpoint = Endpoint {
        session: rekeyed.session().clone(),
        frame: child_frame,
    };
    let other_child = other_page
        .world(&child_endpoint, child_place.clone())
        .await?;
    assert_eq!(other_child.seq().0, rekeyed.seq().0 + 1);
    assert!(matches!(
        other_page.frames().by_seq(overlapping),
        SeqLookup::Retired
    ));
    navigate_frame(&tab, "?next").await?;
    let stale = probe::call_for_tests(&other_child, &ProbeRequest::Describe).await;
    assert!(matches!(stale, Err(ProbeFailure::Gone(_))), "{stale:?}");
    other_page.frames().invalidate(&other_child);
    let next = other_page.world(&child_endpoint, child_place).await?;
    assert_eq!(
        next.seq().0,
        overlapping.0 + 1,
        "the child's next document skips the seq the older instance gave its first"
    );
    let old_ref = probe::call_for_tests(
        &world,
        &resolve(ParsedSelector::frame_key(&key)?, Want::Element),
    )
    .await;
    assert!(
        matches!(&old_ref, Ok(ProbeReply::None { stale_ref: Some(stale) }) if *stale == key),
        "the first instance's ref is stale at once, which the resolver reports as \
         element_detached: {old_ref:?}"
    );
    let described = probe::call_for_tests(&world, &ProbeRequest::Describe).await;
    assert!(
        matches!(described, Ok(ProbeReply::Described { seq, .. }) if seq == rekeyed.seq()),
        "the world answers with the new instance's numbering"
    );
    first.close(&tab).await;
    Ok(())
}

/// A world an earlier claw-sdk sealed keeps core's seal, and its replaced `value` getter, for
/// as long as its document lives; the engine installs in a world of its own name, so it reads a
/// field as the page holds it on such a page too.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_legacy_sdk_world_is_never_reused_by_the_engine() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = fixtures().await?;
    let tab = live
        .open(&server.url("127.0.0.1", "/password.html"))
        .await?;
    tab.main_world("document.getElementById('pw').value = 'typed-secret'")
        .await?;
    // What an earlier build left in the document: the seal's getter in its `browseros-sdk` world.
    let created: Value = tab
        .session
        .send(
            "Page.createIsolatedWorld",
            json!({ "frameId": tab.target.as_str(), "worldName": "browseros-sdk" }),
        )
        .await?;
    let legacy_getter = "Object.defineProperty(HTMLInputElement.prototype, 'value', \
                  { configurable: true, get() { return '••••' } }); 1";
    tab.session
        .send::<_, Value>(
            "Runtime.evaluate",
            json!({ "expression": legacy_getter, "contextId": created["executionContextId"] }),
        )
        .await?;

    let env = live.env();
    let scope = OpScope::new(&env, count_op()?, CallHeader::default());
    let world = main_world(&live, &scope, &tab).await?;
    let read = ProbeRequest::Read {
        target: ProbeTarget::Chunk(css("#pw")?),
        strictness: Strictness::Strict,
        frame_visible: true,
        read: Read::InputValue,
    };
    let Ok(ProbeReply::Read { value, .. }) = probe::call_for_tests(&world, &read).await else {
        return Err("the password field did not read".into());
    };
    assert_eq!(
        value.into_value(),
        json!("typed-secret"),
        "the engine's world is not the one the earlier build sealed"
    );
    live.close(&tab).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_an_older_protocols_probe_is_rekeyed_then_replaced() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = fixtures().await?;
    let tab = live
        .open(&server.url("127.0.0.1", "/password.html"))
        .await?;
    // What an older build would leave in the world: a probe that speaks protocol 0 and
    // records the epoch it was last booted with.
    let created: Value = tab
        .session
        .send(
            "Page.createIsolatedWorld",
            json!({ "frameId": tab.target.as_str(), "worldName": ENGINE_WORLD }),
        )
        .await?;
    let context = created["executionContextId"].clone();
    let old_probe = "globalThis.__neoProbe = { boot(options) { globalThis.__oldBootEpoch = \
                     options.epoch; return { kind: 'described', protocol: 0, seq: 7, url: '' } } }; 1";
    tab.session
        .send::<_, Value>(
            "Runtime.evaluate",
            json!({ "expression": old_probe, "contextId": context }),
        )
        .await?;

    let env = live.env();
    let scope = OpScope::new(&env, count_op()?, CallHeader::default());
    let world = main_world(&live, &scope, &tab).await?;
    assert_eq!(
        world.seq(),
        FrameSeq(0),
        "the new probe takes this instance's seq"
    );
    let described = probe::call_for_tests(&world, &ProbeRequest::Describe).await?;
    assert!(
        matches!(
            described,
            ProbeReply::Described {
                protocol: PROBE_PROTOCOL,
                seq: FrameSeq(0),
                ..
            }
        ),
        "this build's probe answers now: {described:?}"
    );
    let old_boot: Value = tab
        .session
        .send(
            "Runtime.evaluate",
            json!({ "expression": "globalThis.__oldBootEpoch", "contextId": context, "returnByValue": true }),
        )
        .await?;
    assert_eq!(
        old_boot["result"]["value"].as_str(),
        Some(live.playwright.epoch().as_str()),
        "the old probe was booted with the new epoch before it was replaced, so it forgot its refs"
    );
    live.close(&tab).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_an_ops_objects_are_released_when_its_scope_drops() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = fixtures().await?;
    let tab = live
        .open(&server.url("127.0.0.1", "/password.html"))
        .await?;
    let env = live.env();
    let scope = OpScope::new(&env, count_op()?, CallHeader::default());
    let world = main_world(&live, &scope, &tab).await?;
    let frame = probe::call_for_tests(&world, &resolve(css("#frame")?, Want::Frame)).await;
    let Ok(ProbeReply::Frame { pin, .. }) = frame else {
        return Err(format!("the iframe did not resolve: {frame:?}").into());
    };
    let page = OpPage::admit(&live.playwright, &scope, tab.id).await?;
    let iframe = probe::pinned_object(&scope, &page, &world, pin)
        .await?
        .ok_or("the pinned iframe comes back by handle")?;
    let describe = json!({ "objectId": iframe, "depth": 0 });
    world
        .session()
        .send::<_, Value>("DOM.describeNode", describe.clone())
        .await?;
    drop(page);
    drop(scope);
    let mut released = false;
    for _ in 0..40 {
        if world
            .session()
            .send::<_, Value>("DOM.describeNode", describe.clone())
            .await
            .is_err()
        {
            released = true;
            break;
        }
        time::sleep(Duration::from_millis(25)).await;
    }
    assert!(released, "the op's object group went with its scope");
    let alive = probe::call_for_tests(&world, &ProbeRequest::Describe).await;
    assert!(
        matches!(alive, Ok(ProbeReply::Described { .. })),
        "the world's own handle is in another group and stays: {alive:?}"
    );
    live.close(&tab).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_set_content_keeps_the_seq_while_an_old_ref_fails() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = fixtures().await?;
    let tab = live
        .open(&server.url("127.0.0.1", "/password.html"))
        .await?;
    let env = live.env();
    let scope = OpScope::new(&env, count_op()?, CallHeader::default());
    let before = main_world(&live, &scope, &tab).await?;
    // A frame hop's key stands in for a snapshot ref: both live in Neo's aria-ref map.
    let frame = probe::call_for_tests(&before, &resolve(css("#frame")?, Want::Frame)).await;
    let Ok(ProbeReply::Frame { key, .. }) = frame else {
        return Err(format!("the iframe did not resolve: {frame:?}").into());
    };

    // The same markup again: a stale ref that re-resolved by position would find an iframe.
    let tab_id = live
        .browser
        .pages
        .get_tab_id(tab.page.clone())
        .await
        .ok_or("core knows the tab")?;
    let set = live
        .playwright
        .invoke(
            &env,
            Op::from_name("page.setContent").ok_or("page.setContent is an op")?,
            json!({ "page": { "$neo": "page", "tab": tab_id.0 },
                    "html": PASSWORD_PAGE }),
        )
        .await?;
    assert_eq!(set, Value::Null, "a void op replies with nothing");

    let page = OpPage::admit(&live.playwright, &scope, tab.id).await?;
    assert!(
        matches!(page.frames().by_seq(before.seq()), SeqLookup::Live { .. }),
        "document.open keeps the Document, so its seq stays live, as upstream's frame keeps its seq"
    );
    let after = page.world(&page.main_endpoint(), Place::Main).await?;
    assert_eq!(
        after.seq(),
        before.seq(),
        "the written content keeps the frame's seq and its refs' prefix"
    );
    let old_ref = probe::call_for_tests(
        &after,
        &resolve(ParsedSelector::frame_key(&key)?, Want::Element),
    )
    .await;
    assert!(
        matches!(&old_ref, Ok(ProbeReply::None { stale_ref: Some(stale) }) if *stale == key),
        "a ref minted before setContent fails at once: {old_ref:?}"
    );

    live.close(&tab).await;
    Ok(())
}

/// One frame of each kind on a fresh `/frames.html` tab, with the session each is installed on
/// and the URL its world must read: the main frame, a same-process child (on the page's own
/// session) and a cross-site child (an OOPIF, on its own session).
struct FrameKinds {
    tab: Tab,
    same: String,
    oop: String,
    oop_session: ProtocolSession,
    oop_url: String,
}

impl FrameKinds {
    async fn open(live: &LiveBrowser, server: &FixtureServer) -> TestResult<Self> {
        let tab = live.open(&server.url("127.0.0.1", "/frames.html")).await?;
        let tree: Value = tab.session.send("Page.getFrameTree", json!({})).await?;
        let same = tree["frameTree"]["childFrames"][0]["frame"]["id"]
            .as_str()
            .ok_or("the page session lists its same-process child")?
            .to_owned();
        let oop_url = server.url("localhost", "/child.html");
        let targets: Value = live
            .browser
            .cdp("Target.getTargets", json!({}), None)
            .await?;
        let oop = targets["targetInfos"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|info| info["type"] == "iframe" && info["url"] == oop_url.as_str())
            .and_then(|info| info["targetId"].as_str())
            .ok_or("the cross-site child is an OOPIF target")?
            .to_owned();
        let oop_session = live
            .browser
            .frames()
            .resolve_frame_target(
                tab.page.clone(),
                Some(FrameId(oop.clone())),
                Some(&tab.session),
                true,
            )
            .await?
            .session;
        Ok(Self {
            tab,
            same,
            oop,
            oop_session,
            oop_url,
        })
    }

    /// `(label, session, frame, url)` for each kind.
    fn each<'a>(
        &'a self,
        server: &FixtureServer,
    ) -> [(&'static str, &'a ProtocolSession, &'a str, String); 3] {
        [
            (
                "main frame",
                &self.tab.session,
                self.tab.target.as_str(),
                server.url("127.0.0.1", "/frames.html"),
            ),
            (
                "same-process child",
                &self.tab.session,
                self.same.as_str(),
                server.url("127.0.0.1", "/child.html"),
            ),
            (
                "OOPIF",
                &self.oop_session,
                self.oop.as_str(),
                self.oop_url.clone(),
            ),
        ]
    }
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_with_runtime_off_every_kind_of_frame_takes_its_world_by_number() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = fixtures().await?;
    let frames = FrameKinds::open(&live, &server).await?;
    let recorder = Recorder::over(&live);
    for (label, session, frame, url) in frames.each(&server) {
        let world = install_over(&live, &recorder, (session, frame), 0, Reporting::Off).await??;
        let evaluated = recorder.take("Runtime.evaluate");
        assert!(
            evaluated.len() == 1 && evaluated[0].get("contextId").is_some(),
            "{label}: the session reports no contexts, so the number takes the handle: \
             {evaluated:?}"
        );
        assert!(
            recorder.take("Runtime.enable").is_empty(),
            "{label}: the install never switches Runtime on"
        );
        assert_eq!(world_url(&world).await?, json!([url, false]), "{label}");
        let described = probe::call_for_tests(&world, &ProbeRequest::Describe).await;
        assert!(
            matches!(described, Ok(ProbeReply::Described { .. })),
            "{label}: {described:?}"
        );
    }
    live.close(&frames.tab).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_under_a_runtime_lease_every_kind_of_frame_takes_a_new_world_by_its_unique_id()
-> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = fixtures().await?;
    let frames = FrameKinds::open(&live, &server).await?;
    let recorder = Recorder::over(&live);
    for (label, session, frame, url) in frames.each(&server) {
        let world =
            install_over(&live, &recorder, (session, frame), 0, Reporting::Leased).await??;
        assert!(
            recorder.take("Runtime.addBinding").is_empty(),
            "{label}: world setup does not install a typing journal binding"
        );
        let evaluated = recorder.take("Runtime.evaluate");
        assert!(
            evaluated.len() == 1
                && evaluated[0].get("uniqueContextId").is_some()
                && evaluated[0].get("contextId").is_none(),
            "{label}: the session reported the new world, and its unique id took the handle: \
             {evaluated:?}"
        );
        assert_eq!(world_url(&world).await?, json!([url, false]), "{label}");
        let described = probe::call_for_tests(&world, &ProbeRequest::Describe).await;
        assert!(
            matches!(described, Ok(ProbeReply::Described { .. })),
            "{label}: {described:?}"
        );

        // The world exists now, so the create sends no event: the number takes the handle,
        // in a window the session reported nothing about it.
        let again =
            install_over(&live, &recorder, (session, frame), 0, Reporting::Leased).await??;
        let evaluated = recorder.take("Runtime.evaluate");
        assert!(
            evaluated.len() == 1 && evaluated[0].get("contextId").is_some(),
            "{label}: {evaluated:?}"
        );
        assert_eq!(world_url(&again).await?, json!([url, false]), "{label}");
    }
    live.close(&frames.tab).await;
    Ok(())
}

/// The guard holds only while the session reports its contexts, so every install here runs under
/// a `Runtime` lease. With `Runtime` off, as Neo leaves sessions by default, the same swap goes
/// unseen (`IsolatedWorld::install`).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_renderer_swap_in_the_install_window_fails_gone_never_a_page_world()
-> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = fixtures().await?;
    let start = server.url("127.0.0.1", "/password.html");
    let tab = live.open(&start).await?;
    let recorder = Recorder::over(&live);
    let main = (&tab.session, tab.target.as_str());

    // A new world: the document moves to another site between the create and the evaluate.
    recorder.swap_before("Runtime.evaluate", server.url("localhost", "/kids.html"));
    let installed = install_over(&live, &recorder, main, 0, Reporting::Leased).await?;
    let landed = match &installed {
        Ok(world) => world_url(world).await?,
        Err(_) => Value::Null,
    };
    assert!(
        matches!(installed, Err(ProbeFailure::Gone(_))),
        "a new world's unique id names nothing in the next renderer: {installed:?} in {landed}"
    );

    // A world that already existed: the same swap, now on the numeric path.
    tab.navigate(&start).await?;
    install_over(&live, &recorder, main, 0, Reporting::Leased).await??;
    recorder.swap_before(
        "Runtime.evaluate",
        server.url("localhost", "/kids.html?again"),
    );
    let installed = install_over(&live, &recorder, main, 0, Reporting::Leased).await?;
    let landed = match &installed {
        Ok(world) => world_url(world).await?,
        Err(_) => Value::Null,
    };
    assert!(
        matches!(installed, Err(ProbeFailure::Gone(_))),
        "the session reported the swap inside the window, so the number is not trusted: \
         {installed:?} in {landed}"
    );

    // The retry a gone world gets: the next document's own world.
    let world = install_over(&live, &recorder, main, 1, Reporting::Leased).await??;
    assert_eq!(
        world_url(&world).await?,
        json!([server.url("localhost", "/kids.html?again"), false])
    );
    live.close(&tab).await;
    Ok(())
}
