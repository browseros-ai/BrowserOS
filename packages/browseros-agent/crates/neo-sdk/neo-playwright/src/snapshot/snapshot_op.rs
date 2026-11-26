//! The snapshot ops: `page.ariaSnapshot` and `locator.ariaSnapshot`, and their JSON twins
//! `page.ariaSnapshotJSON` and `locator.ariaSnapshotJSON`, read from the tab's APC tree
//! (`tree_snapshot.rs`): one capture holds every frame, child frames in place, so nothing is
//! read frame by frame or stitched.
//!
//! ```text
//!  page     capture ─ TabTree::aria(Page) ─ marked: the cover survey's `# on top:` lines
//!  locator  the resolver pins the one element ─ capture ─ TabTree::aria(its subtree)
//!
//!  then, by the input:
//!    no `format`          the tree, which the prelude renders   (until piece 8)
//!    `format`             the text, printed by neo_snapshot::render (snapshot_text.rs)
//!    `format`, `compare`  both, from this one read (test harnesses only)
//!    ariaSnapshotJSON     `{ snapshot: tree }`, never marked
//! ```
//!
//! Marked (`clickable` on the tree path, Neo's format or `mode: 'full'` on the text path, in
//! mode `ai` only): the page's cover survey and the closed-root seeds. What Chromium reports
//! clickable and what is inert come with the tree itself.
//!
//! A snapshot only reads, so it never calls `send_effect`.
//!
//! The tab's delta baseline (`snapshot_baselines.rs`, `pending_baselines.rs`): on the text path a
//! whole page in Neo's format reads the stored text and records its own as pending itself, after
//! the event wait it takes inside the handler (`snapshot_text.rs`). On the tree path the prelude
//! does both through `page.snapshotBaseline` and `page.snapshotPending`, Neo's own ops, once it
//! has rendered the text; the host commits or drops what is pending when the run ends.

use neo_snapshot::AriaNode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::Instant;
use ts_rs::TS;

use super::aria_snapshot::SnapshotResult;
use super::snapshot_text::{
    self, Finish, LocatorFormat, PageFormat, PreparedFacts, ScopeClock, SnapshotAnswer,
    SnapshotCompared, SnapshotText, TabStore, TextAsk,
};
use super::tree_snapshot::{self, PreparedSnapshot, TreeRead};
use super::{Baseline, SnapshotForm};
use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, PAGE, TARGET};
use crate::page::OpPage;
use crate::probe::SnapshotMode;
use crate::target::{PageRef, Strictness, Target};
use crate::wire::{self, Describe, Nothing, OpWire, Route, Run, options};
use crate::{ErrorCode, Playwright, PlaywrightError};

listed_enum! {
    /// Aria snapshots of a whole page (every frame stitched in) or of one element, as text or as
    /// the tree, and the tab's delta baseline: read, and a run's text recorded as pending.
    pub(crate) enum SnapshotOp {
        Page,
        Locator,
        PageJson,
        LocatorJson,
        Baseline,
        Pending,
    }
}

impl SnapshotOp {
    /// Describes the same input and result types the runtime route decodes and encodes.
    pub(crate) fn wire(self) -> OpWire {
        route(self, Describe)
    }

    /// This op's registration metadata.
    pub(crate) const fn spec(self) -> OpSpec {
        match self {
            Self::Page => OpSpec::read(
                "page.ariaSnapshot",
                PAGE,
                "the page's aria snapshot with its frames stitched in, as the text `format` \
                 names, or the tree without one; mode 'ai' mints refs, and Neo's formats (or \
                 `clickable`) mark what Chromium reports clickable",
            ),
            Self::Locator => OpSpec::read(
                "locator.ariaSnapshot",
                TARGET,
                "an aria snapshot rooted at one element, as the text `format` names, or the tree \
                 without one; mode 'ai' mints refs, and Neo's formats (or `clickable`) mark \
                 what Chromium reports clickable",
            ),
            Self::PageJson => OpSpec::read(
                "page.ariaSnapshotJSON",
                PAGE,
                "the page's aria snapshot tree with its frames stitched in, unmarked; mode 'ai' \
                 mints refs",
            ),
            Self::LocatorJson => OpSpec::read(
                "locator.ariaSnapshotJSON",
                TARGET,
                "the aria snapshot tree rooted at one element, unmarked; mode 'ai' mints refs",
            ),
            // Reads: they touch only text Neo keeps, never the page.
            Self::Baseline => OpSpec::read(
                "page.snapshotBaseline",
                PAGE,
                "the tab's delta baseline: the last whole-page v1 snapshot text of that \
                 document its caller was given, with its form, or null",
            ),
            Self::Pending => OpSpec::read(
                "page.snapshotPending",
                PAGE,
                "records a whole-page v1 snapshot's text, its form, and the string the call \
                 returned, as the run's pending baseline for its tab",
            ),
        }
    }
}

/// One exhaustive table binds each snapshot's handler to the upstream command. `Run` and
/// `Describe` walk these same rows; the policy in `spec()` remains independent of the shapes.
fn route<'a, R: Route<'a>>(op: SnapshotOp, r: R) -> R::Out {
    let command = wire::command("Frame.ariaSnapshot")
        .adds(&[
            (
                "clickable",
                "Neo also marks elements Chromium reports clickable without a pointer cursor, so \
                 the condensed snapshot keeps their refs; read over CDP, never a script in the \
                 page. The tree path's only: with a `format`, the format decides the marking",
            ),
            (
                "format",
                "which text Neo prints: Playwright's, `mode: 'full'` or Neo's condensed format, \
                 with the options the prelude checked; Rust prints it from the stitched tree, \
                 since the page's world must not hold cross-site frames' text",
            ),
            (
                "compare",
                "a test harness's switch while two engines print the text: the tree and Rust's \
                 text from one read, for the prelude to compare with its own",
            ),
        ])
        .result(
            "Each frame masks page-declared private field values outside conformance, and Rust \
             stitches the child frames. With a `format` it answers upstream's `{ snapshot }` \
             text, Neo's formats marking what Chromium reports clickable; without one, the \
             JSON tree plus the frames it could not read and, for a whole page, the document it \
             read for the delta baseline, which the prelude renders (both until the prelude's \
             renderer is removed); with `compare`, the tree and Rust's text beside it",
        );
    match op {
        SnapshotOp::Page => r.op(command, page_snapshot),
        SnapshotOp::Locator => r.op(command, locator_snapshot),
        SnapshotOp::PageJson => r.op(wire::command("Frame.ariaSnapshotJSON"), page_tree),
        SnapshotOp::LocatorJson => r.op(wire::command("Frame.ariaSnapshotJSON"), locator_tree),
        SnapshotOp::Baseline => r.op(
            wire::neo_only(
                "Playwright keeps no snapshot between calls; Neo keeps the last whole-page v1 \
                 text per tab on the host for ariaSnapshot({ delta: true }), since each run is \
                 a fresh realm and the page's world must not hold cross-site frames' text; it \
                 answers how that text was rendered too (its text option, urls and cap), since a \
                 delta takes the options it does not pass from it and the text alone does not \
                 say them",
            ),
            read_baseline,
        ),
        SnapshotOp::Pending => r.op(
            wire::neo_only(
                "the delta baseline is the text the caller was given, which only the host \
                 knows when the run ends, so each whole-page v1 snapshot's text waits on the \
                 run's state until then, with how the prelude rendered it, which the baseline \
                 keeps for the next delta",
            ),
            record_pending,
        ),
    }
}

/// `page.ariaSnapshot`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "PageSnapshotInput")]
struct PageInput {
    page: PageRef,
    #[ts(type = "import('../../../probe/protocol').SnapshotMode")]
    mode: SnapshotMode,
    #[serde(default)]
    #[ts(optional)]
    depth: Option<i64>,
    /// Upstream's `boxes`: every node's box in its own frame's viewport.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    boxes: bool,
    /// Neo's clickable marking, in mode `ai` only: the prelude asks for it in its condensed
    /// format, never under the spec harness's conformance switch (`clickable_targets`). The
    /// tree path's only.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    clickable: bool,
    /// The text to answer, printed in Rust; left out, the op answers the tree.
    #[serde(default)]
    #[ts(optional)]
    format: Option<PageFormat>,
    /// With `format`: answer the tree and Rust's text beside it, record nothing and take no
    /// event wait inside (a test harness's compare switch).
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    compare: bool,
}
// Snapshots have always sent flat fields, not a nested `options` bag. Keep that wire shape;
// there is no options bag for the generator's option-key probe to describe.
options!(PageInput);

/// `locator.ariaSnapshot`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "LocatorSnapshotInput")]
struct LocatorInput {
    target: Target,
    strict: Strictness,
    #[ts(type = "import('../../../probe/protocol').SnapshotMode")]
    mode: SnapshotMode,
    #[serde(default)]
    #[ts(optional)]
    depth: Option<i64>,
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    boxes: bool,
    /// Neo's clickable marking, as on `page.ariaSnapshot`.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    clickable: bool,
    /// The text to answer, as on `page.ariaSnapshot`, without `delta`.
    #[serde(default)]
    #[ts(optional)]
    format: Option<LocatorFormat>,
    /// As on `page.ariaSnapshot`.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    compare: bool,
}
options!(LocatorInput);

/// `page.ariaSnapshotJSON`: upstream's params, the tree never marked.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "PageTreeInput")]
struct PageTreeInput {
    page: PageRef,
    #[ts(type = "import('../../../probe/protocol').SnapshotMode")]
    mode: SnapshotMode,
    #[serde(default)]
    #[ts(optional)]
    depth: Option<i64>,
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    boxes: bool,
}
options!(PageTreeInput);

/// `locator.ariaSnapshotJSON`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "LocatorTreeInput")]
struct LocatorTreeInput {
    target: Target,
    strict: Strictness,
    #[ts(type = "import('../../../probe/protocol').SnapshotMode")]
    mode: SnapshotMode,
    #[serde(default)]
    #[ts(optional)]
    depth: Option<i64>,
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    boxes: bool,
}
options!(LocatorTreeInput);

/// Upstream's `FrameAriaSnapshotJSONResult`: the tree, for `ariaSnapshotJSON()`. Not the
/// engine's `SnapshotTree`, the capture this reply's nodes are emitted from.
#[derive(Debug, Serialize, TS)]
#[ts(rename = "SnapshotTree")]
struct AriaTreeReply {
    // As on `SnapshotResult`: the nodes' type lives in neo-snapshot, which knows nothing of ts-rs.
    #[ts(type = "import('../../vendored').AriaSnapshotJSON")]
    snapshot: Vec<AriaNode>,
}

/// `page.snapshotBaseline`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "SnapshotBaselineInput")]
struct BaselineInput {
    page: PageRef,
    /// The `document` the page's snapshot answered. None finds no baseline.
    #[serde(default)]
    #[ts(optional)]
    document: Option<u32>,
    /// The `snapshotCall` the page's snapshot answered: the read is held to that call's
    /// deadline. None holds it to none.
    #[serde(default, rename = "snapshotCall")]
    #[ts(optional)]
    snapshot_call: Option<u32>,
}
options!(BaselineInput);

/// `page.snapshotPending`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "SnapshotPendingInput")]
struct PendingInput {
    page: PageRef,
    /// The `document` the page's snapshot answered. A text with none is not recorded.
    #[serde(default)]
    #[ts(optional)]
    document: Option<u32>,
    /// The `snapshotCall` the page's snapshot answered: the record is held to that call's
    /// deadline. None holds it to none.
    #[serde(default, rename = "snapshotCall")]
    #[ts(optional)]
    snapshot_call: Option<u32>,
    /// The whole page's v1 text, after the notice and the cap: what the baseline becomes.
    text: String,
    /// How `text` was rendered, which the baseline keeps beside it: the form a later delta
    /// takes the options it does not pass from.
    form: SnapshotForm,
    /// The script passed `delta: true`. Without it, a stored baseline found means a delta was
    /// possible, which the run's reply turns into a tip.
    delta: bool,
    /// What the call returned, when it was not `text` itself: a delta call's delta. The text
    /// becomes the baseline only if this string reaches the caller whole.
    #[serde(default)]
    #[ts(optional)]
    returned: Option<String>,
}
options!(PendingInput);

/// Runs one snapshot op: the tree (`aria_snapshot::SnapshotResult`), a text, or both
/// ([`SnapshotAnswer`]), the tree alone for the JSON ops, or a baseline op's answer.
pub(crate) async fn invoke(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: SnapshotOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    route(op, Run { pw, scope, args }).await
}

/// How a snapshot call answers, decided from its input before anything is sent.
#[derive(Clone, Copy, Debug)]
enum Reading {
    /// No `format`: the stitched tree, for the prelude to render, marked as `clickable` asks.
    Tree { marked: bool },
    /// The text the format names.
    Text(TextAsk),
    /// The tree and Rust's text, from the same read.
    Compare(TextAsk),
}

/// The input's two switches beside its format: the tree path's `clickable`, and `compare`.
#[derive(Clone, Copy, Debug)]
struct Switches {
    clickable: bool,
    compare: bool,
}

impl Reading {
    /// The pairs the prelude sends; any other is a broken producer, refused as `internal`
    /// before the page is admitted.
    fn of(
        scope: &OpScope<'_>,
        mode: SnapshotMode,
        ask: Option<TextAsk>,
        switches: Switches,
    ) -> Result<Self, PlaywrightError> {
        let Switches { clickable, compare } = switches;
        let refused = |message: &str| Err(scope.fail(ErrorCode::Internal, message));
        match ask {
            None if compare => refused("`compare` compares a text, so it comes with a `format`"),
            None => Ok(Self::Tree {
                marked: marks(mode, clickable),
            }),
            Some(_) if clickable => {
                refused("with a `format`, the format decides the marking, not `clickable`")
            }
            Some(ask) if compare => Ok(Self::Compare(ask)),
            Some(ask) => Ok(Self::Text(ask)),
        }
    }

    fn marked(self) -> bool {
        match self {
            Self::Tree { marked } => marked,
            Self::Text(ask) | Self::Compare(ask) => ask.marks(),
        }
    }
}

/// `page.ariaSnapshot`: the whole page's tree, then its text when the input names a format.
///
/// ```text
///   admit the page; note that refs were issued (mode ai)
///   document = generation(tab)                       BEFORE the tree
///   tree     = the main frame within the call's timeout; children stitched, each on its budget
///   ── the op's last reply from the browser is in ───────────────────────────────── the fence
///   Neo's format: the event wait, here (order_events_inside), then finish_page_text,
///                 which cannot send: the clock, the stored text, render, the clock, the record
///   mode 'full', Playwright's format: render; the generic wait follows in Playwright::invoke
/// ```
async fn page_snapshot(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PageInput,
) -> Result<SnapshotAnswer, PlaywrightError> {
    let ask = input
        .format
        .map(|format| TextAsk::page(scope, input.mode, format))
        .transpose()?;
    let switches = Switches {
        clickable: input.clickable,
        compare: input.compare,
    };
    let reading = Reading::of(scope, input.mode, ask, switches)?;
    let page = OpPage::admit(pw, scope, input.page.tab()).await?;
    if input.mode == SnapshotMode::Ai {
        scope.run_state().opened_tabs.refs_issued(page.tab());
    }
    // Before the tree: a document that commits after this makes the text stale for the
    // baseline, so a run that ends later cannot hand the new document the old one's text.
    let document = pw.snapshot_baselines().generation(page.target_id());
    let read = TreeRead {
        mode: input.mode,
        depth: input.depth,
        boxes: input.boxes,
        marked: reading.marked(),
    };
    let stitched = tree_snapshot::read_page_prepared(scope, &page, read).await?;
    match reading {
        Reading::Tree { .. } => Ok(SnapshotAnswer::Tree(with_call(
            scope,
            stitched.into_result(),
            document,
        ))),
        Reading::Text(TextAsk::Neo { passed, delta }) => {
            let facts = PreparedFacts::from_read(stitched);
            // The fence: every browser send of this call is behind it. The wait runs outside
            // any `scope.within`, so the call's deadline cannot cut it.
            snapshot_text::order_events_inside(pw, scope).await;
            let view = facts.view();
            let finish = Finish {
                view: &view,
                passed: &passed,
                delta,
                tab: page.target_id(),
                tab_id: page.tab(),
                document,
                store: pw.snapshot_baselines(),
                run: scope.run_state(),
            };
            let snapshot = snapshot_text::finish_page_text(&finish, &ScopeClock(scope))?;
            Ok(SnapshotAnswer::Text(SnapshotText { snapshot }))
        }
        Reading::Text(ask) => {
            let facts = PreparedFacts::from_read(stitched);
            Ok(SnapshotAnswer::Text(SnapshotText {
                snapshot: snapshot_text::plain_text(&facts.view(), ask),
            }))
        }
        Reading::Compare(ask) => {
            let facts = PreparedFacts::from_read(stitched);
            let at = TabStore {
                store: pw.snapshot_baselines(),
                tab: page.target_id(),
                document,
            };
            let rust =
                snapshot_text::compared_text(&facts.view(), ask, Some(at), &ScopeClock(scope))?;
            let tree = with_call(scope, facts.into_result(), document);
            Ok(SnapshotAnswer::Compared(SnapshotCompared { tree, rust }))
        }
    }
}

/// `page.ariaSnapshotJSON`: the stitched tree, unmarked, as upstream's command answers it.
async fn page_tree(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PageTreeInput,
) -> Result<AriaTreeReply, PlaywrightError> {
    let page = OpPage::admit(pw, scope, input.page.tab()).await?;
    if input.mode == SnapshotMode::Ai {
        scope.run_state().opened_tabs.refs_issued(page.tab());
    }
    let read = TreeRead {
        mode: input.mode,
        depth: input.depth,
        boxes: input.boxes,
        marked: false,
    };
    let stitched = read_page(scope, &page, read).await?;
    Ok(AriaTreeReply {
        snapshot: stitched.json,
    })
}

/// A whole page's tree as the prelude's path takes it: with the document read before the tree,
/// and the id of this call, which the baseline ops after it name so Rust holds them to this
/// call's own deadline (`snapshot_calls.rs`).
fn with_call(scope: &OpScope<'_>, tree: SnapshotResult, document: u32) -> SnapshotResult {
    let call = scope
        .run_state()
        .snapshot_calls
        .taken(scope.timeout(), scope.deadline());
    SnapshotResult {
        document: Some(document),
        snapshot_call: Some(call),
        ..tree
    }
}

/// The page's snapshot, from the tab's APC tree (`tree_snapshot`).
async fn read_page(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    read: TreeRead,
) -> Result<SnapshotResult, PlaywrightError> {
    tree_snapshot::read_page(scope, page, read).await
}

/// `locator.ariaSnapshot`: one element's tree, then its text when the input names a format. A
/// locator's text is never a baseline and has no survey of what is on top, so it keeps the
/// generic event wait after the handler.
async fn locator_snapshot(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: LocatorInput,
) -> Result<SnapshotAnswer, PlaywrightError> {
    let ask = input
        .format
        .map(|format| TextAsk::locator(scope, input.mode, format))
        .transpose()?;
    let switches = Switches {
        clickable: input.clickable,
        compare: input.compare,
    };
    let reading = Reading::of(scope, input.mode, ask, switches)?;
    let read = TreeRead {
        mode: input.mode,
        depth: input.depth,
        boxes: input.boxes,
        marked: reading.marked(),
    };
    let tree = read_locator_prepared(pw, scope, &input.target, input.strict, read).await?;
    Ok(match reading {
        Reading::Tree { .. } => SnapshotAnswer::Tree(tree.into_result()),
        Reading::Text(ask) => {
            let facts = PreparedFacts::from_read(tree);
            SnapshotAnswer::Text(SnapshotText {
                snapshot: snapshot_text::plain_text(&facts.view(), ask),
            })
        }
        Reading::Compare(ask) => {
            let facts = PreparedFacts::from_read(tree);
            let rust = snapshot_text::compared_text(&facts.view(), ask, None, &ScopeClock(scope))?;
            SnapshotAnswer::Compared(SnapshotCompared {
                tree: facts.into_result(),
                rust,
            })
        }
    })
}

/// `locator.ariaSnapshotJSON`: the element's stitched tree, unmarked.
async fn locator_tree(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: LocatorTreeInput,
) -> Result<AriaTreeReply, PlaywrightError> {
    let read = TreeRead {
        mode: input.mode,
        depth: input.depth,
        boxes: input.boxes,
        marked: false,
    };
    let tree = read_locator(pw, scope, &input.target, input.strict, read).await?;
    Ok(AriaTreeReply {
        snapshot: tree.json,
    })
}

/// One element's snapshot, with the frames inside it, kept in compact draft arenas until rendered
/// or materialized.
async fn read_locator_prepared(
    pw: &Playwright,
    scope: &OpScope<'_>,
    target: &Target,
    strict: Strictness,
    read: TreeRead,
) -> Result<PreparedSnapshot, PlaywrightError> {
    let Target::Locator(target) = target else {
        return Err(scope.not_built());
    };
    let page = OpPage::admit_handle(pw, scope, &target.page).await?;
    if read.mode == SnapshotMode::Ai {
        scope.run_state().opened_tabs.refs_issued(page.tab());
    }
    tree_snapshot::read_locator_prepared(scope, &page, target, strict, read).await
}

/// One element's snapshot, with the frames inside it, from the tab's APC tree
/// (`tree_snapshot`). It waits for the locator's one element in both modes, as upstream's
/// default mode does, through the resolver's frame hops and ref jumps.
async fn read_locator(
    pw: &Playwright,
    scope: &OpScope<'_>,
    target: &Target,
    strict: Strictness,
    read: TreeRead,
) -> Result<SnapshotResult, PlaywrightError> {
    read_locator_prepared(pw, scope, target, strict, read)
        .await
        .map(PreparedSnapshot::into_result)
}

/// The tab's stored baseline for the snapshot's document, or `null`: none yet, its main frame
/// committed a new document since, or no `document` was given. Never a text pending in this run,
/// so a delta compares against what the caller held when the run started. The page is admitted
/// like any op's, so a tab the run may not touch keeps its baseline unread.
async fn read_baseline(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: BaselineInput,
) -> Result<Option<Baseline>, PlaywrightError> {
    let page = OpPage::admit(pw, scope, input.page.tab()).await?;
    within_snapshot_call(scope, input.snapshot_call, "reading its delta baseline")?;
    Ok(input
        .document
        .and_then(|document| pw.snapshot_baselines().stored(page.target_id(), document)))
}

/// Records the snapshot's text as pending on the run's state; the host settles it when the run
/// ends (`RunState::settle_snapshot_baselines`). A full snapshot that finds a stored baseline is
/// noted for the reply's tip.
async fn record_pending(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PendingInput,
) -> Result<Nothing, PlaywrightError> {
    let page = OpPage::admit(pw, scope, input.page.tab()).await?;
    within_snapshot_call(scope, input.snapshot_call, "recording its delta baseline")?;
    let Some(document) = input.document else {
        return Ok(());
    };
    let store = pw.snapshot_baselines();
    if !input.delta && store.has_stored(page.target_id(), document) {
        scope.run_state().note_full_snapshot_repeat(page.tab());
    }
    scope.run_state().pending_baselines.record(
        store,
        page.target_id().clone(),
        document,
        input.text,
        input.form,
        input.returned,
    );
    Ok(())
}

/// Fails a baseline op once the timeout of the snapshot call it names has passed, by Rust's clock
/// and the tree op's own record (`snapshot_calls.rs`): that call's `wait_timeout`, naming the
/// `phase` it ran out in, with this op's id. Nothing the realm sends decides it. A script's abort
/// of this op comes first, as in `OpScope::timed_out`.
fn within_snapshot_call(
    scope: &OpScope<'_>,
    call: Option<u32>,
    phase: &str,
) -> Result<(), PlaywrightError> {
    if let Some(aborted) = scope.aborted_error() {
        return Err(aborted);
    }
    let passed = call.and_then(|call| {
        scope
            .run_state()
            .snapshot_calls
            .passed(call, Instant::now())
    });
    match passed {
        Some(timeout) => Err(scope.fail(
            ErrorCode::WaitTimeout,
            &format!(
                "Timeout {}ms exceeded after the snapshot was taken, while {phase}.",
                timeout.as_millis()
            ),
        )),
        None => Ok(()),
    }
}

/// Whether a snapshot is marked: `clickable` asks for it, and only mode `ai` mints the refs
/// marking names. Default mode ignores the key, as it prints no refs.
fn marks(mode: SnapshotMode, clickable: bool) -> bool {
    clickable && mode == SnapshotMode::Ai
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::json;

    use super::{LocatorInput, PageInput};
    use crate::probe::SnapshotMode;

    #[test]
    fn tests_that_snapshot_inputs_decode_as_wire_ts_declares_them() -> Result<(), Box<dyn Error>> {
        let page: PageInput = serde_json::from_value(json!({
            "page": { "$neo": "page", "tab": 1007 }, "mode": "ai", "depth": 3,
        }))?;
        assert_eq!((page.mode, page.depth), (SnapshotMode::Ai, Some(3)));
        let locator: LocatorInput = serde_json::from_value(json!({
            "target": { "$neo": "locator", "page": { "$neo": "page", "tab": 1007 }, "chunks": [
                { "parts": [{ "name": "aria-ref", "body": "f2e5", "source": "f2e5" }] },
            ], "display": "getByRef('f2e5')" },
            "strict": "strict",
            "mode": "default",
        }))?;
        assert_eq!((locator.mode, locator.depth), (SnapshotMode::Default, None));
        let boxed: PageInput = serde_json::from_value(
            json!({ "page": { "$neo": "page", "tab": 1007 }, "mode": "default", "boxes": true }),
        )?;
        assert!(
            boxed.boxes,
            "upstream's boxes option reaches every frame's probe"
        );
        let obsolete_locator = json!({
            "target": { "$neo": "locator", "page": { "$neo": "page", "tab": 1007 }, "chunks": [
                { "parts": [{ "name": "css", "body": "body", "source": "body" }] },
            ], "display": "locator('body')" },
            "strict": "strict", "mode": "default", "conformance": true,
        });
        assert!(serde_json::from_value::<LocatorInput>(obsolete_locator).is_err());
        for bad in [
            json!({ "page": { "$neo": "page", "tab": 1007 }, "mode": "codegen" }),
            json!({ "page": { "$neo": "page", "tab": 1007 }, "mode": "ai", "depth": 1.5 }),
            json!({ "page": { "$neo": "page", "tab": 1007 }, "mode": "ai", "conformance": true }),
            json!({ "page": { "$neo": "page", "tab": 1007 }, "mode": "ai", "discoverMovedFields": false }),
        ] {
            assert!(
                serde_json::from_value::<PageInput>(bad.clone()).is_err(),
                "{bad} is not a page snapshot input"
            );
        }
        Ok(())
    }
}
