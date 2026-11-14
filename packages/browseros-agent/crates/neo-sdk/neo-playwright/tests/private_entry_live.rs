//! The private entry (protected-fill design sections 5.4 to 5.6) through
//! `Playwright::enter_privately`, against a real browser: a sign-in, a covered box, values a box cuts,
//! boxes that submit, a box replaced mid-entry, one character per box, boxes in cross-site
//! frames, focus thieves, a lock, a deadline, and a page that mirrors the value into an
//! attribute. Each case where the tab's place matters runs on a tab in front and a tab behind.
//!
//! The live tests need BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005):
//! `cargo test -p neo-playwright --test private_entry_live -- --ignored`.

#[path = "support/fixture_server.rs"]
#[expect(
    dead_code,
    reason = "shared with suites that serve scripts and the nested-frame fixture"
)]
mod fixture_server;
#[path = "support/input_recording.rs"]
mod input_recording;
#[path = "support/live_browser.rs"]
#[expect(
    dead_code,
    reason = "shared with suites that run ops in a run of their own"
)]
mod live_browser;
#[path = "support/private_host.rs"]
mod private_host;

use std::iter;
use std::sync::Arc;
use std::time::Duration;

use browseros_core::{ProtocolSession, TabId};
use fixture_server::{FixtureServer, Page};
use input_recording::{Failing, InputCall, InputRecording};
use live_browser::{LiveBrowser, TestPage, TestResult};
use neo_playwright::{
    BoxLook, BoxSpec, CoverIn, Drawing, Entry, EntryCode, EntryOutcome, EntryRequest, Expect,
    LookCompare, LookField, LookRequest, Op, PictureCheck, PrivateField, PrivateText, RunState,
    Unread,
};
use neo_trace::{Delivery, ErrorOwner};
use private_host::HostEnv;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, SemaphorePermit, oneshot};
use tokio::time::{self, Instant};
use url::form_urlencoded::byte_serialize;

/// A tab in front of the window, or behind another tab.
#[derive(Clone, Copy, Debug)]
enum Place {
    Front,
    Behind,
}

const PLACES: [Place; 2] = [Place::Front, Place::Behind];

/// One test at a time: which tab is in front is the browser's, and another test's new tab would
/// change it in the middle of an entry.
static ONE_AT_A_TIME: Semaphore = Semaphore::const_new(1);

/// One live browser and the fixture server, held by one test at a time.
struct Bench {
    server: FixtureServer,
    live: LiveBrowser,
    /// Every `Input.*` call on the live browser's connection, with its session (ruling R24).
    recording: Arc<InputRecording>,
    page_origin: String,
    other_origin: String,
    _turn: SemaphorePermit<'static>,
}

/// A fixture page opened in `place`, with the tab that covers it when it is behind.
struct Opened {
    page: TestPage,
    cover: Option<TestPage>,
    place: Place,
}

impl Bench {
    async fn start() -> TestResult<Self> {
        let turn = ONE_AT_A_TIME.acquire().await?;
        let mut pages = private_host::pages();
        pages.insert(
            "/private/moved-window.html".to_owned(),
            Page::html(include_str!("data/private/moved-window.html"))
                .with_header("cache-control", "no-store"),
        );
        let server = FixtureServer::start(pages).await?;
        let recording = InputRecording::over(LiveBrowser::cdp_client().await?);
        let live = LiveBrowser::connect_on(Arc::clone(&recording) as _).await?;
        let (page_origin, other_origin) = private_host::origins(&server);
        Ok(Self {
            server,
            live,
            recording,
            page_origin,
            other_origin,
            _turn: turn,
        })
    }

    async fn open(&self, path: &str, place: Place) -> TestResult<Opened> {
        let page = self.live.open(&self.server.url("127.0.0.1", path)).await?;
        let cover = match place {
            Place::Front => None,
            Place::Behind => Some(
                self.live
                    .open(&self.server.url("127.0.0.1", "/private/look.html"))
                    .await?,
            ),
        };
        Ok(Opened { page, cover, place })
    }

    async fn close(&self, opened: Opened) -> TestResult {
        if let Some(cover) = opened.cover {
            self.live.close(&cover).await?;
        }
        self.live.close(&opened.page).await
    }

    /// The host for `opened`: it brings a tab in front forward, as the server does, and
    /// leaves a tab behind where it is.
    fn host(&self, opened: &Opened) -> HostEnv {
        let host = HostEnv::new(&self.live);
        match opened.place {
            Place::Front => host.in_front(),
            Place::Behind => host,
        }
    }

    async fn r#ref(&self, opened: &Opened, role: &str, name: &str) -> TestResult<String> {
        private_host::ref_of(&self.live, &opened.page, role, name).await
    }

    /// The entry, after checking that the tab is where the case says: in front (visible), or
    /// behind another tab (hidden).
    async fn enter(&self, env: &HostEnv, opened: &Opened, fields: Vec<PrivateField>) -> Entry {
        let visibility = self
            .live
            .main_world(&opened.page, "document.visibilityState")
            .await
            .ok();
        let wanted = match opened.place {
            Place::Front => "visible",
            Place::Behind => "hidden",
        };
        assert_eq!(
            visibility.as_ref().and_then(Value::as_str),
            Some(wanted),
            "{:?}: the tab is not where the case says",
            opened.place
        );
        let request = EntryRequest {
            tab: opened.page.tab,
            url: None,
            fields,
            deadline: Instant::now() + Duration::from_secs(15),
        };
        self.live.playwright.enter_privately(env, request).await
    }

    /// `page.locator(css).ref()`: the node-bound ref a host's check script mints for a box the
    /// snapshot prints no ref for, such as one with no size (`n<k>`).
    async fn mint(&self, opened: &Opened, css: &str) -> TestResult<String> {
        let part = json!({
            "name": "css",
            "body": [{ "simples": [{ "selector": { "css": css, "functions": [] }, "combinator": "" }] }],
            "source": css,
        });
        let target = json!({
            "$neo": "locator", "page": opened.page.brand(),
            "chunks": [{ "parts": [part] }],
            "display": format!("locator('{css}')"),
        });
        let input = json!({
            "call": { "api": "locator.ref", "timeoutMs": 5_000 },
            "target": target, "strict": "strict",
        });
        let minted = self.live.invoke("locator.ref", input).await??;
        Ok(minted
            .as_str()
            .ok_or("locator.ref answers a string")?
            .to_owned())
    }

    /// The box values the page saw at its input events (stale.html's `window.seen`).
    async fn seen(&self, opened: &Opened) -> TestResult<Vec<String>> {
        let seen = self
            .live
            .main_world(&opened.page, "JSON.stringify(window.seen)")
            .await?;
        Ok(serde_json::from_str(seen.as_str().unwrap_or("[]"))?)
    }

    /// What the box `r#ref` holds now, read in its document's main world.
    async fn value(&self, opened: &Opened, r#ref: &str) -> TestResult<String> {
        let target = json!({
            "$neo": "locator", "page": opened.page.brand(),
            "chunks": [{ "parts": [{ "name": "aria-ref", "body": r#ref, "source": r#ref }] }],
            "display": format!("getByRef('{ref}')"),
        });
        let input = json!({
            "call": { "api": "locator.evaluate", "timeoutMs": 5_000 },
            "target": target, "strict": "strict",
            "expression": "(el) => el.value", "isFunction": true,
            "arg": { "value": r#"{"v":"undefined"}"#, "handles": [] },
        });
        let value = self.live.invoke("locator.evaluate", input).await??;
        Ok(value.as_str().unwrap_or_default().to_owned())
    }
}

/// A field the host approved on `origin`.
struct Field {
    r#ref: String,
    origin: String,
    max_length: Option<i32>,
    name: Option<String>,
    cut: bool,
    spaced: bool,
    keys: bool,
}

impl Field {
    fn new(r#ref: &str, origin: &str) -> Self {
        Self {
            r#ref: r#ref.to_owned(),
            origin: origin.to_owned(),
            max_length: None,
            name: None,
            cut: false,
            spaced: false,
            keys: false,
        }
    }

    fn named(mut self, name: &str) -> Self {
        self.name = Some(name.to_owned());
        self
    }

    fn keys(mut self) -> Self {
        self.keys = true;
        self
    }

    fn cut(mut self) -> Self {
        self.cut = true;
        self
    }

    fn max_length(mut self, max_length: i32) -> Self {
        self.max_length = Some(max_length);
        self
    }

    fn spaced(mut self) -> Self {
        self.spaced = true;
        self
    }

    fn with(self, value: &str) -> TestResult<PrivateField> {
        Ok(PrivateField {
            spec: BoxSpec {
                box_ref: self.r#ref,
                expect: Expect {
                    origin: self.origin,
                    max_length: self.max_length,
                    name: self.name,
                    ..Expect::default()
                },
                cut: self.cut,
                spaced: self.spaced,
                keys: self.keys,
            },
            value: PrivateText::new(value.to_owned())?,
        })
    }
}

fn code(entry: &Entry) -> Option<&str> {
    entry.error.as_ref().map(|error| error.code.as_str())
}

const EMAIL: &str = "ada@fixture.example";
const PASSWORD: &str = "CANARY-covered-pw-1";
/// A one-time code: six digits, which a code field's own drawing may show as they are typed.
const CODE: &str = "481516";
const CARD: &str = "4000056655665556";

/// A sign-in's two boxes, typed in one insertion each and read back, on a tab in
/// front and a tab behind. The tab is marked for the camera; the door was asked and told.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_sign_in_is_entered_and_read_back() -> TestResult {
    let bench = Bench::start().await?;
    for place in PLACES {
        let opened = bench
            .open("/private/covered-login.html?mode=none", place)
            .await?;
        let email = bench.r#ref(&opened, "textbox", "Email").await?;
        let password = bench.r#ref(&opened, "textbox", "Password").await?;
        let env = bench.host(&opened);
        let origin = &bench.page_origin;
        assert!(!bench.live.playwright.took_private_text(opened.page.tab));
        let fields = vec![
            Field::new(&email, origin).with(EMAIL)?,
            Field::new(&password, origin).with(PASSWORD)?,
        ];
        let entry = bench.enter(&env, &opened, fields).await;
        assert_eq!(entry.outcome, EntryOutcome::Entered, "{place:?}: {entry:?}");
        assert_eq!(entry.delivery, Delivery::Done, "{place:?}");
        assert_eq!(entry.typed, vec![EMAIL.len(), PASSWORD.len()], "{place:?}");
        assert_eq!(bench.value(&opened, &email).await?, EMAIL, "{place:?}");
        assert_eq!(
            bench.value(&opened, &password).await?,
            PASSWORD,
            "{place:?}"
        );
        assert!(env.was_sent(), "{place:?}: the entry marks itself sent");
        assert!(
            bench.live.playwright.took_private_text(opened.page.tab),
            "{place:?}"
        );
        bench.close(opened).await?;
    }
    Ok(())
}

/// A password box under a cookie banner refuses the whole sign-in before the username is
/// typed; nothing was sent, so the refusal is retryable and the tab is not marked.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_covered_box_refuses_the_form_before_any_key() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/covered-login.html?mode=banner", Place::Front)
        .await?;
    let email = bench.r#ref(&opened, "textbox", "Email").await?;
    let password = bench.r#ref(&opened, "textbox", "Password").await?;
    let env = bench.host(&opened);
    let origin = &bench.page_origin;
    let fields = vec![
        Field::new(&email, origin).with(EMAIL)?,
        Field::new(&password, origin).with(PASSWORD)?,
    ];
    let entry = bench.enter(&env, &opened, fields).await;
    assert_eq!(entry.outcome, EntryOutcome::Refused, "{entry:?}");
    assert_eq!((entry.at, code(&entry)), (Some(1), Some("box_covered")));
    assert_eq!(entry.delivery, Delivery::NotSent);
    assert_eq!(entry.error.map(|error| error.retryable), Some(true));
    assert_eq!(entry.typed, vec![0, 0]);
    assert_eq!(
        bench.value(&opened, &email).await?,
        "",
        "the username is not typed either"
    );
    assert!(!env.was_sent());
    assert!(!bench.live.playwright.took_private_text(opened.page.tab));
    bench.close(opened).await
}

/// Ruling R61's rows 1, 2, 3 and 6 for the fill: a password box far below the fold of a page
/// that scrolls smoothly, on its root or on a container between the box and the root, or in a
/// frame of the page's own origin. Under a sheet over the whole view the entry is refused
/// `box_covered` before any key, nothing typed; without it the box is entered and read back. By
/// one insertion and key by key, on a tab in front and a tab behind, and the box's scroller ends
/// at the box. Fails with the look's instant scroll left out: the page's own scroll animates,
/// its hit finds nothing at the box's place, and the box is typed under the sheet.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_below_the_fold_of_a_smooth_page_is_entered_only_in_view() -> TestResult {
    let bench = Bench::start().await?;
    let origin = &bench.page_origin;
    for shape in ["", "&scroller=1", "&frame=1"] {
        for place in PLACES {
            for keys in [false, true] {
                for sheet in [true, false] {
                    let path = format!(
                        "/private/smooth-login.html?sheet={}{shape}",
                        if sheet { "1" } else { "" }
                    );
                    let case = format!("{path}, {place:?}, keys {keys}");
                    let opened = bench.open(&path, place).await?;
                    let password = bench.r#ref(&opened, "textbox", "Password").await?;
                    let env = bench.host(&opened);
                    let field = Field::new(&password, origin);
                    let field = if keys { field.keys() } else { field };
                    let entry = bench
                        .enter(&env, &opened, vec![field.with(PASSWORD)?])
                        .await;
                    if sheet {
                        assert_eq!(entry.outcome, EntryOutcome::Refused, "{case}: {entry:?}");
                        assert_eq!(code(&entry), Some("box_covered"), "{case}");
                        assert_eq!(entry.typed, vec![0], "{case}");
                        assert_eq!(bench.value(&opened, &password).await?, "", "{case}");
                        assert!(!env.was_sent(), "{case}");
                    } else {
                        assert_eq!(entry.outcome, EntryOutcome::Entered, "{case}: {entry:?}");
                        assert_eq!(bench.value(&opened, &password).await?, PASSWORD, "{case}");
                    }
                    let stood = bench
                        .live
                        .main_world(&opened.page, "standing()")
                        .await?
                        .as_f64()
                        .unwrap_or_default();
                    assert!(
                        stood > 2000.0,
                        "{case}: the box's scroller stands at {stood}"
                    );
                    bench.close(opened).await?;
                }
            }
        }
    }
    Ok(())
}

/// Ruling R60's rows 1 and 2: a banner in the top page over a sign-in frame's password box, the
/// frame on another origin of the same site (sharing the page's process) and on another site.
/// Refused `box_covered` with the page above's own sentence before any character, by one
/// insertion and key by key, on a tab in front and a tab behind: nothing typed, nothing marked
/// sent. Fails with the pages above left unasked (the password is typed).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_banner_above_a_sign_in_frame_refuses_before_any_key() -> TestResult {
    let bench = Bench::start().await?;
    let second = FixtureServer::start(private_host::pages()).await?;
    let inner = "/private/parent-cover-box.html";
    let same_site = second
        .url("127.0.0.1", "/")
        .trim_end_matches('/')
        .to_owned();
    let frames = [
        (second.url("127.0.0.1", inner), same_site),
        (
            bench.server.url("localhost", inner),
            bench.other_origin.clone(),
        ),
    ];
    let above = EntryCode::BoxCovered(CoverIn::PageAbove);
    // Each frame near the top under a banner, and far below the fold of a smooth-scrolling page
    // under a sheet over the whole view (ruling R61 for pages above).
    for (frame, origin) in &frames {
        let frame: String = byte_serialize(frame.as_bytes()).collect();
        for (query, place) in [
            ("cover=banner", Place::Front),
            ("cover=banner", Place::Behind),
            ("cover=sheet&place=smooth", Place::Front),
            ("cover=sheet&place=smooth", Place::Behind),
        ] {
            let path = format!("/private/parent-cover.html?{query}&frame={frame}");
            for keys in [false, true] {
                let case = format!("{origin}, {query}, {place:?}, keys {keys}");
                let opened = bench.open(&path, place).await?;
                let password = bench.r#ref(&opened, "textbox", "Password").await?;
                let env = bench.host(&opened);
                let field = Field::new(&password, origin);
                let field = if keys { field.keys() } else { field };
                let entry = bench
                    .enter(&env, &opened, vec![field.with(PASSWORD)?])
                    .await;
                assert_eq!(entry.outcome, EntryOutcome::Refused, "{case}: {entry:?}");
                assert_eq!(
                    (entry.at, code(&entry)),
                    (Some(0), Some(above.as_str())),
                    "{case}"
                );
                assert_eq!(
                    entry.error.as_ref().map(|error| error.message.as_str()),
                    Some(above.message()),
                    "{case}"
                );
                assert_eq!(entry.delivery, Delivery::NotSent, "{case}");
                assert_eq!(entry.typed, vec![0], "{case}");
                assert_eq!(bench.value(&opened, &password).await?, "", "{case}");
                assert!(!env.was_sent(), "{case}");
                assert!(
                    !bench.live.playwright.took_private_text(opened.page.tab),
                    "{case}"
                );
                bench.close(opened).await?;
            }
        }
    }
    Ok(())
}

/// Ruling R60 with R61: a sign-in frame far below the fold of a page whose root scrolls smoothly,
/// nothing over it, on another origin of the same site and on another site. The box is brought
/// into view at once in the frame and in the page above, the page above is asked, and the box is
/// entered and read back, on a tab in front and a tab behind. Fails when a page above that has
/// not scrolled the frame into view counts as unseen (`cover_unread`).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_sign_in_frame_below_a_smooth_pages_fold_is_entered() -> TestResult {
    let bench = Bench::start().await?;
    let second = FixtureServer::start(private_host::pages()).await?;
    let inner = "/private/parent-cover-box.html";
    let same_site = second
        .url("127.0.0.1", "/")
        .trim_end_matches('/')
        .to_owned();
    let frames = [
        (second.url("127.0.0.1", inner), same_site),
        (
            bench.server.url("localhost", inner),
            bench.other_origin.clone(),
        ),
    ];
    for (frame, origin) in &frames {
        let frame: String = byte_serialize(frame.as_bytes()).collect();
        let path = format!("/private/parent-cover.html?cover=none&place=smooth&frame={frame}");
        for place in PLACES {
            let case = format!("{origin}, {place:?}");
            let opened = bench.open(&path, place).await?;
            let password = bench.r#ref(&opened, "textbox", "Password").await?;
            let env = bench.host(&opened);
            let field = Field::new(&password, origin).with(PASSWORD)?;
            let entry = bench.enter(&env, &opened, vec![field]).await;
            assert_eq!(entry.outcome, EntryOutcome::Entered, "{case}: {entry:?}");
            assert_eq!(bench.value(&opened, &password).await?, PASSWORD, "{case}");
            bench.close(opened).await?;
        }
    }
    Ok(())
}

/// Ruling R62's rows 1 and 2 for the fill: a password box 3000 pixels down a page that cannot
/// scroll (a dialog's scroll lock by a fixed body), with a sheet over the view and without it.
/// The look's scroll cannot bring it into view, so the entry is refused `cover_unread` with its
/// sentence before any key: nothing typed, nothing sent. By one insertion and key by key, on a
/// tab in front and a tab behind. Fails with the view test taken out of the look: the box is
/// typed under the sheet, and without it.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_on_a_page_that_cannot_scroll_is_refused_before_any_key() -> TestResult {
    let bench = Bench::start().await?;
    let origin = &bench.page_origin;
    let unreachable = EntryCode::CoverUnread(Unread::Unreachable);
    for path in [
        "/private/smooth-login.html?lock=1&sheet=1",
        "/private/smooth-login.html?lock=1",
    ] {
        for place in PLACES {
            for keys in [false, true] {
                let case = format!("{path}, {place:?}, keys {keys}");
                let opened = bench.open(path, place).await?;
                let password = bench.r#ref(&opened, "textbox", "Password").await?;
                let env = bench.host(&opened);
                let field = Field::new(&password, origin);
                let field = if keys { field.keys() } else { field };
                let entry = bench
                    .enter(&env, &opened, vec![field.with(PASSWORD)?])
                    .await;
                assert_eq!(entry.outcome, EntryOutcome::Refused, "{case}: {entry:?}");
                assert_eq!(
                    entry.error.as_ref().map(|error| error.message.as_str()),
                    Some(unreachable.message()),
                    "{case}"
                );
                assert_eq!(entry.typed, vec![0], "{case}");
                assert_eq!(bench.value(&opened, &password).await?, "", "{case}");
                assert!(!env.was_sent(), "{case}");
                bench.close(opened).await?;
            }
        }
    }
    Ok(())
}

/// Ruling R62 on a page in quirks mode whose html and body both set overflow, so it has no
/// scrolling element: a password box in view is entered, by one insertion and key by key, front
/// and behind, and under a sheet over the view refused `box_covered` before any key. Fails when
/// the view test reads only the scrolling element: every sized box on such a page answers
/// `cover_unread`.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_on_a_quirks_page_with_no_scroller_is_entered() -> TestResult {
    let bench = Bench::start().await?;
    let origin = &bench.page_origin;
    for (path, sheet) in [
        ("/private/quirks-login.html", false),
        ("/private/quirks-login.html?sheet=1", true),
    ] {
        for place in PLACES {
            for keys in [false, true] {
                let case = format!("{path}, {place:?}, keys {keys}");
                let opened = bench.open(path, place).await?;
                let password = bench.r#ref(&opened, "textbox", "Password").await?;
                let env = bench.host(&opened);
                let field = Field::new(&password, origin);
                let field = if keys { field.keys() } else { field };
                let entry = bench
                    .enter(&env, &opened, vec![field.with(PASSWORD)?])
                    .await;
                if sheet {
                    assert_eq!(entry.outcome, EntryOutcome::Refused, "{case}: {entry:?}");
                    assert_eq!(code(&entry), Some("box_covered"), "{case}");
                    assert_eq!(bench.value(&opened, &password).await?, "", "{case}");
                } else {
                    assert_eq!(entry.outcome, EntryOutcome::Entered, "{case}: {entry:?}");
                    assert_eq!(bench.value(&opened, &password).await?, PASSWORD, "{case}");
                }
                bench.close(opened).await?;
            }
        }
    }
    Ok(())
}

/// Ruling R62's second writing, rows 8 to 11 for the fill: a text box three views tall with its
/// top showing, and one that fills the view, are brought into view by their centre and looked at
/// again there. In the page's own document, by one insertion and key by key, on a tab in front
/// and a tab behind: clear, entered and read back, the box's centre in view when the fill ends;
/// under a sheet over the view, refused `box_covered` before any key. In a small frame of
/// another site: clear, entered; a sheet laid by the page above over the frame, refused. Row 14:
/// a box two views wide whose centre lies under a scrollbar the page draws itself, the same two
/// answers. Row 15: on a page that replaces `scrollIntoView`, `scrollIntoViewIfNeeded`,
/// `scrollTo`, `scrollBy` and `focus` in its own world, the tall box is entered and the page
/// counts no call: Neo scrolls and focuses outside the page's world. Fails
/// with the centre scroll taken out: each is refused `cover_unread`.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_larger_than_its_view_is_entered_by_its_centre() -> TestResult {
    const SECRET: &str = "CANARY-larger-than-view-8";
    let bench = Bench::start().await?;
    let shapes = ["shape=tall", "shape=fills"];
    for shape in shapes.into_iter().chain(["shape=strip&bar=1"]) {
        for sheet in [false, true] {
            let path = format!(
                "/private/larger-than-view.html?{shape}{}",
                if sheet { "&sheet=1" } else { "" }
            );
            for place in PLACES {
                for keys in [false, true] {
                    let case = format!("{path}, {place:?}, keys {keys}");
                    let opened = bench.open(&path, place).await?;
                    let secret = bench.r#ref(&opened, "textbox", "Secret").await?;
                    let env = bench.host(&opened);
                    let field = Field::new(&secret, &bench.page_origin);
                    let field = if keys { field.keys() } else { field };
                    let entry = bench.enter(&env, &opened, vec![field.with(SECRET)?]).await;
                    if sheet {
                        assert_eq!(entry.outcome, EntryOutcome::Refused, "{case}: {entry:?}");
                        assert_eq!(code(&entry), Some("box_covered"), "{case}");
                        assert_eq!(bench.value(&opened, &secret).await?, "", "{case}");
                    } else {
                        assert_eq!(entry.outcome, EntryOutcome::Entered, "{case}: {entry:?}");
                        assert_eq!(bench.value(&opened, &secret).await?, SECRET, "{case}");
                        let centred = bench
                            .live
                            .main_world(&opened.page, "centreInView()")
                            .await?;
                        assert_eq!(
                            centred,
                            json!(true),
                            "{case}: the centre in view at the end"
                        );
                    }
                    bench.close(opened).await?;
                }
            }
        }
    }
    // Both scrolls run where the page cannot see them: R61's is the browser's own call, and the
    // second step's runs in Neo's isolated world. A page that replaces both scroll methods in
    // its own world counts no call while its tall box is entered.
    let path = "/private/larger-than-view.html?shape=tall&spy=1";
    for place in PLACES {
        let opened = bench.open(path, place).await?;
        let secret = bench.r#ref(&opened, "textbox", "Secret").await?;
        let env = bench.host(&opened);
        let field = Field::new(&secret, &bench.page_origin).with(SECRET)?;
        let entry = bench.enter(&env, &opened, vec![field]).await;
        assert_eq!(
            entry.outcome,
            EntryOutcome::Entered,
            "{place:?} spy: {entry:?}"
        );
        assert_eq!(
            bench
                .live
                .main_world(&opened.page, "centreInView()")
                .await?,
            json!(true),
            "{place:?} spy: the centre in view at the end"
        );
        assert_eq!(
            bench.live.main_world(&opened.page, "scrollCalls").await?,
            json!(0),
            "{place:?}: the page's own scroll methods were called"
        );
        bench.close(opened).await?;
    }
    for shape in shapes {
        let inner = bench.server.url(
            "localhost",
            &format!("/private/larger-than-view.html?{shape}"),
        );
        let frame: String = byte_serialize(inner.as_bytes()).collect();
        for (cover, entered) in [("none", true), ("sheet", false)] {
            let path = format!("/private/parent-cover.html?cover={cover}&frame={frame}");
            for place in PLACES {
                let case = format!("{shape} in a frame, cover {cover}, {place:?}");
                let opened = bench.open(&path, place).await?;
                let secret = bench.r#ref(&opened, "textbox", "Secret").await?;
                let env = bench.host(&opened);
                let field = Field::new(&secret, &bench.other_origin).with(SECRET)?;
                let entry = bench.enter(&env, &opened, vec![field]).await;
                if entered {
                    assert_eq!(entry.outcome, EntryOutcome::Entered, "{case}: {entry:?}");
                    assert_eq!(bench.value(&opened, &secret).await?, SECRET, "{case}");
                } else {
                    assert_eq!(entry.outcome, EntryOutcome::Refused, "{case}: {entry:?}");
                    assert_eq!(code(&entry), Some("box_covered"), "{case}");
                    assert_eq!(bench.value(&opened, &secret).await?, "", "{case}");
                }
                bench.close(opened).await?;
            }
        }
    }
    Ok(())
}

/// Ruling R49's row 9 for the fill: with `expect.maxLength` of -1, a box with no `maxlength` is
/// entered and read back, and a box with one is refused `box_changed` before any key, nothing
/// typed. Fails with the -1 case taken out of the page side's compare.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_max_length_minus_one_fills_only_a_box_with_none() -> TestResult {
    let bench = Bench::start().await?;
    let origin = &bench.page_origin;
    for (name, entered) in [("Note", true), ("Code", false)] {
        let opened = bench.open("/private/maxlength.html", Place::Front).await?;
        let r#ref = bench.r#ref(&opened, "textbox", name).await?;
        let env = bench.host(&opened);
        let field = Field::new(&r#ref, origin).max_length(-1);
        let entry = bench
            .enter(&env, &opened, vec![field.with("CANARY-49")?])
            .await;
        if entered {
            assert_eq!(entry.outcome, EntryOutcome::Entered, "{name}: {entry:?}");
            assert_eq!(bench.value(&opened, &r#ref).await?, "CANARY-49", "{name}");
        } else {
            assert_eq!(entry.outcome, EntryOutcome::Refused, "{name}: {entry:?}");
            assert_eq!(code(&entry), Some("box_changed"), "{name}");
            assert_eq!(bench.value(&opened, &r#ref).await?, "", "{name}");
        }
        bench.close(opened).await?;
    }
    Ok(())
}

/// The card value of ruling R50's rows, and its bullets.
const FOLLOWED: &str = "4242424242424242";

/// What Neo's `inputValue()` answers for the box `r#ref` names: a read through the mask.
async fn masked_value(bench: &Bench, opened: &Opened, r#ref: &str) -> TestResult<String> {
    let target = ref_target(&opened.page.brand(), r#ref);
    let read = bench
        .live
        .invoke("locator.inputValue", input_value(&target))
        .await??;
    Ok(read.as_str().unwrap_or_default().to_owned())
}

/// Ruling R50's row 18: after a private fill the page puts two new elements in the box's place,
/// each holding the value and mirroring it into its `value` attribute, as a framework does for a
/// controlled input. A click on a locator that finds both fails strict mode, with no read through
/// the mask before it: its message and call log preview both as `value="<16 characters>"`, never
/// the value. Main prints the value there, with or without a snapshot between (measured by the
/// design's keeper on main 579b45ccf). Fails with the scan run before the five reads alone: no
/// read came between, so the preview prints the value.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_an_error_preview_masks_a_box_the_page_re_created() -> TestResult {
    let bench = Bench::start().await?;
    let marker = format!("value=\"<{} characters>\"", FOLLOWED.len());
    for conformance in [false, true] {
        let opened = bench
            .open("/private/mark-follows.html", Place::Front)
            .await?;
        let card = bench.r#ref(&opened, "textbox", "Card").await?;
        let env = bench.host(&opened);
        let field = Field::new(&card, &bench.page_origin).with(FOLLOWED)?;
        let entry = bench.enter(&env, &opened, vec![field]).await;
        assert_eq!(entry.outcome, EntryOutcome::Entered, "{entry:?}");
        bench
            .live
            .main_world(&opened.page, "recreateMirrored()")
            .await?;
        let mirrors = json!({
            "$neo": "locator", "page": opened.page.brand(),
            "chunks": [{ "parts": [{ "name": "css", "body": [{ "simples": [{ "selector": { "css": "input.mirror", "functions": [] }, "combinator": "" }] }], "source": "input.mirror" }] }],
            "display": "locator('input.mirror')",
        });
        let input = json!({
            "call": { "api": "locator.click", "timeoutMs": 700, "conformance": conformance },
            "target": mirrors, "strict": "strict",
        });
        let failure = preview_failure(&bench, &opened, "locator.click", input).await?;
        assert!(
            failure.message.contains("strict mode violation"),
            "{}",
            failure.message
        );
        assert!(failure.message.contains(&marker), "{}", failure.message);
        for text in iter::once(&failure.message).chain(&failure.notes) {
            assert!(
                !text.contains(FOLLOWED),
                "the preview printed the value: {text}"
            );
        }
        bench.close(opened).await?;
    }
    Ok(())
}

/// Ruling R50's row 13c, modelled on a real pin widget: a code given to its first box is spread
/// one character to each of four boxes, and the whole code is mirrored into an input the widget
/// submits. The read-back finds the box holding one character, so the fill is not entered; the
/// digest of the value the box was given masks the mirror, in an element's read and in the
/// whole-page snapshot. Another box prints its one character: a named limit, and the contract
/// tells a host to fill such a widget one box at a time. Fails with no digest kept of what the
/// box was given: the mirror prints the code.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_mark_follows_a_code_a_widget_mirrors() -> TestResult {
    const CODE: &str = "7391";
    let bench = Bench::start().await?;
    let opened = bench.open("/private/pin-mirror.html", Place::Front).await?;
    let first = bench.r#ref(&opened, "textbox", "Digit 1").await?;
    let env = bench.host(&opened);
    let field = Field::new(&first, &bench.page_origin).with(CODE)?;
    let entry = bench.enter(&env, &opened, vec![field]).await;
    assert_ne!(
        entry.outcome,
        EntryOutcome::Entered,
        "the widget spread it: {entry:?}"
    );
    let whole = bench.r#ref(&opened, "textbox", "Whole code").await?;
    assert_eq!(
        masked_value(&bench, &opened, &whole).await?,
        "••••",
        "the mirror"
    );
    assert_eq!(
        masked_value(&bench, &opened, &first).await?,
        "•",
        "the box the code was given to"
    );
    let second = bench.r#ref(&opened, "textbox", "Digit 2").await?;
    assert_eq!(
        masked_value(&bench, &opened, &second).await?,
        "3",
        "the named limit: one character in another box"
    );
    let page = &opened.page;
    let printed = bench
        .live
        .invoke("page.ariaSnapshot", page_snapshot(&page.brand(), false))
        .await??
        .to_string();
    assert!(
        !printed.contains(CODE),
        "the snapshot prints no code: {printed}"
    );
    bench.close(opened).await
}

/// Ruling R50's row 13b: the page moves a part of the text, all but its first character, into
/// another box and focuses it, inside the box's input event, so the read-back finds the box empty
/// and marks the control that holds the focus (markFocused). That control reads as bullets, as on
/// main, and so does a clone the page makes of it afterwards: markFocused keeps a digest of what
/// the control holds. The part is not the value the box was given, so that digest alone masks
/// the clone. Fails with the digest left out of markFocused (the clone prints the part).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_mark_follows_the_value_into_a_clone_of_the_focused_control() -> TestResult {
    let bench = Bench::start().await?;
    // The page moves all but the first character.
    let bullets = "•".repeat(FOLLOWED.len() - 1);
    let opened = bench.open("/private/mark-moves.html", Place::Front).await?;
    let mover = bench.r#ref(&opened, "textbox", "Mover").await?;
    let catch = bench.r#ref(&opened, "textbox", "Catch").await?;
    let env = bench.host(&opened);
    let field = Field::new(&mover, &bench.page_origin).with(FOLLOWED)?;
    let entry = bench.enter(&env, &opened, vec![field]).await;
    assert_ne!(
        entry.outcome,
        EntryOutcome::Entered,
        "the text moved: {entry:?}"
    );
    assert_eq!(
        masked_value(&bench, &opened, &catch).await?,
        bullets,
        "the control that holds the focus"
    );
    bench.live.main_world(&opened.page, "cloneCatch()").await?;
    let twin = bench.r#ref(&opened, "textbox", "Caught twin").await?;
    assert_eq!(
        masked_value(&bench, &opened, &twin).await?,
        bullets,
        "a clone of it"
    );
    bench.close(opened).await
}

/// Ruling R50's rows 11, 12, 13, 15 and 16: the mark follows the value. After a private fill of
/// the Card box, every text control of the document that holds the value reads as bullets
/// through Neo's reads, though only the Card box carries the mark: a second box the page copies
/// it into while the Card box still holds it (no new power: the page holds it), a clone, a copy
/// the page groups with spaces, and a new element the page puts in the Card box's place, also in
/// a whole-page snapshot, and `toHaveValue` compares it against the bullets. After a reload, a
/// new document keeps no digest: a box set to the value reads it. Fails with the digest compare
/// taken out of the mask: each copy reads the value.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_mark_follows_the_value_into_copies() -> TestResult {
    let bench = Bench::start().await?;
    let bullets = "•".repeat(FOLLOWED.len());
    let opened = bench
        .open("/private/mark-follows.html", Place::Front)
        .await?;
    let card = bench.r#ref(&opened, "textbox", "Card").await?;
    let copy = bench.r#ref(&opened, "textbox", "Copy").await?;
    let env = bench.host(&opened);
    let field = Field::new(&card, &bench.page_origin).with(FOLLOWED)?;
    let entry = bench.enter(&env, &opened, vec![field]).await;
    assert_eq!(entry.outcome, EntryOutcome::Entered, "{entry:?}");
    let page = &opened.page;
    // Row 15: a second box set to the value while the filled box still holds it.
    bench.live.main_world(page, "copyCard(false)").await?;
    assert_eq!(
        masked_value(&bench, &opened, &copy).await?,
        bullets,
        "the copy"
    );
    // Row 13: a copy the page groups with spaces.
    bench.live.main_world(page, "copyCard(true)").await?;
    let grouped = "•".repeat(FOLLOWED.len() + 3);
    assert_eq!(
        masked_value(&bench, &opened, &copy).await?,
        grouped,
        "the grouped copy"
    );
    // Row 12: a clone.
    bench.live.main_world(page, "cloneCard()").await?;
    let twin = bench.r#ref(&opened, "textbox", "Twin").await?;
    assert_eq!(
        masked_value(&bench, &opened, &twin).await?,
        bullets,
        "the clone"
    );
    // Row 11: a new element in the box's place, also in a whole-page snapshot, and toHaveValue.
    bench.live.main_world(page, "recreate()").await?;
    let fresh = bench.r#ref(&opened, "textbox", "Fresh").await?;
    assert_eq!(
        masked_value(&bench, &opened, &fresh).await?,
        bullets,
        "the new box"
    );
    let printed = bench
        .live
        .invoke("page.ariaSnapshot", page_snapshot(&page.brand(), false))
        .await??
        .to_string();
    assert!(
        !printed.contains(FOLLOWED),
        "the snapshot prints no value: {printed}"
    );
    let same_as_bullets = expectation(
        ("target", ref_target(&page.brand(), &fresh)),
        "toHaveValue",
        &json!({ "expression": "to.have.value", "isNot": false,
            "expectedText": [{ "string": bullets }] }),
    );
    bench
        .live
        .invoke("expect.locator", same_as_bullets)
        .await??;
    // Row 16: a new document keeps no digest.
    bench.live.main_world(page, "location.reload()").await?;
    time::sleep(Duration::from_millis(500)).await;
    bench
        .live
        .main_world(
            page,
            &format!("document.getElementById('copy').value = '{FOLLOWED}'"),
        )
        .await?;
    let copy = bench.r#ref(&opened, "textbox", "Copy").await?;
    assert_eq!(
        masked_value(&bench, &opened, &copy).await?,
        FOLLOWED,
        "after a reload"
    );
    bench.close(opened).await
}

/// Ruling R50's row 14, one guess at most: after a private fill the page empties every box that
/// held the value, and a script makes ten guesses, each by setting the Guess box and reading it
/// with `inputValue()`, with no whole-page snapshot between. With the right value fifth, none of
/// the ten reads bullets: the first wrong guess's read forgot the digest. With the right value
/// first, that one read is bullets (the value is on the page again), and no later read can be,
/// the right value again included. Fails with forgetting taken out, or done only at a whole-page
/// snapshot: the fifth guess reads bullets.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_value_that_left_can_be_guessed_once_at_most() -> TestResult {
    let bench = Bench::start().await?;
    for right_at in [5, 1] {
        let opened = bench
            .open("/private/mark-follows.html", Place::Front)
            .await?;
        let card = bench.r#ref(&opened, "textbox", "Card").await?;
        let guess = bench.r#ref(&opened, "textbox", "Guess").await?;
        let env = bench.host(&opened);
        let field = Field::new(&card, &bench.page_origin).with(FOLLOWED)?;
        let entry = bench.enter(&env, &opened, vec![field]).await;
        assert_eq!(entry.outcome, EntryOutcome::Entered, "{entry:?}");
        bench.live.main_world(&opened.page, "emptyAll()").await?;
        let mut masked = Vec::new();
        for attempt in 1..=10 {
            let text = if attempt == right_at || (right_at == 1 && attempt == 6) {
                FOLLOWED.to_owned()
            } else {
                format!("42424242424242{attempt:02}")
            };
            bench
                .live
                .main_world(&opened.page, &format!("guess('{text}')"))
                .await?;
            if masked_value(&bench, &opened, &guess)
                .await?
                .starts_with('•')
            {
                masked.push(attempt);
            }
        }
        let wanted: Vec<i32> = if right_at == 1 { vec![1] } else { Vec::new() };
        assert_eq!(
            masked, wanted,
            "right value at guess {right_at}: the guesses read as bullets"
        );
        bench.close(opened).await?;
    }
    Ok(())
}

/// Ruling R50's row 13a: a page that keeps four characters of what was typed (`value_not_taken`)
/// then puts a new box in its place that holds them. The digest was kept of what the box held at
/// its read-back, so the new box reads as bullets. Fails with a digest kept only when the box
/// holds the value it was given: the new box reads the four characters.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_kept_part_of_the_value_is_masked_where_it_goes() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench.open("/private/rewrite.html", Place::Front).await?;
    let code_box = bench.r#ref(&opened, "textbox", "Short code").await?;
    let env = bench.host(&opened);
    let field = Field::new(&code_box, &bench.page_origin).with("246813")?;
    let entry = bench.enter(&env, &opened, vec![field]).await;
    assert_eq!(code(&entry), Some("value_not_taken"), "{entry:?}");
    bench
        .live
        .main_world(
            &opened.page,
            "(() => { const old = document.querySelector('input'); \
             const fresh = document.createElement('input'); \
             fresh.setAttribute('aria-label', 'Fresh code'); fresh.value = old.value; \
             old.replaceWith(fresh) })()",
        )
        .await?;
    let fresh = bench.r#ref(&opened, "textbox", "Fresh code").await?;
    assert_eq!(
        masked_value(&bench, &opened, &fresh).await?,
        "••••",
        "the new box"
    );
    bench.close(opened).await
}

/// A check call's request on `opened` that compares `value` with the box `r#ref` names, the
/// host having approved `origin` for it (ruling R48), within ten seconds.
fn compare_request(
    opened: &Opened,
    r#ref: &str,
    value: &str,
    spaced: bool,
    origin: &str,
) -> TestResult<LookRequest> {
    Ok(LookRequest {
        tab: opened.page.tab,
        url: None,
        fields: vec![LookField {
            box_ref: r#ref.to_owned(),
            expect: Some(Expect {
                origin: origin.to_owned(),
                ..Expect::default()
            }),
            keys: false,
        }],
        compares: vec![Some(LookCompare {
            value: PrivateText::new(value.to_owned())?,
            spaced,
            cut: false,
        })],
        deadline: Instant::now() + Duration::from_secs(10),
    })
}

/// One check call on `opened` that compares `value` with the box `r#ref` names, on the page's
/// own origin (ruling R48).
async fn compared(
    bench: &Bench,
    opened: &Opened,
    r#ref: &str,
    value: &str,
    spaced: bool,
) -> TestResult<BoxLook> {
    let request = compare_request(opened, r#ref, value, spaced, &bench.page_origin)?;
    look_one(bench, opened, &request).await
}

/// The check call `request` on `opened`, and its one box's answer.
async fn look_one(bench: &Bench, opened: &Opened, request: &LookRequest) -> TestResult<BoxLook> {
    let env = bench.host(opened);
    let looks = bench
        .live
        .playwright
        .look_boxes(&env, request)
        .await
        .map_err(|error| format!("the check failed: {error:?}"))?;
    assert!(!env.was_sent(), "a check never marks itself sent");
    looks
        .into_iter()
        .next()
        .ok_or_else(|| "no answer for the box".into())
}

/// The booleans a check answered, or `None` when it read nothing.
fn booleans(look: &BoxLook) -> Option<(bool, bool)> {
    look.compared
        .map(|compared| (compared.holds, compared.contains))
}

/// Ruling R48's rows 1 to 5: the check call compares a host's value with a named box, in the
/// box's isolated world, and answers two booleans. A box after a private fill holds and contains
/// its value and not another; with `spaced` a box the page groups holds the bare number. A box
/// that holds the number inside a longer text contains it and does not hold it, grouped too with
/// `spaced`, and a box the page masked does neither. A covered, a hidden and an out-of-view box
/// answer their refusal and both booleans: reading needs no hit. A checkbox answers neither. A
/// box whose frame was removed answers neither, and `element_detached`: never a `false` for a
/// box Neo did not read. A tab no longer at the host's address answers `page_changed` and
/// neither boolean: the value goes only where the fill would send it. Fails with containment kept without setting separators aside (the
/// grouped number in a longer text is not contained), and with an unread box answered as
/// `holds: false`.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_check_call_compares_a_value_with_a_named_box() -> TestResult {
    const NUMBER: &str = "4242424242424242";
    const OTHER: &str = "5555555555554444";
    let bench = Bench::start().await?;
    let opened = bench.open("/private/compare.html", Place::Front).await?;
    let r#ref = |name: &'static str| bench.r#ref(&opened, "textbox", name);
    let card = r#ref("Card").await?;
    let env = bench.host(&opened);
    let entry = bench
        .enter(
            &env,
            &opened,
            vec![Field::new(&card, &bench.page_origin).with(NUMBER)?],
        )
        .await;
    assert_eq!(entry.outcome, EntryOutcome::Entered, "{entry:?}");
    let both = Some((true, true));
    let contained = Some((false, true));
    let neither = Some((false, false));
    // Row 1.
    let look = compared(&bench, &opened, &card, NUMBER, false).await?;
    assert_eq!(booleans(&look), both, "the filled box: {look:?}");
    let look = compared(&bench, &opened, &card, OTHER, false).await?;
    assert_eq!(booleans(&look), neither, "another value: {look:?}");
    let grouped = r#ref("Grouped").await?;
    let look = compared(&bench, &opened, &grouped, NUMBER, true).await?;
    assert_eq!(booleans(&look), both, "grouped, spaced: {look:?}");
    let look = compared(&bench, &opened, &grouped, NUMBER, false).await?;
    assert_eq!(booleans(&look), neither, "grouped, not spaced: {look:?}");
    // Row 2.
    let longer = r#ref("Longer").await?;
    let look = compared(&bench, &opened, &longer, NUMBER, false).await?;
    assert_eq!(booleans(&look), contained, "inside a longer text: {look:?}");
    let longer_grouped = r#ref("Longer grouped").await?;
    let look = compared(&bench, &opened, &longer_grouped, NUMBER, true).await?;
    assert_eq!(
        booleans(&look),
        contained,
        "grouped in a longer text: {look:?}"
    );
    let masked = r#ref("Masked").await?;
    let look = compared(&bench, &opened, &masked, NUMBER, true).await?;
    assert_eq!(booleans(&look), neither, "masked by the page: {look:?}");
    // Row 3: the refusal and both booleans.
    // A hidden box prints no ref: a host mints one with `locator.ref()`.
    for (css, refused) in [
        ("#covered", "box_covered"),
        ("#hidden", "box_hidden"),
        ("#away", "cover_unread"),
    ] {
        let name = css;
        let named = bench.mint(&opened, css).await?;
        let look = compared(&bench, &opened, &named, NUMBER, false).await?;
        let code = look.refusal.as_ref().map(|error| error.code.as_str());
        assert_eq!(
            (code, booleans(&look)),
            (Some(refused), both),
            "{name}: {look:?}"
        );
    }
    // Row 4: no text input.
    let agree = bench.r#ref(&opened, "checkbox", "Agree").await?;
    let look = compared(&bench, &opened, &agree, NUMBER, false).await?;
    assert_eq!(booleans(&look), None, "a checkbox: {look:?}");
    unapproved_compares_send_nothing(&bench, &opened, &card).await?;
    // Row 5: the ref is dead.
    let inner = r#ref("Inner box").await?;
    bench
        .live
        .main_world(&opened.page, "document.querySelector('iframe').remove()")
        .await?;
    let look = compared(&bench, &opened, &inner, NUMBER, false).await?;
    let code = look.refusal.as_ref().map(|error| error.code.as_str());
    assert_eq!(
        (code, booleans(&look)),
        (Some("element_detached"), None),
        "a box whose frame was removed: {look:?}"
    );
    bench.close(opened).await
}

/// Ruling R48's rows 8a and 8c on `card`, which holds the number: asserted on what was sent,
/// the session of every command that carried it. A compare on the approved origin comes first,
/// to show the watch sees a value that goes. A tab no longer at the host's address and a box on
/// another origin than the host approved are refused as the fill is, with no boolean, and the
/// value went to no frame at all.
async fn unapproved_compares_send_nothing(
    bench: &Bench,
    opened: &Opened,
    card: &str,
) -> TestResult {
    const NUMBER: &str = "4242424242424242";
    // Rows 8a and 8c assert on what was sent: the session of every command that carried the
    // number. A compare on the approved origin first, to show the watch sees a value that goes.
    bench.recording.watch_for(NUMBER);
    let look = compared(bench, opened, card, NUMBER, false).await?;
    assert_eq!(
        booleans(&look),
        Some((true, true)),
        "the approved origin: {look:?}"
    );
    assert!(
        !bench.recording.take_carriers().is_empty(),
        "the watch sees the compare's request"
    );
    // Row 8a: a tab no longer at the host's address is refused as the fill is, and not compared.
    let mut elsewhere = compare_request(opened, card, NUMBER, false, &bench.page_origin)?;
    elsewhere.url = Some("https://elsewhere.example/".to_owned());
    let look = look_one(bench, opened, &elsewhere).await?;
    let code = look.refusal.as_ref().map(|error| error.code.as_str());
    assert_eq!(
        (code, booleans(&look)),
        (Some("page_changed"), None),
        "another address: {look:?}"
    );
    assert_eq!(
        bench.recording.take_carriers(),
        Vec::<Option<String>>::new(),
        "not sent"
    );
    // Row 8c: a box on another origin than the host approved answers `wrong_origin` and no
    // boolean, and the value never went to its frame, nor anywhere else.
    let foreign = compare_request(opened, card, NUMBER, false, &bench.other_origin)?;
    let look = look_one(bench, opened, &foreign).await?;
    let code = look.refusal.as_ref().map(|error| error.code.as_str());
    assert_eq!(
        (code, booleans(&look)),
        (Some("wrong_origin"), None),
        "another origin: {look:?}"
    );
    assert_eq!(
        bench.recording.take_carriers(),
        Vec::<Option<String>>::new(),
        "not sent"
    );
    Ok(())
}

/// Ruling R48's row 6, not read: the box's frame is still there and stops answering for three
/// seconds. The check ends at its two-second deadline with `run_timeout` and no boolean for the
/// box: Neo never says `false` of a box it did not read.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_whose_frame_does_not_answer_gets_no_boolean() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench.open("/private/busy-frame.html", Place::Front).await?;
    let busy = bench.r#ref(&opened, "textbox", "Busy box").await?;
    bench
        .live
        .main_world(
            &opened.page,
            "document.querySelector('iframe').contentWindow.postMessage('go', '*')",
        )
        .await?;
    let env = bench.host(&opened);
    // The frame's own origin, approved, so only its silence keeps the compare from answering.
    let frame_origin = bench.page_origin.replace("127.0.0.1", "[::1]");
    let mut request = compare_request(&opened, &busy, FOLLOWED, false, &frame_origin)?;
    request.deadline = Instant::now() + Duration::from_secs(2);
    let answered = bench.live.playwright.look_boxes(&env, &request).await;
    match answered {
        Err(error) => assert_eq!(error.code, "run_timeout", "{error:?}"),
        Ok(looks) => {
            let look = looks.first().ok_or("an answer for the box")?;
            let code = look.refusal.as_ref().map(|error| error.code.as_str());
            assert_eq!(
                (code, look.compared),
                (Some("run_timeout"), None),
                "{look:?}"
            );
        }
    }
    // The frame's loop ends a second later: let it, so the browser is idle for the next test.
    time::sleep(Duration::from_millis(1500)).await;
    bench.close(opened).await
}

/// A plain check call on `opened` for the box `r#ref` names, with no expectation and no
/// compare, ending by `within` from now.
fn plain_look(opened: &Opened, r#ref: &str, within: Duration) -> LookRequest {
    LookRequest {
        tab: opened.page.tab,
        url: None,
        fields: vec![LookField {
            box_ref: r#ref.to_owned(),
            expect: None,
            keys: false,
        }],
        compares: vec![None],
        deadline: Instant::now() + within,
    }
}

/// The refusal code and `frame_gone` of the check call's answer for `r#ref`.
async fn gone_of(
    bench: &Bench,
    opened: &Opened,
    r#ref: &str,
) -> TestResult<(Option<String>, bool)> {
    let look = look_one(
        bench,
        opened,
        &plain_look(opened, r#ref, Duration::from_secs(10)),
    )
    .await?;
    Ok((look.refusal.map(|error| error.code), look.frame_gone))
}

/// The code a dead ref answers.
const DETACHED: &str = "element_detached";

/// Ruling R63, rows 1, 3, 7 and 8: whether a dead ref's frame left the tab, in a same-site frame
/// (on the page's session) and a cross-site one (on its own). A live box answers no refusal and
/// no `frame_gone` (row 8). A box the page removed from a document that stays answers
/// `element_detached` and no `frame_gone` (row 7). A frame that loads a new document in place
/// keeps its frame: its old box answers `element_detached` and no `frame_gone` (row 3). A frame
/// the page removed takes its box out of the tab: `element_detached` and `frame_gone` (row 1).
/// Fails with the field never set (row 1 answers false), and with every dead ref read as gone
/// (rows 3 and 7 answer true).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_dead_ref_says_frame_gone_only_when_its_frame_left() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/frames-gone.html", Place::Front)
        .await?;
    let named = |name: &'static str| bench.r#ref(&opened, "textbox", name);
    let (page, near, far) = (
        named("Page box").await?,
        named("Near box").await?,
        named("Card number").await?,
    );
    for r#ref in [&page, &near, &far] {
        assert_eq!(
            gone_of(&bench, &opened, r#ref).await?,
            (None, false),
            "row 8: {ref}"
        );
    }
    bench
        .live
        .main_world(&opened.page, "removePageBox()")
        .await?;
    assert_eq!(
        gone_of(&bench, &opened, &page).await?,
        (Some(DETACHED.to_owned()), false),
        "row 7"
    );
    bench
        .live
        .main_world(&opened.page, "reloadNear() && reloadFar()")
        .await?;
    time::sleep(Duration::from_millis(800)).await;
    for r#ref in [&near, &far] {
        assert_eq!(
            gone_of(&bench, &opened, r#ref).await?,
            (Some(DETACHED.to_owned()), false),
            "row 3: {ref}"
        );
    }
    let (near, far) = (named("Near box").await?, named("Card number").await?);
    bench
        .live
        .main_world(&opened.page, "removeNear() && removeFar()")
        .await?;
    for r#ref in [&near, &far] {
        assert_eq!(
            gone_of(&bench, &opened, r#ref).await?,
            (Some(DETACHED.to_owned()), true),
            "row 1: {ref}"
        );
    }
    bench.close(opened).await
}

/// Ruling R63, row 2: the tab moves on to another document and takes its frames with it. A box
/// of the old page's frames answers `element_detached` and `frame_gone`; a box of the old main
/// document answers `element_detached` and no `frame_gone`, as the main frame stays. Fails with
/// the field never set (the framed boxes answer false), and with every dead ref read as gone
/// (the main document's box answers true).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_top_navigation_takes_its_frames_with_it() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/frames-gone.html", Place::Front)
        .await?;
    let named = |name: &'static str| bench.r#ref(&opened, "textbox", name);
    let (page, near, far) = (
        named("Page box").await?,
        named("Near box").await?,
        named("Card number").await?,
    );
    bench
        .live
        .main_world(&opened.page, "location.href = '/private/look.html'")
        .await?;
    time::sleep(Duration::from_millis(1000)).await;
    assert_eq!(
        gone_of(&bench, &opened, &page).await?,
        (Some(DETACHED.to_owned()), false),
        "the main document"
    );
    for r#ref in [&near, &far] {
        assert_eq!(
            gone_of(&bench, &opened, r#ref).await?,
            (Some(DETACHED.to_owned()), true),
            "a frame: {ref}"
        );
    }
    bench.close(opened).await
}

/// Ruling R63, row 5: a ref from before a restart of Neo, here a second `Playwright` on the same
/// browser, into a frame that is in fact gone: what it answers today (`element_detached`), and no
/// `frame_gone`, as nothing records where that ref's world was. Fails with every dead ref read as
/// gone.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_ref_from_before_a_restart_says_no_frame_gone() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/frames-gone.html", Place::Front)
        .await?;
    let near = bench.r#ref(&opened, "textbox", "Near box").await?;
    bench.live.main_world(&opened.page, "removeNear()").await?;
    let restarted = LiveBrowser::connect().await?;
    let env = HostEnv::new(&restarted);
    let looks = restarted
        .playwright
        .look_boxes(&env, &plain_look(&opened, &near, Duration::from_secs(10)))
        .await
        .map_err(|error| format!("the check failed: {error:?}"))?;
    let look = looks.first().ok_or("an answer for the box")?;
    let code = look.refusal.as_ref().map(|error| error.code.clone());
    assert_eq!(
        (code, look.frame_gone),
        (Some(DETACHED.to_owned()), false),
        "{look:?}"
    );
    bench.close(opened).await
}

/// Ruling R63, row 6: a frame-tree read that fails, refused or never answered, gives no
/// `frame_gone` for a ref whose frame is in fact gone; the same ref answers it once the read goes
/// through. A read that is not whole cannot come back: the read requires every cross-site frame's
/// tree and fails without one (`frame_tree::read_every`). Fails with a failed read taken as
/// "the frame is not there".
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_failed_frame_tree_read_says_no_frame_gone() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/frames-gone.html", Place::Front)
        .await?;
    let near = bench.r#ref(&opened, "textbox", "Near box").await?;
    bench.live.main_world(&opened.page, "removeNear()").await?;
    for (method, how) in [
        ("Page.getFrameTree", Failing::Refuse),
        ("Target.getTargets", Failing::Hang),
    ] {
        bench.recording.fail(method, how);
        let look = look_one(
            &bench,
            &opened,
            &plain_look(&opened, &near, Duration::from_secs(2)),
        )
        .await;
        bench.recording.stop_failing();
        let look = look?;
        let code = look.refusal.as_ref().map(|error| error.code.clone());
        assert_eq!(
            (code, look.frame_gone),
            (Some(DETACHED.to_owned()), false),
            "{method} {how:?}: {look:?}"
        );
    }
    assert_eq!(
        gone_of(&bench, &opened, &near).await?,
        (Some(DETACHED.to_owned()), true),
        "the read goes through"
    );
    bench.close(opened).await
}

/// Ruling R63's two reads, held by a row (the design keeper's control): `frame_gone` needs two
/// steady reads in a row that both miss the frame, so a second read that fails answers no
/// `frame_gone`. Both frames are removed; a first check answers true and counts its lists of the
/// out-of-process frames (two per read, the second read's at the end); the same check again with
/// that list failing from the second read's first call on answers no `frame_gone`. Fails with the
/// second read replaced by the first.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_frame_gone_needs_the_second_read_too() -> TestResult {
    const LISTS: &str = "Target.getTargets";
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/frames-gone.html", Place::Front)
        .await?;
    let near = bench.r#ref(&opened, "textbox", "Near box").await?;
    bench
        .live
        .main_world(&opened.page, "removeNear() && removeFar()")
        .await?;
    bench.recording.reset_calls();
    assert_eq!(
        gone_of(&bench, &opened, &near).await?,
        (Some(DETACHED.to_owned()), true),
        "both reads miss the frame"
    );
    let lists = bench.recording.calls(LISTS);
    assert!(lists >= 4, "two reads of two lists each: {lists}");
    bench.recording.fail_from(LISTS, Failing::Refuse, lists - 1);
    let look = gone_of(&bench, &opened, &near).await;
    bench.recording.stop_failing();
    assert_eq!(
        look?,
        (Some(DETACHED.to_owned()), false),
        "the first read missed the frame and the second failed"
    );
    bench.close(opened).await
}

/// Ruling R63, rows 9 and 10: a field that carries a value, on a frame that left the tab, answers
/// no `holds` and no `contains`, and `frame_gone`. A fill on that ref answers as on main: refused
/// `element_detached`, nothing typed; the fill's answer has no such field. Fails with the field
/// never set.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_value_on_a_gone_frame_is_not_compared_and_says_so() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/frames-gone.html", Place::Front)
        .await?;
    let far = bench.r#ref(&opened, "textbox", "Card number").await?;
    bench.live.main_world(&opened.page, "removeFar()").await?;
    let request = compare_request(&opened, &far, CARD, false, &bench.other_origin)?;
    let look = look_one(&bench, &opened, &request).await?;
    let refused = look.refusal.as_ref().map(|error| error.code.clone());
    assert_eq!(
        (refused, look.compared, look.frame_gone),
        (Some(DETACHED.to_owned()), None, true),
        "{look:?}"
    );
    let env = bench.host(&opened);
    let field = Field::new(&far, &bench.other_origin).with(CARD)?;
    let entry = bench.enter(&env, &opened, vec![field]).await;
    assert_eq!(entry.outcome, EntryOutcome::Refused, "{entry:?}");
    assert_eq!(code(&entry), Some("element_detached"), "{entry:?}");
    bench.close(opened).await
}

/// One ruling R64 row of a one-box fill: its number, the page's own setup, the box's role and
/// name, the approved limit, whether a cut is allowed, the approved name, the value, the answer.
type LimitRow<'a> = (
    u32,
    &'a str,
    (&'a str, &'a str),
    i32,
    bool,
    Option<&'a str>,
    &'a str,
    LimitAnswer,
);

/// What a ruling R64 row expects of a one-box fill.
#[derive(Clone, Copy, Debug)]
enum LimitAnswer {
    /// Entered, this many characters typed, and listed in `max_length_moved` or not.
    Entered { typed: usize, listed: bool },
    /// Refused `box_changed`, naming this fact, nothing typed.
    Changed(&'static str),
}

/// Ruling R64, row 1: a Visa number moves the code box's limit from 4 to 3 while it is typed, as
/// a hosted card field does once it knows the brand. A host that sends back the limits it read
/// (22, 5, 4) gets the form entered, 16, 5 and 3 characters, and the code box listed as moved.
/// Main answers `partial`, `box_changed` at the code box. Fails with the rule off (any moved limit
/// refuses) and with the flag never set.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_limit_moved_by_the_brand_still_takes_a_code_that_fits() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/card-limits.html", Place::Front)
        .await?;
    let named = |name: &'static str| bench.r#ref(&opened, "textbox", name);
    let (number, expiry, code_box) = (
        named("Card number").await?,
        named("Expiry").await?,
        named("Security code").await?,
    );
    let origin = &bench.page_origin;
    let fields = vec![
        Field::new(&number, origin)
            .max_length(22)
            .with("4242424242424242")?,
        Field::new(&expiry, origin).max_length(5).with("12/34")?,
        Field::new(&code_box, origin).max_length(4).with("123")?,
    ];
    let env = bench.host(&opened);
    let entry = bench.enter(&env, &opened, fields).await;
    assert_eq!(entry.outcome, EntryOutcome::Entered, "{entry:?}");
    assert_eq!(entry.typed, vec![16, 5, 3], "{entry:?}");
    assert_eq!(entry.max_length_moved, vec![2], "{entry:?}");
    bench.close(opened).await
}

/// Ruling R64, the one-box rows: a limit that moved refuses only when the value does not fit both
/// the approved limit and the limit now; `cut` plays no part in it; -1 is a box approved with no
/// limit; a box whose type the browser applies no limit to fits any value; a harmless move never
/// hides a later fact that differs; a limit that moves during its own box's typing is not listed
/// and the read-back decides. Each row opens the page afresh. Fails with the rule off (rows 2, 4
/// and 11 refuse), with no refusal for any moved limit (rows 3, 5, 6, 7, 10 and 16 enter), with a
/// harmless move ending the compare of the facts (row 9 enters), and with the flag never set.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
#[expect(clippy::too_many_lines, reason = "one table of the ruling's rows")]
async fn tests_that_a_moved_limit_refuses_only_when_it_would_cut_the_value() -> TestResult {
    use LimitAnswer::{Changed, Entered};
    const VISA: &str = "4242424242424242";
    const AMEX: &str = "378282246310005";
    const TWENTY: &str = "42424242424242424242";
    let bench = Bench::start().await?;
    // (row, the page's own setup, role and name of the box, approved limit, cut, approved name,
    // value, answer)
    let rows: [LimitRow<'_>; 13] = [
        (
            2,
            "setLimit('number', 17)",
            ("textbox", "Card number"),
            22,
            false,
            None,
            AMEX,
            Entered {
                typed: 15,
                listed: true,
            },
        ),
        (
            3,
            "setLimit('code', 3)",
            ("textbox", "Security code"),
            4,
            false,
            None,
            "1234",
            Changed("maxLength"),
        ),
        (
            4,
            "setLimit('plain', 20)",
            ("textbox", "Plain"),
            -1,
            false,
            None,
            VISA,
            Entered {
                typed: 16,
                listed: true,
            },
        ),
        (
            5,
            "setLimit('plain', 8)",
            ("textbox", "Plain"),
            -1,
            false,
            None,
            VISA,
            Changed("maxLength"),
        ),
        (
            6,
            "true",
            ("textbox", "Plain"),
            16,
            true,
            None,
            TWENTY,
            Changed("maxLength"),
        ),
        (
            7,
            "setLimit('plain', 12)",
            ("textbox", "Plain"),
            16,
            true,
            None,
            TWENTY,
            Changed("maxLength"),
        ),
        (
            8,
            "setLimit('plain', 16)",
            ("textbox", "Plain"),
            16,
            true,
            None,
            TWENTY,
            Entered {
                typed: 16,
                listed: false,
            },
        ),
        (
            9,
            "setLimit('code', 3) && setName('code', 'other')",
            ("textbox", "Security code"),
            4,
            false,
            Some("cvc"),
            "123",
            Changed("name"),
        ),
        (
            10,
            "setLimit('plain', 4)",
            ("textbox", "Plain"),
            3,
            false,
            None,
            "1234",
            Changed("maxLength"),
        ),
        (
            11,
            "setLimit('pin', 3)",
            ("spinbutton", "Pin"),
            4,
            false,
            None,
            "123456",
            Entered {
                typed: 6,
                listed: true,
            },
        ),
        (
            14,
            "true",
            ("textbox", "Security code"),
            4,
            false,
            None,
            "123",
            Entered {
                typed: 3,
                listed: false,
            },
        ),
        (
            16,
            "setLimit('plain', 6)",
            ("textbox", "Plain"),
            -1,
            false,
            None,
            "CANARY-49",
            Changed("maxLength"),
        ),
        (
            17,
            "true",
            ("textbox", "Card number"),
            22,
            false,
            None,
            AMEX,
            Entered {
                typed: 15,
                listed: false,
            },
        ),
    ];
    let mut failures = Vec::new();
    for (row, setup, (role, name), approved, cut, approved_name, value, answer) in rows {
        let opened = bench
            .open("/private/card-limits.html", Place::Front)
            .await?;
        bench.live.main_world(&opened.page, setup).await?;
        let r#ref = bench.r#ref(&opened, role, name).await?;
        let mut field = Field::new(&r#ref, &bench.page_origin).max_length(approved);
        if cut {
            field = field.cut();
        }
        if let Some(approved_name) = approved_name {
            field = field.named(approved_name);
        }
        let env = bench.host(&opened);
        let entry = bench.enter(&env, &opened, vec![field.with(value)?]).await;
        let held = match answer {
            Entered { typed, listed } => {
                let moved: Vec<usize> = if listed { vec![0] } else { Vec::new() };
                entry.outcome == EntryOutcome::Entered
                    && entry.typed == vec![typed]
                    && entry.max_length_moved == moved
            }
            Changed(fact) => {
                entry.outcome == EntryOutcome::Refused
                    && code(&entry) == Some("box_changed")
                    && entry
                        .error
                        .as_ref()
                        .is_some_and(|error| error.message.contains(&format!("box's {fact}")))
                    && entry.typed == vec![0]
                    && entry.max_length_moved.is_empty()
            }
        };
        if !held {
            failures.push(format!("row {row}, wanted {answer:?}: {entry:?}"));
        }
        bench.close(opened).await?;
    }
    assert!(failures.is_empty(), "{failures:#?}");
    Ok(())
}

/// Ruling R64, the check call's rows (12 to 15): it carries no length, so it never refuses for a
/// limit that moved; it answers `max_length_moved` with the limit in the facts, with a value too
/// (whose compare still answers), and not when the limit is the approved one. Row 15 is R49's
/// check-call row, which changes: -1 on a box that has a limit answers no refusal and the flag,
/// where main answers `box_changed`. Fails with the rule off (rows 12 and 15 refuse) and with the
/// flag never set.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_check_call_says_a_limit_moved_and_never_refuses_for_it() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/card-limits.html", Place::Front)
        .await?;
    bench
        .live
        .main_world(&opened.page, "setLimit('code', 3) && setLimit('plain', 6)")
        .await?;
    let named = |name: &'static str| bench.r#ref(&opened, "textbox", name);
    let (code_box, plain, number) = (
        named("Security code").await?,
        named("Plain").await?,
        named("Card number").await?,
    );
    let checked = |r#ref: &str, approved: i32, value: Option<&str>| {
        let mut request = plain_look(&opened, r#ref, Duration::from_secs(10));
        request.fields[0].expect = Some(Expect {
            origin: bench.page_origin.clone(),
            max_length: Some(approved),
            ..Expect::default()
        });
        let compare = value.map(|value| {
            PrivateText::new(value.to_owned()).map(|value| LookCompare {
                value,
                spaced: false,
                cut: false,
            })
        });
        (request, compare)
    };
    for (row, r#ref, approved, value, limit_now, moved) in [
        (12, &code_box, 4, None, Some(3), true),
        (13, &code_box, 4, Some("123"), Some(3), true),
        (14, &number, 22, None, Some(22), false),
        (15, &plain, -1, None, Some(6), true),
    ] {
        let (mut request, compare) = checked(r#ref, approved, value);
        request.compares = vec![compare.transpose()?];
        let look = look_one(&bench, &opened, &request).await?;
        assert!(look.refusal.is_none(), "row {row}: {look:?}");
        assert_eq!(look.max_length_moved, moved, "row {row}: {look:?}");
        let facts = look.facts.as_ref().ok_or("facts")?;
        assert_eq!(facts.max_length, limit_now, "row {row}: {look:?}");
        assert_eq!(
            look.compared.is_some(),
            value.is_some(),
            "row {row}: {look:?}"
        );
    }
    bench.close(opened).await
}

/// Rulings R55 and R56: a sign-in whose two boxes each sit under a sibling div that draws the
/// field's name and takes the pointer, which is no `<label>`. With nothing set by the host, both
/// boxes are entered and read back whole, by one insertion and key by key, on a tab in front and
/// a tab behind, and the entry notes each as entered under its field's drawing. Fails with the
/// drawing's branch left out of the look (`box_covered` at the first box, nothing typed).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_boxes_under_drawn_labels_are_entered_by_default() -> TestResult {
    let bench = Bench::start().await?;
    for place in PLACES {
        for keys in [false, true] {
            let case = format!("{place:?}, keys {keys}");
            let opened = bench.open("/private/drawn-label-login.html", place).await?;
            let email = bench.r#ref(&opened, "textbox", "Email").await?;
            let password = bench.r#ref(&opened, "textbox", "Password").await?;
            let env = bench.host(&opened);
            let origin = &bench.page_origin;
            let field = |r#ref: &str| {
                let field = Field::new(r#ref, origin);
                if keys { field.keys() } else { field }
            };
            let fields = vec![field(&email).with(EMAIL)?, field(&password).with(PASSWORD)?];
            let entry = bench.enter(&env, &opened, fields).await;
            assert_eq!(entry.outcome, EntryOutcome::Entered, "{case}: {entry:?}");
            assert_eq!(entry.typed, vec![EMAIL.len(), PASSWORD.len()], "{case}");
            assert_eq!(
                entry.drawings,
                vec![Drawing::Under, Drawing::Under],
                "{case}"
            );
            assert_eq!(bench.value(&opened, &email).await?, EMAIL, "{case}");
            assert_eq!(bench.value(&opened, &password).await?, PASSWORD, "{case}");
            bench.close(opened).await?;
        }
    }
    Ok(())
}

/// Ruling R56: a code box under blank digit drawings that show each digit as it is typed. Key by
/// key it is entered, the code stays drawn (six characters, a code's digits), and the entry notes
/// the drawn value for the record; by one insertion a blank drawing covers the box and nothing is
/// typed. On a tab in front and a tab behind. Fails with a blank drawing let through for one
/// insertion too (`entered`), or with a short drawn value emptied like a long one (`value_drawn`).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_code_under_drawn_digits_is_typed_key_by_key() -> TestResult {
    let bench = Bench::start().await?;
    for place in PLACES {
        let opened = bench.open("/private/drawn-code.html", place).await?;
        let code_box = bench.r#ref(&opened, "textbox", "Code").await?;
        let env = bench.host(&opened);
        let origin = &bench.page_origin;
        let inserted = bench
            .enter(
                &env,
                &opened,
                vec![Field::new(&code_box, origin).with(CODE)?],
            )
            .await;
        assert_eq!(
            (inserted.outcome, inserted.at, code(&inserted)),
            (EntryOutcome::Refused, Some(0), Some("box_covered")),
            "{place:?}: one insertion under a blank drawing: {inserted:?}"
        );
        assert_eq!(inserted.typed, vec![0], "{place:?}");
        let keyed = bench
            .enter(
                &env,
                &opened,
                vec![Field::new(&code_box, origin).keys().with(CODE)?],
            )
            .await;
        assert_eq!(keyed.outcome, EntryOutcome::Entered, "{place:?}: {keyed:?}");
        assert_eq!(keyed.typed, vec![CODE.len()], "{place:?}");
        assert_eq!(
            keyed.drawings,
            vec![Drawing::Drawn],
            "{place:?}: the record notes it"
        );
        assert_eq!(bench.value(&opened, &code_box).await?, CODE, "{place:?}");
        bench.close(opened).await?;
    }
    Ok(())
}

/// Ruling R56: a card-number field under a blank drawing that shows what is typed in groups of
/// four, separated by spaces or by dashes, typed key by key with `spaced` not set (nothing set by
/// the host). The drawn check sets white space, dashes and slashes aside whatever `spaced` says,
/// so the sixteen digits are seen as drawn: Neo empties the box, the drawing empties with it, and
/// the entry answers `value_drawn`; the page no longer shows the digits. On a tab in front and a
/// tab behind. Fails with the drawn check setting dashes and slashes aside only for a `spaced`
/// field (`dash`: `entered`, the digits left on the page).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_long_value_drawn_in_groups_is_emptied() -> TestResult {
    let bench = Bench::start().await?;
    for place in PLACES {
        for groups in ["space", "dash"] {
            let case = format!("{place:?}, {groups}");
            let opened = bench
                .open(&format!("/private/drawn-code.html?card={groups}"), place)
                .await?;
            let card = bench.r#ref(&opened, "textbox", "Card number").await?;
            let env = bench.host(&opened);
            let field = Field::new(&card, &bench.page_origin).keys();
            let entry = bench.enter(&env, &opened, vec![field.with(CARD)?]).await;
            assert_eq!(
                (entry.outcome, entry.at, code(&entry)),
                (EntryOutcome::Uncertain, Some(0), Some("value_drawn")),
                "{case}: {entry:?}"
            );
            assert_eq!(bench.value(&opened, &card).await?, "", "{case}: emptied");
            let shown = bench
                .live
                .main_world(
                    &opened.page,
                    "document.querySelector('.groups').textContent",
                )
                .await?;
            assert_eq!(
                shown,
                json!(""),
                "{case}: the page no longer shows the digits"
            );
            bench.close(opened).await?;
        }
    }
    Ok(())
}

/// Ruling R56: another field's `<label for>` laid over the password box is no drawing of this
/// field's own: the page says whose it is, and a person's click there goes to that field. Key by
/// key, as by one insertion, the box answers `box_covered` and nothing is typed, as main's rule
/// says. Fails with the label condition taken out of the look (`entered`).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_another_fields_label_covers_the_box_key_by_key() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/covered-login.html?mode=otherlabel", Place::Front)
        .await?;
    let password = bench.r#ref(&opened, "textbox", "Password").await?;
    let env = bench.host(&opened);
    let field = Field::new(&password, &bench.page_origin).keys();
    let entry = bench
        .enter(&env, &opened, vec![field.with(PASSWORD)?])
        .await;
    assert_eq!(
        (entry.outcome, entry.at, code(&entry)),
        (EntryOutcome::Refused, Some(0), Some("box_covered")),
        "{entry:?}"
    );
    assert_eq!(entry.typed, vec![0]);
    assert_eq!(bench.value(&opened, &password).await?, "");
    bench.close(opened).await
}

/// Rulings R55 and R56: a drawing an earlier look kept does not carry over. A box under a
/// drawing that shows its name and what it holds takes a long value by one insertion: the drawn
/// check empties it (`value_drawn`). The page then takes the box's size away, so the next look
/// reaches it by focus and never hit-tests it: typed key by key it is entered, under no drawing.
/// Fails with the look's clearing of the kept wrapper taken out (the second entry reads the first
/// look's wrapper: `value_drawn`).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_drawing_an_earlier_look_kept_does_not_carry_over() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/drawn-label-shapes.html", Place::Front)
        .await?;
    let nickname = bench.r#ref(&opened, "textbox", "Nickname").await?;
    let env = bench.host(&opened);
    let origin = &bench.page_origin;
    let first = bench
        .enter(
            &env,
            &opened,
            vec![Field::new(&nickname, origin).with(EMAIL)?],
        )
        .await;
    assert_eq!(code(&first), Some("value_drawn"), "{first:?}");
    page_does(&bench, &opened, "shrink()").await?;
    let second = bench
        .enter(
            &env,
            &opened,
            vec![Field::new(&nickname, origin).keys().with(EMAIL)?],
        )
        .await;
    assert_eq!(second.outcome, EntryOutcome::Entered, "{second:?}");
    assert_eq!(second.drawings, vec![Drawing::None]);
    bench.close(opened).await
}

/// Rulings R55 and R56, after the typing: a drawing that carries the box's name and also draws
/// what the box holds, a long value (nineteen characters), as typed or in capitals through
/// `text-transform` (`upper`). The look lets the box through, and the read-back finds the value
/// drawn outside the box where it was not before the first character: Neo empties the box again
/// and the entry answers `value_drawn`, with what was sent in `typed`. The box is empty, and the
/// value is in no answer and no note the host received. By one insertion and key by key, on a
/// tab in front and a tab behind. The page no longer shows the value afterwards, as the answer
/// says. Fails with the read-back's drawn answer ignored (`entered`, the box still full), or with
/// the box not emptied again (`value_still_drawn`: the box, and so the drawing, still hold it),
/// or with the drawn check minding case (`upper`: `entered`, the value left in the page's text).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_drawn_value_empties_the_box_and_answers_value_drawn() -> TestResult {
    let bench = Bench::start().await?;
    for place in PLACES {
        for (mirror, keys) in [("1", false), ("1", true), ("upper", false), ("upper", true)] {
            let case = format!("{place:?}, {mirror}, keys {keys}");
            let opened = bench
                .open(
                    &format!("/private/drawn-label-login.html?mirror={mirror}"),
                    place,
                )
                .await?;
            let email = bench.r#ref(&opened, "textbox", "Email").await?;
            let env = bench.host(&opened);
            let field = Field::new(&email, &bench.page_origin);
            let field = if keys { field.keys() } else { field };
            let entry = bench.enter(&env, &opened, vec![field.with(EMAIL)?]).await;
            assert_eq!(
                (entry.at, code(&entry)),
                (Some(0), Some("value_drawn")),
                "{case}: {entry:?}"
            );
            assert_eq!(entry.typed, vec![EMAIL.len()], "{case}");
            assert_eq!(
                bench.value(&opened, &email).await?,
                "",
                "{case}: emptied again"
            );
            assert_eq!(
                drawing(&bench, &opened).await?,
                "Email",
                "{case}: the page no longer shows the value"
            );
            let answered = format!("{entry:?}");
            assert!(
                !answered.contains(EMAIL),
                "{case}: the answer holds the value: {answered}"
            );
            for note in env.notes() {
                assert!(
                    !note.contains(EMAIL),
                    "{case}: a note holds the value: {note}"
                );
            }
            bench.close(opened).await?;
        }
    }
    Ok(())
}

/// What the login fixture's first drawing shows now, as the page's own world reads it.
async fn drawing(bench: &Bench, opened: &Opened) -> TestResult<String> {
    let text = bench
        .live
        .main_world(&opened.page, "document.querySelector('.float').textContent")
        .await?;
    Ok(text.as_str().unwrap_or_default().to_owned())
}

/// Rulings R55 and R56, a page that still draws a long value after Neo emptied the box: a drawing
/// that keeps
/// what was typed when the box empties (`sticky`, by one insertion and key by key), and a field
/// that moves each character out of the box into its own drawing (`moves`, which leaves the box
/// empty at the read-back, by one insertion). Neo cannot take text off a page, so the entry
/// answers `uncertain`, `value_still_drawn`, whose sentence says to reload or close the page; the
/// box is empty, the page still shows the value, and the value is in no answer and no note. On a
/// tab in front and a tab behind. Fails with the read-back's `drawn` answered only for a box that
/// holds the value (`moves` answers `value_not_taken`), or with no second reading of the wrapper
/// after the emptying (`value_drawn`, which says the page no longer shows it).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_value_the_page_still_draws_answers_value_still_drawn() -> TestResult {
    let bench = Bench::start().await?;
    for place in PLACES {
        for (mirror, keys) in [("sticky", false), ("sticky", true), ("moves", false)] {
            let case = format!("{place:?}, {mirror}, keys {keys}");
            let opened = bench
                .open(
                    &format!("/private/drawn-label-login.html?mirror={mirror}"),
                    place,
                )
                .await?;
            let email = bench.r#ref(&opened, "textbox", "Email").await?;
            let env = bench.host(&opened);
            let field = Field::new(&email, &bench.page_origin);
            let field = if keys { field.keys() } else { field };
            let entry = bench.enter(&env, &opened, vec![field.with(EMAIL)?]).await;
            assert_eq!(
                (entry.outcome, entry.at, code(&entry)),
                (EntryOutcome::Uncertain, Some(0), Some("value_still_drawn")),
                "{case}: {entry:?}"
            );
            let message = entry.error.as_ref().map(|error| error.message.clone());
            assert!(
                message
                    .as_deref()
                    .is_some_and(|text| text.contains("Reload or close the page")),
                "{case}: {message:?}"
            );
            assert_eq!(
                bench.value(&opened, &email).await?,
                "",
                "{case}: the box is empty"
            );
            assert!(
                drawing(&bench, &opened).await?.contains(EMAIL),
                "{case}: the page still shows the value, as the answer says"
            );
            let answered = format!("{entry:?}");
            assert!(
                !answered.contains(EMAIL),
                "{case}: the answer holds the value: {answered}"
            );
            for note in env.notes() {
                assert!(
                    !note.contains(EMAIL),
                    "{case}: a note holds the value: {note}"
                );
            }
            bench.close(opened).await?;
        }
    }
    Ok(())
}

/// A password the box's maxlength cuts is entered when the host allows the cut, and
/// refused before any key when it does not. A code that cannot fit is refused before any key,
/// never `entered_page_moved`.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_value_the_box_cuts_needs_the_hosts_leave() -> TestResult {
    let bench = Bench::start().await?;
    let origin = &bench.page_origin;
    let long = "CANARY-cut-password";
    let opened = bench
        .open("/private/covered-login.html?mode=cut", Place::Front)
        .await?;
    let password = bench.r#ref(&opened, "textbox", "Password").await?;
    let env = bench.host(&opened);
    let refused = bench
        .enter(
            &env,
            &opened,
            vec![Field::new(&password, origin).with(long)?],
        )
        .await;
    assert_eq!(refused.outcome, EntryOutcome::Refused, "{refused:?}");
    assert_eq!(code(&refused), Some("value_too_long"));
    let entry = bench
        .enter(
            &env,
            &opened,
            vec![Field::new(&password, origin).cut().with(long)?],
        )
        .await;
    assert_eq!(
        entry.outcome,
        EntryOutcome::Entered,
        "truncated password: {entry:?}"
    );
    assert_eq!(entry.typed, vec![8]);
    assert_eq!(bench.value(&opened, &password).await?, "CANARY-c");
    bench.close(opened).await?;

    // A box with no size typed key by key skips the show and cover checks, not the maxlength.
    let opened = bench
        .open("/private/moved.html?mode=short", Place::Front)
        .await?;
    let box_ref = bench.r#ref(&opened, "textbox", "Verification code").await?;
    bench
        .live
        .main_world(
            &opened.page,
            "Object.assign(document.getElementById('code').style,
               { width: '0', height: '0', padding: '0', border: '0' }); true",
        )
        .await?;
    let env = bench.host(&opened);
    let field = Field::new(&box_ref, origin).keys().with("246813")?;
    let entry = bench.enter(&env, &opened, vec![field]).await;
    assert_eq!(
        code(&entry),
        Some("value_too_long"),
        "a sizeless keyed box: {entry:?}"
    );
    bench.close(opened).await?;

    // A number box ignores maxlength (R23): its code is not refused, and goes in whole.
    let opened = bench.open("/private/number.html", Place::Front).await?;
    let box_ref = bench
        .r#ref(&opened, "spinbutton", "Verification code")
        .await?;
    let env = bench.host(&opened);
    let field = Field::new(&box_ref, origin).keys().with("246813")?;
    let entry = bench.enter(&env, &opened, vec![field]).await;
    assert_eq!(
        entry.outcome,
        EntryOutcome::Entered,
        "a number box: {entry:?}"
    );
    assert_eq!(bench.value(&opened, &box_ref).await?, "246813");
    bench.close(opened).await?;

    for (mode, value) in [
        ("short", "246813"),
        ("early", "246813"),
        ("earlyx", "246-813"),
    ] {
        let opened = bench
            .open(&format!("/private/moved.html?mode={mode}"), Place::Front)
            .await?;
        let box_ref = bench.r#ref(&opened, "textbox", "Verification code").await?;
        let env = bench.host(&opened);
        let entry = bench
            .enter(
                &env,
                &opened,
                vec![Field::new(&box_ref, origin).keys().with(value)?],
            )
            .await;
        assert_eq!(entry.outcome, EntryOutcome::Refused, "{mode}: {entry:?}");
        assert_eq!(code(&entry), Some("value_too_long"), "{mode}");
        assert_eq!(entry.typed, vec![0], "{mode}");
        assert_eq!(bench.value(&opened, &box_ref).await?, "", "{mode}");
        bench.close(opened).await?;
    }
    Ok(())
}

/// A box whose form submits on its last character or first input event is
/// `entered_page_moved`. A final-character submission may also be `entered`.
/// A box that stays is `entered`; an address change alone moves nothing. A form that submits
/// as the box takes focus types nothing and is never `entered_page_moved`.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_that_submits_is_entered_page_moved() -> TestResult {
    let bench = Bench::start().await?;
    let origin = &bench.page_origin;
    let moved_or_entered = [EntryOutcome::EnteredPageMoved, EntryOutcome::Entered];
    for place in PLACES {
        for (mode, value, keys, wanted) in [
            ("post", "246813", true, &moved_or_entered[..]),
            (
                "fetch",
                "246813",
                true,
                &[EntryOutcome::EnteredPageMoved][..],
            ),
            ("stay", "246813", true, &[EntryOutcome::Entered][..]),
            ("stay", "246813", false, &[EntryOutcome::Entered][..]),
            ("hash4", "2468", true, &[EntryOutcome::Entered][..]),
            ("replace4", "2468", false, &[EntryOutcome::Entered][..]),
            ("push4", "2468", true, &[EntryOutcome::Entered][..]),
        ] {
            let opened = bench
                .open(&format!("/private/moved.html?mode={mode}"), place)
                .await?;
            let box_ref = bench.r#ref(&opened, "textbox", "Verification code").await?;
            let env = bench.host(&opened);
            let field = Field::new(&box_ref, origin);
            let field = if keys { field.keys() } else { field };
            let entry = bench.enter(&env, &opened, vec![field.with(value)?]).await;
            assert!(
                wanted.contains(&entry.outcome),
                "{place:?} {mode}: {entry:?}"
            );
            assert_eq!(entry.typed, vec![value.len()], "{place:?} {mode}");
            bench.close(opened).await?;
        }

        let opened = bench.open("/private/moved.html?mode=pw", place).await?;
        let email = bench.r#ref(&opened, "textbox", "Email").await?;
        let password = bench.r#ref(&opened, "textbox", "Password").await?;
        let env = bench.host(&opened);
        let fields = vec![
            Field::new(&email, origin).with(EMAIL)?,
            Field::new(&password, origin).with("CANARY-moved-pw")?,
        ];
        let entry = bench.enter(&env, &opened, fields).await;
        // Exactly this: the read-back sees the page moving even when the box still holds the
        // value (ruling R38), so it can never say `entered` for a form that already left.
        assert_eq!(
            entry.outcome,
            EntryOutcome::EnteredPageMoved,
            "{place:?} first-event submit: {entry:?}"
        );
        assert_eq!(
            entry.delivery,
            Delivery::Done,
            "{place:?} first-event submit"
        );
        bench.close(opened).await?;

        let opened = bench.open("/private/moved.html?mode=focus", place).await?;
        let box_ref = bench.r#ref(&opened, "textbox", "Verification code").await?;
        let env = bench.host(&opened);
        let field = Field::new(&box_ref, origin).keys().with("246813")?;
        let entry = bench.enter(&env, &opened, vec![field]).await;
        assert_ne!(
            entry.outcome,
            EntryOutcome::EnteredPageMoved,
            "{place:?} submit on focus"
        );
        assert_ne!(
            entry.outcome,
            EntryOutcome::Entered,
            "{place:?} submit on focus: {entry:?}"
        );
        assert_eq!(
            entry.typed,
            vec![0],
            "{place:?} submit on focus: nothing typed"
        );
        bench.close(opened).await?;
    }
    Ok(())
}

/// The site replaces the box after the third of six keys; the focus check
/// before the fourth stops the entry: `uncertain`, delivery unknown, three characters typed.
/// Six one-digit boxes the form removes after the third digit: `uncertain` at the third.
/// Six one-digit boxes that stay: each holds its digit.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_that_goes_mid_entry_is_uncertain() -> TestResult {
    let bench = Bench::start().await?;
    let origin = &bench.page_origin;
    for place in PLACES {
        let opened = bench.open("/private/moved.html?mode=swap3", place).await?;
        let box_ref = bench.r#ref(&opened, "textbox", "Verification code").await?;
        let env = bench.host(&opened);
        let field = Field::new(&box_ref, origin).keys().with("246813")?;
        let entry = bench.enter(&env, &opened, vec![field]).await;
        assert_eq!(
            entry.outcome,
            EntryOutcome::Uncertain,
            "{place:?} replaced box: {entry:?}"
        );
        assert_eq!(code(&entry), Some("focus_lost"), "{place:?} replaced box");
        assert_eq!(entry.delivery, Delivery::Unknown, "{place:?} replaced box");
        assert_eq!(entry.typed, vec![3], "{place:?} replaced box");
        bench.close(opened).await?;

        for (mode, wanted) in [
            ("sixstay", EntryOutcome::Entered),
            ("six", EntryOutcome::Uncertain),
        ] {
            let opened = bench
                .open(&format!("/private/moved.html?mode={mode}"), place)
                .await?;
            let mut digits = Vec::new();
            let mut fields = Vec::new();
            for (at, digit) in "246813".chars().enumerate() {
                let r#ref = bench
                    .r#ref(&opened, "textbox", &format!("Digit {}", at + 1))
                    .await?;
                fields.push(Field::new(&r#ref, origin).keys().with(&digit.to_string())?);
                digits.push(r#ref);
            }
            let env = bench.host(&opened);
            let entry = bench.enter(&env, &opened, fields).await;
            assert_eq!(entry.outcome, wanted, "{place:?} {mode}: {entry:?}");
            if wanted == EntryOutcome::Entered {
                assert_eq!(entry.typed, vec![1; 6]);
                for (r#ref, digit) in digits.iter().zip("246813".chars()) {
                    assert_eq!(bench.value(&opened, r#ref).await?, digit.to_string());
                }
            } else {
                assert_eq!((entry.at, code(&entry)), (Some(2), Some("page_moved")));
                assert_eq!(entry.typed, vec![1, 1, 1, 0, 0, 0]);
            }
            bench.close(opened).await?;
        }
    }
    Ok(())
}

/// Boxes in cross-site frames, each looked at in its own frame and
/// typed on that frame's own session, on a tab in front and a tab behind.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_boxes_in_cross_site_frames_are_entered() -> TestResult {
    let bench = Bench::start().await?;
    let other = &bench.other_origin;
    for place in PLACES {
        let opened = bench
            .open(
                "/private/xframe.html?src=card-field.html%3Ffield%3Dnumber",
                place,
            )
            .await?;
        let number = bench.r#ref(&opened, "textbox", "Card number").await?;
        let env = bench.host(&opened);
        let field = Field::new(&number, other).spaced().with(CARD)?;
        let entry = bench.enter(&env, &opened, vec![field]).await;
        assert_eq!(
            entry.outcome,
            EntryOutcome::Entered,
            "{place:?} cross-site card: {entry:?}"
        );
        assert_eq!(
            bench.value(&opened, &number).await?,
            CARD,
            "{place:?} cross-site card"
        );
        bench.close(opened).await?;

        let opened = bench.open("/private/card-frames.html", place).await?;
        let mut fields = Vec::new();
        let mut refs = Vec::new();
        for (name, value) in [
            ("Card number", CARD),
            ("Expiration date", "07/31"),
            ("Security code", "737"),
        ] {
            let r#ref = bench.r#ref(&opened, "textbox", name).await?;
            fields.push(Field::new(&r#ref, other).spaced().with(value)?);
            refs.push((r#ref, value));
        }
        let env = bench.host(&opened);
        let entry = bench.enter(&env, &opened, fields).await;
        assert_eq!(
            entry.outcome,
            EntryOutcome::Entered,
            "{place:?} cross-site code: {entry:?}"
        );
        assert_eq!(entry.typed, vec![16, 5, 3]);
        for (r#ref, value) in &refs {
            assert_eq!(
                bench.value(&opened, r#ref).await?,
                *value,
                "{place:?} cross-site code"
            );
        }
        bench.close(opened).await?;

        let opened = bench.open("/private/xframe.html", place).await?;
        let email = bench.r#ref(&opened, "textbox", "Email").await?;
        let password = bench.r#ref(&opened, "textbox", "Password").await?;
        let env = bench.host(&opened);
        let fields = vec![
            Field::new(&email, other).with(EMAIL)?,
            Field::new(&password, other).with("CANARY-xframe-pw")?,
        ];
        let entry = bench.enter(&env, &opened, fields).await;
        assert_eq!(
            entry.outcome,
            EntryOutcome::Entered,
            "{place:?} cross-site sign-in: {entry:?}"
        );
        assert_eq!(bench.value(&opened, &password).await?, "CANARY-xframe-pw");
        bench.close(opened).await?;

        let opened = bench
            .open("/private/xframe.html?src=moved.html%3Fmode%3Dstay", place)
            .await?;
        let code_box = bench.r#ref(&opened, "textbox", "Verification code").await?;
        let env = bench.host(&opened);
        let field = Field::new(&code_box, other).keys().with("246813")?;
        let entry = bench.enter(&env, &opened, vec![field]).await;
        assert_eq!(
            entry.outcome,
            EntryOutcome::Entered,
            "{place:?} cross-site password: {entry:?}"
        );
        assert_eq!(bench.value(&opened, &code_box).await?, "246813");
        bench.close(opened).await?;
    }
    Ok(())
}

/// The card box hands focus to another box as it takes it: `not_focused`, nothing in either box.
/// Or the page takes the focus back into its own box when the frame takes it, a timer later: then
/// `not_focused` with nothing typed, the whole value entered in the frame's box, or
/// `value_not_taken` with the frame's box empty, as the page's race falls; never a part of the
/// value, and never anything in the thief's box.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_focus_thief_gets_nothing() -> TestResult {
    let bench = Bench::start().await?;
    let other = &bench.other_origin;
    for place in PLACES {
        // Repeat the focus races: a thief that wins one race in five still fails.
        for round in 1..=5 {
            let opened = bench
                .open("/private/card-frames.html?swap=number", place)
                .await?;
            let number = bench.r#ref(&opened, "textbox", "Card number").await?;
            let thief = bench.r#ref(&opened, "textbox", "Other").await?;
            let env = bench.host(&opened);
            let entry = bench
                .enter(&env, &opened, vec![Field::new(&number, other).with(CARD)?])
                .await;
            assert_eq!(
                entry.outcome,
                EntryOutcome::Refused,
                "{place:?} focus transfer #{round}: {entry:?}"
            );
            assert_eq!(
                code(&entry),
                Some("not_focused"),
                "{place:?} focus transfer #{round}"
            );
            assert_eq!(
                entry.delivery,
                Delivery::Done,
                "{place:?}: the focus reached the page"
            );
            assert_eq!(
                entry.error.as_ref().map(|error| error.retryable),
                Some(false)
            );
            assert_eq!(
                bench.value(&opened, &number).await?,
                "",
                "{place:?} focus transfer #{round}"
            );
            assert_eq!(
                bench.value(&opened, &thief).await?,
                "",
                "{place:?} focus transfer #{round}"
            );
            bench.close(opened).await?;

            let opened = bench
                .open(
                    "/private/xframe.html?steal=1&src=card-field.html%3Ffield%3Dnumber",
                    place,
                )
                .await?;
            let number = bench.r#ref(&opened, "textbox", "Card number").await?;
            let thief = bench.r#ref(&opened, "textbox", "Search the store").await?;
            let env = bench.host(&opened);
            let entry = bench
                .enter(&env, &opened, vec![Field::new(&number, other).with(CARD)?])
                .await;
            // The page takes the focus back a timer after its window blurs, so the race is the
            // page's, and Neo's text goes on the frame's own connection: it lands whole in the
            // frame's box or nowhere. Before Neo's focus check the thief wins (`not_focused`,
            // nothing sent); after the insertion the box holds the value (entered); between the
            // two the text finds no focus, and the read-back answers `value_not_taken` with an
            // empty box. Never a part of the value, and never anything in the thief's box.
            let held = bench.value(&opened, &number).await?;
            let stolen = bench.value(&opened, &thief).await?;
            let answered = match entry.outcome {
                EntryOutcome::Refused => code(&entry) == Some("not_focused") && held.is_empty(),
                EntryOutcome::Entered => held == CARD,
                EntryOutcome::Uncertain => {
                    code(&entry) == Some("value_not_taken") && held.is_empty()
                }
                _ => false,
            };
            assert!(
                answered && stolen.is_empty(),
                "{place:?} focus thief #{round}: {entry:?}; the frame's box holds {} characters, \
                 the thief's {}",
                held.chars().count(),
                stolen.chars().count()
            );
            bench.close(opened).await?;
        }
    }
    Ok(())
}

/// Gap 2: the page takes the focus into its own box after the third key of a card number typed
/// key by key into its cross-site frame: the entry stops there, and the thief's box stays empty.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_focus_stolen_mid_entry_stops_it() -> TestResult {
    let bench = Bench::start().await?;
    let other = &bench.other_origin;
    for place in PLACES {
        let opened = bench
            .open(
                "/private/xframe.html?stealafter=1&src=card-field.html%3Ffield%3Dnumber%26tell%3D3",
                place,
            )
            .await?;
        let number = bench.r#ref(&opened, "textbox", "Card number").await?;
        let thief = bench.r#ref(&opened, "textbox", "Search the store").await?;
        let env = bench.host(&opened);
        let field = Field::new(&number, other).keys().with(CARD)?;
        let entry = bench.enter(&env, &opened, vec![field]).await;
        assert_eq!(
            entry.outcome,
            EntryOutcome::Uncertain,
            "{place:?} stolen: {entry:?}"
        );
        assert_eq!(code(&entry), Some("focus_lost"), "{place:?} stolen");
        let typed = entry.typed.first().copied().unwrap_or(0);
        assert!(
            (3..=4).contains(&typed),
            "{place:?}: stopped at the theft: {typed}"
        );
        assert_eq!(
            bench.value(&opened, &thief).await?,
            "",
            "{place:?}: the thief got nothing"
        );
        bench.close(opened).await?;
    }
    Ok(())
}

/// The page rewrite case: a box that keeps less than what is typed, with no maxlength, holds
/// something else at the read-back: `uncertain`, `value_not_taken`, the whole value counted.
/// A code that cannot fit its maxlength is refused before any key.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_that_keeps_less_is_not_taken() -> TestResult {
    let bench = Bench::start().await?;
    let origin = &bench.page_origin;
    for place in PLACES {
        let opened = bench.open("/private/rewrite.html", place).await?;
        let box_ref = bench.r#ref(&opened, "textbox", "Short code").await?;
        let env = bench.host(&opened);
        for keys in [false, true] {
            let field = Field::new(&box_ref, origin);
            let field = if keys { field.keys() } else { field };
            let entry = bench
                .enter(&env, &opened, vec![field.with("246813")?])
                .await;
            assert_eq!(
                entry.outcome,
                EntryOutcome::Uncertain,
                "{place:?}: {entry:?}"
            );
            assert_eq!((entry.at, code(&entry)), (Some(0), Some("value_not_taken")));
            assert_eq!(entry.typed, vec![6], "{place:?}");
            assert_eq!(entry.delivery, Delivery::Unknown, "{place:?}");
            assert_eq!(bench.value(&opened, &box_ref).await?, "2468", "{place:?}");
        }
        bench.close(opened).await?;

        let opened = bench.open("/private/moved.html?mode=push4", place).await?;
        let box_ref = bench.r#ref(&opened, "textbox", "Verification code").await?;
        let env = bench.host(&opened);
        let field = Field::new(&box_ref, origin).keys().with("246813")?;
        let entry = bench.enter(&env, &opened, vec![field]).await;
        assert_eq!(
            code(&entry),
            Some("value_too_long"),
            "{place:?} oversized code: {entry:?}"
        );
        assert_eq!(entry.typed, vec![0], "{place:?} oversized code");
        bench.close(opened).await?;
    }
    Ok(())
}

/// The password box is swapped out as it takes the focus: the username is typed,
/// the entry stops at the password box with nothing typed into it or its copy: `partial`. The
/// six one-digit boxes whose fourth hands the focus back to the first once (sixonce): `entered`,
/// each box its digit. The focus step's `select()` already gives the fourth box the focus, which
/// spends the page's one hand-off before Neo's own focus and check, so the box holds it then.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_lost_at_its_focus_step_leaves_the_rest_partial() -> TestResult {
    let bench = Bench::start().await?;
    let origin = &bench.page_origin;
    for place in PLACES {
        let opened = bench.open("/private/swap.html", place).await?;
        let email = bench.r#ref(&opened, "textbox", "Email").await?;
        let password = bench.r#ref(&opened, "textbox", "Password").await?;
        let env = bench.host(&opened);
        let fields = vec![
            Field::new(&email, origin).with(EMAIL)?,
            Field::new(&password, origin).with(PASSWORD)?,
        ];
        let entry = bench.enter(&env, &opened, fields).await;
        assert_eq!(
            entry.outcome,
            EntryOutcome::Partial,
            "{place:?} replaced password: {entry:?}"
        );
        assert_eq!((entry.at, code(&entry)), (Some(1), Some("not_focused")));
        assert_eq!(
            entry.typed,
            vec![EMAIL.len(), 0],
            "{place:?} replaced password"
        );
        assert_eq!(
            entry.delivery,
            Delivery::Done,
            "{place:?} replaced password"
        );
        assert_eq!(
            bench.value(&opened, &email).await?,
            EMAIL,
            "{place:?} replaced password"
        );
        let copy = bench.r#ref(&opened, "textbox", "Password").await?;
        assert_eq!(
            bench.value(&opened, &copy).await?,
            "",
            "{place:?}: the copy got nothing"
        );
        bench.close(opened).await?;

        let opened = bench
            .open("/private/moved.html?mode=sixonce", place)
            .await?;
        let mut digits = Vec::new();
        let mut fields = Vec::new();
        for (at, digit) in "246813".chars().enumerate() {
            let name = format!("Digit {}", at + 1);
            let r#ref = bench.r#ref(&opened, "textbox", &name).await?;
            fields.push(Field::new(&r#ref, origin).keys().with(&digit.to_string())?);
            digits.push(r#ref);
        }
        let env = bench.host(&opened);
        let entry = bench.enter(&env, &opened, fields).await;
        assert_eq!(
            entry.outcome,
            EntryOutcome::Entered,
            "{place:?} sixonce: {entry:?}"
        );
        assert_eq!(entry.typed, vec![1; 6], "{place:?} sixonce");
        for (r#ref, held) in digits.iter().zip(["2", "4", "6", "8", "1", "3"]) {
            assert_eq!(
                bench.value(&opened, r#ref).await?,
                held,
                "{place:?} sixonce"
            );
        }
        bench.close(opened).await?;
    }
    Ok(())
}

/// Case 6b: a box with no size, typed key by key, is entered: it has nothing to show or to
/// cover and is reached by focus alone.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_with_no_size_is_entered_key_by_key() -> TestResult {
    let bench = Bench::start().await?;
    for place in PLACES {
        let opened = bench.open("/private/moved.html?mode=stay", place).await?;
        let box_ref = bench.r#ref(&opened, "textbox", "Verification code").await?;
        bench
            .live
            .main_world(
                &opened.page,
                "Object.assign(document.getElementById('code').style,
                   { width: '0', height: '0', padding: '0', border: '0' }); true",
            )
            .await?;
        let env = bench.host(&opened);
        let field = Field::new(&box_ref, &bench.page_origin)
            .keys()
            .with("246813")?;
        let entry = bench.enter(&env, &opened, vec![field]).await;
        assert_eq!(
            entry.outcome,
            EntryOutcome::Entered,
            "{place:?} 6b: {entry:?}"
        );
        assert_eq!(
            bench.value(&opened, &box_ref).await?,
            "246813",
            "{place:?} 6b"
        );
        bench.close(opened).await?;
    }
    Ok(())
}

/// Ruling R51, the first host's shape: a code box with no size (opacity 0) whose maxlength-6 is
/// full of an old value. The browser does not count the focus step's selection in such a box,
/// so without a key that empties it first the first new key empties it and is dropped
/// (246813 left 46813). Entered, the box holding the new code.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_full_box_with_no_size_takes_the_whole_new_code() -> TestResult {
    let bench = Bench::start().await?;
    for place in PLACES {
        let opened = bench.open("/private/stale.html?mode=host", place).await?;
        let code_box = bench.mint(&opened, "#code").await?;
        let env = bench.host(&opened);
        let field = Field::new(&code_box, &bench.page_origin)
            .keys()
            .with("246813")?;
        let entry = bench.enter(&env, &opened, vec![field]).await;
        assert_eq!(
            entry.outcome,
            EntryOutcome::Entered,
            "{place:?} R51: {entry:?}"
        );
        assert_eq!(entry.typed, vec![6], "{place:?}");
        assert_eq!(
            bench.value(&opened, &code_box).await?,
            "246813",
            "{place:?}"
        );
        bench.close(opened).await?;
    }
    Ok(())
}

/// The same full box with a size needs no key first: the selection is replaced by the first
/// character, by keys and by one insertion, and the page never sees it empty in between (a page
/// that submits on its first input event). One insertion is exactly one input event.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_full_box_with_a_size_is_replaced_without_emptying() -> TestResult {
    let bench = Bench::start().await?;
    for place in PLACES {
        for keys in [true, false] {
            let opened = bench.open("/private/stale.html?mode=sized", place).await?;
            let code_box = bench.mint(&opened, "#code").await?;
            let env = bench.host(&opened);
            let field = Field::new(&code_box, &bench.page_origin);
            let field = if keys { field.keys() } else { field };
            let entry = bench
                .enter(&env, &opened, vec![field.with("246813")?])
                .await;
            assert_eq!(
                entry.outcome,
                EntryOutcome::Entered,
                "{place:?} keys={keys}: {entry:?}"
            );
            assert_eq!(bench.value(&opened, &code_box).await?, "246813");
            let seen = bench.seen(&opened).await?;
            assert!(
                !seen.iter().any(String::is_empty),
                "{place:?} keys={keys}: {seen:?}"
            );
            if !keys {
                assert_eq!(
                    seen,
                    vec!["246813".to_owned()],
                    "{place:?}: one input event"
                );
            }
            bench.close(opened).await?;
        }
    }
    Ok(())
}

/// Six one-digit boxes with no size, built as the first host builds them (Backspace empties a
/// box and focuses the one before): one of them, then all of them, holding an old digit.
/// Entered, each box its digit.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_one_digit_boxes_holding_old_digits_take_the_new_code() -> TestResult {
    let bench = Bench::start().await?;
    for place in PLACES {
        for mode in ["digits1", "digits6"] {
            let opened = bench
                .open(&format!("/private/stale.html?mode={mode}"), place)
                .await?;
            let mut boxes = Vec::new();
            let mut fields = Vec::new();
            for (at, digit) in "246813".chars().enumerate() {
                let r#ref = bench.mint(&opened, &format!("#d{}", at + 1)).await?;
                fields.push(
                    Field::new(&r#ref, &bench.page_origin)
                        .keys()
                        .with(&digit.to_string())?,
                );
                boxes.push(r#ref);
            }
            let env = bench.host(&opened);
            let entry = bench.enter(&env, &opened, fields).await;
            assert_eq!(
                entry.outcome,
                EntryOutcome::Entered,
                "{place:?} {mode}: {entry:?}"
            );
            for (r#ref, digit) in boxes.iter().zip("246813".chars()) {
                assert_eq!(
                    bench.value(&opened, r#ref).await?,
                    digit.to_string(),
                    "{place:?} {mode}"
                );
            }
            bench.close(opened).await?;
        }
    }
    Ok(())
}

/// A box with no size whose page puts its text back on every input cannot be emptied: refused
/// before any character, `box_not_cleared`, and the box holds nothing of the value. One whose
/// form submits as it empties is refused `not_focused`, nothing of the value typed after it. A
/// number box with no size and an old value, and no maxlength, is entered.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_that_will_not_empty_is_refused_before_any_character() -> TestResult {
    let bench = Bench::start().await?;
    for place in PLACES {
        let opened = bench
            .open("/private/stale.html?mode=restore", place)
            .await?;
        let code_box = bench.mint(&opened, "#code").await?;
        let env = bench.host(&opened);
        let field = Field::new(&code_box, &bench.page_origin)
            .keys()
            .with("246813")?;
        let entry = bench.enter(&env, &opened, vec![field]).await;
        assert_eq!(entry.outcome, EntryOutcome::Refused, "{place:?}: {entry:?}");
        assert_eq!(code(&entry), Some("box_not_cleared"), "{place:?}");
        assert_eq!(entry.typed, vec![0], "{place:?}");
        assert_eq!(
            entry.delivery,
            Delivery::Done,
            "{place:?}: the keys went out"
        );
        let held = bench.value(&opened, &code_box).await?;
        assert!(
            !held.chars().any(|c| "246813".contains(c)),
            "{place:?}: {held:?}"
        );
        bench.close(opened).await?;

        // A page that submits its form when the box empties (into a named frame, so the box
        // stays): the form left with an empty box, and nothing of the value follows it.
        let opened = bench.open("/private/stale.html?mode=submit", place).await?;
        let code_box = bench.mint(&opened, "#code").await?;
        let env = bench.host(&opened);
        let field = Field::new(&code_box, &bench.page_origin)
            .keys()
            .with("246813")?;
        let entry = bench.enter(&env, &opened, vec![field]).await;
        assert_eq!(
            entry.outcome,
            EntryOutcome::Refused,
            "{place:?} submit: {entry:?}"
        );
        assert_eq!(code(&entry), Some("not_focused"), "{place:?} submit");
        assert_eq!(entry.typed, vec![0], "{place:?} submit");
        let held = bench.value(&opened, &code_box).await?;
        assert!(
            !held.chars().any(|c| "246813".contains(c)),
            "{place:?}: {held:?}"
        );
        let submitted = bench
            .live
            .main_world(&opened.page, "JSON.stringify(window.submitted)")
            .await?;
        assert_eq!(
            submitted.as_str(),
            Some(r#"[""]"#),
            "{place:?}: one submit, empty"
        );
        bench.close(opened).await?;

        let opened = bench.open("/private/stale.html?mode=number", place).await?;
        let code_box = bench.mint(&opened, "#code").await?;
        let env = bench.host(&opened);
        let field = Field::new(&code_box, &bench.page_origin)
            .keys()
            .with("246813")?;
        let entry = bench.enter(&env, &opened, vec![field]).await;
        assert_eq!(
            entry.outcome,
            EntryOutcome::Entered,
            "{place:?} number: {entry:?}"
        );
        assert_eq!(
            bench.value(&opened, &code_box).await?,
            "246813",
            "{place:?} number"
        );
        bench.close(opened).await?;
    }
    Ok(())
}

/// The id of `session`, as the recording keeps it.
fn session_id(session: &ProtocolSession) -> Option<String> {
    session.session_id().map(|id| id.as_str().to_owned())
}

/// Ruling R24, which connection carried the keys: every `Input.*` call of an entry into a box in
/// a cross-site frame goes out on that frame's own session (its R40 Shift press included), none
/// on the tab's root session; into a box of the page itself, every one on the root session. On
/// a tab in front and behind, by one insertion and by keys.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_an_entry_sends_on_its_boxs_own_session() -> TestResult {
    let bench = Bench::start().await?;
    let other = &bench.other_origin;
    let origin = &bench.page_origin;
    for place in PLACES {
        for keys in [false, true] {
            let opened = bench
                .open(
                    "/private/xframe.html?src=card-field.html%3Ffield%3Dnumber",
                    place,
                )
                .await?;
            let number = bench.r#ref(&opened, "textbox", "Card number").await?;
            let env = bench.host(&opened);
            let field = Field::new(&number, other).spaced();
            let field = if keys { field.keys() } else { field };
            bench.recording.take();
            let entry = bench.enter(&env, &opened, vec![field.with(CARD)?]).await;
            let calls = bench.recording.take();
            assert_eq!(
                entry.outcome,
                EntryOutcome::Entered,
                "{place:?} keys={keys}: {entry:?}"
            );
            let root = session_id(&private_host::root_session(&bench.live, &opened.page).await?);
            let frame = session_id(&private_host::frame_session(&bench.live, &opened.page).await?);
            assert_ne!(root, frame, "the card frame is its own target");
            assert_sent_on(
                &calls,
                frame.as_ref(),
                keys,
                &format!("{place:?} keys={keys} frame"),
            );
            assert!(
                calls
                    .iter()
                    .any(|call| call.named_key.as_deref() == Some("Shift")),
                "{place:?} keys={keys}: R40's Shift press: {calls:?}"
            );
            bench.close(opened).await?;

            let opened = bench
                .open("/private/covered-login.html?mode=none", place)
                .await?;
            let email = bench.r#ref(&opened, "textbox", "Email").await?;
            let env = bench.host(&opened);
            let field = Field::new(&email, origin);
            let field = if keys { field.keys() } else { field };
            bench.recording.take();
            let entry = bench.enter(&env, &opened, vec![field.with(EMAIL)?]).await;
            let calls = bench.recording.take();
            assert_eq!(
                entry.outcome,
                EntryOutcome::Entered,
                "{place:?} keys={keys}: {entry:?}"
            );
            let root = session_id(&private_host::root_session(&bench.live, &opened.page).await?);
            assert_sent_on(
                &calls,
                root.as_ref(),
                keys,
                &format!("{place:?} keys={keys} page"),
            );
            bench.close(opened).await?;
        }
    }
    Ok(())
}

/// Every call in `calls` went out on `session`, and the text did: one `Input.insertText`, or
/// key events and no insertion.
fn assert_sent_on(calls: &[InputCall], session: Option<&String>, keys: bool, case: &str) {
    assert!(
        calls.iter().all(|call| call.session.as_ref() == session),
        "{case}: every Input call on {session:?}: {calls:?}"
    );
    let inserted = calls
        .iter()
        .filter(|call| call.method == "Input.insertText")
        .count();
    let pressed = calls
        .iter()
        .filter(|call| call.method == "Input.dispatchKeyEvent")
        .count();
    if keys {
        assert_eq!(inserted, 0, "{case}: {calls:?}");
        assert!(pressed > 2, "{case}: {calls:?}");
    } else {
        assert_eq!(inserted, 1, "{case}: {calls:?}");
    }
}

/// The value the stray-box rows type: its third character is the only one a page's keydown
/// handler sends into the neighbour.
const STRAY: &str = "CANARY-stray-31";

/// The node a snapshot answer prints for the box named `name`, as JSON: the first object in the
/// tree with a string field equal to `name`.
fn printed_node(tree: &Value, name: &str) -> Option<String> {
    match tree {
        Value::Object(fields) => {
            if fields.values().any(|field| field.as_str() == Some(name)) {
                return Some(tree.to_string());
            }
            fields.values().find_map(|field| printed_node(field, name))
        }
        Value::Array(items) => items.iter().find_map(|item| printed_node(item, name)),
        _ => None,
    }
}

/// Ruling R26, the stray box's mark: an entry the page sends into a neighbouring text box that
/// then holds the focus marks that box too, so every read Neo makes of it answers bullets, the
/// snapshot under the spec harness's conformance switch included (R44); the page's own world
/// still holds what it got. (a) keys: the box's keydown handler focuses the neighbour at the
/// third key, which lands there: `focus_lost`. (b) one insertion: the box's input handler moves
/// the value into the neighbour and focuses it: `value_not_taken`. (c) the same move with the
/// focus on a button marks nothing, but the neighbour holds the whole value the entry was given,
/// whose digest the read-back keeps (ruling R50), so it reads as bullets too. On a tab in front
/// and behind.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_the_page_sends_the_value_into_is_marked_when_it_holds_the_focus()
-> TestResult {
    let bench = Bench::start().await?;
    let third = STRAY
        .chars()
        .nth(2)
        .map(String::from)
        .ok_or("three characters")?;
    for place in PLACES {
        for (mode, keys, stop, neighbour_holds, masked) in [
            ("keys", true, "focus_lost", third.as_str(), true),
            ("insert", false, "value_not_taken", STRAY, true),
            ("button", false, "value_not_taken", STRAY, true),
        ] {
            let case = format!("{place:?} {mode}");
            let opened = bench
                .open(&format!("/private/stray.html?mode={mode}"), place)
                .await?;
            let code_box = bench.r#ref(&opened, "textbox", "Code").await?;
            let other = bench.r#ref(&opened, "textbox", "Other").await?;
            let env = bench.host(&opened);
            let field = Field::new(&code_box, &bench.page_origin);
            let field = if keys { field.keys() } else { field };
            let entry = bench.enter(&env, &opened, vec![field.with(STRAY)?]).await;
            assert_eq!(entry.outcome, EntryOutcome::Uncertain, "{case}: {entry:?}");
            assert_eq!(code(&entry), Some(stop), "{case}");
            assert_eq!(
                bench.value(&opened, &other).await?,
                neighbour_holds,
                "{case}: the page's own world holds what the neighbour got"
            );

            let page = opened.page.brand();
            let read = bench
                .live
                .invoke(
                    "locator.inputValue",
                    input_value(&ref_target(&page, &other)),
                )
                .await??;
            let bullets = "•".repeat(neighbour_holds.chars().count());
            let expected = if masked {
                bullets.as_str()
            } else {
                neighbour_holds
            };
            assert_eq!(read, json!(expected), "{case}: inputValue of the neighbour");
            for conformance in [false, true] {
                let tree = bench
                    .live
                    .invoke("page.ariaSnapshot", page_snapshot(&page, conformance))
                    .await??;
                let node = printed_node(&tree, "Other").ok_or("the snapshot prints Other")?;
                assert_eq!(
                    node.contains(neighbour_holds),
                    !masked,
                    "{case}: the neighbour (conformance {conformance}): {node}"
                );
                assert_eq!(
                    node.contains(&bullets),
                    masked,
                    "{case}: the neighbour (conformance {conformance}): {node}"
                );
            }
            bench.close(opened).await?;
        }
    }
    Ok(())
}

/// Ruling R51 with R24's recording: only a box with no size that holds text gets a key that
/// empties it, one `Delete` on the text's session before the first character. An empty box
/// with no size and a full box with a size get none. On a tab in front and behind, by keys.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_only_a_box_with_no_size_that_holds_text_gets_a_delete() -> TestResult {
    let bench = Bench::start().await?;
    for place in PLACES {
        for (mode, deletes) in [("host", 1), ("empty", 0), ("sized", 0)] {
            let case = format!("{place:?} {mode}");
            let opened = bench
                .open(&format!("/private/stale.html?mode={mode}"), place)
                .await?;
            let code_box = bench.mint(&opened, "#code").await?;
            let env = bench.host(&opened);
            let field = Field::new(&code_box, &bench.page_origin)
                .keys()
                .with("246813")?;
            bench.recording.take();
            let entry = bench.enter(&env, &opened, vec![field]).await;
            let calls = bench.recording.take();
            assert_eq!(entry.outcome, EntryOutcome::Entered, "{case}: {entry:?}");
            assert_eq!(bench.value(&opened, &code_box).await?, "246813", "{case}");
            let pressed = |key: &str| {
                calls
                    .iter()
                    .filter(|call| {
                        call.named_key.as_deref() == Some(key)
                            && call.kind.as_deref() != Some("keyUp")
                    })
                    .count()
            };
            assert_eq!(pressed("Delete"), deletes, "{case}: {calls:?}");
            assert_eq!(pressed("Backspace"), 0, "{case}: {calls:?}");
            let root = session_id(&private_host::root_session(&bench.live, &opened.page).await?);
            assert_sent_on(&calls, root.as_ref(), true, &case);
            bench.close(opened).await?;
        }
    }
    Ok(())
}

/// A lock before the entry refuses it with nothing sent; a lock taken mid-entry stops it at the
/// next character, with the policy's own owner and code, and what went in is counted.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_lock_stops_the_entry() -> TestResult {
    let bench = Bench::start().await?;
    let origin = &bench.page_origin;
    let opened = bench
        .open("/private/moved.html?mode=stay", Place::Front)
        .await?;
    let box_ref = bench.r#ref(&opened, "textbox", "Verification code").await?;

    let env = bench.host(&opened).locked_at(1);
    let field = Field::new(&box_ref, origin).keys().with("246813")?;
    let entry = bench.enter(&env, &opened, vec![field]).await;
    assert_eq!(entry.outcome, EntryOutcome::Refused, "{entry:?}");
    assert_eq!(code(&entry), Some("owner_has_control"));
    let owner = entry.error.as_ref().map(|error| error.owner);
    assert_eq!(owner, Some(ErrorOwner::Host), "the policy's own owner");
    assert_eq!(entry.delivery, Delivery::NotSent);
    assert_eq!(
        entry.error.as_ref().map(|error| error.retryable),
        Some(true)
    );
    assert!(!env.was_sent());

    // Ask 1 is the door's before sending, ask 2 the one before the box; asks 3, 4 and 5 come
    // before the first three keys.
    let env = bench.host(&opened).locked_at(5);
    let field = Field::new(&box_ref, origin).keys().with("246813")?;
    let entry = bench.enter(&env, &opened, vec![field]).await;
    assert_eq!(entry.outcome, EntryOutcome::Uncertain, "{entry:?}");
    assert_eq!(code(&entry), Some("owner_has_control"));
    let owner = entry.error.as_ref().map(|error| error.owner);
    assert_eq!(owner, Some(ErrorOwner::Host), "the policy's own owner");
    assert_eq!(entry.typed, vec![2]);
    assert_eq!(
        entry.delivery,
        Delivery::Done,
        "a refusal Neo saw end is definite"
    );
    assert_eq!(bench.value(&opened, &box_ref).await?, "24");
    bench.close(opened).await
}

/// The entry ends by its own deadline and still answers.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_entry_stops_at_its_deadline() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/moved.html?mode=stay", Place::Front)
        .await?;
    let box_ref = bench.r#ref(&opened, "textbox", "Verification code").await?;
    let env = bench.host(&opened);
    let request = EntryRequest {
        tab: opened.page.tab,
        url: None,
        fields: vec![
            Field::new(&box_ref, &bench.page_origin)
                .keys()
                .with("246813")?,
        ],
        deadline: Instant::now(),
    };
    let entry = bench.live.playwright.enter_privately(&env, request).await;
    assert_eq!(code(&entry), Some("run_timeout"), "{entry:?}");
    assert_eq!(entry.outcome, EntryOutcome::Refused, "nothing was typed");
    assert_eq!(entry.delivery, Delivery::NotSent);
    assert_eq!(entry.typed, vec![0]);
    assert_eq!(entry.error.map(|error| error.retryable), Some(true));
    assert_eq!(bench.value(&opened, &box_ref).await?, "");
    bench.close(opened).await
}

/// A page that copies what is typed into the box's `value` attribute: the entry notes no
/// element preview and no value, and its answer holds none.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_value_reaches_no_note_and_no_answer() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench.open("/private/mirror.html", Place::Front).await?;
    let password = bench.r#ref(&opened, "textbox", "Password").await?;
    let env = bench.host(&opened);
    let canary = "CANARY-mirror-pw";
    for keys in [false, true] {
        let field = Field::new(&password, &bench.page_origin);
        let field = if keys { field.keys() } else { field };
        let entry = bench.enter(&env, &opened, vec![field.with(canary)?]).await;
        assert_eq!(entry.outcome, EntryOutcome::Entered, "{entry:?}");
        assert!(!format!("{entry:?}").contains("CANARY"), "{entry:?}");
    }
    let notes = env.notes();
    assert!(
        notes
            .iter()
            .all(|line| !line.contains("CANARY") && !line.contains('<')),
        "{notes:#?}"
    );
    bench.close(opened).await
}

/// The cost of a character typed key by key: 16 characters with the host's rules and the
/// focus checked before each. Design section 13 asks for it; the report is the assertion's
/// message when it is too slow, and the bound is generous for a loaded machine.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_character_costs_a_few_browser_messages() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/moved.html?mode=stay", Place::Front)
        .await?;
    let box_ref = bench.r#ref(&opened, "textbox", "Verification code").await?;
    let env = bench.host(&opened);
    let field = Field::new(&box_ref, &bench.page_origin).keys().with(CARD)?;
    let started = Instant::now();
    let entry = bench.enter(&env, &opened, vec![field]).await;
    let took = started.elapsed();
    assert_eq!(entry.outcome, EntryOutcome::Entered, "{entry:?}");
    let per_character = took / 16;
    assert!(
        per_character < Duration::from_millis(250),
        "16 characters key by key took {took:?}, {per_character:?} each"
    );
    // The measurement design section 13 asks for, in the suite's output.
    #[expect(
        clippy::print_stderr,
        reason = "the measured cost is this test's report"
    )]
    {
        eprintln!("cost of a character, key by key: {per_character:?} each, 16 in {took:?}");
    }
    bench.close(opened).await
}

/// A value the entry types into a box the page does not declare private, so whatever masks it
/// is the mark alone.
const MARKED: &str = "CANARY-marked-4242";

/// The diagnostic a real host operation returned and the call-log lines its host received.
struct PreviewFailure {
    message: String,
    notes: Vec<String>,
}

async fn preview_failure(
    bench: &Bench,
    opened: &Opened,
    operation: &str,
    input: Value,
) -> TestResult<PreviewFailure> {
    let operation = Op::from_name(operation).ok_or("the preview operation exists")?;
    let env = bench.host(opened);
    let error = bench
        .live
        .playwright
        .invoke(&env, operation, input)
        .await
        .err()
        .ok_or("the preview operation must fail")?;
    Ok(PreviewFailure {
        message: error.message().to_owned(),
        notes: env.notes(),
    })
}

fn assert_marked_preview(failure: &PreviewFailure) {
    let marker = format!("value=\"<{} characters>\"", MARKED.chars().count());
    assert!(failure.message.contains(&marker), "{}", failure.message);
    assert!(
        failure.notes.iter().any(|line| line.contains(&marker)),
        "the native call log contains the projected preview: {:?}",
        failure.notes,
    );
    for text in iter::once(&failure.message).chain(&failure.notes) {
        assert!(!text.contains(MARKED), "marked preview leaked: {text}");
    }
}

/// R52 covers marked boxes whose page mirrors input into markup. Run both host placements,
/// both conformance policies, direct previews and the engine's preview of an intercepting box.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
#[expect(
    clippy::too_many_lines,
    reason = "one real host fill feeds the preview and accessible-error matrices in both policies"
)]
async fn tests_that_marked_previews_hide_mirrored_attributes_in_every_diagnostic() -> TestResult {
    let bench = Bench::start().await?;
    for place in PLACES {
        let opened = bench
            .open("/private/covered-login.html?mode=none", place)
            .await?;
        let email = bench.r#ref(&opened, "textbox", "Email").await?;
        let password = bench.r#ref(&opened, "textbox", "Password").await?;
        bench.live.main_world(&opened.page, "(() => { const u = document.getElementById('user'); u.value = 'PublicAttrControl'; u.setAttribute('value', u.value); u.addEventListener('input', () => u.setAttribute('value', u.value)); const p = document.getElementById('pw'); p.value = 'DeclaredPreviewCanary'; p.setAttribute('value', p.value); return true; })()").await?;
        let page = opened.page.brand();
        let on_box = || ("target", ref_target(&page, &email));
        bench.live.main_world(&opened.page, r#"document.body.insertAdjacentHTML('beforeend', '<input aria-label="Error target" aria-invalid="true" aria-errormessage="marked-error-message"><div id="marked-error-message">Wrong: <button aria-labelledby="user">Error source</button></div><input id="marked-focus-victim" aria-label="Marked focus victim">'); document.getElementById('marked-focus-victim').addEventListener('focus', () => queueMicrotask(() => document.getElementById('user').focus())); true"#).await?;
        let error_ref = bench.r#ref(&opened, "textbox", "Error target").await?;
        let focus_victim = bench
            .r#ref(&opened, "textbox", "Marked focus victim")
            .await?;
        let hidden = || {
            expectation(
                on_box(),
                "toBeHidden",
                &json!({ "expression": "to.be.hidden", "isNot": false }),
            )
        };
        let control = preview_failure(&bench, &opened, "expect.locator", hidden()).await?;
        assert!(control.message.contains("value=\"PublicAttrControl\""));

        let env = bench.host(&opened);
        let entry = bench
            .enter(
                &env,
                &opened,
                vec![Field::new(&email, &bench.page_origin).with(MARKED)?],
            )
            .await;
        assert_eq!(entry.outcome, EntryOutcome::Entered, "{place:?}: {entry:?}");
        assert_eq!(
            bench
                .live
                .main_world(
                    &opened.page,
                    "document.getElementById('user').getAttribute('value')"
                )
                .await?,
            json!(MARKED),
            "the page really copied the host's typed value into the attribute",
        );
        for conformance in [false, true] {
            let fields = json!({
                "$neo": "locator", "page": page,
                "chunks": [{ "parts": [{ "name": "css", "body": [{ "simples": [{ "selector": { "css": "input", "functions": [] }, "combinator": "" }] }], "source": "input" }] }],
                "display": "locator('input')",
            });
            let mut input = expectation(
                ("target", fields),
                "toBeHidden",
                &json!({ "expression": "to.be.hidden", "isNot": false }),
            );
            input["call"]["conformance"] = json!(conformance);
            let strict = preview_failure(&bench, &opened, "expect.locator", input).await?;
            let marker = format!("value=\"<{} characters>\"", MARKED.len());
            assert!(
                strict.message.contains("strict mode violation"),
                "{}",
                strict.message
            );
            assert!(strict.message.contains(&marker), "{}", strict.message);
            for text in iter::once(&strict.message).chain(&strict.notes) {
                assert!(
                    !text.contains(MARKED),
                    "strict match preview leaked: {text}"
                );
            }
            for operation in ["locator.fill", "locator.pressSequentially"] {
                let mut input = json!({
                    "call": { "api": operation, "timeoutMs": 1000, "conformance": conformance },
                    "target": ref_target(&page, &focus_victim), "strict": "strict",
                });
                let argument = if operation == "locator.fill" {
                    "value"
                } else {
                    "text"
                };
                input[argument] = json!("Public attempt");
                let failure = preview_failure(&bench, &opened, operation, input).await?;
                let marker = format!("value=\"<{} characters>\"", MARKED.len());
                assert!(failure.message.contains(&marker), "{}", failure.message);
                assert!(failure.message.contains("focus"), "{}", failure.message);
                for text in iter::once(&failure.message).chain(&failure.notes) {
                    assert!(!text.contains(MARKED), "marked focus holder leaked: {text}");
                }
            }
            let mut error_input = expectation(
                ("target", ref_target(&page, &error_ref)),
                "toHaveAccessibleErrorMessage",
                &json!({ "expression": "to.have.accessible.error.message", "expectedText": [{ "string": "wrong" }], "isNot": false }),
            );
            error_input["call"]["conformance"] = json!(conformance);
            let mut matches = error_input.clone();
            let expected = format!("Wrong: {}", "•".repeat(MARKED.len()));
            matches["expectedText"] = json!([{ "string": expected }]);
            assert_eq!(
                bench.live.invoke("expect.locator", matches).await??,
                Value::Null
            );
            let error = preview_failure(&bench, &opened, "expect.locator", error_input).await?;
            assert!(error.message.contains(&expected), "{}", error.message);
            for text in iter::once(&error.message).chain(&error.notes) {
                assert!(
                    !text.contains(MARKED),
                    "marked error message leaked: {text}"
                );
            }
            for mut input in [
                hidden(),
                expectation(
                    on_box(),
                    "toHaveAttribute",
                    &json!({ "expression": "to.have.attribute.value", "expressionArg": "value", "expectedText": [{ "string": "wrong" }], "isNot": false }),
                ),
            ] {
                input["call"]["conformance"] = json!(conformance);
                assert_marked_preview(
                    &preview_failure(&bench, &opened, "expect.locator", input).await?,
                );
            }
            if conformance {
                let mut input = expectation(
                    ("target", ref_target(&page, &password)),
                    "toBeHidden",
                    &json!({ "expression": "to.be.hidden", "isNot": false }),
                );
                input["call"]["conformance"] = json!(true);
                let native = preview_failure(&bench, &opened, "expect.locator", input).await?;
                assert!(native.message.contains("value=\"DeclaredPreviewCanary\""));
                assert!(!native.message.contains(MARKED));
            }
        }

        bench.live.main_world(&opened.page, "(() => { const r = document.getElementById('user').getBoundingClientRect(); const d = document.createElement('div'); d.id = 'preview-cover'; d.style.cssText = `position:fixed;left:${r.left}px;top:${r.top}px;width:${r.width}px;height:${r.height}px;z-index:2147483000;background:red`; document.body.appendChild(d); return true; })()").await?;
        for conformance in [false, true] {
            let input = json!({ "call": { "api": "locator.click", "timeoutMs": 700, "conformance": conformance }, "target": ref_target(&page, &email), "strict": "strict" });
            let failure = preview_failure(&bench, &opened, "locator.click", input).await?;
            assert!(failure.message.contains("intercepts pointer events"));
            assert_marked_preview(&failure);
        }

        bench.live.main_world(&opened.page, "(() => { document.getElementById('preview-cover').remove(); const u = document.getElementById('user'); const r = u.getBoundingClientRect(); const b = document.createElement('button'); b.id = 'under-marked-field'; b.type = 'button'; b.textContent = 'Under marked field'; b.style.cssText = `position:fixed;left:${r.left}px;top:${r.top}px;width:${r.width}px;height:${r.height}px;z-index:2147483000`; document.body.appendChild(b); u.style.cssText = `position:fixed;box-sizing:border-box;left:${r.left}px;top:${r.top}px;width:${r.width}px;height:${r.height}px;z-index:2147483001`; return true; })()").await?;
        let button = json!({
            "$neo": "locator", "page": page,
            "chunks": [{ "parts": [{ "name": "css", "body": [{ "simples": [{ "selector": { "css": "#under-marked-field", "functions": [] }, "combinator": "" }] }], "source": "#under-marked-field" }] }],
            "display": "locator('#under-marked-field')",
        });
        for conformance in [false, true] {
            let input = json!({ "call": { "api": "locator.click", "timeoutMs": 700, "conformance": conformance }, "target": button, "strict": "strict" });
            let failure = preview_failure(&bench, &opened, "locator.click", input).await?;
            assert!(failure.message.contains("intercepts pointer events"));
            assert_marked_preview(&failure);
        }
        bench.close(opened).await?;
    }
    Ok(())
}

/// A `locator` handle on `page` for an aria ref.
fn ref_target(page: &Value, r#ref: &str) -> Value {
    json!({
        "$neo": "locator", "page": page,
        "chunks": [{ "parts": [{ "name": "aria-ref", "body": r#ref, "source": r#ref }] }],
        "display": format!("getByRef('{ref}')"),
    })
}

/// A `locator.inputValue` input for `target`.
fn input_value(target: &Value) -> Value {
    json!({
        "call": { "api": "locator.inputValue", "timeoutMs": 5_000 },
        "target": target, "strict": "strict",
    })
}

/// A `page.ariaSnapshot` input in ai mode, under the spec harness's switch or not.
fn page_snapshot(page: &Value, conformance: bool) -> Value {
    json!({
        "call": { "api": "page.ariaSnapshot", "timeoutMs": 10_000, "conformance": conformance },
        "page": page, "mode": "ai",
    })
}

/// An `expect.<matcher>` input that polls for 300 ms, with the matcher's own `params`.
fn expectation(target: (&str, Value), matcher: &str, params: &Value) -> Value {
    let mut input = json!({
        "call": { "api": format!("expect.{matcher}"), "timeoutMs": 300 },
        "matcher": matcher, "expectation": "", "expected": "",
    });
    if let Some(input) = input.as_object_mut() {
        input.insert(target.0.to_owned(), target.1);
        if let Some(params) = params.as_object() {
            input.extend(params.clone());
        }
    }
    input
}

/// Protected fill 4/5 (design decision 6, rulings R14, R44 and R47): every read Neo itself makes
/// of a box the entry typed a host's value into answers one bullet per character, on a tab in
/// front and behind. The box is the Email box, which the page does not declare private, so the
/// mark alone masks it: `inputValue()`, the snapshot with and without the spec harness's
/// conformance switch, and every failed assertion (the received value, the received aria tree,
/// the preview and the failure's snapshot). `toHaveValue` compares against the bullets. A box
/// the entry never typed into reads as the page holds it, and the page's own world (`evaluate`)
/// still holds the value, which is the page's and a documented limit.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
#[expect(
    clippy::too_many_lines,
    reason = "one table: each of Neo's reads of the same marked box, on a tab in front and behind"
)]
async fn tests_that_neos_own_reads_answer_bullets_for_a_privately_typed_box() -> TestResult {
    let bench = Bench::start().await?;
    let bullets = "•".repeat(MARKED.chars().count());
    for place in PLACES {
        let opened = bench
            .open("/private/covered-login.html?mode=none", place)
            .await?;
        let email = bench.r#ref(&opened, "textbox", "Email").await?;
        let password = bench.r#ref(&opened, "textbox", "Password").await?;
        let env = bench.host(&opened);
        let fields = vec![Field::new(&email, &bench.page_origin).with(MARKED)?];
        let entry = bench.enter(&env, &opened, fields).await;
        assert_eq!(entry.outcome, EntryOutcome::Entered, "{place:?}: {entry:?}");
        let page = opened.page.brand();
        let read = |op: &'static str, input: Value| {
            let live = &bench.live;
            async move { live.invoke(op, input).await }
        };

        let typed = read(
            "locator.inputValue",
            input_value(&ref_target(&page, &email)),
        )
        .await??;
        assert_eq!(typed, json!(bullets), "{place:?}: inputValue reads bullets");
        let untouched = read(
            "locator.inputValue",
            input_value(&ref_target(&page, &password)),
        )
        .await??;
        assert_eq!(
            untouched,
            json!(""),
            "{place:?}: a box the entry never typed into reads as the page holds it"
        );

        for conformance in [false, true] {
            // The answer is the frames' JSON tree, which the prelude renders: all of it.
            let printed = read("page.ariaSnapshot", page_snapshot(&page, conformance))
                .await??
                .to_string();
            assert!(
                !printed.contains(MARKED) && printed.contains(&bullets),
                "{place:?}: the snapshot (conformance {conformance}) prints bullets: {printed}"
            );
        }

        let missing = json!({
            "$neo": "locator", "page": page,
            "chunks": [{ "parts": [{
                "name": "css",
                "body": [{ "simples": [{
                    "selector": { "css": "#nothing-here", "functions": [] }, "combinator": "",
                }] }],
                "source": "#nothing-here",
            }] }],
            "display": "locator('#nothing-here')",
        });
        let on_box = || ("target", ref_target(&page, &email));
        let not_a_button = json!({
            "expression": "to.match.aria", "isNot": false,
            "expectedValue": { "kind": "role", "role": "button", "name": "No such button" },
        });
        let failing = [
            (
                "expect.locator",
                expectation(
                    on_box(),
                    "toHaveValue",
                    &json!({
                        "expression": "to.have.value", "isNot": false,
                        "expectedText": [{ "string": "something else" }],
                    }),
                ),
            ),
            (
                "expect.locator",
                expectation(
                    on_box(),
                    "toHaveText",
                    &json!({
                        "expression": "to.have.text", "isNot": false,
                        "expectedText": [{ "string": "x" }],
                    }),
                ),
            ),
            (
                "expect.locator",
                expectation(
                    on_box(),
                    "toHaveAttribute",
                    &json!({
                        "expression": "to.have.attribute.value", "isNot": false,
                        "expressionArg": "data-none", "expectedText": [{ "string": "x" }],
                    }),
                ),
            ),
            (
                "expect.locator",
                expectation(on_box(), "toMatchAriaSnapshot", &not_a_button),
            ),
            (
                "expect.page",
                expectation(("page", page.clone()), "toMatchAriaSnapshot", &not_a_button),
            ),
            (
                "expect.locator",
                expectation(
                    ("target", missing),
                    "toBeVisible",
                    &json!({
                        "expression": "to.be.visible", "isNot": false,
                    }),
                ),
            ),
        ];
        for (op, input) in failing {
            let matcher = input.get("matcher").cloned().unwrap_or_default();
            let error = match read(op, input).await? {
                Ok(passed) => return Err(format!("{place:?}: {matcher} passed: {passed}").into()),
                Err(error) => format!("{error:?}"),
            };
            assert!(
                !error.contains(MARKED) && error.contains(&bullets),
                "{place:?}: a failed {matcher} prints the box as bullets: {error}"
            );
        }
        let as_read = read(
            "expect.locator",
            expectation(
                on_box(),
                "toHaveValue",
                &json!({
                    "expression": "to.have.value", "isNot": false,
                    "expectedText": [{ "string": bullets }],
                }),
            ),
        )
        .await?;
        assert_eq!(
            as_read.ok(),
            Some(Value::Null),
            "{place:?}: toHaveValue compares against the bullets inputValue reads"
        );

        assert_eq!(
            bench.value(&opened, &email).await?,
            MARKED,
            "{place:?}: the page's own world still holds it"
        );
        bench.close(opened).await?;
    }
    Ok(())
}

/// Protected fill 4/5: a box in a cross-site frame, where a payment provider draws a card field
/// and where host fills happen, reads as bullets from `inputValue()` and in the page's stitched
/// snapshot, on a tab in front and behind.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_privately_typed_box_in_a_cross_site_frame_reads_as_bullets() -> TestResult {
    let bench = Bench::start().await?;
    let bullets = "•".repeat(CARD.chars().count());
    for place in PLACES {
        let opened = bench
            .open(
                "/private/xframe.html?src=card-field.html%3Ffield%3Dnumber",
                place,
            )
            .await?;
        let number = bench.r#ref(&opened, "textbox", "Card number").await?;
        let env = bench.host(&opened);
        let field = Field::new(&number, &bench.other_origin)
            .spaced()
            .with(CARD)?;
        let entry = bench.enter(&env, &opened, vec![field]).await;
        assert_eq!(entry.outcome, EntryOutcome::Entered, "{place:?}: {entry:?}");
        let page = opened.page.brand();
        let typed = bench
            .live
            .invoke(
                "locator.inputValue",
                input_value(&ref_target(&page, &number)),
            )
            .await??;
        assert_eq!(typed, json!(bullets), "{place:?}: inputValue reads bullets");
        let printed = bench
            .live
            .invoke("page.ariaSnapshot", page_snapshot(&page, false))
            .await??
            .to_string();
        assert!(
            !printed.contains(CARD) && printed.contains(&bullets),
            "{place:?}: the stitched snapshot prints the frame's box as bullets: {printed}"
        );
        assert_eq!(bench.value(&opened, &number).await?, CARD, "{place:?}");
        bench.close(opened).await?;
    }
    Ok(())
}

/// Rulings R46 and R58: once a tab took a private fill, a script's own `page.screenshot()` and
/// `locator.screenshot()` of it work under the host's default `fill.pictures: allowed`, and
/// under `never` are refused with one host-owned error, `private_tab`; another tab still
/// screenshots. Fails with the setting left out of the refusal (refused by default).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_script_takes_no_picture_of_a_tab_that_took_a_private_fill() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/covered-login.html?mode=none", Place::Front)
        .await?;
    let email = bench.r#ref(&opened, "textbox", "Email").await?;
    let env = bench.host(&opened);
    let fields = vec![Field::new(&email, &bench.page_origin).with(MARKED)?];
    let entry = bench.enter(&env, &opened, fields).await;
    assert_eq!(entry.outcome, EntryOutcome::Entered, "{entry:?}");
    let shot = |page: &Value, target: Option<Value>| {
        let mut input = json!({
            "call": { "api": "page.screenshot", "timeoutMs": 10_000 }, "page": page,
        });
        if let (Some(input), Some(target)) = (input.as_object_mut(), target) {
            input.insert("target".to_owned(), target);
            input.insert("strict".to_owned(), json!("strict"));
        }
        input
    };
    let page = opened.page.brand();
    let shots = || {
        [
            ("page.screenshot", shot(&page, None)),
            (
                "locator.screenshot",
                shot(&page, Some(ref_target(&page, &email))),
            ),
        ]
    };
    for (what, input) in shots() {
        let pictured = bench.live.invoke("page.screenshot", input).await?;
        assert!(pictured.is_ok(), "{what} under the default: {pictured:?}");
    }
    bench.live.playwright.set_fill_pictures_never(true);
    for (what, input) in shots() {
        let Err(refused) = bench.live.invoke("page.screenshot", input).await? else {
            return Err(format!("{what} pictured a tab that took a private fill").into());
        };
        assert_eq!(
            (refused.owner(), refused.code()),
            (ErrorOwner::Host, "private_tab"),
            "{what}: {refused:?}"
        );
        assert_eq!(
            refused.message(),
            "This tab took a private fill, and the host set Neo to take no picture of such a \
             tab for the rest of its life. Trying again will not succeed. Read the page with \
             page.ariaSnapshot() instead.",
            "{what}"
        );
    }
    let other = bench.open("/private/look.html", Place::Front).await?;
    let pictured = bench
        .live
        .invoke("page.screenshot", shot(&other.page.brand(), None))
        .await?;
    assert!(
        pictured.is_ok(),
        "another tab still screenshots: {pictured:?}"
    );
    bench.close(other).await?;
    bench.close(opened).await
}

/// Protected fill 4/5 (ruling R47): an element whose accessible name is built from a box the
/// entry typed into (`aria-labelledby` walks into the box and takes its value) carries the box's
/// value in its name; a failed `toHaveAccessibleName` on it answers bullets.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_name_built_from_a_privately_typed_box_reads_as_bullets() -> TestResult {
    let bench = Bench::start().await?;
    let bullets = "•".repeat(MARKED.chars().count());
    let opened = bench.open("/private/labelled.html", Place::Front).await?;
    let code = bench.r#ref(&opened, "textbox", "Code").await?;
    let env = bench.host(&opened);
    let fields = vec![Field::new(&code, &bench.page_origin).with(MARKED)?];
    let entry = bench.enter(&env, &opened, fields).await;
    assert_eq!(entry.outcome, EntryOutcome::Entered, "{entry:?}");
    let page = opened.page.brand();
    let button = json!({
        "$neo": "locator", "page": page,
        "chunks": [{ "parts": [{
            "name": "css",
            "body": [{ "simples": [{
                "selector": { "css": "#go", "functions": [] }, "combinator": "",
            }] }],
            "source": "#go",
        }] }],
        "display": "locator('#go')",
    });
    let input = expectation(
        ("target", button),
        "toHaveAccessibleName",
        &json!({
            "expression": "to.have.accessible.name", "isNot": false,
            "expectedText": [{ "string": "x" }],
        }),
    );
    let error = match bench.live.invoke("expect.locator", input).await? {
        Ok(passed) => return Err(format!("toHaveAccessibleName passed: {passed}").into()),
        Err(error) => format!("{error:?}"),
    };
    assert!(
        !error.contains(MARKED) && error.contains(&bullets),
        "the name built from the box reads as bullets: {error}"
    );
    bench.close(opened).await
}

/// Ruling R45: a card box the page declares, in a cross-site frame, filled by a script: Neo's
/// reading of the tab is private while it holds the number, and clear once the script empties
/// it. The reading walks every frame, the cross-site one on its own session.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_card_number_a_script_typed_in_a_cross_site_frame_keeps_pictures_off()
-> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open(
            "/private/xframe.html?src=card-field.html%3Ffield%3Dnumber",
            Place::Front,
        )
        .await?;
    let number = bench.r#ref(&opened, "textbox", "Card number").await?;
    let page = opened.page.brand();
    let fill = |value: &str| {
        json!({
            "call": { "api": "locator.fill", "timeoutMs": 5_000 },
            "target": ref_target(&page, &number), "strict": "strict", "value": value,
        })
    };
    let playwright = &bench.live.playwright;
    assert!(
        matches!(
            playwright.picture_check(opened.page.tab).await,
            PictureCheck::Clear { .. }
        ),
        "an empty card box reads clear"
    );
    bench.live.invoke("locator.fill", fill(CARD)).await??;
    assert_eq!(
        playwright.picture_check(opened.page.tab).await,
        PictureCheck::Private,
        "a card number in a cross-site frame, painted in clear"
    );
    bench.live.invoke("locator.fill", fill("")).await??;
    assert!(
        matches!(
            playwright.picture_check(opened.page.tab).await,
            PictureCheck::Clear { .. }
        ),
        "emptied, the page reads clear again"
    );
    bench.close(opened).await
}

/// Ruling R45: the input count reaches the reading. A script types a card number and empties
/// it again between two looks, in the page's own document and in a cross-site frame: both looks
/// read clear, and they differ, so a picture taken between them is dropped.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_value_typed_and_emptied_between_two_looks_changes_the_reading() -> TestResult
{
    let bench = Bench::start().await?;
    for path in [
        "/private/card-field.html?field=number",
        "/private/xframe.html?src=card-field.html%3Ffield%3Dnumber",
    ] {
        let opened = bench.open(path, Place::Front).await?;
        let number = bench.r#ref(&opened, "textbox", "Card number").await?;
        let page = opened.page.brand();
        let fill = |value: &str| {
            json!({
                "call": { "api": "locator.fill", "timeoutMs": 5_000 },
                "target": ref_target(&page, &number), "strict": "strict", "value": value,
            })
        };
        let playwright = &bench.live.playwright;
        let before = playwright.picture_check(opened.page.tab).await;
        bench.live.invoke("locator.fill", fill(CARD)).await??;
        bench.live.invoke("locator.fill", fill("")).await??;
        let after = playwright.picture_check(opened.page.tab).await;
        assert!(
            matches!(
                (&before, &after),
                (PictureCheck::Clear { reading: a }, PictureCheck::Clear { reading: b }) if a != b
            ),
            "{path}: both looks clear and different: {before:?} then {after:?}"
        );
        bench.close(opened).await?;
    }
    Ok(())
}

/// A marked snapshot of `opened`: clickable marking reads the DOM capture and hands the closed
/// roots' seeds to the document's world, which opens them to its engine.
async fn marked_snapshot(bench: &Bench, opened: &Opened) -> TestResult {
    bench
        .live
        .invoke(
            "page.ariaSnapshot",
            json!({
                "call": { "api": "page.ariaSnapshot", "timeoutMs": 10_000 },
                "page": opened.page.brand(), "mode": "ai", "clickable": true,
            }),
        )
        .await??;
    Ok(())
}

/// The DOM capture a marked snapshot sends. The camera's walk on demand never sends one.
const CAPTURE: &str = "DOMSnapshot.captureSnapshot";
/// The call that names a host's closed root for the camera's walk on demand, once per host.
const DESCRIBE: &str = "DOM.describeNode";
/// The call that brings a closed root into Neo's world for that walk.
const RESOLVE: &str = "DOM.resolveNode";
/// The walk's object group going, released when the walk ends.
const RELEASE: &str = "Runtime.releaseObjectGroup";

/// `text` typed with the page's keyboard alone, as a script's `keyboard.type` does: trusted key
/// and input events into whatever holds the focus, with no locator and no snapshot.
async fn type_keys(bench: &Bench, opened: &Opened, text: &str) -> TestResult {
    bench
        .live
        .invoke(
            "keyboard.type",
            json!({
                "call": { "api": "keyboard.type", "timeoutMs": 0 },
                "page": opened.page.brand(), "text": text,
            }),
        )
        .await??;
    Ok(())
}

/// Whether the walk's object group was released within two seconds: its owner's drop spawns the
/// release, which may land after the reading returned.
async fn released(bench: &Bench) -> bool {
    let until = Instant::now() + Duration::from_secs(2);
    while bench.recording.calls(RELEASE) == 0 && Instant::now() < until {
        time::sleep(Duration::from_millis(20)).await;
    }
    bench.recording.calls(RELEASE) > 0
}

/// One key pressed with the page's keyboard alone (`keyboard.press`).
async fn press_key(bench: &Bench, opened: &Opened, key: &str) -> TestResult {
    bench
        .live
        .invoke(
            "keyboard.press",
            json!({
                "call": { "api": "keyboard.press", "timeoutMs": 0 },
                "page": opened.page.brand(), "key": key,
            }),
        )
        .await??;
    Ok(())
}

/// A clear reading's text, or an error naming what came instead.
fn clear(check: &PictureCheck, what: &str) -> TestResult<String> {
    match check {
        PictureCheck::Clear { reading } => Ok(reading.clone()),
        other => Err(format!("{what}: a clear reading, not {other:?}").into()),
    }
}

/// The camera's walk on demand (ruling R65), row 1: a card number typed into a box inside a
/// closed shadow root by a script's keyboard alone, no snapshot and no locator, then the focus
/// taken away, so only the typing events bring the root: the next reading reads private. Main
/// reads clear, the box unknown until a snapshot walks the root. One host asked, one description,
/// no DOM capture, and the walk's object group released. Emptied, the page reads clear, and the
/// host is not asked again. Fails with the walk left out (reads clear).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_typing_into_a_closed_root_reads_private_at_the_next_reading() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/closed-card.html", Place::Front)
        .await?;
    let playwright = &bench.live.playwright;
    let tab = opened.page.tab;
    clear(&playwright.picture_check(tab).await, "before typing")?;
    bench.live.main_world(&opened.page, "focusCard()").await?;
    type_keys(&bench, &opened, CARD).await?;
    bench.live.main_world(&opened.page, "blurCard()").await?;
    bench.recording.reset_calls();
    assert_eq!(
        playwright.picture_check(tab).await,
        PictureCheck::Private,
        "row 1: typing reached the closed root"
    );
    assert_eq!(bench.recording.calls(CAPTURE), 0, "no DOM capture");
    assert_eq!(bench.recording.calls(DESCRIBE), 1, "one host asked");
    assert!(released(&bench).await, "the walk's object group released");
    bench.live.main_world(&opened.page, "fillCard('')").await?;
    bench.recording.reset_calls();
    clear(&playwright.picture_check(tab).await, "emptied")?;
    assert_eq!(bench.recording.calls(DESCRIBE), 0, "a host is asked once");
    bench.close(opened).await
}

/// The walk on demand, row 2: a box that holds the focus at a reading is read, so a card number
/// typed before Neo's first reading in the document (before its listeners existed) reads private
/// while its box keeps the focus. The named limit beside it: the same, with the focus taken away
/// before that first reading, reads clear. Fails with the focus not followed (the first case
/// reads clear).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_holding_the_focus_at_a_reading_is_read() -> TestResult {
    let bench = Bench::start().await?;
    let playwright = &bench.live.playwright;
    for blurred in [false, true] {
        let opened = bench
            .open("/private/closed-card.html", Place::Front)
            .await?;
        bench.live.main_world(&opened.page, "focusCard()").await?;
        type_keys(&bench, &opened, CARD).await?;
        if blurred {
            bench.live.main_world(&opened.page, "blurCard()").await?;
        }
        bench.recording.reset_calls();
        let first = playwright.picture_check(opened.page.tab).await;
        if blurred {
            clear(
                &first,
                "typed before the first reading, the focus gone: the limit",
            )?;
        } else {
            assert_eq!(first, PictureCheck::Private, "the box holds the focus");
        }
        assert_eq!(bench.recording.calls(CAPTURE), 0, "no DOM capture");
        bench.close(opened).await?;
    }
    Ok(())
}

/// The walk on demand, row 3: a closed root the page attaches after the document's first
/// reading (a single-page checkout that mounts its card form later), then typed into, reads
/// private at the next reading. On the first head this was the limit: the walk was spent at the
/// first reading. Fails with the walk left out.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_root_attached_after_the_first_reading_is_read_once_typed_into() -> TestResult
{
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/closed-card.html", Place::Front)
        .await?;
    let playwright = &bench.live.playwright;
    clear(&playwright.picture_check(opened.page.tab).await, "first")?;
    bench
        .live
        .main_world(&opened.page, "attachSecond() && focusSecond()")
        .await?;
    type_keys(&bench, &opened, CARD).await?;
    bench.recording.reset_calls();
    assert_eq!(
        playwright.picture_check(opened.page.tab).await,
        PictureCheck::Private,
        "the late root, typed into"
    );
    assert_eq!(bench.recording.calls(CAPTURE), 0, "no DOM capture");
    bench.close(opened).await
}

/// The walk on demand, row 4: a box two closed roots deep, typed into, reads private at the next
/// reading, on a small page and on one of 75,000 elements (whose DOM capture takes about 930 ms):
/// the outer host from the typing, its root's focus to the inner host, two descriptions, each
/// host asked once, no capture. The reading's time is printed. Fails with one round a reading
/// (the inner root is read only at the reading after).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_nested_closed_roots_are_followed_on_a_large_page() -> TestResult {
    let bench = Bench::start().await?;
    let playwright = &bench.live.playwright;
    for nodes in [0, 75_000] {
        let opened = bench
            .open(
                &format!("/private/closed-nested.html?nodes={nodes}"),
                Place::Front,
            )
            .await?;
        clear(&playwright.picture_check(opened.page.tab).await, "first")?;
        bench.live.main_world(&opened.page, "focusDeep()").await?;
        type_keys(&bench, &opened, CARD).await?;
        bench.recording.reset_calls();
        let started = Instant::now();
        let reading = playwright.picture_check(opened.page.tab).await;
        let took = started.elapsed();
        #[expect(
            clippy::print_stderr,
            reason = "the walk's measured cost goes to the test output for the pull request"
        )]
        {
            eprintln!(
                "WALK ON DEMAND, two closed roots deep, {nodes} elements: the reading took {took:?}"
            );
        }
        assert_eq!(
            reading,
            PictureCheck::Private,
            "{nodes}: the inner root's box"
        );
        assert_eq!(
            bench.recording.calls(DESCRIBE),
            2,
            "{nodes}: each host once"
        );
        assert_eq!(bench.recording.calls(CAPTURE), 0, "{nodes}: no DOM capture");
        bench.recording.reset_calls();
        assert_eq!(
            playwright.picture_check(opened.page.tab).await,
            PictureCheck::Private
        );
        assert_eq!(
            bench.recording.calls(DESCRIBE),
            0,
            "{nodes}: not asked again"
        );
        bench.close(opened).await?;
    }
    Ok(())
}

/// The walk on demand, row 5: elements that take the focus and no input. A focusable div with no
/// shadow root, a key pressed on it: the first key moves the reading (the picture around it is
/// dropped), the div is asked once and nothing is resolved; a second key moves nothing and asks
/// nothing. A video with controls and a details element, whose roots the browser makes itself: a
/// key on each asks nothing, as no author root can sit on those tags, and the reading stays as it
/// was. Fails with the tag rule left out (the video and the details are asked).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_host_with_no_closed_root_is_asked_once() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/closed-plain.html", Place::Front)
        .await?;
    let playwright = &bench.live.playwright;
    let tab = opened.page.tab;
    let before = clear(&playwright.picture_check(tab).await, "before")?;
    bench
        .live
        .main_world(&opened.page, "focusOn('pad')")
        .await?;
    press_key(&bench, &opened, "a").await?;
    bench.recording.reset_calls();
    let first_key = clear(&playwright.picture_check(tab).await, "the first key")?;
    assert_ne!(before, first_key, "the first key is counted");
    assert_eq!(bench.recording.calls(DESCRIBE), 1, "the pad asked once");
    assert_eq!(bench.recording.calls(RESOLVE), 0, "no root to resolve");
    press_key(&bench, &opened, "b").await?;
    bench.recording.reset_calls();
    let second_key = clear(&playwright.picture_check(tab).await, "the second key")?;
    assert_eq!(first_key, second_key, "a host asked is not counted again");
    assert_eq!(bench.recording.calls(DESCRIBE), 0, "and not asked again");
    for id in ["v", "d"] {
        bench
            .live
            .main_world(&opened.page, &format!("focusOn('{id}')"))
            .await?;
        press_key(&bench, &opened, "a").await?;
    }
    bench.recording.reset_calls();
    let browser_roots = clear(&playwright.picture_check(tab).await, "video and details")?;
    assert_eq!(second_key, browser_roots, "their keys are not counted");
    assert_eq!(bench.recording.calls(DESCRIBE), 0, "neither is asked");
    assert_eq!(bench.recording.calls(RESOLVE), 0, "nothing resolved");
    assert_eq!(bench.recording.calls(CAPTURE), 0, "no DOM capture");
    bench.close(opened).await
}

/// The walk on demand, row 6: a host whose description fails (refused, or never answered, so the
/// walk's budget ends it) leaves the reading clear as on main, never unknown, and is not asked
/// again at the next reading, though its box keeps the focus. The first key still moves the
/// reading, so the picture around it is dropped whether or not the root is found. Fails with a
/// failed walk answered as unknown, and with a host asked again.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_failed_read_of_a_host_reads_as_main_and_is_not_asked_again() -> TestResult {
    let bench = Bench::start().await?;
    let playwright = &bench.live.playwright;
    for how in [Failing::Refuse, Failing::Hang] {
        let opened = bench
            .open("/private/closed-card.html", Place::Front)
            .await?;
        let before = clear(&playwright.picture_check(opened.page.tab).await, "before")?;
        bench.live.main_world(&opened.page, "focusCard()").await?;
        type_keys(&bench, &opened, CARD).await?;
        bench.recording.reset_calls();
        bench.recording.fail(DESCRIBE, how);
        let failed = playwright.picture_check(opened.page.tab).await;
        bench.recording.stop_failing();
        assert!(
            bench.recording.calls(DESCRIBE) > 0,
            "{how:?}: the host was asked"
        );
        let failed = clear(&failed, &format!("{how:?}: as on main"))?;
        assert_ne!(before, failed, "{how:?}: the first key moved the reading");
        assert!(
            released(&bench).await,
            "{how:?}: the walk's object group released"
        );
        bench.recording.reset_calls();
        clear(&playwright.picture_check(opened.page.tab).await, "next")?;
        assert_eq!(
            bench.recording.calls(DESCRIBE),
            0,
            "{how:?}: a host is asked once"
        );
        bench.close(opened).await?;
    }
    Ok(())
}

/// The walk on demand, row 7, a named limit: a card number the page's own script writes into a
/// box inside a closed root, with no event, and the box never held the focus: the reading is
/// clear, as on main, and nothing is asked.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_value_the_page_wrote_with_no_event_reads_clear() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/closed-card.html?fill=1", Place::Front)
        .await?;
    bench.recording.reset_calls();
    clear(
        &bench.live.playwright.picture_check(opened.page.tab).await,
        "the limit",
    )?;
    assert_eq!(bench.recording.calls(DESCRIBE), 0, "nothing asked");
    assert_eq!(bench.recording.calls(CAPTURE), 0, "no DOM capture");
    bench.close(opened).await
}

/// Waits until every frame of `opened` has loaded (or three seconds), asked of the page itself,
/// so a camera reading sees each frame's document.
async fn frames_loaded(bench: &Bench, opened: &Opened) -> TestResult {
    bench
        .live
        .main_world(
            &opened.page,
            "Promise.all([...document.querySelectorAll('iframe')].map((f) => new Promise((done) => { \
             let ready = false; try { ready = f.contentDocument?.readyState === 'complete' \
             && f.contentDocument.URL !== 'about:blank' } catch {} \
             if (ready) done(true); else { f.addEventListener('load', () => done(true)); \
             setTimeout(() => done(true), 3000) } }))).then(() => true)",
        )
        .await?;
    Ok(())
}

/// The walk on demand, row 8: a box inside a closed root of a child frame, typed into by a
/// script's keyboard alone, reads private: a same-site frame on the page's session, and a
/// cross-site frame on its own, each walked in its own world with no DOM capture. Fails with the
/// walk left out.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_typing_into_a_closed_root_in_a_frame_reads_private() -> TestResult {
    let bench = Bench::start().await?;
    let playwright = &bench.live.playwright;
    for (path, focus) in [
        ("/private/closed-frame.html", "focusFrameCard()"),
        (
            "/private/xframe.html?src=closed-card.html%3Ffocus%3D1",
            "document.getElementById('x').focus(); true",
        ),
    ] {
        let opened = bench.open(path, Place::Front).await?;
        frames_loaded(&bench, &opened).await?;
        clear(&playwright.picture_check(opened.page.tab).await, path)?;
        bench.live.main_world(&opened.page, focus).await?;
        type_keys(&bench, &opened, CARD).await?;
        bench.recording.reset_calls();
        assert_eq!(
            playwright.picture_check(opened.page.tab).await,
            PictureCheck::Private,
            "{path}: the frame's closed root, typed into"
        );
        assert_eq!(bench.recording.calls(CAPTURE), 0, "{path}: no DOM capture");
        bench.close(opened).await?;
    }
    Ok(())
}

/// Input is counted in a closed root a marked snapshot opened first (the snapshot design's
/// reviewer's finding D): at the document an event's path stops at a closed root's host, so only
/// a listener on the root sees the box, and `look` gives one to every closed root it enters,
/// whichever map holds it. One character typed and removed again between two readings moves the
/// second one; nothing is asked, as the root is known. Fails with the listener left to the
/// camera's own roots (the root a snapshot opened gets none).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_input_in_a_closed_root_a_snapshot_opened_moves_the_reading() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/closed-card.html", Place::Front)
        .await?;
    marked_snapshot(&bench, &opened).await?;
    let playwright = &bench.live.playwright;
    let first = clear(&playwright.picture_check(opened.page.tab).await, "first")?;
    bench.live.main_world(&opened.page, "focusCard()").await?;
    type_keys(&bench, &opened, "4").await?;
    press_key(&bench, &opened, "Backspace").await?;
    bench.recording.reset_calls();
    let second = clear(&playwright.picture_check(opened.page.tab).await, "second")?;
    assert_ne!(
        first, second,
        "the input inside the closed root was counted"
    );
    assert_eq!(
        bench.recording.calls(DESCRIBE),
        0,
        "a known root asks nothing"
    );
    bench.close(opened).await
}

/// The design keeper's probe page: a 150,000-element main document and five small same-process
/// frames with closed roots, answering one after another. On the first head the camera's capture
/// of that session cost the picture. The walk on demand sends nothing here, as nothing is typed
/// and no box holds the focus: both readings, 150 ms apart, are clear, with no DOM capture and no
/// description.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_huge_page_with_staggered_small_frames_keeps_its_pictures() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/huge-staggered.html", Place::Front)
        .await?;
    frames_loaded(&bench, &opened).await?;
    let playwright = &bench.live.playwright;
    bench.recording.reset_calls();
    let first = playwright.picture_check(opened.page.tab).await;
    time::sleep(Duration::from_millis(150)).await;
    let second = playwright.picture_check(opened.page.tab).await;
    clear(&first, "first")?;
    clear(&second, "second")?;
    assert_eq!(bench.recording.calls(CAPTURE), 0, "no DOM capture");
    assert_eq!(bench.recording.calls(DESCRIBE), 0, "nothing asked");
    bench.close(opened).await
}

/// Ruling R45's cost: one reading of a tab of ten card frames, every other one cross-site, the
/// first time (each frame's world installed) and the second; and one DOM capture of an ordinary
/// page, what a closed-root walk in the reading would add. Printed, not bounded.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_what_one_reading_of_ten_frames_costs() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/ten-frames.html?nodes=2500", Place::Front)
        .await?;
    // Every frame loaded, asked of the page itself, so no snapshot installs a frame's world
    // before the first reading is timed.
    for _ in 0..100 {
        let loaded = bench
            .live
            .main_world(
                &opened.page,
                "document.querySelectorAll('iframe[data-loaded]').length",
            )
            .await?;
        if loaded == json!(10) {
            break;
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    let playwright = &bench.live.playwright;
    let first = Instant::now();
    let reading = playwright.picture_check(opened.page.tab).await;
    let first = first.elapsed();
    let second = Instant::now();
    let again = playwright.picture_check(opened.page.tab).await;
    let second = second.elapsed();
    assert!(matches!(reading, PictureCheck::Clear { .. }), "{reading:?}");
    assert_eq!(reading, again, "nothing changed between the two readings");
    let session = bench
        .live
        .browser
        .pages
        .get_session(opened.page.id.clone())
        .await?;
    let captured = Instant::now();
    session
        .session
        .send_value(
            "DOMSnapshot.captureSnapshot",
            json!({ "computedStyles": [] }),
        )
        .await?;
    let captured = captured.elapsed();
    #[expect(
        clippy::print_stderr,
        reason = "the measured cost goes to the test output for the pull request"
    )]
    {
        eprintln!(
            "one reading of 10 frames: first {first:?}, second {second:?}; a DOM capture of the \
             page (2500 elements and its frames' documents): {captured:?}"
        );
    }
    bench.close(opened).await
}

/// Ruling R45 (found by the design's author): a child frame that moves to another document with
/// no op touching it leaves the camera a stale world. The reading drops it and installs afresh,
/// so the frame is listed again on its new document. Without that it would answer `Gone` at
/// every reading and, under R54, be left out for good: a payment frame Neo no longer sees.
/// The same-site frame is the strict case: it lives on the page's own session, which never
/// changes, so its stale world is always met. A cross-site frame may get a new session when it
/// moves (the browser's choice, varying by run) and then a fresh world under a new key.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_child_frame_that_moved_on_is_read_afresh() -> TestResult {
    let bench = Bench::start().await?;
    let playwright = &bench.live.playwright;
    // The card frame is the one document holding a private box (a card number, then a security
    // code), so a reading lists it alone: its frame, then its document and input count.
    let entries = |check: &PictureCheck| match check {
        PictureCheck::Clear { reading } => reading
            .split(';')
            .filter(|entry| !entry.is_empty())
            .filter_map(|entry| entry.split_once('='))
            .map(|(frame, rest)| (frame.to_owned(), rest.to_owned()))
            .collect::<Vec<_>>(),
        PictureCheck::Private | PictureCheck::Unknown => Vec::new(),
    };
    for path in [
        "/private/xframe.html?same=1&src=card-field.html%3Ffield%3Dnumber",
        "/private/xframe.html?src=card-field.html%3Ffield%3Dnumber",
    ] {
        let opened = bench.open(path, Place::Front).await?;
        let first = playwright.picture_check(opened.page.tab).await;
        let before = entries(&first);
        assert_eq!(
            before.len(),
            1,
            "{path}: the card frame is listed: {first:?}"
        );
        // The page moves the frame to another document with no op, and says once it loaded.
        bench
            .live
            .main_world(
                &opened.page,
                "(document.getElementById('x').onload = () => { window.moved = true; }, \
                  document.getElementById('x').src = \
                  document.getElementById('x').src.replace('number', 'cvc'), 1)",
            )
            .await?;
        let mut moved = false;
        for _ in 0..250 {
            moved = bench
                .live
                .main_world(&opened.page, "window.moved === true")
                .await?
                == json!(true);
            if moved {
                break;
            }
            time::sleep(Duration::from_millis(20)).await;
        }
        assert!(moved, "{path}: the frame's new document never loaded");
        // Its new document has loaded: the next readings list it again, on that document.
        let mut last = PictureCheck::Unknown;
        for _ in 0..50 {
            last = playwright.picture_check(opened.page.tab).await;
            let after = entries(&last);
            if after.len() == 1 && after != before {
                break;
            }
            time::sleep(Duration::from_millis(200)).await;
        }
        let after = entries(&last);
        assert!(
            after.len() == 1
                && before.first().map(|entry| &entry.0) == after.first().map(|entry| &entry.0)
                && before != after,
            "{path}: the moved frame is read afresh, on its new document: {first:?} then {last:?}"
        );
        bench.close(opened).await?;
    }
    Ok(())
}

/// What Neo's camera does with a picture, from two readings about a capture's time apart.
#[derive(Debug, PartialEq, Eq)]
enum Fate {
    Kept,
    Withheld,
    Dropped,
}

async fn fate(bench: &Bench, tab: TabId) -> Fate {
    let first = bench.live.playwright.picture_check(tab).await;
    time::sleep(Duration::from_millis(150)).await;
    let then = bench.live.playwright.picture_check(tab).await;
    match (first, then) {
        (PictureCheck::Clear { reading: a }, PictureCheck::Clear { reading: b }) if a == b => {
            Fate::Kept
        }
        (PictureCheck::Clear { .. }, PictureCheck::Clear { .. }) => Fate::Dropped,
        _ => Fate::Withheld,
    }
}

/// Ruling R54's own case: the first card ever typed on a checkout page, and the payment frame
/// removed, between the two looks. The frame holds an empty card box at the first look, so it is
/// listed by that alone (no input counted yet); gone at the second, it is not. The readings
/// differ and the picture is dropped. Ads churn beside it all along.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_first_card_typed_as_its_frame_leaves_drops_the_picture() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/checkout-ads.html", Place::Front)
        .await?;
    let number = bench.r#ref(&opened, "textbox", "Card number").await?;
    let page = opened.page.brand();
    let playwright = &bench.live.playwright;
    let first = playwright.picture_check(opened.page.tab).await;
    bench
        .live
        .invoke(
            "locator.fill",
            json!({
                "call": { "api": "locator.fill", "timeoutMs": 5_000 },
                "target": ref_target(&page, &number), "strict": "strict", "value": CARD,
            }),
        )
        .await??;
    bench.live.main_world(&opened.page, "leave()").await?;
    let then = playwright.picture_check(opened.page.tab).await;
    assert!(
        matches!(
            (&first, &then),
            (PictureCheck::Clear { reading: a }, PictureCheck::Clear { reading: b }) if a != b
        ),
        "the first card typed and the payment frame removed between the looks: dropped. \
         {first:?} then {then:?}"
    );
    bench.close(opened).await
}

/// Ruling R54 (the design's author's checkout): a card box in a cross-site payment frame beside
/// two ad frames that move to a new document every 120 ms. Ads hold no private box, so their
/// churn never enters a reading: with the box empty every picture is kept. With the number
/// shown, every one is withheld; emptied, kept again. A payment frame removed between the two
/// looks after the number was typed drops the picture; once it is gone, pictures are kept.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_ad_churn_keeps_pictures_and_a_payment_frame_does_not() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/checkout-ads.html", Place::Front)
        .await?;
    let number = bench.r#ref(&opened, "textbox", "Card number").await?;
    let tab = opened.page.tab;
    let page = opened.page.brand();
    let fill = |value: &str| {
        json!({
            "call": { "api": "locator.fill", "timeoutMs": 5_000 },
            "target": ref_target(&page, &number), "strict": "strict", "value": value,
        })
    };
    for round in 0..10 {
        assert_eq!(
            fate(&bench, tab).await,
            Fate::Kept,
            "box empty, ads churning #{round}"
        );
    }
    bench.live.invoke("locator.fill", fill(CARD)).await??;
    for round in 0..5 {
        assert_eq!(
            fate(&bench, tab).await,
            Fate::Withheld,
            "the number shows #{round}"
        );
    }
    bench.live.invoke("locator.fill", fill("")).await??;
    for round in 0..5 {
        assert_eq!(fate(&bench, tab).await, Fate::Kept, "emptied #{round}");
    }
    let first = bench.live.playwright.picture_check(tab).await;
    bench.live.invoke("locator.fill", fill(CARD)).await??;
    bench.live.main_world(&opened.page, "leave()").await?;
    let then = bench.live.playwright.picture_check(tab).await;
    assert!(
        matches!(
            (&first, &then),
            (PictureCheck::Clear { reading: a }, PictureCheck::Clear { reading: b }) if a != b
        ),
        "the number typed and the payment frame removed between the looks: dropped. \
         {first:?} then {then:?}"
    );
    for round in 0..5 {
        assert_eq!(
            fate(&bench, tab).await,
            Fate::Kept,
            "the frame gone #{round}"
        );
    }
    bench.close(opened).await
}

/// Ruling R54: a frame that does not answer in time still withholds; it is never left out as a
/// frame that has gone would be. The cross-site frame asks a connection this test holds open
/// with a synchronous request, which blocks its renderer for five seconds outside any script.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_frame_that_does_not_answer_withholds() -> TestResult {
    let hold = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://127.0.0.1:{}/hold", hold.local_addr()?.port());
    let (arrived, mut held) = oneshot::channel();
    let holding = tokio::spawn(async move {
        if let Ok((mut stream, _)) = hold.accept().await {
            let mut request = [0_u8; 1024];
            let _read = stream.read(&mut request).await;
            let _told = arrived.send(());
            time::sleep(Duration::from_secs(5)).await;
        }
    });
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/xframe.html?src=stall.html", Place::Front)
        .await?;
    let playwright = &bench.live.playwright;
    let first = playwright.picture_check(opened.page.tab).await;
    assert!(matches!(first, PictureCheck::Clear { .. }), "{first:?}");
    // Told again until the frame's document has loaded; once the held connection has its
    // request, the frame's renderer is blocked in it.
    let tell = format!(
        "(document.getElementById('x').contentWindow.postMessage({}, '*'), 1)",
        json!(url)
    );
    let mut blocked = false;
    for _ in 0..250 {
        bench.live.main_world(&opened.page, &tell).await?;
        if time::timeout(Duration::from_millis(20), &mut held)
            .await
            .is_ok()
        {
            blocked = true;
            break;
        }
    }
    assert!(blocked, "the frame never asked the held connection");
    assert_eq!(
        playwright.picture_check(opened.page.tab).await,
        PictureCheck::Unknown,
        "a frame that cannot answer in time withholds the picture"
    );
    // The connection closes and the frame runs again before the tab closes.
    holding.await?;
    bench.close(opened).await
}

/// A `locator` handle on `page` for a CSS path, one selector a frame: `["#inner", "#code"]` is
/// `#code` inside the frame `#inner` holds.
fn css_target(page: &Value, path: &[&str]) -> Value {
    let chunks: Vec<Value> = path
        .iter()
        .map(|css| {
            json!({ "parts": [{
                "name": "css",
                "body": [{ "simples": [{
                    "selector": { "css": css, "functions": [] }, "combinator": "",
                }] }],
                "source": css,
            }] })
        })
        .collect();
    json!({
        "$neo": "locator", "page": page, "chunks": chunks,
        "display": format!("locator('{}')", path.join(" >> ")),
    })
}

/// Runs `call` in the page's own world until it answers 1: a frame's document loads after its
/// page, and the fixture's moves answer 0 until it is there.
async fn page_does(bench: &Bench, opened: &Opened, call: &str) -> TestResult {
    for _ in 0..250 {
        if bench.live.main_world(&opened.page, call).await? == json!(1) {
            return Ok(());
        }
        time::sleep(Duration::from_millis(20)).await;
    }
    Err(format!("the page never did {call}").into())
}

/// The page's ai-mode snapshot as the agent gets it, all of its JSON.
async fn printed_snapshot(bench: &Bench, page: &Value) -> TestResult<String> {
    printed_in_run(bench, Arc::default(), page).await
}

/// The same, inside `run`: one agent run's caches, its frame tree among them, carry over.
async fn printed_in_run(bench: &Bench, run: Arc<RunState>, page: &Value) -> TestResult<String> {
    Ok(bench
        .live
        .invoke_in_run(run, "page.ariaSnapshot", page_snapshot(page, false))
        .await??
        .to_string())
}

/// Ruling R53: the mark travels with the node. A box the entry typed into, which the page then
/// moves into a same-origin frame, stays masked there: `inputValue()` reads bullets, a name built
/// from it prints bullets, and the snapshot never prints the value. Moved back, it is masked as
/// before. The mark lives only on the box's wrapper in Neo's isolated world: the page's own
/// world sees no property of Neo's on the box, and setting the key there marks nothing. Nothing
/// Neo prints names the key.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_mark_travels_with_a_box_moved_into_another_frame() -> TestResult {
    const KEY: &str = "neo.privatelyTyped";
    let bench = Bench::start().await?;
    let bullets = "•".repeat(MARKED.chars().count());
    let opened = bench.open("/private/moved-box.html", Place::Front).await?;
    let code = bench.r#ref(&opened, "textbox", "Code").await?;
    let env = bench.host(&opened);
    let entry = bench
        .enter(
            &env,
            &opened,
            vec![Field::new(&code, &bench.page_origin).with(MARKED)?],
        )
        .await;
    assert_eq!(entry.outcome, EntryOutcome::Entered, "{entry:?}");
    let page = opened.page.brand();
    let read = |path: &'static [&'static str]| {
        let live = &bench.live;
        let input = input_value(&css_target(&page, path));
        async move { live.invoke("locator.inputValue", input).await }
    };

    // (a) Into a same-origin frame.
    page_does(&bench, &opened, "moveIn()").await?;
    assert_eq!(
        read(&["#inner", "#code"]).await??,
        json!(bullets),
        "(a) inputValue() in the frame reads bullets"
    );
    let printed = printed_snapshot(&bench, &page).await?;
    assert!(
        !printed.contains(MARKED) && printed.contains(&format!("Send {bullets}")),
        "(a) the name built from the moved box prints bullets, the value nowhere: {printed}"
    );

    // (b) Back into the page.
    page_does(&bench, &opened, "moveOut()").await?;
    assert_eq!(
        read(&["#code"]).await??,
        json!(bullets),
        "(b) moved back, inputValue() reads bullets"
    );
    let printed = printed_snapshot(&bench, &page).await?;
    assert!(
        !printed.contains(MARKED) && printed.contains(&format!("Send {bullets}")),
        "(b) moved back, masked as before: {printed}"
    );

    // (d) The page's own world: no property of Neo's on the box, and the key set there on the
    // box the entry never typed into marks nothing.
    let own = bench
        .live
        .main_world(
            &opened.page,
            "(() => { const box = document.getElementById('code'); \
               return JSON.stringify([Object.getOwnPropertyNames(box), \
                 Object.getOwnPropertySymbols(box).map(String)]); })()",
        )
        .await?;
    assert_eq!(
        own,
        json!("[[],[]]"),
        "(d) the page sees nothing of Neo's on the box"
    );
    bench
        .live
        .main_world(
            &opened.page,
            &format!(
                "(() => {{ const plain = document.getElementById('plain'); \
                   plain.value = 'page-held'; \
                   Object.defineProperty(plain, Symbol.for('{KEY}'), {{ value: true }}); \
                   plain['{KEY}'] = true; return 1; }})()"
            ),
        )
        .await?;
    assert_eq!(
        read(&["#plain"]).await??,
        json!("page-held"),
        "(d) the key set in the page's world marks nothing"
    );

    // (e) Nothing Neo prints names the key: not the snapshot, not an element preview.
    let preview = preview_failure(
        &bench,
        &opened,
        "expect.locator",
        expectation(
            ("target", css_target(&page, &["#code"])),
            "toBeHidden",
            &json!({ "expression": "to.be.hidden", "isNot": false }),
        ),
    )
    .await?;
    for (what, text) in [("snapshot", &printed), ("preview", &preview.message)] {
        assert!(
            !text.contains(KEY) && !text.contains("privatelyTyped"),
            "(e) the {what} names no key of Neo's: {text}"
        );
    }
    bench.close(opened).await
}

/// Ruling R53: a box marked inside a frame (so its prototype is that frame's), which the page
/// moves into its own document before removing the frame. A later agent run (another script)
/// learns the tab's frames afresh: after a `page.evaluate`, which stores the tab's frame tree in
/// the run, the tree proves one frame and a snapshot would skip the document-wide field scan.
/// The tab took a private entry, so the scan still runs and guards the box's foreign
/// prototype: a name built from it prints bullets.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_marked_in_a_removed_frame_stays_masked() -> TestResult {
    let bench = Bench::start().await?;
    let bullets = "•".repeat(MARKED.chars().count());
    let opened = bench
        .open("/private/moved-box.html?inside=1", Place::Front)
        .await?;
    let code = bench.r#ref(&opened, "textbox", "Code").await?;
    let env = bench.host(&opened);
    let entry = bench
        .enter(
            &env,
            &opened,
            vec![Field::new(&code, &bench.page_origin).with(MARKED)?],
        )
        .await;
    assert_eq!(entry.outcome, EntryOutcome::Entered, "{entry:?}");
    let page = opened.page.brand();
    page_does(&bench, &opened, "moveOut()").await?;
    page_does(&bench, &opened, "dropFrame()").await?;
    // A run that starts after the frame left and evaluates first, as a script may.
    let run: Arc<RunState> = Arc::default();
    bench
        .live
        .invoke_in_run(
            Arc::clone(&run),
            "page.evaluate",
            json!({
                "call": { "api": "page.evaluate", "timeoutMs": 5_000 }, "page": page,
                "expression": "() => 1", "isFunction": true,
                "arg": { "value": r#"{"v":"undefined"}"#, "handles": [] },
            }),
        )
        .await??;
    for round in 0..3 {
        let printed = printed_in_run(&bench, Arc::clone(&run), &page).await?;
        assert!(
            !printed.contains(MARKED) && printed.contains(&format!("Send {bullets}")),
            "(c) #{round}: the name built from the box prints bullets: {printed}"
        );
    }
    bench.close(opened).await
}

/// Where the box is born and privately filled before the page adopts it into the other window.
#[derive(Clone, Copy)]
enum SourceWindow {
    Popup,
    Opener,
}

/// The page opens the related tab and moves the actual marked node; the receiver never takes
/// a private entry. A fresh run evaluates there first, so its cached tree proves one frame.
/// With the old `if self.pw.took_private_text(self.tab())` hint, that proof skips the foreign
/// prototype search and the derived button name leaks in both directions (R59's negative).
#[expect(
    clippy::too_many_lines,
    reason = "one paired-window lifetime keeps the private entry, adoption and cleanup together"
)]
async fn moved_window_snapshots(
    bench: &Bench,
    source_window: SourceWindow,
) -> TestResult<Vec<String>> {
    let source = match source_window {
        SourceWindow::Popup => "popup",
        SourceWindow::Opener => "opener",
    };
    let path = format!("/private/moved-window.html?source={source}");
    let opener_url = bench.server.url("127.0.0.1", &path);
    let popup_url = format!("{opener_url}&popup=1");
    let result = async {
        let opener = bench.open(&path, Place::Front).await?;
        bench
            .live
            .invoke(
                "locator.click",
                json!({
                    "call": { "api": "locator.click", "timeoutMs": 5_000 },
                    "target": css_target(&opener.page.brand(), &["#open"]), "strict": "strict",
                }),
            )
            .await??;
        page_does(bench, &opener, "popupReady()").await?;
        let info = bench
            .live
            .browser
            .pages
            .list()
            .await?
            .into_iter()
            .find(|info| info.url == popup_url)
            .ok_or("the page-created related tab is listed")?;
        let popup = Opened {
            page: TestPage {
                id: info.page_id,
                tab: info.tab_id,
            },
            cover: None,
            place: Place::Front,
        };
        let (source, receiver, move_box) = match source_window {
            SourceWindow::Popup => (&popup, &opener, "moveToOpener()"),
            SourceWindow::Opener => (&opener, &popup, "moveToPopup()"),
        };
        let code = bench.r#ref(source, "textbox", "Code").await?;
        // The popup may leave its opener behind. Private entry supports background tabs;
        // this adoption row needs no foreground-placement precondition from Bench.enter.
        let env = HostEnv::new(&bench.live);
        let entry = bench
            .live
            .playwright
            .enter_privately(
                &env,
                EntryRequest {
                    tab: source.page.tab,
                    url: None,
                    fields: vec![Field::new(&code, &bench.page_origin).with(MARKED)?],
                    deadline: Instant::now() + Duration::from_secs(15),
                },
            )
            .await;
        if entry.outcome != EntryOutcome::Entered {
            return Err(format!("the source's private entry failed: {entry:?}").into());
        }
        page_does(bench, source, move_box).await?;
        if matches!(source_window, SourceWindow::Opener) {
            bench.live.close(&opener.page).await?;
            page_does(
                bench,
                receiver,
                "!window.opener || window.opener.closed ? 1 : 0",
            )
            .await?;
        }
        let page = receiver.page.brand();
        let run: Arc<RunState> = Arc::default();
        let singleton = bench
            .live
            .invoke_in_run(
                Arc::clone(&run),
                "page.evaluate",
                json!({
                    "call": { "api": "page.evaluate", "timeoutMs": 5_000 }, "page": page,
                    "expression": "() => window.frames.length === 0 && (!window.opener || window.opener.closed)",
                    "isFunction": true,
                    "arg": { "value": r#"{"v":"undefined"}"#, "handles": [] },
                }),
            )
            .await??;
        if singleton.get("value") != Some(&json!(true)) {
            return Err(format!("the receiver has no other frame or live opener: {singleton}").into());
        }
        let mut printed = Vec::new();
        for _ in 0..3 {
            printed.push(printed_in_run(bench, Arc::clone(&run), &page).await?);
        }
        TestResult::Ok(printed)
    }
    .await;
    // Snapshot assertions run only after cleanup. Listing by these fixture URLs also covers
    // a failure after window.open but before the new tab's handle reached the test.
    let cleanup: TestResult = async {
        let pages = bench.live.browser.pages.list().await?;
        let mut first_error = None;
        for info in pages
            .into_iter()
            .filter(|info| info.url == opener_url || info.url == popup_url)
        {
            if let Err(error) = bench
                .live
                .browser
                .cdp(
                    "Target.closeTarget",
                    json!({ "targetId": info.target_id.as_str() }),
                    None,
                )
                .await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        match first_error {
            Some(error) => Err(error.into()),
            None => Ok(()),
        }
    }
    .await;
    let printed = result?;
    cleanup?;
    Ok(printed)
}

/// R59: a plain box privately filled in a popup keeps its mark when the page adopts it into
/// the opener. The old per-tab hint is this row's negative: the opener took no private entry.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_r59_masks_a_box_moved_from_a_popup_into_its_opener() -> TestResult {
    let bench = Bench::start().await?;
    let bullets = "•".repeat(MARKED.chars().count());
    for (round, printed) in moved_window_snapshots(&bench, SourceWindow::Popup)
        .await?
        .iter()
        .enumerate()
    {
        assert!(
            !printed.contains(MARKED) && printed.contains(&format!("Send {bullets}")),
            "#{round}: the opener prints the derived button name as bullets, the value nowhere: {printed}"
        );
    }
    Ok(())
}

/// R59: the mirror move, then the source opener closes before the receiver's first read. The
/// old per-tab hint is this row's negative; the browser's entry memory must outlive the source.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_r59_masks_a_box_moved_into_a_popup_after_its_opener_closes() -> TestResult {
    let bench = Bench::start().await?;
    let bullets = "•".repeat(MARKED.chars().count());
    for (round, printed) in moved_window_snapshots(&bench, SourceWindow::Opener)
        .await?
        .iter()
        .enumerate()
    {
        assert!(
            !printed.contains(MARKED) && printed.contains(&format!("Send {bullets}")),
            "#{round}: the popup prints the derived button name as bullets, the value nowhere: {printed}"
        );
    }
    Ok(())
}

/// Ruling R53, step two: an assertion on a marked box the page moved into another same-origin
/// frame compares against the bullets there, as `inputValue()` reads them (ruling R47), whether
/// the locator finds the box itself or the label that leads to it. That frame's copy of the
/// probe never marked a box, so only the mark on the box tells it to guard the comparison. Both
/// ways: a box marked in the page and moved into the frame, and one marked in the frame and
/// moved out into the page. A failed check prints the box's value as `<N characters>` and holds
/// it nowhere (this input sends no Expected line, which is the script's own argument).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_check_on_a_moved_marked_box_compares_against_bullets() -> TestResult {
    let bench = Bench::start().await?;
    let bullets = "•".repeat(MARKED.chars().count());
    let marker = format!("<{} characters>", MARKED.chars().count());
    // The fixture's query, the page's move, then the box and its label where the move put them.
    let moves: [(&str, &str, &[&str], &[&str]); 2] = [
        ("", "moveIn()", &["#inner", "#code"], &["#inner", "label"]),
        ("?inside=1", "moveOut()", &["#code"], &["label"]),
    ];
    for (query, step, the_box, its_label) in moves {
        let opened = bench
            .open(&format!("/private/moved-box.html{query}"), Place::Front)
            .await?;
        let code = bench.r#ref(&opened, "textbox", "Code").await?;
        let env = bench.host(&opened);
        let entry = bench
            .enter(
                &env,
                &opened,
                vec![Field::new(&code, &bench.page_origin).with(MARKED)?],
            )
            .await;
        assert_eq!(entry.outcome, EntryOutcome::Entered, "{step}: {entry:?}");
        page_does(&bench, &opened, step).await?;
        let page = opened.page.brand();
        for (on, path) in [("box", the_box), ("label", its_label)] {
            let has_value = |expected: &str| {
                expectation(
                    ("target", css_target(&page, path)),
                    "toHaveValue",
                    &json!({
                        "expression": "to.have.value", "isNot": false,
                        "expectedText": [{ "string": expected }],
                    }),
                )
            };
            let as_read = bench
                .live
                .invoke("expect.locator", has_value(&bullets))
                .await?;
            assert_eq!(
                as_read.ok(),
                Some(Value::Null),
                "{step}, on the {on}: toHaveValue compares against the bullets"
            );
            for expected in [MARKED, "something else"] {
                let failed = match bench
                    .live
                    .invoke("expect.locator", has_value(expected))
                    .await?
                {
                    Ok(passed) => {
                        return Err(format!(
                            "{step}, on the {on}: toHaveValue({expected}) passed: {passed}"
                        )
                        .into());
                    }
                    Err(error) => error.message().to_owned(),
                };
                assert!(
                    failed.contains(&marker) && !failed.contains(MARKED),
                    "{step}, on the {on}: a failed toHaveValue({expected}) prints {marker}, \
                     never the value: {failed}"
                );
            }
        }
        bench.close(opened).await?;
    }
    Ok(())
}

/// Ruling R53's mark lives on the box's wrapper in Neo's isolated world, so it lives as long as
/// that wrapper does. A Chromium that ever dropped such a wrapper in a garbage collection while
/// the box lives would take every mark with it, and nothing else would notice; this row does.
/// It guards the browser's wrapper lifetime, not our code: no change of ours makes it fail.
/// First as the design's author probed it: collections while the box sits in its page, which
/// alone proves little, since Neo's own world still holds the box then. Then the real test:
/// only the page's world holds the box while the collections run.
///
/// What in Neo's world can hold a box strongly (probe sources, at this commit), and how the
/// second phase lets go of each, so the next reader can check the list is still complete:
/// - `Pins` (world.ts): every element a resolve, read or action pinned, strongly, until 512
///   newer entries evict it. The 520 reads on Plain evict the box's pins.
/// - `active` (private-entry.ts): the box under entry, until `readBack` or `forget`; the entry
///   is over before the first phase.
/// - The vendored engine's last ai snapshot (`_lastAriaSnapshotForQuery`), replaced by each
///   snapshot: the one taken after the stash does not hold the box.
/// - An armed hit-target check (act.ts), until its verdict or expiry: none is armed here.
/// - Cleared within the call that fills them, so never between calls: the pointer-target maps
///   (pointer-targets.ts, at the end of each snapshot) and the slot maps (shadow-roots.ts, on
///   the next microtask).
/// - Weak, so never: the aria refs (`WeakRef`), the element maps of aria-ref.ts, files.ts,
///   shadow-roots.ts and private-entry.ts (`WeakMap`), and frame pins (`WeakRef`).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_mark_outlives_garbage_collections() -> TestResult {
    let bench = Bench::start().await?;
    let bullets = "•".repeat(MARKED.chars().count());
    let opened = bench.open("/private/moved-box.html", Place::Front).await?;
    let code = bench.r#ref(&opened, "textbox", "Code").await?;
    let env = bench.host(&opened);
    let entry = bench
        .enter(
            &env,
            &opened,
            vec![Field::new(&code, &bench.page_origin).with(MARKED)?],
        )
        .await;
    assert_eq!(entry.outcome, EntryOutcome::Entered, "{entry:?}");
    let page = opened.page.brand();
    let session = private_host::root_session(&bench.live, &opened.page).await?;
    let collect = || async {
        for _ in 0..5 {
            session
                .send_value("HeapProfiler.collectGarbage", json!({}))
                .await?;
            // Churn the page's own heap too, so a collection has something to move.
            bench
                .live
                .main_world(
                    &opened.page,
                    "(() => { const junk = []; \
                       for (let i = 0; i < 200000; i++) junk.push({ i }); \
                       return junk.length; })()",
                )
                .await?;
        }
        TestResult::Ok(())
    };
    let masked = |phase: &'static str| {
        let page = &page;
        let bench = &bench;
        let bullets = &bullets;
        async move {
            let read = bench
                .live
                .invoke(
                    "locator.inputValue",
                    input_value(&css_target(page, &["#code"])),
                )
                .await??;
            assert_eq!(read, json!(bullets), "{phase}: inputValue reads bullets");
            let printed = printed_snapshot(bench, page).await?;
            assert!(
                !printed.contains(MARKED) && printed.contains(&format!("Send {bullets}")),
                "{phase}: the snapshot holds no value: {printed}"
            );
            TestResult::Ok(())
        }
    };

    collect().await?;
    masked("collected while the box sits in its page").await?;

    page_does(&bench, &opened, "stash()").await?;
    printed_snapshot(&bench, &page).await?;
    for _ in 0..520 {
        bench
            .live
            .invoke(
                "locator.inputValue",
                input_value(&css_target(&page, &["#plain"])),
            )
            .await??;
    }
    collect().await?;
    page_does(&bench, &opened, "restore()").await?;
    masked("collected while only the page held the box").await?;
    bench.close(opened).await
}
