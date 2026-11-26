use crate::aria_source::{AriaSource, SnapshotView};
use crate::cap::{LineCap, Offer, UnitsSink, about_tokens, as_number, join, number, partial_head};
use crate::form::{Form, Passed, TextForm};

use super::on_top::{OnTop, on_top_head};
use super::printer::OmittedStats;
use super::unmarked_head::unmarked_head;
use super::walk::{WalkForm, walk_into};

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
pub(crate) fn neo_text_source<'a, S: AriaSource<'a>>(
    view: &SnapshotView<'a, S>,
    passed: &Passed,
) -> NeoText {
    let urls = passed.urls.unwrap_or(false);
    let inert = passed.inert == Some(true);
    let max_tokens = passed.max_tokens.unwrap_or_default();
    let on_top = OnTop::of(view);
    let abandon_above = passed.text.is_none().then_some(TEXT_LIMIT_UNITS);
    let walked = (passed.text != Some(false)).then(|| {
        walk_into(
            view,
            &on_top,
            WalkForm {
                text: true,
                urls,
                inert,
            },
            LineCap::with_abandon(max_tokens.units(), abandon_above),
        )
    });
    let text = match (&walked, passed.text) {
        (None, _) => TextForm::Never,
        (Some(_), Some(true)) => TextForm::Always,
        (Some((cap, _)), _) if cap.joined_units() <= TEXT_LIMIT_UNITS => TextForm::Kept,
        (Some(_), _) => TextForm::LeftOut,
    };
    let form = Form {
        text,
        urls,
        max_tokens,
        inert,
    };
    NeoText {
        text: render_form_source(view, &on_top, &form, walked),
        form,
    }
}

/// `renderSnapshotV1As`: the text rendered exactly as `form` says, the length rule not asked
/// again. `Kept` and `Always` print the page's text whatever its length, `LeftOut` prints the
/// notice whatever its length, `Never` neither. A delta renders the new snapshot in its
/// baseline's form, so the two differ only where the page does.
pub(crate) fn neo_text_as_source<'a, S: AriaSource<'a>>(
    view: &SnapshotView<'a, S>,
    form: &Form,
) -> String {
    let on_top = OnTop::of(view);
    render_form_source(view, &on_top, form, None)
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
fn render_form_source<'a, S: AriaSource<'a>>(
    view: &SnapshotView<'a, S>,
    on_top: &OnTop<S::Id, S::Key>,
    form: &Form,
    walked: Option<(LineCap, OmittedStats)>,
) -> String {
    let text = matches!(form.text, TextForm::Kept | TextForm::Always);
    let walk_form = |text| WalkForm {
        text,
        urls: form.urls,
        inert: form.inert,
    };
    let (shown_cap, omitted, text_form_units) = match walked {
        Some((cap, omitted)) if text => (cap, omitted, None),
        Some((cap, _)) => {
            let text_units = cap.joined_units();
            drop(cap);
            let (shown_cap, omitted) = walk_into(
                view,
                on_top,
                walk_form(false),
                LineCap::new(form.max_tokens.units()),
            );
            (shown_cap, omitted, Some(text_units))
        }
        None => {
            let (shown_cap, omitted) = walk_into(
                view,
                on_top,
                walk_form(text),
                LineCap::new(form.max_tokens.units()),
            );
            (shown_cap, omitted, None)
        }
    };
    let inert_head = (omitted.skipped > 0).then(|| {
        format!(
            "# not listed: {} lines ({} with refs) of this page are inert: nothing in them can \
             be clicked or typed into now. Pass {{ inert: true }} to list them.",
            omitted.lines, omitted.with_refs
        )
    });
    let notice = (form.text == TextForm::LeftOut).then(|| {
        let text_units = text_form_units.unwrap_or_else(|| {
            let (units_sink, _) = walk_into(
                view,
                on_top,
                WalkForm {
                    text: true,
                    urls: form.urls,
                    inert: form.inert,
                },
                UnitsSink::default(),
            );
            units_sink.joined_units()
        });
        let left = as_number(text_units) - as_number(shown_cap.joined_units());
        format!(
            "# text left out: about {} tokens. Pass {{ text: true }} to include it.",
            number(about_tokens(left))
        )
    });
    let pinned = partial_head(view.closed_roots);
    let body = shown_cap.finish(notice.as_slice(), pinned.as_slice(), Offer::Neo);
    let unmarked = unmarked_head(view.unmarked);
    join(
        on_top_head(view.layers)
            .chain(unmarked.as_deref())
            .chain(inert_head.as_deref())
            .chain(Some(body.as_str()).filter(|body| !body.is_empty())),
    )
}
