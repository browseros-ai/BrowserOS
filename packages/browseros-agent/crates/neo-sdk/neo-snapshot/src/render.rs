use crate::answer::Answer;
use crate::aria_source::{AriaSource, SnapshotView};
use crate::call::Call;
use crate::delta::{DeltaText, delta_text_source};
use crate::full_text::full_text_source;
use crate::neo::{NeoText, neo_text_source};
use crate::playwright::tree_text_source;
use crate::snapshot::Snapshot;

/// Every text a snapshot can be, from one read: the crate's one entry for the snapshot operation.
///
/// Pure and deterministic: the same snapshot and call give the same bytes, with no clock, store,
/// browser or async. It cannot fail: a tree that decoded can be printed. Each format walks the
/// tree once on a stack of its own, so no depth is too deep; Neo's default format walks it twice
/// when the length rule leaves the page's text out, to count what it left out, streaming lines
/// directly into the cap without retaining lines past the limit. A delta adds one comparison
/// of the stored text with the page: a search bounded at 1,000 changed lines, whose state is at
/// most about 4 MB.
#[must_use]
pub fn render(snapshot: &Snapshot, call: Call<'_>) -> Answer {
    render_source(&SnapshotView::from_snapshot(snapshot), call)
}

/// Renders any [`AriaSource`] tree through the exact same formatters and cap rules as [`render`],
/// letting emitters stream directly from distilled drafts without materializing `AriaNode`s.
#[must_use]
pub fn render_source<'a, S: AriaSource<'a>>(view: &SnapshotView<'a, S>, call: Call<'_>) -> Answer {
    match call {
        Call::Playwright => Answer::keeping_nothing(tree_text_source(view)),
        Call::Full { max_tokens } => {
            Answer::keeping_nothing(full_text_source(view, max_tokens.unwrap_or_default()))
        }
        Call::Element(passed) => Answer::keeping_nothing(neo_text_source(view, passed).text),
        Call::Page(passed) => {
            let NeoText { text, form } = neo_text_source(view, passed);
            Answer::keeping_itself(text, form)
        }
        Call::Delta { passed, earlier } => {
            let DeltaText { returned, baseline } = delta_text_source(view, passed, earlier);
            match baseline {
                Some(baseline) => Answer::keeping_baseline(returned, baseline),
                None => Answer::keeping_nothing(returned),
            }
        }
    }
}
