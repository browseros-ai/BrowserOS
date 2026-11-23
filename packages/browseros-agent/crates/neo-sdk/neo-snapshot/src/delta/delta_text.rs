use std::borrow::Cow;

use crate::cap::join;
use crate::form::{Form, Passed, TextForm};
use crate::neo::{
    is_on_top_line, is_unmarked_line, neo_text, neo_text_as, on_top_delta, on_top_head,
    unmarked_head,
};
use crate::snapshot::Snapshot;

use super::earlier::{Baseline, Earlier};
use super::list_changes::list_changes;

/// The first line when the tab holds no earlier snapshot of this document.
const NO_BASELINE: &str = "# whole page: no earlier snapshot of this page";

/// The first line when the changes would take more than half of the page, and also when the
/// options passed disagree with the stored form: true either way, so it never says "the page
/// changed".
const TOO_MUCH: &str = "# whole page: too much differs from your last snapshot to list as changes";

/// The first line when the call's time ran out before the stored text could be read: it cannot
/// say whether there was one, so it never says "no earlier snapshot".
const NO_TIME: &str = "# whole page: no time was left to look for an earlier snapshot of this page";

/// A delta's answer and what the tab may keep for it (the TypeScript's `DeltaResult`).
#[derive(Debug)]
pub(crate) struct DeltaText {
    /// The string the script receives.
    pub(crate) returned: String,
    /// What becomes the tab's baseline once the run ends with `returned` in the caller's hands:
    /// after `# no change` or `# changes`, the page now in the stored form; after a whole page,
    /// that page in its own form. `None` when the stored text was not read.
    pub(crate) baseline: Option<Baseline>,
}

/// A delta call's answer from what the tab held (`renderDelta`, and `renderUncompared` when the
/// stored text was not read).
pub(crate) fn delta_text(snapshot: &Snapshot, passed: &Passed, earlier: Earlier<'_>) -> DeltaText {
    match earlier {
        Earlier::NotRead => DeltaText {
            returned: render_uncompared(snapshot, passed),
            baseline: None,
        },
        Earlier::Nothing => render_delta(None, snapshot, passed),
        Earlier::Text(stored) => render_delta(Some(stored), snapshot, passed),
    }
}

/// `renderDelta`: the answer against the tab's stored text (`None`: none for this document).
///
/// With no stored text, the whole page by the options passed. With one whose form every option
/// passed agrees with, the page rendered in that form and compared with it, line by line. Else,
/// or when the changes are too long, the whole page: the options passed, the rest from the stored
/// form, a `text` neither call passed going back to the length rule, since a whole page is a new
/// snapshot and that rule exists so a big page's text is not paid for unasked.
fn render_delta(stored: Option<&Baseline>, snapshot: &Snapshot, passed: &Passed) -> DeltaText {
    let unmarked = unmarked_head(snapshot);
    let Some(stored) = stored else {
        let whole = neo_text(snapshot, passed);
        let body = without_read_heads(&whole.text);
        return DeltaText {
            returned: join(
                [NO_BASELINE]
                    .into_iter()
                    .chain(on_top_head(snapshot))
                    .chain(unmarked.as_deref())
                    .chain([body.as_str()]),
            ),
            baseline: Some(Baseline {
                text: whole.text,
                form: whole.form,
            }),
        };
    };
    let form = stored.form;
    if agrees(passed, &form) {
        let current = neo_text_as(snapshot, &form);
        if let Some(changes) = list_changes(&stored.text, &current) {
            // The first line tells unchanged, changed and whole-page answers apart, so the facts
            // of this read follow it and stay outside every hunk.
            let (status, body) = match changes.split_once('\n') {
                Some((status, body)) => (status, Some(body)),
                None => (changes.as_str(), None),
            };
            let on_top = on_top_delta(&stored.text, snapshot);
            return DeltaText {
                returned: join(
                    [status]
                        .into_iter()
                        .chain(on_top.iter().map(Cow::as_ref))
                        .chain(unmarked.as_deref())
                        .chain(body),
                ),
                baseline: Some(Baseline {
                    text: current,
                    form,
                }),
            };
        }
    }
    let whole = neo_text(
        snapshot,
        &Passed {
            text: passed.text.or(passed_text(form.text)),
            urls: Some(passed.urls.unwrap_or(form.urls)),
            max_tokens: Some(passed.max_tokens.unwrap_or(form.max_tokens)),
            inert: Some(passed.inert.unwrap_or(form.inert)),
        },
    );
    let on_top = on_top_delta(&stored.text, snapshot);
    let body = without_read_heads(&whole.text);
    DeltaText {
        returned: join(
            [TOO_MUCH]
                .into_iter()
                .chain(on_top.iter().map(Cow::as_ref))
                .chain(unmarked.as_deref())
                .chain([body.as_str()]),
        ),
        baseline: Some(Baseline {
            text: whole.text,
            form: whole.form,
        }),
    }
}

/// `renderUncompared`: the whole page by the options passed, as with no stored text, under
/// `NO_TIME`. The call that returns it records nothing, so it keeps nothing.
fn render_uncompared(snapshot: &Snapshot, passed: &Passed) -> String {
    let unmarked = unmarked_head(snapshot);
    let body = without_read_heads(&neo_text(snapshot, passed).text);
    join(
        [NO_TIME]
            .into_iter()
            .chain(on_top_head(snapshot))
            .chain(unmarked.as_deref())
            .chain([body.as_str()]),
    )
}

/// `agrees`: every option the call passed agrees with `form`. `text: true` with a form that shows
/// the page's text, `text: false` with one that does not; `urls`, `maxTokens` and `inert` when
/// equal, `maxTokens` exactly, fractions included. Two forms are never compared: every line they
/// render differently would be listed as a change that did not happen.
fn agrees(passed: &Passed, form: &Form) -> bool {
    passed.text.is_none_or(|text| text == shows_text(form.text))
        && passed.urls.is_none_or(|urls| urls == form.urls)
        && passed
            .max_tokens
            .is_none_or(|max_tokens| max_tokens == form.max_tokens)
        && passed.inert.is_none_or(|inert| inert == form.inert)
}

/// `showsText`: a text of this form prints the page's text.
fn shows_text(text: TextForm) -> bool {
    matches!(text, TextForm::Kept | TextForm::Always)
}

/// `passedText`: the `text` option the call that rendered this form passed, `None` when it left
/// it out.
fn passed_text(text: TextForm) -> Option<bool> {
    match text {
        TextForm::Always => Some(true),
        TextForm::Never => Some(false),
        TextForm::Kept | TextForm::LeftOut => None,
    }
}

/// `withoutReadHeads`: a whole page given back under a first line, without its on-top and
/// `# refs missing:` lines, which the answer prints once, after the first line.
fn without_read_heads(text: &str) -> String {
    join(
        text.split('\n')
            .filter(|line| !is_on_top_line(line) && !is_unmarked_line(line)),
    )
}
