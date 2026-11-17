//! A pointer action under a cover (covered-click spec, sections 1 and 2; the page's half is
//! neo-playwright-js/src/probe/cover.ts): which cover stood at the inner loop's checks, and
//! what the action says about it when its timeout runs out under one.
//!
//! ```text
//!  a check that completed ─ covered, at the arm step ② or a parent page's hit test ─▶ saw
//!                         ├ the verdict missed (a press stopped at the window) ──────▶ missed
//!                         └ anything else ───────────────────────────────────────────▶ cleared
//!  the op's timeout, the last completed check covered, not conformance
//!    ─▶ one `cover` request to the world that saw it, on its own short budget
//!    ─▶ the same cover still on top there: code `covered`, the facts' lines, the `cover` field
//!       anything else (another cover, none, a failed or slow request): today's error
//! ```
//!
//! The facts change only what a covered timeout says. When an action stops waiting is section
//! 1's rule, a later change; this module only keeps the record it needs.

use std::fmt::Write as _;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::time::{self, Instant};

use crate::op_scope::OpScope;
use crate::page::{LastCover, OpPage};
use crate::probe::{
    CoverFacts, CoverKind, CoverMark, Point, ProbeReply, ProbeRequest, ProbeTarget,
};
use crate::world::IsolatedWorld;
use crate::{ErrorCode, PlaywrightError, resolve};

/// How long the facts request may take once the op's timeout has passed, as cleanup has its
/// own: a page that cannot answer in this time leaves today's error.
const FACTS_BUDGET: Duration = Duration::from_millis(1_000);

/// Where a covered check saw its cover: the world to ask for the facts, what was tested there,
/// and at which point of that frame.
#[derive(Clone, Debug)]
pub(crate) struct Seat {
    pub(crate) world: IsolatedWorld,
    pub(crate) target: ProbeTarget,
    pub(crate) point: Point,
    /// `None` for the element's own frame; for a page above it, which one, counted from the
    /// main frame down.
    pub(crate) above: Option<usize>,
}

/// One cover standing since a check, through every completed check after it.
#[derive(Debug)]
pub(crate) struct Standing {
    seat: Seat,
    /// `None` after a verdict's miss alone: the interceptor names no element, and the next
    /// covered check gives the cover its mark.
    mark: Option<CoverMark>,
    /// The control's click point when `mark` was taken, in the same pixels: the place over the
    /// control that a cover moving with it keeps under any scroll.
    mark_at: Point,
    since: Instant,
    checks: u32,
    /// It was there at the action's first completed check.
    from_start: bool,
}

/// The checks a cover stands before a press may go through it as the target's own wrapper.
const WRAPPER_CHECKS: u32 = 4;

/// Covered-click B: how long the same cover stands, on Rust's clock, before a pointer action
/// under the built-in timeout stops waiting (the owner's number, 2026-11-16).
pub(crate) const EARLY_STOP: Duration = Duration::from_secs(5);

/// Covered-click B: the fewest checks a cover stands before that stop. After four the scroll
/// rotation (if needed, end, center, start) has run once, so a sticky header or footer had every
/// alignment.
const EARLY_CHECKS: u32 = 4;

/// A pointer action that stops waiting under a standing cover: how long it stood and how many
/// checks saw it, for the answer's first line and the record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EarlyStop {
    pub(crate) stood: Duration,
    pub(crate) checks: u32,
}

/// Which wait a pointer action keeps under a cover that stands (covered-click B).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Waiting {
    /// Neo's own built-in timeout, which nobody chose, outside the conformance flag: the early
    /// stop applies.
    BuiltIn,
    /// A timeout somebody chose, or the conformance flag: the whole wait, as upstream.
    Whole,
}

impl Waiting {
    /// The op's wait: built-in only under the prelude's stamp and outside the conformance flag.
    pub(crate) fn of(scope: &OpScope<'_>) -> Self {
        if scope.timeout_built_in() && !scope.conformance() {
            Self::BuiltIn
        } else {
            Self::Whole
        }
    }
}

/// Covered-click B's decision, pure: a pointer action stops waiting when the same cover has
/// stood [`EARLY_STOP`] and at least [`EARLY_CHECKS`] checks, and does not say it is loading,
/// under Neo's built-in timeout ([`Waiting::BuiltIn`]). Whether it is the same cover (a pass, a
/// move, a fade or another cover starts the count again) is the watch's; this reads what the
/// watch kept.
pub(crate) fn decide(
    stood: Duration,
    checks: u32,
    loading: bool,
    waiting: Waiting,
) -> Option<EarlyStop> {
    (waiting == Waiting::BuiltIn && !loading && checks >= EARLY_CHECKS && stood >= EARLY_STOP)
        .then_some(EarlyStop { stood, checks })
}

/// The record of a pointer action's completed checks, kept by its runner on Rust's clock.
#[derive(Debug)]
pub(crate) struct CoverWatch {
    began: Instant,
    checks: u32,
    /// Presses that went out and that the interceptor stopped at the window.
    stopped: u32,
    standing: Option<Standing>,
}

impl CoverWatch {
    pub(crate) fn new() -> Self {
        Self {
            began: Instant::now(),
            checks: 0,
            stopped: 0,
            standing: None,
        }
    }

    /// A check that completed and found a cover at `seat`.
    pub(crate) fn saw(&mut self, seat: Seat, mark: Option<CoverMark>) {
        self.covered(seat, mark);
    }

    /// A verdict that found the press stopped: the cover came between the check and the
    /// press. It counts as a sighting, of whatever stands there.
    pub(crate) fn missed(&mut self, seat: Seat) {
        self.stopped += 1;
        self.covered(seat, None);
    }

    /// A check that completed and found anything but a cover; also a resolve that starts over.
    pub(crate) fn cleared(&mut self) {
        self.checks += 1;
        self.standing = None;
    }

    fn covered(&mut self, seat: Seat, mark: Option<CoverMark>) {
        let first = self.checks == 0;
        self.checks += 1;
        if let Some(standing) = self.standing.as_mut()
            && standing.seat.above == seat.above
            && same_cover((standing.mark_at, standing.mark), (seat.point, mark))
        {
            standing.checks += 1;
            // The place stays the first sighting's, so a cover drifting under the tolerance at
            // every check still moves; whether it says it is loading is the latest sighting's.
            if let Some(seen) = mark {
                if let Some(held) = standing.mark.as_mut() {
                    held.busy = seen.busy;
                } else {
                    standing.mark = Some(seen);
                    standing.mark_at = seat.point;
                }
            }
            standing.seat = seat;
            return;
        }
        self.standing = Some(Standing {
            mark_at: seat.point,
            seat,
            mark,
            since: Instant::now(),
            checks: 1,
            from_start: first,
        });
    }

    /// The cover a press may go through as the target's own wrapper or drawing (covered-click
    /// C): the one that has stood for [`WRAPPER_CHECKS`] checks and does not say it is loading.
    /// A cover that just arrived may still be a page's veil on its way somewhere.
    pub(crate) fn wrapper_allowed(&self) -> Option<u32> {
        let standing = self.standing.as_ref()?;
        let mark = standing.mark.as_ref()?;
        (standing.checks >= WRAPPER_CHECKS && !mark.busy).then_some(mark.id)
    }

    /// Covered-click B: whether the action stops waiting now, under the cover that stands. Only
    /// a cover known by its mark counts: one known only from a verdict's miss has no box yet.
    pub(crate) fn early(&self, now: Instant, waiting: Waiting) -> Option<EarlyStop> {
        let standing = self.standing.as_ref()?;
        let mark = standing.mark.as_ref()?;
        decide(
            now.saturating_duration_since(standing.since),
            standing.checks,
            mark.busy,
            waiting,
        )
    }

    /// The cover the last completed check found, if it found one.
    pub(crate) fn standing(&self) -> Option<&Standing> {
        self.standing.as_ref()
    }
}

/// Half a pixel: the same box measured after a scroll is not the same number bit for bit.
const SAME_PLACE_PX: f64 = 0.5;

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < SAME_PLACE_PX
}

/// Two sightings of one standing cover, each with the control's click point it was seen at. The
/// same cover keeps its root's number, its opacity (a fade is a change) and its size, and keeps
/// its place in one of two ways: in the view (a fixed cover the control scrolls under), or over
/// the control's click point (a cover that moves with its control under any scroll, the page's
/// or a nested scroller's, as the runner's retries scroll it). One that changes in both ways is
/// moving. A sighting with no mark (a verdict's miss) is of whatever stands there.
fn same_cover(
    (at, standing): (Point, Option<CoverMark>),
    (now, seen): (Point, Option<CoverMark>),
) -> bool {
    match (standing, seen) {
        (Some(a), Some(b)) => {
            let (a_box, b_box) = (a.rect, b.rect);
            a.id == b.id
                && a.opacity.to_bits() == b.opacity.to_bits()
                && near(a_box.width, b_box.width)
                && near(a_box.height, b_box.height)
                && ((near(a_box.x, b_box.x) && near(a_box.y, b_box.y))
                    || (near(a_box.x - at.x, b_box.x - now.x)
                        && near(a_box.y - at.y, b_box.y - now.y)))
        }
        (None, _) | (_, None) => true,
    }
}

/// `timed_out`, the op's timeout error, as the facts of the cover that stood at the last
/// completed check say it: code `covered`, its lines before the call log, and the `cover`
/// field. Today's error, unchanged, under the conformance flag, when no cover stood, or when
/// the facts cannot be had.
pub(crate) async fn covered_timeout(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    watch: &CoverWatch,
    action: &str,
    timed_out: PlaywrightError,
) -> PlaywrightError {
    if scope.conformance() || timed_out.error_code() != Some(ErrorCode::NotActionable) {
        return timed_out;
    }
    covered_answer(scope, page, watch, action, None)
        .await
        .unwrap_or(timed_out)
}

/// The answer's first line: upstream's timeout line when the timeout ran out, and covered-click
/// B's fixed form when the action stopped early. It keeps the word `Timeout`, so a script that
/// tells a timeout by its message still does; the name stays `TimeoutError` and the code
/// `covered` either way.
fn first_line(scope: &OpScope<'_>, early: Option<EarlyStop>) -> String {
    match early {
        Some(stop) => format!(
            "Timeout cut short: the same cover stayed on top for {:.1} s ({} checks).",
            stop.stood.as_secs_f64(),
            stop.checks
        ),
        None => format!(
            "Timeout {}ms exceeded.",
            scope.timeout().map_or(0, |timeout| timeout.as_millis())
        ),
    }
}

/// The covered answer under the cover that stands: the cover's facts read from the world that saw
/// it (section 2). `None` when no cover stands or its facts cannot be read: then the caller
/// answers as it would without one, since the timing never comes before the facts. The timeout's
/// answer and covered-click B's early stop (`early`) share it: they differ only in the first line
/// and in the cover field's `stoodMs` and `checks`, present only on an early stop.
pub(crate) async fn covered_answer(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    watch: &CoverWatch,
    action: &str,
    early: Option<EarlyStop>,
) -> Option<PlaywrightError> {
    let standing = watch.standing()?;
    let mark = standing.mark?;
    let request = ProbeRequest::Cover {
        target: standing.seat.target.clone(),
        point: standing.seat.point,
        cover: mark.id,
        above: standing.seat.above.is_some(),
    };
    let asked = resolve::ask(scope, page, &standing.seat.world, &request, None);
    let Ok(Ok(ProbeReply::Cover { facts: Some(facts) })) = time::timeout(FACTS_BUDGET, asked).await
    else {
        return None;
    };
    let since = if standing.from_start {
        None
    } else {
        Some(standing.since.saturating_duration_since(watch.began))
    };
    let again = page.state().covered_again(LastCover {
        frame: standing.seat.world.frame().clone(),
        seq: standing.seat.world.seq(),
        cover: facts.layer,
        action: action.to_owned(),
        at: Instant::now(),
    });
    let lines = Lines {
        facts: &facts,
        action,
        since,
        stopped: watch.stopped,
        again: again.map(|before| (before.action, before.at.elapsed())),
    };
    let mut cover = field(&facts);
    if let (Some(stop), Value::Object(fields)) = (early, &mut cover) {
        fields.insert(
            "stoodMs".to_owned(),
            json!(u64::try_from(stop.stood.as_millis()).unwrap_or(u64::MAX)),
        );
        fields.insert("checks".to_owned(), json!(stop.checks));
    }
    Some(
        scope
            .fail(
                ErrorCode::Covered,
                &format!("{}{}", first_line(scope, early), lines.render()),
            )
            .with_cover(cover),
    )
}

/// The facts' lines, in the order a host that cuts a long message keeps them: what is on top
/// (the prelude puts the cover's own snapshot right under it), when it came, what was pressed,
/// the controls, and what to do. The call log follows them.
struct Lines<'a> {
    facts: &'a CoverFacts,
    /// The action's name as upstream logs it: `click`, `hover`, `check`.
    action: &'a str,
    /// The same cover answered this tab's last covered action: its name and how long ago.
    again: Option<(String, Duration)>,
    /// `None` when the cover was there at the action's first check.
    since: Option<Duration>,
    stopped: u32,
}

impl Lines<'_> {
    fn render(&self) -> String {
        let facts = self.facts;
        let action = self.action;
        let cover = node_named(&facts.role, facts.name.as_value(), &facts.r#ref, NAME_CHARS);
        let markup = line_of(facts.markup.as_value());
        // A layer that dims a dialog: the point lies under the layer, and the dialog is open over
        // it. The dialog is not what takes the click, and the words do not say it is.
        let mut out = if facts.behind {
            format!("\n  On top: a blank layer {markup}. Open over it: {cover}.")
        } else {
            format!("\n  On top: {cover} {markup}")
        };
        if let Some(text) = facts.text.as_ref().map(|text| line_of(text.as_value())) {
            let _ = write!(out, "\n    It says: {text}");
        }
        let _ = match self.since {
            None => write!(out, "\n  It has been there since this {action} began."),
            Some(after) => write!(
                out,
                "\n  It appeared {:.1} s into this {action}.",
                after.as_secs_f64()
            ),
        };
        let _ = match self.stopped {
            0 => write!(out, "\n  Nothing was pressed."),
            1 => write!(
                out,
                "\n  One press went out, and Playwright's interceptor stopped it at the window \
                 before any element of the page saw it."
            ),
            n => write!(
                out,
                "\n  {n} presses went out, and Playwright's interceptor stopped each at the \
                 window before any element of the page saw it."
            ),
        };
        if !facts.controls.is_empty() {
            out.push_str("\n  Its controls:");
            for control in &facts.controls {
                let control = node_named(
                    &control.role,
                    control.name.as_value(),
                    &control.r#ref,
                    NAME_CHARS,
                );
                let _ = write!(out, "\n    - {control}");
            }
        }
        let _ = write!(out, "\n  Do: {}", todo(facts));
        if let Some((before, ago)) = &self.again {
            let _ = write!(
                out,
                "\n  The same layer covered your last {before} on this tab, {} s ago. If you \
                 closed something since, it was not this layer. If its own controls do not \
                 remove it, it is stuck: a reload clears it, and with it what the page holds.",
                ago.as_secs()
            );
        }
        out
    }
}

/// The most characters of a cover's name, or a control's in the covered answer, a line prints.
const NAME_CHARS: usize = 80;

/// The most characters of a control's name on the `# on top:` lines' second line.
const CONTROL_NAME_CHARS: usize = 40;

/// The most controls the `# on top:` lines list: the covered answer's list, cut to four.
const ON_TOP_CONTROLS: usize = 4;

/// What a page snapshot's first lines, and a run's reply, say about one layer that takes the
/// pointer from controls (snapshot design section 21.10).
pub(crate) struct OnTopNote<'a> {
    pub(crate) role: &'a str,
    pub(crate) name: &'a Value,
    pub(crate) reference: &'a str,
    /// Nothing in it takes the pointer, it draws no words, and no dialog stands on it.
    pub(crate) blank: bool,
    /// It says it is loading: `aria-busy`, a progress bar.
    pub(crate) loading: bool,
    /// It takes the pointer over the whole view.
    pub(crate) whole_view: bool,
    /// Its own controls that take the pointer, buttons first: role, name and ref.
    pub(crate) controls: &'a [(&'a str, &'a Value, &'a str)],
    /// First drawn words, only for an unnamed nonblank layer with no controls.
    pub(crate) first_words: Option<&'a str>,
}

/// The one place Neo words a layer on top, for a page snapshot (section 21.10):
///
/// ```text
/// # on top: dialog "What You Need To Know" [ref=e301] over the whole view
/// #   its controls: button "Accept & Continue" [ref=e310], button "Dismiss popup" [ref=e299]
/// ```
///
/// A run's reply prints the same words after `[on top] tab <id>: `, the second line joined to
/// the first with `; its controls: `; the covered answer names its cover with the same
/// [`node_named`]. No counts and no advice: those are the covered answer's.
pub(crate) fn on_top_line(note: &OnTopNote<'_>) -> String {
    let mut line = format!(
        "# on top: {}",
        node_named(note.role, note.name, note.reference, NAME_CHARS)
    );
    if note.blank {
        line.push_str(", blank");
    }
    if note.loading {
        line.push_str(", loading");
    }
    if !note.blank
        && note.controls.is_empty()
        && note.name.as_str().is_none_or(|name| name.trim().is_empty())
        && let Some(words) = note.first_words.filter(|words| !words.is_empty())
    {
        let words = clipped(words, 60);
        let _ = write!(line, ", says {}", Value::from(words));
    }
    line.push_str(if note.whole_view {
        " over the whole view"
    } else {
        " over part of the view"
    });
    if !note.controls.is_empty() {
        let controls: Vec<String> = note
            .controls
            .iter()
            .filter(|(role, _, _)| *role == "button")
            .chain(
                note.controls
                    .iter()
                    .filter(|(role, _, _)| *role != "button"),
            )
            .take(ON_TOP_CONTROLS)
            .map(|(role, name, reference)| node_named(role, name, reference, CONTROL_NAME_CHARS))
            .collect();
        let _ = write!(line, "\n#   its controls: {}", controls.join(", "));
    }
    line
}

/// A node as a snapshot line names it: its role, its name quoted as a snapshot quotes one and
/// clipped to `chars` characters, and its ref. The role alone for a nameless node.
fn node_named(role: &str, name: &Value, reference: &str, chars: usize) -> String {
    match name.as_str().map(str::trim) {
        Some(name) if !name.is_empty() => {
            let clipped = clipped(name, chars);
            format!("{role} {} [ref={reference}]", Value::from(clipped))
        }
        _ => format!("{role} [ref={reference}]"),
    }
}

/// Every field in a layer line clips between Unicode characters and shares its ellipsis rule.
fn clipped(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((cut, _)) => format!("{}…", text.get(..cut).unwrap_or(text)),
        None => text.to_owned(),
    }
}

/// A page string on one line.
fn line_of(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// One line of what to do, by how the cover stands to the control (spec section 2's table).
fn todo(facts: &CoverFacts) -> String {
    match &facts.kind {
        CoverKind::Page if facts.closed => "its content is in a closed part of the page: a new \
                                            snapshot shows it."
            .to_owned(),
        CoverKind::Page if facts.behind => {
            "answer or close that dialog with one of its controls, then try again.".to_owned()
        }
        CoverKind::Page if facts.notice => format!(
            "a notice: it may go by itself. Wait for it, getByRef('{}').waitFor({{ state: \
             'hidden' }}), or close it with one of its controls, then try again.",
            facts.r#ref
        ),
        CoverKind::Page if facts.blank => {
            "a blank layer. Look for an open dialog in a new snapshot. If there is none, reload."
                .to_owned()
        }
        CoverKind::Page => format!(
            "answer or close it with one of its controls, then try again. If it is loading, \
             wait for it: getByRef('{}').waitFor({{ state: 'hidden' }}).",
            facts.r#ref
        ),
        CoverKind::Frame { inside: false } => "a frame covers it. Its content is under its ref in \
                                               the snapshot. If it is a bot check, use the solver."
            .to_owned(),
        CoverKind::Frame { inside: true } => "its content is in a frame: it is under the frame's \
                                              ref in the snapshot. If it is a bot check, use the \
                                              solver."
            .to_owned(),
        CoverKind::Partial { position } => format!(
            "only part of it is covered. Act on it at a clear point: {{ position: {} }}.",
            point_text(*position)
        ),
        CoverKind::Label { links, position } => {
            let mut line = "it sits under its own label. Press Space on it.".to_owned();
            if *links > 0 {
                let plural = if *links == 1 { "" } else { "s" };
                let _ = write!(
                    line,
                    " The label holds {links} link{plural}. A click on its middle may follow one"
                );
                let _ = match position {
                    Some(at) => write!(
                        line,
                        "; a click on the label at {{ position: {} }} follows none.",
                        point_text(*at)
                    ),
                    None => write!(line, "."),
                };
            }
            line
        }
        CoverKind::Wrapper => "its own wrapper takes the pointer. Click the wrapper, or press \
                               Space on the control."
            .to_owned(),
        CoverKind::Labels { r#ref } => {
            format!("it labels the control on top. Act on that control: getByRef('{ref}').")
        }
        CoverKind::Nothing => {
            "it takes no pointer at its centre. It may be text and not a control.".to_owned()
        }
        CoverKind::Folded => {
            "it is not shown where it lies: a closed panel or menu holds it. Open that first."
                .to_owned()
        }
    }
}

pub(crate) fn point_text(point: Point) -> String {
    format!("{{ x: {}, y: {} }}", point.x.round(), point.y.round())
}

/// The error's `cover` field: what a script reads as `error.cover` (its ref, role, name, kind
/// and controls) and, for the prelude alone, the cover's snapshot to render under "On top".
fn field(facts: &CoverFacts) -> Value {
    json!({
        "ref": facts.r#ref,
        "role": facts.role,
        "name": facts.name.as_value(),
        "kind": kind_name(&facts.kind),
        "controls": facts.controls.iter().map(|control| json!({
            "ref": control.r#ref,
            "role": control.role,
            "name": control.name.as_value(),
        })).collect::<Vec<_>>(),
        "snapshot": facts.snapshot,
    })
}

fn kind_name(kind: &CoverKind) -> &'static str {
    match kind {
        CoverKind::Page => "page",
        CoverKind::Frame { .. } => "frame",
        CoverKind::Partial { .. } => "partial",
        CoverKind::Label { .. } => "label",
        CoverKind::Wrapper => "wrapper",
        CoverKind::Labels { .. } => "labels",
        CoverKind::Nothing => "nothing",
        CoverKind::Folded => "folded",
    }
}

#[cfg(test)]
mod tests;
