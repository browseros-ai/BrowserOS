//! The fixture registry and the CDP steps that load a fixture page and capture it, shared by the
//! recorder (`examples/record_fixtures.rs`), the live tests and the offline oracle tests.
//!
//! Each fixture isolates APC edge cases: deep cross-site OOPIFs, open and closed shadow DOM,
//! zero-size controls, browser-made shadow content, the role, name and state tables, private
//! form fields, covered and inert content, and a hung cross-site subframe. One page load serves
//! both recorded files:
//!
//! ```text
//!  load_fixture: new tab ─▶ focus emulation ─▶ navigate ─▶ ready ─▶ type secrets
//!  recorder:     ─▶ oracle (oracle_capture.rs) ─▶ before_capture ─▶ APC capture ─▶ close
//! ```

use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fmt::Write as _;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use browseros_cdp::page::{self, EnableParams, GetAnnotatedPageContentParams, NavigateParams};
use browseros_cdp::runtime::{self, EvaluateParams};
use browseros_cdp::target::{self, AttachToTargetParams};
use browseros_cdp::{CdpClient, ConnectOptions, SessionId};
use neo_snapshot_apc::{AnnotatedPageContent, decode, decode_reply_json};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::{self, Instant};

use crate::fixture_server::{FixtureServer, Page};

/// One recorded page: where it is served, when it is ready, and what runs before APC reads it.
/// `name` is the stem of its `tests/fixtures/<name>.pb` and `<name>.oracle.json`.
#[derive(Debug)]
pub(crate) struct Fixture {
    /// Registry name and fixture file stem.
    pub(crate) name: &'static str,
    /// The page the tab opens, as a path on `127.0.0.1`.
    pub(crate) entry: &'static str,
    /// A JavaScript predicate on the entry document that is true once every frame has loaded.
    pub(crate) ready: &'static str,
    /// The documents the fixture server serves for this fixture.
    pub(crate) serve: Serve,
    /// What happens between the oracle read and the APC capture.
    pub(crate) before_capture: BeforeCapture,
}

/// How a fixture's documents reach the fixture server.
#[derive(Debug)]
pub(crate) enum Serve {
    /// One HTML file, served at the entry path.
    File(&'static str),
    /// One document built in code, served at the entry path.
    Built(fn() -> Page),
    /// Several documents at their own paths: a frame tree spread over three loopback sites.
    Site(fn(&mut HashMap<String, Page>)),
}

/// The step after the oracle and before the APC capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BeforeCapture {
    /// APC reads the page as loaded and typed.
    Nothing,
    /// The `[::1]` child's renderer enters a busy loop, so APC hits its subframe timeout. It
    /// must follow the oracle: a spinning renderer answers no script, so the oracle could not
    /// read that frame.
    SpinHungFrame,
}

/// Whether the browser under test carries BrowserOS's APC contract (Chromium PR C1 on). Fields
/// marked `data-typed="contract"` hold secrets only the contract withholds, so they get one only
/// when it is present: stock APC would write the typed value into the `.pb`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ApcContract {
    /// Stock Chromium: only fields stock APC withholds are typed into.
    Absent,
    /// A build carrying the contract: `data-typed="contract"` fields are typed into too.
    Present,
}

/// Every typed secret starts with this, so one byte search over a recorded file finds any leak,
/// even a truncated one.
pub(crate) const CANARY_PREFIX: &str = "NEO-SECRET-CANARY";

/// The readiness predicate of a page that raises `window.__fixtureReady` itself.
const FLAG_READY: &str = "document.readyState === 'complete' && window.__fixtureReady === true";

/// Every recorded fixture. Each needs a `.pb` and an `.oracle.json` under `tests/fixtures/`
/// (tests/oracles.rs checks).
pub(crate) const FIXTURES: &[Fixture] = &[
    // Design 3.8 row 1: the 8-frame, 3-site tree of neo-playwright's tests/data/nest/*.
    Fixture {
        name: "nested-cross-site-frames",
        entry: "/nested/top.html",
        ready: "document.readyState === 'complete' && (window.__loadedFrames || 0) >= 7",
        serve: Serve::Site(insert_nested_cross_site_pages),
        before_capture: BeforeCapture::Nothing,
    },
    Fixture {
        name: "shadow-dom",
        entry: "/shadow-dom/index.html",
        ready: FLAG_READY,
        serve: Serve::Built(shadow_dom_page),
        before_capture: BeforeCapture::Nothing,
    },
    Fixture {
        name: "zero-size-checkbox",
        entry: "/zero-size-checkbox/index.html",
        ready: FLAG_READY,
        serve: Serve::Built(zero_size_checkbox_page),
        before_capture: BeforeCapture::Nothing,
    },
    Fixture {
        name: "covered-button",
        entry: "/covered-button/index.html",
        ready: FLAG_READY,
        serve: Serve::Built(covered_button_page),
        before_capture: BeforeCapture::Nothing,
    },
    Fixture {
        name: "sensitive-form",
        entry: "/sensitive-form/index.html",
        ready: FLAG_READY,
        serve: Serve::Built(sensitive_form_page),
        before_capture: BeforeCapture::Nothing,
    },
    // Design 3.8 row 1's `?hang`.
    Fixture {
        name: "hung-subframe",
        entry: "/hung-subframe/top.html",
        ready: "document.readyState === 'complete' && window.__healthyReady === true && window.__hangReady === true",
        serve: Serve::Site(insert_hung_subframe_pages),
        before_capture: BeforeCapture::SpinHungFrame,
    },
    Fixture {
        name: "deep-nesting",
        entry: "/deep-nesting/index.html",
        ready: FLAG_READY,
        serve: Serve::Built(deep_nesting_page),
        before_capture: BeforeCapture::Nothing,
    },
    // The edge pack (design 3.8 kind 3).
    Fixture {
        name: "shadow-roots",
        entry: "/edge/shadow-roots.html",
        ready: FLAG_READY,
        serve: Serve::File(include_str!("../pages/shadow-roots.html")),
        before_capture: BeforeCapture::Nothing,
    },
    Fixture {
        name: "zero-size-controls",
        entry: "/edge/zero-size-controls.html",
        ready: FLAG_READY,
        serve: Serve::File(include_str!("../pages/zero-size-controls.html")),
        before_capture: BeforeCapture::Nothing,
    },
    // neo-playwright's zero-size control page, verbatim. It raises no ready flag.
    Fixture {
        name: "nosize",
        entry: "/prelude/nosize.html",
        ready: "document.readyState === 'complete'",
        serve: Serve::File(include_str!(
            "../../../neo-playwright/tests/data/prelude/nosize.html"
        )),
        before_capture: BeforeCapture::Nothing,
    },
    Fixture {
        name: "ua-shadow",
        entry: "/edge/ua-shadow.html",
        ready: FLAG_READY,
        serve: Serve::File(include_str!("../pages/ua-shadow.html")),
        before_capture: BeforeCapture::Nothing,
    },
    Fixture {
        name: "roles",
        entry: "/edge/roles.html",
        ready: FLAG_READY,
        serve: Serve::File(include_str!("../pages/roles.html")),
        before_capture: BeforeCapture::Nothing,
    },
    Fixture {
        name: "names",
        entry: "/edge/names.html",
        ready: FLAG_READY,
        serve: Serve::File(include_str!("../pages/names.html")),
        before_capture: BeforeCapture::Nothing,
    },
    Fixture {
        name: "states",
        entry: "/edge/states.html",
        ready: FLAG_READY,
        serve: Serve::File(include_str!("../pages/states.html")),
        before_capture: BeforeCapture::Nothing,
    },
    Fixture {
        name: "private-forms",
        entry: "/edge/private-forms.html",
        ready: FLAG_READY,
        serve: Serve::File(include_str!("../pages/private-forms.html")),
        before_capture: BeforeCapture::Nothing,
    },
    Fixture {
        name: "modal-inert",
        entry: "/edge/modal-inert.html",
        ready: FLAG_READY,
        serve: Serve::File(include_str!("../pages/modal-inert.html")),
        before_capture: BeforeCapture::Nothing,
    },
];

/// Default CDP port when `BROWSEROS_CDP_PORT` is unset.
const DEFAULT_CDP_PORT: u16 = 9005;

/// Request timeout long enough for Chromium's 10 s hung-subframe APC capture timeout to settle.
const CDP_REQUEST_TIMEOUT: Duration = Duration::from_secs(45);

/// Maximum wait for fixture page readiness before failing capture.
const READY_TIMEOUT: Duration = Duration::from_secs(15);

/// Delay after posting `'spin'` so the hung OOPIF renderer enters its busy loop before APC runs.
const SPIN_SETTLE_DELAY: Duration = Duration::from_millis(200);

/// The registry entry named `name`.
pub(crate) fn fixture(name: &str) -> Result<&'static Fixture, Box<dyn Error>> {
    FIXTURES
        .iter()
        .find(|fixture| fixture.name == name)
        .ok_or_else(|| format!("unknown fixture name: {name}").into())
}

/// Connects a [`CdpClient`] to `BROWSEROS_CDP_PORT` (default `9005`) with a 45-second request
/// timeout.
pub(crate) async fn connect_cdp() -> Result<CdpClient, Box<dyn Error>> {
    let port = env::var("BROWSEROS_CDP_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(DEFAULT_CDP_PORT);
    let mut opts = ConnectOptions::new(port);
    opts.request_timeout = CDP_REQUEST_TIMEOUT;
    let client = CdpClient::connect(opts).await?;
    Ok(client)
}

/// The path-to-page map serving every fixture in [`FIXTURES`].
pub(crate) fn all_fixture_pages() -> HashMap<String, Page> {
    let mut pages = HashMap::new();
    for fixture in FIXTURES {
        match fixture.serve {
            Serve::File(html) => {
                pages.insert(fixture.entry.to_owned(), Page::html(html));
            }
            Serve::Built(build) => {
                pages.insert(fixture.entry.to_owned(), build());
            }
            Serve::Site(insert) => insert(&mut pages),
        }
    }
    pages
}

/// Captures `name` through the typed `browseros_cdp::page::get_annotated_page_content` command,
/// returning both the raw protobuf wire bytes and the decoded [`AnnotatedPageContent`] tree.
pub(crate) async fn capture_fixture_typed(
    client: &CdpClient,
    server: &FixtureServer,
    name: &str,
) -> Result<(Vec<u8>, AnnotatedPageContent), Box<dyn Error>> {
    let loaded = load_fixture(client, server, fixture(name)?, ApcContract::Absent).await?;
    let captured = async {
        loaded.before_capture(client).await?;
        loaded.capture_apc(client).await
    }
    .await;
    loaded.close(client).await;
    captured
}

/// Captures `name` through `CdpClient::send_raw` and decodes the raw CDP JSON reply directly via
/// [`decode_reply_json`].
pub(crate) async fn capture_fixture_raw_json(
    client: &CdpClient,
    server: &FixtureServer,
    name: &str,
) -> Result<(String, AnnotatedPageContent), Box<dyn Error>> {
    let loaded = load_fixture(client, server, fixture(name)?, ApcContract::Absent).await?;
    let captured = async {
        loaded.before_capture(client).await?;
        let raw_reply = client
            .send_raw(
                "Page.getAnnotatedPageContent",
                json!({ "includeActionableInformation": true }),
                Some(&loaded.session),
            )
            .await?;
        let apc = decode_reply_json(raw_reply.get())?;
        Ok((raw_reply.into_json(), apc))
    }
    .await;
    loaded.close(client).await;
    captured
}

/// A fixture loaded, ready and typed into, in a tab of its own. Its owner reads it (oracle, APC)
/// and must [`LoadedFixture::close`] it, on failure too.
#[derive(Debug)]
pub(crate) struct LoadedFixture {
    /// The registry entry this tab shows.
    pub(crate) fixture: &'static Fixture,
    /// The tab's target id, which is also its main frame's id.
    pub(crate) target_id: String,
    /// The flattened session attached to the tab.
    pub(crate) session: SessionId,
    /// Each canary typed into the page, in document order of the fields that took them.
    pub(crate) typed: Vec<String>,
}

/// Opens `fixture` in a fresh tab and brings it to the state both recorded files describe:
/// loaded, ready and typed into. The tab is closed again if any step fails.
pub(crate) async fn load_fixture(
    client: &CdpClient,
    server: &FixtureServer,
    fixture: &'static Fixture,
    contract: ApcContract,
) -> Result<LoadedFixture, Box<dyn Error>> {
    let created = client
        .send("Target.createTarget", json!({ "url": "about:blank" }), None)
        .await?;
    let target_id = created
        .get("targetId")
        .and_then(Value::as_str)
        .ok_or("Target.createTarget reply missing targetId")?
        .to_owned();

    let prepared = async {
        let attached = target::attach_to_target(
            client,
            AttachToTargetParams {
                target_id: target_id.clone(),
                flatten: Some(true),
            },
            None,
        )
        .await?;
        let session = SessionId::from(attached.session_id);
        page::enable(client, EnableParams::default(), Some(&session)).await?;
        // Before navigating: a background tab's document has no focus, so `active` and
        // `document.hasFocus()` would depend on which tab the browser last raised.
        client
            .send(
                "Emulation.setFocusEmulationEnabled",
                json!({ "enabled": true }),
                Some(&session),
            )
            .await?;
        page::navigate(
            client,
            NavigateParams {
                url: server.url("127.0.0.1", fixture.entry),
                ..NavigateParams::default()
            },
            Some(&session),
        )
        .await?;
        wait_for_ready(client, &session, fixture).await?;
        let typed = type_secrets(client, &session, fixture, contract).await?;
        Ok::<_, Box<dyn Error>>((session, typed))
    }
    .await;

    match prepared {
        Ok((session, typed)) => Ok(LoadedFixture {
            fixture,
            target_id,
            session,
            typed,
        }),
        Err(error) => {
            close_target_best_effort(client, &target_id).await;
            Err(error)
        }
    }
}

impl LoadedFixture {
    /// Runs the fixture's [`BeforeCapture`] step. Call it after the oracle and before APC.
    pub(crate) async fn before_capture(&self, client: &CdpClient) -> Result<(), Box<dyn Error>> {
        match self.fixture.before_capture {
            BeforeCapture::Nothing => Ok(()),
            BeforeCapture::SpinHungFrame => {
                let triggered = eval_bool(
                    client,
                    &self.session,
                    "(() => { const f = document.getElementById('hung-frame'); \
                     if (!f || !f.contentWindow) return false; \
                     f.contentWindow.postMessage('spin', '*'); return true; })()",
                )
                .await?;
                if !triggered {
                    return Err(format!(
                        "{} failed to locate #hung-frame to start spin loop",
                        self.fixture.name
                    )
                    .into());
                }
                time::sleep(SPIN_SETTLE_DELAY).await;
                Ok(())
            }
        }
    }

    /// Reads the page with `Page.getAnnotatedPageContent`, returning the protobuf wire bytes and
    /// their decoded tree.
    pub(crate) async fn capture_apc(
        &self,
        client: &CdpClient,
    ) -> Result<(Vec<u8>, AnnotatedPageContent), Box<dyn Error>> {
        let reply = page::get_annotated_page_content(
            client,
            GetAnnotatedPageContentParams {
                include_actionable_information: Some(true),
            },
            Some(&self.session),
        )
        .await?;
        let raw_bytes = STANDARD.decode(&reply.content)?;
        let apc = decode(&reply.content)?;
        Ok((raw_bytes, apc))
    }

    /// Closes the tab, ignoring errors so cleanup never masks the read's own result.
    pub(crate) async fn close(self, client: &CdpClient) {
        close_target_best_effort(client, &self.target_id).await;
    }
}

/// A `data-typed` field's mode: whether stock APC already withholds what is typed into it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TypedField {
    /// `data-typed`: stock withholds it (a password field, or one toggled from password).
    Always,
    /// `data-typed="contract"`: only the BrowserOS contract withholds it.
    Contract,
}

/// Types one canary into each `[data-typed]` field of the main document that `contract` allows,
/// then lets the page react (`window.__afterTyping`, which flips a show-password field to text)
/// and blurs the field typed last. Returns the canaries typed.
///
/// Keyboard input, as a host types a secret, rather than a script's value write, so the recorded
/// state is the one privacy has to hold for.
async fn type_secrets(
    client: &CdpClient,
    session: &SessionId,
    fixture: &Fixture,
    contract: ApcContract,
) -> Result<Vec<String>, Box<dyn Error>> {
    let modes: Vec<Option<String>> = serde_json::from_value(
        evaluate(
            client,
            session,
            "Array.from(document.querySelectorAll('[data-typed]'), \
             (field) => field.getAttribute('data-typed'))",
        )
        .await?,
    )?;
    let mut typed = Vec::new();
    for (index, mode) in modes.iter().enumerate() {
        let mode = match mode.as_deref() {
            Some("contract") => TypedField::Contract,
            _ => TypedField::Always,
        };
        if mode == TypedField::Contract && contract == ApcContract::Absent {
            continue;
        }
        let focused = evaluate(
            client,
            session,
            &format!(
                "(() => {{ const field = document.querySelectorAll('[data-typed]')[{index}]; \
                 field.focus(); return document.activeElement === field; }})()"
            ),
        )
        .await?;
        if focused != Value::Bool(true) {
            return Err(format!(
                "{}: data-typed field {index} did not take focus, so no canary was typed",
                fixture.name
            )
            .into());
        }
        let canary = format!("{CANARY_PREFIX}-{}-{index}", fixture.name);
        client
            .send("Input.insertText", json!({ "text": canary }), Some(session))
            .await?;
        typed.push(canary);
    }
    if !typed.is_empty() {
        // Only after typing: a page typed into nothing keeps the element it focused itself
        // (states.html pins `active` that way).
        evaluate(
            client,
            session,
            "(async () => { await window.__afterTyping?.(); document.activeElement?.blur(); })()",
        )
        .await?;
    }
    Ok(typed)
}

/// `Runtime.evaluate`'s reply, as far as the typing steps read it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Evaluated {
    result: EvaluatedValue,
    exception_details: Option<Value>,
}

/// The by-value result of [`Evaluated`].
#[derive(Deserialize)]
struct EvaluatedValue {
    #[serde(default)]
    value: Value,
}

/// Evaluates `expression` in the page's main world, awaiting a promise, and returns its value;
/// a thrown exception is an error that quotes it.
async fn evaluate(
    client: &CdpClient,
    session: &SessionId,
    expression: &str,
) -> Result<Value, Box<dyn Error>> {
    let reply: Evaluated = serde_json::from_value(
        client
            .send(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
                Some(session),
            )
            .await?,
    )?;
    if let Some(exception) = reply.exception_details {
        return Err(format!("page script threw: {exception}").into());
    }
    Ok(reply.result.value)
}

/// Polls the fixture's readiness predicate until it returns `true` or times out.
async fn wait_for_ready(
    client: &CdpClient,
    session: &SessionId,
    fixture: &Fixture,
) -> Result<(), Box<dyn Error>> {
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        if eval_bool(client, session, fixture.ready).await? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let diag = runtime::evaluate(
                client,
                EvaluateParams {
                    expression: "JSON.stringify({ href: location.href, readyState: document.readyState, loadedFrames: window.__loadedFrames, loaded: window.__loadedSet ? Array.from(window.__loadedSet) : null, healthyReady: window.__healthyReady, hangReady: window.__hangReady, fixtureReady: window.__fixtureReady })".to_owned(),
                    return_by_value: Some(true),
                    ..EvaluateParams::default()
                },
                Some(session),
            )
            .await
            .ok()
            .and_then(|reply| reply.result.value)
            .unwrap_or(Value::Null);
            return Err(format!(
                "fixture {} did not become ready within {READY_TIMEOUT:?} (diag={diag})",
                fixture.name
            )
            .into());
        }
        time::sleep(Duration::from_millis(50)).await;
    }
}

/// Evaluates a boolean JavaScript expression inside `session`.
async fn eval_bool(
    client: &CdpClient,
    session: &SessionId,
    expression: &str,
) -> Result<bool, Box<dyn Error>> {
    let reply = runtime::evaluate(
        client,
        EvaluateParams {
            expression: expression.to_owned(),
            return_by_value: Some(true),
            ..EvaluateParams::default()
        },
        Some(session),
    )
    .await?;
    Ok(reply.result.value.and_then(|value| value.as_bool()) == Some(true))
}

/// Closes `target_id`, ignoring errors so cleanup never masks the primary test result.
async fn close_target_best_effort(client: &CdpClient, target_id: &str) {
    let _closed = client
        .send("Target.closeTarget", json!({ "targetId": target_id }), None)
        .await;
}

/// Child iframe descriptor inside `nested-cross-site-frames`.
struct ChildFrameSpec {
    host: &'static str,
    slug: &'static str,
    height_px: u32,
}

/// One frame document in `nested-cross-site-frames`.
struct FrameSpec {
    slug: &'static str,
    label: &'static str,
    children: &'static [ChildFrameSpec],
}

/// Inserts all 8 pages of the 4-level, 3-site `nested-cross-site-frames` tree.
fn insert_nested_cross_site_pages(pages: &mut HashMap<String, Page>) {
    // Tree across 3 sites (A = 127.0.0.1, B = localhost, C = [::1]):
    // Top (A) -> A1 (A) -> B2 (B) -> B3 (B)
    // Top (A) -> B1 (B) -> C2 (C) -> A3 (A) -> C4 (C)
    const SPECS: &[FrameSpec] = &[
        FrameSpec {
            slug: "top",
            label: "Top",
            children: &[
                ChildFrameSpec {
                    host: "127.0.0.1",
                    slug: "a1",
                    height_px: 480,
                },
                ChildFrameSpec {
                    host: "localhost",
                    slug: "b1",
                    height_px: 640,
                },
            ],
        },
        FrameSpec {
            slug: "a1",
            label: "A1",
            children: &[ChildFrameSpec {
                host: "localhost",
                slug: "b2",
                height_px: 340,
            }],
        },
        FrameSpec {
            slug: "b2",
            label: "B2",
            children: &[ChildFrameSpec {
                host: "localhost",
                slug: "b3",
                height_px: 180,
            }],
        },
        FrameSpec {
            slug: "b3",
            label: "B3",
            children: &[],
        },
        FrameSpec {
            slug: "b1",
            label: "B1",
            children: &[ChildFrameSpec {
                host: "[::1]",
                slug: "c2",
                height_px: 500,
            }],
        },
        FrameSpec {
            slug: "c2",
            label: "C2",
            children: &[ChildFrameSpec {
                host: "127.0.0.1",
                slug: "a3",
                height_px: 340,
            }],
        },
        FrameSpec {
            slug: "a3",
            label: "A3",
            children: &[ChildFrameSpec {
                host: "[::1]",
                slug: "c4",
                height_px: 180,
            }],
        },
        FrameSpec {
            slug: "c4",
            label: "C4",
            children: &[],
        },
    ];

    for spec in SPECS {
        let is_top = spec.slug == "top";
        let html = build_nested_frame_html(spec.label, is_top, spec.children);
        pages.insert(format!("/nested/{}.html", spec.slug), Page::html(&html));
    }
}

/// Renders one frame HTML document for `nested-cross-site-frames`.
fn build_nested_frame_html(label: &str, is_top: bool, children: &[ChildFrameSpec]) -> String {
    let mut child_script = String::new();
    for child in children {
        let host = child.host;
        let child_slug = child.slug;
        let height = child.height_px;
        let _infallible = writeln!(
            child_script,
            "{{ const f = document.createElement('iframe'); \
             f.title = '{child_slug}'; \
             f.src = 'http://{host}:' + location.port + '/nested/{child_slug}.html'; \
             f.style.cssText = 'width:96%;height:{height}px;display:block;border:1px solid #999;margin-top:8px;'; \
             document.body.appendChild(f); }}"
        );
    }

    let ready_script = if is_top {
        "window.__loadedSet = new Set();\n\
         window.__loadedFrames = 0;\n\
         window.addEventListener('message', (event) => {\n\
           if (event.data && event.data.type === 'apc-frame-ready') {\n\
             window.__loadedSet.add(event.data.name);\n\
             window.__loadedFrames = window.__loadedSet.size;\n\
           }\n\
         });"
        .to_owned()
    } else {
        format!(
            "const notify = () => setTimeout(() => {{\n\
               window.top.postMessage({{ type: 'apc-frame-ready', name: '{label}' }}, '*');\n\
             }}, 0);\n\
             if (document.readyState === 'complete') notify();\n\
             else window.addEventListener('load', notify);"
        )
    };

    format!(
        "<!doctype html>\n\
         <html lang=\"en\">\n\
         <head><meta charset=\"utf-8\"><title>{label}</title></head>\n\
         <body style=\"margin:8px;font-family:sans-serif;\">\n\
         <h1>{label}</h1>\n\
         <button type=\"button\">{label} button</button>\n\
         <input type=\"text\" aria-label=\"{label} field\">\n\
         <script>\n\
         {ready_script}\n\
         {child_script}\n\
         </script>\n\
         </body>\n\
         </html>\n"
    )
}

/// Builds the `shadow-dom` fixture with light DOM, open shadow root, closed shadow root, and
/// `<slot>` projections.
fn shadow_dom_page() -> Page {
    Page::html(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Shadow DOM Fixture</title></head>
<body style="margin:16px;font-family:sans-serif;">
  <h1>Shadow DOM Fixture</h1>
  <button type="button">Light button</button>
  <div id="open-host">Open slotted text</div>
  <div id="closed-host">Closed slotted text</div>
  <script>
    const openHost = document.getElementById('open-host');
    const openRoot = openHost.attachShadow({ mode: 'open' });
    openRoot.innerHTML =
      '<div style="margin-top:8px;">' +
      '<button type="button">Open shadow button</button>' +
      '<input type="text" aria-label="Open shadow input">' +
      '<p><slot></slot></p>' +
      '</div>';

    const closedHost = document.getElementById('closed-host');
    const closedRoot = closedHost.attachShadow({ mode: 'closed' });
    closedRoot.innerHTML =
      '<div style="margin-top:8px;">' +
      '<button type="button">Closed shadow button</button>' +
      '<input type="text" aria-label="Closed shadow input">' +
      '<p><slot></slot></p>' +
      '</div>';

    window.__fixtureReady = true;
  </script>
</body>
</html>
"#,
    )
}

/// Builds the `zero-size-checkbox` fixture with CSS 0x0 checkboxes paired via `for` and wrapper
/// `<label>` elements.
fn zero_size_checkbox_page() -> Page {
    Page::html(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>Zero-Size Checkbox Fixture</title>
  <style>
    .sr-checkbox {
      width: 0;
      height: 0;
      opacity: 0;
      position: absolute;
      margin: 0;
      padding: 0;
      border: 0;
    }
  </style>
</head>
<body style="margin:16px;font-family:sans-serif;">
  <h1>Zero-Size Checkbox Fixture</h1>
  <div>
    <input type="checkbox" id="terms" class="sr-checkbox">
    <label for="terms">Accept terms</label>
  </div>
  <div style="margin-top:12px;">
    <label>
      <input type="checkbox" checked class="sr-checkbox">
      Subscribe newsletter
    </label>
  </div>
  <script>
    window.__fixtureReady = true;
  </script>
</body>
</html>
"#,
    )
}

/// Builds the `covered-button` fixture with a visible button occluded by a fixed full-viewport
/// backdrop and modal dialog.
fn covered_button_page() -> Page {
    Page::html(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Covered Button Fixture</title></head>
<body style="margin:0;font-family:sans-serif;">
  <main id="stage" style="padding:32px;">
    <h1>Covered Button Fixture</h1>
    <button id="target" type="button" style="width:160px;height:48px;">Buy</button>
  </main>
  <div id="backdrop" style="position:fixed;inset:0;z-index:10;background:rgba(0,0,0,0.4);display:flex;align-items:center;justify-content:center;">
    <div role="dialog" aria-label="What You Need To Know" style="background:#fff;padding:24px;border-radius:8px;min-width:320px;">
      <p>Resale prices often exceed face value.</p>
      <button id="accept" type="button">Accept &amp; Continue</button>
      <button id="dismiss" type="button">Dismiss</button>
    </div>
  </div>
  <script>
    window.__fixtureReady = true;
  </script>
</body>
</html>
"#,
    )
}

/// Builds the `sensitive-form` fixture containing password, credit-card, and OTP fields with dummy
/// test values.
fn sensitive_form_page() -> Page {
    Page::html(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Sensitive Form Fixture</title></head>
<body style="margin:16px;font-family:sans-serif;">
  <h1>Sensitive Form Fixture</h1>
  <form action="/sensitive-form/submit" method="post">
    <div>
      <label for="pw">Password</label>
      <input id="pw" type="password" value="example-pass-value">
    </div>
    <div>
      <label for="cc">Card number</label>
      <input id="cc" autocomplete="cc-number" value="4111111111111111">
    </div>
    <div>
      <label for="otp">One-time code</label>
      <input id="otp" autocomplete="one-time-code" value="123456">
    </div>
    <button type="submit">Pay now</button>
  </form>
  <script>
    window.__fixtureReady = true;
  </script>
</body>
</html>
"#,
    )
}

/// Inserts the top, healthy cross-site child, and hung cross-site child pages for `hung-subframe`.
fn insert_hung_subframe_pages(pages: &mut HashMap<String, Page>) {
    pages.insert(
        "/hung-subframe/top.html".to_owned(),
        Page::html(
            r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Hung Subframe Fixture</title></head>
<body style="margin:16px;font-family:sans-serif;">
  <h1>Hung Subframe Fixture</h1>
  <button type="button">Top button</button>
  <script>
    window.__healthyReady = false;
    window.__hangReady = false;
    window.addEventListener('message', (event) => {
      if (event.data === 'healthy-ready') window.__healthyReady = true;
      if (event.data === 'hang-ready') window.__hangReady = true;
    });

    const healthy = document.createElement('iframe');
    healthy.id = 'healthy-frame';
    healthy.title = 'Healthy frame';
    healthy.src = 'http://localhost:' + location.port + '/hung-subframe/healthy.html';
    healthy.style.cssText = 'width:90%;height:140px;display:block;border:1px solid #090;margin-top:8px;';
    document.body.appendChild(healthy);

    const hung = document.createElement('iframe');
    hung.id = 'hung-frame';
    hung.title = 'Hung frame';
    hung.src = 'http://[::1]:' + location.port + '/hung-subframe/hung.html';
    hung.style.cssText = 'width:90%;height:140px;display:block;border:1px solid #900;margin-top:8px;';
    document.body.appendChild(hung);
  </script>
</body>
</html>
"#,
        ),
    );

    pages.insert(
        "/hung-subframe/healthy.html".to_owned(),
        Page::html(
            r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Healthy Frame</title></head>
<body style="margin:8px;font-family:sans-serif;">
  <h2>Healthy Frame</h2>
  <button type="button">Healthy frame button</button>
  <script>
    window.addEventListener('load', () => {
      setTimeout(() => { parent.postMessage('healthy-ready', '*'); }, 0);
    });
  </script>
</body>
</html>
"#,
        ),
    );

    pages.insert(
        "/hung-subframe/hung.html".to_owned(),
        Page::html(
            r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Hung Frame</title></head>
<body style="margin:8px;font-family:sans-serif;">
  <h2>Hung Frame</h2>
  <button type="button">Hung frame button</button>
  <script>
    window.addEventListener('message', (event) => {
      if (event.data === 'spin' && event.source === parent) {
        for (;;) {}
      }
    });
    window.addEventListener('load', () => {
      setTimeout(() => { parent.postMessage('hang-ready', '*'); }, 0);
    });
  </script>
</body>
</html>
"#,
        ),
    );
}

/// Builds the `deep-nesting` fixture: 120 nested semantic `<div role="group">` containers around a
/// button, exceeding `prost`'s default 100-level recursion limit while remaining valid in Blink.
fn deep_nesting_page() -> Page {
    const NESTED_GROUPS: usize = 120;
    let mut html = String::from(
        "<!doctype html>\n\
         <html lang=\"en\">\n\
         <head><meta charset=\"utf-8\"><title>Deep Nesting Fixture</title></head>\n\
         <body style=\"margin:16px;font-family:sans-serif;\">\n\
         <h1>Deep Nesting Fixture</h1>\n",
    );
    for _ in 0..NESTED_GROUPS {
        html.push_str("<div role=\"group\">");
    }
    html.push_str("<button type=\"button\">Deep nested button</button>");
    for _ in 0..NESTED_GROUPS {
        html.push_str("</div>");
    }
    html.push_str("\n<script>window.__fixtureReady = true;</script>\n</body>\n</html>\n");
    Page::html(&html)
}
