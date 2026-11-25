//! Named HTML edge fixtures and CDP capture helpers shared by fixture recording and live tests.
//!
//! Each fixture isolates one APC edge case (deep cross-site OOPIFs, open/closed shadow DOM with
//! slots, zero-size checkboxes with labels, occluded buttons behind dialogs, sensitive form
//! redaction, and a hung cross-site subframe).

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
use serde_json::{Value, json};
use tokio::time::{self, Instant};

use crate::fixture_server::{FixtureServer, Page};

/// All seven named APC edge fixtures recorded against Chromium 158.
pub(crate) const ALL_FIXTURES: &[&str] = &[
    "nested-cross-site-frames",
    "shadow-dom",
    "zero-size-checkbox",
    "covered-button",
    "sensitive-form",
    "hung-subframe",
    "deep-nesting",
];

/// Default CDP port when `BROWSEROS_CDP_PORT` is unset.
const DEFAULT_CDP_PORT: u16 = 9005;

/// Request timeout long enough for Chromium's 10 s hung-subframe APC capture timeout to settle.
const CDP_REQUEST_TIMEOUT: Duration = Duration::from_secs(45);

/// Maximum wait for fixture page readiness before failing capture.
const READY_TIMEOUT: Duration = Duration::from_secs(15);

/// Delay after posting `'spin'` so the hung OOPIF renderer enters its busy loop before APC runs.
const SPIN_SETTLE_DELAY: Duration = Duration::from_millis(200);

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

/// Builds the full path-to-page map serving all six APC edge fixtures.
pub(crate) fn all_fixture_pages() -> HashMap<String, Page> {
    let mut pages = HashMap::new();
    insert_nested_cross_site_pages(&mut pages);
    pages.insert("/shadow-dom/index.html".to_owned(), shadow_dom_page());
    pages.insert(
        "/zero-size-checkbox/index.html".to_owned(),
        zero_size_checkbox_page(),
    );
    pages.insert(
        "/covered-button/index.html".to_owned(),
        covered_button_page(),
    );
    pages.insert(
        "/sensitive-form/index.html".to_owned(),
        sensitive_form_page(),
    );
    insert_hung_subframe_pages(&mut pages);
    pages.insert("/deep-nesting/index.html".to_owned(), deep_nesting_page());
    pages
}

/// Returns the entry path on `127.0.0.1` for `name`.
pub(crate) fn fixture_entry_path(name: &str) -> Result<&'static str, Box<dyn Error>> {
    match name {
        "nested-cross-site-frames" => Ok("/nested/top.html"),
        "shadow-dom" => Ok("/shadow-dom/index.html"),
        "zero-size-checkbox" => Ok("/zero-size-checkbox/index.html"),
        "covered-button" => Ok("/covered-button/index.html"),
        "sensitive-form" => Ok("/sensitive-form/index.html"),
        "hung-subframe" => Ok("/hung-subframe/top.html"),
        "deep-nesting" => Ok("/deep-nesting/index.html"),
        other => Err(format!("unknown fixture name: {other}").into()),
    }
}

/// Captures `name` through the typed `browseros_cdp::page::get_annotated_page_content` command,
/// returning both the raw protobuf wire bytes and the decoded [`AnnotatedPageContent`] tree.
pub(crate) async fn capture_fixture_typed(
    client: &CdpClient,
    server: &FixtureServer,
    name: &str,
) -> Result<(Vec<u8>, AnnotatedPageContent), Box<dyn Error>> {
    let (target_id, session) = open_and_prepare_fixture(client, server, name).await?;
    let captured = async {
        let reply = page::get_annotated_page_content(
            client,
            GetAnnotatedPageContentParams {
                include_actionable_information: Some(true),
            },
            Some(&session),
        )
        .await?;
        let raw_bytes = STANDARD.decode(&reply.content)?;
        let apc = decode(&reply.content)?;
        Ok((raw_bytes, apc))
    }
    .await;
    close_target_best_effort(client, &target_id).await;
    captured
}

/// Captures `name` through `CdpClient::send_raw` and decodes the raw CDP JSON reply directly via
/// [`decode_reply_json`].
pub(crate) async fn capture_fixture_raw_json(
    client: &CdpClient,
    server: &FixtureServer,
    name: &str,
) -> Result<(String, AnnotatedPageContent), Box<dyn Error>> {
    let (target_id, session) = open_and_prepare_fixture(client, server, name).await?;
    let captured = async {
        let raw_reply = client
            .send_raw(
                "Page.getAnnotatedPageContent",
                json!({ "includeActionableInformation": true }),
                Some(&session),
            )
            .await?;
        let apc = decode_reply_json(raw_reply.get())?;
        Ok((raw_reply.into_json(), apc))
    }
    .await;
    close_target_best_effort(client, &target_id).await;
    captured
}

/// Opens a fresh target tab, attaches a flattened session, navigates to `name`, waits for fixture
/// readiness, and starts the spin loop for `hung-subframe`.
async fn open_and_prepare_fixture(
    client: &CdpClient,
    server: &FixtureServer,
    name: &str,
) -> Result<(String, SessionId), Box<dyn Error>> {
    let entry_path = fixture_entry_path(name)?;
    let url = server.url("127.0.0.1", entry_path);
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
        page::navigate(
            client,
            NavigateParams {
                url,
                ..NavigateParams::default()
            },
            Some(&session),
        )
        .await?;
        wait_for_ready(client, &session, name).await?;
        if name == "hung-subframe" {
            let triggered = eval_bool(
                client,
                &session,
                "(() => { const f = document.getElementById('hung-frame'); \
                 if (!f || !f.contentWindow) return false; \
                 f.contentWindow.postMessage('spin', '*'); return true; })()",
            )
            .await?;
            if !triggered {
                return Err("hung-subframe failed to locate #hung-frame to start spin loop".into());
            }
            time::sleep(SPIN_SETTLE_DELAY).await;
        }
        Ok(session)
    }
    .await;

    match prepared {
        Ok(session) => Ok((target_id, session)),
        Err(error) => {
            close_target_best_effort(client, &target_id).await;
            Err(error)
        }
    }
}

/// Polls the fixture's readiness predicate until it returns `true` or times out.
async fn wait_for_ready(
    client: &CdpClient,
    session: &SessionId,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    let expr = match name {
        "nested-cross-site-frames" => {
            "document.readyState === 'complete' && (window.__loadedFrames || 0) >= 7"
        }
        "hung-subframe" => {
            "document.readyState === 'complete' && window.__healthyReady === true && window.__hangReady === true"
        }
        _ => "document.readyState === 'complete' && window.__fixtureReady === true",
    };
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        if eval_bool(client, session, expr).await? {
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
                "fixture {name} did not become ready within {READY_TIMEOUT:?} (diag={diag})"
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
