//! The private entry (protected-fill design sections 5.4 to 5.6) through
//! `Playwright::enter_privately`, against a real browser, on zuzu's acceptance fixtures
//! (`claw-sdk/zuzu/night/sdk-acceptance.md`, S1): a sign-in, a covered box, values a box cuts,
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
use fixture_server::FixtureServer;
use input_recording::{InputCall, InputRecording};
use live_browser::{LiveBrowser, TestPage, TestResult};
use neo_playwright::{
    BoxSpec, Entry, EntryOutcome, EntryRequest, Expect, Op, PictureCheck, PrivateField,
    PrivateText, RunState,
};
use neo_trace::{Delivery, ErrorOwner};
use private_host::HostEnv;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, SemaphorePermit, oneshot};
use tokio::time::{self, Instant};

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
        let server = FixtureServer::start(private_host::pages()).await?;
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
    cut: bool,
    spaced: bool,
    keys: bool,
}

impl Field {
    fn new(r#ref: &str, origin: &str) -> Self {
        Self {
            r#ref: r#ref.to_owned(),
            origin: origin.to_owned(),
            cut: false,
            spaced: false,
            keys: false,
        }
    }

    fn keys(mut self) -> Self {
        self.keys = true;
        self
    }

    fn cut(mut self) -> Self {
        self.cut = true;
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
const CARD: &str = "4000056655665556";

/// B11, B05y: a sign-in's two boxes, typed in one insertion each and read back, on a tab in
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

/// B11c: a password box under a cookie banner refuses the whole sign-in before the username is
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

/// B11k: a password the box's maxlength cuts is entered when the host allows the cut, and
/// refused before any key when it does not. B05x, B05z, B05zx: a code that cannot fit is
/// refused before any key, never `entered_page_moved`.
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
    assert_eq!(entry.outcome, EntryOutcome::Entered, "B11k: {entry:?}");
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

/// B05v, B05w, B11v: a box whose form submits on its last character, or on its first input
/// event, is `entered_page_moved` (B05v also `entered`, as zuzu allows; B11v exactly
/// `entered_page_moved`). B05y: a box that stays is `entered`. B05h, B05j, B05g: the address
/// changing alone moves nothing. B05zf: a form that submits as the box takes the focus types
/// nothing and is never `entered_page_moved`.
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
            "{place:?} B11v: {entry:?}"
        );
        assert_eq!(entry.delivery, Delivery::Done, "{place:?} B11v");
        bench.close(opened).await?;

        let opened = bench.open("/private/moved.html?mode=focus", place).await?;
        let box_ref = bench.r#ref(&opened, "textbox", "Verification code").await?;
        let env = bench.host(&opened);
        let field = Field::new(&box_ref, origin).keys().with("246813")?;
        let entry = bench.enter(&env, &opened, vec![field]).await;
        assert_ne!(
            entry.outcome,
            EntryOutcome::EnteredPageMoved,
            "{place:?} B05zf"
        );
        assert_ne!(
            entry.outcome,
            EntryOutcome::Entered,
            "{place:?} B05zf: {entry:?}"
        );
        assert_eq!(entry.typed, vec![0], "{place:?} B05zf: nothing typed");
        bench.close(opened).await?;
    }
    Ok(())
}

/// B06x (finding I1): the site replaces the box after the third of six keys; the focus check
/// before the fourth stops the entry: `uncertain`, delivery unknown, three characters typed.
/// B05t: six one-digit boxes the form removes after the third digit: `uncertain` at the third.
/// B05t1: six one-digit boxes that stay: each holds its digit.
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
            "{place:?} B06x: {entry:?}"
        );
        assert_eq!(code(&entry), Some("focus_lost"), "{place:?} B06x");
        assert_eq!(entry.delivery, Delivery::Unknown, "{place:?} B06x");
        assert_eq!(entry.typed, vec![3], "{place:?} B06x");
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

/// B05fx, B05fc, B11fx, B06fx: boxes in cross-site frames, each looked at in its own frame and
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
            "{place:?} B05fx: {entry:?}"
        );
        assert_eq!(
            bench.value(&opened, &number).await?,
            CARD,
            "{place:?} B05fx"
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
            "{place:?} B05fc: {entry:?}"
        );
        assert_eq!(entry.typed, vec![16, 5, 3]);
        for (r#ref, value) in &refs {
            assert_eq!(
                bench.value(&opened, r#ref).await?,
                *value,
                "{place:?} B05fc"
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
            "{place:?} B11fx: {entry:?}"
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
            "{place:?} B06fx: {entry:?}"
        );
        assert_eq!(bench.value(&opened, &code_box).await?, "246813");
        bench.close(opened).await?;
    }
    Ok(())
}

/// B05fs: the card box hands the focus to another box as it takes it; B05ft: the page takes
/// the focus back into its own box when the frame takes it: `not_focused`, nothing in either
/// box.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_focus_thief_gets_nothing() -> TestResult {
    let bench = Bench::start().await?;
    let other = &bench.other_origin;
    for place in PLACES {
        // zuzu runs these five times each: a thief that wins one race in five still fails.
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
                "{place:?} B05fs #{round}: {entry:?}"
            );
            assert_eq!(
                code(&entry),
                Some("not_focused"),
                "{place:?} B05fs #{round}"
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
                "{place:?} B05fs #{round}"
            );
            assert_eq!(
                bench.value(&opened, &thief).await?,
                "",
                "{place:?} B05fs #{round}"
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
            assert_eq!(
                entry.outcome,
                EntryOutcome::Refused,
                "{place:?} B05ft #{round}: {entry:?}"
            );
            assert_eq!(
                code(&entry),
                Some("not_focused"),
                "{place:?} B05ft #{round}"
            );
            assert_eq!(
                bench.value(&opened, &number).await?,
                "",
                "{place:?} B05ft #{round}"
            );
            assert_eq!(
                bench.value(&opened, &thief).await?,
                "",
                "{place:?} B05ft #{round}"
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

/// Finding I2's own case: a box that keeps less than what is typed, with no maxlength, holds
/// something else at the read-back: `uncertain`, `value_not_taken`, the whole value counted.
/// B05g's code that cannot fit its maxlength is refused before any key.
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
            "{place:?} B05g: {entry:?}"
        );
        assert_eq!(entry.typed, vec![0], "{place:?} B05g");
        bench.close(opened).await?;
    }
    Ok(())
}

/// Case 5a, B11f: the password box is swapped out as it takes the focus: the username is typed,
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
            "{place:?} B11f: {entry:?}"
        );
        assert_eq!((entry.at, code(&entry)), (Some(1), Some("not_focused")));
        assert_eq!(entry.typed, vec![EMAIL.len(), 0], "{place:?} B11f");
        assert_eq!(entry.delivery, Delivery::Done, "{place:?} B11f");
        assert_eq!(bench.value(&opened, &email).await?, EMAIL, "{place:?} B11f");
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
/// that submits on its first input event, B11v). One insertion is exactly one input event.
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
/// focus on a button marks nothing: the neighbour reads as the page holds it. On a tab in front
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
            ("button", false, "value_not_taken", STRAY, false),
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

/// Protected fill 4/5 (ruling R46): once a tab took a private fill, a script's own
/// `page.screenshot()` and `locator.screenshot()` of it are refused with one host-owned error,
/// `private_tab`, and another tab still screenshots.
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
    for (what, input) in [
        ("page.screenshot", shot(&page, None)),
        (
            "locator.screenshot",
            shot(&page, Some(ref_target(&page, &email))),
        ),
    ] {
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
            "This tab took a private fill, so Neo takes no picture of it for the rest of the \
             tab's life. Trying again will not succeed. Read the page with page.ariaSnapshot() \
             instead.",
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

/// Ruling R45's named limit: until a snapshot of the document has walked a closed shadow root,
/// Neo does not know the boxes inside it, and a picture taken then can show what one holds,
/// whoever typed it (here the page's own script; the keyboard would do the same). A marked
/// snapshot that walks the root closes it for that document.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_box_in_a_closed_root_is_known_once_a_snapshot_walked_it() -> TestResult {
    let bench = Bench::start().await?;
    let opened = bench
        .open("/private/closed-card.html", Place::Front)
        .await?;
    let typed = bench
        .live
        .main_world(&opened.page, "fillCard('4242424242424242')")
        .await?;
    assert_eq!(typed, json!(16));
    let playwright = &bench.live.playwright;
    assert!(
        matches!(
            playwright.picture_check(opened.page.tab).await,
            PictureCheck::Clear { .. }
        ),
        "the limit: no snapshot walked the closed root yet"
    );
    // A marked snapshot: clickable marking reads the DOM capture and hands the closed root's
    // seeds to the document's world.
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
    assert_eq!(
        playwright.picture_check(opened.page.tab).await,
        PictureCheck::Private,
        "after a marked snapshot walked the root, its box is known"
    );
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
