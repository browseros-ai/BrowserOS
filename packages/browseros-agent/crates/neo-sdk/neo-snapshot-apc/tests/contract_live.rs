//! Decode-layer proof for each BrowserOS Chromium patch of the APC wire contract
//! (`apc-contract-design` section 7).
//!
//! The engine that will read these facts does not exist yet, so the behaviour suites cannot prove
//! a patch on the day it lands. Each test here asserts on the wire instead: it loads a page that
//! exercises one patch, captures `Page.getAnnotatedPageContent` the way the engine will (upstream
//! command, default parameters), and checks the decoded BrowserOS fields and the raw reply bytes.
//!
//! A test skips, and passes, when the browser's `contract_version` is below the version its patch
//! ships in. The same file therefore runs against stock Chromium (every test skips) and against
//! every patched build. Set `NEO_APC_EXPECT_CONTRACT_VERSION` to the version the browser under
//! test should report, and every test fails on any other value, absence included.
//!
//! Run from `packages/browseros-agent`:
//! ```sh
//! NEO_E2E_BROWSER="/Users/shadowfax/code/chromium-1/src/out/Default_browseros_arm64/BrowserOS Dev.app/Contents/MacOS/BrowserOS Dev" \
//!   NEO_APC_EXPECT_CONTRACT_VERSION=1 \
//!   bun run test:browser -- cargo test -p neo-snapshot-apc --test contract_live -- --ignored
//! ```

#[path = "support/fixture_server.rs"]
mod fixture_server;

use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fmt::Write as _;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use browseros_cdp::page::{self, EnableParams, GetAnnotatedPageContentParams, NavigateParams};
use browseros_cdp::target::{self, AttachToTargetParams};
use browseros_cdp::{CdpClient, ConnectOptions, SessionId};
use fixture_server::{FixtureServer, Page};
use neo_snapshot_apc::proto::{
    BrowserOsContentAttributes, ContentAttributes, ContentData, ContentNode, FormControlData,
    RedactionDecision,
};
use neo_snapshot_apc::{AnnotatedPageContent, decode};
use serde_json::{Value, json};
use tokio::time::{self, Instant};

type TestResult = Result<(), Box<dyn Error>>;

/// Default CDP port when `BROWSEROS_CDP_PORT` is unset, as `test:browser` and `live_capture` use.
const DEFAULT_CDP_PORT: u16 = 9005;

/// Long enough for a capture that waits out a hung subframe on a debug build.
const CDP_REQUEST_TIMEOUT: Duration = Duration::from_secs(45);

/// Maximum wait for a page to set `window.__fixtureReady`.
const READY_TIMEOUT: Duration = Duration::from_secs(15);

/// Each Chromium patch of the contract, keyed to the `contract_version` of the first browser
/// that carries it (design section 6: PR 1 is version 1, and each later PR adds one).
#[derive(Clone, Copy, Debug)]
enum Patch {
    /// P2: the BrowserOS messages and the contract marker.
    Skeleton,
    /// P1: autocomplete-declared private text controls are redacted in the renderer.
    Privacy,
}

impl Patch {
    const fn contract_version(self) -> u32 {
        match self {
            Self::Skeleton | Self::Privacy => 1,
        }
    }
}

/// One `Page.getAnnotatedPageContent` reply: the protobuf bytes exactly as they crossed the wire,
/// and their decoding.
struct Capture {
    raw: Vec<u8>,
    apc: AnnotatedPageContent,
}

impl Capture {
    fn contract_version(&self) -> Option<u32> {
        self.apc.browseros.as_ref()?.contract_version
    }

    fn count_hits(&self, needle: &str) -> usize {
        self.raw
            .windows(needle.len())
            .filter(|window| *window == needle.as_bytes())
            .count()
    }

    /// Every node of the tree in document order, across frames.
    fn nodes(&self) -> Vec<&ContentNode> {
        let mut out = Vec::new();
        let mut stack: Vec<&ContentNode> = self.apc.root_node.iter().collect();
        while let Some(node) = stack.pop() {
            out.push(node);
            stack.extend(node.children_nodes.iter().rev());
        }
        out
    }

    /// The form-control node whose `name` attribute is `name`.
    fn form_control(
        &self,
        name: &str,
    ) -> Result<(&ContentNode, &ContentAttributes, &FormControlData), String> {
        self.nodes()
            .into_iter()
            .find_map(|node| {
                let attrs = node.content_attributes.as_ref()?;
                let Some(ContentData::FormControl(control)) = attrs.content_data.as_ref() else {
                    return None;
                };
                (control.field_name.as_deref() == Some(name)).then_some((
                    node,
                    attrs,
                    control.as_ref(),
                ))
            })
            .ok_or_else(|| format!("no form-control node with name {name:?}"))
    }
}

/// An open tab on one fixture page, with a flattened CDP session attached.
struct Tab<'a> {
    client: &'a CdpClient,
    target_id: String,
    session: SessionId,
}

impl<'a> Tab<'a> {
    async fn open(client: &'a CdpClient, url: String) -> Result<Self, Box<dyn Error>> {
        let created = client
            .send("Target.createTarget", json!({ "url": "about:blank" }), None)
            .await?;
        let target_id = created
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or("Target.createTarget reply missing targetId")?
            .to_owned();
        let attached = target::attach_to_target(
            client,
            AttachToTargetParams {
                target_id: target_id.clone(),
                flatten: Some(true),
            },
            None,
        )
        .await;
        let tab = match attached {
            Ok(attached) => Tab {
                client,
                target_id,
                session: SessionId::from(attached.session_id),
            },
            Err(error) => {
                close_target(client, &target_id).await;
                return Err(error.into());
            }
        };
        match tab.navigate(url).await {
            Ok(()) => Ok(tab),
            Err(error) => {
                tab.close().await;
                Err(error)
            }
        }
    }

    async fn navigate(&self, url: String) -> TestResult {
        page::enable(self.client, EnableParams::default(), Some(&self.session)).await?;
        page::navigate(
            self.client,
            NavigateParams {
                url,
                ..NavigateParams::default()
            },
            Some(&self.session),
        )
        .await?;
        let deadline = Instant::now() + READY_TIMEOUT;
        while self
            .eval("document.readyState === 'complete' && window.__fixtureReady === true")
            .await?
            != Value::Bool(true)
        {
            if Instant::now() >= deadline {
                return Err(
                    format!("page did not set __fixtureReady within {READY_TIMEOUT:?}").into(),
                );
            }
            time::sleep(Duration::from_millis(50)).await;
        }
        Ok(())
    }

    /// Captures the page with the upstream command and its default parameters, as the engine
    /// will: everything BrowserOS adds is always on, with no request flag.
    async fn capture(&self) -> Result<Capture, Box<dyn Error>> {
        let reply = page::get_annotated_page_content(
            self.client,
            GetAnnotatedPageContentParams::default(),
            Some(&self.session),
        )
        .await?;
        Ok(Capture {
            raw: STANDARD.decode(&reply.content)?,
            apc: decode(&reply.content)?,
        })
    }

    async fn eval(&self, expression: &str) -> Result<Value, Box<dyn Error>> {
        let reply = self
            .client
            .send(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
                Some(&self.session),
            )
            .await?;
        Ok(reply
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null))
    }

    async fn close(self) {
        close_target(self.client, &self.target_id).await;
    }
}

/// Closes `target_id`, ignoring errors so cleanup never masks the test's own result.
async fn close_target(client: &CdpClient, target_id: &str) {
    let _closed = client
        .send("Target.closeTarget", json!({ "targetId": target_id }), None)
        .await;
}

async fn connect_cdp() -> Result<CdpClient, Box<dyn Error>> {
    let port = env::var("BROWSEROS_CDP_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(DEFAULT_CDP_PORT);
    let mut opts = ConnectOptions::new(port);
    opts.request_timeout = CDP_REQUEST_TIMEOUT;
    Ok(CdpClient::connect(opts).await?)
}

fn expected_contract_version() -> Result<Option<u32>, Box<dyn Error>> {
    match env::var("NEO_APC_EXPECT_CONTRACT_VERSION") {
        Ok(value) => Ok(Some(value.parse().map_err(|error| {
            format!("NEO_APC_EXPECT_CONTRACT_VERSION={value:?} is not a number: {error}")
        })?)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Decides whether `patch` is on the wire. Fails when the browser contradicts
/// `NEO_APC_EXPECT_CONTRACT_VERSION`; otherwise returns `false`, after saying so, when the
/// browser predates the patch.
#[expect(
    clippy::print_stderr,
    reason = "a skipped live test says why in the test output, which is its only report"
)]
fn patch_is_live(patch: Patch, capture: &Capture) -> Result<bool, Box<dyn Error>> {
    let reported = capture.contract_version();
    if let Some(expected) = expected_contract_version()? {
        assert_eq!(
            reported,
            Some(expected),
            "NEO_APC_EXPECT_CONTRACT_VERSION={expected}, but the browser reports {reported:?}"
        );
    }
    let live = reported.is_some_and(|version| version >= patch.contract_version());
    if !live {
        eprintln!(
            "skipped {patch:?}: it ships in contract version {}, and the browser reports {}",
            patch.contract_version(),
            reported.map_or_else(|| "none (stock Chromium)".to_owned(), |v| v.to_string())
        );
    }
    Ok(live)
}

/// Serves `pages`, opens `path` in a fresh tab, captures it, and runs `check` when `patch` is on
/// the wire. The tab and the CDP connection are closed whatever `check` returns.
async fn check_patch(
    patch: Patch,
    pages: HashMap<String, Page>,
    path: &str,
    check: impl AsyncFnOnce(&Tab<'_>, &Capture) -> TestResult,
) -> TestResult {
    let server = FixtureServer::start(pages).await?;
    let client = connect_cdp().await?;
    let result = async {
        let tab = Tab::open(&client, server.url("127.0.0.1", path)).await?;
        let checked = async {
            let capture = tab.capture().await?;
            if !patch_is_live(patch, &capture)? {
                return Ok(());
            }
            check(&tab, &capture).await
        }
        .await;
        tab.close().await;
        checked
    }
    .await;
    client.disconnect().await;
    result
}

fn has_text_descendant(node: &ContentNode) -> bool {
    node.children_nodes.iter().any(|child| {
        matches!(
            child
                .content_attributes
                .as_ref()
                .and_then(|attrs| attrs.content_data.as_ref()),
            Some(ContentData::Text(_))
        ) || has_text_descendant(child)
    })
}

fn redacted_value_length(attrs: &ContentAttributes) -> Option<u32> {
    attrs
        .browseros
        .as_deref()
        .and_then(|facts: &BrowserOsContentAttributes| facts.redacted_value_length)
}

// P2: the contract marker, and APC node ids as CDP backend node ids (O9).

const SKELETON_PAGE: &str = r#"<!doctype html><html><head><title>Contract skeleton</title></head>
<body><main><h1>Contract skeleton</h1>
<button id="o9-target" name="o9-target">O9 target</button>
</main><script>window.__fixtureReady = true;</script></body></html>"#;

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT; run under bun run test:browser"]
async fn p2_contract_marker_and_node_ids_resolve_over_cdp() -> TestResult {
    let pages = HashMap::from([("/skeleton.html".to_owned(), Page::html(SKELETON_PAGE))]);
    check_patch(
        Patch::Skeleton,
        pages,
        "/skeleton.html",
        async |tab, capture| {
            let page = capture
                .apc
                .browseros
                .as_ref()
                .ok_or("a live patch implies a BrowserOS page submessage")?;
            assert!(
                page.subframe_timeout_ms.is_some_and(|ms| ms > 0),
                "contract version 1 reports the subframe deadline it captured with, got {:?}",
                page.subframe_timeout_ms
            );

            // O9: the engine turns an APC node id into a CDP backendNodeId for every ref.
            let (_, attrs, _) = capture.form_control("o9-target")?;
            let node_id = attrs
                .common_ancestor_dom_node_id
                .ok_or("the button's APC node has no dom node id")?;
            let resolved = tab
                .client
                .send(
                    "DOM.resolveNode",
                    json!({ "backendNodeId": node_id }),
                    Some(&tab.session),
                )
                .await?;
            let object_id = resolved
                .pointer("/object/objectId")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    format!("DOM.resolveNode({node_id}) returned no object: {resolved}")
                })?;
            let element_id = tab
                .client
                .send(
                    "Runtime.callFunctionOn",
                    json!({
                        "objectId": object_id,
                        "functionDeclaration": "function() { return this.id; }",
                        "returnByValue": true,
                    }),
                    Some(&tab.session),
                )
                .await?;
            assert_eq!(
                element_id.pointer("/result/value"),
                Some(&json!("o9-target")),
                "APC dom node id {node_id} must resolve as a backendNodeId to the same element"
            );
            Ok(())
        },
    )
    .await
}

// P1: declared private text controls are withheld in the renderer.

/// Autocomplete values that withhold a text control's value, and the decision each must carry.
/// Tokens are matched case-insensitively among whitespace-separated tokens, as the page probe
/// matches them (`private-field-values.ts`); the last two rows hold the parse to that.
const PRIVATE_AUTOCOMPLETE: &[(&str, RedactionDecision)] = &[
    (
        "current-password",
        RedactionDecision::RedactedHasBeenPassword,
    ),
    ("new-password", RedactionDecision::RedactedHasBeenPassword),
    (
        "cc-number",
        RedactionDecision::RedactedIsSensitivePaymentField,
    ),
    ("cc-csc", RedactionDecision::RedactedIsSensitivePaymentField),
    ("one-time-code", RedactionDecision::RedactedIsOtp),
    (
        "CC-NUMBER",
        RedactionDecision::RedactedIsSensitivePaymentField,
    ),
    (
        "section-pay shipping cc-csc",
        RedactionDecision::RedactedIsSensitivePaymentField,
    ),
];

/// Autocomplete values that must leave a value in clear: near misses of the private tokens,
/// `off`, and no attribute at all (`None`) on a card-like value.
const CLEAR_AUTOCOMPLETE: &[Option<&str>] = &[
    Some("cc-exp"),
    Some("cc-name"),
    Some("off"),
    Some("cc-numberx"),
    None,
];

/// Every secret on the privacy page contains this, so one scan of the raw reply covers every
/// path a value could leak by: `field_value`, the inner editor's text, `aria-labelledby` text,
/// fallback labels and the frame's text selection.
const SECRET_CANARY: &str = "SecretCanary";

#[derive(Clone, Copy, Debug)]
enum Control {
    Input,
    Textarea,
}

/// One text control on the privacy page.
struct PrivacyField {
    name: String,
    control: Control,
    autocomplete: Option<&'static str>,
    /// The page-authored value: the `value` attribute, or a textarea's text.
    default_value: String,
    /// The live value the page's script assigns after load, as a user's typing would.
    typed_value: String,
    /// `None` when the value must stay clear.
    withheld_as: Option<RedactionDecision>,
}

fn privacy_fields() -> Vec<PrivacyField> {
    let rows = PRIVATE_AUTOCOMPLETE
        .iter()
        .map(|&(token, decision)| (Some(token), Some(decision)))
        .chain(CLEAR_AUTOCOMPLETE.iter().map(|&token| (token, None)));
    let mut fields = Vec::new();
    for (row, (autocomplete, withheld_as)) in rows.enumerate() {
        for control in [Control::Input, Control::Textarea] {
            let name = format!("field-{row}-{control:?}").to_lowercase();
            let (default_value, typed_value) = match (withheld_as, autocomplete) {
                // The lock is two UTF-16 code units, so the length check sees code units.
                (Some(_), _) => (
                    format!("{SECRET_CANARY}-{name}-default"),
                    format!("{SECRET_CANARY}-{name}-typed-🔒"),
                ),
                (None, None) => ("4111111111111111".to_owned(), "4242424242424242".to_owned()),
                (None, Some(_)) => (
                    format!("PublicCanary-{name}-default"),
                    format!("PublicCanary-{name}-typed"),
                ),
            };
            fields.push(PrivacyField {
                name,
                control,
                autocomplete,
                default_value,
                typed_value,
                withheld_as,
            });
        }
    }
    fields
}

fn privacy_page(fields: &[PrivacyField]) -> Result<String, Box<dyn Error>> {
    let mut controls = String::new();
    let mut typed = serde_json::Map::new();
    for field in fields {
        let autocomplete = field
            .autocomplete
            .map(|value| format!(r#" autocomplete="{value}""#))
            .unwrap_or_default();
        let name = &field.name;
        let default_value = &field.default_value;
        match field.control {
            Control::Input => writeln!(
                controls,
                r#"<input id="{name}" name="{name}"{autocomplete} value="{default_value}">"#
            )?,
            // A textarea's text is its default value, and `aria-labelledby` reads it as the
            // target's textContent: the button pointing at it must not carry a secret.
            Control::Textarea => writeln!(
                controls,
                r#"<textarea id="{name}" name="{name}"{autocomplete}>{default_value}</textarea><button aria-labelledby="{name}"></button>"#
            )?,
        }
        typed.insert(field.name.clone(), Value::String(field.typed_value.clone()));
    }
    let selected = fields
        .iter()
        .find(|field| field.withheld_as.is_some() && matches!(field.control, Control::Textarea))
        .map(|field| field.name.clone())
        .unwrap_or_default();
    Ok(format!(
        r#"<!doctype html><html><head><title>Contract privacy</title></head><body><main>
{controls}<!-- The labelledby walk must also skip a private control nested in the named element. -->
<div id="wrapper">Wrapper text <textarea name="nested-private" autocomplete="one-time-code">{SECRET_CANARY}-nested-default</textarea></div>
<button aria-labelledby="wrapper"></button>
</main><script>
const typed = {typed};
for (const [name, value] of Object.entries(typed)) document.getElementsByName(name)[0].value = value;
// A selection inside a private control must not travel as the frame's selected text.
const selected = document.getElementsByName({selected:?})[0];
selected.focus();
selected.select();
window.__fixtureReady = true;
</script></body></html>"#,
        typed = Value::Object(typed),
    ))
}

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT; run under bun run test:browser"]
async fn p1_declared_private_text_controls_are_withheld_in_the_renderer() -> TestResult {
    let fields = privacy_fields();
    let pages = HashMap::from([(
        "/privacy.html".to_owned(),
        Page::html(&privacy_page(&fields)?),
    )]);
    check_patch(Patch::Privacy, pages, "/privacy.html", async |_, capture| {
        for field in &fields {
            let (node, attrs, control) = capture.form_control(&field.name)?;
            let label = format!("{:?} autocomplete={:?}", field.control, field.autocomplete);
            if let Some(decision) = field.withheld_as {
                    assert_eq!(
                        attrs.redaction_decision,
                        Some(i32::from(decision)),
                        "{label}: decision"
                    );
                    assert_eq!(control.field_value, None, "{label}: no field_value");
                    assert_eq!(
                        redacted_value_length(attrs),
                        Some(u32::try_from(field.typed_value.encode_utf16().count())?),
                        "{label}: redacted_value_length is the live value's UTF-16 length"
                    );
                    assert!(
                        !has_text_descendant(node),
                        "{label}: the inner editor's text must not be a node"
                    );
            } else {
                    assert!(
                        matches!(
                            attrs.redaction_decision.map(RedactionDecision::try_from),
                            None | Some(Ok(RedactionDecision::NoRedactionNecessary))
                        ),
                        "{label}: stays clear, got decision {:?}",
                        attrs.redaction_decision
                    );
                    assert_eq!(
                        control.field_value.as_deref(),
                        Some(field.typed_value.as_str()),
                        "{label}: field_value is the live value"
                    );
                    assert_eq!(redacted_value_length(attrs), None, "{label}: no length");
            }
        }

        // The scan can see values: clear live values, and a clear textarea's default text
        // through the button that names it, both reach the wire.
        for field in fields.iter().filter(|field| field.withheld_as.is_none()) {
            assert!(
                capture.count_hits(&field.typed_value) > 0,
                "positive control: clear value {:?} must be on the wire",
                field.typed_value
            );
            if matches!(field.control, Control::Textarea) {
                assert!(
                    capture.count_hits(&field.default_value) > 0,
                    "positive control: clear textarea text {:?} must reach its labelledby button",
                    field.default_value
                );
            }
        }
        assert_eq!(
            capture.count_hits(SECRET_CANARY),
            0,
            "no secret may reach the reply bytes by any path"
        );
        Ok(())
    })
    .await
}
