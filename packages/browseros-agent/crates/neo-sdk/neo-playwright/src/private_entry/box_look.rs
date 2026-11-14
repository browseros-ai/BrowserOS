//! What the look found at one box: its facts, or the refusal an entry would meet there.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::private_entry::EntryError;

/// A box's facts as the look read them in Neo's isolated world. The host approves these and
/// sends them back as `expect`, so the look at approval and the look at typing are one rule.
/// They describe the box, never what it holds beyond whether it is empty.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoxFacts {
    /// The origin of the box's own frame.
    pub origin: String,
    /// The input's `type` as the DOM reports it.
    #[serde(rename = "type")]
    pub kind: String,
    /// The `autocomplete` attribute; empty when absent.
    pub autocomplete: String,
    /// The `maxlength`; `None` when the box has none.
    pub max_length: Option<u32>,
    /// The `inputmode` attribute; empty when absent.
    pub input_mode: String,
    /// The `name` attribute; empty when absent.
    pub name: String,
    /// The `id` attribute; empty when absent.
    pub id: String,
    /// Whether the box has a size. A box with none skips the covered check: it has nothing to
    /// show or cover, and is reached by focus.
    pub sized: bool,
    /// Whether the box holds nothing now.
    pub empty: bool,
}

/// The look's answer for one box: each box answers on its own, with its facts and the first
/// refusal an entry would meet there now. A covered box answers both.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BoxLook {
    /// The box's facts, whenever the ref names a text input.
    pub facts: Option<BoxFacts>,
    /// The first check an entry would be refused by now; `None` when every check passed.
    pub refusal: Option<EntryError>,
    /// The check call's compare (ruling R48), whenever it sent a value and Neo read the box:
    /// absent when the box could not be read, and then the refusal says why. Never a `false`
    /// for a box that was not read.
    pub compared: Option<Compared>,
    /// Ruling R63, for a box whose ref is dead (`element_detached`): true when the frame the ref's
    /// world was installed in is no longer in the tab, read in a whole frame tree. False in every
    /// other case, the route's absent: unknown, never "the frame is there".
    pub frame_gone: bool,
}

/// What the check call's compare found in a box: booleans only, never the value, a position, a
/// length or a count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Compared {
    /// The box holds the value by the fill's read-back rule.
    pub holds: bool,
    /// The box's text contains the value by the same rule; true whenever `holds` is.
    pub contains: bool,
}
