//! What the probe answers, as `protocol.ts` types it (`ProbeReply` and its parts).
//!
//! Replies are parsed here, at the boundary, into these variants. A shape this build cannot
//! read means the page asset and the crate disagree, which is `neo / internal`, never a guess
//! at what the page meant.

#![expect(
    dead_code,
    reason = "the protocol is mirrored whole, and each op family reads its own replies"
)]

use std::fmt;

use browseros_core::sensitive_fields::{FieldValues, SensitiveKind};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::probe::probe_request::ElementState;
use crate::world::{FrameSeq, Pin};

/// The protocol version this build speaks. It must equal `PROTOCOL` in protocol.ts and the
/// version in `tests/data/probe-protocol.lock`, and it moves with every change to the
/// contract: a world keeps the probe an earlier build installed, and the install replaces that
/// probe only when its number differs (`engine_source`), so a shape changed under the same
/// number reaches this build in the old shape. The lock's test catches a change without a bump.
pub(crate) const PROBE_PROTOCOL: u32 = 13;

/// One reply; `kind` is the tag. Any request may also answer `error`, `uninstalled` when the
/// world has no booted probe, and `needsMask` when it came without the mask its reply needs.
#[derive(Debug, Deserialize)]
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
        retired: Option<FrameSeq>,
    },
    /// Zero matches. `stale_ref` names a ref that can never match in this document again, so
    /// the op fails at once instead of waiting for it.
    None {
        #[serde(default)]
        stale_ref: Option<String>,
    },
    /// Upstream's `strict mode violation: <locator> resolved to N elements: ...`.
    Strict {
        message: String,
    },
    /// One element, pinned. `sensitive` is the seal's `kind` of the field an edit of the
    /// element changes: the element, or the control its label names. An op marks its target
    /// sensitive on it before it logs or sends a value.
    Element {
        pin: Pin,
        preview: String,
        sensitive: Option<SensitiveKind>,
    },
    /// An `<iframe>` or `<frame>`, pinned. `key` names it in this world's ref map, for the
    /// child frame's parent link. It carries no preview, so a hop needs no mask.
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
    },
    /// Measured, not armed: frame-local CSS pixels. `sensitive` is as [`Self::Element`]'s.
    Measured {
        pin: Pin,
        rect: Rect,
        point: Point,
        in_view: bool,
        transformed: bool,
        sensitive: Option<SensitiveKind>,
    },
    /// Upstream's description of what intercepts pointer events: the page's own markup.
    Covered {
        by: PageText,
    },
    /// The interceptor is armed with `token`.
    Armed {
        rect: Rect,
        point: Point,
        token: u32,
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
    },
    /// An edit's result, masked by the seal at sensitive fields' own getters only: the
    /// probe's own words for most edits, and the options' values for `selectOptions`.
    Value {
        value: PageText,
    },
    /// One element's read, masked by the seal at sensitive fields' own getters only. `sweep`
    /// holds the raw values of the document's sensitive fields, for Rust's mask and the host's
    /// `values_swept`, never for the agent. `sensitive` is the seal's `kind` of the element the
    /// value came from.
    Read {
        value: PageText,
        sensitive: Option<SensitiveKind>,
        #[serde(deserialize_with = "field_values")]
        sweep: FieldValues,
    },
    /// A `readAll`: how many matched, and one value per match where the read has values, with
    /// `sweep` as a `Read` has it.
    Values {
        count: u32,
        values: Vec<PageText>,
        #[serde(deserialize_with = "field_values")]
        sweep: FieldValues,
    },
    /// One frame's `ariaSnapshotJSON`, its rendered iframes and the raw values of its
    /// sensitive fields (for Rust's mask, never for the agent).
    Snapshot {
        json: Value,
        iframes: Vec<SnapshotIframe>,
        #[serde(deserialize_with = "field_values")]
        sweep: FieldValues,
    },
    /// One `expect` poll: the matcher's own result before `.not`, upstream's call-log text for
    /// what the locator resolved to, the value it compared (`None` is JavaScript's
    /// `undefined`, which leaves the field out; `null` stays a value), and the seal's kind of
    /// a field compared through `__clawSeal.valueEquals` or `attributeEquals`.
    Expect {
        matches: bool,
        resolved: PageText,
        #[serde(default, deserialize_with = "present")]
        received: Option<PageText>,
        sensitive: Option<SensitiveKind>,
    },
    /// Raw values of this document's sensitive fields.
    Sweep {
        #[serde(deserialize_with = "field_values")]
        values: FieldValues,
    },
    /// The focused element: a sensitive field, an iframe the keys go into, or an element that
    /// may host a closed shadow root, which the sealed world cannot see into.
    Focused {
        sensitive: bool,
        iframe: Option<Pin>,
        host: Option<Pin>,
    },
    UrlMatch {
        matches: bool,
        url: String,
    },
    /// A `document` read: text the seal masked where it serializes a field, and the raw values
    /// of the document's sensitive fields, for Rust's mask of echoes in it.
    DocumentText {
        text: String,
        #[serde(deserialize_with = "field_values")]
        sweep: FieldValues,
    },
    /// `setContent` replaced the document's content; the world stays booted under its seq.
    Replaced,
    /// The reply would have held page text Playwright's engine cut or quoted (a preview, a
    /// strict violation's hints), and the call carried no mask to put on it first: the request
    /// goes again with its frame's site's values (`probe::call_masked`).
    NeedsMask,
    /// An error thrown in the page, classified by the probe.
    Error {
        message: String,
        class: ErrorClass,
    },
    /// The world has no booted probe (its document was replaced, or nothing installed it).
    Uninstalled,
}

impl ProbeReply {
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
            Self::Value { .. } => "value",
            Self::Read { .. } => "read",
            Self::Values { .. } => "values",
            Self::Snapshot { .. } => "snapshot",
            Self::Expect { .. } => "expect",
            Self::Sweep { .. } => "sweep",
            Self::Focused { .. } => "focused",
            Self::UrlMatch { .. } => "urlMatch",
            Self::DocumentText { .. } => "documentText",
            Self::Replaced => "replaced",
            Self::NeedsMask => "needsMask",
            Self::Error { .. } => "error",
            Self::Uninstalled => "uninstalled",
        }
    }
}

/// How the probe classified an error thrown in the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Point {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

/// A rect in one frame's CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

/// `"done"`, or upstream's description of the element the pointer events reached instead,
/// which is the page's own markup.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Hit {
    Done(HitDone),
    Missed {
        #[serde(rename = "hitTargetDescription")]
        by: PageText,
    },
}

/// The string `"done"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub(crate) enum HitDone {
    #[serde(rename = "done")]
    Done,
}

/// A rendered `<iframe>` in one frame's snapshot, for Rust to hop into.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnapshotIframe {
    pub(crate) r#ref: String,
    pub(crate) depth: u32,
    pub(crate) src: String,
}

/// A read's value as the probe sent it. The seal masks a sensitive field's own getters, not the
/// text around it: a `<div>` holding a prefilled `<textarea>` reads the value raw through
/// `innerText`, and a page can copy a value typed in any frame of its site into its own text.
/// So no read's text leaves the op without one of the ways out: [`PageText::masked`], with the
/// values of every sensitive field of the read frame's site (`page::SiteValues`),
/// [`PageText::textless`], for a read that holds no page text, or [`PageText::pin`], which takes
/// only the probe's pin and drops the text unread. It prints as `[page text]`.
#[derive(Deserialize)]
#[serde(transparent)]
pub(crate) struct PageText(Value);

impl PageText {
    /// The value with every string in it masked once with `values`, as `FieldValues::mask_json`
    /// masks one: a long value wherever it occurs, a short one only as a whole string. Only
    /// string leaves are masked: an object's keys are the probe's own (`pin`, `preview`), and a
    /// short value equal to one would break the decode without hiding anything. One pass with
    /// every value matters: a second pass over masked text misses a value that the first pass
    /// cut short (`[private]efgh` once `abcd` masked part of `abcdefgh`).
    pub(crate) fn masked(mut self, values: &FieldValues) -> Value {
        mask_strings(values, &mut self.0);
        self.0
    }

    /// The value of a read that holds no page text: a state, a box or a frame's origin
    /// (protocol.ts `TEXTLESS`), which the probe answers with booleans, numbers and its own
    /// words.
    pub(crate) fn textless(self) -> Value {
        self.0
    }

    /// The pin of a `pins` value (`{pin, preview}`), its preview dropped unread: for an op that
    /// needs the element and nothing of its text, `evaluateAll`'s matches.
    pub(crate) fn pin(self) -> Option<Pin> {
        let Value::Object(mut fields) = self.0 else {
            return None;
        };
        serde_json::from_value(fields.remove("pin")?).ok()
    }

    /// The text as a sensitive field's own values, a select's selected values: what the mask
    /// and the host's `values_swept` add to a site's sweep once an edit set them.
    pub(crate) fn field_values(&self) -> FieldValues {
        FieldValues::from_sweep(&self.0)
    }
}

impl fmt::Debug for PageText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[page text]")
    }
}

/// Masks every string in `value` with `values`.
fn mask_strings(values: &FieldValues, value: &mut Value) {
    match value {
        Value::String(_) => {
            values.mask_json(value);
        }
        Value::Array(items) => {
            for item in items {
                mask_strings(values, item);
            }
        }
        Value::Object(fields) => {
            for field in fields.values_mut() {
                mask_strings(values, field);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// A field that is there, `null` included: `Option<PageText>` alone reads `null` as absent.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<PageText>, D::Error> {
    PageText::deserialize(deserializer).map(Some)
}

/// The seal's raw values go straight into `FieldValues`, which never prints them.
fn field_values<'de, D: Deserializer<'de>>(deserializer: D) -> Result<FieldValues, D::Error> {
    let sweep = Value::deserialize(deserializer)?;
    Ok(FieldValues::from_sweep(&sweep))
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fmt::Write as _;

    use browseros_core::sensitive_fields::FieldValues;
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
            json!({ "kind": "element", "pin": 3, "preview": "<b>", "sensitive": "pin_code" }),
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
        assert_eq!(by.masked(&FieldValues::default()), json!("<div id=\"x\">"));
        Ok(())
    }

    /// Upstream logs `unexpected value "null"` for an absent attribute and `"undefined"` for
    /// no value, so the two must stay apart.
    #[test]
    fn tests_that_an_expect_poll_keeps_null_apart_from_undefined() -> Result<(), Box<dyn Error>> {
        let poll = |received: Option<Value>| {
            let mut reply = json!({ "kind": "expect", "matches": false, "resolved": "<input/>", "sensitive": null });
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
        assert_eq!(received.textless(), Value::Null);
        assert!(matches!(
            poll(None)?,
            ProbeReply::Expect { received: None, .. }
        ));
        Ok(())
    }

    #[test]
    fn tests_that_a_sweep_never_prints_its_values() -> Result<(), Box<dyn Error>> {
        let reply: ProbeReply =
            serde_json::from_value(json!({ "kind": "sweep", "values": ["hunter2"] }))?;
        assert!(
            !format!("{reply:?}").contains("hunter2"),
            "swept values are secrets (STYLE P4)"
        );
        let read: ProbeReply = serde_json::from_value(json!({
            "kind": "read", "value": "Signed in with hunter2", "sensitive": null, "sweep": [],
        }))?;
        assert!(
            !format!("{read:?}").contains("hunter2"),
            "a read's text is unmasked until its site's values mask it, so it never prints"
        );
        Ok(())
    }
}
