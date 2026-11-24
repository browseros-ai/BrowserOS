use crate::delta::Baseline;
use crate::form::Form;

/// What [`render`](crate::render) answers: the string the script receives, and what the call may
/// keep for the tab.
///
/// The fields are private and only `render` builds one. So the snapshot operation can record as
/// pending only a text this crate rendered, in the form it was rendered in, and only for a call
/// that keeps one: a whole page in Neo's default format, or a delta that read the stored text.
#[derive(Debug)]
pub struct Answer {
    text: String,
    keep: Keep,
}

/// What an answer may keep as the tab's pending baseline.
#[derive(Debug)]
enum Keep {
    /// A locator's text, `mode: 'full'`, Playwright's format: none is ever compared with a later
    /// snapshot. And a delta whose stored text was not read: the call that returns it records
    /// nothing, and the older baseline stays.
    Nothing,
    /// A whole page in Neo's default format: the answer's own text, in this form.
    Itself(Form),
    /// A delta that read the stored text: after `# no change` or `# changes`, the page now in the
    /// stored form; after a whole page, that page in its own. The answer is the string the run's
    /// reply must show for it to be committed.
    Other(Baseline),
}

/// The pending record's arguments, lent by [`Answer::pending`], so the string recorded is the
/// very string the operation returns and its form the one it was printed in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pending<'a> {
    /// What becomes the tab's baseline once the run's reply shows it.
    pub text: &'a str,
    /// The form `text` was rendered in, which a later delta renders the new page in.
    pub form: Form,
    /// What the call returned when that is not `text` itself. `None` for a whole page, whose
    /// answer is its own baseline. For a delta, its answer: the run's reply must show it for
    /// `text` to become the baseline.
    pub returned: Option<&'a str>,
}

impl Answer {
    /// An answer that keeps nothing for the tab.
    pub(crate) fn keeping_nothing(text: String) -> Self {
        Self {
            text,
            keep: Keep::Nothing,
        }
    }

    /// A whole page's answer, kept as itself in `form`.
    pub(crate) fn keeping_itself(text: String, form: Form) -> Self {
        Self {
            text,
            keep: Keep::Itself(form),
        }
    }

    /// A delta's answer, kept as `baseline`, which is not the answer itself.
    pub(crate) fn keeping_baseline(text: String, baseline: Baseline) -> Self {
        Self {
            text,
            keep: Keep::Other(baseline),
        }
    }

    /// The string for the script.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The same string, owned, once the pending record has been made.
    #[must_use]
    pub fn into_text(self) -> String {
        self.text
    }

    /// What to record as the tab's pending baseline, or `None` when this call keeps nothing.
    #[must_use]
    pub fn pending(&self) -> Option<Pending<'_>> {
        match &self.keep {
            Keep::Nothing => None,
            Keep::Itself(form) => Some(Pending {
                text: &self.text,
                form: *form,
                returned: None,
            }),
            Keep::Other(baseline) => Some(Pending {
                text: &baseline.text,
                form: baseline.form,
                returned: Some(&self.text),
            }),
        }
    }
}
