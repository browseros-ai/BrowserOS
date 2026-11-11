//! The expect ops, their registration metadata, and the polls they run.

use serde::Deserialize;
use serde_json::Value;

use crate::expect::expectation::{Expectation, MatcherArgs, Poll, Subject};
use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, PAGE, TARGET};
use crate::page::{OpPage, document_op};
use crate::probe::{ExpectParams, ProbeFailure, ProbeRequest, ProbeTarget};
use crate::resolve::{self, Landed, Resolver};
use crate::target::{PageRef, Target};
use crate::world::Place;
use crate::{ErrorCode, Playwright, PlaywrightError};

listed_enum! {
    /// Retrying matchers. Non-retrying value matchers (`toBe`, `toEqual`, ...) run in the
    /// prelude and never reach Rust.
    pub(crate) enum ExpectOp {
        Locator,
        Page,
    }
}

impl ExpectOp {
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

/// `expect(locator).<matcher>`, after `call`. No `Debug`: `expected` and `params` may hold a
/// password the script compares.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocatorInput {
    target: Target,
    matcher: String,
    expectation: String,
    expected: String,
    params: ExpectParams,
    /// Sent only when an aria template did not parse.
    #[serde(default, rename = "templateError")]
    template_error: Option<String>,
}

/// `expect(page).toHaveTitle`, `toHaveURL` and `toMatchAriaSnapshot`, after `call`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageInput {
    page: PageRef,
    matcher: String,
    expectation: String,
    expected: String,
    params: ExpectParams,
    /// Sent only when an aria template did not parse.
    #[serde(default, rename = "templateError")]
    template_error: Option<String>,
}

/// Runs one expect op. Boxed, as an action is: a poll loop holds a large state machine, and the
/// future every op shares (`Playwright::invoke`) is as large as its largest family's.
pub(crate) async fn invoke(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: ExpectOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    match op {
        ExpectOp::Locator => Box::pin(locator(pw, scope, args.decode(scope)?)).await,
        ExpectOp::Page => Box::pin(page(pw, scope, args.decode(scope)?)).await,
    }
}

/// A locator matcher: each poll re-resolves the locator, frames included, and checks it in
/// the last chunk's world, as upstream's `callOnSelector` does.
async fn locator(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: LocatorInput,
) -> Result<Value, PlaywrightError> {
    let LocatorInput {
        target,
        matcher,
        expectation,
        expected,
        params,
        template_error,
    } = input;
    let Target::Locator(target) = target else {
        return Err(scope.fail(
            ErrorCode::InvalidInput,
            &format!("{matcher} can be only used with Locator object, not an ElementHandle"),
        ));
    };
    let args = MatcherArgs {
        matcher,
        expectation,
        expected,
        params,
        template_error,
    };
    let expect = Expectation::new(Subject::Locator(target.display.clone()), args, scope)?;
    let page = OpPage::admit_handle(pw, scope, &target.page).await?;
    let resolver = Resolver::new(&page, scope, &target, expect.strictness());
    let poll = || async {
        let request = |last, frame_visible| expect.request(ProbeTarget::Chunk(last), frame_visible);
        match resolver.attempt_at(request).await? {
            Some(Landed { reply, .. }) => expect.poll_of(scope, reply),
            None => Ok(expect.nothing_found()),
        }
    };
    // Upstream snapshots the body of the frame `expect` ran on, the locator's own frame, not
    // the frame its last chunk enters.
    let root_snapshot = || document_op::expect_snapshot(&page, scope, target.frame.as_ref());
    expect.run(scope, poll, root_snapshot).await
}

/// A page matcher, on the main frame's document (upstream's `:root`; the probe matches an aria
/// template from its `body`, as upstream does).
async fn page(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PageInput,
) -> Result<Value, PlaywrightError> {
    let PageInput {
        page,
        matcher,
        expectation,
        expected,
        params,
        template_error,
    } = input;
    let args = MatcherArgs {
        matcher,
        expectation,
        expected,
        params,
        template_error,
    };
    let expect = Expectation::new(Subject::Page, args, scope)?;
    let page = OpPage::admit_handle(pw, scope, &page).await?;
    let request = expect.request(ProbeTarget::Document, true);
    expect
        .run(
            scope,
            || main_frame(&page, scope, &expect, &request),
            || document_op::expect_snapshot(&page, scope, None),
        )
        .await
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
    match resolve::ask(&world, request, None).await {
        Ok(reply) => expect.poll_of(scope, reply),
        Err(ProbeFailure::Gone(_)) => {
            page.frames().invalidate(&world);
            Ok(Poll::Missing)
        }
        Err(failure) => Err(resolve::failed(scope, failure)),
    }
}
