//! neo-playwright's ops, served through the door (sdk-skeleton decision 23, and the
//! Playwright adapter contract in the program's CONTEXT.md).
//!
//! ```text
//!  prelude ─ __neo.invoke(op, json) ─ Dispatcher::submit ─ OpInput: the brand at `page` or
//!    `target.page` ─ the door resolves it to one tab and runs its rules on that tab
//!      ─ PlaywrightOp::call: the resolved page written back into the input
//!          ─ Playwright::invoke(OpCtxEnv, op, json), raced with the run's stop
//!              ├─ recheck, mark_sent, note ─▶ OpCtx
//!              └─ PlaywrightError ─ leaf_error ─▶ LeafError: owner, code, timeout, answer
//!          ─ opened, navigated and closed pages reported to the door
//!          ─ an event pull's events: each one's tabs through the rules (event_release.rs)
//! ```
//!
//! The crate owns what each op does and how its failures read; the door keeps its own chores
//! (the trace, the rules, delivery and page facts). This file only translates between
//! the two. An op that [`OpMarks`] holds is the exception: its name answers a refusal, and the
//! crate never sees the call.

use std::future::{self, Future};
use std::sync::{Arc, LazyLock};

use browseros_core::{BrowserView, TabId, TargetId};
use futures_util::future::BoxFuture;
use neo_playwright::{
    Answer, ErrorCode as PlaywrightCode, Op, OpEnv, PlaywrightError, Refusal, RunState, TimedOut,
};
use neo_trace::ErrorOwner;
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value, json};

use super::PlaywrightFacade;
use super::event_release::EventRelease;
use super::op_marks::{self, Hold, OpMarks};
use crate::browser::PageRef;
use crate::dispatch::{
    ErrorCode, LeafError, Method, MethodSpec, OpCtx, OperationError, PageChange, PageTarget,
    RegistryBuilder, RegistryError, RunCtx,
};

/// neo-playwright's prelude, which the agent realm evaluates after its bootstrap: the
/// `page`, `context`, `browser` and `expect` parameters of the agent's function, over these ops.
/// Here so that `runtime/`, which never names a browser crate (STYLE.md P2), can install it.
pub(crate) const PLAYWRIGHT_PRELUDE: &str = neo_playwright::PRELUDE_JS;

/// The `run` description's one line for every served Playwright op: the families a model reaches
/// for, what the prelude refuses, and then each op [`OpMarks`] holds and why ([`op_marks::held_clause`]),
/// so a model does not spend a call finding out. Built once, when the first op registers; a
/// `static`, so `Listing::Grouped` gets the `&'static str` it takes without a leak.
static PLAYWRIGHT_LINE: LazyLock<String> = LazyLock::new(|| {
    let mut line = PLAYWRIGHT_API.to_owned();
    if let Some(held) = op_marks::held_clause(Op::all().map(Op::spec)) {
        line.push(' ');
        line.push_str(&held);
    }
    line
});

/// What [`PLAYWRIGHT_LINE`] says whatever is held: what Playwright 1.64.1 does, and then each way
/// Neo's differs that a script would trip on. The main-world scripts' and `setContent`'s
/// differences are the crate's (tier 1 `deviations.yaml`), but for the result depth, which the
/// realm's stack bounds (`Defaults::JS_STACK_BYTES`, measured on BrowserOS in a debug build).
const PLAYWRIGHT_API: &str = "Playwright's API on `page`, `context` and `browser`, as \
    Playwright 1.64.1 behaves where this does not say otherwise: locators (`getByRole`, `getByText`, `getByLabel`, `getByTestId`, `locator`, \
    `frameLocator`, `filter`, `nth`) that wait and are strict, frames (`page.frames()`, \
    `frame.locator`), reads (`textContent`, `innerText`, `getAttribute`, `inputValue`, \
    `count`, `isVisible`, `page.content()`, `page.title()`), actions that wait until the \
    element is actionable (`click`, `dblclick`, `hover`, `fill`, `clear`, `press`, \
    `pressSequentially`, `check`, `uncheck`, `setChecked`, `selectOption`, `selectText`, \
    `focus`, `blur`, `dragTo`, `drop`, `dispatchEvent`, `scrollIntoViewIfNeeded`; no `tap`), \
    `page.keyboard` (`press`, `type`, `insertText`, `down`, `up`), `page.mouse` (`move`, \
    `down`, `up`, `click`, `dblclick`, `wheel`), navigation (`goto`, `reload`, `goBack`, \
    `goForward`, `waitForURL`, `waitForLoadState`, `waitForNavigation`), scripts in the page's \
    main world (`page.evaluate`, \
    `page.waitForFunction`, `locator.evaluate`, `locator.evaluateAll`, \
    `locator.waitForFunction`, `elementHandle.evaluate`, \
    and `$eval` and `$$eval` on a page, frame or element handle; a `frame` form runs in its \
    frame, a child frame too) with Playwright's values, arguments, element handles and errors, \
    `page.requestGC` (its main renderer's garbage collection), \
    `page.emulateMedia` (per page across runs and navigations), `page.setViewportSize` (the \
    tab keeps the size in later runs, until it closes), \
    `page.addScriptTag` and `page.addStyleTag` (also on frames), returning element handles, \
    `page.setContent` and `frame.setContent`, whose HTML's scripts run there too, init scripts \
    (`page.addInitScript`, `context.addInitScript`) that every new document runs in its main \
    world, bindings the page calls back into the script with (`exposeBinding`, \
    `exposeFunction`, on a page or the context, and the `exposeFunctions` option of \
    `addInitScript` and of `evaluate`; the prelude answers each call with \
    `bindingCall.resolve`), \
    `screenshot` and `pdf`, events (`page.on`, `waitForEvent`), downloads \
    (`download.path()` waits for the file and returns where the browser saved it, which a later \
    run can upload with `setInputFiles`; `saveAs(path)` copies it; `failure()`), and `expect`: \
    retrying locator matchers (`toBeVisible`, `toBeChecked`, `toHaveText`, `toContainText`, \
    `toHaveValue`, `toHaveCount`, `toHaveAttribute`, ...) and page matchers (`toHaveTitle`, \
    `toHaveURL`), with `.not`, value matchers (`toBe`, `toEqual`, ...), `expect.poll` and \
    `toPass`; soft assertions, `toBeOK` and screenshot, snapshot and `toMatchAriaSnapshot` \
    comparisons are not available. In hosted BrowserOS, console and pageerror listeners are \
    accepted but stay silent; `page.waitForEvent` for either fails immediately with \
    `neo/unsupported`. A call that waits gives up after 5 s unless given a `timeout`, \
    and a navigation after 30 s. Unlike Playwright, `waitForFunction` \
    polls every 100 ms for its default `polling: 'raf'`, since animation frames never fire in \
    Neo's background tabs, and the handle it resolves to holds a snapshot of the value, which \
    `jsonValue()` reads; `setContent` refuses `waitUntil: 'networkidle'`, which Neo cannot \
    observe after a write, with nothing sent; and a result nested more than about 50 objects \
    deep can fail `Maximum call stack size exceeded`, where Playwright allows 98. `goto` opens \
    `http`, `https`, `data`, `about`, `file` and `javascript` URLs; Chromium executes a \
    `javascript:` URL before reporting `net::ERR_ABORTED`. `reload` opens only `http`, `https`, \
    `data` and `about` URLs. `setInputFiles` uploads any \
    file Neo's process can read, as Playwright does \
    (a relative path resolves against Neo's working directory): the caller sandboxes the \
    filesystem. `{ name, mimeType, buffer }` payloads need no file and total under 10 MiB a \
    call. A directory upload needs its tab in front, and the tabs Neo opens for an agent stay \
    in the background (`bringToFront` is refused). An init script or a binding stays \
    registered until the run ends, then its tab no longer has it; a context's reaches each tab \
    before the run's first write to it, so a popup's first document and cross-site iframes run \
    without it. A page's call of a binding reaches the script through the event pump, so a \
    call the page does not await (from an event handler) may run after the op that caused it \
    returned. A function `evaluate` passes stays exposed until the run ends; `evaluateHandle` \
    has no `exposeFunctions`.";

/// At a run's end, after its ops drained: removes the init scripts its ops registered from every
/// tab, awaiting each removal (neo-playwright's `RunState::remove_init_scripts`). The run service
/// calls it before it answers, so a script ends with the run that registered it.
pub(crate) async fn remove_init_scripts(run: &RunCtx) {
    run.slot::<RunState>().remove_init_scripts().await;
}

/// Schemes shared by goto and reload. Goto additionally passes file and JavaScript URLs to
/// Chromium; other browser-specific schemes remain unsupported.
const NAVIGABLE_SCHEMES: [&str; 4] = ["http", "https", "data", "about"];

/// Registers every neo-playwright op as a script method, as [`OpMarks`] decides from its
/// declarations: a held op as its refusal alone, every other one as a [`PlaywrightOp`]. Called
/// once at startup; a duplicate name fails the start.
pub(super) fn playwright_ops(
    builder: &mut RegistryBuilder,
    facade: &Arc<PlaywrightFacade>,
) -> Result<(), RegistryError> {
    for op in Op::all() {
        let OpMarks { spec, hold } = OpMarks::of(&op.spec(), PLAYWRIGHT_LINE.as_str());
        match hold {
            Some(hold) => builder.define(HeldOp { spec, hold })?,
            None => builder.define(PlaywrightOp {
                op,
                spec,
                facade: Arc::clone(facade),
                pages: PageEffect::of(spec.name),
                events: EventRelease::of(spec.name),
            })?,
        }
    }
    Ok(())
}

/// A held op's name, served by its refusal alone: the crate's op never runs. It names no page,
/// so the door refuses it before it would open the run's lazy page, and it reads no input, so the
/// answer is the hold whatever the script sent. Its spec is the crate op's own, as [`OpMarks`]
/// derived it, so the trace records the attempt as the op it stands for; unlisted, since the
/// `run` description names the hold.
struct HeldOp {
    spec: MethodSpec,
    hold: Hold,
}

impl Method for HeldOp {
    type Input = Value;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        self.spec
    }

    fn target(_input: &Value) -> Option<&PageTarget> {
        None
    }

    fn call(
        &self,
        _op: &OpCtx<Self>,
        _input: Value,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        future::ready(Err(LeafError::definite(self.hold.refusal(self.spec.name))))
    }
}

/// One neo-playwright op as a dispatcher method. Every op is this one type: the op is
/// data (`Op`), and so is all the door reads about it, so the crate's op table is the only list.
struct PlaywrightOp {
    op: Op,
    /// The crate's spec with the door's marks, derived once by [`OpMarks`].
    spec: MethodSpec,
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
        self.spec
    }

    fn target(input: &OpInput) -> Option<&PageTarget> {
        input.page.as_ref()
    }

    async fn call(&self, op: &OpCtx<Self>, input: OpInput) -> Result<Value, LeafError> {
        if self.pages == PageEffect::Navigates
            && let Some(refusal) = input.refused_navigation()
        {
            return Err(LeafError::definite(refusal));
        }
        let page = match &input.page {
            Some(_) => Some(op.page()?),
            None => None,
        };
        if let (PageEffect::Reloads, Some(page)) = (self.pages, page)
            && let Some(refusal) = refused_reload(op, page).await?
        {
            return Err(LeafError::definite(refusal));
        }
        let pages = self.pages.of_call(&input);
        let input = input.resolved(page);
        let playwright = self.facade.get().await;
        let env = OpCtxEnv::new(op, self.facade.hosted_mode());
        let result = op
            .race_classified(playwright.invoke(&env, self.op, input))
            .await;
        // Read once the op ended, failed or not: by then the pump replies that carry the events
        // before it are counted (`Playwright::events_before_reply`).
        op.answer_after_pump(env.run_state().pump_replies());
        // SDK readiness → method facts → OpReply → runtime; the bridge never reads SDK state.
        op.report_publications(env.run_state().completed_navigation_loads());
        let result = result?;
        pages.report(op, page, &result);
        let value = result.map_err(|error| leaf_error(&error))?;
        self.events.release(op, value)
    }
}

/// A Playwright op's input as the door decodes it: the one fact the door needs, the page the op
/// acts on, and the rest left as JSON for the crate, which decodes it once itself, with
/// `deny_unknown_fields`.
struct OpInput {
    /// The page brand `{"$neo":"page","tab"}` the op acts on (adapter contract, point 1).
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

    /// Rejects unsupported URL schemes before sending a navigation. `frame.goto` uses the
    /// same op; reload checks the current URL instead, and history follows a previous entry.
    fn refused_navigation(&self) -> Option<OperationError> {
        let url = self.json.get("url")?.as_str()?;
        // The engine's completeUserURL supplies http:// for bare local addresses.
        // Validate that completed scheme rather than mistaking localhost:port for a scheme.
        if url.starts_with("localhost") || url.starts_with("127.0.0.1") {
            return None;
        }
        let which = match url_scheme(url) {
            Some(scheme)
                if NAVIGABLE_SCHEMES.contains(&scheme.as_str())
                    || matches!(scheme.as_str(), "file" | "javascript") =>
            {
                return None;
            }
            Some(scheme) => format!("this one is `{scheme}:`"),
            None => {
                "this one has no scheme; write it whole, such as https://example.com/".to_owned()
            }
        };
        Some(not_navigable(&which))
    }

    /// The input as the crate gets it (adapter contract, point 2; decision 25, input 2). The
    /// lazy brand `{"$neo":"page","tab": null}` names the run's page wherever it sits, so once the
    /// door resolved that page for this op, every one of them becomes it: `dragTo`'s `to.page`,
    /// a screenshot's `page` and `target.page`. The crate refuses `tab: null` on purpose: it acts
    /// on the very tab the door's rules checked, which a second lookup of its own could disagree
    /// with (two lazy pages, a tab that changed in between). For the same reason a lazy brand
    /// beside another page's target is left for the crate to refuse: the door checked that other
    /// tab, not the run's page. A target that names its tab needs nothing written back, since
    /// the door resolves it to the same tab.
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

/// Checks the tab's current scheme before reload. Core's cached URL can trail a navigation
/// the page made on its own, so refresh once through `Browser.getTabInfo` first.
async fn refused_reload<M>(
    op: &OpCtx<M>,
    _page: &PageRef,
) -> Result<Option<OperationError>, LeafError> {
    let shown = op
        .race(op.run().view().page(op.core_page()?.clone()))
        .await?;
    Ok(match url_scheme(&shown.url) {
        Some(scheme) if NAVIGABLE_SCHEMES.contains(&scheme.as_str()) => None,
        Some(scheme) => Some(not_navigable(&format!(
            "this page is on a `{scheme}:` one, so Neo does not reload it"
        ))),
        None => Some(not_navigable(
            "this page's URL has no scheme Neo can read, so Neo does not reload it",
        )),
    })
}

/// A navigation the door does not send: `script / invalid_input`, delivery `not_sent`.
fn not_navigable(which: &str) -> OperationError {
    OperationError::script(
        ErrorCode::INVALID_INPUT,
        format_args!("Neo does not open this URL: {which}."),
    )
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
    /// The result names a related tab whose current fact belongs in the reply.
    Links,
    /// A navigation: the target's URL may have changed, whatever the outcome.
    Navigates,
    /// `page.reload`: a navigation too, to the URL the target is on, which the door checks
    /// first ([`refused_reload`]).
    Reloads,
    /// `page.close` with nothing that can veto it: once it succeeds, the target is gone by this
    /// run's hand.
    Closes,
}

impl PageEffect {
    /// The crate's op names, as the prelude sends them; a test checks each is an op.
    fn of(name: &str) -> Self {
        match name {
            "context.newPage" => Self::Opens,
            "page.opener" => Self::Links,
            // A successful navigation wait observes a URL change started by another op or
            // the page. Refresh core's cache too, so a later ordinary reply cannot restore
            // the pre-navigation URL over the wait's fresh frame facts.
            "page.goto"
            | "page.goBack"
            | "page.goForward"
            | "page.waitForNavigation"
            | "page.waitForURL"
            | "page.waitForLoadState"
            | "page.evaluate" => Self::Navigates,
            "page.reload" => Self::Reloads,
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
            Self::Untracked
            | Self::Opens
            | Self::Links
            | Self::Navigates
            | Self::Reloads
            | Self::Closes => self,
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
            (Self::Links, Ok(value), _) => {
                #[derive(Deserialize)]
                struct Linked {
                    opener: Option<OpenedBrand>,
                }
                if let Ok(Linked { opener: Some(page) }) = Linked::deserialize(value) {
                    op.report_page(
                        &PageRef {
                            tab: TabId(page.tab),
                        },
                        PageChange::Linked,
                    );
                }
            }
            (Self::Navigates | Self::Reloads, _, Some(page)) => {
                op.report_page(page, PageChange::Navigated);
            }
            (Self::Closes, Ok(_), Some(page)) => op.report_page(page, PageChange::Closed),
            (Self::Untracked, ..)
            | (Self::Opens | Self::Links | Self::Closes, Err(_), _)
            | (Self::Navigates | Self::Reloads | Self::Closes, _, None) => {}
        }
    }
}

/// `context.newPage`'s result, `{ page: {"$neo":"page","tab"}, url }`: the tab it opened.
#[derive(Deserialize)]
struct OpenedPage {
    page: OpenedBrand,
}

#[derive(Deserialize)]
struct OpenedBrand {
    tab: i64,
}

impl OpenedPage {
    fn page(&self) -> PageRef {
        PageRef {
            tab: TabId(self.page.tab),
        }
    }
}

/// `OpEnv` for one op, over the door's `OpCtx`: the crate reports facts through it, and the door
/// decides what they mean. Generic over the method only so that tests can build one over an op
/// of their own.
pub(super) struct OpCtxEnv<'a, M> {
    op: &'a OpCtx<M>,
    hosted: bool,
    /// The run's one `RunState`, shared by all of its ops and dropped when the run ends.
    run_state: Arc<RunState>,
}

impl<'a, M> OpCtxEnv<'a, M> {
    /// The env for the op that `op` runs.
    pub(super) fn new(op: &'a OpCtx<M>, hosted: bool) -> Self {
        Self {
            op,
            hosted,
            run_state: op.run().slot::<RunState>(),
        }
    }
}

impl<M> OpEnv for OpCtxEnv<'_, M> {
    fn hosted_mode(&self) -> bool {
        self.hosted
    }

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

    fn note(&self, line: String) {
        self.op.note(line);
    }

    fn report_target(&self, tab: TabId, target: &TargetId) {
        self.op.report_page(
            &PageRef { tab },
            PageChange::Acted {
                target: target.clone(),
            },
        );
    }

    fn report_close(&self, tab: TabId, target: &TargetId, seq: u64) {
        self.op.report_page(
            &PageRef { tab },
            PageChange::Gone {
                target: target.clone(),
                seq,
            },
        );
    }

    fn report_navigation(&self, tab: TabId) {
        self.op.report_page(&PageRef { tab }, PageChange::Navigated);
    }

    fn abort_reason(&self) -> Option<String> {
        self.op.abort_reason()
    }

    fn aborted(&self) -> BoxFuture<'_, ()> {
        Box::pin(self.op.aborted())
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
        Some(code) => Some(error_code(code)),
        None => refusal_code(error),
    };
    let message = error.message();
    let failure = match code {
        Some(code) => match error.owner() {
            ErrorOwner::Neo => OperationError::neo(code, message),
            ErrorOwner::Script => OperationError::script(code, message),
            ErrorOwner::Website => OperationError::website(code, message),
            ErrorOwner::Host => OperationError::host(code, message),
        },
        // Neo and its crate disagree on the codes, whoever the refusal named.
        None => OperationError::neo(ErrorCode::INTERNAL_ERROR, message),
    };
    let failure = match error.timed_out() {
        TimedOut::Yes => failure.timed_out(),
        TimedOut::No => failure,
    };
    let failure = match error.matcher_details() {
        Some(facts) => failure.with_matcher(facts.clone()),
        None => failure,
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
        PlaywrightCode::DownloadFailed => ErrorCode::DOWNLOAD_FAILED,
        PlaywrightCode::WaitTimeout => ErrorCode::WAIT_TIMEOUT,
        PlaywrightCode::NavigationObservationTimeout => ErrorCode::NAVIGATION_OBSERVATION_TIMEOUT,
        PlaywrightCode::InvalidInput => ErrorCode::INVALID_INPUT,
        PlaywrightCode::Unsupported => ErrorCode::UNSUPPORTED,
        PlaywrightCode::TargetClosed => ErrorCode::TARGET_CLOSED,
        PlaywrightCode::BrowserDisconnected => ErrorCode::BROWSER_DISCONNECTED,
        PlaywrightCode::Aborted => ErrorCode::ABORTED,
        PlaywrightCode::Internal => ErrorCode::INTERNAL_ERROR,
    }
}

/// The code of one of Neo's own refusals, coming back through the crate as a string, or `None`
/// when claw-sdk does not know it. The door's rules refuse through `OpEnv::recheck`, whose
/// refusal the door keeps standing as the op's error whatever this returns; each keeps its code,
/// so the agent reads who must act.
fn refusal_code(error: &PlaywrightError) -> Option<ErrorCode> {
    ErrorCode::known(error.code())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::error::Error;

    use browseros_core::TabId;
    use neo_playwright::{ErrorCode as PlaywrightCode, Op, PlaywrightError, Refusal};
    use neo_trace::ErrorOwner;
    use serde_json::{Value, json};

    use super::{Hold, OpInput, PLAYWRIGHT_LINE, PageEffect, error_code, leaf_error, op_marks};
    use crate::browser::PageRef;
    use crate::dispatch::{ErrorCode, PageTarget};

    fn decoded(input: Value) -> Result<OpInput, serde_json::Error> {
        serde_json::from_value(input)
    }

    fn brand(input: &Value) -> Result<PageTarget, serde_json::Error> {
        serde_json::from_value(input.clone())
    }

    #[test]
    fn the_run_description_explains_hosted_console_and_pageerror_limits() {
        let description = PLAYWRIGHT_LINE.as_str();
        assert!(description.contains("In hosted BrowserOS, console and pageerror listeners"));
        assert!(description.contains("accepted but stay silent"));
        assert!(description.contains("fails immediately with `neo/unsupported`"));
    }

    #[test]
    fn the_page_is_read_from_page_or_from_the_targets_page() -> Result<(), Box<dyn Error>> {
        let page = json!({ "$neo": "page", "tab": 1007 });
        let lazy = json!({ "$neo": "page", "tab": null });
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
        let resolved = PageRef { tab: TabId(1009) };
        let brand = json!({ "$neo": "page", "tab": 1009 });
        let lazy = json!({ "$neo": "page", "tab": null });
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
        let resolved = PageRef { tab: TabId(1009) };
        let brand = json!({ "$neo": "page", "tab": 1009 });
        let lazy = json!({ "$neo": "page", "tab": null });
        let locator = |page: &Value| json!({ "$neo": "locator", "page": page, "frame": null, "chunks": [], "display": "x" });
        // `locator.dragTo`: the locator dragged and the one it is dropped on.
        let drag = json!({ "target": locator(&lazy), "to": locator(&lazy), "strict": "strict" });
        let mut expected = drag.clone();
        expected["target"]["page"] = brand.clone();
        expected["to"]["page"] = brand.clone();
        assert_eq!(
            decoded(drag)?.resolved(Some(&resolved)),
            expected,
            "the crate refuses `tab: null`, so the drop target needs the tab too"
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
        let other = PageRef { tab: TabId(1004) };
        let lazy = json!({ "$neo": "page", "tab": null });
        let locator = |page: &Value| json!({ "$neo": "locator", "page": page, "frame": null, "chunks": [], "display": "x" });
        let drag = json!({
            "target": locator(&json!({ "$neo": "page", "tab": 1004 })),
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
    fn a_navigation_is_allowed_by_its_scheme_however_it_is_spelled() -> Result<(), Box<dyn Error>> {
        let page = json!({ "$neo": "page", "tab": 1007 });
        for (url, allowed) in [
            ("https://bank.example/file:///etc/hosts", true),
            ("HTTP://bank.example/", true),
            ("data:text/html,<script>document.title = 'x'</script>", true),
            ("about:blank", true),
            ("file:///etc/hosts", true),
            ("JavaScript:void(0)", true),
            // The browser drops what surrounds a URL, and tabs and newlines inside it.
            (" \u{1}fi\tl\ne:///etc/hosts", true),
            (" \u{1}java\tscr\nipt:void(0)", true),
            ("view-source:https://bank.example/", false),
            // No scheme the URL Standard reads, so none to allow.
            ("/etc/hosts", false),
            ("java script:void(0)", false),
        ] {
            let input = decoded(json!({ "page": page, "url": url, "waitUntil": "load" }))?;
            assert_eq!(input.refused_navigation().is_none(), allowed, "{url:?}");
        }
        let reload = decoded(json!({ "page": page, "waitUntil": "load" }))?;
        assert!(
            reload.refused_navigation().is_none(),
            "a reload names no URL: the door checks the one the page is on (refused_reload)"
        );
        Ok(())
    }

    /// Every crate op that `text` names in backticks, by its wire name.
    fn ops_named(text: &str) -> BTreeSet<&str> {
        text.split('`')
            .skip(1)
            .step_by(2)
            .filter(|name| Op::from_name(name).is_some())
            .collect()
    }

    /// A model reads which ops are held before it writes a call, so the line names each one the
    /// op table holds and no other, and nowhere else offers one. With none held, as on this door,
    /// the clause is empty.
    #[test]
    fn the_description_names_each_held_op_and_only_those() {
        let held: BTreeSet<&str> = Op::all()
            .map(Op::spec)
            .filter(|spec| Hold::of(spec).is_some())
            .map(|spec| spec.name)
            .collect();
        let clause = op_marks::held_clause(Op::all().map(Op::spec)).unwrap_or_default();
        let line: &str = &PLAYWRIGHT_LINE;
        assert!(line.contains(&clause), "the line says what is held: {line}");
        assert_eq!(
            ops_named(&clause),
            held,
            "the clause names each held op and only those: {clause}"
        );
        let rest = line.replacen(&clause, "", 1);
        let offered = ops_named(&rest);
        assert!(
            offered.is_disjoint(&held),
            "the rest of the line offers no held op: {offered:?}"
        );
        let served = Op::all()
            .map(Op::spec)
            .filter(|spec| Hold::of(spec).is_none());
        assert_eq!(
            op_marks::held_clause(served),
            None,
            "with nothing held, the line adds nothing"
        );
    }

    /// A model calls only what the line offers, so outside the held clause it names each op the
    /// door serves that reaches past the page's isolated world: by its wire name, or by the method a
    /// script calls (`pdf`, `setInputFiles`).
    #[test]
    fn the_line_offers_every_served_op_that_reaches_past_the_isolated_world() {
        let line: &str = &PLAYWRIGHT_LINE;
        let offered = match op_marks::held_clause(Op::all().map(Op::spec)) {
            Some(clause) => line.replacen(&clause, "", 1),
            None => line.to_owned(),
        };
        for spec in Op::all()
            .map(Op::spec)
            .filter(|spec| !spec.capabilities.is_empty() && Hold::of(spec).is_none())
        {
            let method = spec
                .name
                .rsplit_once('.')
                .map_or(spec.name, |(_, method)| method);
            assert!(
                offered.contains(&format!("`{}`", spec.name))
                    || offered.contains(&format!("`{method}`")),
                "{} is served, so the line offers it: {offered}",
                spec.name
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

    /// A refusal the crate hands back reads as whoever refused made it, so the agent learns who
    /// must act and why, never a bug in Neo.
    #[test]
    fn a_refusal_the_crate_hands_back_keeps_its_owner_and_code() {
        let cases = [
            // The crate's own, for a Playwright feature Neo does not support here.
            (
                ErrorOwner::Neo,
                "unsupported",
                "Neo refused this call: the popup opened as a window",
            ),
            // The door's rules, through `OpEnv::recheck`.
            (
                ErrorOwner::Host,
                "owner_has_control",
                "the owner has control of this tab",
            ),
            (
                ErrorOwner::Script,
                "method_unavailable",
                "this method is not available here",
            ),
        ];
        let read: Vec<_> = cases
            .iter()
            .map(|&(owner, code, message)| {
                let leaf = leaf_error(&PlaywrightError::from(Refusal {
                    owner,
                    code: code.to_owned(),
                    message: message.to_owned(),
                    host_code: None,
                }));
                let error = leaf.error();
                (
                    error.owner(),
                    error.code().as_str(),
                    error.is_timeout(),
                    error.message().to_owned(),
                )
            })
            .collect();
        let made: Vec<_> = cases
            .iter()
            .map(|&(owner, code, message)| (owner, code, false, message.to_owned()))
            .collect();
        assert_eq!(read, made, "the agent reads each refusal as it was made");
    }

    /// A code claw-sdk does not know means Neo and its Playwright disagree on the list: a bug in
    /// Neo, whoever the refusal named.
    #[test]
    fn a_refusal_under_a_code_neo_does_not_know_reads_as_a_bug_in_neo() {
        let leaf = leaf_error(&PlaywrightError::from(Refusal {
            owner: ErrorOwner::Host,
            code: "not_a_neo_code".to_owned(),
            message: "refused".to_owned(),
            host_code: None,
        }));
        let error = leaf.error();
        assert_eq!(
            (error.owner(), error.code()),
            (ErrorOwner::Neo, ErrorCode::INTERNAL_ERROR),
            "the host did not make that code, so it is not the host's to fix"
        );
    }
}
