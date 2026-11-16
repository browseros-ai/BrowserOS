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
    since: Instant,
    checks: u32,
    /// It was there at the action's first completed check.
    from_start: bool,
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
            && same_cover(standing.mark, mark)
        {
            standing.checks += 1;
            if standing.mark.is_none() {
                standing.mark = mark;
            }
            standing.seat = seat;
            return;
        }
        self.standing = Some(Standing {
            seat,
            mark,
            since: Instant::now(),
            checks: 1,
            from_start: first,
        });
    }

    /// The cover the last completed check found, if it found one.
    pub(crate) fn standing(&self) -> Option<&Standing> {
        self.standing.as_ref()
    }
}

/// The same cover: the same root, box and opacity. A cover known only from a verdict's miss
/// takes the next mark.
fn same_cover(standing: Option<CoverMark>, seen: Option<CoverMark>) -> bool {
    match (standing, seen) {
        (Some(a), Some(b)) => {
            a.id == b.id && a.rect == b.rect && a.opacity.to_bits() == b.opacity.to_bits()
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
    let Some(standing) = watch.standing() else {
        return timed_out;
    };
    let Some(mark) = standing.mark else {
        return timed_out;
    };
    let request = ProbeRequest::Cover {
        target: standing.seat.target.clone(),
        point: standing.seat.point,
        cover: mark.id,
        above: standing.seat.above.is_some(),
    };
    let asked = resolve::ask(scope, page, &standing.seat.world, &request, None);
    let Ok(Ok(ProbeReply::Cover { facts: Some(facts) })) = time::timeout(FACTS_BUDGET, asked).await
    else {
        return timed_out;
    };
    let millis = scope.timeout().map_or(0, |timeout| timeout.as_millis());
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
    scope
        .fail(
            ErrorCode::Covered,
            &format!("Timeout {millis}ms exceeded.{}", lines.render()),
        )
        .with_cover(field(&facts))
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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the snapshot's `# on top:` lines and the run reply's are built on this module"
    )
)]
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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the snapshot's `# on top:` lines and the run reply's are built on this module"
    )
)]
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
    line.push_str(if note.whole_view {
        " over the whole view"
    } else {
        " over part of the view"
    });
    if !note.controls.is_empty() {
        let controls: Vec<String> = note
            .controls
            .iter()
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
            let clipped: String = match name.char_indices().nth(chars) {
                Some((cut, _)) => format!("{}…", name.get(..cut).unwrap_or(name)),
                None => name.to_owned(),
            };
            format!("{role} {} [ref={reference}]", Value::from(clipped))
        }
        _ => format!("{role} [ref={reference}]"),
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
        CoverKind::Frame => "a frame covers it. Its content is under its ref in the snapshot. If \
                             it is a bot check, use the solver."
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

fn point_text(point: Point) -> String {
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
        CoverKind::Frame => "frame",
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
