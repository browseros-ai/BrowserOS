//! The private entry's look (protected-fill design section 5.3) through
//! `Playwright::look_boxes`, against a real browser:
//!
//! - covered-rule cases on local fixture pages, each answering as the core rule answers;
//! - main's rule itself, `browseros-core`'s own point and hit test (`get_element_center`,
//!   `click_blocker_at_point`, its `HIT_TEST_BLOCKER_JS`) run on every box of those pages beside
//!   the port, so the two cannot drift apart;
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
#[path = "support/private_host.rs"]
#[expect(
    dead_code,
    reason = "the entry's suite scripts the host's lock and the tab's place; the look needs neither"
)]
mod private_host;

use std::time::Duration;

use browseros_core::input::geometry;

use fixture_server::FixtureServer;
use live_browser::{LiveBrowser, TestPage, TestResult};
use neo_playwright::{
    BoxLook, CoverIn, EntryCode, EntryError, Expect, LookField, LookRequest, Unread,
};
use private_host::HostEnv;
use serde_json::{Value, json};
use tokio::time::{self, Instant};
use url::form_urlencoded::byte_serialize;

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
            error.message.contains("Nothing was typed.") && !error.message.contains('<'),
            "{error:?}"
        );
    }
    refusal.map(|error| error.code.as_str())
}

/// Whether main's own fill would find a blocker at the box `r#ref` names: `browseros-core`'s
/// own functions, called as its fill called them (input/fill.rs), on the box's own target
/// session: `scroll_into_view`, `get_element_center` (the centre of the first content quad,
/// `DOM.getContentQuads`) and `click_blocker_at_point` (`HIT_TEST_BLOCKER_JS`). `own_frame`
/// names a box in the page's only cross-site frame, which is its own target.
async fn main_rule_blocks(
    live: &LiveBrowser,
    page: &TestPage,
    r#ref: &str,
    own_frame: bool,
) -> TestResult<bool> {
    let target = json!({
        "$neo": "locator", "page": page.brand(),
        "chunks": [{ "parts": [{ "name": "aria-ref", "body": r#ref, "source": r#ref }] }],
        "display": format!("getByRef('{ref}')"),
    });
    let handle = live
        .invoke(
            "locator.waitFor",
            json!({ "call": { "api": "locator.elementHandle", "timeoutMs": 10_000 },
                "target": target, "strict": "strict", "state": "attached",
                "omitReturnValue": false }),
        )
        .await??;
    let node = handle
        .get("node")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("no node in {handle}"))?;
    let session = if own_frame {
        private_host::frame_session(live, page).await?
    } else {
        private_host::root_session(live, page).await?
    };
    geometry::scroll_into_view(&session, node).await;
    let point = geometry::get_element_center(&session, node).await?;
    Ok(geometry::click_blocker_at_point(&session, node, point)
        .await?
        .is_some())
}

/// A fixture page and the core rule's answer for each box: (role, name, covered).
struct CoveredPage {
    cases: &'static str,
    path: &'static str,
    /// The origin the boxes' frame is on: the page's own, or the other site's.
    frame_site: Site,
    boxes: &'static [(&'static str, &'static str, bool)],
}

/// The covered pages whose cover is a blank drawing inside the box's row, within its rectangle
/// grown by half, holding no control (ruling R56): one insertion still answers `box_covered`
/// there, as main's rule does, and key by key the box passes. Measured on every page of
/// [`COVERED`]; these are the only ones that differ. (case, box name).
const OWN_DRAWINGS: &[(&str, &str)] = &[
    ("an overlay on the password box", "Password"),
    (
        "an overlay that is a sibling of the box's ancestor",
        "Password",
    ),
    (
        "a decoration that is a child of the label's parent",
        "Password",
    ),
    (
        "the card box covered inside its cross-site frame",
        "Card number",
    ),
    (
        "a card box two same-origin frames deep, covered in the innermost frame",
        "Card number",
    ),
];

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

/// Pages where the look refuses and main's rule typed: a cover in the page above a frame on
/// another origin, which main's hit test never asked (ruling R60, `look_op.rs`'s
/// `pages_above`). Main's own rule still answers not covered there, and is held to that.
const ABOVE_MAIN: &[&str] = &["the page's overlay over a cross-site frame"];

/// Fixture cases grouped by page. Fill eligibility and the look's `covered` fact must agree
/// with the core's hit-test rule.
const COVERED: &[CoveredPage] = &[
    CoveredPage {
        cases: "a cookie banner over the password box",
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
        cases: "the box's own <label for> drawn over it",
        path: "/private/covered-login.html?mode=label",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, false)],
    },
    CoveredPage {
        cases: "a wrapping label whose text floats over the box",
        path: "/private/covered-login.html?mode=wrap",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, false)],
    },
    CoveredPage {
        cases: "an icon inside the box's own label",
        path: "/private/covered-login.html?mode=icon",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, false)],
    },
    CoveredPage {
        cases: "the box drawn under its own wrapper, an ancestor (kept as main)",
        path: "/private/covered-login.html?mode=under",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, false)],
    },
    CoveredPage {
        cases: "a cover inside the box's own shadow root (kept as main)",
        path: "/private/covered-login.html?mode=shadow",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, false)],
    },
    CoveredPage {
        cases: "an overlay that is a sibling of the box's ancestor",
        path: "/private/covered-login.html?mode=ancsib",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, true)],
    },
    CoveredPage {
        cases: "another field's label drawn over the box",
        path: "/private/covered-login.html?mode=otherlabel",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, true)],
    },
    CoveredPage {
        cases: "a decoration that is a child of the label's parent",
        path: "/private/covered-login.html?mode=sibling",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, true)],
    },
    CoveredPage {
        cases: "nothing over the box",
        path: "/private/covered-login.html?mode=none",
        frame_site: Site::Page,
        boxes: &[boxed(EMAIL, false), boxed(PASSWORD, false)],
    },
    CoveredPage {
        cases: "the card box covered inside its cross-site frame",
        path: "/private/covered-cardframe.html?inner=cover",
        frame_site: Site::Other,
        boxes: &[boxed(NUMBER, true)],
    },
    CoveredPage {
        cases: "the page's overlay over a cross-site frame",
        path: "/private/covered-cardframe.html?inner=none&outer=1",
        frame_site: Site::Other,
        boxes: &[boxed(NUMBER, true)],
    },
    CoveredPage {
        cases: "the page's overlay over a same-origin frame is seen",
        path: "/private/covered-cardframe.html?inner=none&outer=1&same=1",
        frame_site: Site::Page,
        boxes: &[boxed(NUMBER, true)],
    },
    CoveredPage {
        cases: "a card box in a cross-site frame, nothing over it",
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
        cases: "a card box in a same-origin frame, nothing over it",
        path: "/private/covered-cardframe.html?inner=none&same=1",
        frame_site: Site::Page,
        boxes: &[boxed(NUMBER, false)],
    },
];

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_look_answers_covered_cases_as_the_core_rule() -> TestResult {
    let server = FixtureServer::start(private_host::pages()).await?;
    let live = LiveBrowser::connect().await?;
    let (page_origin, other_origin) = private_host::origins(&server);
    let mut failures = Vec::new();
    for case in COVERED {
        let page = live.open(&server.url("127.0.0.1", case.path)).await?;
        let origin = match case.frame_site {
            Site::Page => &page_origin,
            Site::Other => &other_origin,
        };
        for &(role, name, covered) in case.boxes {
            let r#ref = private_host::ref_of(&live, &page, role, name).await?;
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
            // Key by key, the field's own blank drawing over its box lets it through (ruling
            // R56): the look differs from main's rule there, and only there.
            let keyed = LookField {
                keys: true,
                ..field(&r#ref, Some(expecting(origin)))
            };
            let keyed = look_one(&live, &env, &page, keyed).await?;
            let own_drawing = OWN_DRAWINGS.contains(&(case.cases, name));
            let wanted = (covered && !own_drawing).then_some("box_covered");
            if code(&keyed) != wanted {
                failures.push(format!(
                    "{}: {name}: key by key the look answered {:?}",
                    case.cases,
                    code(&keyed)
                ));
            }
            if answer.facts.is_none() {
                failures.push(format!(
                    "{}: {name}: a covered box still answers its facts",
                    case.cases
                ));
            }
            let own_frame = matches!(case.frame_site, Site::Other);
            let main = main_rule_blocks(&live, &page, &r#ref, own_frame).await?;
            if main != (covered && !ABOVE_MAIN.contains(&case.cases)) {
                failures.push(format!(
                    "{}: {name}: main's own rule says covered={main}",
                    case.cases
                ));
            }
            assert!(!env.was_sent(), "a look never marks itself sent");
        }
        live.close(&page).await?;
    }
    assert!(failures.is_empty(), "{failures:#?}");
    Ok(())
}

/// An input with `pointer-events: none` in a clickable wrapper gets no ref in the snapshot,
/// so no host can name it by a snapshot's ref (a fact, not a call). A dropdown is not a
/// text box: the look refuses it as `not_an_input`.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_without_a_ref_and_a_dropdown_answer_as_the_host_expects() -> TestResult {
    let server = FixtureServer::start(private_host::pages()).await?;
    let live = LiveBrowser::connect().await?;
    let (page_origin, _) = private_host::origins(&server);

    let page = live
        .open(&server.url("127.0.0.1", "/private/covered-login.html?mode=pe"))
        .await?;
    private_host::ref_of(&live, &page, "textbox", "Email").await?;
    let printed = private_host::snapshot(&live, &page).await?;
    assert!(
        !printed
            .iter()
            .any(|node| node.role == "textbox" && node.name == "Password"),
        "noninteractive box: a box with pointer-events: none gets no ref: {printed:?}"
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
    assert_eq!(
        code(&answer),
        None,
        "noninteractive box by a minted ref: {answer:?}"
    );
    let kind = answer.facts.map(|facts| facts.kind);
    assert_eq!(kind.as_deref(), Some("password"));
    live.close(&page).await?;

    let page = live
        .open(&server.url("127.0.0.1", "/private/covered-select.html"))
        .await?;
    let month = private_host::ref_of(&live, &page, "combobox", "Expiry month").await?;
    let env = HostEnv::new(&live);
    let answer = look_one(
        &live,
        &env,
        &page,
        field(&month, Some(expecting(&page_origin))),
    )
    .await?;
    assert_eq!(code(&answer), Some("not_an_input"), "dropdown: {answer:?}");
    assert_eq!(answer.facts, None, "a dropdown has no box facts");
    live.close(&page).await?;
    Ok(())
}

/// Covered when the look runs, free 1.5 s later. The look does not wait for the page: it
/// refuses now, and a later look passes.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_look_answers_the_page_as_it_is_now() -> TestResult {
    let server = FixtureServer::start(private_host::pages()).await?;
    let live = LiveBrowser::connect().await?;
    let (page_origin, _) = private_host::origins(&server);
    let page = live
        .open(&server.url("127.0.0.1", "/private/covered-login.html?mode=none"))
        .await?;
    let password = private_host::ref_of(&live, &page, "textbox", "Password").await?;
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
        "temporary cover: covered when the look ran"
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
    let later = later.ok_or("temporary cover: the box was never free again")?;
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
        let server = FixtureServer::start(private_host::pages()).await?;
        let live = LiveBrowser::connect().await?;
        let (origin, _) = private_host::origins(&server);
        let url = server.url("127.0.0.1", "/private/look.html");
        let page = live.open(&url).await?;
        let password = private_host::ref_of(&live, &page, "textbox", "Password").await?;
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
        private_host::ref_of(&self.live, &self.page, role, name).await
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
    // The check call has no value, so no length: a box's maxlength refuses nothing (step 7 is
    // the entry's), and the facts carry it for the host to compare.
    let code_box = at.ref_of("textbox", "Code").await?;
    let answer = at.look(&env, at.plain(&code_box)).await?;
    assert_eq!(code(&answer), None);
    assert_eq!(answer.facts.and_then(|facts| facts.max_length), Some(6));
    // Each box answers on its own: one that names nothing does not end the look at the others.
    let three = vec![
        at.plain(&at.password),
        at.plain("e999"),
        at.plain(&code_box),
    ];
    let answers = look(&at.live, &env, &at.page, None, three).await?;
    let codes: Vec<Option<&str>> = answers.iter().map(code).collect();
    assert_eq!(codes, vec![None, Some("element_detached"), None]);
    assert!(answers.first().is_some_and(|first| first.facts.is_some()));
    assert!(answers.get(2).is_some_and(|third| third.facts.is_some()));
    // The check call before any approval checks no origin and no fact.
    let answer = at.look(&env, field(&at.password, None)).await?;
    assert_eq!(code(&answer), None);
    let answer = at.look(&env, field(&code_box, None)).await?;
    assert_eq!(
        answer.facts.map(|facts| facts.empty),
        Some(false),
        "a box with text is not empty"
    );
    assert!(!env.was_sent(), "a look never marks itself sent");
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
    assert!(!env.was_sent(), "a look never marks itself sent");
    at.close().await
}

/// A box in a cross-site frame answers its own frame's origin, and the look runs on a tab that
/// is not in front as on one that is.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_cross_site_box_answers_its_frames_origin_on_a_tab_behind() -> TestResult {
    let server = FixtureServer::start(private_host::pages()).await?;
    let live = LiveBrowser::connect().await?;
    let (page_origin, other_origin) = private_host::origins(&server);
    let page = live
        .open(&server.url("127.0.0.1", "/private/covered-cardframe.html?inner=none"))
        .await?;
    let number = private_host::ref_of(&live, &page, "textbox", "Card number").await?;
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

/// Rulings R55 and R56 at the look, as the host's check call makes it: a field's own drawing over
/// its box, which is no `<label>`, with nothing set by the host. Main's own rule calls every box
/// here covered. The look lets through a drawing that lies over its box, holds no control, and
/// draws the box's accessible name, either way of typing: a sibling div or span, one whose
/// wrapper focuses the box on a click, a span the box is named by (`aria-labelledby`), an outline
/// fieldset's legend, a `<label>` that labels no control. A drawing that draws nothing (an SVG
/// whose one text is a `<title>` included) only key by key: one insertion answers
/// `box_covered`. Every other shape covers the box either way: a banner far wider than the box,
/// other words, a drawing that holds a button, one outside the box's parent and grandparent, and
/// a `<label>` for another field, which a person's click would send to that field.
/// These are the pages where the look and main's rule differ (the parity test's other pages).
///
/// Fails, each at the boxes named: the drawing's branch left out of the look (every box the
/// look should let through answers `box_covered`); point 5 reading `aria-label` alone
/// (`#named-box`); a blank drawing let through for one insertion too (`#blank-box`); the
/// rendered text read as `textContent` (`#blank-box` draws its hidden name and passes by one
/// insertion, as does `#svg-box` its `<title>`); the label condition taken out (`#for-box`, a
/// label for another field, passes); the rectangle grown without bound on each side (`#banner-box`); a control
/// selector that matches nothing (`#button-box`).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_fields_own_drawing_lets_its_box_through() -> TestResult {
    // (page, box, let through by one insertion, let through key by key)
    const BOXES: [(&str, &str, bool, bool); 15] = [
        ("/private/drawn-label-login.html", "#user", true, true),
        ("/private/drawn-label-login.html", "#pw", true, true),
        ("/private/drawn-label-shapes.html", "#span-box", true, true),
        ("/private/drawn-label-shapes.html", "#focus-box", true, true),
        ("/private/drawn-label-shapes.html", "#named-box", true, true),
        (
            "/private/drawn-label-shapes.html",
            "#outline-box",
            true,
            true,
        ),
        ("/private/drawn-label-shapes.html", "#label-box", true, true),
        (
            "/private/drawn-label-shapes.html",
            "#shrink-box",
            true,
            true,
        ),
        (
            "/private/drawn-label-shapes.html",
            "#blank-box",
            false,
            true,
        ),
        ("/private/drawn-label-shapes.html", "#svg-box", false, true),
        (
            "/private/drawn-label-shapes.html",
            "#banner-box",
            false,
            false,
        ),
        (
            "/private/drawn-label-shapes.html",
            "#other-box",
            false,
            false,
        ),
        (
            "/private/drawn-label-shapes.html",
            "#button-box",
            false,
            false,
        ),
        ("/private/drawn-label-shapes.html", "#far-box", false, false),
        ("/private/drawn-label-shapes.html", "#for-box", false, false),
    ];
    let server = FixtureServer::start(private_host::pages()).await?;
    let live = LiveBrowser::connect().await?;
    let (page_origin, _other) = private_host::origins(&server);
    let mut failures = Vec::new();
    for path in [
        "/private/drawn-label-login.html",
        "/private/drawn-label-shapes.html",
    ] {
        let page = live.open(&server.url("127.0.0.1", path)).await?;
        for &(_, css, inserted, keyed) in BOXES.iter().filter(|(on, ..)| *on == path) {
            let r#ref = mint_ref(&live, &page, css).await?;
            let env = HostEnv::new(&live);
            if !main_rule_blocks(&live, &page, &r#ref, false).await? {
                failures.push(format!("{css}: main's own rule does not call it covered"));
            }
            let plain = field(&r#ref, Some(expecting(&page_origin)));
            for (keys, through) in [(false, inserted), (true, keyed)] {
                let ask = LookField {
                    keys,
                    ..plain.clone()
                };
                let answer = look_one(&live, &env, &page, ask).await?;
                let wanted = (!through).then_some("box_covered");
                if code(&answer) != wanted {
                    failures.push(format!(
                        "{css}, keys {keys}: the look answered {:?}",
                        code(&answer)
                    ));
                }
            }
            assert!(!env.was_sent(), "a look never marks itself sent");
        }
        live.close(&page).await?;
    }
    assert!(failures.is_empty(), "{failures:#?}");
    Ok(())
}

/// Where the box's own scroller stands on `smooth-login.html`, in CSS pixels down.
async fn standing(live: &LiveBrowser, page: &TestPage) -> TestResult<f64> {
    live.main_world(page, "standing()")
        .await?
        .as_f64()
        .ok_or_else(|| "standing() answers a number".into())
}

/// Ruling R61's rows 1 to 7 for the look, which the host's check call makes: a password box far
/// below the fold of a page that scrolls smoothly, on its root or on a container between the box
/// and the root, or in a frame of the page's own origin, is brought into view at once before the
/// look asks what takes the pointer there. Refused under a sheet over the whole view and passed
/// without it, by one insertion and key by key; the box's scroller ends at the box, and a box
/// already in view does not move it. With smooth scrolling off the answers are main's. Fails with
/// the look's instant scroll left out (`look_op.rs`): the page's own scroll animates, the hit
/// finds nothing at the box's place, and the sheet passes.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_below_the_fold_of_a_smooth_page_is_looked_at_in_view() -> TestResult {
    let server = FixtureServer::start(private_host::pages()).await?;
    let live = LiveBrowser::connect().await?;
    let (page_origin, _) = private_host::origins(&server);
    let mut failures = Vec::new();
    for shape in ["", "&scroller=1", "&frame=1", "&plain=1"] {
        for (sheet, wanted) in [("1", Some("box_covered")), ("", None)] {
            let path = format!("/private/smooth-login.html?sheet={sheet}{shape}");
            let page = live.open(&server.url("127.0.0.1", &path)).await?;
            let r#ref = private_host::ref_of(&live, &page, "textbox", "Password").await?;
            let env = HostEnv::new(&live);
            for keys in [false, true] {
                let ask = LookField {
                    keys,
                    ..field(&r#ref, Some(expecting(&page_origin)))
                };
                let answer = look_one(&live, &env, &page, ask).await?;
                if code(&answer) != wanted {
                    failures.push(format!("{path}, keys {keys}: {:?}", code(&answer)));
                }
            }
            let stood = standing(&live, &page).await?;
            if stood < 2000.0 {
                failures.push(format!("{path}: the box's scroller stands at {stood}"));
            }
            let ask = field(&r#ref, Some(expecting(&page_origin)));
            look_one(&live, &env, &page, ask).await?;
            if (standing(&live, &page).await? - stood).abs() > 0.5 {
                failures.push(format!("{path}: a box in view moved the page"));
            }
            assert!(!env.was_sent(), "a look never marks itself sent");
            live.close(&page).await?;
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    Ok(())
}

/// Where one frame of ruling R60's pages comes from, on the way down from the top page, which is
/// always on `127.0.0.1` at the first server's port.
#[derive(Clone, Copy, Debug)]
enum Host {
    /// The top page's own origin.
    Page,
    /// The second server's port on the same name: another origin of the same site, so the frame
    /// shares the page's process and still cannot reach the element that holds it (the shape a
    /// host reported: a site's page framing its sign-in subdomain).
    SameSite,
    /// `localhost`: another site, in a process of its own.
    OtherSite,
    /// `[::1]`: a third site.
    ThirdSite,
}

/// The servers ruling R60's pages need: the first for the top page and the other sites, the
/// second for a frame on the top page's site but not its origin.
struct Sites {
    first: FixtureServer,
    second: FixtureServer,
}

impl Sites {
    async fn start() -> TestResult<Self> {
        Ok(Self {
            first: FixtureServer::start(private_host::pages()).await?,
            second: FixtureServer::start(private_host::pages()).await?,
        })
    }

    fn url(&self, host: Host, path: &str) -> String {
        match host {
            Host::Page => self.first.url("127.0.0.1", path),
            Host::SameSite => self.second.url("127.0.0.1", path),
            Host::OtherSite => self.first.url("localhost", path),
            Host::ThirdSite => self.first.url("[::1]", path),
        }
    }

    fn origin(&self, host: Host) -> String {
        self.url(host, "/").trim_end_matches('/').to_owned()
    }

    /// The top page with `query` (what it lays over the frame, and the frame's shape) over
    /// `frames`, outermost first, the innermost loading `inner`.
    fn page(&self, query: &str, frames: &[Host], inner: &str) -> String {
        let encoded = |url: &str| byte_serialize(url.as_bytes()).collect();
        let Some((last, middle)) = frames.split_last() else {
            return self.url(Host::Page, inner);
        };
        let mut url = self.url(*last, inner);
        for host in middle.iter().rev() {
            let frame: String = encoded(&url);
            url = self.url(
                *host,
                &format!("/private/parent-cover-middle.html?frame={frame}"),
            );
        }
        let frame: String = encoded(&url);
        self.url(
            Host::Page,
            &format!("/private/parent-cover.html?{query}&frame={frame}"),
        )
    }
}

const BOX_PAGE: &str = "/private/parent-cover-box.html";

/// The refusal's sentence, which tells a page above's cover from the box's own.
fn sentence(look: &BoxLook) -> Option<&str> {
    look.refusal.as_ref().map(|error| error.message.as_str())
}
const ABOVE_COVERED: Option<EntryCode> = Some(EntryCode::BoxCovered(CoverIn::PageAbove));

/// One of ruling R60's pages, and the look's answer there by one insertion and key by key.
struct AbovePage {
    case: &'static str,
    /// `parent-cover.html`'s query: what the top page lays over the frame, and its shape.
    query: &'static str,
    frames: &'static [Host],
    inner: &'static str,
    name: &'static str,
    inserted: Option<EntryCode>,
    keyed: Option<EntryCode>,
}

const fn above(
    case: &'static str,
    query: &'static str,
    frames: &'static [Host],
    wanted: Option<EntryCode>,
) -> AbovePage {
    AbovePage {
        case,
        query,
        frames,
        inner: BOX_PAGE,
        name: "Password",
        inserted: wanted,
        keyed: wanted,
    }
}

/// Ruling R60's rows 1 to 5, 7 and 8, and the keeper's two from real pages. Each refusal fails
/// with the pages above left unasked (`pages_above` answering `Clear`); each typed row fails
/// with a parent's hit on what holds the frame, or no hit at all, counted as a cover.
const ABOVE: &[AbovePage] = &[
    above(
        "a banner above a frame on another origin of the same site",
        "cover=banner",
        &[Host::SameSite],
        ABOVE_COVERED,
    ),
    above(
        "a banner above a cross-site frame",
        "cover=banner",
        &[Host::OtherSite],
        ABOVE_COVERED,
    ),
    above(
        "a dialog and its backdrop above a same-site frame",
        "cover=dialog",
        &[Host::SameSite],
        ABOVE_COVERED,
    ),
    above(
        "a dialog and its backdrop above a cross-site frame",
        "cover=dialog",
        &[Host::OtherSite],
        ABOVE_COVERED,
    ),
    above(
        "a grandparent's banner, both frame boundaries cross-site",
        "cover=banner",
        &[Host::OtherSite, Host::ThirdSite],
        ABOVE_COVERED,
    ),
    above(
        "a grandparent's banner over a same-origin parent the page side reaches",
        "cover=banner",
        &[Host::OtherSite, Host::OtherSite],
        ABOVE_COVERED,
    ),
    above(
        "a banner above a payment frame with an identity transform",
        "cover=banner&shape=stripe",
        &[Host::OtherSite],
        ABOVE_COVERED,
    ),
    above(
        "a sheet over a smooth-scrolling page, its same-site frame far below the fold",
        "cover=sheet&place=smooth",
        &[Host::SameSite],
        ABOVE_COVERED,
    ),
    above(
        "a sheet over a smooth-scrolling page, its cross-site frame far below the fold",
        "cover=sheet&place=smooth",
        &[Host::OtherSite],
        ABOVE_COVERED,
    ),
    above(
        "nothing over a smooth-scrolling page, its same-site frame far below the fold",
        "cover=none&place=smooth",
        &[Host::SameSite],
        None,
    ),
    above(
        "nothing over a smooth-scrolling page, its cross-site frame far below the fold",
        "cover=none&place=smooth",
        &[Host::OtherSite],
        None,
    ),
    above(
        "the frame fixed below the view, where no scroll brings it",
        "cover=none&place=pinned",
        &[Host::OtherSite],
        Some(EntryCode::CoverUnread(Unread::OutOfView)),
    ),
    above(
        "the frame clipped away by its container",
        "cover=none&place=clipped",
        &[Host::OtherSite],
        Some(EntryCode::CoverUnread(Unread::OutOfView)),
    ),
    above(
        "nothing above a same-site frame",
        "cover=none",
        &[Host::SameSite],
        None,
    ),
    above(
        "nothing above a cross-site frame",
        "cover=none",
        &[Host::OtherSite],
        None,
    ),
    above(
        "a tinted sheet above the frame that lets the pointer through",
        "cover=glass",
        &[Host::OtherSite],
        None,
    ),
    above(
        "a banner above the frame that stops short of the box",
        "cover=aside",
        &[Host::OtherSite],
        None,
    ),
    above(
        "the frame in a bordered, padded wrapper inside a scrolled box",
        "cover=none&shape=wrapper",
        &[Host::OtherSite],
        None,
    ),
    above(
        "the frame's owner inside an open shadow root",
        "cover=none&shape=shadow",
        &[Host::OtherSite],
        None,
    ),
    above(
        "a payment frame with an identity transform",
        "cover=none&shape=stripe",
        &[Host::OtherSite],
        None,
    ),
    AbovePage {
        // Passed until ruling R62: a box out of its own frame's view cannot be brought there.
        inner: "/private/parent-cover-box.html?away=1",
        ..above(
            "a box far off its own frame's viewport, as payment frames lay a helper box",
            "cover=none",
            &[Host::OtherSite],
            Some(EntryCode::CoverUnread(Unread::Unreachable)),
        )
    },
    above(
        "a frame the page draws half as large again",
        "cover=none&shape=scaled",
        &[Host::OtherSite],
        Some(EntryCode::CoverUnread(Unread::SizeOrAngle)),
    ),
    above(
        "a frame the page turns upside down, drawn at its own size",
        "cover=none&shape=turned",
        &[Host::OtherSite],
        Some(EntryCode::CoverUnread(Unread::SizeOrAngle)),
    ),
    above(
        "a frame that lets the pointer through to what holds it",
        "cover=none&shape=inert",
        &[Host::OtherSite],
        None,
    ),
    // Row 8: rulings R55 and R56 under a clear parent, in the shape a sign-in was built for.
    AbovePage {
        inner: "/private/drawn-code.html",
        name: "Code",
        inserted: Some(EntryCode::BoxCovered(CoverIn::OwnDocument)),
        ..above(
            "the code field's own drawing in a same-site frame, nothing above",
            "cover=none",
            &[Host::SameSite],
            None,
        )
    },
];

/// Ruling R60: a cover in any page above the box's frame refuses, as one in the box's own
/// document does, with its own sentence; what holds the frame, and a point outside a page's
/// viewport, are no cover; a frame scaled or turned answers `cover_unread`. A look never marks
/// itself sent.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_cover_in_a_page_above_the_frame_refuses() -> TestResult {
    let sites = Sites::start().await?;
    let live = LiveBrowser::connect().await?;
    let mut failures = Vec::new();
    for case in ABOVE {
        let Some(&inner) = case.frames.last() else {
            continue;
        };
        let page = live
            .open(&sites.page(case.query, case.frames, case.inner))
            .await?;
        let r#ref = private_host::ref_of(&live, &page, "textbox", case.name).await?;
        let env = HostEnv::new(&live);
        let origin = sites.origin(inner);
        for (keys, wanted) in [(false, case.inserted), (true, case.keyed)] {
            let ask = LookField {
                keys,
                ..field(&r#ref, Some(expecting(&origin)))
            };
            let answer = look_one(&live, &env, &page, ask).await?;
            let answered = code(&answer);
            if answered != wanted.map(EntryCode::as_str) {
                failures.push(format!(
                    "{}, keys {keys}: {answered:?} {:?}",
                    case.case,
                    sentence(&answer)
                ));
            }
            // Each code's own sentence: a page above's cover is not told as the box's own.
            if let (Some(error), Some(wanted)) = (answer.refusal.as_ref(), wanted)
                && error.message != wanted.message()
            {
                failures.push(format!("{}: the sentence {:?}", case.case, error.message));
            }
            if answer.facts.is_none() {
                failures.push(format!("{}: no facts", case.case));
            }
        }
        assert!(!env.was_sent(), "a look never marks itself sent");
        live.close(&page).await?;
    }
    assert!(failures.is_empty(), "{failures:#?}");
    Ok(())
}

/// Ruling R60's same-site frame is the shape a host reported: the frame shares the top page's
/// process, so the browser gives it no target of its own, and it still cannot reach the element
/// that holds it, so the page side's look never asks the top page. Rust's question does.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_same_site_frame_shares_the_pages_process_and_reaches_no_holder() -> TestResult
{
    let sites = Sites::start().await?;
    let live = LiveBrowser::connect().await?;
    for (host, own_target) in [(Host::SameSite, false), (Host::OtherSite, true)] {
        let inner = sites.url(host, BOX_PAGE);
        let page = live
            .open(&sites.page("cover=banner", &[host], BOX_PAGE))
            .await?;
        let r#ref = private_host::ref_of(&live, &page, "textbox", "Password").await?;
        private_host::ref_of(&live, &page, "button", "No frame element").await?;
        let targets = live
            .browser
            .cdp("Target.getTargets", json!({}), None)
            .await?;
        let listed = targets
            .get("targetInfos")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|info| {
                info.get("type").and_then(Value::as_str) == Some("iframe")
                    && info.get("url").and_then(Value::as_str) == Some(inner.as_str())
            });
        assert_eq!(listed, own_target, "{host:?}: the frame's own target");
        let env = HostEnv::new(&live);
        let answer = look_one(
            &live,
            &env,
            &page,
            field(&r#ref, Some(expecting(&sites.origin(host)))),
        )
        .await?;
        assert_eq!(
            sentence(&answer),
            ABOVE_COVERED.map(EntryCode::message),
            "{host:?}"
        );
        live.close(&page).await?;
    }
    Ok(())
}

/// Ruling R60's row 6: a sticky header of the top page over a cross-site frame's box refuses,
/// and once the page is scrolled so the frame clears the header, the same box passes. Neo
/// scrolls nothing for it: the box is in view both times.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_under_a_sticky_header_above_its_frame_passes_once_clear() -> TestResult {
    let sites = Sites::start().await?;
    let live = LiveBrowser::connect().await?;
    let page = live
        .open(&sites.page("cover=sticky", &[Host::OtherSite], BOX_PAGE))
        .await?;
    let r#ref = private_host::ref_of(&live, &page, "textbox", "Password").await?;
    let origin = sites.origin(Host::OtherSite);
    // The frame starts 640 pixels down the page, under a sticky header 120 pixels tall.
    for (scroll, wanted) in [(620, ABOVE_COVERED), (400, None)] {
        live.main_world(&page, &format!("scrollTo(0, {scroll})"))
            .await?;
        let env = HostEnv::new(&live);
        let answer = look_one(&live, &env, &page, field(&r#ref, Some(expecting(&origin)))).await?;
        assert_eq!(
            sentence(&answer),
            wanted.map(EntryCode::message),
            "scrolled to {scroll}"
        );
    }
    live.close(&page).await
}

/// Ruling R62's rows 1 to 4 and 6 for the look, which the host's check call makes: a sized box
/// that the look's instant scroll cannot bring into its own frame's view answers `cover_unread`,
/// by one insertion and key by key: a password box 3000 pixels down a page that cannot scroll,
/// with a sheet over the view and without it, a helper box parked far off the page's left edge,
/// and a 1 by 1 box just off its top-left corner. A box in view, a box below the fold of a page
/// that can scroll, and a box in view on a page in quirks mode with no scrolling element (its
/// view is the window's), pass as before; the last answers `box_covered` under a sheet. Fails with the view test taken out of the look: nothing
/// is at an out-of-view box's centre, and nothing passes.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_that_cannot_be_brought_into_view_is_refused() -> TestResult {
    let server = FixtureServer::start(private_host::pages()).await?;
    let live = LiveBrowser::connect().await?;
    let (page_origin, _) = private_host::origins(&server);
    let unreachable = Some(EntryCode::CoverUnread(Unread::Unreachable));
    let cases = [
        (
            "/private/smooth-login.html?lock=1&sheet=1",
            "#pw",
            unreachable,
        ),
        ("/private/smooth-login.html?lock=1", "#pw", unreachable),
        ("/private/out-of-view.html", "#helper", unreachable),
        ("/private/out-of-view.html", "#corner", unreachable),
        ("/private/out-of-view.html", "#email", None),
        // A page in quirks mode with no scrolling element: its view is the window's.
        ("/private/quirks-login.html", "#pw", None),
        (
            "/private/quirks-login.html?sheet=1",
            "#pw",
            Some(EntryCode::BoxCovered(CoverIn::OwnDocument)),
        ),
        ("/private/smooth-login.html?plain=1", "#pw", None),
    ];
    let mut failures = Vec::new();
    for (path, css, wanted) in cases {
        let page = live.open(&server.url("127.0.0.1", path)).await?;
        let r#ref = mint_ref(&live, &page, css).await?;
        let env = HostEnv::new(&live);
        for keys in [false, true] {
            let ask = LookField {
                keys,
                ..field(&r#ref, Some(expecting(&page_origin)))
            };
            let answer = look_one(&live, &env, &page, ask).await?;
            if sentence(&answer) != wanted.map(EntryCode::message) {
                failures.push(format!("{path} {css}, keys {keys}: {:?}", code(&answer)));
            }
        }
        assert!(!env.was_sent(), "a look never marks itself sent");
        live.close(&page).await?;
    }
    assert!(failures.is_empty(), "{failures:#?}");
    Ok(())
}

/// Ruling R62's second writing, rows 8 to 12 for the look, which the host's check call makes: a
/// box larger than its view, which the browser's scroll shows by its nearest edge or leaves
/// where it is, is brought into view by its centre and looked at again there. A text box three
/// views tall with its top showing, and one that fills the view, in the page's own document and
/// in a small frame of another site: clear, they pass and end with their centre in view; under
/// a sheet, laid in their own document or by the page above over the frame, `box_covered`. The
/// tall box on a page that cannot scroll, and a fixed box with its centre off the view's edge,
/// still answer `cover_unread`. Fails with the centre scroll taken out: the tall and filling
/// boxes answer `cover_unread`; with the second look taken out, their sheet rows in the own
/// document pass.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_larger_than_its_view_is_looked_at_by_its_centre() -> TestResult {
    let sites = Sites::start().await?;
    let live = LiveBrowser::connect().await?;
    let own = Some(EntryCode::BoxCovered(CoverIn::OwnDocument));
    let above = Some(EntryCode::BoxCovered(CoverIn::PageAbove));
    let unreachable = Some(EntryCode::CoverUnread(Unread::Unreachable));
    let inner = |query: &str| format!("/private/larger-than-view.html?{query}");
    let cases = [
        (
            sites.url(Host::Page, &inner("shape=tall")),
            Host::Page,
            None,
        ),
        (
            sites.url(Host::Page, &inner("shape=tall&sheet=1")),
            Host::Page,
            own,
        ),
        (
            sites.url(Host::Page, &inner("shape=fills")),
            Host::Page,
            None,
        ),
        (
            sites.url(Host::Page, &inner("shape=fills&sheet=1")),
            Host::Page,
            own,
        ),
        (
            sites.url(Host::Page, &inner("shape=tall&lock=1")),
            Host::Page,
            unreachable,
        ),
        (
            sites.url(Host::Page, &inner("shape=fixed")),
            Host::Page,
            unreachable,
        ),
        (
            sites.page("cover=none", &[Host::OtherSite], &inner("shape=tall")),
            Host::OtherSite,
            None,
        ),
        (
            sites.page("cover=sheet", &[Host::OtherSite], &inner("shape=tall")),
            Host::OtherSite,
            above,
        ),
        (
            sites.page("cover=none", &[Host::OtherSite], &inner("shape=fills")),
            Host::OtherSite,
            None,
        ),
        (
            sites.page("cover=sheet", &[Host::OtherSite], &inner("shape=fills")),
            Host::OtherSite,
            above,
        ),
    ];
    let mut failures = Vec::new();
    for (url, host, wanted) in cases {
        let page = live.open(&url).await?;
        let r#ref = private_host::ref_of(&live, &page, "textbox", "Secret").await?;
        let env = HostEnv::new(&live);
        for keys in [false, true] {
            let ask = LookField {
                keys,
                ..field(&r#ref, Some(expecting(&sites.origin(host))))
            };
            let answer = look_one(&live, &env, &page, ask).await?;
            if sentence(&answer) != wanted.map(EntryCode::message) {
                failures.push(format!("{url}, keys {keys}: {:?}", code(&answer)));
            }
        }
        // The page is left where the centre scroll put it.
        if wanted.is_none()
            && matches!(host, Host::Page)
            && live.main_world(&page, "centreInView()").await? != json!(true)
        {
            failures.push(format!(
                "{url}: the box's centre is not in view after the look"
            ));
        }
        assert!(!env.was_sent(), "a look never marks itself sent");
        live.close(&page).await?;
    }
    assert!(failures.is_empty(), "{failures:#?}");
    Ok(())
}
