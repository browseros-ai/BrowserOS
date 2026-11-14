//! What a host hands Neo for one private entry, and for one look without values.

use browseros_core::TabId;
use tokio::time::Instant;

use crate::private_entry::{BoxSpec, Expect, PrivateText};

/// One box and the value that goes into it. Deliberately not `Serialize`: the value is
/// [`PrivateText`]. A host records [`PrivateField::spec`] and the value's
/// [`PrivateText::char_count`] instead.
#[derive(Debug)]
pub struct PrivateField {
    /// The box, as the host names it.
    pub spec: BoxSpec,
    /// What Neo types into it.
    pub value: PrivateText,
}

/// One private entry: every box of a form, typed in order in one operation.
#[derive(Debug)]
pub struct EntryRequest {
    /// The browser's tab. There is no default tab.
    pub tab: TabId,
    /// When set, the tab's address must still equal it (`page_changed`).
    pub url: Option<String>,
    /// The boxes in typing order, at least one.
    pub fields: Vec<PrivateField>,
    /// When the whole entry must end. The entry stops itself there and answers `run_timeout`:
    /// `refused` before the first character, `uncertain` after it, and no further character
    /// goes out. The host's own race with the run's deadline is only a backstop.
    pub deadline: Instant,
}

/// One look at boxes without typing: the check call, which answers each box's facts or the
/// refusal the entry would meet.
#[derive(Debug)]
pub struct LookRequest {
    /// The browser's tab.
    pub tab: TabId,
    /// When set, the tab's address must still equal it.
    pub url: Option<String>,
    /// The boxes, at least one.
    pub fields: Vec<LookField>,
    /// What the check call compares in each box (ruling R48), in the fields' order: `None`, or a
    /// list shorter than the fields, compares nothing for a field. Kept beside the fields, as a
    /// fill's values are, because a host's value is never cloned or recorded.
    pub compares: Vec<Option<LookCompare>>,
    /// When the look must end.
    pub deadline: Instant,
}

/// A host's value the check call compares with one box (ruling R48), and the read-back's rule for
/// it. The value goes to the box's isolated world only, is compared there, and comes back as two
/// booleans.
#[derive(Debug)]
pub struct LookCompare {
    /// The value.
    pub value: PrivateText,
    /// Equal once whitespace, dashes and slashes are set aside, as the fill's `spaced`.
    pub spaced: bool,
    /// The part the box's `maxlength` keeps, as the fill's `cut`.
    pub cut: bool,
}

/// One box of a look. The check call can come before the host approved anything, so its
/// expectation is optional.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookField {
    /// The box's ref, as in [`BoxSpec::box_ref`].
    pub box_ref: String,
    /// The facts the box must have. `None` checks no origin and no fact: the look answers the
    /// box's facts and only the refusals that need no approval (gone, not a text input, not
    /// editable, hidden, covered).
    pub expect: Option<Expect>,
    /// The field will be typed key by key, so a box with no size is not refused as hidden.
    pub keys: bool,
}

impl From<&BoxSpec> for LookField {
    fn from(spec: &BoxSpec) -> Self {
        Self {
            box_ref: spec.box_ref.clone(),
            expect: Some(spec.expect.clone()),
            keys: spec.keys,
        }
    }
}
