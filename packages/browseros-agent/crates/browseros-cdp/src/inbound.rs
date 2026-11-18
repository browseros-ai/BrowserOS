//! Reads one text message from the browser into JSON, for the client's reader.
//!
//! Chromium writes every non-ASCII UTF-16 unit of a string as a `\uXXXX` escape, and page text
//! can hold half of a surrogate pair: a string cut in the middle of an emoji. serde refuses such
//! a lone half. The reader used to drop the whole message silently, so a reply never arrived
//! (its request waited out its timeout) and an event never fired. [`read`] repairs the halves to
//! U+FFFD and, when the text still cannot be read, says which reply or event it was, so the
//! reader can fail that request at once instead.

use serde_json::Value;

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

/// Parses `text`, repairing lone UTF-16 halves only when the plain parse fails, so a message
/// without one costs exactly what it did before.
pub(crate) fn read(text: &str) -> Result<Value, Unreadable<'_>> {
    let error = match serde_json::from_str(text) {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    let error = match well_formed(text) {
        Some(repaired) => match serde_json::from_str(&repaired) {
            Ok(value) => return Ok(value),
            Err(error) => error,
        },
        None => error,
    };
    Err(Unreadable {
        error,
        envelope: envelope(text),
    })
}

/// `text` with each lone UTF-16 half's escape replaced by U+FFFD's, or `None` when it has none.
///
/// The SDK's page-side `wellFormedReply` does the same to its own replies: an escaped backslash
/// is consumed first, so page text that spells `\ud800` stays literal. Unlike `JSON.stringify`,
/// Chromium escapes both halves of an intact pair, so a high half followed at once by a low half
/// is a pair and is kept, exactly as serde pairs them. Both escapes are six ASCII bytes, so the
/// repair rewrites four hex digits in place.
fn well_formed(text: &str) -> Option<String> {
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
    use super::{Envelope, envelope, read, well_formed};
    use serde_json::json;

    fn read_ok(text: &str) -> serde_json::Value {
        read(text).unwrap_or_else(|unreadable| panic!("{text} unreadable: {unreadable:?}"))
    }

    /// The SDK's page-side cases (`probe-reply-text.test.ts`), plus the pair Chromium escapes.
    #[test]
    fn tests_that_lone_halves_become_the_replacement_character_and_nothing_else_changes() {
        let text = r#"{"high":"\ud83d","low":"\ude00","pair":"\ud83d\ude00","upper":"\uD83D\uDE00","literal":"\\ud83d","prefixed":"\\\ud83d","\ud83d":"\ude00","high,high":"\ud83d\ud83d\ude00","low,high":"\ude00\ud83d","plain":"\u00e9\u0041"}"#;
        assert_eq!(
            read_ok(text),
            json!({
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
            })
        );
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

    #[test]
    fn tests_that_an_unreadable_message_still_names_its_reply_or_event() {
        let deep = format!("{}0{}", "[".repeat(200), "]".repeat(200));
        let reply = format!(r#"{{"id":17,"result":{{"value":{deep}}},"sessionId":"S"}}"#);
        let unreadable = read(&reply)
            .err()
            .unwrap_or_else(|| panic!("past serde's limit"));
        assert_eq!(unreadable.envelope, Envelope::Reply(17));
        assert!(unreadable.error.to_string().contains("recursion limit"));

        let event = format!(r#"{{ "method" : "Runtime.consoleAPICalled", "params":{deep}}}"#);
        assert_eq!(
            envelope(&event),
            Envelope::Event("Runtime.consoleAPICalled")
        );
        assert_eq!(
            envelope(r#"{"params":{"id":3,"method":"Inner.x","s":"\"id\":4"},"method":"Outer.y"}"#),
            Envelope::Event("Outer.y"),
            "nested and quoted names are not the message's"
        );
        assert_eq!(envelope(r#"{"result":{}}"#), Envelope::Unknown);
        assert_eq!(envelope("not json"), Envelope::Unknown);
    }
}
