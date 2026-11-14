//! What the private entry's live suites share: the host's side of a call (its door's rules,
//! scripted), the fixture pages under `/private/`, and the refs an ai snapshot prints.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_core::{BrowserSession, BrowserView, FrameId, ProtocolSession, TabId};
use futures_util::future::BoxFuture;
use neo_playwright::{OpEnv, Refusal, RunState};
use neo_trace::ErrorOwner;
use serde_json::{Value, json};
use tokio::time;

use crate::fixture_server::{FixtureServer, Page};
use crate::live_browser::{LiveBrowser, TestPage, TestResult};

/// The host's side of one call: its door's rules (a lock that can be taken at the n-th ask),
/// whether it brings the tab forward before input, and what it was told.
pub(crate) struct HostEnv {
    view: BrowserView,
    run_state: Arc<RunState>,
    browser: Arc<BrowserSession>,
    /// Bring the tab forward before input, as the server does; `false` leaves it behind.
    front: bool,
    /// The door's rules refuse from this ask on (counting from 1).
    locked_at: Option<usize>,
    asked: AtomicUsize,
    pub(crate) sent: AtomicBool,
    notes: Mutex<Vec<String>>,
}

impl HostEnv {
    pub(crate) fn new(live: &LiveBrowser) -> Self {
        Self {
            view: BrowserView::unbound(Arc::clone(&live.browser), None),
            run_state: Arc::default(),
            browser: Arc::clone(&live.browser),
            front: false,
            locked_at: None,
            asked: AtomicUsize::new(0),
            sent: AtomicBool::new(false),
            notes: Mutex::default(),
        }
    }

    /// The same host, bringing the tab forward before input.
    pub(crate) fn in_front(mut self) -> Self {
        self.front = true;
        self
    }

    /// The same host, whose owner takes a lock on the tab at its `ask`-th ask of the rules.
    pub(crate) fn locked_at(mut self, ask: usize) -> Self {
        self.locked_at = Some(ask);
        self
    }

    pub(crate) fn was_sent(&self) -> bool {
        self.sent.load(Ordering::SeqCst)
    }

    pub(crate) fn notes(&self) -> Vec<String> {
        self.notes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl OpEnv for HostEnv {
    fn view(&self) -> &BrowserView {
        &self.view
    }

    fn run_state(&self) -> &Arc<RunState> {
        &self.run_state
    }

    fn recheck(&self) -> Result<(), Refusal> {
        let ask = self.asked.fetch_add(1, Ordering::SeqCst) + 1;
        if self.locked_at.is_some_and(|locked| ask >= locked) {
            return Err(Refusal {
                owner: ErrorOwner::Host,
                code: "owner_has_control".to_owned(),
                message: "The owner has control of this tab.".to_owned(),
                host_code: None,
            });
        }
        Ok(())
    }

    fn mark_sent(&self) {
        self.sent.store(true, Ordering::SeqCst);
    }

    fn note(&self, line: String) {
        self.notes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line);
    }

    fn bring_forward(&self, tab: TabId) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if !self.front {
                return;
            }
            let Ok(pages) = self.browser.pages.list().await else {
                return;
            };
            if let Some(info) = pages.into_iter().find(|info| info.tab_id == tab) {
                let target = json!({ "targetId": info.target_id.as_str() });
                let _brought = self
                    .browser
                    .cdp("Target.activateTarget", target, None)
                    .await;
            }
        })
    }
}

/// The fixture pages, under `/private/`.
#[expect(
    clippy::too_many_lines,
    reason = "one table of the private suites' fixture pages, a line pair each"
)]
pub(crate) fn pages() -> HashMap<String, Page> {
    [
        (
            "covered-login",
            include_str!("../data/private/covered-login.html"),
        ),
        (
            "covered-card",
            include_str!("../data/private/covered-card.html"),
        ),
        (
            "covered-cardframe",
            include_str!("../data/private/covered-cardframe.html"),
        ),
        (
            "smooth-login",
            include_str!("../data/private/smooth-login.html"),
        ),
        ("maxlength", include_str!("../data/private/maxlength.html")),
        ("compare", include_str!("../data/private/compare.html")),
        (
            "card-limits",
            include_str!("../data/private/card-limits.html"),
        ),
        (
            "frames-gone",
            include_str!("../data/private/frames-gone.html"),
        ),
        (
            "closed-plain",
            include_str!("../data/private/closed-plain.html"),
        ),
        (
            "pin-mirror",
            include_str!("../data/private/pin-mirror.html"),
        ),
        (
            "mark-moves",
            include_str!("../data/private/mark-moves.html"),
        ),
        (
            "busy-frame",
            include_str!("../data/private/busy-frame.html"),
        ),
        (
            "mark-follows",
            include_str!("../data/private/mark-follows.html"),
        ),
        (
            "out-of-view",
            include_str!("../data/private/out-of-view.html"),
        ),
        (
            "larger-than-view",
            include_str!("../data/private/larger-than-view.html"),
        ),
        (
            "quirks-login",
            include_str!("../data/private/quirks-login.html"),
        ),
        (
            "covered-select",
            include_str!("../data/private/covered-select.html"),
        ),
        ("look", include_str!("../data/private/look.html")),
        (
            "parent-cover",
            include_str!("../data/private/parent-cover.html"),
        ),
        (
            "parent-cover-middle",
            include_str!("../data/private/parent-cover-middle.html"),
        ),
        (
            "parent-cover-box",
            include_str!("../data/private/parent-cover-box.html"),
        ),
        (
            "drawn-label-login",
            include_str!("../data/private/drawn-label-login.html"),
        ),
        (
            "drawn-label-shapes",
            include_str!("../data/private/drawn-label-shapes.html"),
        ),
        (
            "drawn-code",
            include_str!("../data/private/drawn-code.html"),
        ),
        ("nested", include_str!("../data/private/nested.html")),
        ("moved", include_str!("../data/private/moved.html")),
        (
            "card-field",
            include_str!("../data/private/card-field.html"),
        ),
        (
            "card-frames",
            include_str!("../data/private/card-frames.html"),
        ),
        ("xframe", include_str!("../data/private/xframe.html")),
        ("mirror", include_str!("../data/private/mirror.html")),
        ("number", include_str!("../data/private/number.html")),
        ("rewrite", include_str!("../data/private/rewrite.html")),
        ("swap", include_str!("../data/private/swap.html")),
        ("labelled", include_str!("../data/private/labelled.html")),
        ("moved-box", include_str!("../data/private/moved-box.html")),
        (
            "closed-card",
            include_str!("../data/private/closed-card.html"),
        ),
        (
            "ten-frames",
            include_str!("../data/private/ten-frames.html"),
        ),
        (
            "checkout-ads",
            include_str!("../data/private/checkout-ads.html"),
        ),
        ("ad", include_str!("../data/private/ad.html")),
        ("stall", include_str!("../data/private/stall.html")),
        ("stale", include_str!("../data/private/stale.html")),
        ("stray", include_str!("../data/private/stray.html")),
    ]
    .into_iter()
    .map(|(name, body)| {
        let page = Page::html(body).with_header("cache-control", "no-store");
        (format!("/private/{name}.html"), page)
    })
    .collect()
}

/// The page's site and the other one, as origins.
pub(crate) fn origins(server: &FixtureServer) -> (String, String) {
    let origin = |host: &str| server.url(host, "/").trim_end_matches('/').to_owned();
    (origin("127.0.0.1"), origin("localhost"))
}

/// One printed node: its role, its name and its ref.
#[derive(Debug)]
pub(crate) struct Printed {
    pub(crate) role: String,
    pub(crate) name: String,
    pub(crate) r#ref: String,
}

/// Every ref a snapshot printed, child frames included.
fn printed(json: &Value) -> Vec<Printed> {
    fn visit(node: &Value, out: &mut Vec<Printed>) {
        if let Some(r#ref) = node.get("ref").and_then(Value::as_str) {
            out.push(Printed {
                role: text_of(node, "role"),
                name: text_of(node, "name"),
                r#ref: r#ref.to_owned(),
            });
        }
        for child in node
            .get("children")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            visit(child, out);
        }
    }
    let mut out = Vec::new();
    for node in json.as_array().into_iter().flatten() {
        visit(node, &mut out);
    }
    out
}

fn text_of(node: &Value, key: &str) -> String {
    node.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// The page's ai-mode snapshot, as an agent takes it.
pub(crate) async fn snapshot(live: &LiveBrowser, page: &TestPage) -> TestResult<Vec<Printed>> {
    let reply = live
        .invoke(
            "page.ariaSnapshot",
            json!({
                "call": { "api": "page.ariaSnapshot", "timeoutMs": 10_000 },
                "page": page.brand(),
                "mode": "ai",
            }),
        )
        .await??;
    Ok(printed(reply.get("json").unwrap_or(&Value::Null)))
}

/// The ref of the node `role` named `name`, once a snapshot prints it (a card frame loads after
/// its page).
pub(crate) async fn ref_of(
    live: &LiveBrowser,
    page: &TestPage,
    role: &str,
    name: &str,
) -> TestResult<String> {
    for _ in 0..50 {
        let found = snapshot(live, page)
            .await?
            .into_iter()
            .find(|node| node.role == role && node.name == name);
        if let Some(node) = found {
            return Ok(node.r#ref);
        }
        time::sleep(Duration::from_millis(100)).await;
    }
    Err(format!("no {role} named {name:?} printed within 5 s").into())
}

/// The tab's own (root) session: what every input to the page goes out on, except text for a
/// box in a cross-site frame.
pub(crate) async fn root_session(
    live: &LiveBrowser,
    page: &TestPage,
) -> TestResult<ProtocolSession> {
    Ok(live
        .browser
        .pages
        .get_session(page.id.clone())
        .await?
        .session)
}

/// The session of the page's only cross-site frame, which is its own target. The page's frame
/// tree does not list such a frame; the page's DOM still names it on the `<iframe>` element.
pub(crate) async fn frame_session(
    live: &LiveBrowser,
    page: &TestPage,
) -> TestResult<ProtocolSession> {
    let document = root_session(live, page)
        .await?
        .send_value("DOM.getDocument", json!({ "depth": -1 }))
        .await?;
    let child = iframe_frame_id(&document).ok_or("the page has no <iframe> with a frame")?;
    Ok(BrowserView::unbound(Arc::clone(&live.browser), None)
        .frame_session(page.id.clone(), FrameId(child))
        .await?)
}

/// The `frameId` of the first `<iframe>` in a `DOM.getDocument` tree.
fn iframe_frame_id(node: &Value) -> Option<String> {
    if node.get("nodeName").and_then(Value::as_str) == Some("IFRAME")
        && let Some(frame) = node.get("frameId").and_then(Value::as_str)
    {
        return Some(frame.to_owned());
    }
    let children = node.get("children").and_then(Value::as_array);
    let document = node.get("contentDocument").into_iter();
    let root = node.get("root").into_iter();
    children
        .into_iter()
        .flatten()
        .chain(document)
        .chain(root)
        .find_map(iframe_frame_id)
}
