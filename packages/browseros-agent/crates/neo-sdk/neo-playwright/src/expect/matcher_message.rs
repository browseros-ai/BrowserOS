//! How Playwright Test prints a retrying matcher, at v1.64.1: the failure message of
//! `formatMatcherMessage` (matchers/matcherHint.ts), the Received lines of its three matcher
//! shapes (`toBeTruthy`, `toMatchText`, `toEqual`) and the call-log lines the server writes
//! while it polls (`Frame.expect`, server/frames.ts). Rust prints all of it, not the prelude,
//! because Rust decides the failure, and the trace records the same text the script sees.
//!
//! ```text
//!  expect(locator).toHaveText(expected) failed      receiver, .not, matcher, its argument
//!
//!  Locator:  locator('#greeting')                   locator matchers only
//!  Expected: "Hello"                                the prelude's line
//!  Received: "Hullo"                                printed here, per matcher shape
//!  Timeout:  5000ms                                 when the op's own timeout passed
//!  Error: element(s) not found                      in place of Received
//!
//!  Call log:                                        OpScope::fail_matcher appends it
//!    - Expect "toHaveText" locator('#greeting') with timeout 5000ms
//! ```
//!
//! One deviation: where upstream prints a jest diff (an array, or text that spans lines), Neo
//! prints the Expected line and a Received line, as it does for one-line text.

use std::borrow::Cow;
use std::time::Duration;

use serde_json::Value;

/// Whose matcher it is: `expect(locator)` prints its locator, `expect(page)` nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Receiver<'a> {
    /// The locator's display, as `Locator.toString()` prints it.
    Locator(&'a str),
    Page,
}

/// How a matcher prints its received value: by the upstream function that prints it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    /// `toBeTruthy`: a state (`hidden`, `attribute not present`), printed bare.
    State,
    /// `toMatchText`: text against a string, a substring or a pattern.
    Text(TextExpected),
    /// `toEqual`: a count, or one value per match.
    Equal,
}

/// What a `toMatchText` matcher's expected value is, which names both of its labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextExpected {
    Exact,
    Substring,
    Pattern,
}

/// The two labels a `toMatchText` failure prints, such as `Expected substring` over
/// `Received string`.
struct TextLabels {
    expected: &'static str,
    received: &'static str,
}

impl TextExpected {
    const fn labels(self) -> TextLabels {
        match self {
            Self::Exact => TextLabels {
                expected: "Expected",
                received: "Received",
            },
            Self::Substring => TextLabels {
                expected: "Expected substring",
                received: "Received string",
            },
            Self::Pattern => TextLabels {
                expected: "Expected pattern",
                received: "Received string",
            },
        }
    }
}

/// What a failure reports under its Expected line.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Reported<'a> {
    /// The value the matcher last compared; `None` is JavaScript's `undefined`.
    Received(Option<&'a Value>),
    /// Upstream's error line in place of Received, after `Error: `.
    Error(&'a str),
}

/// One failed matcher, everything its message prints above the call log.
#[derive(Debug)]
pub(crate) struct MatcherMessage<'a> {
    pub(crate) receiver: Receiver<'a>,
    pub(crate) is_not: bool,
    pub(crate) matcher: &'a str,
    /// The header's parenthesized argument: `expected`, `{ visible: false }` or nothing.
    pub(crate) expectation: &'a str,
    /// Playwright's Expected line, as the prelude printed it.
    pub(crate) expected: Cow<'a, str>,
    pub(crate) shape: Shape,
    pub(crate) reported: Reported<'a>,
    /// Set when the op's own timeout passed: the `Timeout:` line.
    pub(crate) timed_out: Option<Duration>,
}

impl MatcherMessage<'_> {
    /// The message up to its call log, every line ending in a newline.
    pub(crate) fn render(&self) -> String {
        use std::fmt::Write as _;
        let (received, error) = match self.reported {
            Reported::Received(value) => (Some(self.received_line(value)), None),
            Reported::Error(error) => (None, Some(error)),
        };
        // Upstream pads `Locator:` and `Timeout:` to line up with a plain `Expected:` and
        // `Received:`, and not under the longer labels.
        let align = error.is_none()
            && self.expected.starts_with("Expected:")
            && received
                .as_deref()
                .is_none_or(|line| line.starts_with("Received:"));
        let pad = if align { " " } else { "" };
        let receiver = match self.receiver {
            Receiver::Locator(_) => "locator",
            Receiver::Page => "page",
        };
        let not = if self.is_not { ".not" } else { "" };
        let mut message = format!(
            "expect({receiver}){not}.{}({}) failed\n\n",
            self.matcher, self.expectation
        );
        if let Receiver::Locator(display) = self.receiver {
            let _infallible = writeln!(message, "Locator: {pad}{display}");
        }
        let _infallible = writeln!(message, "{}", self.expected);
        if let Some(received) = received {
            let _infallible = writeln!(message, "{received}");
        }
        if let Some(timeout) = self.timed_out.filter(|timeout| !timeout.is_zero()) {
            let _infallible = writeln!(message, "Timeout: {pad}{}ms", timeout.as_millis());
        }
        if let Some(error) = error {
            // A strict violation's text already ends its element list with a newline.
            let _infallible = write!(message, "Error: {error}");
            if !error.ends_with('\n') {
                message.push('\n');
            }
        }
        message
    }

    /// Upstream's Received line for this shape. A failed `.not` matched, so its line says
    /// what matched: the state's own word, or the text with no label padding.
    fn received_line(&self, value: Option<&Value>) -> String {
        match self.shape {
            Shape::State if self.is_not => {
                let word = self
                    .expected
                    .strip_prefix("Expected: not ")
                    .map_or_else(|| as_js_string(value), str::to_owned);
                format!("Received: {word}")
            }
            Shape::State => format!("Received: {}", as_js_string(value)),
            Shape::Text(expected) => {
                let labels = expected.labels();
                // `receivedValue || ''`: a missing attribute (`null`) prints as empty text.
                let text = match value {
                    Some(Value::String(text)) => text.as_str(),
                    _ => "",
                };
                let pad = if self.is_not {
                    0
                } else {
                    labels.expected.len().saturating_sub(labels.received.len())
                };
                format!("{}: {}{}", labels.received, " ".repeat(pad), quoted(text))
            }
            Shape::Equal => format!("Received: {}", printed(value)),
        }
    }
}

/// The call log's first line, as the server titles the call: `Expect "not toBeVisible"
/// locator('#x') with timeout 5000ms`. The script's own `expect(x, message)` is not part of
/// the op, so the title never shows it.
pub(crate) fn title(
    receiver: Receiver<'_>,
    is_not: bool,
    matcher: &str,
    timeout: Option<Duration>,
) -> String {
    let not = if is_not { "not " } else { "" };
    let subtitle = match receiver {
        Receiver::Locator(display) => format!(" {display}"),
        Receiver::Page => String::new(),
    };
    let timeout = timeout.map_or_else(String::new, |timeout| {
        format!(" with timeout {}ms", timeout.as_millis())
    });
    format!("Expect \"{not}{matcher}\"{subtitle}{timeout}")
}

/// The poll's call-log line for a value that did not match, nested under its `locator
/// resolved to` line. Upstream logs none for a list, whose diff the message prints.
pub(crate) fn unexpected_value(value: Option<&Value>) -> Option<String> {
    if let Some(Value::Array(_)) = value {
        return None;
    }
    Some(format!("  unexpected value \"{}\"", as_js_string(value)))
}

/// JavaScript's `String(value)`, as a template literal prints the received value.
fn as_js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_owned(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => as_js_string(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".to_owned(),
        Some(other) => other.to_string(),
    }
}

/// How the matchers print a value (jest's `printReceived`, the prelude's `stringify`):
/// strings quoted, lists as `["a", "b"]`.
fn printed(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_owned(),
        Some(Value::String(text)) => quoted(text),
        Some(Value::Array(items)) => {
            let items: Vec<String> = items.iter().map(|item| printed(Some(item))).collect();
            format!("[{}]", items.join(", "))
        }
        Some(Value::Object(fields)) => {
            let fields: Vec<String> = fields
                .iter()
                .map(|(name, field)| format!("{}: {}", quoted(name), printed(Some(field))))
                .collect();
            format!("{{{}}}", fields.join(", "))
        }
        Some(other) => other.to_string(),
    }
}

/// A string as jest prints one: in double quotes, with `"` and `\` escaped.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        if matches!(c, '"' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::time::Duration;

    use serde_json::{Value, json};

    use super::{MatcherMessage, Receiver, Reported, Shape, TextExpected, title, unexpected_value};
    use crate::call_log::CallLog;

    const TIMEOUT: Option<Duration> = Some(Duration::from_millis(300));

    /// A message as the matcher ran it on `receiver`, failing with `reported` after the
    /// 300 ms timeout the goldens were recorded with.
    fn failed<'a>(
        receiver: Receiver<'a>,
        matcher: &'a str,
        expected: &'a str,
        shape: Shape,
        reported: Reported<'a>,
    ) -> MatcherMessage<'a> {
        MatcherMessage {
            receiver,
            is_not: false,
            matcher,
            expectation: "expected",
            expected: Cow::Borrowed(expected),
            shape,
            reported,
            timed_out: TIMEOUT,
        }
    }

    /// The message and its call log: `lines` once, then `polls` copies of `poll`.
    fn with_log(
        message: &MatcherMessage<'_>,
        lines: &[&str],
        poll: &[&str],
        polls: usize,
    ) -> String {
        let mut log = CallLog::default();
        for line in lines {
            log.push(line);
        }
        for _ in 0..polls {
            for line in poll {
                log.push(line);
            }
        }
        format!("{}{}", message.render(), log.render())
    }

    fn locator_log(matcher: &str, is_not: bool, display: &str) -> [String; 2] {
        [
            title(Receiver::Locator(display), is_not, matcher, TIMEOUT),
            format!("waiting for {display}"),
        ]
    }

    // Each golden below is a real failure's message, recorded from Playwright Test 1.64.1
    // against BrowserOS with a 300 ms timeout and the colors stripped.

    #[test]
    fn tests_that_a_state_that_never_held_prints_as_upstream() {
        let hidden = json!("hidden");
        let message = MatcherMessage {
            expectation: "",
            ..failed(
                Receiver::Locator("locator('#hidden')"),
                "toBeVisible",
                "Expected: visible",
                Shape::State,
                Reported::Received(Some(&hidden)),
            )
        };
        let [first, waiting] = locator_log("toBeVisible", false, "locator('#hidden')");
        let poll = [
            "  locator resolved to <div id=\"hidden\">secret</div>",
            "  unexpected value \"hidden\"",
        ];
        assert_eq!(
            with_log(&message, &[&first, &waiting], &poll, 6),
            "expect(locator).toBeVisible() failed\n\nLocator:  locator('#hidden')\nExpected: visible\nReceived: hidden\nTimeout:  300ms\n\nCall log:\n  - Expect \"toBeVisible\" locator('#hidden') with timeout 300ms\n  - waiting for locator('#hidden')\n    6 × locator resolved to <div id=\"hidden\">secret</div>\n      - unexpected value \"hidden\"\n"
        );
    }

    #[test]
    fn tests_that_a_missing_element_prints_its_error_unaligned() {
        let message = MatcherMessage {
            expectation: "",
            ..failed(
                Receiver::Locator("locator('#nope')"),
                "toBeVisible",
                "Expected: visible",
                Shape::State,
                Reported::Error("element(s) not found"),
            )
        };
        let [first, waiting] = locator_log("toBeVisible", false, "locator('#nope')");
        assert_eq!(
            with_log(&message, &[&first, &waiting], &[], 0),
            "expect(locator).toBeVisible() failed\n\nLocator: locator('#nope')\nExpected: visible\nTimeout: 300ms\nError: element(s) not found\n\nCall log:\n  - Expect \"toBeVisible\" locator('#nope') with timeout 300ms\n  - waiting for locator('#nope')\n"
        );
    }

    #[test]
    fn tests_that_a_failed_not_prints_what_matched() {
        let visible = json!("visible");
        let message = MatcherMessage {
            is_not: true,
            expectation: "",
            ..failed(
                Receiver::Locator("locator('#greeting')"),
                "toBeVisible",
                "Expected: not visible",
                Shape::State,
                Reported::Received(Some(&visible)),
            )
        };
        assert_eq!(
            message.render(),
            "expect(locator).not.toBeVisible() failed\n\nLocator:  locator('#greeting')\nExpected: not visible\nReceived: visible\nTimeout:  300ms\n"
        );
        assert_eq!(
            title(
                Receiver::Locator("locator('#greeting')"),
                true,
                "toBeVisible",
                TIMEOUT
            ),
            "Expect \"not toBeVisible\" locator('#greeting') with timeout 300ms"
        );

        let hullo = json!("Hullo");
        let substring = MatcherMessage {
            is_not: true,
            ..failed(
                Receiver::Locator("locator('#greeting')"),
                "toContainText",
                "Expected substring: not \"ull\"",
                Shape::Text(TextExpected::Substring),
                Reported::Received(Some(&hullo)),
            )
        };
        assert_eq!(
            substring.render(),
            "expect(locator).not.toContainText(expected) failed\n\nLocator: locator('#greeting')\nExpected substring: not \"ull\"\nReceived string: \"Hullo\"\nTimeout: 300ms\n"
        );
    }

    #[test]
    fn tests_that_text_labels_line_up_as_upstreams() {
        let hullo = json!("Hullo");
        let display = Receiver::Locator("locator('#greeting')");
        let [first, waiting] = locator_log("toHaveText", false, "locator('#greeting')");
        let poll = [
            "  locator resolved to <h1 id=\"greeting\">Hullo</h1>",
            "  unexpected value \"Hullo\"",
        ];
        let exact = failed(
            display,
            "toHaveText",
            "Expected: \"Hello\"",
            Shape::Text(TextExpected::Exact),
            Reported::Received(Some(&hullo)),
        );
        assert_eq!(
            with_log(&exact, &[&first, &waiting], &poll, 7),
            "expect(locator).toHaveText(expected) failed\n\nLocator:  locator('#greeting')\nExpected: \"Hello\"\nReceived: \"Hullo\"\nTimeout:  300ms\n\nCall log:\n  - Expect \"toHaveText\" locator('#greeting') with timeout 300ms\n  - waiting for locator('#greeting')\n    7 × locator resolved to <h1 id=\"greeting\">Hullo</h1>\n      - unexpected value \"Hullo\"\n"
        );
        let pattern = failed(
            display,
            "toHaveText",
            "Expected pattern: /Hel+o/",
            Shape::Text(TextExpected::Pattern),
            Reported::Received(Some(&hullo)),
        );
        assert_eq!(
            pattern.render(),
            "expect(locator).toHaveText(expected) failed\n\nLocator: locator('#greeting')\nExpected pattern: /Hel+o/\nReceived string:  \"Hullo\"\nTimeout: 300ms\n"
        );
        let substring = failed(
            display,
            "toContainText",
            "Expected substring: \"ell\"",
            Shape::Text(TextExpected::Substring),
            Reported::Received(Some(&hullo)),
        );
        assert_eq!(
            substring.render(),
            "expect(locator).toContainText(expected) failed\n\nLocator: locator('#greeting')\nExpected substring: \"ell\"\nReceived string:    \"Hullo\"\nTimeout: 300ms\n"
        );
    }

    #[test]
    fn tests_that_an_absent_attribute_prints_empty_and_logs_null() {
        let null = Value::Null;
        let message = failed(
            Receiver::Locator("locator('#name')"),
            "toHaveAttribute",
            "Expected: \"y\"",
            Shape::Text(TextExpected::Exact),
            Reported::Received(Some(&null)),
        );
        assert_eq!(
            message.render(),
            "expect(locator).toHaveAttribute(expected) failed\n\nLocator:  locator('#name')\nExpected: \"y\"\nReceived: \"\"\nTimeout:  300ms\n"
        );
        assert_eq!(
            unexpected_value(Some(&null)).as_deref(),
            Some("  unexpected value \"null\"")
        );
        assert_eq!(
            unexpected_value(None).as_deref(),
            Some("  unexpected value \"undefined\"")
        );
    }

    #[test]
    fn tests_that_counts_and_lists_print_as_values() {
        let three = json!(3);
        let count = failed(
            Receiver::Locator("locator('li')"),
            "toHaveCount",
            "Expected: 2",
            Shape::Equal,
            Reported::Received(Some(&three)),
        );
        let [first, waiting] = locator_log("toHaveCount", false, "locator('li')");
        let poll = [
            "  locator resolved to 3 elements",
            "  unexpected value \"3\"",
        ];
        assert_eq!(
            with_log(&count, &[&first, &waiting], &poll, 7),
            "expect(locator).toHaveCount(expected) failed\n\nLocator:  locator('li')\nExpected: 2\nReceived: 3\nTimeout:  300ms\n\nCall log:\n  - Expect \"toHaveCount\" locator('li') with timeout 300ms\n  - waiting for locator('li')\n    7 × locator resolved to 3 elements\n      - unexpected value \"3\"\n"
        );
        // Upstream prints a jest diff here; Neo prints the two lines.
        let texts = json!(["one", "two", "three"]);
        let list = failed(
            Receiver::Locator("locator('li')"),
            "toHaveText",
            "Expected: [\"one\", \"two\"]",
            Shape::Equal,
            Reported::Received(Some(&texts)),
        );
        assert_eq!(
            list.render(),
            "expect(locator).toHaveText(expected) failed\n\nLocator:  locator('li')\nExpected: [\"one\", \"two\"]\nReceived: [\"one\", \"two\", \"three\"]\nTimeout:  300ms\n"
        );
        assert_eq!(
            unexpected_value(Some(&texts)),
            None,
            "upstream logs no unexpected value for a list"
        );
    }

    #[test]
    fn tests_that_a_thrown_error_prints_without_received_or_timeout() {
        let violation = "strict mode violation: locator('li') resolved to 3 elements:\n    1) <li class=\"a b\">one</li> aka getByText('one', { exact: true })\n    2) <li>two</li> aka getByText('two', { exact: true })\n    3) <li>three</li> aka getByText('three')\n";
        let message = MatcherMessage {
            timed_out: None,
            ..failed(
                Receiver::Locator("locator('li')"),
                "toHaveText",
                "Expected: \"one\"",
                Shape::Text(TextExpected::Exact),
                Reported::Error(violation),
            )
        };
        let [first, waiting] = locator_log("toHaveText", false, "locator('li')");
        assert_eq!(
            with_log(&message, &[&first, &waiting], &[], 0),
            "expect(locator).toHaveText(expected) failed\n\nLocator: locator('li')\nExpected: \"one\"\nError: strict mode violation: locator('li') resolved to 3 elements:\n    1) <li class=\"a b\">one</li> aka getByText('one', { exact: true })\n    2) <li>two</li> aka getByText('two', { exact: true })\n    3) <li>three</li> aka getByText('three')\n\nCall log:\n  - Expect \"toHaveText\" locator('li') with timeout 300ms\n  - waiting for locator('li')\n"
        );
    }

    #[test]
    fn tests_that_a_page_matcher_prints_no_locator() {
        let received = json!("Expect page");
        let message = failed(
            Receiver::Page,
            "toHaveTitle",
            "Expected: \"Nope\"",
            Shape::Text(TextExpected::Exact),
            Reported::Received(Some(&received)),
        );
        let first = title(Receiver::Page, false, "toHaveTitle", TIMEOUT);
        let poll = [
            "  locator resolved to <html>…</html>",
            "  unexpected value \"Expect page\"",
        ];
        assert_eq!(
            with_log(&message, &[&first], &poll, 7),
            "expect(page).toHaveTitle(expected) failed\n\nExpected: \"Nope\"\nReceived: \"Expect page\"\nTimeout:  300ms\n\nCall log:\n  - Expect \"toHaveTitle\" with timeout 300ms\n    7 × locator resolved to <html>…</html>\n      - unexpected value \"Expect page\"\n"
        );
        assert_eq!(
            title(Receiver::Page, false, "toHaveURL", None),
            "Expect \"toHaveURL\"",
            "timeout 0 prints no timeout, as upstream"
        );
    }
}
