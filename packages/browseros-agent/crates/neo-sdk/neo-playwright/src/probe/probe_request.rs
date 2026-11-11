//! What Rust asks the probe, as `protocol.ts` types it (`ProbeRequest` and its parts).
//!
//! Field names are camelCase on the wire, and an optional TypeScript field (`position?`) is
//! left out when it is `None`, never sent as `null`. The types also decode, so the tests can
//! read every example request in `probe-protocol.json` and write it back unchanged: a field
//! one side has and the other lacks fails that round trip.

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
    /// How many trusted pointer moves this frame has received. The route check reads it
    /// before an action's approach and again after: once it grew, the browser routes the
    /// action's point here. It reads no interceptor, so a forced action checks its route too.
    Moves,
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
    /// Paint every explicit screenshot-mask match in this world, without waiting.
    ScreenshotMask { target: ProbeTarget, color: String },
    /// Remove this world's caller-selected screenshot overlays. Idempotent.
    ScreenshotMaskCleanup,
    /// Upstream's `generateSelectorSimple` for the first element `target` matches now
    /// (`locator.normalize`, one frame's part).
    Selector { target: ProbeTarget },
    /// `ariaSnapshotJSON` of `root` and this frame's rendered iframe refs. `root: document` is
    /// upstream's frame root, `body,frameset`.
    Snapshot {
        root: ProbeTarget,
        mode: SnapshotMode,
        /// Any integer, as upstream passes it: a child frame's depth can come out zero or
        /// negative, and the engine reads those differently from no depth.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        depth: Option<i64>,
        /// Upstream's `boxes`: each node's box in this frame's own viewport.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        boxes: bool,
    },
    /// One poll of one retrying matcher; Rust's loop owns the timing, `.not` and the failure.
    Expect {
        target: ProbeTarget,
        strictness: Strictness,
        frame_visible: bool,
        params: ExpectParams,
    },
    /// The vendored `urlMatches(pattern, url)`: the page is JavaScript's regex engine.
    MatchUrl { pattern: UrlPattern, url: String },
    /// The pinned element itself, by handle (`returnByValue: false`).
    Pinned { pin: Pin },
    /// The document's title, or its doctype and root markup.
    Document { part: DocumentPart },
    /// `document.open`, `write` and `close` with `html`. The Document and this world outlive
    /// the write, so the world keeps its engine and its seq.
    SetContent { html: String },
    /// A directory upload's `input` wait, armed on the pinned element, connected or not, before
    /// Rust sends `DOM.setFileInputFiles`. Answers `value` null, or `detached` for an evicted
    /// pin.
    ArmInput { pin: Pin },
    /// Whether that `input` has come: `value` true or false.
    InputArrived { pin: Pin },
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
    /// How long the probe keeps it armed without a verdict: past the last event the op can
    /// still send, so its expiry only frees a page whose Rust is gone.
    pub(crate) expiry_ms: u64,
    /// The stability baseline: the probe arms only while the element still has this rect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) expect_rect: Option<Rect>,
    /// Root quads own area/viewport checks; the interceptor checks actual event coordinates.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) browser_mapped: bool,
}

/// One `selectOptions` option, as upstream's client sends it: an option matches when every
/// attribute named here does (injected `selectOptions`), so `{ value, label }` picks the option
/// with both, and one that names none matches any option, as upstream's does.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SelectOption {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) value_or_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) index: Option<u32>,
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
    /// Upstream's injected `setInputFiles` with `payloads`: `null`, or `{ threw }` with what it
    /// threw as CDP describes it.
    SetFiles {
        payloads: Vec<FilePayload>,
    },
    /// External drag data, dispatched after pointer actionability checks.
    Drop {
        payloads: Vec<FilePayload>,
        data: Vec<DropEntry>,
        point: Point,
    },
}

/// MIME data in the caller's insertion order, matching DataTransfer.types ordering.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DropEntry {
    pub(crate) mime_type: String,
    pub(crate) value: String,
}

/// One in-memory file as upstream's injected `setInputFiles` takes it, and as the prelude's
/// files.ts sends it: `mime_type` already defaulted from the name, and `buffer` the base64 text
/// upstream's server makes of the script's bytes (`buffer.toString('base64')`), which the page
/// decodes with `atob`. Local drop files also preserve their modification time; payloads use
/// the browser's current time when it is absent. A file's bytes may be large or
/// private, so they never print.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FilePayload {
    pub(crate) name: String,
    pub(crate) mime_type: String,
    pub(crate) buffer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_modified_ms: Option<f64>,
}

impl fmt::Debug for FilePayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilePayload")
            .field("name", &self.name)
            .field("mime_type", &self.mime_type)
            .field("last_modified_ms", &self.last_modified_ms)
            .field(
                "buffer",
                &format_args!("[{} base64 characters]", self.buffer.len()),
            )
            .finish()
    }
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

/// What a `read` reads from its element.
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
    /// What `check` and `uncheck` read around their click: `{ checked, radio }` of the element,
    /// or of the control its label names.
    Checkable,
    /// The `<input>` that `setInputFiles` sets, the element or the control its label names,
    /// after upstream's checks for `multiple` files or a `directory`: `{ pin, target,
    /// takesPaths }`, the input and the element named pinned, and whether the input is
    /// `type=file`.
    FileInput {
        multiple: bool,
        directory: bool,
    },
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

/// `ariaSnapshot`'s mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SnapshotMode {
    Ai,
    Default,
}

/// Upstream's per-poll expect parameters minus the selector, passed through as the prelude
/// built them. They hold the script's expected value, which for `toHaveValue` on a password
/// field is the password, so they never print (STYLE P4).
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct ExpectParams(pub(crate) Map<String, Value>);

impl fmt::Debug for ExpectParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[expect params]")
    }
}

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

    use super::{Edit, FillValue, ParsedSelector, ProbeRequest, SelectOption};

    const PROTOCOL: &str = include_str!("../../tests/data/probe-protocol.json");

    /// An option names any of its attributes, all of which must match (SO-008), and one the
    /// probe does not know is refused.
    #[test]
    fn tests_that_a_select_option_keeps_every_attribute_it_names() -> Result<(), Box<dyn Error>> {
        let both: SelectOption = serde_json::from_value(json!({ "value": "g", "label": "Green" }))?;
        assert_eq!(
            both,
            SelectOption {
                value: Some("g".to_owned()),
                label: Some("Green".to_owned()),
                ..SelectOption::default()
            }
        );
        assert_eq!(
            serde_json::to_value(&both)?,
            json!({ "value": "g", "label": "Green" })
        );
        assert!(serde_json::from_value::<SelectOption>(json!({ "text": "Green" })).is_err());
        Ok(())
    }

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
            "moves",
            "disarm",
            "frameBox",
            "edit",
            "read",
            "readAll",
            "screenshotMask",
            "screenshotMaskCleanup",
            "snapshot",
            "expect",
            "matchUrl",
            "pinned",
            "document",
            "setContent",
            "armInput",
            "inputArrived",
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
