//! A command's reply as the browser wrote it, kept for its caller to decode.

use crate::{CdpError, inbound};
use serde::de::DeserializeOwned;
use serde_json::{Value, error::Category, value::RawValue};
use tracing::warn;

/// JSON-RPC's "Parse error", the code Chromium gives a command it cannot parse; here the client
/// could not parse the browser's reply.
pub(crate) const UNREADABLE_REPLY: i64 = -32700;

/// A command's `result`, kept as the JSON text the browser wrote.
///
/// One reader loop serves every tab on a connection, and it handles one message at a time. A
/// page's DOM capture is tens of megabytes of JSON and several times that as a [`Value`], and
/// while the loop built that tree, every other tab's replies and events waited behind it. So the
/// loop only copies a reply's result out of its message, and the caller decodes the text in its
/// own task, straight into its own type ([`Self::decode`]) with no `Value` in between.
#[derive(Debug, Clone)]
pub struct RawReply(Box<RawValue>);

impl RawReply {
    /// The text of a member the reader found in a message. It is copied: the message's buffer is
    /// the socket reader's, and it is gone once the reader moves on.
    pub(crate) fn copied(text: &RawValue) -> Self {
        Self(text.to_owned())
    }

    /// The result of a reply that carries none, which has always read as `{}`.
    pub(crate) fn empty() -> Self {
        // `{}` is JSON, so the fallback is never taken.
        Self(RawValue::from_string("{}".to_owned()).unwrap_or_else(|_| RawValue::NULL.to_owned()))
    }

    /// A reply encoded from a parsed `value`, for a transport that answers with one.
    ///
    /// # Errors
    ///
    /// [`CdpError::Json`] when serde cannot encode the value.
    pub fn from_value(value: &Value) -> Result<Self, CdpError> {
        serde_json::value::to_raw_value(value)
            .map(Self)
            .map_err(CdpError::from)
    }

    /// The reply's text exactly as the browser wrote it. It can hold half of a UTF-16 pair (page
    /// text cut inside an emoji), which serde refuses: read it through [`Self::decode`] or take
    /// [`Self::into_json`].
    #[must_use]
    pub fn get(&self) -> &str {
        self.0.get()
    }

    /// Decodes the reply into `T`, in the caller's task. A lone UTF-16 half reads as U+FFFD, as
    /// in every message the reader parses; the repair is tried only after a plain decode failed,
    /// so a reply without one is read in one pass.
    ///
    /// # Errors
    ///
    /// [`CdpError::Protocol`] with JSON-RPC's parse-error code when serde cannot read the text
    /// even repaired (nesting past its recursion limit): the error such a reply always failed its
    /// request with. [`CdpError::Json`] when the text is JSON but not a `T`.
    pub fn decode<T: DeserializeOwned>(&self) -> Result<T, CdpError> {
        let text = self.0.get();
        let error = match serde_json::from_str(text) {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        let error = match inbound::well_formed(text) {
            Some(repaired) => match serde_json::from_str(&repaired) {
                Ok(value) => return Ok(value),
                Err(error) => error,
            },
            None => error,
        };
        Err(match error.classify() {
            Category::Data => CdpError::from(error),
            Category::Syntax | Category::Eof | Category::Io => {
                // The text is page content and can be megabytes, so only its size is logged.
                warn!(bytes = text.len(), "CDP reply could not be read: {error}");
                CdpError::Protocol {
                    code: UNREADABLE_REPLY,
                    message: format!("the browser's reply could not be read: {error}"),
                }
            }
        })
    }

    /// The reply's text with each lone UTF-16 half's escape replaced by U+FFFD's, so any JSON
    /// reader can read it. Nothing else is rewritten, and nothing is parsed.
    #[must_use]
    pub fn into_json(self) -> String {
        match inbound::well_formed(self.0.get()) {
            Some(repaired) => repaired,
            None => Box::<str>::from(self.0).into_string(),
        }
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
