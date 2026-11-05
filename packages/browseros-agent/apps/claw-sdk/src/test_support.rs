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

mod fake_cdp;
mod test_config;

pub use fake_cdp::{FakeCall, FakeCdp};
pub use test_config::test_config;
