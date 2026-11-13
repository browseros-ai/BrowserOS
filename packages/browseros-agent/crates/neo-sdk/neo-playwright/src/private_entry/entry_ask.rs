//! The entry's steps on a pinned box as the probe protocol carries them (probe request
//! `private`, its `value` reply's payload). These also generate the page side's declarations.

use std::{fmt, hint, mem};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::private_entry::{Drawing, PrivateText};

/// A private value on its way to Neo's isolated world, where the box's listener and the
/// read-back compare with it: the whole value, or the part a `maxlength` lets in. It prints
/// as `[private]` and is zeroed when dropped, as [`PrivateText`] is. Only the entry builds one,
/// from a [`PrivateText`]; probe calls never log their requests (`probe/probe_call.rs`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(transparent)]
#[ts(type = "string")]
pub(crate) struct PageSecret(String);

impl PageSecret {
    /// The part of `value` that fits `max_units` UTF-16 code units, as a box's `maxlength`
    /// counts, cut between characters, never inside one (ruling R12); all of it for `None`.
    pub(crate) fn fitting(value: &PrivateText, max_units: Option<usize>) -> Self {
        let mut text = String::new();
        let mut units = 0_usize;
        for character in value.chars() {
            units += character.len_utf16();
            if max_units.is_some_and(|max| units > max) {
                break;
            }
            text.push(character);
        }
        Self(text)
    }

    /// No text: what a box holds once it is empty.
    pub(crate) fn empty() -> Self {
        Self(String::new())
    }

    /// The same text without its last character: what a box typed key by key holds right
    /// before its last key.
    pub(crate) fn all_but_last(&self) -> Self {
        let mut text = String::with_capacity(self.0.len());
        let count = self.0.chars().count();
        text.extend(self.0.chars().take(count.saturating_sub(1)));
        Self(text)
    }

    /// The characters, in order, for the keyboard.
    pub(crate) fn chars(&self) -> impl Iterator<Item = char> + '_ {
        self.0.chars()
    }

    /// The whole text, for one insertion.
    pub(crate) fn text(&self) -> &str {
        &self.0
    }

    /// How many characters: what `typed` counts for this box.
    pub(crate) fn char_count(&self) -> usize {
        self.0.chars().count()
    }
}

impl fmt::Debug for PageSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[private]")
    }
}

impl Drop for PageSecret {
    fn drop(&mut self) {
        let mut bytes = mem::take(&mut self.0).into_bytes();
        let capacity = bytes.capacity();
        bytes.clear();
        bytes.resize(capacity, 0);
        hint::black_box(&mut bytes);
    }
}

/// One step of the entry on a pinned box.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum PrivateStep {
    /// Marks the box, starts its listener with the text it should hold, focuses it and selects
    /// what it holds; answers the focus state.
    Focus { text: PageSecret, spaced: bool },
    /// The focus state alone.
    Focused,
    /// Whether the box holds exactly `text` now.
    Holds { text: PageSecret, spaced: bool },
    /// The read-back; it also forgets the text.
    ReadBack,
    /// Forgets the text and stops the listener.
    Forget,
    /// Marks the text control that holds the focus in the box's document, when it is not the
    /// box (ruling R26): what an entry that stopped at `focus_lost` or `value_not_taken` may
    /// have typed into.
    MarkFocused,
    /// After a box whose read-back found the value drawn was emptied (ruling R55): whether the
    /// wrapper that drew it still draws `text`. Reads only that wrapper, whatever became of the
    /// box, and keeps nothing of `text`. Compared with white space, dashes and slashes set
    /// aside, whatever the field's `spaced`.
    Drawn { text: PageSecret },
}

/// Where the focus is, as Playwright's focused rule reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) enum FocusState {
    /// The box holds its tree's focus and its document has the focus.
    Held,
    /// The box holds its tree's focus, and its document does not have the focus yet: a
    /// cross-site frame whose focus handlers wait for input.
    NoDocumentFocus,
    /// Another element holds the focus.
    Elsewhere,
    /// The box's page started to move since the focus step: a form submitted, or the document
    /// began to unload. A key now would reach a document that already left with what it had.
    Moving,
    /// The box left its document.
    Gone,
}

/// What the read-back found in the box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) enum BoxState {
    /// The value, or with `spaced` the value as the box groups it.
    Holds,
    /// Anything else.
    Other,
    /// The box left its document, which is still there.
    Left,
}

/// The answer to one step (`value` payload of the `private` request).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum PrivateAnswer {
    Focus {
        focus: FocusState,
    },
    Holds {
        holds: bool,
    },
    /// `moving`: a submit no handler prevented, or the document unloading, since the focus step.
    /// `drawing`: whether the box was entered under its field's own drawing, and whether that
    /// drawing now shows the value (rulings R55, R56).
    ReadBack {
        state: BoxState,
        held_at_last_input: bool,
        moving: bool,
        drawing: Drawing,
    },
    Forgot,
    /// Whether a text control other than the box held the focus and is now marked.
    MarkedFocused {
        marked: bool,
    },
    /// Whether the wrapper still draws the value, or `true` when the page side cannot tell.
    Drawn {
        drawn: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::PageSecret;
    use crate::private_entry::{PrivateText, PrivateTextRefused};

    #[test]
    fn tests_that_a_cut_counts_utf16_units_and_never_splits_a_character()
    -> Result<(), PrivateTextRefused> {
        let value = PrivateText::new("pässwörd-😀-long".to_owned())?;
        assert_eq!(PageSecret::fitting(&value, Some(8)).text(), "pässwörd");
        // The emoji takes two units: with room for one, it is left out whole.
        assert_eq!(PageSecret::fitting(&value, Some(10)).text(), "pässwörd-");
        assert_eq!(PageSecret::fitting(&value, Some(11)).text(), "pässwörd-😀");
        assert_eq!(PageSecret::fitting(&value, None).char_count(), 15);
        assert_eq!(
            PageSecret::fitting(&value, Some(3)).all_but_last().text(),
            "pä"
        );
        assert_eq!(
            format!("{:?}", PageSecret::fitting(&value, None)),
            "[private]"
        );
        Ok(())
    }
}
