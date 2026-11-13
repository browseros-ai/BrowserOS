//! Private typing: the host names boxes by ref and hands Neo the values; Neo looks at every
//! box, types each value character by character, reads it back and answers one outcome. The
//! value never becomes an op's input, so it reaches no record, event, reply or picture.
//! Design: `claw-sdk/protected-fill/design.md`.
//!
//! ```text
//!  host route (neo-server, host.fill)            this module                     the page
//!  body ─▶ PrivateText at the edge ─▶ EntryRequest
//!          a run of its own, the door ─▶ Playwright::enter_privately(env, request)
//!                                        hold the tab's input
//!                                        look at every box ─────────────────────▶ isolated world
//!                                        env.recheck, env.mark_sent (once)
//!                                        per box: look, focus, then per character
//!                                          env.recheck, focus check, key ───────▶ trusted keys
//!                                        read back ────────────────────────────▶ yes or no
//!          Entry { outcome, delivery, at, typed, error } ◀─┘
//! ```
//!
//! The look alone is [`crate::Playwright::look_boxes`]: the host's check call, which types
//! nothing. Neither entry point is an op of the script-facing table, so no script reaches
//! them; only a host's own Rust does.

mod box_ask;
mod box_look;
mod box_spec;
mod entry;
mod entry_ask;
mod entry_op;
mod entry_request;
mod entry_run;
mod look_op;
mod picture_check;
mod private_text;

pub(crate) use box_ask::{BoxAsk, LookRefusal, LookRefused, Looked};
pub use box_look::{BoxFacts, BoxLook};
pub use box_spec::{BoxSpec, Expect};
pub use entry::{
    BoxFact, Drawing, Entry, EntryCode, EntryError, EntryOutcome, Sending, delivery_of,
};
pub(crate) use entry_ask::{BoxState, FocusState, PageSecret, PrivateAnswer, PrivateStep};
pub use entry_request::{EntryRequest, LookField, LookRequest, PrivateField};
pub(crate) use entry_run::PrivateTabs;
pub use picture_check::PictureCheck;
pub use private_text::{MAX_PRIVATE_CHARS, PrivateText, PrivateTextRefused};
