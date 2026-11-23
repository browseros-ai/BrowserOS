use crate::answer::Answer;
use crate::call::Call;
use crate::delta::Earlier;

use super::case_file::Pending;
use super::first_difference::first_difference;

/// What today's renderer printed and kept for one call. A committed case, a saved case and today's
/// renderer in QuickJS each say it in this shape, so every judge holds [`render`](crate::render)'s
/// answer to it the same way.
#[derive(Clone, Copy)]
pub(crate) struct Expected<'a> {
    /// The text the script received.
    pub(crate) text: Option<&'a str>,
    /// The tab's pending baseline and its form, for a whole page in Neo's format.
    pub(crate) pending: Option<&'a Pending>,
}

/// How a difference is worded.
#[derive(Clone, Copy)]
pub(crate) enum Shown {
    /// The first differing line's two texts: for the committed cases and the generated trees.
    Lines,
    /// The first differing line's number and the two lengths only: the private sets are real
    /// pages' content, and none of it may reach a test's output.
    Lengths,
}

impl Expected<'_> {
    /// How `answer` to `call` differs from this, `None` when it does not: its text; what it keeps
    /// by the crate's own rule (a whole page in Neo's format keeps itself; a delta that read the
    /// stored text keeps a baseline, recorded as returned with the answer's very text; a delta
    /// that did not, and every other call, nothing); and what it keeps against what today kept,
    /// text and form.
    pub(crate) fn difference(
        &self,
        call: &Call<'_>,
        answer: &Answer,
        shown: Shown,
    ) -> Result<Option<String>, String> {
        let expected = self.text.ok_or("every call render prints has a text")?;
        if let Some((line, printed_line, expected_line)) = first_difference(answer.text(), expected)
        {
            return Ok(Some(match shown {
                Shown::Lines => format!(
                    "line {line}\n  printed:  {printed_line:?}\n  expected: {expected_line:?}"
                ),
                Shown::Lengths => format!(
                    "line {line} differs; {} bytes printed, {} expected",
                    answer.text().len(),
                    expected.len()
                ),
            }));
        }
        let pending = answer.pending();
        let rule = match (call, pending) {
            (Call::Page(_), None) => Some("a whole page keeps no baseline"),
            (Call::Page(_), Some(kept)) if kept.returned.is_some() => {
                Some("a whole page's pending record names another returned text")
            }
            (Call::Page(_), Some(kept)) if kept.text != answer.text() => {
                Some("a whole page keeps a text that is not its answer")
            }
            (Call::Playwright | Call::Full { .. } | Call::Element(_), Some(_)) => {
                Some("a call that keeps nothing keeps a baseline")
            }
            (Call::Delta { earlier, .. }, kept) => match (earlier, kept) {
                (Earlier::NotRead, Some(_)) => {
                    Some("a delta whose stored text was not read keeps a baseline")
                }
                (Earlier::Nothing | Earlier::Text(_), None) => {
                    Some("a delta that read the stored text keeps no baseline")
                }
                (_, Some(kept)) if kept.returned != Some(answer.text()) => {
                    Some("a delta's pending record does not name its answer as returned")
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(rule) = rule {
            return Ok(Some(rule.to_owned()));
        }
        Ok(match (self.pending, pending) {
            (None, None) => None,
            (Some(_), None) => Some("today keeps a baseline and Rust keeps none".to_owned()),
            (None, Some(_)) => Some("Rust keeps a baseline and today keeps none".to_owned()),
            (Some(today), Some(kept)) if today.text != kept.text => {
                Some("the kept text is not today's".to_owned())
            }
            (Some(today), Some(kept)) => {
                let form = today.form()?;
                (form != kept.form)
                    .then(|| format!("the form {:?} kept, {form:?} today", kept.form))
            }
        })
    }
}
