//! The expect ops, their registration metadata, and the polls they run.

use serde::Deserialize;
use serde_json::Value;

use crate::expect::expectation::{Expectation, MatcherArgs, Poll, Subject};
use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, PAGE, TARGET};
use crate::page::OpPage;
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
    /// neither ever marks itself sent. `toHaveValue` on a password field carries the password
    /// in `expected` and `params`, which the trace redacts once a poll compared such a field.
    pub(crate) const fn spec(self) -> OpSpec {
        match self {
            Self::Locator => OpSpec::read(
                "expect.locator",
                TARGET,
                "one retrying locator matcher (toBeVisible, toHaveText, ...), with .not",
            )
            .sensitive(&["/expected", "/params"]),
            Self::Page => OpSpec::read(
                "expect.page",
                PAGE,
                "one retrying page matcher (toHaveTitle, toHaveURL), with .not",
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
}

/// `expect(page).toHaveTitle` and `toHaveURL`, after `call`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageInput {
    page: PageRef,
    matcher: String,
    expectation: String,
    expected: String,
    params: ExpectParams,
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
/// the last chunk's world, as upstream's `callOnSelector` does. Every poll previews page text
/// (what the locator resolved to, cut at 50 characters in the page), so each sweeps its
/// element's site first and sends the values along (`Resolver::attempt_masked`); the same
/// values mask the `Received` value and the preview again in Rust.
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
    };
    let expect = Expectation::new(Subject::Locator(target.display.clone()), args, scope)?;
    let page = OpPage::admit_handle(pw, scope, &target.page).await?;
    let resolver = Resolver::new(&page, scope, &target, expect.strictness());
    let poll = || async {
        let request = |last, frame_visible| expect.request(ProbeTarget::Chunk(last), frame_visible);
        match resolver.attempt_masked(request).await? {
            Some(Landed {
                reply,
                alongside: site,
                ..
            }) => expect.poll_of(scope, reply, || Ok(site)),
            None => Ok(expect.nothing_found()),
        }
    };
    expect.run(scope, poll).await
}

/// A page matcher, on the main frame's document (upstream's `:root`).
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
    } = input;
    let args = MatcherArgs {
        matcher,
        expectation,
        expected,
        params,
    };
    let expect = Expectation::new(Subject::Page, args, scope)?;
    let page = OpPage::admit_handle(pw, scope, &page).await?;
    let request = expect.request(ProbeTarget::Document, true);
    expect
        .run(scope, || main_frame(&page, scope, &expect, &request))
        .await
}

/// One poll of the main frame, with its site's values swept first and sent along: the title
/// and the URL it compares are page text, and the probe previews the document's root as it
/// does a locator's element. A navigation that replaced the document reads as nothing there
/// for this poll; the next one installs in the new document.
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
    match resolve::ask_masked(page, scope, &world, request).await? {
        Ok(asked) => expect.poll_of(scope, asked.reply, || Ok(asked.alongside)),
        Err(ProbeFailure::Gone(_)) => {
            page.frames().invalidate(&world);
            Ok(Poll::Missing)
        }
        Err(failure) => Err(resolve::failed(scope, failure)),
    }
}
