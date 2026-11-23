//! What the probe answers. These decoded types also generate the page-side declarations
//! (`generated.ts`), keeping reply fields and serde spellings in one place.
//!
//! Replies are parsed here, at the boundary, into these variants. A shape this build cannot
//! read means the page asset and the crate disagree, which is `neo / internal`, never a guess
//! at what the page meant.

#![expect(
    dead_code,
    reason = "the protocol is generated whole, and each op family reads its own replies"
)]

use std::fmt;

use neo_snapshot::AriaNode;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::probe::probe_request::ElementState;
use crate::world::{FrameSeq, Pin};

/// The protocol version this build speaks. It must equal `PROTOCOL` in protocol.ts and the
/// version in `tests/data/probe-protocol.lock`, and it moves with every change to the
/// contract: a world keeps the probe an earlier build installed, and the install replaces that
/// probe only when its number differs (`engine_source`), so a shape changed under the same
/// number reaches this build in the old shape. The lock's test catches a change without a bump.
pub(crate) const PROBE_PROTOCOL: u32 = 53;

/// One message template for page-side and stitched depth refusals. The numeric reply field,
/// rather than these words, identifies a child that must become partial.
pub(crate) const SNAPSHOT_DEPTH_ERROR: &str = "This tree is nested about {levels} levels deep, more than a snapshot carries (about 147). Pass { depth: n } with n of 140 or less to cut it there, or take a locator's snapshot of a smaller section.";

/// Words use tree levels because the suggested snapshot depth option uses those levels too.
pub(crate) fn depth_error(levels: usize) -> String {
    SNAPSHOT_DEPTH_ERROR.replace("{levels}", &levels.to_string())
}

/// One reply; `kind` is the tag. Any request may also answer `error`, and `uninstalled` when
/// the world has no booted probe.
#[derive(Debug, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum ProbeReply {
    /// What `boot` and `describe` return. `seq` is the document's seq as the world knows it.
    Described {
        protocol: u32,
        seq: FrameSeq,
        url: String,
        /// Only from a boot that replaced the world's numbering: the seq it held before. Under
        /// another epoch that is a seq an earlier `Playwright` printed refs under.
        #[serde(default)]
        #[ts(optional)]
        retired: Option<FrameSeq>,
    },
    /// Zero matches. `stale_ref` names a ref that can never match in this document again, so
    /// the op fails at once instead of waiting for it.
    None {
        #[serde(default)]
        #[ts(optional)]
        stale_ref: Option<String>,
    },
    /// Upstream's `strict mode violation: <locator> resolved to N elements: ...`.
    Strict {
        message: String,
    },
    /// One element, pinned, with upstream's preview of it.
    Element {
        pin: Pin,
        preview: String,
        #[serde(default)]
        #[ts(optional)]
        resolved: Option<Resolution>,
    },
    /// An `<iframe>` or `<frame>`, pinned. `key` names it in this world's ref map, for the
    /// child frame's parent link. It carries no preview, which a hop never needs.
    Frame {
        pin: Pin,
        key: String,
        visible: bool,
        src: String,
    },
    /// Upstream's non-retriable "resolved to <div>, <iframe> was expected".
    NotFrame {
        preview: String,
    },
    /// The pinned element is gone or was evicted: back to the outer retry loop.
    Detached,
    /// The first state that does not hold.
    State {
        missing: ElementState,
        #[serde(default)]
        #[ts(optional)]
        resolved: Option<Resolution>,
    },
    /// Measured, not armed: frame-local CSS pixels.
    Measured {
        pin: Pin,
        rect: Rect,
        point: Point,
        in_view: bool,
        transformed: bool,
        /// The original element's border width plus explicit position, not a retarget's rect.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        border_offset: Option<Point>,
        #[serde(default)]
        #[ts(optional)]
        resolved: Option<Resolution>,
    },
    /// Upstream's description of what intercepts pointer events: the page's own markup, and
    /// which cover it is, for the pointer runner's count (covered-click spec section 1).
    Covered {
        #[ts(type = "string")]
        by: PageText,
        /// Absent when a second look found the element on top after all.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        cover: Option<CoverMark>,
    },
    /// The interceptor is armed with `token`.
    Armed {
        rect: Rect,
        point: Point,
        token: u32,
        /// The interceptor is armed on what is the target's own, at `point`: the press lands
        /// there by itself (`OwnPress`).
        #[serde(default)]
        #[ts(optional)]
        own: Option<OwnPress>,
    },
    Verdict {
        hit: Hit,
    },
    /// `moves`'s answer: the trusted pointer moves the frame received so far.
    Moves {
        moves: u64,
    },
    /// A parent frame's iframe origin (`None` when transformed) and its hit test.
    FrameBox {
        origin: Option<Point>,
        hit: Option<Hit>,
        /// Only for the private look (ruling R60): this page cannot show `point`, which lies
        /// outside its viewport, or where the iframe is clipped away though it takes the
        /// pointer. A click's `frameBox` always answers false.
        #[serde(default)]
        unseen: bool,
        /// A click's hit test that missed: which cover of this page took the point.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        cover: Option<CoverMark>,
    },
    /// The facts of a covered pointer action, from the world whose hit test found the cover
    /// (covered-click spec section 2). `None` when nothing covers the point any more.
    Cover {
        facts: Option<Box<CoverFacts>>,
    },
    /// An edit's result: the probe's own words for most edits, and the options' values for
    /// `selectOptions`.
    Value {
        value: PageText,
    },
    /// A `fill` or `focusToType` focused an element that did not keep the focus, and typed
    /// nothing. `holder` is upstream's preview of the element that has it now, absent when no
    /// element does (the document's `<body>`).
    NotFocused {
        #[serde(default)]
        #[ts(optional)]
        holder: Option<String>,
    },
    /// One element's read; a file input's names what its chunk resolved to.
    Read {
        value: PageText,
        #[serde(default)]
        #[ts(optional)]
        resolved: Option<Resolution>,
    },
    /// A `readAll`: how many matched, and one value per match where the read has values.
    Values {
        count: u32,
        values: Vec<PageText>,
    },
    /// The selector upstream's generator wrote for one element.
    Selector {
        selector: String,
    },
    /// One frame's `ariaSnapshotJSON` and its rendered iframes.
    Snapshot {
        /// The main page's same-request cover survey; absent outside Neo's two text formats.
        #[serde(default)]
        #[ts(optional, as = "Option<Vec<SurveyLayer>>")]
        on_top: Vec<SurveyLayer>,
        #[serde(default)]
        #[ts(optional)]
        survey_tested: Option<u32>,
        #[serde(default)]
        #[ts(optional)]
        survey_ms: Option<f64>,
        #[serde(default)]
        #[ts(optional)]
        survey_phases: Option<SurveyPhases>,
        #[serde(default)]
        #[ts(optional, as = "Option<bool>")]
        on_top_skipped: bool,
        /// Isolated-world inert annotation time, for the operation trace only.
        #[serde(default)]
        #[ts(optional)]
        inert_marking_ms: Option<f64>,
        // The frame's tree, read from the reply's text straight into typed nodes, with no
        // `serde_json::Value` between. The tagged enum still buffers the whole reply once to find
        // its `kind`; that pass was under 0.1 s of the largest tree's measured read, and the nodes
        // leave the buffer without a copy per level. A plain comment: ts-rs copies doc comments
        // into generated.ts.
        #[serde(deserialize_with = "snapshot_nodes")]
        #[ts(type = "import('../prelude/vendored').AriaSnapshotJSON")]
        json: Vec<AriaNode>,
        iframes: Vec<SnapshotIframe>,
        /// Clickable marking threw on this page and the snapshot was taken again without it;
        /// the op's call log says so.
        #[serde(default)]
        #[ts(optional, as = "Option<bool>")]
        marks_skipped: bool,
    },
    /// One `expect` poll: the matcher's own result before `.not`, upstream's call-log text for
    /// what the locator resolved to, the value it compared (`None` is JavaScript's
    /// `undefined`, which leaves the field out; `null` stays a value), and, on a poll that
    /// fails the matcher, the aria snapshot upstream puts in `error.matcherResult`.
    Expect {
        matches: bool,
        #[ts(type = "string")]
        resolved: PageText,
        #[serde(default, deserialize_with = "present")]
        #[ts(optional)]
        received: Option<PageText>,
        #[serde(default)]
        #[ts(optional, as = "Option<String>")]
        aria_snapshot: Option<PageText>,
    },
    UrlMatch {
        matches: bool,
        url: String,
    },
    /// A `document` read: the title, or the doctype and the root's markup.
    DocumentText {
        text: String,
    },
    /// `setContent` replaced the document's content; the world stays booted under its seq.
    Replaced,
    /// An error thrown in the page, classified by the probe.
    Error {
        message: String,
        class: ErrorClass,
        /// Approximate tree levels for a depth refusal. Presence identifies the failure without
        /// coupling a child frame's partial handling to the message's wording.
        #[serde(default)]
        #[ts(optional)]
        tree_depth: Option<usize>,
        /// Only from a file input that refused the files: what its chunk resolved to.
        #[serde(default)]
        #[ts(optional)]
        resolved: Option<Resolution>,
    },
    /// The world has no booted probe (its document was replaced, or nothing installed it).
    Uninstalled,
}

/// What a request's selector chunk resolved to: the first match's preview and how many
/// matched. Only a reply to a request that targets a chunk carries one.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct Resolution {
    preview: String,
    count: u32,
}

impl Resolution {
    /// Upstream's call-log line for one pass that found the element (frames.ts
    /// `_retryWithProgressIfNotConnected`).
    pub(crate) fn log_line(&self) -> String {
        let Self { preview, count } = self;
        if *count > 1 {
            format!(
                "  locator resolved to {count} elements. Proceeding with the first one: {preview}"
            )
        } else {
            format!("  locator resolved to {preview}")
        }
    }
}

impl ProbeReply {
    /// What the request's chunk resolved to, on a reply that carries it.
    pub(crate) fn resolution(&self) -> Option<&Resolution> {
        match self {
            Self::Element { resolved, .. }
            | Self::State { resolved, .. }
            | Self::Measured { resolved, .. }
            | Self::Read { resolved, .. }
            | Self::Error { resolved, .. } => resolved.as_ref(),
            _ => None,
        }
    }

    /// The reply's `kind` as protocol.ts spells it, for messages about a reply that did not
    /// fit its request.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Described { .. } => "described",
            Self::None { .. } => "none",
            Self::Strict { .. } => "strict",
            Self::Element { .. } => "element",
            Self::Frame { .. } => "frame",
            Self::NotFrame { .. } => "notFrame",
            Self::Detached => "detached",
            Self::State { .. } => "state",
            Self::Measured { .. } => "measured",
            Self::Covered { .. } => "covered",
            Self::Armed { .. } => "armed",
            Self::Verdict { .. } => "verdict",
            Self::Moves { .. } => "moves",
            Self::FrameBox { .. } => "frameBox",
            Self::Cover { .. } => "cover",
            Self::Value { .. } => "value",
            Self::NotFocused { .. } => "notFocused",
            Self::Read { .. } => "read",
            Self::Values { .. } => "values",
            Self::Selector { .. } => "selector",
            Self::Snapshot { .. } => "snapshot",
            Self::Expect { .. } => "expect",
            Self::UrlMatch { .. } => "urlMatch",
            Self::DocumentText { .. } => "documentText",
            Self::Replaced => "replaced",
            Self::Error { .. } => "error",
            Self::Uninstalled => "uninstalled",
        }
    }
}

/// How the probe classified an error thrown in the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ErrorClass {
    /// One of Playwright's own errors about the element the script chose, such as "Not a
    /// checkbox or radio button".
    Script,
    /// A selector the engine rejects.
    Selector,
    /// A Playwright feature Neo refuses here.
    Unsupported,
    /// A probe bug.
    Internal,
}

/// A point in one frame's CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct Point {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

/// A rect in one frame's CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

/// `"done"`, or upstream's description of the element the pointer events reached instead,
/// which is the page's own markup.
#[derive(Debug, Deserialize, TS)]
#[serde(untagged)]
pub(crate) enum Hit {
    Done(HitDone),
    Missed {
        #[serde(rename = "hitTargetDescription")]
        #[ts(type = "string")]
        by: PageText,
    },
}

/// Which of the target's own takes a press that lands by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) enum OwnCase {
    Label,
    Wrapper,
    Point,
}

/// A press that lands by itself (covered-click C): what takes it, as the call log names it.
#[derive(Clone, Debug, PartialEq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OwnPress {
    pub(crate) case: OwnCase,
    /// The label's or the wrapper's markup, as the call log prints an element.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) markup: Option<String>,
    /// The clear point, relative to the target, as the covered answer prints it.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) position: Option<Point>,
    /// A checkbox's, radio's or switch's state before a press through its own wrapper: the
    /// runner reads it again after, since only the state shows what such a press did.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) checked: Option<bool>,
}

/// Which cover stood at one check of a pointer action (covered-click spec section 1): the root
/// of its subtree as upstream's hit test finds it (`injectedScript.ts` `expectHitTarget`), with
/// its box and how opaque it shows. The pointer runner counts a cover as standing while each
/// check finds the same number, box and opacity: a cover that moves or fades is one the page
/// may still take away.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CoverMark {
    /// The root's number in the world that tested, the same at every check while the element
    /// is the same (a weak map, as the ref map keeps frame keys). Not a pin: a pin is a new
    /// number at every call and spends one of the world's pin entries.
    pub(crate) id: u32,
    pub(crate) rect: Rect,
    /// Its own opacity times every ancestor's.
    pub(crate) opacity: f64,
    /// It says it is loading: `aria-busy="true"` on it or around it, or a progress bar in it.
    pub(crate) busy: bool,
}

/// What a covered pointer action says about its cover (covered-click spec section 2), read in
/// the world that saw it. Its text comes from the snapshot's own reads, under the mask (R47).
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the page's reply: one flag per fact it read, each said by its own line"
)]
pub(crate) struct CoverFacts {
    /// The cover as named (`whichDialog`, before a blank layer is named after its dialog), by its
    /// number in this world: what "the same layer again" compares, since a banner's root can be
    /// `main` for one control and the banner for the next.
    pub(crate) layer: u32,
    /// The cover, "which dialog": walking up from what is on top, the first dialog-like role,
    /// else the first section role, else the root of its subtree. The ref the snapshot printed
    /// for it, else one minted for it (`RefMap.forElement`), which `getByRef` resolves.
    pub(crate) r#ref: String,
    pub(crate) role: String,
    #[ts(type = "string")]
    pub(crate) name: PageText,
    /// Upstream's markup line for it, as today's `intercepts pointer events` line prints.
    #[ts(type = "string")]
    pub(crate) markup: PageText,
    /// Its own snapshot (`ariaSnapshotJSON` in `default` mode, which mints no ref and moves
    /// nothing the agent's own snapshots depend on), which Rust prints as the covered block
    /// where it writes the error's cover (action/cover.rs). `null` for a cover that holds the
    /// control: its snapshot would be the page's.
    #[ts(type = "unknown")]
    pub(crate) snapshot: Value,
    /// Only when that snapshot holds no words: what the cover draws as text, cut at 300
    /// characters. The engine drops a `role=textbox` div's own text, so such a cover prints as
    /// a nameless textbox, and its words are here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "string")]
    pub(crate) text: Option<PageText>,
    /// Up to eight of its controls that take the pointer, a dialog's buttons kept.
    pub(crate) controls: Vec<CoverControl>,
    /// The cover holds no control and draws no text.
    pub(crate) blank: bool,
    /// The cover is a blank layer, and the ref, role, name, snapshot and controls are those of
    /// the dialog open over it, nearest the point; `markup` stays the layer's. The point lies
    /// under the layer: the dialog is not what takes the click.
    #[serde(default)]
    pub(crate) behind: bool,
    /// The cover is, or holds, a custom element whose closed shadow root this world does not
    /// know: it is not blank, and a new snapshot shows its content.
    #[serde(default)]
    pub(crate) closed: bool,
    /// A live region is on the cover or around it (a notice's role, `aria-live`): it may go by
    /// itself.
    #[serde(default)]
    pub(crate) notice: bool,
    pub(crate) kind: CoverKind,
}

/// Each added read phase is measured separately so a large document cannot hide the hit budget.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct SurveyPhases {
    pub(crate) hits: f64,
    pub(crate) count: f64,
    pub(crate) naming: f64,
    pub(crate) linking: f64,
    pub(crate) marks: f64,
    pub(crate) stopped: bool,
}

/// A layer the main frame found in the snapshot's own request. The action module words it.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SurveyLayer {
    pub(crate) role: String,
    #[ts(type = "string")]
    pub(crate) name: PageText,
    pub(crate) r#ref: String,
    pub(crate) blank: bool,
    pub(crate) loading: bool,
    pub(crate) whole_view: bool,
    pub(crate) controls: Vec<CoverControl>,
    #[serde(default)]
    #[ts(optional, type = "string")]
    pub(crate) first_words: Option<PageText>,
}

/// One control of a cover that takes the pointer.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct CoverControl {
    pub(crate) role: String,
    #[ts(type = "string")]
    pub(crate) name: PageText,
    pub(crate) r#ref: String,
}

/// How the cover stands to the control, which picks the answer's line (spec section 2's table).
#[derive(Debug, Deserialize, TS)]
#[serde(
    tag = "is",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum CoverKind {
    /// Another part of the page: a dialog, an overlay, a popup.
    Page,
    /// What is on top is an `iframe`, or the cover lists nothing but frames. `inside`: the cover
    /// is not the frame itself but holds only frames, so its way out is inside a frame.
    Frame {
        #[serde(default)]
        #[ts(optional, as = "Option<bool>")]
        inside: bool,
    },
    /// Only part of the control is covered: `position` is a clear point of it, from its
    /// padding edge, as `click({ position })` takes it.
    Partial { position: Point },
    /// The control's own label is on top. `links` counts the label's links, and `position`
    /// is a point of the label on none of them, from its padding edge, when it has links.
    Label {
        links: u32,
        #[serde(default)]
        #[ts(optional)]
        position: Option<Point>,
    },
    /// The control's own wrapper or drawing takes the pointer.
    Wrapper,
    /// The control is a label for the control on top, whose ref this is.
    Labels { r#ref: String },
    /// What is on top is a far ancestor or the page's root: nothing covers it.
    Nothing,
    /// A parent with `overflow: hidden` or `clip` clips the click point away.
    Folded,
}

/// The string `"done"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, TS)]
pub(crate) enum HitDone {
    #[serde(rename = "done")]
    Done,
}

/// A rendered `<iframe>` in one frame's snapshot, for Rust to hop into.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnapshotIframe {
    pub(crate) r#ref: String,
    pub(crate) depth: u32,
    pub(crate) src: String,
}

/// A value as the probe sent it: the page's own text. It prints as `[page text]`, so no log line carries a value the page
/// shows (STYLE P4); [`PageText::into_value`] hands it to the op's output, and [`PageText::pin`]
/// takes only the probe's pin and drops the text unread.
#[derive(Deserialize, TS)]
#[serde(transparent)]
pub(crate) struct PageText(#[ts(type = "unknown")] Value);

impl PageText {
    /// The value, for the op's output.
    pub(crate) fn into_value(self) -> Value {
        self.0
    }

    /// The value borrowed by the covered-action formatter, without consuming the reply.
    pub(crate) fn as_value(&self) -> &Value {
        &self.0
    }

    /// The value as one line for a message or the call log: a string as it is, anything else
    /// as its JSON.
    pub(crate) fn into_line(self) -> String {
        match self.0 {
            Value::String(text) => text,
            other => other.to_string(),
        }
    }

    /// The pin of a `pins` value (`{pin, preview}`), its preview dropped unread: for an op that
    /// needs the element and nothing of its text, `evaluateAll`'s matches.
    pub(crate) fn pin(self) -> Option<Pin> {
        let Value::Object(mut fields) = self.0 else {
            return None;
        };
        serde_json::from_value(fields.remove("pin")?).ok()
    }
}

impl fmt::Debug for PageText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[page text]")
    }
}

/// A field that is there, `null` included: `Option<PageText>` alone reads `null` as absent.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<PageText>, D::Error> {
    PageText::deserialize(deserializer).map(Some)
}

/// A snapshot's nodes. Any node this build cannot read fails with one fixed message: serde's own
/// message quotes the value it could not read, and a node's values are the page's text.
/// `reply_of` adds the rebuild hint and makes it `ProbeFailure::Broken`.
fn snapshot_nodes<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<AriaNode>, D::Error> {
    use serde::de::Error as _;
    Vec::<AriaNode>::deserialize(deserializer).map_err(|_unreadable| {
        D::Error::custom("the probe's snapshot has a node this build cannot read")
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fmt::Write as _;

    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};

    use super::{Hit, PROBE_PROTOCOL, ProbeReply, Rect};

    const PROTOCOL: &str = include_str!("../../tests/data/probe-protocol.json");
    const LOCK: &str = include_str!("../../tests/data/probe-protocol.lock");

    #[test]
    fn tests_that_the_contract_changes_only_with_a_protocol_bump() -> Result<(), Box<dyn Error>> {
        let digest = Sha256::digest(PROTOCOL.as_bytes());
        let mut hash = String::with_capacity(64);
        for byte in digest {
            write!(hash, "{byte:02x}")?;
        }
        let record = format!("{PROBE_PROTOCOL} {hash}");
        let locked = LOCK
            .lines()
            .find(|line| !line.starts_with('#') && !line.trim().is_empty())
            .ok_or("probe-protocol.lock records no version")?;
        let (version, locked_hash) = locked
            .split_once(' ')
            .ok_or("a lock line is `<version> <sha256>`")?;
        assert_eq!(
            version,
            PROBE_PROTOCOL.to_string(),
            "PROBE_PROTOCOL moved: record `{record}` in tests/data/probe-protocol.lock"
        );
        assert_eq!(
            locked_hash, hash,
            "probe-protocol.json changed under protocol {PROBE_PROTOCOL}: a world that kept the \
             old probe would answer the old shapes. Bump PROTOCOL in protocol.ts and \
             PROBE_PROTOCOL here, rebuild, and record the line this test then prints"
        );
        Ok(())
    }

    /// An action echoes the rect the probe measured back as `expectRect`, and the probe arms
    /// only while its new rect is `===` that one. A transformed element's rect has 17-digit
    /// floats, so a parse even one ULP off (`serde_json`'s default) makes it unstable forever.
    #[test]
    fn tests_that_a_measured_rect_comes_back_to_the_probe_bit_for_bit() -> Result<(), Box<dyn Error>>
    {
        let measured = r#"{"x":586.4763793945312,"y":364.80352783203125,"width":107.90814208984375,"height":48.043212890625}"#;
        let reply: Value = serde_json::from_str(measured)?;
        let rect: Rect = serde_json::from_value(reply)?;
        assert_eq!(serde_json::to_string(&rect)?, measured);
        Ok(())
    }

    #[test]
    fn tests_that_the_protocol_number_is_the_assets() -> Result<(), Box<dyn Error>> {
        let protocol: Value = serde_json::from_str(PROTOCOL)?;
        assert_eq!(
            protocol["protocol"].as_u64(),
            Some(u64::from(PROBE_PROTOCOL)),
            "an install refuses a probe of another protocol, so the two must move together"
        );
        Ok(())
    }

    #[test]
    fn tests_that_every_example_reply_decodes() -> Result<(), Box<dyn Error>> {
        let protocol: Value = serde_json::from_str(PROTOCOL)?;
        let examples = protocol["replies"]
            .as_array()
            .ok_or("probe-protocol.json has no replies")?;
        let mut kinds: Vec<&str> = Vec::new();
        for example in examples {
            let reply = serde_json::from_value::<ProbeReply>(example.clone())
                .map_err(|error| format!("{example} does not decode: {error}"))?;
            let kind = example["kind"].as_str().unwrap_or_default();
            assert_eq!(reply.kind(), kind, "messages name a reply as the wire does");
            kinds.push(kind);
        }
        let per_request = protocol["replyKinds"]
            .as_object()
            .ok_or("probe-protocol.json has no replyKinds")?
            .values();
        let any_request = protocol["anyRequest"].as_array().into_iter().flatten();
        for kind in per_request
            .filter_map(Value::as_array)
            .flatten()
            .chain(any_request)
            .filter_map(Value::as_str)
        {
            assert!(
                kinds.contains(&kind),
                "every reply kind a request may produce has an example Rust decodes: {kind}"
            );
        }
        Ok(())
    }

    #[test]
    fn tests_that_a_reply_this_build_cannot_read_is_refused() {
        for bad in [
            json!({ "kind": "described", "protocol": 1, "seq": 0 }),
            json!({ "kind": "element", "pin": 3, "preview": "<b>", "unexpected": null }),
            json!({ "kind": "none", "staleRef": "e5", "extra": true }),
            json!({ "kind": "teleported" }),
        ] {
            assert!(
                serde_json::from_value::<ProbeReply>(bad.clone()).is_err(),
                "{bad} is not a reply"
            );
        }
    }

    #[test]
    fn tests_that_a_hit_is_done_or_names_what_was_hit() -> Result<(), Box<dyn Error>> {
        let done: ProbeReply = serde_json::from_value(json!({ "kind": "verdict", "hit": "done" }))?;
        assert!(matches!(done, ProbeReply::Verdict { hit: Hit::Done(_) }));
        let missed: ProbeReply = serde_json::from_value(
            json!({ "kind": "verdict", "hit": { "hitTargetDescription": "<div id=\"x\">" } }),
        )?;
        let ProbeReply::Verdict {
            hit: Hit::Missed { by },
        } = missed
        else {
            return Err("a description is a miss".into());
        };
        assert_eq!(by.into_value(), json!("<div id=\"x\">"));
        Ok(())
    }

    /// Upstream logs `unexpected value "null"` for an absent attribute and `"undefined"` for
    /// no value, so the two must stay apart.
    #[test]
    fn tests_that_an_expect_poll_keeps_null_apart_from_undefined() -> Result<(), Box<dyn Error>> {
        let poll = |received: Option<Value>| {
            let mut reply = json!({ "kind": "expect", "matches": false, "resolved": "<input/>" });
            if let (Some(received), Some(fields)) = (received, reply.as_object_mut()) {
                fields.insert("received".to_owned(), received);
            }
            serde_json::from_value::<ProbeReply>(reply)
        };
        let ProbeReply::Expect {
            received: Some(received),
            ..
        } = poll(Some(Value::Null))?
        else {
            return Err("null is a value".into());
        };
        assert_eq!(received.into_value(), Value::Null);
        assert!(matches!(
            poll(None)?,
            ProbeReply::Expect { received: None, .. }
        ));
        Ok(())
    }

    #[test]
    fn tests_that_page_text_never_prints() -> Result<(), Box<dyn Error>> {
        let read: ProbeReply = serde_json::from_value(json!({
            "kind": "read", "value": "Signed in with hunter2",
        }))?;
        assert!(
            !format!("{read:?}").contains("hunter2"),
            "a read's text is the page's, so it never prints (STYLE P4)"
        );
        Ok(())
    }
}
