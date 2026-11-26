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
//!   bun run test:browser -- cargo test -p neo-snapshot-apc --test contract_live -- --ignored --nocapture
//! ```
//!
//! `--nocapture` shows each test's report: why it skipped, and its page's node count and reply
//! bytes, which a Chromium PR compares before and after its change.
//!
//! Do not run it under `nice`: a debug component build's cold start nearly fills `test:browser`'s
//! 30 s CDP deadline. The P5 test also needs BrowserOS's takeover bar, which only a browser
//! launched through `crates/neo-sdk/neo-playwright/tests/support/neo-with-takeover.sh` mounts
//! (`NEO_TAKEOVER_BROWSER` picks the build); without it that test skips and says so.

#[path = "support/fixture_server.rs"]
mod fixture_server;

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::Write as _;
use std::time::Duration;
use std::{env, ptr};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use browseros_cdp::page::{self, EnableParams, GetAnnotatedPageContentParams, NavigateParams};
use browseros_cdp::target::{self, AttachToTargetParams};
use browseros_cdp::{CdpClient, ConnectOptions, SessionId};
use fixture_server::{FixtureServer, Page};
use neo_snapshot_apc::proto::{
    BoundingRect, BrowserOsContentAttributes, BrowserOsRef, BrowserOsTriState, ClickabilityReason,
    ContentAttributeType, ContentAttributes, ContentData, ContentNode, FormControlData, FrameData,
    IframeDataContent, REASON_BROWSEROS_TIMEOUT, RedactedFrameMetadata, RedactionDecision,
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
    /// P12: every document's `<html>` element has a node, below the fold or inert included.
    HtmlRetention,
    /// P13: an SVG element's first direct `<title>` child arrives as `svg_title`.
    SvgTitle,
    /// P14: non-inline `::before` and `::after` boxes raise `break_before`.
    GeneratedBoxBreaks,
    /// P15: skipped collapsible-whitespace text boxes between inline boxes raise `break_before`.
    InlineWhitespace,
    /// P18: the CDP capture runs on the critical path, so it reads the page as it is when the
    /// request arrives, not after the next rendered frame.
    CriticalPath,
    /// P19: a frameset's `<frame>` arrives as an iframe node holding its document, and every
    /// `<frameset>` is a node.
    FramesetFrames,
    /// P20: the top element of each `visibility: hidden` region with visible content is a node.
    HiddenRegionRetention,
    /// P21: nodes of a cross-site frame out of view keep their interaction info.
    CrossSiteInteraction,
    /// P22: an element whose boxes were borrowed from its content around an empty own box
    /// carries `own_box_empty`.
    OwnBoxEmpty,
}

impl Patch {
    const fn contract_version(self) -> u32 {
        match self {
            Self::Skeleton | Self::Privacy => 1,
            Self::Frames => 2,
            Self::Retention | Self::TreeFacts | Self::BrowserOsUi => 3,
            Self::Names | Self::States => 4,
            Self::DisplayContents => 5,
            Self::HtmlRetention | Self::SvgTitle | Self::GeneratedBoxBreaks => 6,
            Self::InlineWhitespace => 7,
            Self::CriticalPath
            | Self::FramesetFrames
            | Self::HiddenRegionRetention
            | Self::CrossSiteInteraction
            | Self::OwnBoxEmpty => 8,
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

/// Where a test's tab opens.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Placement {
    /// A tab in the shared window. Tests run concurrently, so a newer tab may hide it.
    SharedWindow,
    /// A window of its own, so the tab stays visible while other tests open tabs. A test that
    /// waits on `requestAnimationFrame` needs it: a hidden tab stops producing frames, and stock
    /// code captures a hidden tab on the critical path at every version.
    OwnWindow,
}

/// An open tab on one fixture page, with a flattened CDP session attached.
struct Tab<'a> {
    client: &'a CdpClient,
    target_id: String,
    session: SessionId,
}

impl<'a> Tab<'a> {
    async fn open(
        client: &'a CdpClient,
        url: String,
        placement: Placement,
    ) -> Result<Self, Box<dyn Error>> {
        let created = client
            .send(
                "Target.createTarget",
                json!({
                    "url": "about:blank",
                    "newWindow": placement == Placement::OwnWindow,
                }),
                None,
            )
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
    check_patch_in(patch, pages, path, Placement::SharedWindow, check).await
}

/// [`check_patch`] with the tab in a window of its own, for checks that wait on the page's frames.
async fn check_patch_visible(
    patch: Patch,
    pages: HashMap<String, Page>,
    path: &str,
    check: impl AsyncFnOnce(&Tab<'_>, &Capture) -> TestResult,
) -> TestResult {
    check_patch_in(patch, pages, path, Placement::OwnWindow, check).await
}

async fn check_patch_in(
    patch: Patch,
    pages: HashMap<String, Page>,
    path: &str,
    placement: Placement,
    check: impl AsyncFnOnce(&Tab<'_>, &Capture) -> TestResult,
) -> TestResult {
    let server = FixtureServer::start(pages).await?;
    let client = connect_cdp().await?;
    let result = async {
        let tab = Tab::open(&client, server.url("127.0.0.1", path), placement).await?;
        let checked = async {
            let capture = tab.capture().await?;
            // Printed at every version, so a run before and after a patch compares page size
            // (O1 asks PR 3 for exactly that).
            report(&format!(
                "{patch:?} page: {} nodes, {} reply bytes",
                capture.nodes().len(),
                capture.raw.len()
            ));
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
// Two sites, 127.0.0.1 and localhost, nest three levels deep on one port, crossing processes at
// every level. With `?hang` a child on a third site, [::1], joins: alone on its site it has a
// process of its own, so its spin stalls no other frame.
const hang = location.search.includes('hang');
const wanted = hang ? 6 : 5;
let ready = 0;
addEventListener('message', (event) => {
  if (event.data === 'ready') ready += 1;
  window.__fixtureReady = ready >= wanted;
});
const child = (id, host, path) => {
  const frame = document.createElement('iframe');
  frame.id = id;
  frame.src = `http://${host}:${location.port}${path}`;
  document.body.append(frame);
};
child('a1', '127.0.0.1', `/frames/child.html?name=a1&nest=${encodeURIComponent('localhost|a2,127.0.0.1|a3')}`);
child('b1', 'localhost', `/frames/child.html?name=b1&nest=${encodeURIComponent('127.0.0.1|b2')}`);
if (hang) child('hung', '[::1]', '/frames/hung.html');
</script></body></html>";

/// A frame named by `name`, which embeds the next `host|name` pair of `nest` and passes the rest
/// down.
const FRAMES_CHILD: &str = r#"<!doctype html><html><body><p id="text"></p><script>
const params = new URLSearchParams(location.search);
document.getElementById('text').textContent = `${params.get('name')} frame text`;
const [next, ...rest] = (params.get('nest') || '').split(',').filter(Boolean);
if (next) {
  const [host, name] = next.split('|');
  const frame = document.createElement('iframe');
  frame.src = `http://${host}:${location.port}/frames/child.html?name=${name}&nest=${encodeURIComponent(rest.join(','))}`;
  document.body.append(frame);
}
addEventListener('load', () => top.postMessage('ready', '*'));
</script></body></html>"#;

const FRAMES_HUNG: &str = r"<!doctype html><html><body><p>Hung frame text</p><script>
addEventListener('message', (event) => {
  if (event.data === 'spin' && event.source === parent) for (;;) {}
});
addEventListener('load', () => top.postMessage('ready', '*'));
</script></body></html>";

/// The frames below the top page that stay healthy, by the name each prints.
const HEALTHY_FRAMES: &[&str] = &["a1", "a2", "a3", "b1", "b2"];

/// A capture that waits out a hung frame must still return inside this (design section 7, F6).
const HUNG_CAPTURE_BUDGET: Duration = Duration::from_millis(2_500);

/// One frame as CDP names it.
#[derive(Debug)]
struct CdpFrame {
    id: String,
    url: String,
}

/// The frames CDP knows for one tab: its own frame tree, plus out-of-process frames, whose
/// target id is their frame id.
#[derive(Debug)]
struct CdpFrames {
    main_id: String,
    frames: Vec<CdpFrame>,
}

impl CdpFrames {
    async fn read(tab: &Tab<'_>) -> Result<Self, Box<dyn Error>> {
        let tree = tab.send("Page.getFrameTree", json!({})).await?;
        let main_id = tree
            .pointer("/frameTree/frame/id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Page.getFrameTree gave no main frame: {tree}"))?
            .to_owned();
        let mut entries: Vec<&Value> = Vec::new();
        let mut stack: Vec<&Value> = tree.get("frameTree").into_iter().collect();
        while let Some(entry) = stack.pop() {
            if let Some(frame) = entry.get("frame") {
                entries.push(frame);
            }
            if let Some(children) = entry.get("childFrames").and_then(Value::as_array) {
                stack.extend(children);
            }
        }
        let targets = tab
            .client
            .send("Target.getTargets", json!({}), None)
            .await?;
        let iframe_targets = targets
            .get("targetInfos")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|info| info.get("type").and_then(Value::as_str) == Some("iframe"));
        let frames = entries
            .into_iter()
            .filter_map(|frame| Some((frame.get("id")?, frame.get("url")?)))
            .chain(
                iframe_targets.filter_map(|info| Some((info.get("targetId")?, info.get("url")?))),
            )
            .filter_map(|(id, url)| {
                Some(CdpFrame {
                    id: id.as_str()?.to_owned(),
                    url: url.as_str()?.to_owned(),
                })
            })
            .collect();
        Ok(Self { main_id, frames })
    }

    fn knows(&self, id: &str) -> bool {
        self.frames.iter().any(|frame| frame.id == id)
    }

    fn id_of(&self, url_part: &str) -> Option<&str> {
        self.frames
            .iter()
            .find(|frame| frame.url.contains(url_part))
            .map(|frame| frame.id.as_str())
    }
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

/// Every healthy frame, at every depth, is whole and names a distinct CDP frame.
fn assert_healthy_frames(capture: &Capture, cdp: &CdpFrames, when: &str) -> TestResult {
    assert_eq!(
        capture
            .apc
            .main_frame_data
            .as_ref()
            .and_then(|frame| frame.browseros.as_ref())
            .and_then(|facts| facts.devtools_frame_token.as_deref()),
        Some(cdp.main_id.as_str()),
        "{when}: the main frame's token is its CDP frame id"
    );
    let tokens: HashSet<&str> = capture
        .iframes()
        .into_iter()
        .filter_map(frame_token)
        .collect();
    assert_eq!(
        tokens.len(),
        HEALTHY_FRAMES.len(),
        "{when}: every healthy frame carries its own token: {tokens:?}"
    );
    for token in &tokens {
        assert!(
            cdp.knows(token),
            "{when}: token {token} is a CDP frame id, known {cdp:?}"
        );
    }
    for name in HEALTHY_FRAMES {
        capture.text_node(&format!("{name} frame text"))?;
    }
    Ok(())
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
        assert_healthy_frames(capture, &CdpFrames::read(tab).await?, "healthy page")?;
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

        let cdp = CdpFrames::read(tab).await?;
        assert_healthy_frames(&hung, &cdp, "hung page")?;
        let hung_id = cdp
            .id_of("/frames/hung.html")
            .ok_or_else(|| format!("no CDP frame for the hung page in {cdp:?}"))?;
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
                        "function() { return this.nodeType === Node.ELEMENT_NODE \
                         ? window.__roots.indexOf(this.getRootNode()) : 'not an element'; }",
                    )
                    .await?,
                );
            }
            roots.sort_by_key(Value::as_i64);
            assert_eq!(
                roots,
                vec![json!(0), json!(1)],
                "one seed in each closed root, an element resolving into it, none in the open root"
            );
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
        // `test:browser` disables BrowserOS's extensions, which mount the bar; only the takeover
        // wrapper keeps them. Without it the bar cannot appear, so the test skips. With it a
        // missing bar is a failed prerequisite, not a pass.
        let wrapped = env::var("NEO_E2E_BROWSER")
            .is_ok_and(|browser| browser.ends_with("neo-with-takeover.sh"));
        if !wrapped {
            report(
                "skipped BrowserOsUi: it needs the takeover bar; set NEO_E2E_BROWSER to \
                 crates/neo-sdk/neo-playwright/tests/support/neo-with-takeover.sh",
            );
            return Ok(());
        }
        let mounted = "!!document.querySelector('browseros-neo-takeover')?.shadowRoot?.querySelector('body')";
        let deadline = Instant::now() + TAKEOVER_MOUNT_TIMEOUT;
        while tab.eval(mounted).await? != Value::Bool(true) {
            assert!(
                Instant::now() < deadline,
                "the takeover wrapper's browser mounted no takeover bar within \
                 {TAKEOVER_MOUNT_TIMEOUT:?}"
            );
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
            // The plain list, then the same shape under aria-hidden, which must also reach both
            // the wrapper and its items.
            for (wrapper, items, hidden) in [
                ("#list", ["#one", "#two"], None),
                ("#hidden-list", ["#three", "#four"], Some(true)),
            ] {
                let id = tab
                    .backend_node_id(&format!("document.querySelector({wrapper:?})"))
                    .await?;
                let nodes: Vec<&ContentNode> = capture
                    .nodes()
                    .into_iter()
                    .filter(|node| dom_node_id(node) == Some(id))
                    .collect();
                assert_eq!(nodes.len(), 1, "{wrapper}: exactly one node");
                let list = nodes.first().ok_or("checked above")?;
                let list_facts = facts_of(list);
                assert_eq!(
                    (
                        list_facts.display_contents,
                        list_facts.role.as_deref(),
                        list_facts.aria_hidden
                    ),
                    (Some(true), Some("list"), hidden),
                    "{wrapper}: its facts"
                );
                assert_eq!(
                    list.content_attributes
                        .as_ref()
                        .and_then(|attrs| attrs.geometry.as_ref()),
                    None,
                    "{wrapper}: a display: contents node has no geometry"
                );
                let under: Vec<Option<i32>> =
                    descendants(list).into_iter().map(dom_node_id).collect();
                for item in items {
                    let item_node = tab.require_node(capture, item).await?;
                    assert!(
                        under.contains(&dom_node_id(item_node)),
                        "{item} hangs under {wrapper}"
                    );
                    assert_eq!(
                        facts_of(item_node).aria_hidden,
                        hidden,
                        "{item}: aria_hidden"
                    );
                }
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

// P12: every document's `<html>` element has a node, even below the fold or when inert.

const HTML_BELOW_FOLD_MAIN: &str = r#"<!doctype html><html><head><title>P12 main</title></head>
<body style="margin:0">
<div style="height:3000px">Tall spacer</div>
<iframe id="below-fold" width="200" height="100"></iframe>
<script>
const frame = document.getElementById('below-fold');
frame.addEventListener('load', () => { window.__fixtureReady = true; });
frame.src = '/html-subframe.html';
</script></body></html>"#;

const HTML_BELOW_FOLD_SUBFRAME: &str = r"<!doctype html><html><head><title>P12 below-fold iframe</title></head>
<body><p>Subframe body</p></body></html>";

const HTML_MODAL_PAGE: &str = r#"<!doctype html><html><head><title>P12 modal</title></head>
<body><p>Behind modal</p><dialog id="dialog"><p>Modal</p></dialog>
<script>
document.getElementById('dialog').showModal();
window.__fixtureReady = true;
</script></body></html>"#;

/// The single direct `<html>` child of a document root node, paired with its DOM node id.
struct HtmlChild<'n> {
    node: &'n ContentNode,
    dom_node_id: i32,
}

/// Returns the single direct `<html>` child of a document root node and its DOM node id.
fn require_single_html_child<'n>(
    doc_root: &'n ContentNode,
    label: &str,
) -> Result<HtmlChild<'n>, Box<dyn Error>> {
    let html_children: Vec<&ContentNode> = doc_root
        .children_nodes
        .iter()
        .filter(|child| facts_of(child).tag.as_deref() == Some("html"))
        .collect();
    assert_eq!(
        html_children.len(),
        1,
        "{label}: document root must have exactly one direct html child, found {}",
        html_children.len()
    );
    let node = html_children.first().copied().ok_or("checked above")?;
    let dom_node_id =
        dom_node_id(node).ok_or_else(|| format!("{label}: html node has no dom_node_id"))?;
    Ok(HtmlChild { node, dom_node_id })
}

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT; run under bun run test:browser"]
async fn p12_every_document_has_an_html_node() -> TestResult {
    let pages = HashMap::from([
        (
            "/html-below-fold.html".to_owned(),
            Page::html(HTML_BELOW_FOLD_MAIN),
        ),
        (
            "/html-subframe.html".to_owned(),
            Page::html(HTML_BELOW_FOLD_SUBFRAME),
        ),
        ("/html-modal.html".to_owned(), Page::html(HTML_MODAL_PAGE)),
    ]);
    check_patch(
        Patch::HtmlRetention,
        pages,
        "/html-below-fold.html",
        async |tab, capture| {
            let main_root = capture
                .apc
                .root_node
                .as_ref()
                .ok_or("below-fold capture has no root_node")?;
            let main_html = require_single_html_child(main_root, "below-fold main")?;
            assert_eq!(
                tab.call_on_node(
                    main_html.dom_node_id,
                    "function() { return this === document.documentElement && document.title === 'P12 main'; }",
                )
                .await?,
                json!(true),
                "main html node id {} must resolve to main document.documentElement",
                main_html.dom_node_id
            );

            let iframe_roots: Vec<&ContentNode> = capture
                .nodes()
                .into_iter()
                .filter(|node| {
                    matches!(
                        node.content_attributes
                            .as_ref()
                            .and_then(|attrs| attrs.content_data.as_ref()),
                        Some(ContentData::Iframe(_))
                    )
                })
                .flat_map(|iframe| iframe.children_nodes.iter())
                .collect();
            assert_eq!(
                iframe_roots.len(),
                1,
                "below-fold page must carry one subframe root, found {}",
                iframe_roots.len()
            );
            let sub_root = iframe_roots
                .first()
                .copied()
                .ok_or("checked above")?;
            let sub_html = require_single_html_child(sub_root, "below-fold subframe")?;
            assert_eq!(
                tab.call_on_node(
                    sub_html.dom_node_id,
                    "function() { return this === this.ownerDocument.documentElement && this.ownerDocument.title === 'P12 below-fold iframe'; }",
                )
                .await?,
                json!(true),
                "subframe html node id {} must resolve to the subframe's documentElement",
                sub_html.dom_node_id
            );

            let modal_url = tab
                .eval("new URL('/html-modal.html', location.href).href")
                .await?;
            let modal_url = modal_url
                .as_str()
                .ok_or("modal URL did not evaluate to a string")?;
            tab.navigate(modal_url.to_owned()).await?;
            let modal_capture = tab.capture().await?;
            let modal_root = modal_capture
                .apc
                .root_node
                .as_ref()
                .ok_or("modal capture has no root_node")?;
            let modal_html = require_single_html_child(modal_root, "modal main")?;
            assert_eq!(
                facts_of(modal_html.node).inert,
                Some(true),
                "modal dialog must make the page's html element inert"
            );
            assert_eq!(
                tab.call_on_node(
                    modal_html.dom_node_id,
                    "function() { return this === document.documentElement && document.title === 'P12 modal'; }",
                )
                .await?,
                json!(true),
                "modal html node id {} must resolve to modal document.documentElement",
                modal_html.dom_node_id
            );
            Ok(())
        },
    )
    .await
}

// P13: an inline SVG's first direct `<title>` child arrives as `svg_title` (field 34).

const SVG_TITLE_PAGE: &str = r#"<!doctype html><html><head><title>Contract svg title</title></head><body>
<button><svg id="s1" width="24" height="24"><title>Close</title><path d="M0 0 L24 24"/></svg></button>
<svg id="s2" width="24" height="24"><title></title><rect width="24" height="24"/></svg>
<svg id="s3" width="24" height="24" title="attr"><rect width="24" height="24"/></svg>
<svg id="s4" width="24" height="24"><title>Outer</title><title>Second</title><g id="g1"><title>Group</title><rect width="24" height="24"/></g><g id="g2"><rect width="24" height="24"/></g></svg>
<script>window.__fixtureReady = true;</script></body></html>"#;

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT; run under bun run test:browser"]
async fn p13_svg_titles_arrive_as_a_fact() -> TestResult {
    let pages = HashMap::from([("/svg-title.html".to_owned(), Page::html(SVG_TITLE_PAGE))]);
    check_patch(
        Patch::SvgTitle,
        pages,
        "/svg-title.html",
        async |tab, capture| {
            assert_eq!(
                tab.facts_at(capture, "#s1").await?.svg_title.as_deref(),
                Some("Close"),
                "svg inside a button carries its direct <title> child as svg_title"
            );
            assert_eq!(
                tab.facts_at(capture, "#s2").await?.svg_title.as_deref(),
                Some(""),
                "an empty <title> child keeps explicit presence so Rust does not fall back to title"
            );
            let s3 = tab.facts_at(capture, "#s3").await?;
            assert_eq!(
                (s3.svg_title.as_deref(), s3.title.as_deref()),
                (None, Some("attr")),
                "an svg with only a title attribute leaves svg_title unset"
            );
            assert_eq!(
                tab.facts_at(capture, "#g1").await?.svg_title.as_deref(),
                Some("Group"),
                "an inner svg element node carries its own direct <title> child"
            );
            assert_eq!(
                tab.facts_at(capture, "#s4").await?.svg_title.as_deref(),
                Some("Outer"),
                "only the first direct <title> child supplies svg_title"
            );
            assert_eq!(
                tab.facts_at(capture, "#g2").await?.svg_title,
                None,
                "an inner svg element without its own <title> child does not inherit its parent's"
            );
            Ok(())
        },
    )
    .await
}

// P14: non-inline `::before` and `::after` boxes raise `break_before`.

const GENERATED_BREAKS_PAGE: &str = r#"<!doctype html><html><head><title>Contract generated breaks</title><style>
#inline-after::after { content: "inline-more"; }
#block-after::after { content: "block-more"; display: block; }
#block-before::before { content: "block-pre"; display: block; }
#inline-before::before { content: "inline-pre"; }
</style></head><body>
<button id="inline-after">inline-base</button>
<button id="block-after">after-base</button>
<button id="block-before">before-base</button>
<button id="inline-before">inline-core</button>
<script>window.__fixtureReady = true;</script></body></html>"#;

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT; run under bun run test:browser"]
async fn p14_generated_boxes_break_text() -> TestResult {
    let pages = HashMap::from([(
        "/generated-breaks.html".to_owned(),
        Page::html(GENERATED_BREAKS_PAGE),
    )]);
    check_patch(
        Patch::GeneratedBoxBreaks,
        pages,
        "/generated-breaks.html",
        async |_, capture| {
            assert_eq!(
                facts_of(capture.text_node("inline-base")?).break_before,
                None,
                "first button text has no preceding break"
            );
            assert_eq!(
                facts_of(capture.text_node("inline-more")?).break_before,
                None,
                "inline ::after raises no break"
            );
            assert_eq!(
                facts_of(capture.text_node("block-more")?).break_before,
                Some(true),
                "display: block ::after raises break_before on its generated text"
            );
            assert_eq!(
                facts_of(capture.text_node("before-base")?).break_before,
                Some(true),
                "display: block ::before raises break_before on the following button text"
            );
            assert_eq!(
                facts_of(capture.text_node("inline-core")?).break_before,
                None,
                "inline ::before raises no break on the following button text"
            );
            Ok(())
        },
    )
    .await
}

// P15: skipped collapsible-whitespace text boxes between inline boxes raise `break_before`.

const INLINE_WHITESPACE_PAGE: &str = r##"<!doctype html><html><head><title>Contract inline whitespace</title></head><body>
<button id="b2"><span>Add</span><span>to cart</span></button>
<button id="b1"><span>Add</span> <span>to cart</span></button>
<button id="b3"><span>Add </span><span>to cart</span></button>
<button id="b4"><span>Add</span><wbr><span>to cart</span></button>
<a id="a1" href="#"><b>New</b>
  <i>message</i></a>
<p id="p1"><span>one</span><span> <b>two</b></span></p>
<script>window.__fixtureReady = true;</script></body></html>"##;

/// One text node under an element, paired with its raw `text_content`.
struct TextDescendant<'n> {
    node: &'n ContentNode,
    text: &'n str,
}

/// Every text descendant of `node`, in document order.
fn text_descendants(node: &ContentNode) -> Vec<TextDescendant<'_>> {
    let mut out = Vec::new();
    let mut stack: Vec<&ContentNode> = node.children_nodes.iter().rev().collect();
    while let Some(current) = stack.pop() {
        if let Some(ContentData::Text(info)) = current
            .content_attributes
            .as_ref()
            .and_then(|attrs| attrs.content_data.as_ref())
            && let Some(text) = info.text_content.as_deref()
        {
            out.push(TextDescendant {
                node: current,
                text,
            });
        }
        stack.extend(current.children_nodes.iter().rev());
    }
    out
}

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT; run under bun run test:browser"]
async fn p15_whitespace_between_inline_boxes_raises_a_break() -> TestResult {
    let pages = HashMap::from([(
        "/inline-whitespace.html".to_owned(),
        Page::html(INLINE_WHITESPACE_PAGE),
    )]);
    check_patch(
        Patch::InlineWhitespace,
        pages,
        "/inline-whitespace.html",
        async |tab, capture| {
            // b2 is first on the page and has no space between spans, so neither text carries
            // break_before.
            let b2 = text_descendants(tab.require_node(capture, "#b2").await?);
            assert_eq!(b2.len(), 2, "#b2 has two text nodes");
            let b2_first = b2.first().ok_or("checked above")?;
            let b2_second = b2.get(1).ok_or("checked above")?;
            assert_eq!((b2_first.text, b2_second.text), ("Add", "to cart"));
            assert_eq!(
                (
                    facts_of(b2_first.node).break_before,
                    facts_of(b2_second.node).break_before
                ),
                (None, None),
                "#b2: no whitespace between spans raises no break"
            );

            // b1 has a space between two spans: "to cart" carries break_before.
            let b1 = text_descendants(tab.require_node(capture, "#b1").await?);
            assert_eq!(b1.len(), 2, "#b1 has two text nodes");
            let b1_second = b1.get(1).ok_or("checked above")?;
            assert_eq!(b1_second.text, "to cart");
            assert_eq!(
                facts_of(b1_second.node).break_before,
                Some(true),
                "#b1: whitespace between two spans raises break_before on the second text"
            );

            // b3 keeps the trailing space inside the first span's text_content; no skipped
            // whitespace box sits between the spans, so "to cart" carries no break_before.
            let b3 = text_descendants(tab.require_node(capture, "#b3").await?);
            assert_eq!(b3.len(), 2, "#b3 has two text nodes");
            let b3_first = b3.first().ok_or("checked above")?;
            let b3_second = b3.get(1).ok_or("checked above")?;
            assert_eq!(
                b3_first.text, "Add ",
                "#b3 keeps the trailing space in text_content"
            );
            assert_eq!(b3_second.text, "to cart");
            assert_eq!(
                facts_of(b3_second.node).break_before,
                None,
                "#b3: trailing space inside the first span raises no extra break"
            );

            // b4 separates two spans with `<wbr>` (an empty LayoutText): no space, no break.
            let b4 = text_descendants(tab.require_node(capture, "#b4").await?);
            assert_eq!(b4.len(), 2, "#b4 has two text nodes");
            let b4_second = b4.get(1).ok_or("checked above")?;
            assert_eq!(b4_second.text, "to cart");
            assert_eq!(
                facts_of(b4_second.node).break_before,
                None,
                "#b4: <wbr> (empty text) raises no break"
            );

            // a1 has a newline and indent between <b> and <i>: "message" carries break_before.
            let a1 = text_descendants(tab.require_node(capture, "#a1").await?);
            assert_eq!(a1.len(), 2, "#a1 has two text nodes");
            let a1_second = a1.get(1).ok_or("checked above")?;
            assert_eq!(a1_second.text, "message");
            assert_eq!(
                facts_of(a1_second.node).break_before,
                Some(true),
                "#a1: newline and indent between inline elements raises break_before"
            );

            // p1 puts the whitespace as the first child of the second inline box.
            let p1 = text_descendants(tab.require_node(capture, "#p1").await?);
            assert_eq!(p1.len(), 2, "#p1 has two text nodes");
            let p1_second = p1.get(1).ok_or("checked above")?;
            assert_eq!(p1_second.text, "two");
            assert_eq!(
                facts_of(p1_second.node).break_before,
                Some(true),
                "#p1: leading whitespace inside an inline box raises break_before"
            );

            // P15 adds a fact, not a node: no text node on the page is whitespace-only.
            let whitespace_only: Vec<&str> = capture
                .nodes()
                .into_iter()
                .filter_map(|node| {
                    match node.content_attributes.as_ref()?.content_data.as_ref()? {
                        ContentData::Text(info) => info.text_content.as_deref(),
                        _ => None,
                    }
                })
                .filter(|text| text.trim().is_empty())
                .collect();
            assert!(
                whitespace_only.is_empty(),
                "no text node on the page may be whitespace-only: {whitespace_only:?}"
            );
            Ok(())
        },
    )
    .await
}

// Helpers for the version 8 tests (P18 to P22).

/// Waits for two of the page's frames, so a change made before it has been painted, then reads
/// the page's visibility.
const TWO_FRAMES_THEN_VISIBILITY: &str = "new Promise((done) => requestAnimationFrame(() => \
    requestAnimationFrame(done))).then(() => document.visibilityState)";

impl Tab<'_> {
    /// Lets two of the page's frames pass. Fails unless the tab is still visible afterwards: the
    /// frame-clock checks of P18 and P21 mean nothing in a hidden tab.
    async fn settle_two_frames(&self) -> TestResult {
        let reply = self
            .send(
                "Runtime.evaluate",
                json!({
                    "expression": TWO_FRAMES_THEN_VISIBILITY,
                    "awaitPromise": true,
                    "returnByValue": true,
                }),
            )
            .await?;
        let state = reply
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null);
        if state == json!("visible") {
            Ok(())
        } else {
            Err(format!("the tab must stay visible for a frame-clock check, it is {state}").into())
        }
    }
}

/// Whether `node` embeds a child document: an `<iframe>`, or since P19 a frameset's `<frame>`.
fn is_frame_node(node: &ContentNode) -> bool {
    matches!(
        node.content_attributes
            .as_ref()
            .and_then(|attrs| attrs.content_data.as_ref()),
        Some(ContentData::Iframe(_))
    )
}

/// The frame data of the document a frame node holds; `None` for no document or a redaction.
fn frame_data(node: &ContentNode) -> Option<&FrameData> {
    match node.content_attributes.as_ref()?.content_data.as_ref()? {
        ContentData::Iframe(iframe) => match iframe.data.as_ref()? {
            IframeDataContent::FrameData(frame) => Some(frame.as_ref()),
            IframeDataContent::RedactedFrameMetadata(_) => None,
        },
        _ => None,
    }
}

/// The nodes of the document under `root` in document order, without entering the documents
/// its frame nodes hold. The frame nodes themselves are included.
fn document_nodes(root: &ContentNode) -> Vec<&ContentNode> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        out.push(node);
        if !is_frame_node(node) {
            stack.extend(node.children_nodes.iter().rev());
        }
    }
    out
}

fn tag_of(node: &ContentNode) -> Option<&str> {
    facts(node)?.tag.as_deref()
}

fn outer_box(node: &ContentNode) -> Option<&BoundingRect> {
    node.content_attributes
        .as_ref()?
        .geometry
        .as_ref()?
        .outer_bounding_box
        .as_ref()
}

fn visible_box(node: &ContentNode) -> Option<&BoundingRect> {
    node.content_attributes
        .as_ref()?
        .geometry
        .as_ref()?
        .visible_bounding_box
        .as_ref()
}

/// Whether a box is absent or has no area.
fn has_no_area(rect: Option<&BoundingRect>) -> bool {
    rect.is_none_or(|rect| rect.width.unwrap_or(0) <= 0 || rect.height.unwrap_or(0) <= 0)
}

/// Whether some text node under `node`, in any document, reads `text` once trimmed.
fn holds_text(node: &ContentNode, text: &str) -> bool {
    descendants(node).into_iter().any(|child| {
        matches!(
            child.content_attributes.as_ref().and_then(|attrs| attrs.content_data.as_ref()),
            Some(ContentData::Text(info))
                if info.text_content.as_deref().map(str::trim) == Some(text)
        )
    })
}

/// Whether `inner` is `outer` or one of its descendants, by identity.
fn is_under(inner: &ContentNode, outer: &ContentNode) -> bool {
    ptr::eq(inner, outer)
        || descendants(outer)
            .into_iter()
            .any(|node| ptr::eq(node, inner))
}

// P18: the CDP capture runs on the critical path (design 14.2).

const CRITICAL_PATH_PAGE: &str = r"<!doctype html><html><head><title>Contract critical path</title></head>
<body><button id='target'>idle</button><script>
// Sets the button's text now, and again in the next frame's requestAnimationFrame callbacks. A
// capture queued until that frame reads the second text; one run on arrival reads the first.
window.__flip = (round) => {
  const button = document.getElementById('target');
  button.textContent = `before ${round}`;
  requestAnimationFrame(() => { button.textContent = `after ${round}`; });
  return round;
};
window.__fixtureReady = true;
</script></body></html>";

/// Rounds of change, then capture.
const CRITICAL_PATH_ROUNDS: usize = 20;

/// Rounds that must read the page before its next frame. A round may still read the second text
/// when a frame happens to begin between the change and the capture's arrival.
const CRITICAL_PATH_MIN_BEFORE: usize = 15;

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT; run under bun run test:browser"]
async fn p18_capture_reads_the_page_as_it_is_now() -> TestResult {
    let pages = HashMap::from([(
        "/critical-path.html".to_owned(),
        Page::html(CRITICAL_PATH_PAGE),
    )]);
    check_patch_visible(
        Patch::CriticalPath,
        pages,
        "/critical-path.html",
        async |tab, _| {
            let mut before = 0;
            for round in 0..CRITICAL_PATH_ROUNDS {
                // Two frames first, so the previous round's callback cannot land in this one.
                tab.settle_two_frames().await?;
                tab.eval(&format!("window.__flip({round})")).await?;
                let capture = tab.capture().await?;
                if capture.text_node(&format!("before {round}")).is_ok() {
                    before += 1;
                } else {
                    capture.text_node(&format!("after {round}"))?;
                }
            }
            report(&format!(
                "CriticalPath: {before} of {CRITICAL_PATH_ROUNDS} captures read the page before its next frame"
            ));
            assert!(
                before >= CRITICAL_PATH_MIN_BEFORE,
                "a capture runs when it arrives, not after the next frame's callbacks: {before} of \
                 {CRITICAL_PATH_ROUNDS} rounds read the page before its next frame"
            );
            Ok(())
        },
    )
    .await
}

// P19: the frames of a frameset arrive as iframe nodes (design 15.2).

const FRAMESET_TOP: &str = r#"<!doctype html><html><head><title>Contract frameset</title><script>
// Frame two moves to a second site (localhost) once the frameset is parsed, so its document comes
// from another renderer. The page is ready when every frame has loaded.
let loaded = false;
let twoReady = false;
const check = () => { window.__fixtureReady = loaded && twoReady; };
addEventListener('message', (event) => {
  if (event.data === 'two-ready') { twoReady = true; check(); }
});
addEventListener('load', () => { loaded = true; check(); });
addEventListener('DOMContentLoaded', () => {
  document.getElementById('two').src = `http://localhost:${location.port}/frameset/two.html`;
});
</script></head><frameset rows="50%,50%"><frameset cols="50%,50%"><frame src="/frameset/one.html"><frame id="two"></frameset><frame src="/frameset/nested.html"></frameset></html>"#;

const FRAMESET_ONE: &str = r"<!doctype html><html><body><button>Button one</button></body></html>";

const FRAMESET_TWO: &str = r"<!doctype html><html><body><button>Button two</button><script>
addEventListener('load', () => top.postMessage('two-ready', '*'));
</script></body></html>";

const FRAMESET_NESTED: &str = r#"<!doctype html><html><head><title>Nested frameset</title></head><frameset><frame src="/frameset/leaf.html"></frameset></html>"#;

const FRAMESET_LEAF: &str = r#"<!doctype html><html><body><button>Leaf button</button>
<iframe srcdoc="<button>Srcdoc button</button>"></iframe></body></html>"#;

/// The main document's two framesets are nodes, the inner a child of the outer, with two frame
/// nodes under the inner and one under the outer.
fn assert_frameset_nesting(main: &[&ContentNode]) -> TestResult {
    let framesets: Vec<&ContentNode> = main
        .iter()
        .copied()
        .filter(|node| tag_of(node) == Some("frameset"))
        .collect();
    let [outer, inner] = framesets.as_slice() else {
        return Err(format!("both framesets are nodes, found {}", framesets.len()).into());
    };
    assert!(
        outer
            .children_nodes
            .iter()
            .any(|child| ptr::eq(child, *inner)),
        "the inner frameset is a child of the outer"
    );
    let child_frames = |node: &ContentNode| {
        node.children_nodes
            .iter()
            .filter(|child| tag_of(child) == Some("frame"))
            .count()
    };
    assert_eq!(
        (child_frames(inner), child_frames(outer)),
        (2, 1),
        "two frames under the inner frameset, one under the outer"
    );
    Ok(())
}

/// Under the third frame: its document's frameset, a frame, the leaf document, and in it a
/// button and an iframe with a document of its own.
fn assert_nested_frameset_document(nested: &ContentNode) -> TestResult {
    let nested_frameset = descendants(nested)
        .into_iter()
        .find(|node| tag_of(node) == Some("frameset"))
        .ok_or("the nested document has a frameset node")?;
    let leaf = nested_frameset
        .children_nodes
        .iter()
        .find(|node| tag_of(node) == Some("frame"))
        .ok_or("the nested frameset has a frame node")?;
    assert!(
        holds_text(leaf, "Leaf button"),
        "the leaf frame holds its document"
    );
    let srcdoc = descendants(leaf)
        .into_iter()
        .find(|node| tag_of(node) == Some("iframe"))
        .ok_or("the leaf document has an iframe node")?;
    assert!(
        holds_text(srcdoc, "Srcdoc button"),
        "the leaf's iframe holds a document of its own"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT; run under bun run test:browser"]
async fn p19_frameset_frames_arrive_as_iframe_nodes() -> TestResult {
    let pages = HashMap::from([
        ("/frameset/top.html".to_owned(), Page::html(FRAMESET_TOP)),
        ("/frameset/one.html".to_owned(), Page::html(FRAMESET_ONE)),
        ("/frameset/two.html".to_owned(), Page::html(FRAMESET_TWO)),
        (
            "/frameset/nested.html".to_owned(),
            Page::html(FRAMESET_NESTED),
        ),
        ("/frameset/leaf.html".to_owned(), Page::html(FRAMESET_LEAF)),
    ]);
    check_patch(
        Patch::FramesetFrames,
        pages,
        "/frameset/top.html",
        async |tab, capture| {
            let root = capture
                .apc
                .root_node
                .as_ref()
                .ok_or("capture has no root_node")?;
            let main = document_nodes(root);

            let frames: Vec<&ContentNode> = main
                .iter()
                .copied()
                .filter(|node| tag_of(node) == Some("frame"))
                .collect();
            assert_eq!(
                frames.len(),
                3,
                "the main document has a node for each of its frames"
            );
            let cdp = CdpFrames::read(tab).await?;
            for (index, frame) in frames.iter().enumerate() {
                assert!(is_frame_node(frame), "frame {index} is an iframe node");
                let id =
                    dom_node_id(frame).ok_or_else(|| format!("frame {index} has no node id"))?;
                assert_eq!(
                    tab.call_on_node(id, "function() { return this.tagName; }")
                        .await?,
                    json!("FRAME"),
                    "frame {index}'s node id resolves to its <frame>"
                );
                let token = frame_data(frame)
                    .and_then(|data| data.browseros.as_ref())
                    .and_then(|facts| facts.devtools_frame_token.as_deref())
                    .ok_or_else(|| format!("frame {index} holds no document with a frame token"))?;
                assert!(
                    cdp.knows(token),
                    "frame {index}'s document token {token} is a CDP frame id, known {cdp:?}"
                );
            }
            let [one, two, nested] = frames.as_slice() else {
                return Err("three frames, checked above".into());
            };
            assert!(
                holds_text(one, "Button one"),
                "the first frame holds its document"
            );
            assert!(
                holds_text(two, "Button two"),
                "the frame on a second site holds its document, sent by another renderer"
            );

            assert_frameset_nesting(&main)?;
            assert_nested_frameset_document(nested)
        },
    )
    .await
}

// P20: the top of each `visibility: hidden` region is a node (design 15.3). No wrapper has an
// id, a role or a retained tag; `data-t` is read by no retention rule.

const HIDDEN_REGIONS_PAGE: &str = r#"<!doctype html><html><head><title>Contract hidden regions</title></head><body>
<div data-t="a" style="visibility:hidden"><div style="visibility:visible"><button data-t="a-button">AR033 button</button></div></div>
<div data-t="b" style="visibility:hidden"><p>never shown</p></div>
<div><div data-t="c1" style="visibility:hidden"><div style="visibility:visible"><div data-t="c2" style="visibility:hidden"><button data-t="c-button" style="visibility:visible">Nested button</button></div></div></div></div>
<button data-t="e"><span data-t="e-span" style="visibility:hidden"><span style="visibility:visible">Save</span></span></button>
<div style="height:4000px">Tall spacer</div>
<div data-t="d" style="visibility:hidden"><div style="visibility:visible"><button data-t="d-button">Below the fold button</button></div></div>
<script>window.__fixtureReady = true;</script></body></html>"#;

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT; run under bun run test:browser"]
async fn p20_the_top_of_a_hidden_region_is_a_node() -> TestResult {
    let pages = HashMap::from([(
        "/hidden-regions.html".to_owned(),
        Page::html(HIDDEN_REGIONS_PAGE),
    )]);
    check_patch(
        Patch::HiddenRegionRetention,
        pages,
        "/hidden-regions.html",
        async |tab, capture| {
            // (a) AR-033, and (d) the same shape 4,000 px down, with no scrolling.
            for (region, button) in [("a", "a-button"), ("d", "d-button")] {
                let top = tab
                    .require_node(capture, &format!("[data-t={region}]"))
                    .await?;
                let top_facts = facts_of(top);
                assert_eq!(
                    (top_facts.visibility_hidden, top_facts.aria_hidden),
                    (Some(true), None),
                    "{region}: the top of the hidden region is visibility_hidden, not aria_hidden"
                );
                let inside = tab
                    .require_node(capture, &format!("[data-t={button}]"))
                    .await?;
                assert!(
                    is_under(inside, top),
                    "{region}: the visible button sits under it"
                );
            }

            // (b) Nothing visible inside: no node.
            assert!(
                tab.element_node(capture, "[data-t=b]").await?.is_none(),
                "a hidden region with nothing visible inside has no node"
            );

            // (c) Two regions, one inside the other.
            let c1 = tab.require_node(capture, "[data-t=c1]").await?;
            let c2 = tab.require_node(capture, "[data-t=c2]").await?;
            for (name, node) in [("c1", c1), ("c2", c2)] {
                assert_eq!(
                    facts_of(node).visibility_hidden,
                    Some(true),
                    "{name} is visibility_hidden"
                );
            }
            assert!(
                is_under(c2, c1),
                "the inner region's top sits under the outer's"
            );
            let c_button = tab.require_node(capture, "[data-t=c-button]").await?;
            assert!(
                is_under(c_button, c2),
                "the button sits under the inner region"
            );

            // (e) A name: the hidden span is a node between the button and the text.
            let button = tab.require_node(capture, "[data-t=e]").await?;
            let span = tab.require_node(capture, "[data-t=e-span]").await?;
            assert_eq!(
                facts_of(span).visibility_hidden,
                Some(true),
                "the hidden span is visibility_hidden"
            );
            assert!(
                is_under(span, button) && !ptr::eq(span, button),
                "the hidden span sits under the button"
            );
            assert!(
                holds_text(span, "Save"),
                "the text sits under the hidden span"
            );
            Ok(())
        },
    )
    .await
}

// P21: a cross-site frame out of view keeps its interaction info (design 16.2).

const CROSS_SITE_TOP: &str = r#"<!doctype html><html><head><title>Contract cross-site frames</title></head>
<body style="margin:0">
<iframe id="in-view" title="in view" width="600" height="150"></iframe>
<div style="height:3000px">Tall spacer</div>
<iframe id="cross" title="cross" width="600" height="150"></iframe>
<iframe id="same" title="same" width="600" height="150"></iframe>
<script>
// Two frames on a second site (localhost), one on screen and one 3,000 px down, and a same-site
// twin of the lower one. The page is ready when all three documents have loaded.
let ready = 0;
addEventListener('message', (event) => {
  if (event.data === 'ready') ready += 1;
  window.__fixtureReady = ready >= 3;
});
const child = (id, host) => {
  document.getElementById(id).src = `http://${host}:${location.port}/cross-site/child.html?name=${id}`;
};
child('in-view', 'localhost');
child('cross', 'localhost');
child('same', '127.0.0.1');
</script></body></html>"#;

const CROSS_SITE_CHILD: &str = r##"<!doctype html><html><body>
<button>Child button</button>
<div role="group" aria-label="Child cursor" style="cursor:pointer">Child cursor</div>
<div tabindex="0" aria-label="Tab stop">Tab stop</div>
<a href="#x" aria-label="Parked" style="position:absolute;left:-9999px">Parked</a>
<script>addEventListener('load', () => top.postMessage('ready', '*'));</script>
</body></html>"##;

/// The viewport of the desktop fixtures that found the gap.
const DESKTOP_VIEWPORT: (u32, u32) = (800, 600);

/// The probe nodes of a P21 child document: the button by its tag, the rest by `aria-label`.
const CROSS_SITE_PROBES: [&str; 4] = ["button", "Child cursor", "Tab stop", "Parked"];

/// A node's interaction info as P21 compares it.
#[derive(Debug, PartialEq, Eq)]
struct Interaction {
    /// Clickability reasons, sorted.
    reasons: Vec<i32>,
    is_focusable: Option<bool>,
    is_tabbable: Option<bool>,
}

impl Interaction {
    fn has(&self, reason: ClickabilityReason) -> bool {
        self.reasons.contains(&i32::from(reason))
    }

    /// Focusable and in the tab order.
    fn is_tab_stop(&self) -> bool {
        self.is_focusable == Some(true) && self.is_tabbable == Some(true)
    }
}

/// `None` when the node carries no interaction info at all.
fn interaction_of(node: &ContentNode) -> Option<Interaction> {
    let info = node
        .content_attributes
        .as_ref()?
        .interaction_info
        .as_ref()?;
    let mut reasons = info.clickability_reasons.clone();
    reasons.sort_unstable();
    Some(Interaction {
        reasons,
        is_focusable: info.is_focusable,
        is_tabbable: info.is_tabbable,
    })
}

/// The frame node holding the child document named `name`, and that document's probe nodes in
/// [`CROSS_SITE_PROBES`] order.
fn cross_site_frame<'c>(
    capture: &'c Capture,
    name: &str,
) -> Result<(&'c ContentNode, Vec<&'c ContentNode>), Box<dyn Error>> {
    let suffix = format!("name={name}");
    let frame = capture
        .nodes()
        .into_iter()
        .find(|node| {
            frame_data(node)
                .and_then(|data| data.url.as_deref())
                .is_some_and(|url| url.ends_with(&suffix))
        })
        .ok_or_else(|| format!("no frame node holds the {name} document"))?;
    let inside = descendants(frame);
    let mut probes = Vec::new();
    for probe in CROSS_SITE_PROBES {
        let node = inside
            .iter()
            .copied()
            .find(|node| {
                let node_facts = facts_of(node);
                if probe == "button" {
                    node_facts.tag.as_deref() == Some("button")
                } else {
                    node_facts.aria_label.as_deref() == Some(probe)
                }
            })
            .ok_or_else(|| format!("{name}: no {probe} node"))?;
        probes.push(node);
    }
    Ok((frame, probes))
}

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT; run under bun run test:browser"]
async fn p21_cross_site_frame_out_of_view_keeps_interaction_info() -> TestResult {
    let pages = HashMap::from([
        (
            "/cross-site/top.html".to_owned(),
            Page::html(CROSS_SITE_TOP),
        ),
        (
            "/cross-site/child.html".to_owned(),
            Page::html(CROSS_SITE_CHILD),
        ),
    ]);
    check_patch_visible(
        Patch::CrossSiteInteraction,
        pages,
        "/cross-site/top.html",
        async |tab, _| {
            let (width, height) = DESKTOP_VIEWPORT;
            tab.send(
                "Emulation.setDeviceMetricsOverride",
                json!({ "width": width, "height": height, "deviceScaleFactor": 1, "mobile": false }),
            )
            .await?;
            // Let the new viewport reach every renderer before the capture; the page never scrolls.
            tab.settle_two_frames().await?;
            time::sleep(Duration::from_millis(300)).await;
            let capture = tab.capture().await?;

            let (twin_frame, twin) = cross_site_frame(&capture, "same")?;
            let (cross_frame, cross) = cross_site_frame(&capture, "cross")?;
            let (in_view_frame, in_view) = cross_site_frame(&capture, "in-view")?;
            assert!(
                has_no_area(visible_box(twin_frame)) && has_no_area(visible_box(cross_frame)),
                "both lower frames are out of view"
            );
            assert!(!has_no_area(visible_box(in_view_frame)), "the upper frame is on screen");

            // The same-site twin, which v7 already gets right.
            let [button, cursor, tab_stop, parked] = twin.as_slice() else {
                return Err("four probes, checked by cross_site_frame".into());
            };
            let button = interaction_of(button).ok_or("twin button: no interaction info")?;
            assert!(
                button.has(ClickabilityReason::ClickableControl) && button.is_tab_stop(),
                "twin button: ClickableControl, focusable and tabbable: {button:?}"
            );
            let cursor = interaction_of(cursor).ok_or("twin group: no interaction info")?;
            assert!(
                cursor.has(ClickabilityReason::CursorPointer),
                "twin group: CursorPointer: {cursor:?}"
            );
            let tab_stop = interaction_of(tab_stop).ok_or("twin tab stop: no interaction info")?;
            assert!(
                tab_stop.has(ClickabilityReason::TabIndex) && tab_stop.is_tab_stop(),
                "twin tab stop: TabIndex, focusable and tabbable: {tab_stop:?}"
            );
            assert_eq!(
                interaction_of(parked),
                None,
                "twin parked link: the offscreen filter drops its info"
            );

            // The cross-site frame out of view carries exactly what its twin carries, and the one
            // on screen is unchanged. On v7 the out-of-view frame's nodes carry none.
            for (index, probe) in CROSS_SITE_PROBES.iter().enumerate() {
                let expected = twin.get(index).copied().map(interaction_of);
                assert_eq!(
                    cross.get(index).copied().map(interaction_of),
                    expected,
                    "{probe}: the cross-site frame out of view matches its same-site twin"
                );
                assert_eq!(
                    in_view.get(index).copied().map(interaction_of),
                    expected,
                    "{probe}: the cross-site frame on screen matches too"
                );
            }
            Ok(())
        },
    )
    .await
}

// P22: an element whose boxes were borrowed around an empty own box carries `own_box_empty`
// (design 16.3). Each row of the design's table is one shape, by its id.

fn own_box_page() -> String {
    format!(
        r##"<!doctype html><html><head><title>Contract own box</title><style>
.zero {{ display: inline-block; width: 0; height: 0; overflow: visible; cursor: pointer; }}
.sr {{ position: absolute; width: 1px; height: 1px; overflow: hidden; white-space: nowrap; }}
</style></head><body style="margin:0">
<div><span id="r1" class="zero">Zero-sized excluded target</span></div>
<div id="r2" style="height:0"><div style="float:left;width:80px;height:30px">Float</div></div>
<div style="clear:both;height:40px"></div>
<div style="position:relative;height:30px"><div id="r3" style="position:absolute;left:0;top:0;width:0;height:0"><div style="position:absolute;left:0;top:0;width:50px;height:20px">Abs child</div></div></div>
<div id="r4" style="height:0">Overflowing text</div>
<div style="height:30px"></div>
<div><span id="r5"><span style="float:left;width:60px;height:20px">Float in span</span></span></div>
<div style="clear:both"></div>
<button id="r6" style="width:0;height:0;padding:0;border:0">Zero button</button>
<div style="height:50px"></div>
<a id="r7" class="sr" style="clip-path:inset(50%)" href="#r7">Skip to content</a>
<a id="r8" class="sr" style="clip:rect(0,0,0,0)" href="#r8">Clip only</a>
<a id="r9" href="#r9"><div style="height:40px">Block link</div></a>
<a id="r10" href="#r10"><img width="40" height="18" alt="Image link" src="{GIF}"></a>
<span id="r11">Plain span</span>
<div id="r12"></div>
<script>window.__fixtureReady = true;</script></body></html>"##
    )
}

/// Rows whose own client rect is empty while the walk borrowed their boxes from their content.
const BORROWED_AROUND_EMPTY: &[&str] = &["#r1", "#r2", "#r3", "#r4", "#r5", "#r6"];

/// Rows that must not carry the field: a borrowed box around a 1×1 client rect, then boxes of
/// their own, then a box that stays empty.
const NOT_FLAGGED: &[&str] = &["#r7", "#r8", "#r9", "#r10", "#r11", "#r12"];

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT; run under bun run test:browser"]
async fn p22_own_box_empty_marks_a_borrowed_box() -> TestResult {
    let pages = HashMap::from([("/own-box.html".to_owned(), Page::html(&own_box_page()))]);
    check_patch(
        Patch::OwnBoxEmpty,
        pages,
        "/own-box.html",
        async |tab, capture| {
            // The page is what the table says: rows 1 to 6 and 12 have no client area, 7 to 11 do.
            let areas = tab
                .eval(
                    "Array.from({ length: 12 }, (_, i) => { \
                   const r = document.getElementById('r' + (i + 1)).getBoundingClientRect(); \
                   return r.width > 0 && r.height > 0; })",
                )
                .await?;
            assert_eq!(
                areas,
                json!([
                    false, false, false, false, false, false, true, true, true, true, true, false
                ]),
                "each row's own client rect has the area the table gives it"
            );

            for selector in BORROWED_AROUND_EMPTY {
                let node = tab.require_node(capture, selector).await?;
                assert_eq!(
                    facts_of(node).own_box_empty,
                    Some(true),
                    "{selector}: own_box_empty"
                );
                assert!(
                    !has_no_area(outer_box(node)),
                    "{selector}: its outer box is still the one borrowed from its content"
                );
            }
            for selector in NOT_FLAGGED {
                let node = tab.require_node(capture, selector).await?;
                assert_eq!(
                    facts_of(node).own_box_empty,
                    None,
                    "{selector}: no own_box_empty"
                );
            }
            assert!(
                !has_no_area(outer_box(tab.require_node(capture, "#r7").await?)),
                "the clip-path link keeps its borrowed outer box, and so its ref"
            );
            assert!(
                has_no_area(outer_box(tab.require_node(capture, "#r12").await?)),
                "the empty div's outer box stays empty"
            );
            Ok(())
        },
    )
    .await
}
