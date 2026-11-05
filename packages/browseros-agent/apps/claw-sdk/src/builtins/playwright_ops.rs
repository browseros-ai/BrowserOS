//! browseros-playwright's ops, served through the door (sdk-skeleton decision 23, and the
//! Playwright adapter contract in the program's CONTEXT.md).
//!
//! ```text
//!  prelude ─ __neo.invoke(op, json) ─ Dispatcher::submit ─ OpInput: the brand at `page` or
//!    `target.page` ─ the door resolves it to one tab and runs its rules on that tab
//!      ─ PlaywrightOp::call: the resolved page written back into the input
//!          ─ Playwright::invoke(OpCtxEnv, op, json), raced with the run's stop
//!              ├─ recheck, mark_sent, mark_sensitive_target, note ─▶ OpCtx
//!              └─ PlaywrightError ─ leaf_error ─▶ LeafError: owner, code, timeout, answer
//!          ─ opened, navigated and closed pages reported to the door
//!          ─ an event pull's events: each one's tabs through the rules (event_release.rs)
//! ```
//!
//! The crate owns what each op does and how its failures read; the door keeps its own chores
//! (the trace, the rules, delivery, masking, page facts). This file only translates between
//! the two. The ops in [`HELD`] are the exception: their names answer a refusal, and the crate
//! never sees the call.

use std::future::{self, Future};
use std::sync::Arc;

use browseros_core::{BrowserView, PageId, TabId};
use browseros_playwright::{
    Answer, ErrorCode as PlaywrightCode, Op, OpEnv, OpLifetime, PlaywrightError, Refusal, RunState,
    TimedOut,
};
use browseros_trace::{Effect, ErrorOwner};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value, json};

use super::PlaywrightFacade;
use super::event_release::EventRelease;
use crate::browser::PageRef;
use crate::dispatch::{
    ErrorCode, LeafError, Lifetime, Listing, Method, MethodSpec, OpCtx, OperationError, PageChange,
    PageTarget, RegistryBuilder, RegistryError,
};

/// browseros-playwright's prelude, which the agent realm evaluates after its bootstrap: the
/// `page`, `context`, `browser` and `expect` parameters of the agent's function, over these ops.
/// Here so that `runtime/`, which never names a browser crate (STYLE.md P2), can install it.
pub(crate) const PLAYWRIGHT_PRELUDE: &str = browseros_playwright::PRELUDE_JS;

/// The `run` description's one line for every Playwright op. It names what is not built yet and
/// what is held ([`HELD`]), so a model does not spend a call finding out.
const PLAYWRIGHT_LINE: &str = "Playwright's API (as of 1.56) on `page`, `context` and \
    `browser`: locators (`getByRole`, `getByText`, `getByLabel`, `getByTestId`, `locator`, \
    `frameLocator`, `filter`, `nth`) that wait and are strict, frames (`page.frames()`, \
    `frame.locator`), reads (`textContent`, `innerText`, `getAttribute`, `count`, \
    `isVisible`, `page.content()`, `page.title()`), navigation (`goto`, `reload`, \
    `waitForURL`, `waitForLoadState`), `screenshot`, and events (`page.on`, `waitForEvent`). \
    Actions (`click`, `fill`, `press`), `keyboard`, `mouse` and `expect` are not built yet and \
    fail with `internal_error`. Running script in the page (`page.evaluate`, \
    `page.waitForFunction`, `locator.evaluate`, `locator.evaluateAll`, `$eval`, `$$eval`, \
    `elementHandle.evaluate`, `page.setContent`, and their `frame` forms) is not available for \
    now and fails with `method_unavailable`, and `goto` refuses `javascript:` URLs.";

/// The ops claw-sdk holds back for now: every op that runs the agent's script in the page's main
/// world, where the page's own state and its same-origin frames are in reach and no seal masks
/// a field. A held op's name answers `script / method_unavailable` with the reason, and the
/// crate's op never runs.
/// - `page.evaluate` and `page.waitForFunction` (decision 25a) run it between the evaluate
///   guard's sweeps (`page/evaluate_guard.rs`), which missed same-site frames on another CDP
///   session, so a password in one came back raw. sdk-playwright is fixing that.
/// - `locator.evaluate` and `locator.evaluateAll` run it on elements the same way. They are not
///   built yet, and held before they are.
/// - `page.setContent` writes the agent's HTML from the sealed world, but the scripts in that
///   HTML run in the main world, with the page's origin and the state its own scripts keep, and
///   no sweep at all.
///
/// `page.title` and `page.content` read in the sealed world and are served as usual. Serving an
/// op again is a commit of its own, once a fix covers it.
///
/// The rule is the property, not this list: an op is held because it runs script in the main
/// world. Once sdk-playwright marks such ops in `OpSpec`, `HELD` becomes "every op so marked",
/// so a main-world op the crate adds later is held by default instead of served until someone
/// lists it here. Until then each entry is a crate op name, which covers every API the prelude
/// sends under it: the `frame` forms send the page ops; `$eval`, `frame.$eval` and
/// `elementHandle.evaluate` send `locator.evaluate`; `$$eval` and `frame.$$eval` send
/// `locator.evaluateAll`. `elementHandle.$eval` and `$$eval` send nothing: the prelude refuses
/// them itself. A served op that runs script only for some inputs is refused by input instead:
/// `page.goto` to a `javascript:` URL ([`OpInput::navigates_to_script`]).
const HELD: [Held; 5] = [
    Held {
        name: "page.evaluate",
        instead: "read the page through locators (`textContent`, `innerText`, \
                  `getAttribute`), `page.content()` or `page.title()`",
    },
    Held {
        name: "page.waitForFunction",
        instead: "wait through a locator, which waits by itself, or through \
                  `page.waitForURL()` and `page.waitForLoadState()`",
    },
    Held {
        name: "locator.evaluate",
        instead: "read the element with `textContent()`, `innerText()`, `getAttribute()` or \
                  `inputValue()`",
    },
    Held {
        name: "locator.evaluateAll",
        instead: "read the matches with `allTextContents()`, `allInnerTexts()` or `count()`, \
                  or one at a time through `nth()`",
    },
    Held {
        name: "page.setContent",
        instead: "open the HTML as a page of its own with \
                  `page.goto('data:text/html,' + encodeURIComponent(html))`, whose scripts run \
                  in an origin of their own",
    },
];

/// A held op, and what a script uses meanwhile. A plain data bag.
#[derive(Clone, Copy, Debug)]
struct Held {
    /// The crate's op name, which the prelude sends for every alias of the API ([`HELD`] lists
    /// them).
    name: &'static str,
    /// What the refusal suggests instead.
    instead: &'static str,
}

/// Registers every browseros-playwright op as a script method, a held one as its refusal.
/// Called once at startup; a duplicate name fails the start.
pub(super) fn playwright_ops(
    builder: &mut RegistryBuilder,
    facade: &Arc<PlaywrightFacade>,
) -> Result<(), RegistryError> {
    for op in Op::all() {
        let spec = op.spec();
        if let Some(&held) = HELD.iter().find(|held| held.name == spec.name) {
            builder.define(HeldOp {
                held,
                effect: spec.effect,
            })?;
            continue;
        }
        builder.define(PlaywrightOp {
            op,
            facade: Arc::clone(facade),
            pages: PageEffect::of(spec.name),
            events: EventRelease::of(spec.name),
        })?;
    }
    Ok(())
}

/// A held op's name, served by its refusal alone: the crate's op never runs. It names no page,
/// so the door refuses it before it would open the run's lazy page, and it reads no input, so the
/// answer is the hold whatever the script sent. Unlisted: the `run` description names the hold.
struct HeldOp {
    held: Held,
    /// The crate op's own, so the trace records the attempt as the op it stands for.
    effect: Effect,
}

impl Method for HeldOp {
    type Input = Value;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new(self.held.name, self.effect)
    }

    fn target(_input: &Value) -> Option<&PageTarget> {
        None
    }

    fn call(
        &self,
        _op: &OpCtx<Self>,
        _input: Value,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        let Held { name, instead } = self.held;
        future::ready(Err(LeafError::definite(OperationError::script(
            ErrorCode::METHOD_UNAVAILABLE,
            format_args!(
                "`{name}` is held while a privacy fix lands: it runs script in the page's main \
                 world. Meanwhile, {instead}."
            ),
        ))))
    }
}

/// One browseros-playwright op as a dispatcher method. Every op is this one type: the op is
/// data (`Op`), and so is all the door reads about it, so the crate's op table is the only list.
struct PlaywrightOp {
    op: Op,
    facade: Arc<PlaywrightFacade>,
    /// What the op does to pages that the door keeps books on.
    pages: PageEffect,
    /// Whether its result carries events, whose tabs the door's rules clear before release.
    events: EventRelease,
}

impl Method for PlaywrightOp {
    type Input = OpInput;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        let spec = self.op.spec();
        MethodSpec {
            listing: Listing::Grouped(PLAYWRIGHT_LINE),
            lifetime: match spec.lifetime {
                OpLifetime::Operation => Lifetime::Operation,
                OpLifetime::Background => Lifetime::Background,
            },
            sensitive_args: spec.sensitive_args,
            ..MethodSpec::new(spec.name, spec.effect)
        }
    }

    fn target(input: &OpInput) -> Option<&PageTarget> {
        input.page.as_ref()
    }

    async fn call(&self, op: &OpCtx<Self>, input: OpInput) -> Result<Value, LeafError> {
        if self.pages == PageEffect::Navigates && input.navigates_to_script() {
            return Err(LeafError::definite(OperationError::script(
                ErrorCode::INVALID_INPUT,
                "Neo does not open `javascript:` URLs: their script runs in the page's main \
                 world, with no sweep to mask its fields. Read the page through locators instead.",
            )));
        }
        let page = match &input.page {
            Some(_) => Some(op.page()?),
            None => None,
        };
        let pages = self.pages.of_call(&input);
        let input = input.resolved(page);
        let playwright = self.facade.get().await;
        let env = OpCtxEnv::new(op);
        let result = op
            .race_classified(playwright.invoke(&env, self.op, input))
            .await?;
        pages.report(op, page, &result);
        let value = result.map_err(|error| leaf_error(&error))?;
        self.events.release(op, value)
    }
}

/// A Playwright op's input as the door decodes it: the one fact the door needs, the page the op
/// acts on, and the rest left as JSON for the crate, which decodes it once itself, with
/// `deny_unknown_fields`.
struct OpInput {
    /// The page brand `{"$neo":"page","id","tab"}` the op acts on (adapter contract, point 1).
    page: Option<PageTarget>,
    json: Map<String, Value>,
}

/// A page brand in an op's input and the field it sits in, so a brand that does not decode is
/// refused by name. A plain data bag.
struct FoundBrand<'a> {
    /// `page` or `target.page`.
    field: &'static str,
    brand: &'a Value,
}

/// The brand an op acts on (adapter contract, point 1): at `page` for page, keyboard, mouse,
/// dialog and `expect.page` ops, a page's event listeners and waits, and screenshots (whose
/// `target` is an element on that same page); else at `target.page`, the page a locator or an
/// element handle belongs to. A context-wide listener sends `page: null`, which names no page.
/// Read for every op, the event ops included, though their `OpSpec` names no target: the crate
/// decodes their page as a checked handle too, so it must never see the lazy page there either.
fn page_brand(input: &Map<String, Value>) -> Option<FoundBrand<'_>> {
    match input.get("page") {
        Some(brand) if !brand.is_null() => Some(FoundBrand {
            field: "page",
            brand,
        }),
        Some(_) | None => input
            .get("target")
            .and_then(|target| target.get("page"))
            .map(|brand| FoundBrand {
                field: "target.page",
                brand,
            }),
    }
}

impl<'de> Deserialize<'de> for OpInput {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        let json = Map::<String, Value>::deserialize(deserializer)?;
        let page = match page_brand(&json) {
            None => None,
            Some(FoundBrand { field, brand }) => Some(
                PageTarget::deserialize(brand)
                    .map_err(|error| D::Error::custom(format_args!("`{field}`: {error}")))?,
            ),
        };
        Ok(Self { page, json })
    }
}

impl OpInput {
    /// `page.close({ runBeforeUnload: true })`: the page's `beforeunload` handlers run first,
    /// and one of them can keep the tab open.
    fn runs_before_unload(&self) -> bool {
        self.json.get("runBeforeUnload") == Some(&Value::Bool(true))
    }

    /// Whether the op navigates to a `javascript:` URL. Chromium runs one through
    /// `Page.navigate` in the current document's main world, and only then reports the
    /// navigation aborted, so such a `goto` is a `page.evaluate` with no evaluate guard around
    /// it: the door refuses it before it is sent ([`HELD`] holds the ops that always run script).
    fn navigates_to_script(&self) -> bool {
        self.json
            .get("url")
            .and_then(Value::as_str)
            .and_then(url_scheme)
            .is_some_and(|scheme| scheme == "javascript")
    }

    /// The input as the crate gets it (adapter contract, point 2; decision 25, input 2). The
    /// lazy brand `{"$neo":"page","id":null}` names the run's page wherever it sits, so once the
    /// door resolved that page for this op, every one of them becomes it: `dragTo`'s `to.page`,
    /// a screenshot's `page` and `target.page`. The crate refuses `id: null` on purpose: it acts
    /// on the very tab the door's rules checked, which a second lookup of its own could disagree
    /// with (two lazy pages, a tab that changed in between). For the same reason a lazy brand
    /// beside another page's target is left for the crate to refuse: the door checked that other
    /// tab, not the run's page. A target that names its tab needs nothing written back, since
    /// the door resolves it to the same id and tab.
    fn resolved(self, page: Option<&PageRef>) -> Value {
        let Self { page: target, json } = self;
        let mut json = Value::Object(json);
        if let (Some(target), Some(page)) = (target, page)
            && target.is_run_default()
        {
            replace_lazy_brands(&mut json, &json!(page));
        }
        json
    }
}

/// Puts `resolved` in place of every lazy page brand in `value`.
fn replace_lazy_brands(value: &mut Value, resolved: &Value) {
    let lazy = value.is_object()
        && PageTarget::deserialize(&*value).is_ok_and(|page| page.is_run_default());
    if lazy {
        resolved.clone_into(value);
        return;
    }
    match value {
        Value::Object(fields) => {
            for field in fields.values_mut() {
                replace_lazy_brands(field, resolved);
            }
        }
        Value::Array(items) => {
            for item in items {
                replace_lazy_brands(item, resolved);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// The scheme of `url` as the URL Standard reads it: leading and trailing C0 controls and spaces
/// dropped, tabs and newlines removed wherever they are, then an ASCII letter and letters,
/// digits, `+`, `-` or `.` up to the first `:`, lowercased. `None` when there is none. Chromium
/// trims, strips and lowercases the same way, so ` JavaScript:` and `java\tscript:` are
/// `javascript:` to it too; a scheme with any other character is no URL it navigates to.
fn url_scheme(url: &str) -> Option<String> {
    let mut scheme = String::new();
    let chars = url
        .trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'));
    for c in chars {
        match c {
            ':' if !scheme.is_empty() => return Some(scheme),
            'a'..='z' | 'A'..='Z' => scheme.push(c.to_ascii_lowercase()),
            '0'..='9' | '+' | '-' | '.' if !scheme.is_empty() => scheme.push(c),
            _ => return None,
        }
    }
    None
}

/// What an op does to a page that the door keeps books on (`PageChange`): agent-opened marks
/// for the idle-tab janitor and the task's tab, a refresh of core's cached URL and title after a
/// navigation, and the pages this run closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PageEffect {
    /// Nothing the door tracks.
    Untracked,
    /// `context.newPage`: the tab in the result is agent-opened.
    Opens,
    /// A navigation: the target's URL may have changed, whatever the outcome.
    Navigates,
    /// `page.close` with nothing that can veto it: once it succeeds, the target is gone by this
    /// run's hand.
    Closes,
}

impl PageEffect {
    /// The crate's op names, as the prelude sends them; a test checks each is an op.
    fn of(name: &str) -> Self {
        match name {
            "context.newPage" => Self::Opens,
            "page.goto" | "page.reload" | "page.goBack" | "page.goForward" => Self::Navigates,
            "page.close" => Self::Closes,
            _ => Self::Untracked,
        }
    }

    /// The effect of one call. A close that runs the page's `beforeunload` handlers returns
    /// once the browser took the request (the crate's `Page.close`, as upstream), and a
    /// handler's dialog can still keep the tab open. So only a close nothing can veto is
    /// reported closed, which makes `isClosed()` true at once; the tab watch sees the other kind
    /// go if it does, and the page's facts and `close` event say so from then on.
    fn of_call(self, input: &OpInput) -> Self {
        match self {
            Self::Closes if input.runs_before_unload() => Self::Untracked,
            Self::Untracked | Self::Opens | Self::Navigates | Self::Closes => self,
        }
    }

    fn report<M>(
        self,
        op: &OpCtx<M>,
        target: Option<&PageRef>,
        result: &Result<Value, PlaywrightError>,
    ) {
        match (self, result, target) {
            (Self::Opens, Ok(value), _) => match OpenedPage::deserialize(value) {
                Ok(opened) => op.report_page(&opened.page(), PageChange::Opened),
                Err(error) => tracing::warn!(
                    %error,
                    "context.newPage returned no page brand; the janitor will never close its tab"
                ),
            },
            (Self::Navigates, _, Some(page)) => op.report_page(page, PageChange::Navigated),
            (Self::Closes, Ok(_), Some(page)) => op.report_page(page, PageChange::Closed),
            (Self::Untracked, ..)
            | (Self::Opens | Self::Closes, Err(_), _)
            | (Self::Navigates | Self::Closes, _, None) => {}
        }
    }
}

/// `context.newPage`'s result, `{ page: {"$neo":"page","id","tab"}, url }`: the tab it opened.
#[derive(Deserialize)]
struct OpenedPage {
    page: OpenedBrand,
}

#[derive(Deserialize)]
struct OpenedBrand {
    id: u32,
    tab: i64,
}

impl OpenedPage {
    fn page(&self) -> PageRef {
        PageRef {
            id: PageId(self.page.id),
            tab: TabId(self.page.tab),
        }
    }
}

/// `OpEnv` for one op, over the door's `OpCtx`: the crate reports facts through it, and the door
/// decides what they mean. Generic over the method only so that tests can build one over an op
/// of their own.
pub(super) struct OpCtxEnv<'a, M> {
    op: &'a OpCtx<M>,
    /// The run's one `RunState`, shared by all of its ops and dropped when the run ends.
    run_state: Arc<RunState>,
}

impl<'a, M> OpCtxEnv<'a, M> {
    /// The env for the op that `op` runs.
    pub(super) fn new(op: &'a OpCtx<M>) -> Self {
        Self {
            op,
            run_state: op.run().slot::<RunState>(),
        }
    }
}

impl<M> OpEnv for OpCtxEnv<'_, M> {
    fn view(&self) -> &BrowserView {
        self.op.run().view()
    }

    fn run_state(&self) -> &Arc<RunState> {
        &self.run_state
    }

    fn recheck(&self) -> Result<(), Refusal> {
        // The door keeps a refusal standing as the op's error (`ending_of`), so whatever the
        // crate returns after it, the script sees the rule's own error.
        self.op
            .recheck()
            .map_err(|refused| refusal(refused.error()))
    }

    fn mark_sent(&self) {
        self.op.mark_sent();
    }

    fn mark_sensitive_target(&self) {
        self.op.mark_sensitive_target();
    }

    fn note(&self, line: String) {
        self.op.note(line);
    }
}

/// One of the door's refusals (a rule, a stopped run, a replaced connection) as the crate
/// carries it: owner, code and message unchanged. claw-sdk's errors carry no host code yet.
fn refusal(error: &OperationError) -> Refusal {
    Refusal {
        owner: error.owner(),
        code: error.code().as_str().to_owned(),
        message: error.message().to_owned(),
        host_code: None,
    }
}

/// The crate's failure as the door's. The crate's one error table already decided its owner,
/// whether it is a Playwright timeout (the bootstrap then throws `TimeoutError`) and whether Neo
/// can vouch for what happened, from which the door derives delivery once the op is sent.
pub(super) fn leaf_error(error: &PlaywrightError) -> LeafError {
    let code = match error.error_code() {
        Some(code) => error_code(code),
        None => refusal_code(error),
    };
    let message = error.message();
    let failure = match error.owner() {
        ErrorOwner::Neo => OperationError::neo(code, message),
        ErrorOwner::Script => OperationError::script(code, message),
        ErrorOwner::Website => OperationError::website(code, message),
        ErrorOwner::Host => OperationError::host(code, message),
    };
    let failure = match error.timed_out() {
        TimedOut::Yes => failure.timed_out(),
        TimedOut::No => failure,
    };
    match error.answer() {
        Answer::Definite => LeafError::definite(failure),
        Answer::Ambiguous => LeafError::ambiguous(failure),
    }
}

/// The crate's codes in claw-sdk's one list. Every name carries over as it is, except
/// `internal`: a bug in Neo reads `internal_error` wherever it happens.
fn error_code(code: PlaywrightCode) -> ErrorCode {
    match code {
        PlaywrightCode::StrictModeViolation => ErrorCode::STRICT_MODE_VIOLATION,
        PlaywrightCode::NoMatch => ErrorCode::NO_MATCH,
        PlaywrightCode::NotActionable => ErrorCode::NOT_ACTIONABLE,
        PlaywrightCode::AssertionFailed => ErrorCode::ASSERTION_FAILED,
        PlaywrightCode::ElementDetached => ErrorCode::ELEMENT_DETACHED,
        PlaywrightCode::PageError => ErrorCode::PAGE_ERROR,
        PlaywrightCode::NavigationFailed => ErrorCode::NAVIGATION_FAILED,
        PlaywrightCode::WaitTimeout => ErrorCode::WAIT_TIMEOUT,
        PlaywrightCode::InvalidInput => ErrorCode::INVALID_INPUT,
        PlaywrightCode::Unsupported => ErrorCode::UNSUPPORTED,
        PlaywrightCode::SensitiveCopyRefused => ErrorCode::SENSITIVE_COPY_REFUSED,
        PlaywrightCode::TargetClosed => ErrorCode::TARGET_CLOSED,
        PlaywrightCode::BrowserDisconnected => ErrorCode::BROWSER_DISCONNECTED,
        PlaywrightCode::Internal => ErrorCode::INTERNAL_ERROR,
    }
}

/// The code of one of Neo's own refusals, coming back through the crate. Two paths refuse: the
/// door's rules through `OpEnv::recheck`, whose refusal the door keeps standing as the op's
/// error whatever this returns, and the capture hook, whose only refusal is `page_private`.
fn refusal_code(error: &PlaywrightError) -> ErrorCode {
    if error.code() == ErrorCode::PAGE_PRIVATE.as_str() {
        ErrorCode::PAGE_PRIVATE
    } else {
        ErrorCode::INTERNAL_ERROR
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use browseros_core::{PageId, TabId};
    use browseros_playwright::{ErrorCode as PlaywrightCode, Op, PlaywrightError, Refusal};
    use browseros_trace::ErrorOwner;
    use serde_json::{Value, json};

    use super::{HELD, OpInput, PageEffect, error_code, leaf_error};
    use crate::browser::PageRef;
    use crate::dispatch::{ErrorCode, PageTarget};

    fn decoded(input: Value) -> Result<OpInput, serde_json::Error> {
        serde_json::from_value(input)
    }

    fn brand(input: &Value) -> Result<PageTarget, serde_json::Error> {
        serde_json::from_value(input.clone())
    }

    #[test]
    fn the_page_is_read_from_page_or_from_the_targets_page() -> Result<(), Box<dyn Error>> {
        let page = json!({ "$neo": "page", "id": 7, "tab": 1007 });
        let lazy = json!({ "$neo": "page", "id": null });
        let locator = |page: &Value| json!({ "$neo": "locator", "page": page, "frame": null, "chunks": [], "display": "x" });
        let cases = [
            (json!({ "page": page }), Some(&page)),
            (json!({ "target": locator(&lazy) }), Some(&lazy)),
            // A screenshot names its page and the element on it: the page is the target.
            (
                json!({ "page": page, "target": locator(&lazy) }),
                Some(&page),
            ),
            // A context-wide listener, and an op with no page at all.
            (json!({ "page": null, "event": "page" }), None),
            (json!({ "call": { "api": "browserContext.newPage" } }), None),
        ];
        for (input, expected) in cases {
            let found = decoded(input.clone())?.page;
            let expected = expected.map(brand).transpose()?;
            assert_eq!(found, expected, "{input}");
        }
        Ok(())
    }

    #[test]
    fn a_page_that_is_not_a_full_brand_is_refused_where_it_sits() {
        for (input, place) in [
            // A locator nests the whole page brand (adapter contract), never a bare id.
            (
                json!({ "target": { "$neo": "locator", "page": 7 } }),
                "`target.page`",
            ),
            (json!({ "page": { "$neo": "page", "id": 7 } }), "`page`"),
            (json!(["not", "an", "object"]), "map"),
        ] {
            let refused = decoded(input.clone()).err().map(|error| error.to_string());
            assert!(
                refused
                    .as_deref()
                    .is_some_and(|error| error.contains(place)),
                "{input} is refused naming {place}: {refused:?}"
            );
        }
    }

    #[test]
    fn the_resolved_page_replaces_the_scripts_brand_and_nothing_else() -> Result<(), Box<dyn Error>>
    {
        let resolved = PageRef {
            id: PageId(9),
            tab: TabId(1009),
        };
        let brand = json!({ "$neo": "page", "id": 9, "tab": 1009 });
        let lazy = json!({ "$neo": "page", "id": null });
        let input = json!({
            "call": { "api": "locator.click", "timeoutMs": 5000 },
            "target": { "$neo": "locator", "page": lazy, "frame": null, "chunks": [], "display": "x" },
            "strict": "strict",
        });
        let mut expected = input.clone();
        expected["target"]["page"] = brand.clone();
        assert_eq!(
            decoded(input)?.resolved(Some(&resolved)),
            expected,
            "the crate acts on the tab the door's rules checked"
        );
        let input = json!({ "page": lazy, "url": "https://example.com/", "waitUntil": "load" });
        let mut expected = input.clone();
        expected["page"] = brand;
        assert_eq!(decoded(input)?.resolved(Some(&resolved)), expected);
        Ok(())
    }

    #[test]
    fn every_lazy_brand_in_the_input_becomes_the_page_the_door_resolved()
    -> Result<(), Box<dyn Error>> {
        let resolved = PageRef {
            id: PageId(9),
            tab: TabId(1009),
        };
        let brand = json!({ "$neo": "page", "id": 9, "tab": 1009 });
        let lazy = json!({ "$neo": "page", "id": null });
        let locator = |page: &Value| json!({ "$neo": "locator", "page": page, "frame": null, "chunks": [], "display": "x" });
        // `locator.dragTo`: the locator dragged and the one it is dropped on.
        let drag = json!({ "target": locator(&lazy), "to": locator(&lazy), "strict": "strict" });
        let mut expected = drag.clone();
        expected["target"]["page"] = brand.clone();
        expected["to"]["page"] = brand.clone();
        assert_eq!(
            decoded(drag)?.resolved(Some(&resolved)),
            expected,
            "the crate refuses `id: null`, so the drop target needs the tab too"
        );
        // An element screenshot names the page, and the element on it.
        let shot = json!({ "page": lazy, "target": locator(&lazy), "type": "png" });
        let mut expected = shot.clone();
        expected["page"] = brand.clone();
        expected["target"]["page"] = brand.clone();
        assert_eq!(decoded(shot)?.resolved(Some(&resolved)), expected);
        // Wherever it sits, in a list too.
        let listed = json!({ "page": lazy, "mask": [locator(&lazy)] });
        let mut expected = listed.clone();
        expected["page"] = brand.clone();
        expected["mask"][0]["page"] = brand;
        assert_eq!(decoded(listed)?.resolved(Some(&resolved)), expected);
        Ok(())
    }

    #[test]
    fn a_lazy_brand_beside_another_pages_target_is_left_for_the_crate_to_refuse()
    -> Result<(), Box<dyn Error>> {
        // The run's page has no tab yet, so the lazy brand names a page the door did not resolve
        // for this op; the target's tab would be the wrong page for it.
        let other = PageRef {
            id: PageId(4),
            tab: TabId(1004),
        };
        let lazy = json!({ "$neo": "page", "id": null });
        let locator = |page: &Value| json!({ "$neo": "locator", "page": page, "frame": null, "chunks": [], "display": "x" });
        let drag = json!({
            "target": locator(&json!({ "$neo": "page", "id": 4, "tab": 1004 })),
            "to": locator(&lazy),
        });
        assert_eq!(
            decoded(drag.clone())?.resolved(Some(&other)),
            drag,
            "the crate refuses the lazy brand rather than act on a tab the door never checked"
        );
        Ok(())
    }

    #[test]
    fn a_navigation_runs_script_for_a_javascript_url_however_it_is_spelled()
    -> Result<(), Box<dyn Error>> {
        let page = json!({ "$neo": "page", "id": 7, "tab": 1007 });
        for (url, runs_script) in [
            ("javascript:void(document.title = 'x')", true),
            ("JavaScript:void(0)", true),
            // The browser drops what surrounds a URL, and tabs and newlines inside it.
            (" \u{1}java\tscr\nipt:void(0)", true),
            ("https://bank.example/javascript:void(0)", false),
            (
                "data:text/html,<script>document.title = 'x'</script>",
                false,
            ),
            ("about:blank", false),
            // No scheme at all, so the browser never runs it as script.
            ("/javascript:void(0)", false),
            ("java script:void(0)", false),
        ] {
            let input = decoded(json!({ "page": page, "url": url, "waitUntil": "load" }))?;
            assert_eq!(input.navigates_to_script(), runs_script, "{url:?}");
        }
        Ok(())
    }

    #[test]
    fn every_held_op_is_a_crate_op() {
        for held in HELD {
            assert!(
                Op::from_name(held.name).is_some(),
                "{}: a renamed op would be served instead of held",
                held.name
            );
        }
    }

    #[test]
    fn every_page_effect_names_an_op() {
        for name in [
            "context.newPage",
            "page.goto",
            "page.reload",
            "page.goBack",
            "page.goForward",
            "page.close",
        ] {
            assert!(Op::from_name(name).is_some(), "{name} is a crate op");
            assert_ne!(PageEffect::of(name), PageEffect::Untracked, "{name}");
        }
        assert_eq!(PageEffect::of("page.title"), PageEffect::Untracked);
    }

    #[test]
    fn every_crate_code_keeps_its_name_but_internal() {
        for &code in PlaywrightCode::ALL {
            let expected = match code {
                PlaywrightCode::Internal => "internal_error",
                other => other.as_str(),
            };
            assert_eq!(error_code(code).as_str(), expected, "{code:?}");
        }
    }

    #[test]
    fn a_failure_keeps_the_owner_and_timeout_the_crates_table_gave_it() {
        let cases = [
            (PlaywrightCode::NotActionable, ErrorOwner::Website, true),
            (PlaywrightCode::NoMatch, ErrorOwner::Script, true),
            (PlaywrightCode::WaitTimeout, ErrorOwner::Website, true),
            (
                PlaywrightCode::StrictModeViolation,
                ErrorOwner::Script,
                false,
            ),
            (PlaywrightCode::Internal, ErrorOwner::Neo, false),
        ];
        for (code, owner, timed_out) in cases {
            let message = "locator.click: Timeout 5000ms exceeded.\nCall log:\n  - waiting for x\n";
            let leaf = leaf_error(&PlaywrightError::new(code, message.to_owned()));
            let error = leaf.error();
            assert_eq!(error.owner(), owner, "{code:?}");
            assert_eq!(error.code(), error_code(code), "{code:?}");
            assert_eq!(
                error.is_timeout(),
                timed_out,
                "{code:?}: a Playwright timeout throws TimeoutError"
            );
            assert_eq!(error.message(), message, "the call log reaches the script");
        }
    }

    #[test]
    fn the_capture_hooks_refusal_comes_back_as_page_private() {
        let leaf = leaf_error(&PlaywrightError::from(Refusal {
            owner: ErrorOwner::Host,
            code: "page_private".to_owned(),
            message: "this tab is private (takeover)".to_owned(),
            host_code: None,
        }));
        let error = leaf.error();
        assert_eq!(
            (error.owner(), error.code(), error.is_timeout()),
            (ErrorOwner::Host, ErrorCode::PAGE_PRIVATE, false)
        );
        assert_eq!(error.message(), "this tab is private (takeover)");
    }
}
