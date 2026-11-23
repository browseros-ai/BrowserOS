use crate::cap::{Offer, about_tokens, as_number, cap, join, joined_units, number, partial_head};
use crate::form::{Form, Passed, TextForm};
use crate::snapshot::Snapshot;

use super::on_top::on_top_head;
use super::unmarked_head::unmarked_head;
use super::walk::{WalkForm, Walked, walk};

/// The longest text form Neo's default format returns when the script did not choose, in tokens:
/// past it the page's text is left out under the `# text left out:` notice. Part of the
/// interface because the run description says it in words, and its test compares its prose with
/// this value.
pub const TEXT_LIMIT_TOKENS: u32 = 3_000;

/// [`TEXT_LIMIT_TOKENS`] in UTF-16 units, four to a token, as the length rule measures the text.
const TEXT_LIMIT_UNITS: usize = TEXT_LIMIT_TOKENS as usize * 4;

/// One text in Neo's default format and how it was rendered. They travel together because a
/// whole page's text is kept as the tab's baseline in the form of the very text the caller got.
#[derive(Debug)]
pub(crate) struct NeoText {
    pub(crate) text: String,
    pub(crate) form: Form,
}

/// `renderSnapshotV1(result, withDefaults(passed))`: the default format with Neo's defaults for
/// what the script left out (no urls, a cap of 8,000 tokens, inert subtrees counted). Unless
/// `text: false`, the page is walked with its text first; left out by the script, the text is
/// kept while those lines join to at most 12,000 units, and is otherwise left out under the
/// notice. The walk is reused where the form needs it.
pub(crate) fn neo_text(snapshot: &Snapshot, passed: &Passed) -> NeoText {
    let urls = passed.urls.unwrap_or(false);
    let inert = passed.inert == Some(true);
    let walked = (passed.text != Some(false)).then(|| {
        walk(
            snapshot,
            WalkForm {
                text: true,
                urls,
                inert,
            },
        )
    });
    let text = match (&walked, passed.text) {
        (None, _) => TextForm::Never,
        (Some(_), Some(true)) => TextForm::Always,
        (Some(walked), _) if joined_units(&walked.lines) <= TEXT_LIMIT_UNITS => TextForm::Kept,
        (Some(_), _) => TextForm::LeftOut,
    };
    let form = Form {
        text,
        urls,
        max_tokens: passed.max_tokens.unwrap_or_default(),
        inert,
    };
    NeoText {
        text: render_form(snapshot, &form, walked),
        form,
    }
}

/// `renderSnapshotV1As`: the text rendered exactly as `form` says, the length rule not asked
/// again. `Kept` and `Always` print the page's text whatever its length, `LeftOut` prints the
/// notice whatever its length, `Never` neither. A delta renders the new snapshot in its
/// baseline's form, so the two differ only where the page does.
pub(crate) fn neo_text_as(snapshot: &Snapshot, form: &Form) -> String {
    render_form(snapshot, form, None)
}

/// `renderForm`: `form`'s text, reusing the text form's walk when the caller made it. In order:
///
/// ```text
///  the on-top lines                       outside the cap
///  # refs missing: …                      outside the cap, when the check did not finish
///  # not listed: N lines (M with refs) …  outside the cap, whenever an inert root was left out,
///                                         N = 0 included
///  the cap's text:
///    # text left out: about T tokens …    the cap's head, counted toward the limit
///    # partial: closed shadow roots …     pinned: printed whatever the limit, not counted
///    the tree's lines, # cut: …, the kept blocks after the cut
/// ```
///
/// An empty cap's text prints no line, so the text never ends in a blank one. The notice's
/// number is what the text form's lines would add, so a `LeftOut` form walks the page with its
/// text too, here as well when a delta renders a stored form: saving that walk would print
/// another number.
fn render_form(snapshot: &Snapshot, form: &Form, walked: Option<Walked>) -> String {
    let text = matches!(form.text, TextForm::Kept | TextForm::Always);
    let walk_form = |text| WalkForm {
        text,
        urls: form.urls,
        inert: form.inert,
    };
    let (shown, walked) = match walked {
        Some(walked) if text => (walked, None),
        walked => (walk(snapshot, walk_form(text)), walked),
    };
    let inert_head = (shown.skipped > 0).then(|| {
        let with_refs = shown.omitted.iter().filter(|line| line.has_ref).count();
        format!(
            "# not listed: {} lines ({with_refs} with refs) of this page are inert: nothing in \
             them can be clicked or typed into now. Pass {{ inert: true }} to list them.",
            shown.omitted.len()
        )
    });
    let notice = (form.text == TextForm::LeftOut).then(|| {
        let text_form = walked.unwrap_or_else(|| walk(snapshot, walk_form(true)));
        let left =
            as_number(joined_units(&text_form.lines)) - as_number(joined_units(&shown.lines));
        format!(
            "# text left out: about {} tokens. Pass {{ text: true }} to include it.",
            number(about_tokens(left))
        )
    });
    let pinned = partial_head(snapshot);
    let body = cap(
        notice.as_slice(),
        pinned.as_slice(),
        &shown.lines,
        form.max_tokens.units(),
        Offer::Neo,
    );
    let unmarked = unmarked_head(snapshot);
    join(
        on_top_head(snapshot)
            .chain(unmarked.as_deref())
            .chain(inert_head.as_deref())
            .chain(Some(body.as_str()).filter(|body| !body.is_empty())),
    )
}
