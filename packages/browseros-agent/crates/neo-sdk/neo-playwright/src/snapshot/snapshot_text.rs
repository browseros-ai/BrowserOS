//! A snapshot's text printed in Rust, by `neo_snapshot::render`, when the prelude asks the
//! snapshot op for one (a `format` in its input): which text the call means, the facts it is
//! printed from, and, for a whole page in Neo's format, the tab's baseline read and recorded under
//! the call's own clock.
//!
//! ```text
//!  snapshot_op: the tree read, stitched ── SnapshotResult ─▶ facts() ─▶ neo_snapshot::Snapshot
//!  the input's format ─▶ TextAsk (checked; refused as internal: the prelude never sends it)
//!
//!  a locator, mode 'full', Playwright's format:  render ─▶ the text; the generic event wait
//!                                                follows in Playwright::invoke, as for any op
//!  a whole page in Neo's format:
//!    ── the fence: order_events_inside (Playwright::events_before_reply, the same function) ──
//!    finish_page_text: not async, given no page: it cannot send
//!       look at the clock ─ late, a delta ─▶ Earlier::NotRead: the stored text is not read
//!                         ─ in time, a delta ─▶ the stored text of the document read before
//!                                               the tree
//!       render
//!       look at the clock ─ in time ─▶ the tip, then the pending record ─ late ─▶ nothing
//!  compare (test harnesses only): the tree AND Rust's text from that one read; nothing is
//!    recorded and no wait is taken inside, so the prelude's own path runs as without it
//! ```
//!
//! The clock decides at three places, as the prelude's `LATE_SNAPSHOT = 'return'` did (design
//! 1.7): a call whose timeout passed before its tree fails with that timeout (in `snapshot_op`);
//! after the tree and before the stored text is read, a delta answers the no-time line; before
//! the pending record, the text is returned and nothing is recorded. A script's abort fails the
//! call at any look.

use std::sync::Arc;

use browseros_core::{TabId, TargetId};
use neo_snapshot::{
    Answer, Call, ClosedRoots, Earlier, Layer, MaxTokens, PartialFrame, Passed, Snapshot, Unmarked,
};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;
use ts_rs::TS;

use super::aria_snapshot::{self, SnapshotResult, SnapshotTop};
use super::{SnapshotBaselines, SnapshotForm};
use crate::op::Op;
use crate::op_scope::OpScope;
use crate::playwright::ordering_wait_for;
use crate::probe::SnapshotMode;
use crate::snapshot::SnapshotOp;
use crate::{ErrorCode, Playwright, PlaywrightError, RunState, TimedOut};

#[cfg(test)]
mod tests;

/// Which text a whole page's snapshot answers, as the prelude chose it from the script's `mode`
/// and the conformance switch, with the options it already checked (their messages stay in the
/// prelude). On the wire: `"playwright"` | `{ full: { maxTokens? } }` |
/// `{ neo: { text?, urls?, maxTokens?, inert?, delta? } }`. An option the script left out stays
/// left out: the renderer owns the default cap, and a delta takes what was left out from its
/// baseline's form.
#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PageSnapshotFormat")]
pub(super) enum PageFormat {
    /// Playwright's own text: `mode: 'default'`, and every mode under the conformance switch.
    Playwright,
    /// `mode: 'full'`.
    Full(FullFormat),
    /// Neo's default format; `delta` compares it with the tab's stored text.
    Neo(PageNeoFormat),
}

/// [`PageFormat`] for one element: no `delta`, so the wire cannot carry an option the call would
/// ignore (the prelude checks a script's `delta` on a locator and drops it, as before).
#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "LocatorSnapshotFormat")]
pub(super) enum LocatorFormat {
    /// Playwright's own text.
    Playwright,
    /// `mode: 'full'`.
    Full(FullFormat),
    /// Neo's default format.
    Neo(NeoFormat),
}

/// `mode: 'full'`'s one option.
#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename = "FullSnapshotFormat")]
pub(super) struct FullFormat {
    /// The cap the script passed; left out, the renderer's default.
    #[serde(default)]
    #[ts(optional)]
    max_tokens: Option<f64>,
}

/// Neo's default format's options for one element, each as the script passed it.
#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename = "NeoSnapshotFormat")]
pub(super) struct NeoFormat {
    #[serde(default)]
    #[ts(optional)]
    text: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    urls: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    max_tokens: Option<f64>,
    #[serde(default)]
    #[ts(optional)]
    inert: Option<bool>,
}

/// Neo's default format's options for a whole page, each as the script passed it.
#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename = "PageNeoSnapshotFormat")]
pub(super) struct PageNeoFormat {
    #[serde(default)]
    #[ts(optional)]
    text: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    urls: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    max_tokens: Option<f64>,
    #[serde(default)]
    #[ts(optional)]
    inert: Option<bool>,
    /// What changed since the tab's stored text, instead of the whole page.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    delta: bool,
}

/// A text call, its format checked: what `render` is asked for.
#[derive(Clone, Copy, Debug)]
pub(super) enum TextAsk {
    /// [`Call::Playwright`].
    Playwright,
    /// [`Call::Full`], the cap `None` when the script left it out.
    Full(Option<MaxTokens>),
    /// Neo's default format: [`Call::Element`] for a locator, [`Call::Page`] or a delta for a
    /// whole page.
    Neo {
        /// The options the script passed.
        passed: Passed,
        /// A whole page's `delta: true`; always false for a locator.
        delta: bool,
    },
}

impl TextAsk {
    /// A whole page's format, checked against the tree `mode` it reads.
    pub(super) fn page(
        scope: &OpScope<'_>,
        mode: SnapshotMode,
        format: PageFormat,
    ) -> Result<Self, PlaywrightError> {
        let ask = match format {
            PageFormat::Playwright => Self::Playwright,
            PageFormat::Full(full) => Self::Full(cap(scope, full.max_tokens)?),
            PageFormat::Neo(neo) => Self::Neo {
                passed: passed(scope, neo.text, neo.urls, neo.max_tokens, neo.inert)?,
                delta: neo.delta,
            },
        };
        ask.reads(scope, mode)
    }

    /// One element's format, checked against the tree `mode` it reads.
    pub(super) fn locator(
        scope: &OpScope<'_>,
        mode: SnapshotMode,
        format: LocatorFormat,
    ) -> Result<Self, PlaywrightError> {
        let ask = match format {
            LocatorFormat::Playwright => Self::Playwright,
            LocatorFormat::Full(full) => Self::Full(cap(scope, full.max_tokens)?),
            LocatorFormat::Neo(neo) => Self::Neo {
                passed: passed(scope, neo.text, neo.urls, neo.max_tokens, neo.inert)?,
                delta: false,
            },
        };
        ask.reads(scope, mode)
    }

    /// Whether the tree is read with Neo's marks (clickable marking, the cover survey, the
    /// inert marks): Neo's format and `mode: 'full'` print them, Playwright's never does. So the
    /// format alone decides the marking, which `clickable` decides on the tree path.
    pub(super) fn marks(self) -> bool {
        !matches!(self, Self::Playwright)
    }

    /// Neo's formats print the refs only the `'ai'` tree mints, so the prelude always reads that
    /// tree for them; Playwright's format reads the mode the script passed.
    fn reads(self, scope: &OpScope<'_>, mode: SnapshotMode) -> Result<Self, PlaywrightError> {
        if self.marks() && mode != SnapshotMode::Ai {
            return Err(scope.fail(
                ErrorCode::Internal,
                "Neo's snapshot formats read the 'ai' tree; the prelude sends no other",
            ));
        }
        Ok(self)
    }
}

/// The cap as `render` takes it. The prelude has refused every number `MaxTokens::new` refuses,
/// with its own message, so one here is a broken producer: `internal`.
fn cap(scope: &OpScope<'_>, tokens: Option<f64>) -> Result<Option<MaxTokens>, PlaywrightError> {
    tokens
        .map(|tokens| {
            MaxTokens::new(tokens).ok_or_else(|| {
                scope.fail(
                    ErrorCode::Internal,
                    &format!("maxTokens {tokens} is not a cap the prelude passes"),
                )
            })
        })
        .transpose()
}

fn passed(
    scope: &OpScope<'_>,
    text: Option<bool>,
    urls: Option<bool>,
    max_tokens: Option<f64>,
    inert: Option<bool>,
) -> Result<Passed, PlaywrightError> {
    Ok(Passed {
        text,
        urls,
        max_tokens: cap(scope, max_tokens)?,
        inert,
    })
}

/// What `page.ariaSnapshot` and `locator.ariaSnapshot` answer, by what the input asked for. One
/// type for the three while both engines exist (piece 8 keeps only the text).
#[derive(Debug, Serialize, TS)]
#[serde(untagged)]
pub(super) enum SnapshotAnswer {
    /// No `format`: the stitched tree, which the prelude renders.
    Tree(SnapshotResult),
    /// A `format`: the text alone, upstream's `FrameAriaSnapshotResult`. It has no field for the
    /// tree, so the tree cannot reach the script engine on a text call.
    Text(SnapshotText),
    /// A `format` with `compare`: the tree and Rust's text beside it.
    Compared(SnapshotCompared),
}

/// Upstream's `FrameAriaSnapshotResult`.
#[derive(Debug, Serialize, TS)]
pub(super) struct SnapshotText {
    pub(super) snapshot: String,
}

/// The compare switch's answer (design 2.3, layer 5): the tree the prelude renders as without
/// the switch, and Rust's text of that same read, which the prelude compares with its own.
#[derive(Debug, Serialize, TS)]
pub(super) struct SnapshotCompared {
    #[serde(flatten)]
    pub(super) tree: SnapshotResult,
    pub(super) rust: RustText,
}

/// What Rust printed for a compared call, and what it would have kept for the tab.
#[derive(Debug, Serialize, TS)]
pub(super) struct RustText {
    /// The text the call would return.
    pub(super) snapshot: String,
    /// The text and form the call would record as pending: a whole page in Neo's format, or a
    /// delta that read the stored text. Never recorded: the prelude's own path records.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(super) pending: Option<RustPending>,
}

/// A pending record's text and form, for the compare switch to hold beside the prelude's.
#[derive(Debug, Serialize, TS)]
pub(super) struct RustPending {
    pub(super) text: String,
    pub(super) form: SnapshotForm,
}

impl RustText {
    /// Rust's side of a compared call, from its answer.
    pub(super) fn of(answer: &Answer) -> Self {
        Self {
            snapshot: answer.text().to_owned(),
            pending: answer.pending().map(|pending| RustPending {
                text: pending.text.to_owned(),
                form: SnapshotForm::of(pending.form),
            }),
        }
    }
}

/// The facts `render` prints from, moved out of the stitched tree without a copy: the tree, the
/// frames not read, the layers worded by `action::on_top_line`, the closed roots and whether
/// the root frame's clickable check finished. [`tree_of`] moves them back.
pub(super) fn facts(result: SnapshotResult) -> Snapshot {
    let SnapshotResult {
        json,
        partial,
        unmarked,
        on_top,
        document: _,
        snapshot_call: _,
        closed_roots,
    } = result;
    Snapshot {
        tree: json,
        partial: partial
            .into_iter()
            .map(|frame| PartialFrame {
                r#ref: frame.r#ref,
                frame: frame.frame,
                reason: frame.reason,
            })
            .collect(),
        layers: on_top
            .into_iter()
            .map(|top| Layer {
                r#ref: top.r#ref,
                line: top.line,
                whole_view: top.whole_view,
            })
            .collect(),
        closed_roots: closed_roots.map(|roots| ClosedRoots {
            sent: roots.sent,
            found: roots.found,
        }),
        unmarked: unmarked.map(|unmarked| Unmarked {
            reason: unmarked.reason,
        }),
    }
}

/// [`facts`] undone: the stitched tree the prelude renders, with no document or call id yet.
pub(super) fn tree_of(snapshot: Snapshot) -> SnapshotResult {
    let Snapshot {
        tree,
        partial,
        layers,
        closed_roots,
        unmarked,
    } = snapshot;
    SnapshotResult {
        json: tree,
        partial: partial
            .into_iter()
            .map(|frame| aria_snapshot::PartialFrame {
                r#ref: frame.r#ref,
                frame: frame.frame,
                reason: frame.reason,
            })
            .collect(),
        unmarked: unmarked.map(|unmarked| aria_snapshot::Unmarked {
            reason: unmarked.reason,
        }),
        on_top: layers
            .into_iter()
            .map(|layer| SnapshotTop {
                r#ref: layer.r#ref,
                line: layer.line,
                whole_view: layer.whole_view,
            })
            .collect(),
        document: None,
        snapshot_call: None,
        closed_roots: closed_roots.map(|roots| aria_snapshot::ClosedRoots {
            sent: roots.sent,
            found: roots.found,
        }),
    }
}

/// THE EVENT WAIT, for the one call that reads and records its baseline: a whole page in Neo's
/// format. Every op that did not time out waits, before its reply leaves, for the listeners of
/// what the page did before its last browser reply (`Playwright::events_before_reply`). The
/// baseline must be read and recorded after that wait, as when the prelude sent the two baseline
/// ops after the held tree landed: a reply held past the call's timeout is then late for the
/// baseline. So this call takes the wait here, right after its last browser reply, and the
/// generic wait after the handler is skipped (`OpScope::mark_ordered_inside`): never both, and
/// never neither.
///
/// - It is the same function as the generic wait's, with its bound from the same rule
///   (`ordering_wait_for`, asked as not timed out: the tree is in hand). So it waits for the
///   listened tabs' held events, or every tab's once a context listener exists, and a held seed
///   read on another tab costs it nothing.
/// - It is not inside any `scope.within`: the call's deadline does not cut it, as it cuts no
///   successful op's wait. A reply it holds past the timeout makes the next look at the clock
///   late.
/// - A run with no listener has nothing to order and does not wait; that counts as having run.
/// - Dropping the op's future (the host's cancel, the run's deadline) ends it, as any await.
pub(super) async fn order_events_inside(pw: &Playwright, scope: &OpScope<'_>) {
    if let Some(bound) = ordering_wait_for(Op::snapshot(SnapshotOp::Page), TimedOut::No) {
        pw.events_before_reply(scope, bound).await;
    }
    scope.mark_ordered_inside();
}

/// The call's clock, lent to [`finish_page_text`]: Rust's own for a real call ([`ScopeClock`]), a
/// script of looks for a test, so each window of design 1.7 is pinned without a browser.
pub(super) trait CallClock {
    /// `Err` once the script aborted the call: it fails, and nothing is recorded.
    fn look(&self) -> Result<Look, PlaywrightError>;
}

/// What a look at the clock found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Look {
    /// The call's timeout has not passed, or it has none (`timeout: 0`).
    InTime,
    /// The call's timeout has passed: the call still returns its text and records nothing. This
    /// is the owner's choice between two defensible answers (`LATE_SNAPSHOT = 'return'` in the
    /// prelude until piece 8); the other, failing the call, would need no code.
    Late,
}

/// The op's own clock: its deadline on Rust's clock, the script's abort first.
pub(super) struct ScopeClock<'a, 'op>(pub(super) &'a OpScope<'op>);

impl CallClock for ScopeClock<'_, '_> {
    fn look(&self) -> Result<Look, PlaywrightError> {
        if let Some(aborted) = self.0.aborted_error() {
            return Err(aborted);
        }
        Ok(match self.0.deadline() {
            Some(deadline) if Instant::now() >= deadline => Look::Late,
            _ => Look::InTime,
        })
    }
}

/// What a whole page's call holds once its tree is in and the event wait is over.
pub(super) struct Finish<'a> {
    pub(super) snapshot: &'a Snapshot,
    pub(super) passed: &'a Passed,
    /// The script passed `delta: true`.
    pub(super) delta: bool,
    /// The target of the page admitted before the tree read: nothing is admitted after the
    /// fence, since an admission can attach to the page and send.
    pub(super) tab: &'a TargetId,
    pub(super) tab_id: TabId,
    /// The tab's document generation, read before the tree: a document that commits after it
    /// makes this text stale for the baseline.
    pub(super) document: u32,
    pub(super) store: &'a Arc<SnapshotBaselines>,
    pub(super) run: &'a RunState,
}

/// Everything a whole page's text call does after the fence. Not async and given no page, world
/// or sender, so it cannot talk to a browser: the event order above holds by construction.
///
/// What a caller cannot get wrong: compare with a text pending in this run (`Earlier` takes the
/// store's stored text only); read the baseline of another document (the generation read before
/// the tree is the one used); record a text other than the one returned, or in another form
/// (`Answer::pending` lends both).
pub(super) fn finish_page_text(
    finish: &Finish<'_>,
    clock: &dyn CallClock,
) -> Result<String, PlaywrightError> {
    let at = TabStore {
        store: finish.store,
        tab: finish.tab,
        document: finish.document,
    };
    // Read at the first look, and alive across the render, which borrows it.
    let stored = finish
        .delta
        .then(|| StoredRead::at(&at, clock))
        .transpose()?;
    let call = match &stored {
        Some(stored) => Call::Delta {
            passed: finish.passed,
            earlier: stored.earlier(),
        },
        None => Call::Page(finish.passed),
    };
    let answer = neo_snapshot::render(finish.snapshot, call);
    // A delta that did not read the stored text keeps nothing, so it looks no second time.
    if let Some(pending) = answer.pending() {
        match clock.look()? {
            // Returned, not recorded: the older baseline stays.
            Look::Late => {}
            Look::InTime => {
                // A whole page where a stored text exists could have been a delta: the run's
                // reply turns that into a tip.
                if !finish.delta && finish.store.has_stored(finish.tab, finish.document) {
                    finish.run.note_full_snapshot_repeat(finish.tab_id);
                }
                finish.run.pending_baselines.record(
                    finish.store,
                    finish.tab.clone(),
                    finish.document,
                    pending.text.to_owned(),
                    SnapshotForm::of(pending.form),
                    pending.returned.map(str::to_owned),
                );
            }
        }
    }
    Ok(answer.into_text())
}

/// Where a whole page's stored text is: the store, the tab's target and the document generation
/// read before the tree.
#[derive(Clone, Copy)]
pub(super) struct TabStore<'a> {
    pub(super) store: &'a SnapshotBaselines,
    pub(super) tab: &'a TargetId,
    pub(super) document: u32,
}

/// The tab's stored text as a delta reads it, at a look of the call's clock.
enum StoredRead {
    /// The call's time is gone (design 1.7's second window): nothing stored is read, and the
    /// delta answers that no time was left to look.
    Late,
    /// The stored text of the document read before the tree, if there is one. Never a text
    /// pending in this run: those wait in the run's state until it ends.
    Read(Option<neo_snapshot::Baseline>),
}

impl StoredRead {
    fn at(at: &TabStore<'_>, clock: &dyn CallClock) -> Result<Self, PlaywrightError> {
        Ok(match clock.look()? {
            Look::Late => Self::Late,
            Look::InTime => Self::Read(
                at.store
                    .stored(at.tab, at.document)
                    .map(|baseline| {
                        baseline.printed().ok_or_else(|| {
                            PlaywrightError::new(
                                ErrorCode::Internal,
                                "page.ariaSnapshot: the tab's stored snapshot has a cap no call \
                                 can pass"
                                    .to_owned(),
                            )
                        })
                    })
                    .transpose()?,
            ),
        })
    }

    fn earlier(&self) -> Earlier<'_> {
        match self {
            Self::Late => Earlier::NotRead,
            Self::Read(None) => Earlier::Nothing,
            Self::Read(Some(baseline)) => Earlier::Text(baseline),
        }
    }
}

/// A text that keeps nothing for the tab: Playwright's format, `mode: 'full'`, and a locator's in
/// Neo's format ([`Call::Element`]). A whole page in Neo's format is [`finish_page_text`]'s.
pub(super) fn plain_text(snapshot: &Snapshot, ask: TextAsk) -> String {
    plain(snapshot, ask).into_text()
}

fn plain(snapshot: &Snapshot, ask: TextAsk) -> Answer {
    match ask {
        TextAsk::Playwright => neo_snapshot::render(snapshot, Call::Playwright),
        TextAsk::Full(max_tokens) => neo_snapshot::render(snapshot, Call::Full { max_tokens }),
        TextAsk::Neo { passed, .. } => neo_snapshot::render(snapshot, Call::Element(&passed)),
    }
}

/// Rust's text of a compared call: what the call would answer, by the same rules, with nothing
/// recorded. `page` is where a whole page's stored text is, `None` for a locator. A delta reads
/// the stored text itself, under the same clock.
pub(super) fn compared_text(
    snapshot: &Snapshot,
    ask: TextAsk,
    page: Option<TabStore<'_>>,
    clock: &dyn CallClock,
) -> Result<RustText, PlaywrightError> {
    let answer = match (ask, page) {
        (TextAsk::Neo { passed, delta }, Some(at)) => {
            let stored = delta.then(|| StoredRead::at(&at, clock)).transpose()?;
            let call = match &stored {
                Some(stored) => Call::Delta {
                    passed: &passed,
                    earlier: stored.earlier(),
                },
                None => Call::Page(&passed),
            };
            neo_snapshot::render(snapshot, call)
        }
        (other, _) => plain(snapshot, other),
    };
    Ok(RustText::of(&answer))
}
