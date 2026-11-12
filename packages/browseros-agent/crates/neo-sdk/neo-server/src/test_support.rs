//! Test doubles for claw-sdk's own tests and for integration tests under `tests/`. Compiled only
//! with the `test-support` feature, which this crate's dev-dependency on itself turns on for
//! every test build; a release build never contains it.
//!
//! [`FakeCdp`] is a browser behind core's `CdpConnection` seam: tabs, target events, a socket
//! that can drop and come back at a new epoch, and a log of every CDP call. Core's own test
//! connection is private to core, and it answers from a script of queued replies, which cannot
//! model a tab that stays closed or a reconnect.

//!
//! [`test_config`] is a process config for `App::start_with_fake_browser`, which runs the real
//! router and wiring over a `FakeCdp`.
//!
//! [`envelope_examples`] renders the run envelope (a run's init, op replies) through the code a
//! real run uses, for `tests/envelope_generated.rs` to pin byte for byte; [`envelope_types`] is
//! the TypeScript of the same types, which that test writes for the prelude.

mod envelope_examples;
mod envelope_types;
mod fake_cdp;
mod run_value;
mod test_config;

pub use crate::mcp::{ReplyText, parse_text};
pub use envelope_examples::{EnvelopeExample, envelope_examples};
pub use envelope_types::{EnvelopeType, envelope_types};
pub use fake_cdp::{FakeCall, FakeCdp};
pub use run_value::run_value;
pub use test_config::{CLOSED_XVNC, test_config};
