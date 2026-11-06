//! sdk-privacy's L1 against a real browser, which unit tests cannot stand in for:
//! - **parity:** the Rust classifier (snapshot) and `seal.js` (sealed world) agree on every
//!   field of a fixture that covers each rule;
//! - **snapshot:** with `SensitiveValues::Mask` no secret reaches the snapshot text, echoes
//!   included, while the default `Heuristic` output is unchanged;
//! - **seal:** every read from the sealed world is masked, the page's own world and its form
//!   submission still see raw values, a page patching its own prototypes does not reach the
//!   seal, frames are not crossed, the journal reports each typed value once, the sweep finds
//!   values in open and watched closed shadow roots, and installing twice is harmless;
//! - **detached trees:** a watched element out of the document is swept with its tree, its own
//!   open shadow root included, while it is out, and a tree too big to sweep fails the sweep;
//!   a password the page removes is still one to the sweep after the page toggles it to text,
//!   and a field its labels make sensitive is still sensitive out of the document.
//!
//! Needs a BrowserOS instance (it reads tabs through `Browser.getTabs`) with CDP on
//! `BROWSEROS_CDP_PORT` (default 9005):
//! `cargo test -p browseros-core --test sensitive_fields_live -- --ignored`.
//! The test opens and closes its own tab.

use browseros_cdp::{CdpClient, CdpEvent, ConnectOptions};
use browseros_core::connection::CdpConnection;
use browseros_core::sensitive_fields::{
    JOURNAL_BINDING, JournalEntry, SEALED_WORLD, SensitiveValues, seal_source,
};
use browseros_core::{BrowserSession, BrowserSessionHooks, ProtocolSession, SessionId};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const FIXTURE: &str = include_str!("data/sensitive-fields.html");

/// Where a field lives, which decides how it is focused and which world reads it.
#[derive(Clone, Copy, PartialEq)]
enum Place {
    Main,
    OpenShadow,
    ClosedShadow,
    Iframe,
}

/// (id, label in the snapshot, typed value, place). `None`: the page sets the value itself.
const FIELDS: &[(&str, &str, Option<&str>, Place)] = &[
    ("email", "Email", Some("user@example.com"), Place::Main),
    ("pw", "Password", Some("PWTYPED_4f7h"), Place::Main),
    ("pw_attr", "Prefilled password", None, Place::Main),
    (
        "pw_shown",
        "Shown password",
        Some("PWSHOWN_2j6r"),
        Place::Main,
    ),
    ("pw_new", "New password", Some("PWNEW_6t8b"), Place::Main),
    (
        "pw_mirror",
        "Mirrored password",
        Some("PWMIRROR_5g5g"),
        Place::Main,
    ),
    ("pin", "Account PIN", Some("PIN_8842"), Place::Main),
    ("pw_img", "Backup password", Some("PWIMG_2c2c"), Place::Main),
    (
        "cc_number",
        "Card number",
        Some("4111111111111111"),
        Place::Main,
    ),
    ("cc_exp", "Expiry", Some("12/31"), Place::Main),
    ("cc_csc", "CVC", Some("987"), Place::Main),
    ("cc_name", "Name on card", Some("CCNAME Qxvz"), Place::Main),
    ("cc_noac", "Card no.", Some("5555555555554444"), Place::Main),
    ("cc_month", "Expiry month", None, Place::Main),
    ("otp_ac", "One-time code", Some("482913"), Place::Main),
    ("otp_name", "Verification code", Some("551234"), Place::Main),
    ("d1", "digit 1", Some("7"), Place::Main),
    ("d2", "digit 2", Some("3"), Place::Main),
    ("d3", "digit 3", Some("9"), Place::Main),
    ("d4", "digit 4", Some("2"), Place::Main),
    ("d5", "digit 5", Some("0"), Place::Main),
    ("d6", "digit 6", Some("6"), Place::Main),
    ("notes", "Notes", Some("NOTES_visible_ok"), Place::Main),
    ("otp_backup", "One-time backup codes", None, Place::Main),
    ("search", "Search", Some("cheap flights"), Place::Main),
    (
        "pw_open",
        "Open shadow password",
        Some("PWOPEN_3k3k"),
        Place::OpenShadow,
    ),
    (
        "otp_shadow",
        "Shadow code",
        Some("OTPSHADOW_44"),
        Place::OpenShadow,
    ),
    (
        "pw_bare",
        "Bare field",
        Some("PWBARE_6r6r"),
        Place::OpenShadow,
    ),
    (
        "pw_closed",
        "Closed shadow password",
        Some("PWCLOSED_9q9q"),
        Place::ClosedShadow,
    ),
    (
        "pw_iframe",
        "Iframe password",
        Some("PWIFRAME_7y7y"),
        Place::Iframe,
    ),
];

/// Values that must never appear once Neo masks (the non-sensitive fields' values may).
const SECRETS: &[&str] = &[
    "PWTYPED_4f7h",
    "PWATTR_7d1q",
    "PWSHOWN_2j6r",
    "PWNEW_6t8b",
    "PWMIRROR_5g5g",
    "PIN_8842",
    "PWIMG_2c2c",
    "BACKUP_7777",
    "OTPSHADOW_44",
    "PWBARE_6r6r",
    "4111111111111111",
    "CCNAME Qxvz",
    "5555555555554444",
    "482913",
    "551234",
    "PWOPEN_3k3k",
    "PWCLOSED_9q9q",
    "PWIFRAME_7y7y",
];

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn sensitive_values_are_hidden_at_the_source() -> TestResult {
    let port = std::env::var("BROWSEROS_CDP_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(9005);
    let client = Arc::new(CdpClient::connect(ConnectOptions::new(port)).await?);
    let root = ProtocolSession::root(client.clone());
    let url = format!(
        "data:text/html;charset=utf-8,{}",
        utf8_percent_encode(FIXTURE, NON_ALPHANUMERIC)
    );
    let target = root
        .send_value("Target.createTarget", json!({ "url": "about:blank" }))
        .await?;
    let target_id = target["targetId"]
        .as_str()
        .ok_or("no target id")?
        .to_string();
    let result = exercise(&client, &root, &target_id, &url).await;
    let _ = root
        .send_value("Target.closeTarget", json!({ "targetId": target_id }))
        .await;
    result
}

async fn exercise(
    client: &Arc<CdpClient>,
    root: &ProtocolSession,
    target_id: &str,
    url: &str,
) -> TestResult {
    let attached = root
        .send_value(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
        )
        .await?;
    let session_id = attached["sessionId"].as_str().ok_or("no session id")?;
    let page = ProtocolSession::for_session(client.clone(), SessionId::from(session_id));
    for method in ["Page.enable", "DOM.enable", "Runtime.enable"] {
        page.send_value(method, json!({})).await?;
    }
    let mut journal = client.events_targeted("Runtime.bindingCalled");
    page.send_value(
        "Runtime.addBinding",
        json!({ "name": JOURNAL_BINDING, "executionContextName": SEALED_WORLD }),
    )
    .await?;
    page.send_value("Page.navigate", json!({ "url": url }))
        .await?;
    wait_until(
        &page,
        "document.readyState === 'complete' && !!window.__fields",
    )
    .await?;

    // ---- seal the main frame and the iframe, twice ----
    let tree = page.send_value("Page.getFrameTree", json!({})).await?;
    let main_frame = tree["frameTree"]["frame"]["id"]
        .as_str()
        .ok_or("no frame")?;
    let child_frame = tree["frameTree"]["childFrames"][0]["frame"]["id"]
        .as_str()
        .ok_or("no child frame")?;
    let main = seal(&page, main_frame).await?;
    let child = seal(&page, child_frame).await?;
    let again: i64 = serde_json::from_value(eval(&page, seal_source(), Some(main)).await?)?;
    assert_eq!(again, 1, "a second install returns the installed version");
    watch_closed_shadow_root(&page, main).await?;

    // ---- type with trusted input, the way the owner or a fill would ----
    for &(id, _, value, place) in FIELDS {
        let Some(value) = value else { continue };
        let focus = match place {
            Place::Main => format!("document.getElementById('{id}').focus()"),
            Place::OpenShadow | Place::ClosedShadow => format!("window.__fields.{id}.focus()"),
            Place::Iframe => format!(
                "document.getElementById('child').contentDocument.getElementById('{id}').focus()"
            ),
        };
        eval(&page, &format!("{focus}; true"), None).await?;
        page.send_value("Input.insertText", json!({ "text": value }))
            .await?;
    }
    eval(
        &page,
        "document.getElementById('pw_shown').type = 'text';
         const month = document.getElementById('cc_month');
         month.value = '12';
         month.dispatchEvent(new Event('change', { bubbles: true }));
         true",
        None,
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // ---- the sealed world: classification and masked reads ----
    let read_fields = "(() => {
        const out = {};
        const fields = [...document.querySelectorAll('input,textarea,select')];
        const open = document.getElementById('open_host');
        if (open && open.shadowRoot) fields.push(...open.shadowRoot.querySelectorAll('input'));
        for (const el of fields) out[el.id] = {
            kind: __clawSeal.kind(el), expect: el.dataset.expect,
            value: el.value, attribute: el.getAttribute('value'),
        };
        // A prefilled sensitive textarea, through every way its text can be read.
        const area = document.getElementById('otp_backup');
        if (area) out.otp_backup_text = {
            text: area.textContent, node: area.firstChild && area.firstChild.nodeValue,
            data: area.firstChild && area.firstChild.data, inner: area.innerHTML,
            innerText: area.innerText, parent: area.parentElement.textContent,
        };
        return out;
    })()";
    let mut sealed: HashMap<String, Value> =
        serde_json::from_value(eval(&page, read_fields, Some(main)).await?)?;
    let iframe: HashMap<String, Value> =
        serde_json::from_value(eval(&page, read_fields, Some(child)).await?)?;
    sealed.extend(iframe);
    let mut js_kinds = HashMap::new();
    for &(id, _, value, place) in FIELDS {
        if place == Place::ClosedShadow {
            continue; // unreachable from JavaScript; checked through the sweep and the journal
        }
        let field = sealed.get(id).ok_or(format!("{id} not read"))?;
        let kind = field["kind"].as_str().map(str::to_string);
        let expect = field["expect"]
            .as_str()
            .ok_or(format!("{id} has no data-expect"))?;
        assert_eq!(
            kind.as_deref().unwrap_or("none"),
            expect,
            "{id}: rule table"
        );
        let shown = field["value"].as_str().unwrap_or_default();
        if kind.is_some() {
            assert!(is_mask(shown), "{id}: sealed value {shown:?}");
        } else if let Some(value) = value {
            assert_eq!(shown, value, "{id}: a plain field reads raw");
        }
        js_kinds.insert(id, kind);
    }
    let markup: String = serde_json::from_value(
        eval(&page, "document.documentElement.outerHTML", Some(main)).await?,
    )?;
    assert_no_secret("sealed outerHTML", &markup);
    let body_text: String =
        serde_json::from_value(eval(&page, "document.body.textContent", Some(main)).await?)?;
    assert_no_secret("sealed body text", &body_text);

    // Password history and selection inside an open shadow root.
    let bare = "document.getElementById('open_host').shadowRoot.getElementById('pw_bare').value";
    assert_eq!(eval(&page, bare, Some(main)).await?, json!("••••"));
    eval(&page, "window.__fields.pw_bare.type = 'text'; true", None).await?;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let shadow = eval(
        &page,
        "(() => {
            const root = document.getElementById('open_host').shadowRoot;
            const code = root.getElementById('otp_shadow');
            code.focus(); code.select();
            return {
                after: root.getElementById('pw_bare').value,
                swept: __clawSeal.sensitiveValues().includes('PWBARE_6r6r'),
                selection: String(getSelection()),
            };
        })()",
        Some(main),
    )
    .await?;
    assert_eq!(
        shadow["after"],
        json!("••••"),
        "a toggled shadow password stays masked"
    );
    assert_eq!(shadow["swept"], json!(true));
    assert_eq!(
        shadow["selection"],
        json!(""),
        "a selection in a shadow root is hidden"
    );
    // Rust cannot see that history (risk R3), so restore the type before the snapshot checks.
    eval(
        &page,
        "window.__fields.pw_bare.type = 'password'; true",
        None,
    )
    .await?;
    assert_no_secret("sealed field reads", &serde_json::to_string(&sealed)?);

    let checks: Value = eval(
        &page,
        "(() => {
            const otp = document.getElementById('otp_ac');
            const prefilled = document.getElementById('pw_attr');
            const card = document.getElementById('cc_number');
            card.focus(); card.select();
            return {
                equals: [__clawSeal.valueEquals(otp, '482913'), __clawSeal.valueEquals(otp, '000000')],
                attributes: [
                    __clawSeal.attributeEquals(prefilled, 'value', 'PWATTR_7d1q'),
                    __clawSeal.attributeEquals(prefilled, 'value', 'PWATTR_7d1'),
                    __clawSeal.attributeEquals(otp, 'value', '482913'),
                ],
                length: __clawSeal.rawLength(document.getElementById('pw')),
                selection: String(getSelection()),
                frame: document.getElementById('child').contentDocument,
                formData: typeof FormData,
                binding: typeof __neoJournal,
                sweep: __clawSeal.sensitiveValues(),
            };
        })()",
        Some(main),
    )
    .await?;
    assert_eq!(checks["equals"], json!([true, false]));
    assert_eq!(
        checks["attributes"],
        json!([true, false, false]),
        "attributeEquals compares the markup exactly, not the typed value"
    );
    assert_eq!(checks["length"], json!(12));
    assert_eq!(checks["selection"], json!(""));
    assert_eq!(
        checks["frame"],
        Value::Null,
        "the seal never crosses into a child frame"
    );
    assert_eq!(checks["formData"], json!("undefined"));
    assert_eq!(
        checks["binding"],
        json!("undefined"),
        "the seal keeps the binding private"
    );
    let sweep = checks["sweep"].as_array().ok_or("no sweep")?;
    for secret in SECRETS
        .iter()
        .filter(|secret| !secret.starts_with("PWIFRAME"))
    {
        let found = sweep
            .iter()
            .any(|value| value.as_str().is_some_and(|value| value.contains(secret)));
        assert!(found, "the sweep missed {secret}");
    }
    assert!(!sweep.contains(&json!("user@example.com")));

    // ---- the journal: each value entered into a sensitive field, once, nothing else ----
    let mut journaled = Vec::new();
    while let Ok(event) = journal.try_recv() {
        if event.params["name"] == JOURNAL_BINDING
            && let Some(entry) = event.params["payload"]
                .as_str()
                .and_then(JournalEntry::parse)
        {
            journaled.push(entry.value.to_string());
        }
    }
    let mut expected = FIELDS
        .iter()
        .filter(|(id, ..)| js_kinds.get(id).is_none_or(Option::is_some))
        .filter_map(|(_, _, value, _)| value.map(str::to_string))
        .collect::<Vec<_>>();
    expected.push("12".to_string());
    journaled.sort();
    expected.sort();
    assert_eq!(journaled, expected, "journal entries");

    // ---- the page's own world is untouched, and its own patches do not reach the seal ----
    let page_view = eval(
        &page,
        "(() => {
            const raw = document.getElementById('pw').value;
            const seal = typeof __clawSeal;
            const patched = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value');
            Object.defineProperty(HTMLInputElement.prototype, 'value', {
                ...patched, get() { return 'PAGE_PATCHED'; },
            });
            return { raw, seal, formData: typeof FormData, now: document.getElementById('email').value };
        })()",
        None,
    )
    .await?;
    assert_eq!(page_view["raw"], json!("PWTYPED_4f7h"));
    assert_eq!(page_view["seal"], json!("undefined"));
    assert_eq!(page_view["formData"], json!("function"));
    assert_eq!(page_view["now"], json!("PAGE_PATCHED"));
    let after_patch = eval(
        &page,
        "[document.getElementById('email').value, document.getElementById('pw').value]",
        Some(main),
    )
    .await?;
    assert_eq!(after_patch, json!(["user@example.com", "••••"]));

    // ---- the snapshot: Rust's classifier agrees with the seal, and nothing leaks ----
    let browser = BrowserSession::new(
        client.clone() as Arc<dyn CdpConnection>,
        BrowserSessionHooks::default(),
    );
    let page_id = browser
        .pages
        .list()
        .await?
        .into_iter()
        .find(|info| info.target_id.as_str() == target_id)
        .ok_or("the fixture tab is not listed")?
        .page_id;
    // Page text that repeats a field's value. The seal leaves page text alone (that is L2's
    // job); the snapshot masks the echo of a field it masked.
    eval(
        &page,
        "document.getElementById('echo').textContent = 'We sent code 482913 to your phone'; true",
        None,
    )
    .await?;
    let observer = browser.observe(page_id).await;
    let heuristic = observer.snapshot().await?.text;
    assert!(
        heuristic.contains(r#": "482913""#),
        "the default is unchanged:\n{heuristic}"
    );
    browser.set_sensitive_values(SensitiveValues::Mask);
    // The baseline above is raw; a diff right after the switch must not print it.
    let diff = observer.diff().await?;
    assert_no_secret("diff after the switch", &diff.text);
    let masked = observer.snapshot().await?.text;
    assert_no_secret("masked snapshot", &masked);
    assert!(
        masked.contains("We sent code [private] to your phone"),
        "the echo is masked:\n{masked}"
    );
    for &(id, label, ..) in FIELDS {
        let line = masked
            .lines()
            .find(|line| line.contains(&format!("\"{label}\"")) && line.contains("[ref="))
            .ok_or(format!("no snapshot line for {label}:\n{masked}"))?;
        let shown = line
            .rsplit_once(": \"")
            .map(|(_, value)| value.trim_end_matches('"'))
            .unwrap_or_default();
        let rust_masks = is_mask(shown);
        let js_masks = match js_kinds.get(id) {
            Some(kind) => kind.is_some(),
            None => true, // the closed shadow root's password
        };
        assert_eq!(rust_masks, js_masks, "parity for {id}: {line}");
    }

    // ---- the page still submits real values: the seal changed nothing it sends ----
    page.send_value("Network.enable", json!({})).await?;
    let mut requests = client.events_targeted("Network.requestWillBeSent");
    eval(
        &page,
        "const login = document.getElementById('login');
         login.action = 'http://127.0.0.1:9/sdk-privacy-login';
         login.submit(); true",
        None,
    )
    .await?;
    let body = post_body(&mut requests).await?;
    assert!(
        body.contains("PWTYPED_4f7h") && body.contains("PWMIRROR_5g5g"),
        "{body}"
    );
    Ok(())
}

/// A page that removes a password field's container, and a shadow host with a password in its
/// own open root, keeps both alive in globals of its own world.
const DETACHED: &str = r#"<!doctype html><body>
<div id="box"><input id="gone" type="password"></div>
<div id="host"></div>
<script>
  window.box = document.getElementById('box');
  window.host = document.getElementById('host');
  window.host.attachShadow({ mode: 'open' }).innerHTML = '<input id="inner" type="password">';
  window.ready = true;
</script></body>"#;

/// The node `expression` names in the page's own world, handed to the seal's `watch` in the
/// sealed world `context`, as the evaluate guard hands it the nodes a script receives.
async fn watch_node(page: &ProtocolSession, context: i64, expression: &str) -> TestResult {
    let named = page
        .send_value("Runtime.evaluate", json!({ "expression": expression }))
        .await?;
    let object = named["result"]["objectId"]
        .as_str()
        .ok_or_else(|| format!("the page named no node: {named}"))?;
    let described = page
        .send_value("DOM.describeNode", json!({ "objectId": object }))
        .await?;
    let resolved = page
        .send_value(
            "DOM.resolveNode",
            json!({ "backendNodeId": described["node"]["backendNodeId"],
                    "executionContextId": context }),
        )
        .await?;
    let sealed = resolved["object"]["objectId"]
        .as_str()
        .ok_or("the node did not resolve in the sealed world")?;
    page.send_value(
        "Runtime.callFunctionOn",
        json!({
            "objectId": sealed,
            "functionDeclaration": "function () { __clawSeal.watch(this); return true; }",
            "returnByValue": true,
        }),
    )
    .await?;
    Ok(())
}

/// A fresh tab on the BrowserOS at `BROWSEROS_CDP_PORT`, for a test that closes it with
/// `close_tab` whatever its result.
async fn open_tab() -> TestResult<(Arc<CdpClient>, ProtocolSession, String)> {
    let port = std::env::var("BROWSEROS_CDP_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(9005);
    let client = Arc::new(CdpClient::connect(ConnectOptions::new(port)).await?);
    let root = ProtocolSession::root(client.clone());
    let target = root
        .send_value("Target.createTarget", json!({ "url": "about:blank" }))
        .await?;
    let target_id = target["targetId"]
        .as_str()
        .ok_or("no target id")?
        .to_string();
    Ok((client, root, target_id))
}

async fn close_tab(root: &ProtocolSession, target_id: &str) {
    let _ = root
        .send_value("Target.closeTarget", json!({ "targetId": target_id }))
        .await;
}

/// The tab's page with `html` loaded once `window.ready` is set, and the seal installed in its
/// main frame; returns the page and the sealed world's context id.
async fn sealed_page(
    client: &Arc<CdpClient>,
    root: &ProtocolSession,
    target_id: &str,
    html: &str,
) -> TestResult<(ProtocolSession, i64)> {
    let attached = root
        .send_value(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
        )
        .await?;
    let session_id = attached["sessionId"].as_str().ok_or("no session id")?;
    let page = ProtocolSession::for_session(client.clone(), SessionId::from(session_id));
    for method in ["Page.enable", "DOM.enable", "Runtime.enable"] {
        page.send_value(method, json!({})).await?;
    }
    let url = format!(
        "data:text/html;charset=utf-8,{}",
        utf8_percent_encode(html, NON_ALPHANUMERIC)
    );
    page.send_value("Page.navigate", json!({ "url": url }))
        .await?;
    wait_until(&page, "window.ready === true").await?;
    let tree = page.send_value("Page.getFrameTree", json!({})).await?;
    let frame = tree["frameTree"]["frame"]["id"]
        .as_str()
        .ok_or("no frame")?;
    let sealed = seal(&page, frame).await?;
    Ok((page, sealed))
}

/// Whether a sweep's values hold `secret`.
fn swept(values: &Value, secret: &str) -> bool {
    values
        .as_array()
        .is_some_and(|values| values.iter().any(|value| value == secret))
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn detached_trees_are_swept_while_they_live() -> TestResult {
    let (client, root, target_id) = open_tab().await?;
    let result = detached(&client, &root, &target_id).await;
    close_tab(&root, &target_id).await;
    result
}

async fn detached(client: &Arc<CdpClient>, root: &ProtocolSession, target_id: &str) -> TestResult {
    let (page, sealed) = sealed_page(client, root, target_id, DETACHED).await?;
    eval(
        &page,
        "document.getElementById('gone').value = 'PWGONE_1a1a';
         host.shadowRoot.getElementById('inner').value = 'PWHOSTROOT_2b2b';
         true",
        None,
    )
    .await?;
    // Watched while still in the document: its state is read at each sweep.
    watch_node(&page, sealed, "box").await?;
    eval(&page, "box.remove(); host.remove(); true", None).await?;
    let unwatched = eval(&page, "__clawSeal.sensitiveValues()", Some(sealed)).await?;
    assert!(
        swept(&unwatched, "PWGONE_1a1a"),
        "a node watched before its removal is swept after it: {unwatched}"
    );
    assert!(
        !swept(&unwatched, "PWHOSTROOT_2b2b"),
        "the document's sweep does not reach a removed host it was never handed"
    );
    watch_node(&page, sealed, "host").await?;
    let watched = eval(&page, "__clawSeal.sensitiveValues()", Some(sealed)).await?;
    assert!(
        swept(&watched, "PWHOSTROOT_2b2b"),
        "a removed host's own open root is swept once it is watched: {watched}"
    );
    assert_eq!(
        eval(&page, "__clawSeal.detachedTops().length", Some(sealed)).await?,
        json!(2)
    );
    watch_node(&page, sealed, "box.querySelector('#gone')").await?;
    eval(&page, "document.body.append(box); true", None).await?;
    assert_eq!(
        eval(&page, "__clawSeal.detachedTops().length", Some(sealed)).await?,
        json!(1),
        "a tree put back is the document's again"
    );
    let back = eval(&page, "__clawSeal.sensitiveValues()", Some(sealed)).await?;
    assert!(swept(&back, "PWGONE_1a1a"), "{back}");
    eval(
        &page,
        "window.huge = document.createElement('div');
         for (let i = 0; i < 50001; i++) huge.append(document.createElement('span'));
         true",
        None,
    )
    .await?;
    watch_node(&page, sealed, "huge").await?;
    let failed = eval(&page, "__clawSeal.sensitiveValues()", Some(sealed)).await;
    assert!(
        failed.is_err(),
        "a detached tree too big to sweep fails the sweep instead of leaving fields unseen"
    );
    Ok(())
}

/// A page with nothing in it when the seal is installed: every field below is added later, so
/// the install's own scan of the document never sees it.
const EMPTY: &str = "<!doctype html><body><script>window.ready = true;</script></body>";

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn a_password_out_of_the_document_stays_one() -> TestResult {
    let (client, root, target_id) = open_tab().await?;
    let result = toggled_out_of_the_document(&client, &root, &target_id).await;
    close_tab(&root, &target_id).await;
    result
}

/// The page's show-password toggle on fields it removed: each one, however the seal came to know
/// it was a password, is still one to the sweep after the page turns it into text.
async fn toggled_out_of_the_document(
    client: &Arc<CdpClient>,
    root: &ProtocolSession,
    target_id: &str,
) -> TestResult {
    let (page, sealed) = sealed_page(client, root, target_id, EMPTY).await?;
    eval(
        &page,
        "window.form = document.createElement('form');
         form.innerHTML = '<input id=typed type=password><input id=set type=password><input id=plain>';
         window.early = document.createElement('input');
         early.type = 'password';
         document.body.append(form, early);
         form.querySelector('#set').value = 'PWSET_2b2b';
         form.querySelector('#plain').value = 'PLAIN_ok';
         window.lone = document.createElement('input');
         lone.type = 'password';
         lone.value = 'PWLONE_5e5e';
         window.kept = document.createElement('div');
         document.body.append(kept);
         true",
        None,
    )
    .await?;
    for (field, text) in [
        ("form.querySelector('#typed')", "PWTYPED_1a1a"),
        ("early", "PWEARLY_4d4d"),
    ] {
        eval(&page, &format!("{field}.focus(); true"), None).await?;
        page.send_value("Input.insertText", json!({ "text": text }))
            .await?;
    }
    // Each `eval` is a task of its own, so the removal's mutation records are delivered before
    // the toggle that follows it.
    eval(&page, "early.remove(); true", None).await?;
    eval(&page, "early.type = 'text'; true", None).await?;
    watch_node(&page, sealed, "early").await?;
    watch_node(&page, sealed, "kept").await?;
    eval(&page, "kept.remove(); true", None).await?;
    eval(&page, "form.remove(); true", None).await?;
    watch_node(&page, sealed, "form").await?;
    watch_node(&page, sealed, "lone").await?;
    eval(
        &page,
        "form.querySelector('#typed').type = 'text';
         form.querySelector('#set').type = 'text';
         lone.type = 'text';
         true",
        None,
    )
    .await?;
    let toggled = eval(&page, "__clawSeal.sensitiveValues()", Some(sealed)).await?;
    let unswept = missing(
        &toggled,
        &[
            (
                "PWEARLY_4d4d",
                "typed in the document, removed, then toggled before anything watched it",
            ),
            (
                "PWTYPED_1a1a",
                "typed in the document, toggled once watched",
            ),
            ("PWSET_2b2b", "set by the page, toggled once watched"),
            ("PWLONE_5e5e", "never in the document, toggled once watched"),
        ],
    );
    assert!(unswept.is_empty(), "still passwords: {unswept:?}");
    assert!(
        !swept(&toggled, "PLAIN_ok"),
        "a plain field out of the document is not swept: {toggled}"
    );
    eval(
        &page,
        "form.querySelector('#typed').value = 'PWTYPED_new_1a';
         form.querySelector('#set').value = 'PWSET_new_2b';
         for (const [tree, value] of [[form, 'PWADDED_3c3c'], [kept, 'PWKEPT_6f6f']]) {
           const added = document.createElement('input');
           added.type = 'password';
           added.value = value;
           tree.append(added);
         }
         true",
        None,
    )
    .await?;
    eval(
        &page,
        "for (const tree of [form, kept]) tree.lastChild.type = 'text'; true",
        None,
    )
    .await?;
    let changed = eval(&page, "__clawSeal.sensitiveValues()", Some(sealed)).await?;
    let unswept = missing(
        &changed,
        &[
            ("PWTYPED_new_1a", "a value the page sets after the toggle"),
            ("PWSET_new_2b", "a value the page sets after the toggle"),
            (
                "PWADDED_3c3c",
                "added to a tree removed before its watch, toggled before the next sweep",
            ),
            (
                "PWKEPT_6f6f",
                "added to a tree removed after its watch, toggled before the next sweep",
            ),
        ],
    );
    assert!(unswept.is_empty(), "still passwords: {unswept:?}");
    Ok(())
}

/// Fields that only their labels make sensitive: a `for` label, a label around the field, and
/// `aria-labelledby`, in a tree the page removes with them and in a fragment it never attached.
const LABELLED: &str = r#"<!doctype html><body>
<div id="box"><label for="f1">Card number</label><input id="f1"><label>Verification code
<input id="f2"></label><span id="l3">Password</span><input id="f3" aria-labelledby="l3">
<input id="f4"></div>
<script>
  window.box = document.getElementById('box');
  window.frag = document.createDocumentFragment();
  const inner = document.createElement('div');
  inner.innerHTML = '<label for="g1">Card number</label><input id="g1">'
    + '<span id="m3">Password</span><input id="g3" aria-labelledby="m3">';
  frag.append(inner);
  window.ready = true;
</script></body>"#;

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn a_labelled_field_out_of_the_document_keeps_its_label() -> TestResult {
    let (client, root, target_id) = open_tab().await?;
    let result = labelled_out_of_the_document(&client, &root, &target_id).await;
    close_tab(&root, &target_id).await;
    result
}

async fn labelled_out_of_the_document(
    client: &Arc<CdpClient>,
    root: &ProtocolSession,
    target_id: &str,
) -> TestResult {
    let (page, sealed) = sealed_page(client, root, target_id, LABELLED).await?;
    eval(
        &page,
        "const values = { f1: '4111111111111111', f2: '551234', f3: 'PWLABEL_3c3c', f4: 'PLAIN_ok' };
         for (const [id, value] of Object.entries(values)) box.querySelector('#' + id).value = value;
         frag.querySelector('#g1').value = '5555555555554444';
         frag.querySelector('#g3').value = 'PWFRAG_7d7d';
         true",
        None,
    )
    .await?;
    let kinds = "(() => {
        const out = {};
        for (const el of document.querySelector('#box').querySelectorAll('input'))
            out[el.id] = __clawSeal.kind(el);
        return out;
    })()";
    let attached = eval(&page, kinds, Some(sealed)).await?;
    assert_eq!(
        attached,
        json!({ "f1": "card", "f2": "one_time_code", "f3": "password", "f4": null }),
        "the labels classify the fields in the document"
    );
    eval(
        &page,
        "window.held = document.querySelector('#box'); true",
        Some(sealed),
    )
    .await?;
    eval(&page, "box.remove(); true", None).await?;
    let removed = eval(
        &page,
        &kinds.replace("document.querySelector('#box')", "held"),
        Some(sealed),
    )
    .await?;
    assert_eq!(
        removed, attached,
        "a field the page removes with its labels keeps its kind"
    );
    watch_node(&page, sealed, "box").await?;
    watch_node(&page, sealed, "frag").await?;
    let values = eval(&page, "__clawSeal.sensitiveValues()", Some(sealed)).await?;
    let unswept = missing(
        &values,
        &[
            ("4111111111111111", "a `for` label's card number"),
            ("551234", "a code in the label around it"),
            ("PWLABEL_3c3c", "a password named by aria-labelledby"),
            ("5555555555554444", "a `for` label in a fragment"),
            ("PWFRAG_7d7d", "aria-labelledby in a fragment"),
        ],
    );
    assert!(
        unswept.is_empty(),
        "labelled out of the document: {unswept:?}"
    );
    assert!(
        !swept(&values, "PLAIN_ok"),
        "an unlabelled field stays plain: {values}"
    );
    Ok(())
}

/// Each `(secret, why)` the sweep's values do not hold, with the values, so one run names every
/// field a sweep left out.
fn missing<'a>(values: &Value, expected: &[(&str, &'a str)]) -> Vec<(String, &'a str)> {
    expected
        .iter()
        .filter(|(secret, _)| !swept(values, secret))
        .map(|&(secret, why)| (format!("{secret} ({values})"), why))
        .collect()
}

/// Creates the sealed world in a frame and installs the seal; returns the context id.
async fn seal(page: &ProtocolSession, frame_id: &str) -> TestResult<i64> {
    let world = page
        .send_value(
            "Page.createIsolatedWorld",
            json!({ "frameId": frame_id, "worldName": SEALED_WORLD }),
        )
        .await?;
    let context = world["executionContextId"]
        .as_i64()
        .ok_or("no execution context")?;
    let version: i64 = serde_json::from_value(eval(page, seal_source(), Some(context)).await?)?;
    assert_eq!(version, 1);
    Ok(context)
}

/// Finds the closed shadow root through the pierced DOM, as Rust does at takeover, and hands it
/// to the seal so the journal and the sweep cover it.
async fn watch_closed_shadow_root(page: &ProtocolSession, context: i64) -> TestResult {
    let document = page
        .send_value("DOM.getDocument", json!({ "depth": -1, "pierce": true }))
        .await?;
    let backend = closed_root(&document["root"]).ok_or("no closed shadow root")?;
    let resolved = page
        .send_value(
            "DOM.resolveNode",
            json!({ "backendNodeId": backend, "executionContextId": context }),
        )
        .await?;
    let object = resolved["object"]["objectId"]
        .as_str()
        .ok_or("closed root did not resolve")?;
    page.send_value(
        "Runtime.callFunctionOn",
        json!({
            "objectId": object,
            "functionDeclaration": "function () { __clawSeal.watch(this); return true; }",
            "returnByValue": true,
        }),
    )
    .await?;
    Ok(())
}

fn closed_root(node: &Value) -> Option<i64> {
    for root in node["shadowRoots"].as_array().into_iter().flatten() {
        if root["shadowRootType"] == "closed" {
            return root["backendNodeId"].as_i64();
        }
    }
    let children = node["children"].as_array().into_iter().flatten();
    let content = node.get("contentDocument").into_iter();
    children.chain(content).find_map(closed_root)
}

async fn eval(page: &ProtocolSession, expression: &str, context: Option<i64>) -> TestResult<Value> {
    let mut params =
        json!({ "expression": expression, "returnByValue": true, "awaitPromise": true });
    if let Some(context) = context {
        params["contextId"] = json!(context);
    }
    let result = page.send_value("Runtime.evaluate", params).await?;
    if let Some(exception) = result.get("exceptionDetails") {
        return Err(format!("script threw: {exception}").into());
    }
    Ok(result["result"]["value"].clone())
}

async fn wait_until(page: &ProtocolSession, condition: &str) -> TestResult {
    for _ in 0..100 {
        if eval(page, condition, None).await.ok() == Some(json!(true)) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(format!("timed out waiting for {condition}").into())
}

async fn post_body(requests: &mut UnboundedReceiver<CdpEvent>) -> TestResult<String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let event = tokio::time::timeout_at(deadline, requests.recv())
            .await?
            .ok_or("event stream closed")?;
        if let Some(body) = event.params["request"]["postData"].as_str()
            && event.params["request"]["url"]
                .as_str()
                .is_some_and(|url| url.ends_with("/sdk-privacy-login"))
        {
            return Ok(body.to_string());
        }
    }
}

/// `••••`, or `•••• 1234` for a card number.
fn is_mask(shown: &str) -> bool {
    shown == "••••"
        || shown
            .strip_prefix("•••• ")
            .is_some_and(|rest| rest.len() == 4)
}

fn assert_no_secret(place: &str, text: &str) {
    for secret in SECRETS {
        assert!(!text.contains(secret), "{place} leaked {secret}:\n{text}");
    }
}
