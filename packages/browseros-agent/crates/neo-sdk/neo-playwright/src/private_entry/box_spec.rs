//! One box as the host names it for a private entry: a ref, the site it must be on, and the facts
//! the host approved. No value: this is what may be recorded.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What the host approved about a box. The look refuses the box when a fact the host sent no
/// longer holds (`box_changed`), so the box Neo types into is the box the host approved.
///
/// The look in the page reads the same shape (probe request `look`), so the facts the host
/// approved travel to the box's isolated world unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename = "BoxExpect")]
pub struct Expect {
    /// The origin of the box's own frame, `https://pay.example`. Required: Neo never types
    /// "wherever the ref ended up".
    pub origin: String,
    /// The input's `type` attribute as the DOM reports it (`password`, `text`, ...).
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub kind: Option<String>,
    /// The `autocomplete` attribute, exactly as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub autocomplete: Option<String>,
    /// The box's `maxlength`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub max_length: Option<u32>,
    /// The `inputmode` attribute, exactly as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub input_mode: Option<String>,
    /// The `name` attribute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// The `id` attribute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub id: Option<String>,
}

/// One box of a private entry or a look, without its value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoxSpec {
    /// The box's ref, as a snapshot printed it or `locator.ref()` minted it: `e5`, or `f2e4`
    /// for a box in frame document 2. A ref names one node and never re-binds; a gone node
    /// answers `element_detached`.
    #[serde(rename = "ref")]
    pub box_ref: String,
    /// The facts the box must still have.
    pub expect: Expect,
    /// Accept a value the box's own `maxlength` cuts: Neo types the part that fits.
    #[serde(default)]
    pub cut: bool,
    /// Compare the read-back with spaces, dashes and slashes set aside, for a box that groups
    /// digits (`4242 4242 …`) or formats an expiry (`12 / 29`).
    #[serde(default)]
    pub spaced: bool,
    /// How the value is typed. `false`, the default: one insertion, as `fill` types, for a
    /// password, a username, a card number or a code in one box; it needs a box that shows.
    /// `true`: key by key, with the focus checked before each character, for a code with one
    /// character in each box, a box with no size, or a masked box that needs key events
    /// (design section 5.5). A form that submits on its first input event takes only one
    /// insertion.
    #[serde(default)]
    pub keys: bool,
}
