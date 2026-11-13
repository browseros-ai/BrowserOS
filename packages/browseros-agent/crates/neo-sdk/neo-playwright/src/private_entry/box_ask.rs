//! The look's request to a pinned box and its answer, as the probe protocol carries them
//! (probe request `look`, its `value` reply's payload). These also generate the page side's
//! declarations (`generated.ts`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::private_entry::{BoxFact, BoxFacts, Expect};

/// What the look checks at one box. It carries the value's length, never the value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BoxAsk {
    /// The facts the host approved; absent for a check call made before any approval, which
    /// checks no origin and no fact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(crate) expect: Option<Expect>,
    /// The value's length in UTF-16 code units, as a box's `maxlength` counts it. Absent for
    /// the host's check call, which has no value: that look skips the length check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(crate) length: Option<u32>,
    /// A value the box's `maxlength` cuts is accepted.
    pub(crate) cut: bool,
    /// The field is typed key by key, so a box with no size is reached by focus alone.
    pub(crate) keys: bool,
    /// Every `<iframe>` on the way down to the box's frame is visible: a box in a hidden frame
    /// does not show either.
    pub(crate) frame_visible: bool,
}

/// The look's answer for one box (`value` payload of the `look` request): its facts whenever it
/// is a text input, and the first check it fails. A covered box answers both. A box whose pin
/// is gone answers the protocol's `detached` instead.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Looked {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(crate) facts: Option<BoxFacts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(crate) refusal: Option<LookRefused>,
}

/// The first check a box fails, and for `boxChanged` the fact that differs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LookRefused {
    pub(crate) code: LookRefusal,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(crate) fact: Option<BoxFact>,
}

/// The checks the page side makes, in the order it makes them (design section 5.3, steps 2
/// to 7). Step 1, the ref naming one element, and the tab's address of step 3 are Rust's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) enum LookRefusal {
    /// Not an `<input>` whose type takes text.
    NotAnInput,
    /// The box's frame is not on the expected origin.
    WrongOrigin,
    /// The input's type is not the expected one.
    WrongType,
    /// Another approved fact differs.
    BoxChanged,
    /// Disabled or read-only.
    BoxNotEditable,
    /// The box does not show, or has no size and is not typed key by key.
    BoxHidden,
    /// Main's covered rule found something else at the box's centre.
    BoxCovered,
    /// The value is longer than the box's `maxlength`, and no cut is allowed.
    ValueTooLong,
}
