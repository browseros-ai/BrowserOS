//! A command's reply as the browser wrote it, kept for its caller to decode.

use crate::{CdpError, inbound};
use serde::de::DeserializeOwned;
use serde_json::{Value, error::Category, value::RawValue};
use std::fmt;
use tokio_tungstenite::tungstenite::{Bytes, Utf8Bytes};
use tracing::warn;

/// JSON-RPC's "Parse error", the code Chromium gives a command it cannot parse; here the client
/// could not parse the browser's reply.
pub(crate) const UNREADABLE_REPLY: i64 = -32700;

/// A command's `result`, kept as the JSON text the browser wrote.
///
/// One reader loop serves every tab on a connection, and it handles one message at a time. A
/// page's DOM capture is tens of megabytes of JSON and several times that as a [`Value`], and
/// while the loop built that tree, every other tab's replies and events waited behind it. So the
/// loop only checks that the result is JSON and hands over its text, which shares the WebSocket
/// message's buffer rather than copying it. The caller decodes the text in its own task, straight
/// into its own type ([`Self::decode`]) with no `Value` in between.
#[derive(Clone)]
pub struct RawReply(Utf8Bytes);

/// Only the size: the text is page content, and can be megabytes.
impl fmt::Debug for RawReply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawReply")
            .field("bytes", &self.0.len())
            .finish()
    }
}

impl RawReply {
    /// The `member` the reader found in `message`. When the member lies inside the message, the
    /// reply is a slice of the message's buffer and nothing is copied; one parsed from other text
    /// (the repaired copy of a message with a lone UTF-16 half) is copied.
    pub(crate) fn member_of(message: &Utf8Bytes, member: &RawValue) -> Self {
        let text = member.get();
        let shared = text
            .as_ptr()
            .addr()
            .checked_sub(message.as_ptr().addr())
            .filter(|start| start + text.len() <= message.len())
            .and_then(|start| {
                let slice = Bytes::from(message.clone()).slice(start..start + text.len());
                // A member's ends are JSON punctuation, so the slice is UTF-8; checking it is
                // the only way to say so without `unsafe`.
                Utf8Bytes::try_from(slice).ok()
            });
        Self(shared.unwrap_or_else(|| Utf8Bytes::from(text)))
    }

    /// The result of a reply that carries none, which has always read as `{}`.
    pub(crate) fn empty() -> Self {
        Self(Utf8Bytes::from_static("{}"))
    }

    /// A reply encoded from a parsed `value`, for a transport that answers with one.
    ///
    /// # Errors
    ///
    /// [`CdpError::Json`] when serde cannot encode the value.
    pub fn from_value(value: &Value) -> Result<Self, CdpError> {
        serde_json::to_string(value)
            .map(|text| Self(Utf8Bytes::from(text)))
            .map_err(CdpError::from)
    }

    /// The reply's text exactly as the browser wrote it. It can hold half of a UTF-16 pair (page
    /// text cut inside an emoji), which serde refuses: read it through [`Self::decode`] or take
    /// [`Self::into_json`].
    #[must_use]
    pub fn get(&self) -> &str {
        self.0.as_str()
    }

    /// Decodes the reply into `T`, in the caller's task. A lone UTF-16 half reads as U+FFFD, as
    /// in every message the reader parses, with the same repair ([`inbound::with_repair`]).
    ///
    /// # Errors
    ///
    /// [`CdpError::Protocol`] with JSON-RPC's parse-error code when serde cannot read the text
    /// even repaired (nesting past its recursion limit): the error such a reply always failed its
    /// request with. [`CdpError::Json`] when the text is JSON but not a `T`.
    pub fn decode<T: DeserializeOwned>(&self) -> Result<T, CdpError> {
        let text = self.get();
        inbound::with_repair(text, |text| serde_json::from_str(text)).map_err(|error| {
            if error.classify() == Category::Data {
                return CdpError::from(error);
            }
            warn!(bytes = text.len(), "CDP reply could not be read: {error}");
            CdpError::Protocol {
                code: UNREADABLE_REPLY,
                message: format!("the browser's reply could not be read: {error}"),
            }
        })
    }

    /// The reply's text with each lone UTF-16 half's escape replaced by U+FFFD's, so any JSON
    /// reader can read it. Nothing else is rewritten, and nothing is parsed.
    #[must_use]
    pub fn into_json(self) -> String {
        inbound::well_formed(self.get()).unwrap_or_else(|| self.get().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::RawReply;
    use crate::CdpError;
    use serde::Deserialize;
    use serde_json::{Value, json};

    #[derive(Debug, Deserialize, PartialEq)]
    struct Shape {
        count: u32,
    }

    fn raw(text: &str) -> RawReply {
        RawReply::from_value(
            &serde_json::from_str::<Value>(text).unwrap_or_else(|err| panic!("{text}: {err}")),
        )
        .unwrap_or_else(|err| panic!("encode: {err}"))
    }

    #[test]
    fn a_reply_decodes_into_the_callers_type_and_a_mismatch_is_a_json_error() {
        assert_eq!(
            raw(r#"{"count":3,"other":[1,2]}"#).decode::<Shape>(),
            Ok(Shape { count: 3 })
        );
        assert!(matches!(
            raw(r#"{"count":"three"}"#).decode::<Shape>(),
            Err(CdpError::Json(message)) if message.contains("invalid type")
        ));
    }

    #[test]
    fn a_value_from_a_transport_round_trips_and_its_text_is_not_rewritten() {
        let value = json!({ "b": [1, 2.5, "\u{e9}"], "a": null });
        let reply = RawReply::from_value(&value).unwrap_or_else(|err| panic!("encode: {err}"));
        assert_eq!(reply.decode::<Value>(), Ok(value));
        let text = reply.get().to_owned();
        assert_eq!(reply.into_json(), text);
        assert_eq!(RawReply::empty().get(), "{}");
    }
}
