//! One host fill or check, parsed from the route's body into domain types. The fill's values
//! become `PrivateText` here and nowhere else (protected-fill design, section 4.2).
//!
//! A refusal never quotes the body. serde's own messages quote a scalar they could not take
//! ("invalid type: integer `4242…`"), so a value sent in the wrong shape would end up in the
//! host's error and in its logs. Each refusal is built from the field's path, whose keys are
//! printed only when they are this body's own, and a fixed sentence.

use std::time::Duration;

use browseros_core::TabId;
use neo_playwright::{Expect, PrivateText};
use neo_takeover::LockId;
use neo_trace::{RequestId, TaskId};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_path_to_error::{Error as PathError, Segment};

use crate::builtins::RecordedBox;

/// At most this many boxes in one call (design section 4.2).
pub(crate) const MAX_FIELDS: usize = 16;
/// The call's limit when the host sends none.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
/// The shortest limit a host may set.
pub(crate) const MIN_TIMEOUT: Duration = Duration::from_secs(1);
/// The longest limit a host may set.
pub(crate) const MAX_TIMEOUT: Duration = Duration::from_mins(1);

/// The keys this body has, the only ones a refusal prints. Any other key is the host's own
/// string, which could be anything, a value pasted as a key included.
const KNOWN_KEYS: [&str; 19] = [
    "requestId",
    "taskId",
    "url",
    "lockId",
    "timeoutMs",
    "fields",
    "ref",
    "value",
    "expect",
    "cut",
    "spaced",
    "keys",
    "origin",
    "type",
    "autocomplete",
    "maxLength",
    "inputMode",
    "name",
    "id",
];

/// One fill, ready to run. Holds the values, so it is never logged: `PrivateText` prints as
/// `[private]`, and nothing here prints more. A plain data bag.
#[derive(Debug)]
pub(crate) struct FillRequest {
    /// The tab from the path.
    pub(crate) tab: TabId,
    /// The host's id for this call: a repeat never types again (the fill's own record).
    pub(crate) request_id: Option<RequestId>,
    /// The host's task, which lists the fill's run with its runs.
    pub(crate) task_id: Option<TaskId>,
    /// The host lock the fill may pass, held on this tab.
    pub(crate) lock: Option<LockId>,
    /// The whole call's limit.
    pub(crate) timeout: Duration,
    /// The address the tab must still be at.
    pub(crate) url: Option<String>,
    /// The boxes as the record names them, each with its value's length, in typing order.
    pub(crate) boxes: Vec<RecordedBox>,
    /// The values, one per box, in the same order.
    pub(crate) values: Vec<PrivateText>,
}

/// One check: a fill's look without values. A plain data bag.
#[derive(Debug)]
pub(crate) struct CheckRequest {
    /// The tab from the path.
    pub(crate) tab: TabId,
    /// The host's task, which lists the check's run with its runs.
    pub(crate) task_id: Option<TaskId>,
    /// The host lock the look may pass, held on this tab (ruling R34).
    pub(crate) lock: Option<LockId>,
    /// The look's limit.
    pub(crate) timeout: Duration,
    /// The address the tab must still be at.
    pub(crate) url: Option<String>,
    /// The boxes, with no length.
    pub(crate) boxes: Vec<RecordedBox>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct WireFill {
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    lock_id: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    fields: Vec<WireField>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireField {
    #[serde(rename = "ref")]
    box_ref: String,
    value: PrivateText,
    expect: Expect,
    #[serde(default)]
    cut: bool,
    #[serde(default)]
    spaced: bool,
    #[serde(default)]
    keys: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct WireCheck {
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    lock_id: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    fields: Vec<WireCheckField>,
}

/// A check's box: a fill's field without its value, whose `expect` may be left out to read the
/// box's facts before approving any (design section 4.8).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireCheckField {
    #[serde(rename = "ref")]
    box_ref: String,
    #[serde(default)]
    expect: Option<Expect>,
    #[serde(default)]
    cut: bool,
    #[serde(default)]
    spaced: bool,
    #[serde(default)]
    keys: bool,
}

impl FillRequest {
    /// Parses a fill's body. `Err` is the 400's message.
    pub(crate) fn parse(tab: TabId, body: &[u8]) -> Result<Self, String> {
        let WireFill {
            request_id,
            task_id,
            url,
            lock_id,
            timeout_ms,
            fields,
        } = decode::<WireFill>(body)?;
        boxes_fit(fields.len())?;
        let mut boxes = Vec::with_capacity(fields.len());
        let mut values = Vec::with_capacity(fields.len());
        for field in fields {
            origin_named(&field.expect)?;
            boxes.push(RecordedBox {
                box_ref: field.box_ref,
                expect: Some(field.expect),
                cut: field.cut,
                spaced: field.spaced,
                keys: field.keys,
                characters: Some(field.value.char_count()),
            });
            values.push(field.value);
        }
        Ok(Self {
            tab,
            request_id: request_id.as_deref().map(request_id_of).transpose()?,
            task_id: task_id.as_deref().map(task_id_of).transpose()?,
            lock: lock_id.as_deref().map(lock_id_of).transpose()?,
            timeout: timeout_of(timeout_ms)?,
            url,
            boxes,
            values,
        })
    }
}

impl CheckRequest {
    /// Parses a check's body. `Err` is the 400's message.
    pub(crate) fn parse(tab: TabId, body: &[u8]) -> Result<Self, String> {
        let WireCheck {
            task_id,
            url,
            lock_id,
            timeout_ms,
            fields,
        } = decode::<WireCheck>(body)?;
        boxes_fit(fields.len())?;
        for expect in fields.iter().filter_map(|field| field.expect.as_ref()) {
            origin_named(expect)?;
        }
        let boxes = fields
            .into_iter()
            .map(|field| RecordedBox {
                box_ref: field.box_ref,
                // Absent: the look checks no origin and no fact, and reports the facts.
                expect: field.expect,
                cut: field.cut,
                spaced: field.spaced,
                keys: field.keys,
                characters: None,
            })
            .collect();
        Ok(Self {
            tab,
            task_id: task_id.as_deref().map(task_id_of).transpose()?,
            lock: lock_id.as_deref().map(lock_id_of).transpose()?,
            timeout: timeout_of(timeout_ms)?,
            url,
            boxes,
        })
    }
}

fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T, String> {
    let mut json = serde_json::Deserializer::from_slice(body);
    let decoded = serde_path_to_error::deserialize(&mut json).map_err(|error| refusal(&error))?;
    // One JSON value and nothing after it: a fill with junk or a second object behind it is not
    // a fill the host meant, and must not type or spend its requestId.
    json.end()
        .map_err(|_trailing| "body: one JSON object, with nothing after it".to_owned())?;
    Ok(decoded)
}

/// The 400's message for a body that does not decode: where, and a fixed sentence for why.
fn refusal(error: &PathError<serde_json::Error>) -> String {
    let inner = error.inner();
    let text = inner.to_string();
    let why = if inner.is_syntax() || inner.is_eof() {
        "the body is not valid JSON"
    } else if let Some(refused) = private_refusal(&text) {
        refused
    } else if text.starts_with("unknown field `requestId`") {
        "a check types nothing, so it takes no requestId; send it with the fill"
    } else if text.starts_with("unknown field `value`") {
        "a check takes no value; send values with the fill alone"
    } else if text.starts_with("unknown field") {
        "this key is not part of the request"
    } else if text.starts_with("missing field") {
        // The key that is missing is one of this body's own.
        return format!(
            "{}: {}",
            path(error),
            text.split(" at line")
                .next()
                .unwrap_or("a required key is missing")
        );
    } else {
        "this has the wrong type, or is out of range"
    };
    format!("{}: {why}", path(error))
}

/// `PrivateText`'s own refusal: a fixed sentence it chose, which quotes nothing.
fn private_refusal(text: &str) -> Option<&'static str> {
    [
        neo_playwright::PrivateTextRefused::Empty,
        neo_playwright::PrivateTextRefused::TooLong,
        neo_playwright::PrivateTextRefused::ControlCharacter,
    ]
    .into_iter()
    .map(neo_playwright::PrivateTextRefused::message)
    .find(|message| text.starts_with(message))
}

/// The field's path, with keys that are not this body's own replaced, e.g. `fields[1].value`.
fn path(error: &PathError<serde_json::Error>) -> String {
    let mut out = String::from("body");
    for segment in error.path() {
        match segment {
            Segment::Seq { index } => {
                out.push('[');
                out.push_str(&index.to_string());
                out.push(']');
            }
            Segment::Map { key } => {
                out.push('.');
                out.push_str(
                    KNOWN_KEYS
                        .iter()
                        .find(|known| **known == key)
                        .unwrap_or(&"?"),
                );
            }
            Segment::Enum { .. } | Segment::Unknown => out.push_str(".?"),
        }
    }
    out
}

fn boxes_fit(count: usize) -> Result<(), String> {
    if count == 0 || count > MAX_FIELDS {
        return Err(format!(
            "body.fields: send 1 to {MAX_FIELDS} boxes; a code in six boxes is six fields"
        ));
    }
    Ok(())
}

/// Neo never types "wherever the ref ended up": each box names its frame's origin.
fn origin_named(expect: &Expect) -> Result<(), String> {
    if expect.origin.trim().is_empty() {
        return Err("body.fields[].expect.origin: name the origin of the box's own frame".into());
    }
    Ok(())
}

fn timeout_of(millis: Option<u64>) -> Result<Duration, String> {
    let Some(millis) = millis else {
        return Ok(DEFAULT_TIMEOUT);
    };
    let timeout = Duration::from_millis(millis);
    if timeout < MIN_TIMEOUT || timeout > MAX_TIMEOUT {
        return Err(format!(
            "body.timeoutMs: from {} to {}; leave it out for {}",
            MIN_TIMEOUT.as_millis(),
            MAX_TIMEOUT.as_millis(),
            DEFAULT_TIMEOUT.as_millis()
        ));
    }
    Ok(timeout)
}

fn request_id_of(text: &str) -> Result<RequestId, String> {
    RequestId::parse(text)
        .map_err(|_refused| "body.requestId: from 1 to 128 bytes, as on `run`".to_owned())
}

fn task_id_of(text: &str) -> Result<TaskId, String> {
    TaskId::parse(text)
        .map_err(|_refused| "body.taskId: from 1 to 128 bytes, as on `run`".to_owned())
}

fn lock_id_of(text: &str) -> Result<LockId, String> {
    text.parse::<LockId>().map_err(|_refused| {
        "body.lockId: not a lock id; send the id `POST /v1/locks` answered".to_owned()
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use browseros_core::TabId;

    use super::{CheckRequest, FillRequest};

    const SECRET: &str = "CANARY-parse-4242424242424242";

    fn fill(body: &str) -> Result<FillRequest, String> {
        FillRequest::parse(TabId(7), body.as_bytes())
    }

    #[test]
    fn a_fill_body_becomes_boxes_and_private_values() -> Result<(), Box<dyn Error>> {
        let request = fill(&format!(
            r#"{{"requestId":"pay-1","taskId":"task-42","url":"https://shop.example/","timeoutMs":5000,
                "fields":[{{"ref":"f2e4","value":"{SECRET}","spaced":true,
                            "expect":{{"origin":"https://pay.example","autocomplete":"cc-number"}}}}]}}"#
        ))?;
        assert_eq!(request.tab, TabId(7));
        assert_eq!(request.values.len(), 1);
        let field = request.boxes.first().ok_or("one field")?;
        assert_eq!(field.box_ref, "f2e4");
        assert!(field.spaced && !field.cut && !field.keys);
        assert_eq!(field.characters, Some(SECRET.chars().count()));
        assert_eq!(request.timeout.as_millis(), 5000);
        assert!(
            !format!("{request:?}").contains("CANARY"),
            "a parsed request never prints its value"
        );
        Ok(())
    }

    #[test]
    fn a_refused_body_never_quotes_a_value() {
        let cases = [
            // The value in the wrong shape: serde would quote the number.
            r#"{"fields":[{"ref":"e1","value":4242424242424242,"expect":{"origin":"https://a.example"}}]}"#
                .to_owned(),
            // A control character.
            format!(
                r#"{{"fields":[{{"ref":"e1","value":"{SECRET}\n","expect":{{"origin":"https://a.example"}}}}]}}"#
            ),
            // The value pasted as a key.
            format!(r#"{{"fields":[{{"ref":"e1","{SECRET}":1,"expect":{{"origin":"x"}}}}]}}"#),
            // The value in another field.
            format!(
                r#"{{"timeoutMs":"{SECRET}","fields":[{{"ref":"e1","value":"x","expect":{{"origin":"x"}}}}]}}"#
            ),
            // Not JSON, the value inside.
            format!(r#"{{"fields":[{{"value":"{SECRET}" "#),
            // A whole fill, then junk, or a second object with the value.
            format!(
                r#"{{"fields":[{{"ref":"e1","value":"x","expect":{{"origin":"x"}}}}]}} {SECRET}"#
            ),
            format!(
                r#"{{"fields":[{{"ref":"e1","value":"x","expect":{{"origin":"x"}}}}]}}{{"value":"{SECRET}"}}"#
            ),
        ];
        for body in cases {
            let refused = fill(&body).err().unwrap_or_default();
            assert!(!refused.is_empty(), "refused: {body}");
            assert!(!refused.contains("CANARY"), "{refused}");
            assert!(!refused.contains("4242"), "{refused}");
        }
    }

    #[test]
    fn limits_are_checked_with_fixed_sentences() {
        let one = r#"{"ref":"e1","value":"x","expect":{"origin":"https://a.example"}}"#;
        let many = vec![one; 17].join(",");
        let cases = [
            (r#"{"fields":[]}"#.to_owned(), "1 to 16 boxes"),
            (format!(r#"{{"fields":[{many}]}}"#), "1 to 16 boxes"),
            (
                format!(r#"{{"timeoutMs":999,"fields":[{one}]}}"#),
                "body.timeoutMs",
            ),
            (
                format!(r#"{{"timeoutMs":60001,"fields":[{one}]}}"#),
                "body.timeoutMs",
            ),
            (
                r#"{"fields":[{"ref":"e1","value":"x","expect":{"origin":" "}}]}"#.to_owned(),
                "expect.origin",
            ),
            (
                r#"{"fields":[{"ref":"e1","value":"x","expect":{}}]}"#.to_owned(),
                "missing field `origin`",
            ),
            (
                format!(r#"{{"requestId":"","fields":[{one}]}}"#),
                "body.requestId",
            ),
            (
                format!(r#"{{"lockId":"lk","fields":[{one}]}}"#),
                "body.lockId",
            ),
            (
                r#"{"fields":[{"ref":"e1","value":"","expect":{"origin":"x"}}]}"#.to_owned(),
                "at least one character",
            ),
        ];
        for (body, says) in cases {
            let refused = fill(&body).err().unwrap_or_default();
            assert!(refused.contains(says), "{body}: {refused}");
        }
    }

    #[test]
    fn a_check_takes_no_value_and_no_request_id() {
        let check = |body: &str| CheckRequest::parse(TabId(7), body.as_bytes());
        let with_value = check(&format!(
            r#"{{"fields":[{{"ref":"e1","value":"{SECRET}","expect":{{"origin":"x"}}}}]}}"#
        ))
        .err()
        .unwrap_or_default();
        assert!(with_value.contains("takes no value"), "{with_value}");
        assert!(!with_value.contains("CANARY"), "{with_value}");
        let with_id = check(r#"{"requestId":"r","fields":[{"ref":"e1","expect":{"origin":"x"}}]}"#)
            .err()
            .unwrap_or_default();
        assert!(with_id.contains("no requestId"), "{with_id}");
        assert!(
            check(r#"{"taskId":"t","fields":[{"ref":"e1","expect":{"origin":"x"},"cut":true}]}"#)
                .is_ok()
        );
        // `expect` may be left out to read the facts before approving them.
        assert!(check(r#"{"fields":[{"ref":"e1"}]}"#).is_ok());
    }
}
