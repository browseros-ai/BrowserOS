//! What a private entry answers: one outcome for the whole form, where it stopped, how many
//! characters went into each box, and the error record. None of it holds page text or a value.
//!
//! Delivery and `retryable` keep the SDK's one meaning each, so the host's door and the entry
//! can never disagree (design section 15):
//!
//! ```text
//!  stopped before env.mark_sent   delivery not_sent   retryable true   (every look refusal)
//!  stopped after it, definite     delivery done       retryable false  (not_focused, a lock)
//!  stopped after it, ambiguous    delivery unknown    retryable false  (focus_lost, timeout …)
//! ```

use neo_trace::{Delivery, ErrorOwner};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Answer, Refusal};

/// The host's first question about an entry: did any character go out, and did all land.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryOutcome {
    /// Every box holds its value; Neo read each one back.
    Entered,
    /// Every character went in, and Neo saw the value go into the last box; then the page
    /// moved, so it could not read it back. The form may have submitted.
    EnteredPageMoved,
    /// No character was typed anywhere.
    Refused,
    /// The boxes before `at` hold their values; box `at` got no character.
    Partial,
    /// Box `at` got some characters, or could not be read back.
    Uncertain,
}

impl EntryOutcome {
    /// The wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Entered => "entered",
            Self::EnteredPageMoved => "entered_page_moved",
            Self::Refused => "refused",
            Self::Partial => "partial",
            Self::Uncertain => "uncertain",
        }
    }
}

/// How a box's field drawing figured in its entry (rulings R55, R56): the read-back answers it,
/// and the operation's record notes every box entered under one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum Drawing {
    /// No drawing of its field covered the box.
    #[default]
    None,
    /// The box was entered under its field's own drawing, which did not draw the value.
    Under,
    /// Entered under its field's own drawing, which now draws the typed value outside the box,
    /// where it did not before the first character.
    Drawn,
}

/// An approved fact that no longer holds (`box_changed` names it). The input's `type` is not
/// one: a box of another type answers its own code, `wrong_type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum BoxFact {
    /// The `autocomplete` attribute.
    Autocomplete,
    /// The `maxlength`.
    MaxLength,
    /// The `inputmode` attribute.
    InputMode,
    /// The `name` attribute.
    Name,
    /// The `id` attribute.
    Id,
}

impl BoxFact {
    /// The fact's name in the host's `expect`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Autocomplete => "autocomplete",
            Self::MaxLength => "maxLength",
            Self::InputMode => "inputMode",
            Self::Name => "name",
            Self::Id => "id",
        }
    }
}

/// Where a look found what takes the pointer at a box's centre (`box_covered`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverIn {
    /// The box's own document: main's covered rule, with the field's own label and drawing
    /// excused (rulings R55 and R56).
    OwnDocument,
    /// A page above the box's frame (ruling R60): its hit test at the box's centre lands on
    /// something other than the frame's owner element or what holds it. Nothing is excused
    /// there, since Neo cannot tell that an element of one document belongs to a field of
    /// another.
    PageAbove,
}

/// Why a look could not ask a page above the box's frame what takes the pointer there
/// (`cover_unread`, ruling R60). Nothing is typed: an unasked page counts as covered, under a
/// code of its own so that a host can tell it from a real cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unread {
    /// A page above did not answer: its world or session failed under the asking.
    NotAsked,
    /// After the box was brought into view at once, a page above still cannot show its place:
    /// the page cannot scroll there, or the frame is clipped away.
    OutOfView,
    /// The box is not in view in its own frame after the look's instant scroll, and so could not
    /// be brought into view: the page cannot scroll there (ruling R62).
    Unreachable,
    /// A page above changes the frame's size or angle (a scale, a rotation, a skew, a 3D
    /// transform that is not a plain shift, the `scale` and `rotate` properties, a zoom), so
    /// the box's centre has no single place there. Neo never guesses a point.
    SizeOrAngle,
}

/// Every failure of a look or an entry Neo itself decides (design sections 4.4 and 15). A host
/// policy's refusal (`owner_has_control`, `page_busy`) is not one of these: it passes through
/// with the policy's own owner and code ([`EntryError::refused`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryCode {
    /// The ref names no element on the page now.
    ElementDetached,
    /// The element is not an `<input>` whose type takes text.
    NotAnInput,
    /// The box's frame is not on `expect.origin`.
    WrongOrigin,
    /// The input's `type` is not the host's `expect.type`: the host's own input disagrees with
    /// itself, as when an agent swapped two refs.
    WrongType,
    /// Another approved fact differs.
    BoxChanged(BoxFact),
    /// The tab is not at the host's `url`.
    PageChanged,
    /// Disabled or read-only.
    BoxNotEditable,
    /// The box does not show, or has no size and is not typed key by key.
    BoxHidden,
    /// Something else takes the pointer at the box's centre, in its own document or in a page
    /// above its frame.
    BoxCovered(CoverIn),
    /// A page above the box's frame could not be asked what takes the pointer at the box.
    CoverUnread(Unread),
    /// The value is longer than the box's `maxlength` and the host did not allow a cut.
    ValueTooLong,
    /// The box did not take the focus, or the page moved it before the box's first character.
    NotFocused,
    /// The focus left the box after some characters.
    FocusLost,
    /// The box holds something else at the read-back.
    ValueNotTaken,
    /// A box with no size still held text after Neo tried to empty it before the first
    /// character (ruling R51). Nothing of the value was typed.
    BoxNotCleared,
    /// A box entered under a cover (ruling R55): after the typing, the cover's wrapper drew the
    /// value outside the box, where it did not before. Neo emptied the box, and the wrapper no
    /// longer draws it.
    ValueDrawn,
    /// As [`Self::ValueDrawn`], but the wrapper still draws the value after the emptying: Neo
    /// cannot take text off a page, so the host must reload or close it before anything reads it.
    ValueStillDrawn,
    /// The page moved before Neo saw the value go into the box.
    PageMoved,
    /// The entry's own deadline passed.
    RunTimeout,
    /// The tab closed under the entry.
    TargetClosed,
    /// The browser connection dropped.
    BrowserDisconnected,
    /// A Neo bug: a probe reply Rust cannot read, a stale engine asset.
    Internal,
}

impl EntryCode {
    /// The stable code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ElementDetached => "element_detached",
            Self::NotAnInput => "not_an_input",
            Self::WrongOrigin => "wrong_origin",
            Self::WrongType => "wrong_type",
            Self::BoxChanged(_) => "box_changed",
            Self::PageChanged => "page_changed",
            Self::BoxNotEditable => "box_not_editable",
            Self::BoxHidden => "box_hidden",
            Self::BoxCovered(_) => "box_covered",
            Self::CoverUnread(_) => "cover_unread",
            Self::ValueTooLong => "value_too_long",
            Self::NotFocused => "not_focused",
            Self::FocusLost => "focus_lost",
            Self::ValueNotTaken => "value_not_taken",
            Self::BoxNotCleared => "box_not_cleared",
            Self::ValueDrawn => "value_drawn",
            Self::ValueStillDrawn => "value_still_drawn",
            Self::PageMoved => "page_moved",
            Self::RunTimeout => "run_timeout",
            Self::TargetClosed => "target_closed",
            Self::BrowserDisconnected => "browser_disconnected",
            Self::Internal => "internal",
        }
    }

    /// Whose behavior must change: the host's for what it asked, the website's for what the
    /// page did, Neo's for its own limits.
    #[must_use]
    pub const fn owner(self) -> ErrorOwner {
        match self {
            Self::NotAnInput | Self::WrongOrigin | Self::WrongType | Self::ValueTooLong => {
                ErrorOwner::Host
            }
            Self::ElementDetached
            | Self::BoxChanged(_)
            | Self::PageChanged
            | Self::BoxNotEditable
            | Self::BoxHidden
            | Self::BoxCovered(_)
            | Self::NotFocused
            | Self::FocusLost
            | Self::ValueNotTaken
            | Self::BoxNotCleared
            | Self::ValueDrawn
            | Self::ValueStillDrawn
            | Self::PageMoved => ErrorOwner::Website,
            Self::CoverUnread(_)
            | Self::RunTimeout
            | Self::TargetClosed
            | Self::BrowserDisconnected
            | Self::Internal => ErrorOwner::Neo,
        }
    }

    /// Whether Neo knows what happened once the entry was marked sent: the host's door derives
    /// `done` from a definite failure and `unknown` from an ambiguous one.
    #[must_use]
    pub const fn answer(self) -> Answer {
        match self {
            Self::ElementDetached
            | Self::NotAnInput
            | Self::WrongOrigin
            | Self::WrongType
            | Self::BoxChanged(_)
            | Self::PageChanged
            | Self::BoxNotEditable
            | Self::BoxHidden
            | Self::BoxCovered(_)
            | Self::CoverUnread(_)
            | Self::ValueTooLong
            | Self::NotFocused
            | Self::BoxNotCleared
            | Self::ValueDrawn
            | Self::ValueStillDrawn
            | Self::Internal => Answer::Definite,
            Self::FocusLost
            | Self::ValueNotTaken
            | Self::PageMoved
            | Self::RunTimeout
            | Self::TargetClosed
            | Self::BrowserDisconnected => Answer::Ambiguous,
        }
    }

    /// The fixed sentence. None quotes the page or the value.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::ElementDetached => {
                "The box is not on the page any more. Look at the page again. Nothing was typed."
            }
            Self::NotAnInput => "The ref does not name a text input. Nothing was typed.",
            Self::WrongOrigin => {
                "The box's frame is not on the origin the host expected. Nothing was typed."
            }
            Self::WrongType => {
                "The box's type is not the one the host expected. Nothing was typed."
            }
            Self::BoxChanged(fact) => match fact {
                BoxFact::Autocomplete => {
                    "The box's autocomplete is not what the host approved. Nothing was typed."
                }
                BoxFact::MaxLength => {
                    "The box's maxLength is not what the host approved. Nothing was typed."
                }
                BoxFact::InputMode => {
                    "The box's inputMode is not what the host approved. Nothing was typed."
                }
                BoxFact::Name => "The box's name is not what the host approved. Nothing was typed.",
                BoxFact::Id => "The box's id is not what the host approved. Nothing was typed.",
            },
            Self::PageChanged => {
                "The tab is not at the address the host expected. Nothing was typed."
            }
            Self::BoxNotEditable => "The box is disabled or read-only. Nothing was typed.",
            Self::BoxHidden => "The box does not show on the page. Nothing was typed.",
            Self::BoxCovered(CoverIn::OwnDocument) => {
                "Something else on the page takes the pointer at the box. Nothing was typed. \
                 Dismiss it, or scroll until the box is clear of it, and try again."
            }
            Self::BoxCovered(CoverIn::PageAbove) => {
                "Nothing was typed. Something in the page above the frame that holds this box \
                 takes the pointer where the box is, such as a banner, a dialog or a header. \
                 Dismiss it, or scroll until the box is clear of it, and try again."
            }
            Self::CoverUnread(Unread::NotAsked) => {
                "Nothing was typed. Neo could not check whether the page above this box's frame \
                 covers the box. Try again."
            }
            Self::CoverUnread(Unread::OutOfView) => {
                "Nothing was typed. The frame that holds this box is not in view in the page \
                 above it. Scroll the frame into view and try again."
            }
            Self::CoverUnread(Unread::Unreachable) => {
                "Nothing was typed. This box is not in view and could not be brought into view. \
                 Close what locks the page, such as a dialog, or scroll the box into view, and \
                 try again."
            }
            Self::CoverUnread(Unread::SizeOrAngle) => {
                "Nothing was typed. The page above this box's frame scales or rotates the frame, \
                 so Neo could not check what covers the box. If the page is still moving, try \
                 again."
            }
            Self::ValueTooLong => {
                "The value is longer than the box's maxlength allows. Nothing was typed."
            }
            Self::NotFocused => "The box did not keep the focus. Nothing was typed into it.",
            Self::FocusLost => "The focus left the box during the entry. Typing stopped there.",
            Self::ValueNotTaken => {
                "The box does not hold the value after the entry. The page changed what was typed."
            }
            Self::BoxNotCleared => "The box still holds text after Neo tried to empty it.",
            Self::ValueDrawn => {
                "The site drew the typed value outside the box. Neo emptied the box, and the page \
                 no longer shows the value."
            }
            Self::ValueStillDrawn => {
                "The site drew the typed value outside the box, and the page still shows it. \
                 Reload or close the page before anything reads it."
            }
            Self::PageMoved => {
                "The page moved during the entry, before Neo saw the value go into the box."
            }
            Self::RunTimeout => "The entry ran out of time.",
            Self::TargetClosed => "The tab closed during the entry.",
            Self::BrowserDisconnected => "Neo lost its connection to the browser.",
            Self::Internal => "Neo failed inside the entry.",
        }
    }
}

/// Whether the entry had been marked sent when it stopped: what `retryable` and the delivery
/// are derived from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sending {
    /// Nothing reached the page beyond a scroll: every look runs before this.
    NotYet,
    /// `env.mark_sent` was called.
    Sent,
}

/// The operation's error record, in the SDK's format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryError {
    /// Whose behavior must change.
    pub owner: ErrorOwner,
    /// The stable code: an [`EntryCode`]'s, or a host policy's own.
    pub code: String,
    /// A fixed sentence, or a host policy's message.
    pub message: String,
    /// The SDK's one meaning: `true` only when nothing was sent, so the same call is safe to
    /// repeat. Whether a repeat could succeed is the host's reading of `code`.
    pub retryable: bool,
    /// The host's own code, when a host policy refused with `host_error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_code: Option<String>,
}

impl EntryError {
    /// The record of one of Neo's own codes, `sending` as it stood when the entry stopped.
    #[must_use]
    pub fn of(code: EntryCode, sending: Sending) -> Self {
        Self {
            owner: code.owner(),
            code: code.as_str().to_owned(),
            message: code.message().to_owned(),
            retryable: sending == Sending::NotYet,
            host_code: None,
        }
    }

    /// A host policy's refusal, passed through with its own owner, code and message. Neo saw
    /// it end, so it is definite.
    #[must_use]
    pub fn refused(refusal: Refusal, sending: Sending) -> Self {
        Self {
            owner: refusal.owner,
            code: refusal.code,
            message: refusal.message,
            retryable: sending == Sending::NotYet,
            host_code: refusal.host_code,
        }
    }

    /// Whether Neo knows what happened once the entry was sent, for the host's door: it derives
    /// `done` from a definite error and `unknown` from an ambiguous one. Derived from `code`, so
    /// the two cannot disagree; a host policy's own code is a refusal Neo saw end, definite.
    #[must_use]
    pub fn answer(&self) -> Answer {
        match self.code.as_str() {
            "focus_lost"
            | "value_not_taken"
            | "page_moved"
            | "run_timeout"
            | "target_closed"
            | "browser_disconnected" => Answer::Ambiguous,
            _ => Answer::Definite,
        }
    }
}

/// One private entry's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The answer for the whole form.
    pub outcome: EntryOutcome,
    /// The door's word for the entry's effect, derived as the door derives it: `not_sent` when
    /// the entry was never marked sent, else `done` for success or a definite error and
    /// `unknown` for an ambiguous one.
    pub delivery: Delivery,
    /// The index of the field the entry stopped at; `None` when it did not stop.
    pub at: Option<usize>,
    /// How many characters went to each box, in the request's order.
    pub typed: Vec<usize>,
    /// Why it stopped; `None` for `entered` and `entered_page_moved`.
    pub error: Option<EntryError>,
    /// How each box's field drawing figured in its entry, in the request's order; `None` for a
    /// box not read back (rulings R55, R56).
    pub drawings: Vec<Drawing>,
}

/// The door's derivation (`neo-server/src/dispatch/operation_guard.rs`): what an op marked sent
/// and its error's answer make of its delivery.
#[must_use]
pub fn delivery_of(sending: Sending, error: Option<&EntryError>) -> Delivery {
    match (sending, error.map(EntryError::answer)) {
        (Sending::NotYet, _) => Delivery::NotSent,
        (Sending::Sent, None | Some(Answer::Definite)) => Delivery::Done,
        (Sending::Sent, Some(Answer::Ambiguous)) => Delivery::Unknown,
    }
}

impl Entry {
    /// Refused before anything reached the page.
    #[must_use]
    pub fn refused_before_sending(fields: usize, at: usize, error: EntryError) -> Self {
        Self {
            outcome: EntryOutcome::Refused,
            delivery: Delivery::NotSent,
            at: Some(at),
            typed: vec![0; fields],
            error: Some(error),
            drawings: vec![Drawing::None; fields],
        }
    }
}

#[cfg(test)]
mod tests {
    use neo_trace::Delivery;

    use super::{CoverIn, EntryCode, EntryError, Sending, delivery_of};
    use crate::Answer;

    #[test]
    fn tests_that_retryable_means_nothing_was_sent() {
        let look = EntryError::of(EntryCode::BoxCovered(CoverIn::OwnDocument), Sending::NotYet);
        assert!(look.retryable, "every look refusal comes before sending");
        let focus = EntryError::of(EntryCode::NotFocused, Sending::Sent);
        assert!(!focus.retryable, "the focus reached the page");
        let late = EntryError::of(EntryCode::RunTimeout, Sending::Sent);
        assert!(!late.retryable);
        let early = EntryError::of(EntryCode::RunTimeout, Sending::NotYet);
        assert!(
            early.retryable,
            "a timeout before the first focus sent nothing"
        );
    }

    #[test]
    fn tests_that_the_delivery_is_the_doors() {
        let focus = EntryError::of(EntryCode::NotFocused, Sending::Sent);
        assert_eq!(focus.answer(), Answer::Definite);
        assert_eq!(delivery_of(Sending::Sent, Some(&focus)), Delivery::Done);
        let lost = EntryError::of(EntryCode::FocusLost, Sending::Sent);
        assert_eq!(delivery_of(Sending::Sent, Some(&lost)), Delivery::Unknown);
        let covered = EntryError::of(EntryCode::BoxCovered(CoverIn::OwnDocument), Sending::NotYet);
        assert_eq!(
            delivery_of(Sending::NotYet, Some(&covered)),
            Delivery::NotSent
        );
        assert_eq!(delivery_of(Sending::Sent, None), Delivery::Done);
    }

    #[test]
    fn tests_that_an_errors_answer_is_its_codes() {
        let codes = [
            EntryCode::ElementDetached,
            EntryCode::NotAnInput,
            EntryCode::WrongOrigin,
            EntryCode::WrongType,
            EntryCode::BoxChanged(super::BoxFact::Name),
            EntryCode::PageChanged,
            EntryCode::BoxNotEditable,
            EntryCode::BoxHidden,
            EntryCode::BoxCovered(CoverIn::OwnDocument),
            EntryCode::ValueTooLong,
            EntryCode::NotFocused,
            EntryCode::FocusLost,
            EntryCode::ValueNotTaken,
            EntryCode::BoxNotCleared,
            EntryCode::ValueDrawn,
            EntryCode::ValueStillDrawn,
            EntryCode::PageMoved,
            EntryCode::RunTimeout,
            EntryCode::TargetClosed,
            EntryCode::BrowserDisconnected,
            EntryCode::Internal,
        ];
        for code in codes {
            assert_eq!(
                EntryError::of(code, Sending::Sent).answer(),
                code.answer(),
                "{code:?}"
            );
        }
    }

    #[test]
    fn tests_that_wrong_type_is_the_hosts() {
        assert_eq!(EntryCode::WrongType.as_str(), "wrong_type");
        assert_eq!(EntryCode::WrongType.owner(), neo_trace::ErrorOwner::Host);
    }
}
