use crate::cap::Printed;
use crate::snapshot::Snapshot;

use super::hidden::Hidden;
use super::on_top::OnTop;
use super::printer::{Inert, Omissions, Partials, Printer};
use super::rule::Child;

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

/// One form's lines, and the inert subtrees it left out, each printed with the text form it
/// inherited where it was left out.
#[derive(Debug)]
pub(super) struct Walked {
    pub(super) lines: Vec<Printed>,
    /// The lines the inert subtrees left out would have printed, for the `# not listed:` count.
    pub(super) omitted: Vec<Printed>,
    /// How many inert roots were left out; the `# not listed:` line prints when any was, even if
    /// they would have printed no line.
    pub(super) skipped: usize,
}

/// `walk`: the page in one form. What a whole-view layer holds first, always with its text,
/// then the rest of the page as a new text run, then the frames not read whose iframe printed no
/// line.
pub(super) fn walk(snapshot: &Snapshot, form: WalkForm) -> Walked {
    let on_top = OnTop::of(snapshot);
    let inert = if form.inert {
        Inert::Listed
    } else {
        Inert::Omitted(Omissions::default())
    };
    let mut printer = Printer::new(form.urls, Partials::of(&snapshot.partial), inert, &on_top);
    if !on_top.first.is_empty() {
        // One walk per held node, each with its own mark: a walk prints all of its children
        // before it returns, so the lines come out as one walk would print them.
        for held in &on_top.first {
            printer.walk([Child::Node(held.node)].into_iter(), 0, true, held.hidden);
        }
        printer.separate_text();
    }
    printer.walk(on_top.rest(&snapshot.tree), 0, form.text, Hidden::No);
    printer.comment_unplaced_partials();
    printer.finish()
}
