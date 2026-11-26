use crate::aria_source::{AriaSource, SnapshotView, SourceChild};
use crate::cap::LineSink;

use super::hidden::Hidden;
use super::on_top::OnTop;
use super::printer::{Inert, Omissions, OmittedStats, Partials, Printer};

/// How one walk prints the page.
#[derive(Clone, Copy, Debug)]
pub(super) struct WalkForm {
    /// The page's text is printed (the text form), or left out (the interactive form).
    pub(super) text: bool,
    /// Each link's `/url` line is printed.
    pub(super) urls: bool,
    /// Inert subtrees are listed, not counted.
    pub(super) inert: bool,
}

/// `walk`: the page in one form, streamed into `sink`. What a whole-view layer holds first,
/// always with its text, then the rest of the page as a new text run, then the frames not read
/// whose iframe printed no line.
pub(super) fn walk_into<'a, S: AriaSource<'a>, K: LineSink>(
    view: &SnapshotView<'a, S>,
    on_top: &OnTop<S::Id, S::Key>,
    form: WalkForm,
    sink: K,
) -> (K, OmittedStats) {
    let inert = if form.inert {
        Inert::Listed
    } else {
        Inert::Omitted(Box::new(Omissions::default()))
    };
    let mut printer = Printer::new(
        &view.tree,
        sink,
        form.urls,
        Partials::of(view.partial),
        inert,
        on_top,
    );
    if !on_top.first.is_empty() {
        // One walk per held node, each with its own mark: a walk prints all of its children
        // before it returns, so the lines come out as one walk would print them.
        for held in &on_top.first {
            printer.walk(
                [SourceChild::Node(held.id)].into_iter(),
                0,
                true,
                held.hidden,
            );
        }
        printer.separate_text();
    }
    printer.walk(on_top.rest(&view.tree), 0, form.text, Hidden::No);
    printer.comment_unplaced_partials();
    printer.finish()
}
