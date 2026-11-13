//! The private entry's look (protected-fill design section 5.3) through
//! `Playwright::look_boxes`, against a real browser:
//!
//! - zuzu's 32 covered-rule cases (`claw-sdk/zuzu/pin-move/covered-own-label.md`), on zuzu's own
//!   fixture pages, each answering as main's rule answers;
//! - main's rule itself, read out of `browseros-core`'s source (`HIT_TEST_BLOCKER_JS`) and run on
//!   every box of those pages beside the port, so the two cannot drift apart;
//! - each refusal of the look on its own page, the facts, and the look's promises to the host:
//!   it never marks itself sent and its notes carry no preview.
//!
//! The live tests need BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005):
//! `cargo test -p neo-playwright --test private_look_live -- --ignored`. They serve the fixtures
//! on `127.0.0.1` and `localhost` (two sites, so a card frame on the other one is out of
//! process), and open and close their own tabs.

#[path = "support/fixture_server.rs"]
#[expect(
    dead_code,
    reason = "shared with suites that serve scripts and the nested-frame fixture"
)]
mod fixture_server;
#[path = "support/live_browser.rs"]
#[expect(
    dead_code,
    reason = "shared with suites that run ops in a run of their own"
)]
mod live_browser;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_core::BrowserView;
use fixture_server::{FixtureServer, Page};
use live_browser::{LiveBrowser, TestPage, TestResult};
use neo_playwright::{
    BoxLook, EntryError, Expect, LookField, LookRequest, OpEnv, Refusal, RunState,
};
use serde_json::{Value, json};
use tokio::time::{self, Instant};

/// Main's covered rule, as `browseros-core` ships it.
const GEOMETRY: &str = include_str!("../../../browseros-core/src/input/geometry.rs");

/// The host's side of the look: records whether it was marked sent and what it noted.
struct HostEnv {
    view: BrowserView,
    run_state: Arc<RunState>,
    sent: AtomicBool,
    notes: Mutex<Vec<String>>,
}

impl HostEnv {
    fn new(live: &LiveBrowser) -> Self {
        Self {
            view: BrowserView::unbound(Arc::clone(&live.browser), None),
            run_state: Arc::default(),
            sent: AtomicBool::new(false),
            notes: Mutex::default(),
        }
    }

    fn notes(&self) -> Vec<String> {
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
}

fn pages() -> HashMap<String, Page> {
    [
        (
            "covered-login",
            include_str!("data/private/covered-login.html"),
        ),
        (
            "covered-card",
            include_str!("data/private/covered-card.html"),
        ),
        (
            "covered-cardframe",
            include_str!("data/private/covered-cardframe.html"),
        ),
        (
            "covered-select",
            include_str!("data/private/covered-select.html"),
        ),
        ("look", include_str!("data/private/look.html")),
        ("nested", include_str!("data/private/nested.html")),
    ]
    .into_iter()
    .map(|(name, body)| {
        let page = Page::html(body).with_header("cache-control", "no-store");
        (format!("/private/{name}.html"), page)
    })
    .collect()
}

/// One printed node: its role, its name and its ref.
#[derive(Debug)]
struct Printed {
    role: String,
    name: String,
    r#ref: String,
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
async fn snapshot(live: &LiveBrowser, page: &TestPage) -> TestResult<Vec<Printed>> {
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
async fn ref_of(live: &LiveBrowser, page: &TestPage, role: &str, name: &str) -> TestResult<String> {
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

fn expecting(origin: &str) -> Expect {
    Expect {
        origin: origin.to_owned(),
        ..Expect::default()
    }
}

fn field(r#ref: &str, expect: Option<Expect>) -> LookField {
    LookField {
        box_ref: r#ref.to_owned(),
        expect,
        keys: false,
    }
}

/// One look at `fields` on `page`, as the host's check call makes it.
async fn look(
    live: &LiveBrowser,
    env: &HostEnv,
    page: &TestPage,
    url: Option<String>,
    fields: Vec<LookField>,
) -> TestResult<Vec<BoxLook>> {
    let request = LookRequest {
        tab: page.tab,
        url,
        fields,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    live.playwright
        .look_boxes(env, &request)
        .await
        .map_err(|error| format!("the look failed: {error:?}").into())
}

async fn look_one(
    live: &LiveBrowser,
    env: &HostEnv,
    page: &TestPage,
    field: LookField,
) -> TestResult<BoxLook> {
    look(live, env, page, None, vec![field])
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| "no answer for the box".into())
}

/// `page.locator(css).ref()`: Neo's node-bound ref for one attached node, as a host's check
/// script mints it for a box the snapshot printed no ref for (`n<k>`).
async fn mint_ref(live: &LiveBrowser, page: &TestPage, css: &str) -> TestResult<String> {
    let part = json!({
        "name": "css",
        "body": [{ "simples": [{ "selector": { "css": css, "functions": [] }, "combinator": "" }] }],
        "source": css,
    });
    let target = json!({
        "$neo": "locator", "page": page.brand(),
        "chunks": [{ "parts": [part] }],
        "display": format!("locator('{css}')"),
    });
    let input = json!({
        "call": { "api": "locator.ref", "timeoutMs": 5_000 },
        "target": target, "strict": "strict",
    });
    let minted = live.invoke("locator.ref", input).await??;
    Ok(minted
        .as_str()
        .ok_or("locator.ref answers a string")?
        .to_owned())
}

/// The refusal's code. Every refusal of the look is a fixed sentence that quotes nothing of
/// the page and says nothing was typed; checked on each one read here.
fn code(look: &BoxLook) -> Option<&str> {
    let refusal: Option<&EntryError> = look.refusal.as_ref();
    if let Some(error) = refusal {
        assert!(
            error.message.ends_with("Nothing was typed.") && !error.message.contains('<'),
            "{error:?}"
        );
    }
    refusal.map(|error| error.code.as_str())
}

/// Main's `HIT_TEST_BLOCKER_JS`, verbatim from `browseros-core`.
fn main_rule() -> TestResult<&'static str> {
    let start = "const HIT_TEST_BLOCKER_JS: &str = r#\"";
    let from = GEOMETRY
        .find(start)
        .ok_or("geometry.rs has no HIT_TEST_BLOCKER_JS")?
        + start.len();
    let rest = GEOMETRY.get(from..).ok_or("geometry.rs ends early")?;
    let to = rest
        .find("\"#;")
        .ok_or("HIT_TEST_BLOCKER_JS is not closed")?;
    Ok(rest.get(..to).ok_or("geometry.rs ends early")?.trim())
}

/// Whether main's own rule finds a blocker at the box `r#ref` names: its script, called on the
/// box with the centre main took from the protocol, in the box's own target's coordinates (the
/// page's for a same-origin frame, the frame's own for a cross-site one). Runs in the box's
/// document's main world, as `locator.evaluate` does; the DOM it reads is the same.
async fn main_rule_blocks(live: &LiveBrowser, page: &TestPage, r#ref: &str) -> TestResult<bool> {
    let function = format!(
        "(box) => {{
           const blockerAt = {};
           const rect = box.getBoundingClientRect();
           let x = rect.left + rect.width / 2, y = rect.top + rect.height / 2;
           for (let doc = box.ownerDocument; doc.defaultView && doc.defaultView.frameElement;) {{
             const frame = doc.defaultView.frameElement, at = frame.getBoundingClientRect();
             x += at.x + frame.clientLeft; y += at.y + frame.clientTop; doc = frame.ownerDocument;
           }}
           return blockerAt.call(box, x, y) !== null;
         }}",
        main_rule()?
    );
    let target = json!({
        "$neo": "locator", "page": page.brand(),
        "chunks": [{ "parts": [{ "name": "aria-ref", "body": r#ref, "source": r#ref }] }],
        "display": format!("getByRef('{ref}')"),
    });
    let input = json!({
        "call": { "api": "locator.evaluate", "timeoutMs": 10_000 },
        "target": target, "strict": "strict",
        "expression": function, "isFunction": true,
        "arg": { "value": r#"{"v":"undefined"}"#, "handles": [] },
    });
    let value = live.invoke("locator.evaluate", input).await??;
    let blocks = value
        .as_bool()
        .or_else(|| value.get("b").and_then(Value::as_bool))
        .ok_or_else(|| format!("main's rule answered {value}"))?;
    Ok(blocks)
}

/// One of zuzu's pages and what main's rule says about its boxes: (role, name, covered).
struct CoveredPage {
    cases: &'static str,
    path: &'static str,
    /// The origin the boxes' frame is on: the page's own, or the other site's.
    frame_site: Site,
    boxes: &'static [(&'static str, &'static str, bool)],
}

#[derive(Clone, Copy)]
enum Site {
    Page,
    Other,
}

const PASSWORD: (&str, &str) = ("textbox", "Password");
const EMAIL: (&str, &str) = ("textbox", "Email");
const NUMBER: (&str, &str) = ("textbox", "Card number");

const fn boxed(
    named: (&'static str, &'static str),
    covered: bool,
) -> (&'static str, &'static str, bool) {
    (named.0, named.1, covered)
}

/// zuzu's 32 cases by page. A `B` case is zuzu's fill on the page, a `C05k` case its field
/// read's `covered` fact: both answer main's covered rule, which the look is.
const COVERED: &[CoveredPage] = &[
    CoveredPage {
        cases: "B11c B05c C05kc: a cookie banner over the password box",
        path: "/private/covered-login.html?mode=banner",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, true)],
    },
    CoveredPage {
        cases: "an overlay on the password box",
        path: "/private/covered-login.html?mode=cover",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, true)],
    },
    CoveredPage {
        cases: "B11l C05kl: the box's own <label for> drawn over it",
        path: "/private/covered-login.html?mode=label",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, false)],
    },
    CoveredPage {
        cases: "B11w C05kw: a wrapping label whose text floats over the box",
        path: "/private/covered-login.html?mode=wrap",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, false)],
    },
    CoveredPage {
        cases: "B11i C05ki: an icon inside the box's own label",
        path: "/private/covered-login.html?mode=icon",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, false)],
    },
    CoveredPage {
        cases: "B11p C05kp: the box drawn under its own wrapper, an ancestor (kept as main)",
        path: "/private/covered-login.html?mode=under",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, false)],
    },
    CoveredPage {
        cases: "B11h C05kh: a cover inside the box's own shadow root (kept as main)",
        path: "/private/covered-login.html?mode=shadow",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, false)],
    },
    CoveredPage {
        cases: "B11a C05ka: an overlay that is a sibling of the box's ancestor",
        path: "/private/covered-login.html?mode=ancsib",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, true)],
    },
    CoveredPage {
        cases: "B11x C05kx: another field's label drawn over the box",
        path: "/private/covered-login.html?mode=otherlabel",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, true)],
    },
    CoveredPage {
        cases: "B11y C05ky: a decoration that is a child of the label's parent",
        path: "/private/covered-login.html?mode=sibling",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, true)],
    },
    CoveredPage {
        cases: "B05n C05kn: nothing over the box",
        path: "/private/covered-login.html?mode=none",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, false)],
    },
    CoveredPage {
        cases: "B05k C05kf: the card box covered inside its cross-site frame",
        path: "/private/covered-cardframe.html?inner=cover",
        frame_site: Site::Other,
        boxes: &[boxed(NUMBER, true)],
    },
    CoveredPage {
        cases: "B05o C05ko: the page's overlay over a cross-site frame is not seen (kept as main)",
        path: "/private/covered-cardframe.html?inner=none&outer=1",
        frame_site: Site::Other,
        boxes: &[boxed(NUMBER, false)],
    },
    CoveredPage {
        cases: "B05s C05ks: the page's overlay over a same-origin frame is seen",
        path: "/private/covered-cardframe.html?inner=none&outer=1&same=1",
        frame_site: Site::Page,
        boxes: &[boxed(NUMBER, true)],
    },
    CoveredPage {
        cases: "B05q: a card box in a cross-site frame, nothing over it",
        path: "/private/covered-cardframe.html?inner=none",
        frame_site: Site::Other,
        boxes: &[boxed(NUMBER, false)],
    },
    CoveredPage {
        cases: "a card box two same-origin frames deep, nothing over it",
        path: "/private/nested.html?inner=none",
        frame_site: Site::Page,
        boxes: &[boxed(NUMBER, false)],
    },
    CoveredPage {
        cases: "a card box two same-origin frames deep, covered in the innermost frame",
        path: "/private/nested.html?inner=cover",
        frame_site: Site::Page,
        boxes: &[boxed(NUMBER, true)],
    },
    CoveredPage {
        cases: "B05r: a card box in a same-origin frame, nothing over it",
        path: "/private/covered-cardframe.html?inner=none&same=1",
        frame_site: Site::Page,
        boxes: &[boxed(NUMBER, false)],
    },
];

/// The page's site and the other one, as origins.
fn origins(server: &FixtureServer) -> (String, String) {
    let origin = |host: &str| server.url(host, "/").trim_end_matches('/').to_owned();
    (origin("127.0.0.1"), origin("localhost"))
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_look_answers_zuzus_covered_cases_as_mains_rule() -> TestResult {
    let server = FixtureServer::start(pages()).await?;
    let live = LiveBrowser::connect().await?;
    let (page_origin, other_origin) = origins(&server);
    let mut failures = Vec::new();
    for case in COVERED {
        let page = live.open(&server.url("127.0.0.1", case.path)).await?;
        let origin = match case.frame_site {
            Site::Page => &page_origin,
            Site::Other => &other_origin,
        };
        for &(role, name, covered) in case.boxes {
            let r#ref = ref_of(&live, &page, role, name).await?;
            let env = HostEnv::new(&live);
            let answer =
                look_one(&live, &env, &page, field(&r#ref, Some(expecting(origin)))).await?;
            let wanted = covered.then_some("box_covered");
            if code(&answer) != wanted {
                failures.push(format!(
                    "{}: {name}: the look answered {:?}",
                    case.cases,
                    code(&answer)
                ));
            }
            if answer.facts.is_none() {
                failures.push(format!(
                    "{}: {name}: a covered box still answers its facts",
                    case.cases
                ));
            }
            let main = main_rule_blocks(&live, &page, &r#ref).await?;
            if main != covered {
                failures.push(format!(
                    "{}: {name}: main's own rule says covered={main}",
                    case.cases
                ));
            }
            assert!(
                !env.sent.load(Ordering::SeqCst),
                "a look never marks itself sent"
            );
        }
        live.close(&page).await?;
    }
    assert!(failures.is_empty(), "{failures:#?}");
    Ok(())
}

/// B11q: an input with `pointer-events: none` in a clickable wrapper gets no ref in the snapshot,
/// so no host can name it by a snapshot's ref (a fact, not a call). C18c: a dropdown is not a
/// text box: the look refuses it as `not_an_input`, where zuzu's script chooses it.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_without_a_ref_and_a_dropdown_answer_as_zuzu_expects() -> TestResult {
    let server = FixtureServer::start(pages()).await?;
    let live = LiveBrowser::connect().await?;
    let (page_origin, _) = origins(&server);

    let page = live
        .open(&server.url("127.0.0.1", "/private/covered-login.html?mode=pe"))
        .await?;
    ref_of(&live, &page, "textbox", "Email").await?;
    let printed = snapshot(&live, &page).await?;
    assert!(
        !printed
            .iter()
            .any(|node| node.role == "textbox" && node.name == "Password"),
        "B11q: a box with pointer-events: none gets no ref: {printed:?}"
    );
    // Ruling R17, design decision 2: the host mints a ref for such a box with locator.ref()
    // and names it by that ref. Its wrapper takes the hit at its centre: an ancestor, so it is
    // not covered under main's rule.
    let minted = mint_ref(&live, &page, "#pw").await?;
    assert!(minted.starts_with('n'), "a minted ref: {minted}");
    let env = HostEnv::new(&live);
    let answer = look_one(
        &live,
        &env,
        &page,
        field(&minted, Some(expecting(&page_origin))),
    )
    .await?;
    assert_eq!(code(&answer), None, "B11q by a minted ref: {answer:?}");
    let kind = answer.facts.map(|facts| facts.kind);
    assert_eq!(kind.as_deref(), Some("password"));
    live.close(&page).await?;

    let page = live
        .open(&server.url("127.0.0.1", "/private/covered-select.html"))
        .await?;
    let month = ref_of(&live, &page, "combobox", "Expiry month").await?;
    let env = HostEnv::new(&live);
    let answer = look_one(
        &live,
        &env,
        &page,
        field(&month, Some(expecting(&page_origin))),
    )
    .await?;
    assert_eq!(code(&answer), Some("not_an_input"), "C18c: {answer:?}");
    assert_eq!(answer.facts, None, "a dropdown has no box facts");
    live.close(&page).await?;
    Ok(())
}

/// B11m: covered when the look runs, free 1.5 s later. The look does not wait for the page: it
/// refuses now, and a later look passes.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_look_answers_the_page_as_it_is_now() -> TestResult {
    let server = FixtureServer::start(pages()).await?;
    let live = LiveBrowser::connect().await?;
    let (page_origin, _) = origins(&server);
    let page = live
        .open(&server.url("127.0.0.1", "/private/covered-login.html?mode=none"))
        .await?;
    let password = ref_of(&live, &page, "textbox", "Password").await?;
    live.main_world(&page, "window.coverFor(1500)").await?;
    let env = HostEnv::new(&live);
    let started = Instant::now();
    let now = look_one(
        &live,
        &env,
        &page,
        field(&password, Some(expecting(&page_origin))),
    )
    .await?;
    assert_eq!(
        code(&now),
        Some("box_covered"),
        "B11m: covered when the look ran"
    );
    assert!(
        started.elapsed() < Duration::from_millis(1000),
        "the look did not wait for the page"
    );
    // The page's own timer frees the box 1.5 s on; a tab behind another runs it a little late.
    let mut later = None;
    for _ in 0..30 {
        time::sleep(Duration::from_millis(250)).await;
        let field = field(&password, Some(expecting(&page_origin)));
        let answer = look_one(&live, &env, &page, field).await?;
        if code(&answer).is_none() {
            later = Some(started.elapsed());
            break;
        }
    }
    let later = later.ok_or("B11m: the box was never free again")?;
    assert!(
        later >= Duration::from_millis(1400),
        "free only once the page freed it: {later:?}"
    );
    live.close(&page).await?;
    Ok(())
}

/// The look's page (`look.html`), opened, with the facts its password box was approved with.
struct LookPage {
    server: FixtureServer,
    live: LiveBrowser,
    page: TestPage,
    url: String,
    origin: String,
    password: String,
    approved: Expect,
}

impl LookPage {
    async fn open() -> TestResult<Self> {
        let server = FixtureServer::start(pages()).await?;
        let live = LiveBrowser::connect().await?;
        let (origin, _) = origins(&server);
        let url = server.url("127.0.0.1", "/private/look.html");
        let page = live.open(&url).await?;
        let password = ref_of(&live, &page, "textbox", "Password").await?;
        let approved = Expect {
            origin: origin.clone(),
            kind: Some("password".to_owned()),
            autocomplete: Some("current-password".to_owned()),
            max_length: Some(24),
            input_mode: Some("text".to_owned()),
            name: Some("password".to_owned()),
            id: Some("pw".to_owned()),
        };
        Ok(Self {
            server,
            live,
            page,
            url,
            origin,
            password,
            approved,
        })
    }

    async fn ref_of(&self, role: &str, name: &str) -> TestResult<String> {
        ref_of(&self.live, &self.page, role, name).await
    }

    async fn look(&self, env: &HostEnv, field: LookField) -> TestResult<BoxLook> {
        look_one(&self.live, env, &self.page, field).await
    }

    /// A field for `r#ref` that expects only this page's origin.
    fn plain(&self, r#ref: &str) -> LookField {
        field(r#ref, Some(expecting(&self.origin)))
    }

    async fn close(self) -> TestResult {
        self.live.close(&self.page).await?;
        drop(self.server);
        Ok(())
    }
}

/// Every fact as the host approved it, with the tab at its address: no refusal, and the facts
/// as the host's check call returns them.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_an_approved_box_answers_its_facts() -> TestResult {
    let at = LookPage::open().await?;
    let env = HostEnv::new(&at.live);
    let fields = vec![field(&at.password, Some(at.approved.clone()))];
    let answers = look(&at.live, &env, &at.page, Some(at.url.clone()), fields).await?;
    let ready = answers.first().ok_or("one answer")?;
    assert_eq!(code(ready), None, "{ready:?}");
    let facts = ready.facts.clone().ok_or("the box's facts")?;
    assert_eq!(
        serde_json::to_value(&facts)?,
        json!({ "origin": at.origin, "type": "password", "autocomplete": "current-password",
                "maxLength": 24, "inputMode": "text", "name": "password", "id": "pw",
                "sized": true, "empty": true }),
    );
    // The check call before any approval checks no origin and no fact.
    let answer = at.look(&env, field(&at.password, None)).await?;
    assert_eq!(code(&answer), None);
    let code_box = at.ref_of("textbox", "Code").await?;
    let answer = at.look(&env, field(&code_box, None)).await?;
    assert_eq!(
        answer.facts.map(|facts| facts.empty),
        Some(false),
        "a box with text is not empty"
    );
    assert!(
        !env.sent.load(Ordering::SeqCst),
        "a look never marks itself sent"
    );
    // A preview prints the `value` attribute (123 on the code box): the look notes none.
    // The look writes no call-log line at all, so no preview and no text of the page.
    let notes = env.notes();
    assert!(notes.is_empty(), "{notes:#?}");
    at.close().await
}

/// Step 3: the frame's origin and the tab's address, in that order, both before the facts.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_origin_and_the_address_come_first() -> TestResult {
    let at = LookPage::open().await?;
    let env = HostEnv::new(&at.live);
    let elsewhere = Expect {
        origin: "https://elsewhere.example".to_owned(),
        ..at.approved.clone()
    };
    let answer = at.look(&env, field(&at.password, Some(elsewhere))).await?;
    assert_eq!(code(&answer), Some("wrong_origin"));
    let fields = vec![field(&at.password, Some(at.approved.clone()))];
    let moved = look(
        &at.live,
        &env,
        &at.page,
        Some(format!("{}#other", at.url)),
        fields,
    )
    .await?;
    assert_eq!(moved.first().and_then(code), Some("page_changed"));
    // Step 3's order: the frame's origin before the tab's address, the address before the facts.
    let elsewhere_moved = Expect {
        origin: "https://elsewhere.example".to_owned(),
        ..at.approved.clone()
    };
    let both = vec![field(&at.password, Some(elsewhere_moved))];
    let answers = look(
        &at.live,
        &env,
        &at.page,
        Some(format!("{}#other", at.url)),
        both,
    )
    .await?;
    assert_eq!(answers.first().and_then(code), Some("wrong_origin"));
    let typed_moved = Expect {
        kind: Some("email".to_owned()),
        ..at.approved.clone()
    };
    let both = vec![field(&at.password, Some(typed_moved))];
    let answers = look(
        &at.live,
        &env,
        &at.page,
        Some(format!("{}#other", at.url)),
        both,
    )
    .await?;
    assert_eq!(answers.first().and_then(code), Some("page_changed"));

    at.close().await
}

/// Step 4: the type (its own code, the host's) and each other fact (`box_changed`, the fact
/// named in the message).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_the_host_did_not_approve_is_refused() -> TestResult {
    let at = LookPage::open().await?;
    let env = HostEnv::new(&at.live);
    let swapped = Expect {
        kind: Some("email".to_owned()),
        ..at.approved.clone()
    };
    let answer = at.look(&env, field(&at.password, Some(swapped))).await?;
    assert_eq!(code(&answer), Some("wrong_type"));
    let owner = answer.refusal.as_ref().map(|error| error.owner);
    assert_eq!(owner, Some(neo_trace::ErrorOwner::Host));

    let approved = &at.approved;
    let changed = [
        (
            "autocomplete",
            Expect {
                autocomplete: Some("username".to_owned()),
                ..approved.clone()
            },
        ),
        (
            "maxLength",
            Expect {
                max_length: Some(8),
                ..approved.clone()
            },
        ),
        (
            "inputMode",
            Expect {
                input_mode: Some("numeric".to_owned()),
                ..approved.clone()
            },
        ),
        (
            "name",
            Expect {
                name: Some("pass".to_owned()),
                ..approved.clone()
            },
        ),
        (
            "id",
            Expect {
                id: Some("password".to_owned()),
                ..approved.clone()
            },
        ),
    ];
    for (fact, expect) in changed {
        let answer = at.look(&env, field(&at.password, Some(expect))).await?;
        assert_eq!(code(&answer), Some("box_changed"), "{fact}");
        let message = answer
            .refusal
            .map(|error| error.message)
            .unwrap_or_default();
        assert!(
            message.contains(&format!("box's {fact} ")),
            "{fact}: {message}"
        );
    }
    at.close().await
}

/// Steps 2 and 5: not a text input (no facts), and not editable (facts still answered).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_that_takes_no_typing_is_refused() -> TestResult {
    let at = LookPage::open().await?;
    let env = HostEnv::new(&at.live);
    for (role, name) in [
        ("textbox", "Editable div"),
        ("combobox", "Month"),
        ("checkbox", "Remember me"),
    ] {
        let r#ref = at.ref_of(role, name).await?;
        let answer = at.look(&env, at.plain(&r#ref)).await?;
        assert_eq!(code(&answer), Some("not_an_input"), "{name}");
        assert_eq!(answer.facts, None, "{name}");
    }
    for name in ["Disabled", "Read only", "In a disabled fieldset"] {
        let r#ref = at.ref_of("textbox", name).await?;
        let answer = at.look(&env, at.plain(&r#ref)).await?;
        assert_eq!(code(&answer), Some("box_not_editable"), "{name}");
        assert!(answer.facts.is_some(), "{name} still answers its facts");
        // Before any approval too: this refusal needs none.
        let answer = at.look(&env, field(&r#ref, None)).await?;
        assert_eq!(code(&answer), Some("box_not_editable"), "{name}, no expect");
    }
    at.close().await
}

/// Steps 6 and 1: a box that stops showing, one that loses its size (hidden for one insertion,
/// reached by focus when typed key by key), a box removed, a ref never printed, and text that
/// is no ref. Each refusal of the look comes before anything is sent, so it is retryable.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_that_changed_since_the_snapshot_is_refused() -> TestResult {
    let at = LookPage::open().await?;
    let env = HostEnv::new(&at.live);
    let hidden = at.ref_of("textbox", "Goes hidden").await?;
    let sizeless = at.ref_of("textbox", "Loses its size").await?;
    let gone = at.ref_of("textbox", "Goes away").await?;
    at.live
        .main_world(
            &at.page,
            "document.querySelector('[aria-label=\"Goes hidden\"]').style.visibility = 'hidden';
             Object.assign(document.querySelector('[aria-label=\"Loses its size\"]').style,
               { width: '0', height: '0', padding: '0', border: '0' });
             document.querySelector('[aria-label=\"Goes away\"]').remove(); true",
        )
        .await?;
    let answer = at.look(&env, at.plain(&hidden)).await?;
    assert_eq!(code(&answer), Some("box_hidden"));
    assert_eq!(answer.refusal.map(|error| error.retryable), Some(true));
    let answer = at.look(&env, at.plain(&sizeless)).await?;
    assert_eq!(
        code(&answer),
        Some("box_hidden"),
        "one insertion needs a box that shows"
    );
    let keyed = LookField {
        keys: true,
        ..at.plain(&sizeless)
    };
    let answer = at.look(&env, keyed).await?;
    assert_eq!(
        code(&answer),
        None,
        "a field typed key by key may have no size"
    );
    assert_eq!(answer.facts.map(|facts| facts.sized), Some(false));
    for r#ref in [gone.as_str(), "e999", "#4", "e5'); alert(1"] {
        let answer = at.look(&env, at.plain(r#ref)).await?;
        assert_eq!(code(&answer), Some("element_detached"), "{ref}");
        assert_eq!(answer.facts, None, "{ref}");
    }
    assert!(
        !env.sent.load(Ordering::SeqCst),
        "a look never marks itself sent"
    );
    at.close().await
}

/// A box in a cross-site frame answers its own frame's origin, and the look runs on a tab that
/// is not in front as on one that is.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_cross_site_box_answers_its_frames_origin_on_a_tab_behind() -> TestResult {
    let server = FixtureServer::start(pages()).await?;
    let live = LiveBrowser::connect().await?;
    let (page_origin, other_origin) = origins(&server);
    let page = live
        .open(&server.url("127.0.0.1", "/private/covered-cardframe.html?inner=none"))
        .await?;
    let number = ref_of(&live, &page, "textbox", "Card number").await?;
    assert!(
        number.starts_with('f'),
        "the card box is in a frame: {number}"
    );
    // Another tab in front of the card's tab.
    let front = live
        .open(&server.url("127.0.0.1", "/private/look.html"))
        .await?;
    let env = HostEnv::new(&live);
    let answer = look_one(
        &live,
        &env,
        &page,
        field(&number, Some(expecting(&other_origin))),
    )
    .await?;
    assert_eq!(code(&answer), None, "{answer:?}");
    assert_eq!(
        answer.facts.map(|facts| facts.origin),
        Some(other_origin.clone())
    );
    let answer = look_one(
        &live,
        &env,
        &page,
        field(&number, Some(expecting(&page_origin))),
    )
    .await?;
    assert_eq!(
        code(&answer),
        Some("wrong_origin"),
        "the page's origin is not the frame's"
    );
    live.close(&front).await?;
    live.close(&page).await?;
    Ok(())
}
