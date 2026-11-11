//! A prelude's refusal as the door reads it (FR/RC C4). The Playwright prelude refuses a method Neo
//! does not serve with `__neo.refuse(api, code, reason)`, which calls the door under the method's
//! name with `{"$neo":"refused","code","reason"}` as its input. The door refuses that call at
//! admission, under any name, and records it like every admission refusal, so the run's error
//! keeps the refusal's code and every refused try is one trace row.
//!
//! The script can send the brand itself through `__neo_invoke`, so a brand buys no more than the
//! script's own mistake would (the spoof rule): its owner is always `script`, and its code one of
//! the two a script's mistake carries, never a failure of Neo's, the host's or the website's.

use serde_json::Value;

use super::{ErrorCode, OperationError, TEXT_CHARS, clip};

/// The codes a refusal may name: a method Neo does not serve, and an input it does not take, such
/// as one option of a method it does serve.
const REFUSAL_CODES: [ErrorCode; 2] = [ErrorCode::METHOD_UNAVAILABLE, ErrorCode::INVALID_INPUT];

/// The refusal brand a script call's input is, read once at admission. Only `code` and `reason`
/// are read; the record keeps every field the brand carried, redacted by the name's spec as any
/// input is.
pub(super) enum RefusalBrand<'a> {
    /// A refusal the script may make: a code from [`REFUSAL_CODES`], and the reason as sent.
    Refused { code: ErrorCode, reason: &'a str },
    /// Any other code, or a reason that is missing or not a string.
    Malformed,
}

impl<'a> RefusalBrand<'a> {
    /// The brand `input` is, if it is one: an object whose `$neo` is `"refused"`.
    #[must_use]
    pub(super) fn of(input: &'a Value) -> Option<Self> {
        let brand = input.as_object()?;
        if brand.get("$neo")?.as_str()? != "refused" {
            return None;
        }
        let code = brand
            .get("code")
            .and_then(Value::as_str)
            .and_then(|sent| REFUSAL_CODES.into_iter().find(|code| code.as_str() == sent));
        let reason = brand.get("reason").and_then(Value::as_str);
        Some(match (code, reason) {
            (Some(code), Some(reason)) => Self::Refused { code, reason },
            (None, _) | (_, None) => Self::Malformed,
        })
    }

    /// The door's answer, always the script's own failure. A refusal keeps its code, and its
    /// reason cut to [`TEXT_CHARS`]. A malformed brand is `invalid_input`, naming the method as
    /// `shown`, the name as the door shows it.
    #[must_use]
    pub(super) fn refusal(self, shown: &str) -> OperationError {
        match self {
            Self::Refused { code, reason } => {
                OperationError::script(code, clip(reason.to_owned(), TEXT_CHARS))
            }
            Self::Malformed => OperationError::script(
                ErrorCode::INVALID_INPUT,
                format_args!(
                    "`{shown}`: a refusal may name only method_unavailable or invalid_input"
                ),
            ),
        }
    }
}
