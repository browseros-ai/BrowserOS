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
//!
//! Do not run it under `nice`: a debug component build's cold start nearly fills `test:browser`'s
//! 30 s CDP deadline. The P5 test also needs BrowserOS's takeover bar, which only a browser
//! launched through `crates/neo-sdk/neo-playwright/tests/support/neo-with-takeover.sh` mounts
//! (`NEO_TAKEOVER_BROWSER` picks the build); without it that test skips and says so.

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
    BrowserOsContentAttributes, BrowserOsRef, BrowserOsTriState, ContentAttributeType,
    ContentAttributes, ContentData, ContentNode, FormControlData, IframeDataContent,
    REASON_BROWSEROS_TIMEOUT, RedactedFrameMetadata, RedactionDecision,
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
    /// P6: frame tokens, the 2 s subframe deadline, and the timeout marker.
    Frames,
    /// P3: role-bearing elements are kept whatever the scroll position; closed-root seeds.
    Retention,
    /// P4: tag, raw attributes, hidden-ness, text and tree-shape facts.
    TreeFacts,
    /// P5: BrowserOS's own UI stays out of the tree.
    BrowserOsUi,
    /// P7: names and descriptions.
    Names,
    /// P8: states.
    States,
    /// P11: `display: contents` elements get a node.
    DisplayContents,
}

impl Patch {
    const fn contract_version(self) -> u32 {
        match self {
            Self::Skeleton | Self::Privacy => 1,
            Self::Frames => 2,
            Self::Retention | Self::TreeFacts | Self::BrowserOsUi => 3,
            Self::Names | Self::States => 4,
            Self::DisplayContents => 5,
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

    /// The node of the element CDP knows as `backend_node_id`. APC node ids are backend node ids
    /// (O9, proven by the P2 test), but only within one renderer, so pages that look nodes up
    /// this way have a single frame.
    fn node_for(&self, backend_node_id: i32) -> Option<&ContentNode> {
        self.nodes()
            .into_iter()
            .find(|node| dom_node_id(node) == Some(backend_node_id))
    }

    /// The text node whose rendered text, trimmed, is `text`.
    fn text_node(&self, text: &str) -> Result<&ContentNode, String> {
        self.nodes()
            .into_iter()
            .find(|node| {
                matches!(
                    node.content_attributes.as_ref().and_then(|attrs| attrs.content_data.as_ref()),
                    Some(ContentData::Text(info))
                        if info.text_content.as_deref().map(str::trim) == Some(text)
                )
            })
            .ok_or_else(|| format!("no text node reads {text:?}"))
    }

    /// The content of every iframe node, in document order.
    fn iframes(&self) -> Vec<&IframeDataContent> {
        self.nodes()
            .into_iter()
            .filter_map(
                |node| match node.content_attributes.as_ref()?.content_data.as_ref()? {
                    ContentData::Iframe(iframe) => iframe.data.as_ref(),
                    _ => None,
                },
            )
            .collect()
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

    async fn send(&self, method: &str, params: Value) -> Result<Value, Box<dyn Error>> {
        Ok(self
            .client
            .send(method, params, Some(&self.session))
            .await?)
    }

    async fn eval(&self, expression: &str) -> Result<Value, Box<dyn Error>> {
        let reply = self
            .send(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
            )
            .await?;
        Ok(reply
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// CDP's `backendNodeId` for the node that `expression` evaluates to in the page.
    async fn backend_node_id(&self, expression: &str) -> Result<i32, Box<dyn Error>> {
        let evaluated = self
            .send("Runtime.evaluate", json!({ "expression": expression }))
            .await?;
        let object_id = evaluated
            .pointer("/result/objectId")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{expression} is not a node: {evaluated}"))?;
        let described = self
            .send("DOM.describeNode", json!({ "objectId": object_id }))
            .await?;
        let id = described
            .pointer("/node/backendNodeId")
            .and_then(Value::as_i64)
            .ok_or_else(|| format!("DOM.describeNode gave no backendNodeId: {described}"))?;
        Ok(i32::try_from(id)?)
    }

    /// The APC node of the element `selector` matches, if the capture has one.
    async fn element_node<'c>(
        &self,
        capture: &'c Capture,
        selector: &str,
    ) -> Result<Option<&'c ContentNode>, Box<dyn Error>> {
        let id = self
            .backend_node_id(&format!("document.querySelector({selector:?})"))
            .await?;
        Ok(capture.node_for(id))
    }

    /// The APC node of the element `selector` matches; an error when the capture has none.
    async fn require_node<'c>(
        &self,
        capture: &'c Capture,
        selector: &str,
    ) -> Result<&'c ContentNode, Box<dyn Error>> {
        Ok(self
            .element_node(capture, selector)
            .await?
            .ok_or_else(|| format!("{selector} has no node"))?)
    }

    /// The BrowserOS facts of the element `selector` matches, all unset when it carries none.
    async fn facts_at(
        &self,
        capture: &Capture,
        selector: &str,
    ) -> Result<BrowserOsContentAttributes, Box<dyn Error>> {
        Ok(facts_of(self.require_node(capture, selector).await?))
    }

    /// Calls `function` with `this` bound to the node CDP knows as `backend_node_id`.
    async fn call_on_node(
        &self,
        backend_node_id: i32,
        function: &str,
    ) -> Result<Value, Box<dyn Error>> {
        let resolved = self
            .send(
                "DOM.resolveNode",
                json!({ "backendNodeId": backend_node_id }),
            )
            .await?;
        let object_id = resolved
            .pointer("/object/objectId")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                format!("DOM.resolveNode({backend_node_id}) returned no object: {resolved}")
            })?;
        let called = self
            .send(
                "Runtime.callFunctionOn",
                json!({
                    "objectId": object_id,
                    "functionDeclaration": function,
                    "returnByValue": true,
                }),
            )
            .await?;
        Ok(called
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

/// Writes one line to the test output, which is a live test's only report: why it skipped, or a
/// number a Chromium PR records (O1).
#[expect(
    clippy::print_stderr,
    reason = "live tests report through the test output; there is no other channel"
)]
fn report(line: &str) {
    eprintln!("{line}");
}

/// Decides whether `patch` is on the wire. Fails when the browser contradicts
/// `NEO_APC_EXPECT_CONTRACT_VERSION`; otherwise returns `false`, after saying so, when the
/// browser predates the patch.
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
        report(&format!(
            "skipped {patch:?}: it ships in contract version {}, and the browser reports {}",
            patch.contract_version(),
            reported.map_or_else(|| "none (stock Chromium)".to_owned(), |v| v.to_string())
        ));
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

fn dom_node_id(node: &ContentNode) -> Option<i32> {
    node.content_attributes
        .as_ref()?
        .common_ancestor_dom_node_id
}

/// The node's BrowserOS facts; `None` when no fact applies or the browser predates them.
fn facts(node: &ContentNode) -> Option<&BrowserOsContentAttributes> {
    node.content_attributes.as_ref()?.browseros.as_deref()
}

/// The node's BrowserOS facts, all unset when it carries none.
fn facts_of(node: &ContentNode) -> BrowserOsContentAttributes {
    facts(node).cloned().unwrap_or_default()
}

/// Every descendant of `node`, not counting `node` itself.
fn descendants(node: &ContentNode) -> Vec<&ContentNode> {
    let mut out = Vec::new();
    let mut stack: Vec<&ContentNode> = node.children_nodes.iter().collect();
    while let Some(next) = stack.pop() {
        out.push(next);
        stack.extend(&next.children_nodes);
    }
    out
}

fn redacted_value_length(attrs: &ContentAttributes) -> Option<u32> {
    attrs.browseros.as_deref()?.redacted_value_length
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
            assert_eq!(
                tab.call_on_node(node_id, "function() { return this.id; }")
                    .await?,
                json!("o9-target"),
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

// P6: every frame names its CDP frame, and a hung frame is marked as a timeout.

const FRAMES_TOP: &str = r"<!doctype html><html><head><title>Contract frames</title></head>
<body><h1>Frames top</h1><script>
// Three sites on one port: 127.0.0.1, localhost and [::1]. With `?hang` a third child, on its
// own site and so in its own process, joins and spins once told to.
const wanted = location.search.includes('hang') ? 3 : 2;
let ready = 0;
addEventListener('message', (event) => {
  if (event.data === 'ready') ready += 1;
  window.__fixtureReady = ready >= wanted;
});
const children = [['same', '127.0.0.1', '/frames/child.html'], ['cross', 'localhost', '/frames/child.html']];
if (wanted === 3) children.push(['hung', '[::1]', '/frames/hung.html']);
for (const [id, host, path] of children) {
  const frame = document.createElement('iframe');
  frame.id = id;
  frame.src = `http://${host}:${location.port}${path}`;
  document.body.append(frame);
}
</script></body></html>";

const FRAMES_CHILD: &str = r#"<!doctype html><html><body><p id="text"></p><script>
document.getElementById('text').textContent = `${location.hostname} frame text`;
addEventListener('load', () => parent.postMessage('ready', '*'));
</script></body></html>"#;

const FRAMES_HUNG: &str = r"<!doctype html><html><body><p>Hung frame text</p><script>
addEventListener('message', (event) => {
  if (event.data === 'spin' && event.source === parent) for (;;) {}
});
addEventListener('load', () => parent.postMessage('ready', '*'));
</script></body></html>";

/// A capture that waits out a hung frame must still return inside this (design section 7, F6).
const HUNG_CAPTURE_BUDGET: Duration = Duration::from_millis(2_500);

/// Every CDP frame id the browser knows, with its URL: the tab's own frame tree, plus out-of-
/// process frames, whose target id is their frame id.
async fn cdp_frames(tab: &Tab<'_>) -> Result<(String, Vec<(String, String)>), Box<dyn Error>> {
    let tree = tab.send("Page.getFrameTree", json!({})).await?;
    let main = tree
        .pointer("/frameTree/frame/id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Page.getFrameTree gave no main frame: {tree}"))?
        .to_owned();
    let mut frames = Vec::new();
    let mut stack: Vec<&Value> = tree.get("frameTree").into_iter().collect();
    while let Some(entry) = stack.pop() {
        if let (Some(id), Some(url)) = (
            entry.pointer("/frame/id").and_then(Value::as_str),
            entry.pointer("/frame/url").and_then(Value::as_str),
        ) {
            frames.push((id.to_owned(), url.to_owned()));
        }
        if let Some(children) = entry.get("childFrames").and_then(Value::as_array) {
            stack.extend(children);
        }
    }
    let targets = tab
        .client
        .send("Target.getTargets", json!({}), None)
        .await?;
    for info in targets
        .get("targetInfos")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|info| info.get("type").and_then(Value::as_str) == Some("iframe"))
    {
        if let (Some(id), Some(url)) = (
            info.get("targetId").and_then(Value::as_str),
            info.get("url").and_then(Value::as_str),
        ) {
            frames.push((id.to_owned(), url.to_owned()));
        }
    }
    Ok((main, frames))
}

fn frame_token(content: &IframeDataContent) -> Option<&str> {
    match content {
        IframeDataContent::FrameData(frame) => {
            frame.browseros.as_ref()?.devtools_frame_token.as_deref()
        }
        IframeDataContent::RedactedFrameMetadata(_) => None,
    }
}

fn timed_out_frames(capture: &Capture) -> Vec<&RedactedFrameMetadata> {
    capture
        .iframes()
        .into_iter()
        .filter_map(|content| match content {
            IframeDataContent::RedactedFrameMetadata(meta)
                if meta.reason == Some(REASON_BROWSEROS_TIMEOUT) =>
            {
                Some(meta)
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT; run under bun run test:browser"]
async fn p6_frames_carry_devtools_tokens_and_a_hung_frame_times_out() -> TestResult {
    let pages = HashMap::from([
        ("/frames/top.html".to_owned(), Page::html(FRAMES_TOP)),
        ("/frames/child.html".to_owned(), Page::html(FRAMES_CHILD)),
        ("/frames/hung.html".to_owned(), Page::html(FRAMES_HUNG)),
    ]);
    check_patch(Patch::Frames, pages, "/frames/top.html", async |tab, capture| {
        let deadline_ms = capture
            .apc
            .browseros
            .as_ref()
            .and_then(|page| page.subframe_timeout_ms)
            .ok_or("contract version 2 reports its subframe deadline")?;
        assert!(
            deadline_ms <= 2_000,
            "the compiled subframe deadline is at most 2 s, got {deadline_ms} ms"
        );

        let (main_id, frames) = cdp_frames(tab).await?;
        assert_eq!(
            capture
                .apc
                .main_frame_data
                .as_ref()
                .and_then(|frame| frame.browseros.as_ref())
                .and_then(|facts| facts.devtools_frame_token.as_deref()),
            Some(main_id.as_str()),
            "the main frame's token is its CDP frame id"
        );
        let tokens: Vec<&str> = capture.iframes().into_iter().filter_map(frame_token).collect();
        assert_eq!(tokens.len(), 2, "both child frames carry a token: {tokens:?}");
        assert_ne!(tokens.first(), tokens.get(1), "frame tokens are distinct");
        for token in &tokens {
            assert!(
                frames.iter().any(|(id, _)| id == token),
                "token {token} is a CDP frame id, known frames {frames:?}"
            );
        }
        assert!(timed_out_frames(capture).is_empty(), "no frame timed out on the healthy page");

        let href = tab.eval("location.href").await?;
        let href = href.as_str().ok_or("location.href is a string")?;
        tab.navigate(format!("{href}?hang")).await?;
        tab.eval("document.getElementById('hung').contentWindow.postMessage('spin', '*')")
            .await?;
        time::sleep(Duration::from_millis(200)).await;
        let started = Instant::now();
        let hung = tab.capture().await?;
        let elapsed = started.elapsed();
        assert!(
            elapsed < HUNG_CAPTURE_BUDGET,
            "a capture with a hung frame returns inside {HUNG_CAPTURE_BUDGET:?}, took {elapsed:?}"
        );

        let (_, frames) = cdp_frames(tab).await?;
        let hung_id = frames
            .iter()
            .find(|(_, url)| url.contains("/frames/hung.html"))
            .map(|(id, _)| id.as_str())
            .ok_or_else(|| format!("no CDP frame for the hung page in {frames:?}"))?;
        let timed_out = timed_out_frames(&hung);
        assert_eq!(timed_out.len(), 1, "exactly the hung frame times out");
        assert_eq!(
            timed_out
                .first()
                .and_then(|meta| meta.browseros.as_ref())
                .and_then(|facts| facts.devtools_frame_token.as_deref()),
            Some(hung_id),
            "the timeout names the hung frame by its CDP frame id"
        );
        for text in ["127.0.0.1 frame text", "localhost frame text"] {
            hung.text_node(text)?;
        }
        Ok(())
    })
    .await
}

// P3: retention keeps role-bearing elements off screen, and each closed root gets one seed.

const RETENTION_PAGE: &str = r#"<!doctype html><html><head><title>Contract retention</title></head>
<body style="margin:0">
<div id="closed-host"></div><div id="open-host"></div>
<div style="height:5000px">Tall spacer</div>
<div role="status">Saved</div>
<progress value="3" max="10"></progress>
<table><tbody><tr><td>Cell</td></tr></tbody></table>
<span id="only-id">Id span</span>
<script>
const outer = document.getElementById('closed-host').attachShadow({ mode: 'closed' });
outer.innerHTML = '<p>Outer closed text</p><div class="inner-host"></div>';
const inner = outer.querySelector('.inner-host').attachShadow({ mode: 'closed' });
inner.innerHTML = '<div>Inner closed text</div>';
const open = document.getElementById('open-host').attachShadow({ mode: 'open' });
open.innerHTML = '<p>Open text</p>';
// Closed roots are reachable only through references kept at creation.
window.__roots = [outer, inner, open];
window.__fixtureReady = true;
</script></body></html>"#;

/// Elements below the fold, one per retention rule: a `role`, a kept tag, a kept tag with no APC
/// type of its own, and an `id` alone.
const RETAINED_BELOW_THE_FOLD: &[&str] = &["[role=status]", "progress", "tbody", "#only-id"];

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT; run under bun run test:browser"]
async fn p3_retention_keeps_role_bearing_nodes_and_seeds_each_closed_root() -> TestResult {
    let pages = HashMap::from([("/retention.html".to_owned(), Page::html(RETENTION_PAGE))]);
    check_patch(
        Patch::Retention,
        pages,
        "/retention.html",
        async |tab, capture| {
            for selector in RETAINED_BELOW_THE_FOLD {
                assert!(
                    tab.element_node(capture, selector).await?.is_some(),
                    "{selector} is a node without scrolling"
                );
            }

            let seeds: Vec<i32> = capture
                .nodes()
                .into_iter()
                .filter(|node| facts(node).and_then(|facts| facts.closed_root_seed) == Some(true))
                .filter_map(dom_node_id)
                .collect();
            let mut roots = Vec::new();
            for seed in seeds {
                roots.push(
                    tab.call_on_node(
                        seed,
                        "function() { return window.__roots.indexOf(this.getRootNode()); }",
                    )
                    .await?,
                );
            }
            roots.sort_by_key(Value::as_i64);
            assert_eq!(
                roots,
                vec![json!(0), json!(1)],
                "one seed in each closed root, resolving into it, and none in the open root"
            );
            report(&format!(
                "P3 retention page (O1): {} nodes, {} reply bytes",
                capture.nodes().len(),
                capture.raw.len()
            ));
            Ok(())
        },
    )
    .await
}

// P4: tree facts.

const GIF: &str = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

fn tree_facts_page() -> String {
    format!(
        r#"<!doctype html><html><head><title>Contract tree facts</title></head><body>
<div style="display:table"><div style="display:table-row"><div id="css-cell" style="display:table-cell">Css cell</div></div></div>
<table><tr><th id="real-th" scope="row">Row head</th><td>Value</td></tr></table>
<a id="empty-href" href="">Empty link</a>
<img id="empty-alt" alt="" width="10" height="10" src="{GIF}">
<img id="no-alt" width="10" height="10" src="{GIF}">
<div aria-hidden="true" style="pointer-events:none">Hidden text</div>
<div id="vis-hidden" style="visibility:hidden">Hidden parent <span style="visibility:visible">Visible child</span></div>
<div>Add</div><div>to cart</div>
<p>Add<b>ed</b></p>
<button style="text-transform:uppercase">Buy</button>
<input id="ua-date" type="date"> <input id="ua-file" type="file">
<video id="ua-video" controls width="200" height="100"></video>
<details><summary>More</summary><p>Details content</p></details>
<script>window.__fixtureReady = true;</script></body></html>"#
    )
}

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT; run under bun run test:browser"]
async fn p4_tree_facts_carry_tags_raw_attributes_hiddenness_and_text() -> TestResult {
    let pages = HashMap::from([("/tree.html".to_owned(), Page::html(&tree_facts_page()))]);
    check_patch(Patch::TreeFacts, pages, "/tree.html", async |tab, capture| {
        let css_cell = tab.require_node(capture, "#css-cell").await?;
        assert_eq!(
            css_cell.content_attributes.as_ref().and_then(|attrs| attrs.attribute_type),
            Some(i32::from(ContentAttributeType::TableCell)),
            "stock types the CSS cell by layout"
        );
        assert_eq!(facts_of(css_cell).tag, None, "a div-built table cell has no tag: div");
        let th = tab.facts_at(capture, "#real-th").await?;
        assert_eq!((th.tag.as_deref(), th.scope.as_deref()), (Some("th"), Some("row")));
        assert_eq!(tab.facts_at(capture, "#empty-href").await?.href.as_deref(), Some(""), "href=\"\" kept");
        assert_eq!(tab.facts_at(capture, "#empty-alt").await?.alt.as_deref(), Some(""), "alt=\"\" kept");
        assert_eq!(tab.facts_at(capture, "#no-alt").await?.alt, None, "no alt differs from empty alt");

        assert_eq!(
            facts_of(capture.text_node("Hidden text")?).aria_hidden,
            Some(true),
            "aria-hidden reaches text under pointer-events: none"
        );
        let hidden_parent = tab.facts_at(capture, "#vis-hidden").await?;
        assert_eq!(
            (hidden_parent.visibility_hidden, hidden_parent.aria_hidden),
            (Some(true), None),
            "a visibility-hidden parent kept for its child is visibility_hidden, not aria_hidden"
        );

        assert_eq!(facts_of(capture.text_node("to cart")?).break_before, Some(true), "block boundary");
        assert_eq!(facts_of(capture.text_node("ed")?).break_before, None, "no break inside Add<b>ed</b>");
        assert_eq!(facts_of(capture.text_node("BUY")?).dom_text.as_deref(), Some("Buy"));

        for selector in ["#ua-date", "#ua-file", "#ua-video"] {
            let control = tab.require_node(capture, selector).await?;
            let inner = descendants(control);
            if selector == "#ua-date" {
                assert!(!inner.is_empty(), "the date field's browser-made parts are nodes");
            }
            for node in inner {
                let inner_facts = facts_of(node);
                assert_eq!(inner_facts.ua_shadow, Some(true), "{selector}: inside is ua_shadow");
                assert_eq!(
                    (inner_facts.tag, inner_facts.role),
                    (None, None),
                    "{selector}: a ua_shadow node carries no tag or role"
                );
            }
        }
        assert_eq!(
            facts_of(capture.text_node("Details content")?).ua_shadow,
            None,
            "the page's own content under a closed details is not ua_shadow"
        );
        Ok(())
    })
    .await
}

// P5: BrowserOS's own UI stays out of the tree.

const TAKEOVER_PAGE: &str = r"<!doctype html><html><head><title>Contract takeover</title></head>
<body><h1>Takeover page heading</h1><script>window.__fixtureReady = true;</script></body></html>";

/// How long BrowserOS gets to mount its takeover bar, as `expect_live.rs` allows.
const TAKEOVER_MOUNT_TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT; run under bun run test:browser"]
async fn p5_browseros_takeover_bar_stays_out_of_the_tree() -> TestResult {
    let pages = HashMap::from([("/takeover.html".to_owned(), Page::html(TAKEOVER_PAGE))]);
    check_patch(Patch::BrowserOsUi, pages, "/takeover.html", async |tab, _| {
        let mounted = "!!document.querySelector('browseros-neo-takeover')?.shadowRoot?.querySelector('body')";
        let deadline = Instant::now() + TAKEOVER_MOUNT_TIMEOUT;
        while tab.eval(mounted).await? != Value::Bool(true) {
            if Instant::now() >= deadline {
                report(
                    "skipped BrowserOsUi: no takeover bar mounted; launch the browser through \
                     crates/neo-sdk/neo-playwright/tests/support/neo-with-takeover.sh",
                );
                return Ok(());
            }
            time::sleep(Duration::from_millis(100)).await;
        }
        // An active takeover shows a button; give the bar one so there is something to leak.
        tab.eval(
            "document.querySelector('browseros-neo-takeover').shadowRoot.querySelector('body')\
             .append(Object.assign(document.createElement('button'), { textContent: 'Hand back control' }))",
        )
        .await?;
        let capture = tab.capture().await?;
        assert_eq!(capture.count_hits("Hand back control"), 0, "no text of the bar is on the wire");
        assert!(
            tab.element_node(&capture, "browseros-neo-takeover").await?.is_none(),
            "the bar's host is not a node"
        );
        capture.text_node("Takeover page heading")?;
        Ok(())
    })
    .await
}

// P7: names and descriptions.

const NAMES_PAGE: &str = r#"<!doctype html><html><head><title>Contract names</title></head><body>
<span id="first">First</span><span id="second" style="display:none">Second</span>
<button id="named" aria-labelledby="first second">X</button>
<div id="chain" style="display:none">Wrapper<textarea autocomplete="new-password">SecretCanary-chain</textarea></div>
<button id="chained" aria-labelledby="chain">Y</button>
<button id="described" title="Tip" aria-description="More detail">Z</button>
<script>window.__fixtureReady = true;</script></body></html>"#;

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT; run under bun run test:browser"]
async fn p7_names_carry_relation_refs_title_and_description() -> TestResult {
    let pages = HashMap::from([("/names.html".to_owned(), Page::html(NAMES_PAGE))]);
    check_patch(Patch::Names, pages, "/names.html", async |tab, capture| {
        let first_id = tab
            .backend_node_id("document.getElementById('first')")
            .await?;
        let named = tab.facts_at(capture, "#named").await?;
        let refs: Vec<(Option<i32>, Option<&str>)> = named
            .labelledby
            .iter()
            .map(|target: &BrowserOsRef| (target.dom_node_id, target.unrendered_text.as_deref()))
            .collect();
        assert_eq!(refs.len(), 2, "two refs, in attribute order: {refs:?}");
        assert_eq!(
            refs.first(),
            Some(&(Some(first_id), None)),
            "the rendered target by node id"
        );
        assert_eq!(
            refs.get(1).and_then(|target| target.1),
            Some("Second"),
            "the display: none target by its text"
        );

        let chained = tab.facts_at(capture, "#chained").await?;
        assert_eq!(
            chained
                .labelledby
                .first()
                .and_then(|target| target.unrendered_text.as_deref())
                .map(str::trim),
            Some("Wrapper"),
            "an unrendered target's text leaves its text controls out"
        );
        assert_eq!(
            capture.count_hits(SECRET_CANARY),
            0,
            "the private textarea adds no text"
        );

        let described = tab.facts_at(capture, "#described").await?;
        assert_eq!(
            (
                described.title.as_deref(),
                described.aria_description.as_deref()
            ),
            (Some("Tip"), Some("More detail")),
            "title and aria-description arrive apart"
        );
        Ok(())
    })
    .await
}

// P8: states.

const STATES_PAGE: &str = r#"<!doctype html><html><head><title>Contract states</title></head><body>
<input type="checkbox" id="indeterminate">
<div role="checkbox" id="mixed" aria-checked="mixed" tabindex="0">Mixed</div>
<button id="aria-disabled" aria-disabled="true" style="pointer-events:none">Aria disabled</button>
<button id="collapsed" aria-expanded="false">Collapsed</button>
<button id="plain">Plain</button>
<select id="list-box" size="4"><option>One</option><option id="selected-option" selected>Two</option><option id="disabled-option" disabled>Three</option></select>
<select id="collapsed-select"><option>A</option><option disabled>B</option></select>
<button id="outside">Outside</button>
<dialog id="dialog"><button id="inside">Inside</button></dialog>
<div style="height:5000px">Tall spacer</div>
<button id="below" disabled>Below the fold</button>
<script>
document.getElementById('indeterminate').indeterminate = true;
window.__fixtureReady = true;
</script></body></html>"#;

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT; run under bun run test:browser"]
async fn p8_states_carry_tri_states_disabled_inert_and_pointer_events() -> TestResult {
    let pages = HashMap::from([("/states.html".to_owned(), Page::html(STATES_PAGE))]);
    check_patch(
        Patch::States,
        pages,
        "/states.html",
        async |tab, capture| {
            let mixed = Some(i32::from(BrowserOsTriState::Mixed));
            assert_eq!(
                tab.facts_at(capture, "#indeterminate").await?.checked,
                mixed,
                "indeterminate"
            );
            assert_eq!(
                tab.facts_at(capture, "#mixed").await?.checked,
                mixed,
                "aria-checked=mixed"
            );
            assert_eq!(
                tab.facts_at(capture, "#below").await?.disabled,
                Some(true),
                "a disabled button below the fold"
            );
            let aria_disabled = tab.facts_at(capture, "#aria-disabled").await?;
            assert_eq!(
                (
                    aria_disabled.aria_disabled,
                    aria_disabled.pointer_events_none
                ),
                (Some(true), Some(true))
            );
            assert_eq!(
                tab.facts_at(capture, "#collapsed").await?.expanded,
                Some(i32::from(BrowserOsTriState::False)),
                "aria-expanded=false"
            );
            assert_eq!(
                tab.facts_at(capture, "#plain").await?.expanded,
                None,
                "no aria-expanded: unset"
            );
            assert_eq!(
                tab.facts_at(capture, "#selected-option").await?.selected,
                Some(true)
            );
            assert_eq!(
                tab.facts_at(capture, "#disabled-option").await?.disabled,
                Some(true)
            );
            let collapsed_select = tab
                .element_node(capture, "#collapsed-select")
                .await?
                .and_then(|node| node.content_attributes.as_ref()?.content_data.as_ref())
                .ok_or("the collapsed select has no node")?;
            let ContentData::FormControl(control) = collapsed_select else {
                return Err("the collapsed select is not a form control".into());
            };
            assert_eq!(
                control
                    .select_options
                    .get(1)
                    .and_then(|option| option.is_disabled),
                Some(true),
                "a collapsed select's disabled option carries is_disabled"
            );

            tab.eval("document.getElementById('dialog').showModal()")
                .await?;
            let modal = tab.capture().await?;
            let outside = tab.facts_at(&modal, "#outside").await?;
            assert_eq!(
                (outside.inert, outside.pointer_events_none),
                (Some(true), None),
                "outside a modal dialog: inert, though its own pointer-events are not none"
            );
            let inside = tab.facts_at(&modal, "#inside").await?;
            assert_eq!(
                (inside.inert, inside.pointer_events_none),
                (None, None),
                "inside the dialog"
            );
            Ok(())
        },
    )
    .await
}

// P11: `display: contents` elements get a node.

const DISPLAY_CONTENTS_PAGE: &str = r#"<!doctype html><html><head><title>Contract display contents</title></head><body>
<div id="list" role="list" style="display:contents"><div role="listitem" id="one">One</div><div role="listitem" id="two">Two</div></div>
<div id="hidden-list" role="list" aria-hidden="true" style="display:contents"><div role="listitem" id="three">Three</div><div role="listitem" id="four">Four</div></div>
<div id="empty-list" role="list" style="display:contents"></div>
<script>window.__fixtureReady = true;</script></body></html>"#;

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT; run under bun run test:browser"]
async fn p11_display_contents_elements_get_a_node_without_geometry() -> TestResult {
    let pages = HashMap::from([(
        "/contents.html".to_owned(),
        Page::html(DISPLAY_CONTENTS_PAGE),
    )]);
    check_patch(
        Patch::DisplayContents,
        pages,
        "/contents.html",
        async |tab, capture| {
            let list = tab
                .element_node(capture, "#list")
                .await?
                .ok_or("the display: contents list has no node")?;
            let list_facts = facts(list).cloned().unwrap_or_default();
            assert_eq!(
                (list_facts.display_contents, list_facts.role.as_deref()),
                (Some(true), Some("list"))
            );
            assert_eq!(
                list.content_attributes
                    .as_ref()
                    .and_then(|attrs| attrs.geometry.as_ref()),
                None,
                "a display: contents node has no geometry"
            );
            let under: Vec<Option<i32>> = descendants(list).into_iter().map(dom_node_id).collect();
            for item in ["one", "two"] {
                let id = tab
                    .backend_node_id(&format!("document.getElementById({item:?})"))
                    .await?;
                assert!(
                    under.contains(&Some(id)),
                    "item {item} hangs under the list's node"
                );
            }
            for item in ["#three", "#four"] {
                let node = tab
                    .element_node(capture, item)
                    .await?
                    .ok_or_else(|| format!("{item} has no node"))?;
                assert_eq!(
                    facts(node).and_then(|facts| facts.aria_hidden),
                    Some(true),
                    "{item}: aria-hidden on a display: contents parent reaches its items"
                );
            }
            assert!(
                tab.element_node(capture, "#empty-list").await?.is_none(),
                "an empty display: contents element has no node"
            );
            Ok(())
        },
    )
    .await
}
