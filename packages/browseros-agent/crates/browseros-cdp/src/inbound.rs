//! Reads one text message from the browser, for the client's reader loop.
//!
//! The loop serves every tab on the connection and handles one message at a time, so it does as
//! little per message as it can. A reply's `result` is not parsed: serde checks it is JSON, and
//! it is handed over as the text the browser wrote, a slice of the message's buffer
//! ([`RawReply`]). The caller decodes it in its own task. An event is parsed here, since the
//! loop routes it by method and hands its params to every reader.
//!
//! Chromium writes every non-ASCII UTF-16 unit of a string as a `\uXXXX` escape, and page text
//! can hold half of a surrogate pair: a string cut in the middle of an emoji. serde refuses such
//! a lone half. The reader used to drop the whole message silently, so a reply never arrived
//! (its request waited out its timeout) and an event never fired. [`read`] repairs the halves to
//! U+FFFD in what it parses, [`RawReply::decode`] in what it kept, and when a message still
//! cannot be read, [`read`] says which reply or event it was, so the reader can fail that request
//! at once instead.

use crate::{CdpError, CdpEvent, RawReply, SessionId};
use serde::{Deserialize, Deserializer, de::IgnoredAny};
use serde_json::{Value, json, value::RawValue};
use tokio_tungstenite::tungstenite::Utf8Bytes;

/// One message from the browser, routed.
#[derive(Debug)]
pub(crate) enum Inbound {
    /// The reply to the request with this id: its result as the browser wrote it, or the
    /// browser's error.
    Reply {
        id: u64,
        outcome: Result<RawReply, CdpError>,
    },
    Event(CdpEvent),
    /// JSON that is neither, which the reader drops.
    Neither,
}

/// What a message that could not be read was, as far as a scan of its top level can tell.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Envelope<'a> {
    /// The reply to the request with this id.
    Reply(u64),
    /// An event of this method.
    Event(&'a str),
    /// Neither an id nor a method was found.
    Unknown,
}

/// A message serde could not read, even repaired: serde's reason and what the message was.
#[derive(Debug)]
pub(crate) struct Unreadable<'a> {
    pub(crate) error: serde_json::Error,
    pub(crate) envelope: Envelope<'a>,
}

/// Parses and routes one message ([`with_repair`]).
pub(crate) fn read(message: &Utf8Bytes) -> Result<Inbound, Unreadable<'_>> {
    with_repair(message, |text| parse(message, text)).map_err(|error| Unreadable {
        error,
        envelope: envelope(message),
    })
}

/// Runs `parse` on `text`, and once more on its repaired copy ([`well_formed`]) only when the
/// first parse failed and there is a lone UTF-16 half to repair: text without one is read in one
/// pass. The reader's messages and its callers' replies go through this one rule.
pub(crate) fn with_repair<T>(
    text: &str,
    parse: impl Fn(&str) -> Result<T, serde_json::Error>,
) -> Result<T, serde_json::Error> {
    let error = match parse(text) {
        Ok(parsed) => return Ok(parsed),
        Err(error) => error,
    };
    match well_formed(text) {
        Some(repaired) => parse(&repaired),
        None => Err(error),
    }
}

/// Parses `text`, which is `message` or its repaired copy.
fn parse(message: &Utf8Bytes, text: &str) -> Result<Inbound, serde_json::Error> {
    // serde reads a struct from an array as well, by position; a message that is not an object is
    // neither a reply nor an event, and is only checked to be JSON.
    if text
        .trim_start_matches([' ', '\t', '\n', '\r'])
        .starts_with('{')
    {
        serde_json::from_str::<Members<'_>>(text).map(|members| members.route(message))
    } else {
        serde_json::from_str::<IgnoredAny>(text).map(|_| Inbound::Neither)
    }
}

/// The top-level members the reader routes by. A reply's `result` is borrowed as the browser's
/// text, which serde checks is JSON but does not build into anything; every other member is
/// parsed, and a member the reader does not read is skipped. Each is `Some` whenever the message
/// has it, `null` included, as a lookup in the parsed message always was.
#[derive(Deserialize)]
struct Members<'a> {
    #[serde(default, deserialize_with = "present")]
    id: Option<Value>,
    #[serde(default, borrow, deserialize_with = "present")]
    result: Option<&'a RawValue>,
    #[serde(default, deserialize_with = "present")]
    error: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    method: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    params: Option<Value>,
    #[serde(rename = "sessionId", default, deserialize_with = "present")]
    session_id: Option<Value>,
}

/// A member that is there, whatever its value: plain `Option` would read `null` as absent.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

impl Members<'_> {
    /// A message with a numeric id is a reply, one with a string method an event.
    fn route(self, message: &Utf8Bytes) -> Inbound {
        if let Some(id) = self.id.as_ref().and_then(Value::as_u64) {
            let outcome = match self.error {
                Some(error) => Err(protocol_error(&error)),
                None => Ok(self.result.map_or_else(RawReply::empty, |result| {
                    RawReply::member_of(message, result)
                })),
            };
            return Inbound::Reply { id, outcome };
        }
        let Some(Value::String(method)) = self.method else {
            return Inbound::Neither;
        };
        let session_id = match self.session_id {
            Some(Value::String(session)) => Some(SessionId(session)),
            _ => None,
        };
        Inbound::Event(CdpEvent {
            method,
            params: self.params.unwrap_or_else(|| json!({})),
            session_id,
        })
    }
}

fn protocol_error(error: &Value) -> CdpError {
    let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown CDP protocol error")
        .to_string();
    CdpError::from_protocol(code, message)
}

/// `text` with each lone UTF-16 half's escape replaced by U+FFFD's, or `None` when it has none.
///
/// The SDK's page-side `wellFormedReply` does the same to its own replies: an escaped backslash
/// is consumed first, so page text that spells `\ud800` stays literal. Unlike `JSON.stringify`,
/// Chromium escapes both halves of an intact pair, so a high half followed at once by a low half
/// is a pair and is kept, exactly as serde pairs them. Both escapes are six ASCII bytes, so the
/// repair rewrites four hex digits in place.
pub(crate) fn well_formed(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut repaired: Option<Vec<u8>> = None;
    let mut at = 0;
    // Every escape is skipped whole, so each backslash found here starts one.
    while let Some(offset) = bytes
        .get(at..)
        .and_then(|rest| rest.iter().position(|&byte| byte == b'\\'))
    {
        let escape = at + offset;
        let Some(unit) = utf16_escape(bytes, escape) else {
            // `\\`, `\"`, `\n` and the rest are two bytes.
            at = escape + 2;
            continue;
        };
        at = escape + 6;
        let paired = (0xd800..=0xdbff).contains(&unit)
            && utf16_escape(bytes, at).is_some_and(|next| (0xdc00..=0xdfff).contains(&next));
        if paired {
            at += 6;
        } else if (0xd800..=0xdfff).contains(&unit)
            && let Some(digits) = repaired
                .get_or_insert_with(|| bytes.to_vec())
                .get_mut(escape + 2..escape + 6)
        {
            digits.copy_from_slice(b"fffd");
        }
    }
    repaired.and_then(|repaired| String::from_utf8(repaired).ok())
}

/// The UTF-16 unit of the `\uXXXX` escape that starts at `at`, if one does.
fn utf16_escape(bytes: &[u8], at: usize) -> Option<u16> {
    let (prefix, digits) = bytes.get(at..at + 6)?.split_at(2);
    if prefix != b"\\u" || !digits.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    u16::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()
}

/// The top-level `id` or `method` of a JSON object, found without parsing it. The scan follows
/// only strings and nesting, so neither a bad escape nor nesting past serde's recursion limit
/// stops it, and it stops at the first of the two it finds: Chromium writes either first.
fn envelope(text: &str) -> Envelope<'_> {
    let bytes = text.as_bytes();
    let mut depth = 0_usize;
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        match byte {
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth = depth.saturating_sub(1),
            b'"' => {
                let Some(end) = string_end(bytes, at) else {
                    break;
                };
                if depth == 1
                    && let Some(value) = member_value(bytes, end + 1)
                {
                    match text.get(at + 1..end) {
                        Some("id") => {
                            let digits = bytes.get(value..).map_or(0, |rest| {
                                rest.iter().take_while(|b| b.is_ascii_digit()).count()
                            });
                            if let Some(id) = text
                                .get(value..value + digits)
                                .and_then(|digits| digits.parse().ok())
                            {
                                return Envelope::Reply(id);
                            }
                        }
                        Some("method") if bytes.get(value) == Some(&b'"') => {
                            if let Some(method) =
                                string_end(bytes, value).and_then(|end| text.get(value + 1..end))
                            {
                                return Envelope::Event(method);
                            }
                        }
                        _ => {}
                    }
                }
                at = end;
            }
            _ => {}
        }
        at += 1;
    }
    Envelope::Unknown
}

/// The index of the quote that closes the string opening at `open`.
fn string_end(bytes: &[u8], open: usize) -> Option<usize> {
    let mut at = open + 1;
    loop {
        match bytes.get(at)? {
            b'\\' => at += 2,
            b'"' => return Some(at),
            _ => at += 1,
        }
    }
}

/// Where a member's value starts, when the string that ended just before `after` is a key.
fn member_value(bytes: &[u8], after: usize) -> Option<usize> {
    let skip_space = |from: usize| {
        Some(
            from + bytes
                .get(from..)?
                .iter()
                .position(|b| !b.is_ascii_whitespace())?,
        )
    };
    let colon = skip_space(after)?;
    if bytes.get(colon) != Some(&b':') {
        return None;
    }
    skip_space(colon + 1)
}

#[cfg(test)]
mod tests {
    use super::{Envelope, Inbound, envelope, read, well_formed};
    use crate::{CdpError, CdpEvent, RawReply, alloc_count, raw_reply::UNREADABLE_REPLY};
    use serde_json::{Value, json};
    use tokio_tungstenite::tungstenite::Utf8Bytes;

    fn read_ok(text: &str) -> Inbound {
        let message = Utf8Bytes::from(text);
        read(&message).unwrap_or_else(|unreadable| panic!("{text} unreadable: {unreadable:?}"))
    }

    fn reply(text: &str) -> (u64, Result<RawReply, CdpError>) {
        match read_ok(text) {
            Inbound::Reply { id, outcome } => (id, outcome),
            other => panic!("{text} is not a reply: {other:?}"),
        }
    }

    fn result(text: &str) -> RawReply {
        reply(text)
            .1
            .unwrap_or_else(|err| panic!("{text} failed: {err}"))
    }

    fn event(text: &str) -> CdpEvent {
        match read_ok(text) {
            Inbound::Event(event) => event,
            other => panic!("{text} is not an event: {other:?}"),
        }
    }

    /// The SDK's page-side cases (`probe-reply-text.test.ts`), plus the pair Chromium escapes,
    /// read the same in an event the reader parses and in a reply its caller decodes.
    #[test]
    fn tests_that_lone_halves_become_the_replacement_character_and_nothing_else_changes() {
        let text = r#"{"high":"\ud83d","low":"\ude00","pair":"\ud83d\ude00","upper":"\uD83D\uDE00","literal":"\\ud83d","prefixed":"\\\ud83d","\ud83d":"\ude00","high,high":"\ud83d\ud83d\ude00","low,high":"\ude00\ud83d","plain":"\u00e9\u0041"}"#;
        let expected = json!({
            "high": "\u{fffd}",
            "low": "\u{fffd}",
            "pair": "\u{1f600}",
            "upper": "\u{1f600}",
            "literal": "\\ud83d",
            "prefixed": "\\\u{fffd}",
            "\u{fffd}": "\u{fffd}",
            "high,high": "\u{fffd}\u{1f600}",
            "low,high": "\u{fffd}\u{fffd}",
            "plain": "\u{e9}A",
        });
        let parsed = event(&format!(r#"{{"method":"Test.text","params":{text}}}"#));
        assert_eq!(parsed.params, expected);

        let kept = result(&format!(r#"{{"id":3,"result":{text}}}"#));
        assert_eq!(kept.get(), text, "the loop keeps the halves as written");
        let decoded: Value = kept.decode().unwrap_or_else(|err| panic!("decode: {err}"));
        assert_eq!(decoded, expected);
        let repaired: Value = serde_json::from_str(&kept.into_json())
            .unwrap_or_else(|err| panic!("the repaired text is JSON: {err}"));
        assert_eq!(repaired, expected);
    }

    #[test]
    fn tests_that_text_without_a_lone_half_is_not_rewritten() {
        assert_eq!(
            well_formed(r#"{"a":"\ud83d\ude00 \\ud800 \n \u0041"}"#),
            None
        );
        assert_eq!(
            well_formed(r#"{"a":"x\udc00y"}"#).as_deref(),
            Some(r#"{"a":"x\ufffdy"}"#)
        );
    }

    /// The loop's whole cost for a reply is checking its text is JSON: the result it hands over
    /// is a slice of the message's buffer, so on the thread that reads it, it allocates nothing
    /// the size of the result. The same text parsed into a `Value`, as the loop once did,
    /// allocates many times its size, for a tree every other message on the connection waited
    /// behind.
    #[test]
    fn tests_that_a_replys_result_is_kept_as_the_browsers_text_and_not_built() {
        let rows: Vec<String> = (0..20_000)
            .map(|row| format!(r#"[{row}, {{"z":{row},"a":"\u00e9"}}]"#))
            .collect();
        let written = format!("[{}]", rows.join(",  "));
        let message = Utf8Bytes::from(format!(r#"{{"id":41,"result":{written},"sessionId":"S"}}"#));

        let before = alloc_count::allocated();
        let routed = read(&message);
        let on_loop = alloc_count::allocated() - before;
        let (id, outcome) = match routed {
            Ok(Inbound::Reply { id, outcome }) => (id, outcome),
            other => panic!("not a reply: {other:?}"),
        };
        let kept = outcome.unwrap_or_else(|err| panic!("reply: {err}"));
        assert_eq!(id, 41);
        assert_eq!(
            kept.get(),
            written,
            "spacing, key order and escapes are the browser's"
        );
        assert!(
            on_loop < 4096,
            "the reader allocated {on_loop} bytes for a {} byte result",
            written.len()
        );

        let before = alloc_count::allocated();
        let tree: Value =
            serde_json::from_str(&written).unwrap_or_else(|err| panic!("parse: {err}"));
        let parsed = alloc_count::allocated() - before;
        assert!(
            parsed >= 4 * written.len(),
            "a parse allocates {parsed} bytes, so the bound above would catch one"
        );
        drop(tree);
    }

    #[test]
    fn tests_that_routing_reads_each_member_as_a_lookup_in_the_message_did() {
        assert_eq!(result(r#"{"id":5,"result":null}"#).get(), "null");
        assert_eq!(
            result(r#"{"id":4,"result":{"a" : 1},"sessionId":"\ud83d"}"#).get(),
            r#"{"a" : 1}"#,
            "a message read only once repaired hands over its result from the repaired copy"
        );
        assert_eq!(result(r#"{"id":5}"#).get(), "{}");
        assert!(matches!(
            reply(r#"{"id":5,"error":{"code":-32000,"message":"No session with given id"}}"#).1,
            Err(CdpError::SessionGone)
        ));
        assert!(matches!(
            reply(r#"{"id":5,"error":null,"result":{}}"#).1,
            Err(CdpError::Protocol { code: 0, ref message }) if message == "unknown CDP protocol error"
        ));

        let parsed = event(r#"{"method":"Test.a","params":null,"sessionId":"S"}"#);
        assert_eq!(parsed.params, Value::Null);
        assert_eq!(parsed.session_id.as_ref().map(|s| s.as_str()), Some("S"));
        assert_eq!(event(r#"{"method":"Test.b"}"#).params, json!({}));
        assert_eq!(
            event(r#"{"id":"7","method":"Test.c","sessionId":7}"#).session_id,
            None,
            "an id that is not a request's number does not make a reply"
        );
        for neither in [r#"{"method":7}"#, "[1, 2]", " 3", r#"{"result":{}}"#] {
            assert!(matches!(read_ok(neither), Inbound::Neither), "{neither}");
        }
    }

    #[test]
    fn tests_that_an_unreadable_message_still_names_its_reply_or_event() {
        let deep = format!("{}0{}", "[".repeat(200), "]".repeat(200));
        let event = Utf8Bytes::from(format!(
            r#"{{ "method" : "Runtime.consoleAPICalled", "params":{deep}}}"#
        ));
        let unreadable = read(&event)
            .err()
            .unwrap_or_else(|| panic!("past serde's limit"));
        assert_eq!(
            unreadable.envelope,
            Envelope::Event("Runtime.consoleAPICalled")
        );
        assert!(unreadable.error.to_string().contains("recursion limit"));

        let broken = Utf8Bytes::from_static(r#"{"id":17,"result":{"value":tru},"sessionId":"S"}"#);
        let unreadable = read(&broken).err().unwrap_or_else(|| panic!("not JSON"));
        assert_eq!(unreadable.envelope, Envelope::Reply(17));

        assert_eq!(
            envelope(r#"{"params":{"id":3,"method":"Inner.x","s":"\"id\":4"},"method":"Outer.y"}"#),
            Envelope::Event("Outer.y"),
            "nested and quoted names are not the message's"
        );
        assert_eq!(envelope(r#"{"result":{}}"#), Envelope::Unknown);
        assert_eq!(envelope("not json"), Envelope::Unknown);
    }

    /// A reply's result is not parsed on the loop, so nesting past serde's limit reaches its
    /// caller, whose decode fails it with the error the loop used to give it.
    #[test]
    fn tests_that_a_result_nested_past_serdes_limit_fails_its_decode_as_unreadable() {
        let deep = format!("{}0{}", "[".repeat(200), "]".repeat(200));
        let kept = result(&format!(
            r#"{{"id":17,"result":{{"value":{deep}}},"sessionId":"S"}}"#
        ));
        match kept.decode::<Value>() {
            Err(CdpError::Protocol { code, message }) => {
                assert_eq!(code, UNREADABLE_REPLY);
                assert!(
                    message.starts_with("the browser's reply could not be read: recursion limit"),
                    "{message}"
                );
            }
            other => panic!("expected the unreadable reply's error, got {other:?}"),
        }
    }
}
