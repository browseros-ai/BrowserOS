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
#[path = "support/live_browser.rs"]
#[expect(
    dead_code,
    reason = "shared with suites that run ops in a run of their own"
)]
mod live_browser;
#[path = "support/private_host.rs"]
mod private_host;

use std::time::Duration;

use fixture_server::FixtureServer;
use live_browser::{LiveBrowser, TestPage, TestResult};
use neo_playwright::{
    BoxSpec, Entry, EntryOutcome, EntryRequest, Expect, PrivateField, PrivateText,
};
use neo_trace::Delivery;
use private_host::HostEnv;
use serde_json::{Value, json};
use tokio::sync::{Semaphore, SemaphorePermit};
use tokio::time::Instant;

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
        let live = LiveBrowser::connect().await?;
        let (page_origin, other_origin) = private_host::origins(&server);
        Ok(Self {
            server,
            live,
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
    assert_eq!(
        owner,
        Some(neo_trace::ErrorOwner::Host),
        "the policy's own owner"
    );
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
    assert_eq!(
        owner,
        Some(neo_trace::ErrorOwner::Host),
        "the policy's own owner"
    );
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
