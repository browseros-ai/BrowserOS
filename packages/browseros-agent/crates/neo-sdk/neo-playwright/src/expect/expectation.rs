//! One retrying matcher: what Rust reads from the prelude's parameters, upstream's rules for
//! each expression, and the loop that turns probe polls into a pass or Playwright's failure.
//!
//! The loop is upstream's `Frame.expect` (server/frames.ts at v1.64.1): one poll at once, then
//! the outer backoff (0, 20, 50, 100, 100, 500 ms), until the matcher's result differs from
//! `.not` or the op's timeout passes. The probe matches with the engine's own checks; Rust
//! owns the timing, the codes and the message, so the trace and the script's error agree.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::{Value, json};
use tokio::time;

use crate::backoff::Backoff;
use crate::expect::matcher_message::{
    self, MatcherMessage, Receiver, Reported, Shape, TextExpected,
};
use crate::op_scope::OpScope;
use crate::page::site_sweep::SiteValues;
use crate::probe::{ErrorClass, ExpectParams, ProbeReply, ProbeRequest, ProbeTarget};
use crate::target::Strictness;
use crate::{ErrorCode, PlaywrightError, resolve};

/// The least time the first poll gets, whatever the script's timeout: a poll takes a few
/// milliseconds, and an install or a frame hop on the way can take a few hundred.
const ONE_SHOT_FLOOR: Duration = Duration::from_secs(1);

/// An upstream expect expression (`FrameExpectParams.expression`), as matchers.ts sends it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub(crate) enum Expression {
    #[serde(rename = "to.be.attached")]
    Attached,
    #[serde(rename = "to.be.detached")]
    Detached,
    #[serde(rename = "to.be.checked")]
    Checked,
    #[serde(rename = "to.be.disabled")]
    Disabled,
    #[serde(rename = "to.be.editable")]
    Editable,
    #[serde(rename = "to.be.readonly")]
    Readonly,
    #[serde(rename = "to.be.empty")]
    Empty,
    #[serde(rename = "to.be.enabled")]
    Enabled,
    #[serde(rename = "to.be.focused")]
    Focused,
    #[serde(rename = "to.be.hidden")]
    Hidden,
    #[serde(rename = "to.be.visible")]
    Visible,
    #[serde(rename = "to.be.in.viewport")]
    InViewport,
    #[serde(rename = "to.have.attribute")]
    HasAttribute,
    #[serde(rename = "to.have.attribute.value")]
    AttributeValue,
    #[serde(rename = "to.have.text")]
    Text,
    #[serde(rename = "to.have.accessible.name")]
    AccessibleName,
    #[serde(rename = "to.have.accessible.description")]
    AccessibleDescription,
    #[serde(rename = "to.have.accessible.error.message")]
    AccessibleErrorMessage,
    #[serde(rename = "to.have.class")]
    Class,
    #[serde(rename = "to.contain.class")]
    ContainClass,
    #[serde(rename = "to.have.css")]
    Css,
    #[serde(rename = "to.have.id")]
    Id,
    #[serde(rename = "to.have.role")]
    Role,
    #[serde(rename = "to.have.value")]
    Value,
    #[serde(rename = "to.have.values")]
    Values,
    #[serde(rename = "to.have.count")]
    Count,
    #[serde(rename = "to.have.text.array")]
    TextArray,
    #[serde(rename = "to.contain.text.array")]
    ContainTextArray,
    #[serde(rename = "to.have.class.array")]
    ClassArray,
    #[serde(rename = "to.contain.class.array")]
    ContainClassArray,
    #[serde(rename = "to.have.property")]
    Property,
    #[serde(rename = "to.have.title")]
    Title,
    #[serde(rename = "to.have.url")]
    Url,
}

impl Expression {
    /// How the failure prints its received value: the upstream matcher behind it.
    const fn shape(self, text: TextExpected) -> Shape {
        use Expression as E;
        match self {
            E::Attached
            | E::Detached
            | E::Checked
            | E::Disabled
            | E::Editable
            | E::Readonly
            | E::Empty
            | E::Enabled
            | E::Focused
            | E::Hidden
            | E::Visible
            | E::InViewport
            | E::HasAttribute => Shape::State,
            E::AttributeValue
            | E::Text
            | E::AccessibleName
            | E::AccessibleDescription
            | E::AccessibleErrorMessage
            | E::Class
            | E::ContainClass
            | E::Css
            | E::Id
            | E::Role
            | E::Value
            | E::Title
            | E::Url => Shape::Text(text),
            E::Values
            | E::Count
            | E::TextArray
            | E::ContainTextArray
            | E::ClassArray
            | E::ContainClassArray
            | E::Property => Shape::Equal,
        }
    }

    /// Count and the array forms read every match, with no strict check, and zero matches
    /// is a value for them (upstream's `isArray`).
    const fn every_match(self) -> bool {
        matches!(
            self,
            Self::Count
                | Self::TextArray
                | Self::ContainTextArray
                | Self::ClassArray
                | Self::ContainClassArray
        )
    }

    /// Whether no element at all is the answer, as upstream's `_expectInternal` decides when
    /// nothing matched: a hidden or detached element, or `.not` of visible, attached or in
    /// the viewport. Every other single-element matcher fails with `element(s) not found`.
    const fn passes_when_missing(self, is_not: bool) -> bool {
        matches!(
            (self, is_not),
            (Self::Hidden | Self::Detached, false)
                | (Self::Visible | Self::Attached | Self::InViewport, true)
        )
    }

    /// The page matchers, which run on the main frame's document.
    const fn on_page(self) -> bool {
        matches!(self, Self::Title | Self::Url)
    }
}

/// The parameters as Rust reads them; the probe receives the prelude's object unchanged. The
/// expected strings may be a password the script compares (`toHaveValue`), so they are
/// skipped, never held: Rust needs only what kind of value each is.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct MatcherParams {
    expression: Expression,
    is_not: bool,
    #[serde(default)]
    expected_text: Vec<ExpectedText>,
    #[serde(default)]
    expected_number: Option<f64>,
    #[serde(default, rename = "expressionArg")]
    _expression_arg: Option<IgnoredAny>,
    #[serde(default, rename = "pseudo")]
    _pseudo: Option<IgnoredAny>,
    #[serde(default, rename = "expectedValue")]
    _expected_value: Option<IgnoredAny>,
    #[serde(default, rename = "useInnerText")]
    _use_inner_text: Option<bool>,
}

/// Upstream's `ExpectedTextValue`, without its string.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ExpectedText {
    #[serde(default, rename = "string")]
    _string: Option<IgnoredAny>,
    #[serde(default)]
    regex_source: Option<IgnoredAny>,
    #[serde(default, rename = "regexFlags")]
    _regex_flags: Option<IgnoredAny>,
    #[serde(default)]
    match_substring: Option<bool>,
    #[serde(default, rename = "ignoreCase")]
    _ignore_case: Option<bool>,
    #[serde(default, rename = "normalizeWhiteSpace")]
    _normalize_white_space: Option<bool>,
}

/// What the script expects, as the prelude sent it after the op's receiver.
pub(crate) struct MatcherArgs {
    /// Playwright's matcher name, `toHaveText`.
    pub(crate) matcher: String,
    /// The failure header's argument: `expected`, `{ visible: false }`, or nothing.
    pub(crate) expectation: String,
    /// Playwright's Expected line, `Expected: "Hello"`.
    pub(crate) expected: String,
    pub(crate) params: ExpectParams,
}

/// What the matcher runs on.
pub(crate) enum Subject {
    /// `expect(locator)`, with the locator's display.
    Locator(String),
    /// `expect(page)`.
    Page,
}

/// What one poll saw.
#[derive(Debug)]
pub(crate) enum Poll {
    /// The matcher ran on what the locator resolved to.
    Ran {
        /// The matcher's own result, before `.not`.
        matches: bool,
        /// What the locator resolved to, for the call log, masked with the element's site.
        resolved: String,
        /// The compared value, masked by the seal and with the element's site; `None` is
        /// `undefined`.
        received: Option<Value>,
    },
    /// No element matched (or a frame on the way is not there yet).
    Missing,
}

/// One retrying matcher, decoded and checked before anything is sent. It holds the
/// script's Expected line, which may name a secret, so it has no `Debug`.
pub(crate) struct Expectation {
    subject: Subject,
    args: MatcherArgs,
    expression: Expression,
    is_not: bool,
    shape: Shape,
    /// What zero matches answers for a matcher of every match: count 0, or an empty list
    /// matching nothing expected.
    nothing_matches: bool,
    /// A poll compared a password, card or one-time-code field: the op's target is
    /// sensitive, and the message hides the expected value.
    sensitive: AtomicBool,
}

impl Expectation {
    /// Reads the prelude's parameters for `subject`. A matcher Neo does not run is refused
    /// here, before anything reaches the page.
    pub(crate) fn new(
        subject: Subject,
        args: MatcherArgs,
        scope: &OpScope<'_>,
    ) -> Result<Self, PlaywrightError> {
        let raw = Value::Object(args.params.0.clone());
        // The decode error is left out: serde quotes the value it could not read, which may
        // be the expected password.
        let params: MatcherParams = serde_path_to_error::deserialize(raw).map_err(|error| {
            scope.fail(
                ErrorCode::InvalidInput,
                &format!(
                    "params.{} is not what Playwright's matchers send",
                    error.path()
                ),
            )
        })?;
        let expression = params.expression;
        if expression == Expression::Property {
            return Err(scope.fail(
                ErrorCode::Unsupported,
                "toHaveJSProperty is not supported: Neo matches in a sealed world that does \
                 not see the page's own JavaScript objects; use expect.poll(() => \
                 locator.evaluate((element) => element.name)).toEqual(value) instead",
            ));
        }
        if expression.on_page() != matches!(subject, Subject::Page) {
            return Err(scope.fail(
                ErrorCode::InvalidInput,
                &format!("{} is not a matcher of this receiver", args.matcher),
            ));
        }
        let text = match params.expected_text.first() {
            Some(first) if first.regex_source.is_some() => TextExpected::Pattern,
            Some(first) if first.match_substring == Some(true) => TextExpected::Substring,
            Some(_) | None => TextExpected::Exact,
        };
        let nothing_matches = match expression {
            Expression::Count => params.expected_number == Some(0.0),
            _ => params.expected_text.is_empty(),
        };
        Ok(Self {
            subject,
            args,
            expression,
            is_not: params.is_not,
            shape: expression.shape(text),
            nothing_matches,
            sensitive: AtomicBool::new(false),
        })
    }

    /// Locator matchers are strict, as upstream's `callOnSelector({ strict: !isArray })`.
    pub(crate) fn strictness(&self) -> Strictness {
        if self.expression.every_match() {
            Strictness::FirstMatch
        } else {
            Strictness::Strict
        }
    }

    /// The probe request for one poll on `target`.
    pub(crate) fn request(&self, target: ProbeTarget, frame_visible: bool) -> ProbeRequest {
        ProbeRequest::Expect {
            target,
            strictness: self.strictness(),
            frame_visible,
            params: self.args.params.clone(),
        }
    }

    /// Polls until the matcher's result differs from `.not`. `poll` runs one probe call and
    /// maps its reply with [`Expectation::poll_of`]; it fails only for what ends the op at
    /// once (a strict violation, an error the engine threw, a stale ref, a gone page).
    pub(crate) async fn run<F, Fut>(
        &self,
        scope: &OpScope<'_>,
        mut poll: F,
    ) -> Result<Value, PlaywrightError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<Poll, PlaywrightError>>,
    {
        let receiver = self.receiver();
        scope.log(matcher_message::title(
            receiver,
            self.is_not,
            &self.args.matcher,
            scope.timeout(),
        ));
        if let Receiver::Locator(display) = receiver {
            scope.log(format!("waiting for {display}"));
        }
        // Upstream's one-shot finishes even past the timeout, so `toBeVisible({ timeout: 1 })`
        // passes on a visible element, and upstream drops its call-log lines. It gets at least
        // ONE_SHOT_FLOOR, not forever: a check that never answers (an IntersectionObserver in
        // a tab that does not render) must still end at the script's timeout.
        let first = match scope.timeout() {
            Some(timeout) => time::timeout(timeout.max(ONE_SHOT_FLOOR), poll())
                .await
                .ok(),
            None => Some(poll().await),
        };
        let mut last = match first {
            Some(polled) => Some(polled?),
            None => None,
        };
        if last.as_ref().is_some_and(|poll| self.passes(poll)) {
            return Ok(Value::Null);
        }
        let backoff = Backoff::outer(scope.timeout());
        for attempt in 0.. {
            if scope.sleep(backoff.wait(attempt)).await.is_err() {
                break;
            }
            let Ok(polled) = scope.within(poll()).await else {
                break;
            };
            let polled = polled?;
            self.log(scope, &polled);
            if self.passes(&polled) {
                return Ok(Value::Null);
            }
            last = Some(polled);
        }
        Err(self.timed_out(scope, last.as_ref()))
    }

    /// One probe reply as a poll, or the failure it ends the op with. What the locator
    /// resolved to, the value compared and a strict violation's previews are page text, masked
    /// once with the values of the element's site, which `site` gives from the sweep taken with
    /// the poll, as a read's (`SiteSweep::values`). It is asked only for a reply that holds
    /// such text, and its failure (a frame of the site that missed its budget) fails the
    /// matcher closed.
    pub(crate) fn poll_of(
        &self,
        scope: &OpScope<'_>,
        reply: ProbeReply,
        site: impl FnOnce() -> Result<SiteValues, PlaywrightError>,
    ) -> Result<Poll, PlaywrightError> {
        match reply {
            ProbeReply::Expect {
                matches,
                resolved,
                received,
                sensitive,
            } => {
                if sensitive.is_some() {
                    self.sensitive.store(true, Ordering::Relaxed);
                }
                let values = site()?;
                Ok(Poll::Ran {
                    matches,
                    resolved: values.mask_line(resolved),
                    received: received.map(|received| values.mask(received)),
                })
            }
            ProbeReply::None { stale_ref: None } | ProbeReply::Detached => Ok(self.nothing_found()),
            // Upstream throws these from the page, so they end the matcher at once and print
            // as its `Error:` line.
            ProbeReply::Strict { message } => {
                let message = site()?.mask_text(message);
                Err(self.thrown(scope, ErrorCode::StrictModeViolation, &message))
            }
            ProbeReply::Error {
                message,
                class: ErrorClass::Script | ErrorClass::Selector,
            } => Err(self.thrown(scope, ErrorCode::InvalidInput, &message)),
            // The probe refuses only a RegExp, or a case-insensitive match, against a sensitive
            // field (expect.ts), so the script's pattern is hidden like its string would be.
            ProbeReply::Error {
                message,
                class: ErrorClass::Unsupported,
            } => {
                self.sensitive.store(true, Ordering::Relaxed);
                Err(self.thrown(scope, ErrorCode::Unsupported, &message))
            }
            other => Err(resolve::refused(scope, other)),
        }
    }

    /// What a poll answers when nothing matched, or a frame on the way is not there yet. For
    /// count and the array forms that is zero matches, which the probe would have checked;
    /// upstream answers it the same way when its frame is missing.
    pub(crate) fn nothing_found(&self) -> Poll {
        if !self.expression.every_match() {
            return Poll::Missing;
        }
        let received = if self.expression == Expression::Count {
            json!(0)
        } else {
            json!([])
        };
        Poll::Ran {
            matches: self.nothing_matches,
            resolved: "0 elements".to_owned(),
            received: Some(received),
        }
    }

    fn receiver(&self) -> Receiver<'_> {
        match &self.subject {
            Subject::Locator(display) => Receiver::Locator(display),
            Subject::Page => Receiver::Page,
        }
    }

    fn passes(&self, poll: &Poll) -> bool {
        match poll {
            Poll::Ran { matches, .. } => *matches != self.is_not,
            Poll::Missing => self.expression.passes_when_missing(self.is_not),
        }
    }

    /// Upstream's lines for one poll after the one-shot.
    fn log(&self, scope: &OpScope<'_>, poll: &Poll) {
        let Poll::Ran {
            resolved, received, ..
        } = poll
        else {
            return;
        };
        scope.log(format!("  locator resolved to {resolved}"));
        if !self.passes(poll)
            && let Some(line) = matcher_message::unexpected_value(received.as_ref())
        {
            scope.log(line);
        }
    }

    /// The op's timeout passed with the last poll failing. An element that matched means the
    /// website did not reach the state; nothing matched means the script's locator found
    /// nothing. Count and the array forms always read a value, so theirs is the website's. A
    /// page that never answered a poll is the website's too, printed as upstream prints a
    /// matcher with no result (`Received: undefined`).
    fn timed_out(&self, scope: &OpScope<'_>, last: Option<&Poll>) -> PlaywrightError {
        let (code, reported) = match last {
            Some(Poll::Missing) => (ErrorCode::NoMatch, Reported::Error("element(s) not found")),
            Some(Poll::Ran { received, .. }) => (
                ErrorCode::AssertionFailed,
                Reported::Received(received.as_ref()),
            ),
            None => (ErrorCode::AssertionFailed, Reported::Received(None)),
        };
        scope.fail_matcher(code, &self.message(reported, scope.timeout()))
    }

    /// A failure that ends the matcher before its timeout, printed as its `Error:` line.
    fn thrown(&self, scope: &OpScope<'_>, code: ErrorCode, error: &str) -> PlaywrightError {
        scope.fail_matcher(code, &self.message(Reported::Error(error), None))
    }

    fn message(&self, reported: Reported<'_>, timed_out: Option<Duration>) -> String {
        let expected = if self.sensitive.load(Ordering::Relaxed) {
            matcher_message::hidden_expected(&self.args.expected).into()
        } else {
            self.args.expected.as_str().into()
        };
        MatcherMessage {
            receiver: self.receiver(),
            is_not: self.is_not,
            matcher: &self.args.matcher,
            expectation: &self.args.expectation,
            expected,
            shape: self.shape,
            reported,
            timed_out,
        }
        .render()
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::future;
    use std::time::Duration;

    use browseros_core::sensitive_fields::FieldValues;
    use serde_json::{Map, Value, json};
    use tokio::time::{self, Instant};

    use super::{Expectation, Expression, MatcherArgs, ONE_SHOT_FLOOR, Poll, Subject};
    use crate::op::Op;
    use crate::op_scope::OpScope;
    use crate::page::site_sweep::SiteValues;
    use crate::probe::{ExpectParams, ProbeReply};
    use crate::target::{CallHeader, Strictness};
    use crate::test_env::TestEnv;
    use crate::{ErrorCode, PlaywrightError};

    fn locator_scope(env: &TestEnv) -> Result<OpScope<'_>, Box<dyn Error>> {
        let op = Op::from_name("expect.locator").ok_or("expect.locator is an op")?;
        Ok(OpScope::new(
            env,
            op,
            CallHeader {
                api: Some("expect.toHaveValue".to_owned()),
                timeout_ms: Some(5000),
            },
        ))
    }

    /// A site whose sweep found no sensitive field.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the shape of the values a poll's site sweep gives"
    )]
    fn unswept() -> Result<SiteValues, PlaywrightError> {
        Ok(SiteValues::found(FieldValues::default()))
    }

    /// `matcher` on `locator('#pw')`, with the prelude's `params`.
    fn expectation(
        scope: &OpScope<'_>,
        matcher: &str,
        params: Value,
    ) -> Result<Expectation, PlaywrightError> {
        let params = match params {
            Value::Object(params) => params,
            _ => Map::new(),
        };
        let args = MatcherArgs {
            matcher: matcher.to_owned(),
            expectation: "expected".to_owned(),
            expected: "Expected: \"hunter2\"".to_owned(),
            params: ExpectParams(params),
        };
        Expectation::new(Subject::Locator("locator('#pw')".to_owned()), args, scope)
    }

    #[test]
    fn tests_that_every_expression_the_prelude_sends_decodes() -> Result<(), Box<dyn Error>> {
        let prelude = include_str!("../../../neo-playwright-js/src/prelude/expect-retrying.ts");
        let mut seen = 0;
        for (start, _) in prelude.match_indices("'to.") {
            let quoted = prelude
                .get(start + 1..)
                .and_then(|rest| rest.split('\'').next())
                .unwrap_or_default();
            serde_json::from_value::<Expression>(json!(quoted))
                .map_err(|error| format!("{quoted}: {error}"))?;
            seen += 1;
        }
        assert!(seen >= 30, "found the prelude's expressions: {seen}");
        Ok(())
    }

    #[test]
    fn tests_that_missing_elements_answer_as_upstream() {
        let passing: Vec<(Expression, bool)> = [
            Expression::Hidden,
            Expression::Detached,
            Expression::Visible,
            Expression::Attached,
            Expression::InViewport,
            Expression::Text,
        ]
        .into_iter()
        .flat_map(|expression| [(expression, false), (expression, true)])
        .filter(|(expression, is_not)| expression.passes_when_missing(*is_not))
        .collect();
        assert_eq!(
            passing,
            [
                (Expression::Hidden, false),
                (Expression::Detached, false),
                (Expression::Visible, true),
                (Expression::Attached, true),
                (Expression::InViewport, true),
            ]
        );
    }

    #[tokio::test]
    async fn tests_that_matchers_neo_does_not_run_are_refused_before_polling()
    -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let scope = locator_scope(&env)?;
        let property = expectation(
            &scope,
            "toHaveJSProperty",
            json!({ "expression": "to.have.property", "expressionArg": "value", "expectedValue": 1, "isNot": false }),
        )
        .err()
        .ok_or("toHaveJSProperty is refused")?;
        assert_eq!(property.error_code(), Some(ErrorCode::Unsupported));
        let title = expectation(
            &scope,
            "toHaveTitle",
            json!({ "expression": "to.have.title", "expectedText": [{ "string": "x" }], "isNot": false }),
        )
        .err()
        .ok_or("a page matcher on a locator is refused")?;
        assert_eq!(title.error_code(), Some(ErrorCode::InvalidInput));
        let unknown = expectation(
            &scope,
            "toHaveValue",
            json!({ "expression": "to.have.value", "expectedText": [{ "string": "hunter2", "fuzzy": true }], "isNot": false }),
        )
        .err()
        .ok_or("an unknown parameter is refused")?;
        assert_eq!(unknown.error_code(), Some(ErrorCode::InvalidInput));
        assert!(
            !unknown.message().contains("hunter2"),
            "the decode error never quotes the expected value: {unknown}"
        );
        assert!(env.calls().is_empty(), "nothing reached the host");
        Ok(())
    }

    fn visible(scope: &OpScope<'_>) -> Result<Expectation, PlaywrightError> {
        expectation(
            scope,
            "toBeVisible",
            json!({ "expression": "to.be.visible", "isNot": false }),
        )
    }

    fn scope_with_timeout(env: &TestEnv, timeout_ms: u64) -> Result<OpScope<'_>, Box<dyn Error>> {
        let op = Op::from_name("expect.locator").ok_or("expect.locator is an op")?;
        Ok(OpScope::new(
            env,
            op,
            CallHeader {
                api: Some("expect.toBeVisible".to_owned()),
                timeout_ms: Some(timeout_ms),
            },
        ))
    }

    /// Upstream's one-shot finishes past a 1 ms timeout, so a slower first poll still passes.
    #[tokio::test(start_paused = true)]
    async fn tests_that_the_first_poll_outlives_a_short_timeout() -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let scope = scope_with_timeout(&env, 1)?;
        let expect = visible(&scope)?;
        let passed = expect
            .run(&scope, || async {
                time::sleep(Duration::from_millis(50)).await;
                Ok(Poll::Ran {
                    matches: true,
                    resolved: "<button>Go</button>".to_owned(),
                    received: Some(json!("visible")),
                })
            })
            .await?;
        assert_eq!(passed, Value::Null);
        Ok(())
    }

    /// A check that never answers still ends at the script's timeout, never at the run's.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_first_poll_that_never_answers_still_times_out()
    -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let scope = scope_with_timeout(&env, 300)?;
        let expect = visible(&scope)?;
        let started = Instant::now();
        let error = expect
            .run(&scope, future::pending::<Result<Poll, PlaywrightError>>)
            .await
            .err()
            .ok_or("a poll that never answers cannot pass")?;
        assert_eq!(
            started.elapsed(),
            ONE_SHOT_FLOOR,
            "the first poll gets its floor"
        );
        assert_eq!(error.error_code(), Some(ErrorCode::AssertionFailed));
        assert!(
            error
                .message()
                .contains("\nReceived: undefined\nTimeout:  300ms\n"),
            "{error}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_count_and_arrays_read_every_match_and_zero_is_a_value()
    -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let scope = locator_scope(&env)?;
        let count = expectation(
            &scope,
            "toHaveCount",
            json!({ "expression": "to.have.count", "expectedNumber": 0, "isNot": false }),
        )?;
        assert_eq!(count.strictness(), Strictness::FirstMatch);
        let Poll::Ran {
            matches, received, ..
        } = count.nothing_found()
        else {
            return Err("zero matches is a value for count".into());
        };
        assert!(matches, "toHaveCount(0) holds with nothing there");
        assert_eq!(received, Some(json!(0)));
        let texts = expectation(
            &scope,
            "toHaveText",
            json!({ "expression": "to.have.text.array", "expectedText": [{ "string": "a" }], "isNot": false }),
        )?;
        assert!(matches!(
            texts.nothing_found(),
            Poll::Ran {
                matches: false,
                received: Some(Value::Array(_)),
                ..
            }
        ));
        let single = expectation(
            &scope,
            "toHaveText",
            json!({ "expression": "to.have.text", "expectedText": [{ "string": "a" }], "isNot": false }),
        )?;
        assert_eq!(single.strictness(), Strictness::Strict);
        assert!(matches!(single.nothing_found(), Poll::Missing));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_sensitive_comparison_hides_the_expected_value()
    -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let scope = locator_scope(&env)?;
        let value = expectation(
            &scope,
            "toHaveValue",
            json!({ "expression": "to.have.value", "expectedText": [{ "string": "hunter2" }], "isNot": false }),
        )?;
        let reply: ProbeReply = serde_json::from_value(json!({
            "kind": "expect", "matches": false, "resolved": "<input type=\"password\" id=\"pw\"/>",
            "received": "••••", "sensitive": "password",
        }))?;
        let poll = value.poll_of(&scope, reply, unswept)?;
        assert!(env.calls().is_empty(), "reads do not report effects");
        let error = value.timed_out(&scope, Some(&poll));
        assert_eq!(error.error_code(), Some(ErrorCode::AssertionFailed));
        assert!(
            error
                .message()
                .contains("Expected: \"••••\"\nReceived: \"••••\"\n"),
            "{error}"
        );
        assert!(!error.message().contains("hunter2"), "{error}");

        let refused = value
            .poll_of(
                &scope,
                serde_json::from_value(
                    json!({ "kind": "error", "class": "unsupported", "message": "no RegExp here" }),
                )?,
                unswept,
            )
            .err()
            .ok_or("the probe's refusal ends the op")?;
        assert_eq!(refused.error_code(), Some(ErrorCode::Unsupported));
        assert!(
            refused.message().contains("\nError: no RegExp here\n"),
            "{refused}"
        );
        assert!(env.calls().is_empty(), "reads do not report effects");
        Ok(())
    }

    /// A value copied from a frame of the element's site reaches the call log, the message and
    /// a strict violation's previews masked, though the poll's own frame holds no such field.
    #[tokio::test]
    async fn tests_that_a_poll_masks_its_page_text_with_the_values_of_its_site()
    -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let scope = locator_scope(&env)?;
        let text = expectation(
            &scope,
            "toHaveText",
            json!({ "expression": "to.have.text", "expectedText": [{ "string": "Signed out" }], "isNot": false }),
        )?;
        let site = || {
            Ok(SiteValues::found(FieldValues::from_sweep(&json!([
                "copied-secret-71"
            ]))))
        };
        let reply: ProbeReply = serde_json::from_value(json!({
            "kind": "expect", "matches": false,
            "resolved": "<h1 data-who=\"copied-secret-71\">Signed in with copied-secret-71</h1>",
            "received": "Signed in with copied-secret-71", "sensitive": null,
        }))?;
        let poll = text.poll_of(&scope, reply, site)?;
        let Poll::Ran {
            resolved, received, ..
        } = &poll
        else {
            return Err("an expect reply is a poll that ran".into());
        };
        assert_eq!(
            resolved,
            "<h1 data-who=\"[private]\">Signed in with [private]</h1>"
        );
        assert_eq!(received, &Some(json!("Signed in with [private]")));
        let message = text.timed_out(&scope, Some(&poll)).to_string();
        assert!(
            message.contains("Received: \"Signed in with [private]\"")
                && !message.contains("copied-secret-71"),
            "{message}"
        );

        let strict: ProbeReply = serde_json::from_value(json!({
            "kind": "strict",
            "message": "strict mode violation: locator('.who') resolved to 2 elements:\n    1) <p>copied-secret-71</p>",
        }))?;
        let violation = text
            .poll_of(&scope, strict, site)
            .err()
            .ok_or("a strict violation ends the matcher")?;
        assert!(
            violation.message().contains("1) <p>[private]</p>")
                && !violation.message().contains("copied-secret-71"),
            "{violation}"
        );
        Ok(())
    }
}
