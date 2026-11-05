//! Password, card and one-time-code fields: which fields hold secrets, and how Neo shows their
//! values wherever it reads them (sdk-privacy's L1, "hide it at the source").
//!
//! One rule table (`rules.json`) has two evaluators that must agree:
//! - [`FieldRules`], over a snapshot's DOM capture and accessibility names; the snapshot masks
//!   with it when the session's [`SensitiveValues`] is `Mask`;
//! - `seal.js` ([`seal_source`]), in claw-sdk's sealed isolated world over the live DOM; there
//!   every read of a sensitive field returns the same mask, and a journal reports values typed
//!   into such fields.
//!
//! Additive: claw-server keeps [`SensitiveValues::Heuristic`] and never installs the seal.

mod field_rules;
mod field_values;
mod mask_value;
mod seal;
mod sensitive_values;

pub use field_rules::{FieldAttributes, FieldRules, FieldTag, SensitiveKind};
pub use field_values::{ECHO_MASK, FieldValues, MIN_SUBSTRING_LEN, html_spellings};
pub use mask_value::{FIELD_MASK, is_card_number, mask_value};
pub use seal::{FRAME_CROSSING_TOKENS, JOURNAL_BINDING, JournalEntry, SEALED_WORLD, seal_source};
pub use sensitive_values::SensitiveValues;

pub(crate) use mask_value::passes_luhn;
pub(crate) use sensitive_values::SensitiveValuesSetting;
