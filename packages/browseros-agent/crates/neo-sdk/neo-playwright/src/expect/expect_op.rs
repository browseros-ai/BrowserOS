//! The expect ops, their registration metadata, and the polls they run.

use std::sync::{Mutex, PoisonError};

use serde::Deserialize;
use serde_json::{Map, Value};
use ts_rs::TS;

use neo_snapshot_apc::tree::{AriaRoot, DocId, NameCache, NodeId, SnapshotTree, TreeDoc};

use crate::expect::aria_expect::{AriaExpect, AriaExpectError, FailureSnapshot, Visibility};
use crate::expect::expectation::{
    Expectation, FailedOn, FailedPollSnapshot, MatcherArgs, Poll, Subject,
};
use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, PAGE, TARGET};
use crate::page::OpPage;
use crate::probe::{self, ExpectParams, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget, Want};
use crate::resolve::{self, Landed, Resolver};
use crate::target::{PageRef, Target};
use crate::tree::{AttemptStart, TreeAsk};
use crate::wire::{self, Command, Describe, Nothing, OpWire, Route, Run, options};
use crate::world::{IsolatedWorld, Place};
use crate::{ErrorCode, Playwright, PlaywrightError, locator_handler, snapshot};

listed_enum! {
    /// Retrying matchers. Non-retrying value matchers (`toBe`, `toEqual`, ...) run in the
    /// prelude and never reach Rust.
    pub(crate) enum ExpectOp {
        Locator,
        Page,
    }
}

impl ExpectOp {
    /// Describes the same matcher inputs and null result the runtime route handles.
    pub(crate) fn wire(self) -> OpWire {
        route(self, Describe)
    }

    /// This op's registration metadata. Both are reads: a poll only looks at the page, so
    /// neither ever marks itself sent.
    pub(crate) const fn spec(self) -> OpSpec {
        match self {
            Self::Locator => OpSpec::read(
                "expect.locator",
                TARGET,
                "one retrying locator matcher (toBeVisible, toHaveText, ...), with .not",
            ),
            Self::Page => OpSpec::read(
                "expect.page",
                PAGE,
                "one retrying page matcher (toHaveTitle, toHaveURL, toMatchAriaSnapshot), with .not",
            ),
        }
    }
}

/// `expect(locator).<matcher>`, after `call`: `Frame.expect`'s params with the locator brand in
/// place of `selector`, then the four fields Rust prints the failure from (the row's `adds`).
/// The optional params stay JSON: Rust hands them to the probe as the prelude sent them and
/// checks them there (`Expectation::new`), so a decode error never quotes the expected value.
/// No `Debug`: `expectedText`, `expectedValue` and `expected` may hold a password the script
/// compares.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename = "LocatorExpectInput")]
struct LocatorInput {
    target: Target,
    expression: String,
    #[serde(default, with = "present")]
    #[ts(
        optional,
        type = "import('../../../probe/protocol').ExpectParams['expressionArg']"
    )]
    expression_arg: Option<Value>,
    #[serde(default, with = "present")]
    #[ts(
        optional,
        type = "import('../../../probe/protocol').ExpectParams['pseudo']"
    )]
    pseudo: Option<Value>,
    #[serde(default, with = "present")]
    #[ts(
        optional,
        type = "import('../../../probe/protocol').ExpectParams['expectedText']"
    )]
    expected_text: Option<Value>,
    #[serde(default, with = "present")]
    #[ts(
        optional,
        type = "import('../../../probe/protocol').ExpectParams['expectedNumber']"
    )]
    expected_number: Option<Value>,
    #[serde(default, with = "present")]
    #[ts(
        optional,
        type = "import('../../../probe/protocol').ExpectParams['expectedValue']"
    )]
    expected_value: Option<Value>,
    #[serde(default, with = "present")]
    #[ts(
        optional,
        type = "import('../../../probe/protocol').ExpectParams['useInnerText']"
    )]
    use_inner_text: Option<Value>,
    is_not: bool,
    matcher: String,
    expectation: String,
    expected: String,
    /// Sent only when an aria template did not parse.
    #[serde(default)]
    #[ts(optional)]
    template_error: Option<String>,
}
options!(LocatorInput);

impl LocatorInput {
    /// The locator, and the matcher with its params rebuilt as the probe receives them.
    fn split(self) -> Split<Target> {
        let Self {
            target,
            expression,
            expression_arg,
            pseudo,
            expected_text,
            expected_number,
            expected_value,
            use_inner_text,
            is_not,
            matcher,
            expectation,
            expected,
            template_error,
        } = self;
        let params = FrameExpect {
            expression,
            expression_arg,
            pseudo,
            expected_text,
            expected_number,
            expected_value,
            use_inner_text,
            is_not,
        };
        Split {
            receiver: target,
            args: params.into_args(matcher, expectation, expected, template_error),
        }
    }
}

/// `expect(page).toHaveTitle`, `toHaveURL` and `toMatchAriaSnapshot`, after `call`: the
/// locator op's fields, with the page brand in place of `selector`.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename = "PageExpectInput")]
struct PageInput {
    page: PageRef,
    expression: String,
    #[serde(default, with = "present")]
    #[ts(
        optional,
        type = "import('../../../probe/protocol').ExpectParams['expressionArg']"
    )]
    expression_arg: Option<Value>,
    #[serde(default, with = "present")]
    #[ts(
        optional,
        type = "import('../../../probe/protocol').ExpectParams['pseudo']"
    )]
    pseudo: Option<Value>,
    #[serde(default, with = "present")]
    #[ts(
        optional,
        type = "import('../../../probe/protocol').ExpectParams['expectedText']"
    )]
    expected_text: Option<Value>,
    #[serde(default, with = "present")]
    #[ts(
        optional,
        type = "import('../../../probe/protocol').ExpectParams['expectedNumber']"
    )]
    expected_number: Option<Value>,
    #[serde(default, with = "present")]
    #[ts(
        optional,
        type = "import('../../../probe/protocol').ExpectParams['expectedValue']"
    )]
    expected_value: Option<Value>,
    #[serde(default, with = "present")]
    #[ts(
        optional,
        type = "import('../../../probe/protocol').ExpectParams['useInnerText']"
    )]
    use_inner_text: Option<Value>,
    is_not: bool,
    matcher: String,
    expectation: String,
    expected: String,
    /// Sent only when an aria template did not parse.
    #[serde(default)]
    #[ts(optional)]
    template_error: Option<String>,
}
options!(PageInput);

impl PageInput {
    /// The page, and the matcher with its params rebuilt as the probe receives them.
    fn split(self) -> Split<PageRef> {
        let Self {
            page,
            expression,
            expression_arg,
            pseudo,
            expected_text,
            expected_number,
            expected_value,
            use_inner_text,
            is_not,
            matcher,
            expectation,
            expected,
            template_error,
        } = self;
        let params = FrameExpect {
            expression,
            expression_arg,
            pseudo,
            expected_text,
            expected_number,
            expected_value,
            use_inner_text,
            is_not,
        };
        Split {
            receiver: page,
            args: params.into_args(matcher, expectation, expected, template_error),
        }
    }
}

/// A decoded expect input, apart: what the matcher runs on (the locator or page brand), and
/// the matcher Rust polls and prints.
struct Split<R> {
    receiver: R,
    args: MatcherArgs,
}

/// Upstream's `FrameExpectParams` without `selector`, as both ops carry them flat. The probe
/// protocol keeps them nested (`ProbeRequest::Expect.params`, `ExpectParams`), so its bytes do
/// not depend on the op's wire.
struct FrameExpect {
    expression: String,
    expression_arg: Option<Value>,
    pseudo: Option<Value>,
    expected_text: Option<Value>,
    expected_number: Option<Value>,
    expected_value: Option<Value>,
    use_inner_text: Option<Value>,
    is_not: bool,
}

impl FrameExpect {
    /// The matcher Rust runs: these params as the probe's object, with exactly the keys the
    /// prelude sent (`null` included, absent left out), and the fields the failure prints.
    fn into_args(
        self,
        matcher: String,
        expectation: String,
        expected: String,
        template_error: Option<String>,
    ) -> MatcherArgs {
        let Self {
            expression,
            expression_arg,
            pseudo,
            expected_text,
            expected_number,
            expected_value,
            use_inner_text,
            is_not,
        } = self;
        let mut params = Map::new();
        params.insert("expression".to_owned(), Value::String(expression));
        let optional = [
            ("expressionArg", expression_arg),
            ("pseudo", pseudo),
            ("expectedText", expected_text),
            ("expectedNumber", expected_number),
            ("expectedValue", expected_value),
            ("useInnerText", use_inner_text),
        ];
        for (key, value) in optional {
            if let Some(value) = value {
                params.insert(key.to_owned(), value);
            }
        }
        params.insert("isNot".to_owned(), Value::Bool(is_not));
        MatcherArgs {
            matcher,
            expectation,
            expected,
            params: ExpectParams(params),
            template_error,
        }
    }
}

/// `#[serde(with)]` for an optional param: a field that is there, `null` included, is `Some`;
/// absent stays `None` through `#[serde(default)]`. A plain `Option<Value>` reads `null` as
/// absent, and the probe must see the `null` a script's argument made (`toHaveCount(null)`).
mod present {
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Value>, D::Error> {
        Value::deserialize(deserializer).map(Some)
    }
}

/// Runs one expect op through the shared decode/call/encode boundary. The route boxes its
/// poll future so the dispatcher does not inherit the matcher loop's state-machine size.
pub(crate) async fn invoke(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: ExpectOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    route(op, Run { pw, scope, args }).await
}

/// Both ops carry `Frame.expect`'s params, flat as upstream's, beside their receiver. Upstream's
/// client prints a failed matcher from the error's details; Neo's Rust prints it, so the trace
/// records the text the script sees, and needs four fields upstream's client keeps to itself.
/// Run and Describe share these rows; matcher policy and polling remain handwritten.
fn route<'a, R: Route<'a>>(op: ExpectOp, r: R) -> R::Out {
    match op {
        ExpectOp::Locator => r.op(FRAME_EXPECT, locator),
        ExpectOp::Page => r.op(FRAME_EXPECT, page),
    }
}

/// The row both ops share: each difference from `Frame.expect`, with its reason.
const FRAME_EXPECT: Command = wire::command("Frame.expect")
    .adds(&[
        (
            "matcher",
            "the matcher's name: Rust prints upstream's failure header, \
             `expect(locator).toHaveText(...)`, and refuses a matcher of the wrong receiver \
             by name",
        ),
        (
            "expectation",
            "the header's parenthesized argument as upstream's client prints it: Rust \
             formats the failure, upstream's client does",
        ),
        (
            "expected",
            "the printed `Expected:` line: Rust formats the failure, upstream's client does",
        ),
        (
            "templateError",
            "an aria template that did not parse: the prelude parses templates with the \
             vendored yaml, and Rust raises the error where upstream's server does",
        ),
    ])
    .retypes(&[
        (
            "expressionArg",
            "upstream's `json` param, which channels.d.ts and the probe type `any` and the \
             check cannot compare: the name toHaveAttribute, toHaveCSS or toHaveJSProperty \
             passes, as JSON",
        ),
        (
            "expectedValue",
            "plain JSON, as upstream's server hands it to the injected script after \
             `parseArgument` (Neo has no JS handles); an aria template arrives parsed by the \
             prelude",
        ),
    ]);

/// A locator matcher: each poll re-resolves the locator, frames included, and checks it in
/// the last chunk's world, as upstream's `callOnSelector` does. The accessibility matchers and
/// `toMatchAriaSnapshot` check it in the tab's APC tree instead ([`tree_poll`]).
async fn locator(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: LocatorInput,
) -> Result<Nothing, PlaywrightError> {
    let Split {
        receiver: target,
        args,
    } = input.split();
    let Target::Locator(target) = target else {
        return Err(scope.fail(
            ErrorCode::InvalidInput,
            &format!(
                "{} can be only used with Locator object, not an ElementHandle",
                args.matcher
            ),
        ));
    };
    let expect = Expectation::new(Subject::Locator(target.display.clone()), args, scope)?;
    let aria = AriaExpect::new(expect.params()).transpose();
    let page = OpPage::admit_handle(pw, scope, &target.page).await?;
    let resolver = Resolver::new(&page, scope, &target, expect.strictness());
    // The element the last page poll ran on, for the one failure snapshot read when the op
    // fails (`Expectation::run`); a tree poll carries its own, read from its own capture.
    let polled_on: Mutex<Option<Sent>> = Mutex::new(None);
    let poll = || async {
        if let Some(aria) = &aria {
            return tree_poll(scope, &page, &resolver, &expect, aria).await;
        }
        let request = |target, frame_visible| expect.request(target, frame_visible);
        match resolver.attempt_at(request).await? {
            Some(Landed {
                reply, world, sent, ..
            }) => {
                *polled_on.lock().unwrap_or_else(PoisonError::into_inner) =
                    sent.map(|sent| Sent::from((world, sent)));
                expect.poll_of(scope, reply)
            }
            None => Ok(expect.nothing_found()),
        }
    };
    let (page_ref, expect_ref, polled_ref) = (&page, &expect, &polled_on);
    let frame = target.frame.as_ref();
    let failure_snapshot = |on| async move {
        match (on, expect_ref.failed_poll_snapshot()) {
            // Upstream snapshots the body of the frame `expect` ran on, the locator's own
            // frame, not the frame its last chunk enters.
            (FailedOn::Nothing, _) | (FailedOn::Element, FailedPollSnapshot::Body) => {
                snapshot::expect_body_snapshot(scope, page_ref, frame).await
            }
            (FailedOn::Element, FailedPollSnapshot::Nothing) => None,
            (FailedOn::Element, FailedPollSnapshot::OwnLine) => {
                let element = polled_ref
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take()?;
                element_failure_snapshot(scope, page_ref, expect_ref, element, Some(1)).await
            }
            (FailedOn::Element, FailedPollSnapshot::Whole) => {
                let element = polled_ref
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take()?;
                element_failure_snapshot(scope, page_ref, expect_ref, element, None).await
            }
        }
    };
    expect
        .run(
            scope,
            || locator_handler::checkpoint(scope, &page),
            poll,
            failure_snapshot,
        )
        .await
}

/// One poll of a matcher the tree answers (design 5.6): the locator's one element, pinned by
/// the resolver, then found in a capture taken after that, and read there. The params' error, a
/// regex that does not compile or a template that cannot run, is raised once a poll has found
/// the element, as upstream's page throws it.
async fn tree_poll(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    resolver: &Resolver<'_>,
    expect: &Expectation,
    aria: &Result<AriaExpect, AriaExpectError>,
) -> Result<Poll, PlaywrightError> {
    let request = |target, _frame_visible| ProbeRequest::Resolve {
        target,
        strictness: expect.strictness(),
        want: Want::Element,
    };
    let Some(Landed { reply, world, .. }) = resolver.attempt_at(request).await? else {
        return Ok(expect.nothing_found());
    };
    let (pin, preview) = match reply {
        ProbeReply::Element { pin, preview, .. } => (pin, preview),
        other => return expect.poll_of(scope, other),
    };
    let aria = aria
        .as_ref()
        .map_err(|error| expect.refused_matcher(scope, &error.to_string()))?;
    let Some(found) = snapshot::element_in_tree(scope, page, &world, pin).await? else {
        return Ok(expect.nothing_found());
    };
    let tree = found.captured.tree();
    let verdict = match aria {
        AriaExpect::Property(matcher) => {
            let names = NameCache::new(tree);
            matcher.check(tree, &names, found.node)
        }
        AriaExpect::Template(matcher) => {
            let Some(roots) = found.roots(scope, page, &world, pin).await? else {
                return Ok(expect.nothing_found());
            };
            matcher
                .check_roots(tree, &roots)
                .map_err(|error| expect.refused_matcher(scope, &error.to_string()))?
        }
    };
    // Upstream attaches the element's own line, or its frame's body when it is hidden, to a
    // property poll that failed; a template poll attaches none.
    let aria_snapshot = match aria {
        AriaExpect::Property(_) if verdict.matches == expect.is_not() => {
            failure_snapshot(page, tree, found.doc, found.node, true, Some(1))
        }
        AriaExpect::Property(_) | AriaExpect::Template(_) => None,
    };
    Ok(Poll::Ran {
        matches: verdict.matches,
        resolved: preview,
        received: Some(expect.printed(verdict.received)),
        aria_snapshot,
    })
}

/// The element a page poll ran its matcher on: the world the poll ran in, what its request
/// named there, and whether every frame on the way is visible.
struct Sent {
    world: IsolatedWorld,
    target: ProbeTarget,
    frame_visible: bool,
}

impl From<(IsolatedWorld, resolve::SentRequest)> for Sent {
    fn from((world, sent): (IsolatedWorld, resolve::SentRequest)) -> Self {
        Self {
            world,
            target: sent.target,
            frame_visible: sent.frame_visible,
        }
    }
}

/// The failure snapshot of the element the last page poll ran on, at `depth` (`None`: whole):
/// it is pinned once more in that poll's world and found in a capture taken after that (design
/// 6.2), since the page's engine prints none. `None` when it is gone or cannot be read: the
/// snapshot is best effort, as it leaves the failure itself as it is.
async fn element_failure_snapshot(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    expect: &Expectation,
    element: Sent,
    depth: Option<usize>,
) -> Option<String> {
    let request = ProbeRequest::Resolve {
        target: element.target,
        strictness: expect.strictness(),
        want: Want::Element,
    };
    let Ok(ProbeReply::Element { pin, .. }) =
        probe::call(scope, page, &element.world, &request).await
    else {
        return None;
    };
    let found = snapshot::element_in_tree(scope, page, &element.world, pin)
        .await
        .ok()
        .flatten()?;
    let tree = found.captured.tree();
    failure_snapshot(
        page,
        tree,
        found.doc,
        found.node,
        element.frame_visible,
        depth,
    )
}

/// A failed poll's aria snapshot (`aria_expect::FailureSnapshot`): the element in default mode
/// to `depth` (`None`: whole), or the body of its frame when the frame or the element is hidden
/// or the tree has no node for it. `None` when it prints nothing.
fn failure_snapshot(
    page: &OpPage<'_>,
    tree: &SnapshotTree,
    doc: DocId,
    node: Option<NodeId>,
    frame_visible: bool,
    depth: Option<usize>,
) -> Option<String> {
    let visible = node
        .and_then(|node| tree.node(node))
        .is_some_and(|node| node.hidden().is_box_visible());
    let element = Visibility {
        frame: frame_visible,
        element: visible,
    };
    let (root, depth) = match (FailureSnapshot::for_failed_poll(element), node) {
        (FailureSnapshot::Element, Some(node)) => (AriaRoot::Subtree(node), depth),
        (FailureSnapshot::Element | FailureSnapshot::Body, _) => (AriaRoot::Document(doc), None),
    };
    let ask = TreeAsk {
        ai: false,
        root,
        depth,
        boxes: false,
    };
    let snapshot = page.tree().aria(tree, ask).ok()?.into_snapshot();
    let text = neo_snapshot::render(&snapshot, neo_snapshot::Call::Playwright).into_text();
    Some(text).filter(|text| !text.is_empty())
}

/// A page matcher, on the main frame's document (upstream's `:root`; an aria template matches
/// from its `body`, as upstream does, which the tree reads as the main document).
async fn page(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PageInput,
) -> Result<Nothing, PlaywrightError> {
    let Split {
        receiver: page,
        args,
    } = input.split();
    let expect = Expectation::new(Subject::Page, args, scope)?;
    let aria = AriaExpect::new(expect.params()).transpose();
    let page = OpPage::admit_handle(pw, scope, &page).await?;
    let request = expect.request(ProbeTarget::Document, true);
    expect
        .run(
            scope,
            || locator_handler::checkpoint(scope, &page),
            || async {
                match &aria {
                    Some(aria) => main_document_poll(&page, scope, &expect, aria).await,
                    None => main_frame(&page, scope, &expect, &request).await,
                }
            },
            |_| snapshot::expect_body_snapshot(scope, &page, None),
        )
        .await
}

/// One poll of a page matcher the tree answers: an aria template against the main document of
/// a fresh capture. A capture with no main document yet reads as nothing there for this poll.
async fn main_document_poll(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    expect: &Expectation,
    aria: &Result<AriaExpect, AriaExpectError>,
) -> Result<Poll, PlaywrightError> {
    let aria = aria
        .as_ref()
        .map_err(|error| expect.refused_matcher(scope, &error.to_string()))?;
    let AriaExpect::Template(matcher) = aria else {
        // The property matchers are a locator's only: no page matcher sends one.
        return Err(scope.fail(
            ErrorCode::Internal,
            "an accessibility property matcher on a page",
        ));
    };
    let captured = page
        .tree()
        .capture_seeded(scope, page, AttemptStart::now())
        .await?;
    let tree = captured.tree();
    let Some(doc) = tree.main_document().map(TreeDoc::id) else {
        return Ok(Poll::Missing);
    };
    let verdict = matcher
        .check_roots(tree, &[AriaRoot::Document(doc)])
        .map_err(|error| expect.refused_matcher(scope, &error.to_string()))?;
    Ok(Poll::Ran {
        matches: verdict.matches,
        resolved: "<body>".to_owned(),
        received: Some(expect.printed(verdict.received)),
        aria_snapshot: None,
    })
}

/// One poll of the main frame, on the document's root. A navigation that replaced the document
/// reads as nothing there for this poll; the next one installs in the new document.
async fn main_frame(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    expect: &Expectation,
    request: &ProbeRequest,
) -> Result<Poll, PlaywrightError> {
    let world = match page.world(&page.main_endpoint(), Place::Main).await {
        Ok(world) => world,
        Err(ProbeFailure::Gone(_)) => return Ok(Poll::Missing),
        Err(failure) => return Err(resolve::failed(scope, failure)),
    };
    match resolve::ask(scope, page, &world, request, None).await {
        Ok(reply) => expect.poll_of(scope, reply),
        Err(ProbeFailure::Gone(_)) => {
            page.frames().invalidate(&world);
            Ok(Poll::Missing)
        }
        Err(failure) => Err(resolve::failed(scope, failure)),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::{Value, json};

    use super::{LocatorInput, PageInput};

    /// The probe's object holds exactly the params the prelude sent flat: a `null` a script's
    /// argument made stays, an absent param stays absent, and Neo's four fields stay out.
    #[test]
    fn tests_that_the_flat_params_reach_the_probe_as_the_prelude_sent_them()
    -> Result<(), Box<dyn Error>> {
        let input: PageInput = serde_json::from_value(json!({
            "page": { "$neo": "page", "tab": 7 },
            "expression": "to.have.url",
            "expressionArg": null,
            "expectedText": [{ "string": "https://a.test/", "ignoreCase": true }],
            "isNot": true,
            "matcher": "toHaveURL",
            "expectation": "expected",
            "expected": "Expected: not \"https://a.test/\"",
        }))?;
        let args = input.split().args;
        assert_eq!(
            Value::Object(args.params.0),
            json!({
                "expression": "to.have.url",
                "expressionArg": null,
                "expectedText": [{ "string": "https://a.test/", "ignoreCase": true }],
                "isNot": true,
            })
        );
        assert_eq!(args.matcher, "toHaveURL");
        assert_eq!(args.expectation, "expected");
        assert_eq!(args.template_error, None);
        Ok(())
    }

    /// Every param a locator matcher can send reaches the probe under upstream's name.
    #[test]
    fn tests_that_every_frame_expect_param_reaches_the_probe() -> Result<(), Box<dyn Error>> {
        let input: LocatorInput = serde_json::from_value(json!({
            "target": {
                "$neo": "locator",
                "page": { "$neo": "page", "tab": 7 },
                "frame": null,
                "chunks": [{ "parts": [{ "name": "css", "body": "p", "source": "p" }] }],
                "display": "locator('p')",
            },
            "expression": "to.have.css",
            "expressionArg": "color",
            "pseudo": "before",
            "expectedText": [{ "string": "red" }],
            "expectedNumber": 1,
            "expectedValue": { "checked": false },
            "useInnerText": false,
            "isNot": false,
            "matcher": "toHaveCSS",
            "expectation": "expected",
            "expected": "Expected: \"red\"",
            "templateError": "bad template",
        }))?;
        let args = input.split().args;
        assert_eq!(
            Value::Object(args.params.0),
            json!({
                "expression": "to.have.css",
                "expressionArg": "color",
                "pseudo": "before",
                "expectedText": [{ "string": "red" }],
                "expectedNumber": 1,
                "expectedValue": { "checked": false },
                "useInnerText": false,
                "isNot": false,
            })
        );
        assert_eq!(args.template_error.as_deref(), Some("bad template"));
        Ok(())
    }

    /// The nested shape this op carried before its params went flat is refused at decode.
    #[test]
    fn tests_that_nested_params_are_refused() -> Result<(), Box<dyn Error>> {
        let error = serde_json::from_value::<PageInput>(json!({
            "page": { "$neo": "page", "tab": 7 },
            "matcher": "toHaveTitle",
            "expectation": "expected",
            "expected": "Expected: \"x\"",
            "params": { "expression": "to.have.title", "isNot": false },
        }))
        .err()
        .ok_or("the nested shape must not decode")?;
        assert!(
            error.to_string().contains("`params`"),
            "the refusal names the key: {error}"
        );
        Ok(())
    }
}
