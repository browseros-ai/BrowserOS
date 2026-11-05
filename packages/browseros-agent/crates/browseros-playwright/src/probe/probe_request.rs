//! What Rust asks the probe, as `protocol.ts` types it (`ProbeRequest` and its parts).
//!
//! Field names are camelCase on the wire, and an optional TypeScript field (`position?`) is
//! left out when it is `None`, never sent as `null`. The types also decode, so the tests can
//! read every example request in `probe-protocol.json` and write it back unchanged: a field
//! one side has and the other lacks fails that round trip.

#![expect(
    dead_code,
    reason = "the protocol is mirrored whole, and each op family sends its own requests"
)]

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::value::{self, RawValue};
use serde_json::{Map, Value, json};

use crate::probe::probe_reply::{Point, Rect};
use crate::target::{Chunk, Strictness};
use crate::world::Pin;

/// A vendored `ParsedSelector` as the page receives it. Rust never reads or builds selector
/// syntax in it, with one exception: the key of an iframe a frame hop went through
/// ([`ParsedSelector::frame_key`]).
#[derive(Clone, Debug)]
pub(crate) struct ParsedSelector(Box<RawValue>);

impl ParsedSelector {
    /// The chunk exactly as the prelude parsed it.
    pub(crate) fn of(chunk: &Chunk) -> Self {
        Self(chunk.parsed.clone())
    }

    /// `aria-ref=<key>` for a Neo-internal iframe key (`#4`) the probe registered on a frame
    /// hop. The probe's aria-ref engine resolves it to that iframe for as long as it lives.
    /// Agent input can never name such a key: `target.rs` refuses any `aria-ref` body that is
    /// not a printed ref.
    pub(crate) fn frame_key(key: &str) -> Result<Self, serde_json::Error> {
        let parsed = json!({ "parts": [{ "name": "aria-ref", "body": key, "source": key }] });
        value::to_raw_value(&parsed).map(Self)
    }
}

impl Serialize for ParsedSelector {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ParsedSelector {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        // Through `Value`: inside the tagged `ProbeRequest`, serde hands over buffered content,
        // which cannot become a `RawValue` directly.
        let parsed = Value::deserialize(deserializer)?;
        value::to_raw_value(&parsed)
            .map(Self)
            .map_err(D::Error::custom)
    }
}

/// What a request runs on inside the world.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum ProbeTarget {
    /// One frame's part of a locator; queried from the document (or from the call's root
    /// element).
    Chunk(ParsedSelector),
    /// An element this op already pinned in this world.
    Pin(Pin),
    /// The document element: page-level reads and snapshots.
    Document,
    /// The call's root element itself, an element handle's node ([`super::call_on`]); detached
    /// once it has left its document.
    Root,
}

/// One request; `op` is the tag.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum ProbeRequest {
    /// `described`: the protocol, the seq and the url.
    Describe,
    /// Query, strict check and pin. `want: frame` also requires an `<iframe>` or `<frame>`.
    Resolve {
        target: ProbeTarget,
        strictness: Strictness,
        want: Want,
    },
    /// Resolve (or reuse a pin), retarget, check `states`, scroll per `scroll`, measure; with
    /// `arm`, hit-test and arm the interceptor while the rect still equals `expect_rect`.
    Act {
        target: ProbeTarget,
        strictness: Strictness,
        retarget: Retarget,
        states: Vec<ElementState>,
        /// Every `<iframe>` on the way down is visible (the resolver's hops).
        frame_visible: bool,
        scroll: ScrollPlan,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position: Option<Point>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arm: Option<ArmPlan>,
    },
    /// Stops the interceptor armed with `token` and reports whether the trusted events hit.
    Verdict { token: u32 },
    /// Stops whatever interceptor is armed in this world. Idempotent.
    Disarm,
    /// For a parent frame of an action's element: the iframe's content-box origin, and with
    /// `point`, `expectHitTarget(point, iframe)`.
    FrameBox {
        iframe: ProbeTarget,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        point: Option<Point>,
    },
    /// `fill`, `selectOptions`, `selectText`, `focus`, `blur` or `dispatchEvent` on a pin.
    Edit { pin: Pin, edit: Edit },
    /// One read of one element, resolved in the same call.
    Read {
        target: ProbeTarget,
        strictness: Strictness,
        frame_visible: bool,
        read: Read,
    },
    /// One read of every match, without waiting.
    ReadAll { target: ProbeTarget, read: ReadAll },
    /// `ariaSnapshotJSON` of `root`, this frame's rendered iframe refs and the sweep. `root:
    /// document` is upstream's frame root, `body,frameset`.
    Snapshot {
        root: ProbeTarget,
        mode: SnapshotMode,
        /// Any integer, as upstream passes it: a child frame's depth can come out zero or
        /// negative, and the engine reads those differently from no depth.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        depth: Option<i64>,
    },
    /// One `expect` poll; `final_poll` asks for the failure-only extras.
    Expect {
        target: ProbeTarget,
        strictness: Strictness,
        frame_visible: bool,
        params: ExpectParams,
        final_poll: bool,
    },
    /// `__clawSeal.sensitiveValues()`, for the evaluate guard.
    Sweep,
    /// The focused element: a sensitive field? An iframe (pinned, to hop into)?
    Focused,
    /// The vendored `urlMatches(pattern, url)`: the page is JavaScript's regex engine.
    MatchUrl { pattern: UrlPattern, url: String },
    /// The pinned element itself, by handle (`returnByValue: false`).
    Pinned { pin: Pin },
    /// The document's title, or its doctype and root markup, read through the seal.
    Document { part: DocumentPart },
    /// `document.open`, `write` and `close` with `html`; the probe then unboots itself, so
    /// the new document is booted under a new seq.
    SetContent { html: String },
}

/// Which text of the document a `document` request reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DocumentPart {
    /// `page.title()`.
    Title,
    /// `page.content()`: the doctype and the root element's `outerHTML`.
    Content,
}

/// What a `resolve` must find.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Want {
    Element,
    /// An `<iframe>` or `<frame>`, for a frame hop.
    Frame,
}

/// Upstream's retarget behaviors (`InjectedScript.retarget`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Retarget {
    None,
    FollowLabel,
    NoFollowLabel,
    ButtonLink,
}

/// States the probe checks. `stable` is not one: Rust compares two measurements in time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ElementState {
    Visible,
    Enabled,
    Editable,
}

/// Upstream's scroll rotation by attempt. `AlreadyProtocol`: Rust scrolled with
/// `DOM.scrollIntoViewIfNeeded`, and the probe only measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ScrollPlan {
    None,
    IfNeeded,
    End,
    Center,
    Start,
    AlreadyProtocol,
}

/// Which hit-target interceptor to arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum HitAction {
    Mouse,
    Hover,
    Drag,
}

/// Arms the hit-target interceptor. `block_all` is Playwright's `trial`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArmPlan {
    pub(crate) action: HitAction,
    pub(crate) block_all: bool,
    pub(crate) token: u32,
    /// The stability baseline: the probe arms only while the element still has this rect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) expect_rect: Option<Rect>,
}

/// One `selectOptions` option, as upstream's client sends it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum SelectOption {
    ValueOrLabel(String),
    Value(String),
    Label(String),
    Index(u32),
}

/// An edit of a pinned element.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum Edit {
    /// `injected.fill`; `needsinput` means Rust types the value with `Input.insertText`.
    Fill {
        value: FillValue,
    },
    SelectOptions {
        options: Vec<SelectOption>,
    },
    SelectText,
    Focus {
        reset_selection: bool,
    },
    Blur,
    DispatchEvent {
        r#type: String,
        init: Map<String, Value>,
    },
}

/// The text a fill puts into a field, which may be a password. It travels to the page in the
/// request, and never prints (STYLE P4).
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct FillValue(pub(crate) String);

impl fmt::Debug for FillValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[typed value]")
    }
}

/// A state `read` checks, as `injected.elementState` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum QueryState {
    Visible,
    Hidden,
    Enabled,
    Disabled,
    Editable,
    Checked,
    Unchecked,
}

/// What a `read` reads from its element. Values come through the seal, so a sensitive field
/// reads `••••`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Read {
    TextContent,
    InnerText,
    InnerHtml,
    InputValue,
    /// The border box in the element's own frame's viewport, or null with no layout box.
    BoundingBox,
    Attribute {
        name: String,
    },
    State {
        state: QueryState,
    },
    /// An `<iframe>`'s content-box origin in its frame's viewport, or null when not rendered:
    /// what a child frame's boxes are offset by.
    FrameOrigin,
    /// What `waitForSelector` checks on one poll: the first match pinned, its visibility and
    /// how many matched.
    Located,
}

impl Read {
    /// Whether the value can hold page text, so the read is masked with its site's values. A
    /// state, a box and a frame's origin cannot (protocol.ts `TEXTLESS`, whose reads send no
    /// sweep).
    pub(crate) fn holds_page_text(&self) -> bool {
        match self {
            Self::TextContent
            | Self::InnerText
            | Self::InnerHtml
            | Self::InputValue
            | Self::Attribute { .. }
            | Self::Located => true,
            Self::BoundingBox | Self::State { .. } | Self::FrameOrigin => false,
        }
    }
}

/// What a `readAll` reads from every match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ReadAll {
    Count,
    TextContents,
    InnerTexts,
    Pins,
}

impl ReadAll {
    /// Whether the values can hold page text: every read but the count (a pin's preview is
    /// the element's markup).
    pub(crate) fn holds_page_text(self) -> bool {
        match self {
            Self::TextContents | Self::InnerTexts | Self::Pins => true,
            Self::Count => false,
        }
    }
}

/// `ariaSnapshot`'s mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SnapshotMode {
    Ai,
    Default,
}

/// Upstream's per-poll expect parameters minus the selector, passed through as the prelude
/// built them.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct ExpectParams(pub(crate) Map<String, Value>);

/// A `waitForURL` pattern: a glob, or a `RegExp`'s source and flags.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, rename_all_fields = "camelCase", deny_unknown_fields)]
pub(crate) enum UrlPattern {
    Glob {
        glob: String,
    },
    Regex {
        regex_source: String,
        regex_flags: String,
    },
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::{Number, Value, json};

    use super::{Edit, FillValue, ParsedSelector, ProbeRequest};

    const PROTOCOL: &str = include_str!("../../tests/data/probe-protocol.json");

    /// Integral numbers decode into `f64` fields and come back as `5.0`, which `Value` keeps
    /// apart from `5`; JavaScript has one number type, so compare as it would.
    fn as_javascript(value: Value) -> Value {
        match value {
            Value::Number(number) => number
                .as_f64()
                .and_then(Number::from_f64)
                .map_or(Value::Null, Value::Number),
            Value::Array(items) => Value::Array(items.into_iter().map(as_javascript).collect()),
            Value::Object(fields) => Value::Object(
                fields
                    .into_iter()
                    .map(|(name, field)| (name, as_javascript(field)))
                    .collect(),
            ),
            other => other,
        }
    }

    #[test]
    fn tests_that_every_example_request_round_trips_unchanged() -> Result<(), Box<dyn Error>> {
        let protocol: Value = serde_json::from_str(PROTOCOL)?;
        let examples = protocol["requests"]
            .as_array()
            .ok_or("probe-protocol.json has no requests")?;
        let mut ops = Vec::new();
        for example in examples {
            let request: ProbeRequest = serde_json::from_value(example.clone())
                .map_err(|error| format!("{example} does not decode: {error}"))?;
            let written = serde_json::to_value(&request)?;
            assert_eq!(
                as_javascript(written),
                as_javascript(example.clone()),
                "Rust writes back exactly what protocol.ts typed"
            );
            ops.push(example["op"].as_str().unwrap_or_default().to_owned());
        }
        for op in [
            "describe",
            "resolve",
            "act",
            "verdict",
            "disarm",
            "frameBox",
            "edit",
            "read",
            "readAll",
            "snapshot",
            "expect",
            "sweep",
            "focused",
            "matchUrl",
            "pinned",
            "document",
            "setContent",
        ] {
            assert!(
                ops.iter().any(|seen| seen == op),
                "every request kind has an example both sides decode: {op}"
            );
        }
        Ok(())
    }

    /// Unit variants (`describe`) take no fields to check; the round trip above catches an
    /// example field Rust would drop there.
    #[test]
    fn tests_that_a_request_with_a_field_protocol_ts_lacks_is_refused() {
        for bad in [
            json!({ "op": "verdict", "token": 1, "seq": 1 }),
            json!({ "op": "resolve", "target": "document", "strictness": "strict", "want": "element", "extra": 1 }),
            json!({ "op": "readAll", "target": { "pin": 1, "chunk": null }, "read": "count" }),
            json!({ "op": "teleport" }),
        ] {
            assert!(
                serde_json::from_value::<ProbeRequest>(bad.clone()).is_err(),
                "{bad} is not a probe request"
            );
        }
    }

    #[test]
    fn tests_that_a_frame_key_is_the_selector_the_probe_expects() -> Result<(), Box<dyn Error>> {
        let protocol: Value = serde_json::from_str(PROTOCOL)?;
        let example = protocol["requests"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|request| request["op"] == "frameBox" && request["iframe"]["chunk"].is_object())
            .ok_or("no frameBox example on a frame key")?;
        let key = ParsedSelector::frame_key("#1")?;
        assert_eq!(
            serde_json::to_value(&key)?,
            example["iframe"]["chunk"],
            "a parent link's key reaches Neo's aria-ref engine as protocol.ts's example does"
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_fill_value_never_prints() {
        let edit = Edit::Fill {
            value: FillValue("hunter2".to_owned()),
        };
        assert!(
            !format!("{edit:?}").contains("hunter2"),
            "a typed value may be a password"
        );
    }
}
