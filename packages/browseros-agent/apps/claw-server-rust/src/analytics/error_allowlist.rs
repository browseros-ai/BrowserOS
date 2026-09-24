//! Classifies a `run` failure's error text against a closed set of shapes we own.
//!
//! The error string is whatever the script threw, verbatim, so it can hold anything the
//! page held. Scrubbing it would be a blocklist, and a blocklist fails open on the case
//! nobody predicted. This matches instead: a recognised shape reports its class, and
//! anything else is reduced to its bare error type with the message discarded.

/// Every member of the `browser` SDK. An identifier named in an engine error is only
/// echoed back when it is one of these, because this set is ours and contains no user
/// data by construction.
const SDK_SURFACE: &[&str] = &[
    // top-level members
    "pages",
    "observe",
    "input",
    "nav",
    "cdp",
    "cdpJsonForPage",
    "read",
    "grep",
    "wait",
    "screenshot",
    "evaluate",
    "download",
    "pdf",
    "upload",
    "tabGroups",
    "windows",
    "saveHelper",
    "listHelpers",
    "readHelper",
    // chained methods, the ones agents reach for on the wrong shape
    "snapshot",
    "diff",
    "resolveRef",
    "click",
    "fill",
    "type",
    "press",
    "hover",
    "selectOption",
    "scroll",
    "goto",
    "back",
    "forward",
    "reload",
    "newPage",
    "close",
    "list",
    "getInfo",
    // globals the sandbox does bridge, so a complaint about one is ours to hear
    "browser",
    "console",
    "sleep",
    "setTimeout",
    "clearTimeout",
    // globals it does not bridge, the ones agents assume
    "fetch",
    "require",
    "process",
    "window",
    "document",
    "localStorage",
    "XMLHttpRequest",
    "Buffer",
    "__dirname",
    "global",
];

/// What we are willing to say about a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorClass {
    /// Hit the hard wall-clock ceiling.
    Timeout,
    /// The script did not parse.
    SyntaxError,
    /// The return value was too large to serialize back.
    ReturnTooLarge,
    /// Arguments failed validation on the way across the bridge.
    InvalidCallArguments,
    /// An engine error naming something on our own surface, for example
    /// `TypeError` about `forText`, or `ReferenceError` about `fetch`.
    Engine {
        kind: String,
        identifier: Option<String>,
    },
    /// Recognised as an error type but nothing else. The message is dropped.
    Unrecognized { kind: String },
}

impl ErrorClass {
    /// A stable, low-cardinality label for grouping.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Timeout => "timeout".to_string(),
            Self::SyntaxError => "syntax_error".to_string(),
            Self::ReturnTooLarge => "return_too_large".to_string(),
            Self::InvalidCallArguments => "invalid_call_arguments".to_string(),
            Self::Engine { kind, identifier } => match identifier {
                Some(id) => format!("engine:{kind}:{id}"),
                None => format!("engine:{kind}"),
            },
            Self::Unrecognized { kind } => format!("unrecognized:{kind}"),
        }
    }
}

/// The error types an engine can raise. Anything outside this list reports as `Error`,
/// so a thrown string cannot invent a label.
const ENGINE_KINDS: &[&str] = &[
    "TypeError",
    "ReferenceError",
    "SyntaxError",
    "RangeError",
    "EvalError",
    "URIError",
    "InternalError",
    "AggregateError",
    "Error",
];

/// Classifies an error message. Never returns any span of the input that is not either a
/// fixed label or a member of `SDK_SURFACE`.
#[must_use]
pub fn classify(message: &str) -> ErrorClass {
    let trimmed = message.trim();

    // Both script tools share the QuickJS runtime, which names the tool in
    // its deadline and syntax messages (`run exceeded …`, `playwright exceeded …`).
    if trimmed.starts_with("run exceeded") || trimmed.starts_with("playwright exceeded") {
        return ErrorClass::Timeout;
    }
    if trimmed.starts_with("run: syntax error") || trimmed.starts_with("playwright: syntax error") {
        return ErrorClass::SyntaxError;
    }
    if trimmed.starts_with("run return value exceeded") {
        return ErrorClass::ReturnTooLarge;
    }
    if trimmed.starts_with("Invalid browser call arguments") {
        return ErrorClass::InvalidCallArguments;
    }

    let Some((kind, rest)) = trimmed.split_once(':') else {
        return ErrorClass::Unrecognized {
            kind: "Error".to_string(),
        };
    };
    let kind = kind.trim();
    if !ENGINE_KINDS.contains(&kind) {
        return ErrorClass::Unrecognized {
            kind: "Error".to_string(),
        };
    }

    ErrorClass::Engine {
        kind: kind.to_string(),
        identifier: sdk_identifier_in(rest),
    }
}

/// Pulls the first SDK-surface identifier out of free text. Returning `None` is always
/// safe; returning a value is only possible for names we ship.
fn sdk_identifier_in(rest: &str) -> Option<String> {
    let mut token = String::new();
    let mut found: Option<String> = None;
    for c in rest.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
            token.push(c);
            continue;
        }
        if !token.is_empty() {
            if SDK_SURFACE.contains(&token.as_str()) && found.is_none() {
                found = Some(token.clone());
            }
            token.clear();
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shapes_we_own_are_recognised() {
        assert_eq!(classify("run exceeded 30000ms"), ErrorClass::Timeout);
        assert_eq!(
            classify("run: syntax error - unexpected token"),
            ErrorClass::SyntaxError
        );
        assert_eq!(
            classify("run return value exceeded 2000000 byte limit"),
            ErrorClass::ReturnTooLarge
        );
        assert_eq!(
            classify("Invalid browser call arguments: missing field `code`"),
            ErrorClass::InvalidCallArguments
        );
    }

    #[test]
    fn engine_errors_keep_the_identifier_when_it_is_ours() {
        assert_eq!(
            classify("TypeError: browser.wait(...).forText is not a function").label(),
            "engine:TypeError:browser"
        );
        assert_eq!(
            classify("ReferenceError: fetch is not defined").label(),
            "engine:ReferenceError:fetch"
        );
        assert_eq!(
            classify("ReferenceError: document is not defined").label(),
            "engine:ReferenceError:document"
        );
    }

    #[test]
    fn engine_errors_drop_identifiers_that_are_not_ours() {
        let class = classify("ReferenceError: customerAccountNumber is not defined");
        assert_eq!(
            class,
            ErrorClass::Engine {
                kind: "ReferenceError".to_string(),
                identifier: None
            }
        );
        assert!(!class.label().contains("customerAccountNumber"));
    }

    /// The case the allowlist exists for. An agent throwing a descriptive error is doing
    /// the right thing for itself and handing us a payload we never asked for.
    #[test]
    fn a_thrown_message_carrying_user_data_is_discarded_entirely() {
        const LEAKS: &[(&str, &str)] = &[
            ("Error: no results for 'pregnancy test'", "pregnancy"),
            (
                "Error: login failed for person@example.com",
                "person@example.com",
            ),
            (
                "Error: could not find /Users/someone/Desktop/taxes.pdf",
                "/Users/someone",
            ),
            (
                "Error: unexpected balance 48,201.55 on account 4471",
                "48,201.55",
            ),
            (
                "net::ERR_NAME_NOT_RESOLVED at https://intranet.acme.test/hr/salaries",
                "salaries",
            ),
            ("the page said: Welcome back, Amara Okafor", "Amara"),
        ];
        for (message, secret) in LEAKS {
            let label = classify(message).label();
            assert!(
                !label.contains(secret),
                "leaked {secret:?} from {message:?} -> {label}"
            );
        }
    }

    #[test]
    fn an_unparseable_message_reports_a_bare_type() {
        assert_eq!(
            classify("total nonsense with no colon"),
            ErrorClass::Unrecognized {
                kind: "Error".to_string()
            }
        );
    }

    /// A thrown string can look like an engine error. It must not be able to invent a
    /// label or smuggle text through the `kind` field.
    #[test]
    fn a_forged_error_kind_cannot_invent_a_label() {
        let class = classify("PaymentDeclinedFor4471: card ending 4471");
        assert_eq!(
            class,
            ErrorClass::Unrecognized {
                kind: "Error".to_string()
            }
        );
        assert!(!class.label().contains("4471"));
    }

    #[test]
    fn labels_stay_low_cardinality_for_grouping() {
        for message in [
            "run exceeded 30000ms",
            "run exceeded 12000ms",
            "run exceeded 5ms",
        ] {
            assert_eq!(classify(message).label(), "timeout");
        }
    }
}
