//! The op contract, derived from the handlers themselves.
//!
//! Each family writes one exhaustive `route` match whose rows bind an op to its place in
//! Playwright's protocol and to its handler: `Self::Click => r.op(command("ElementHandle.click"),
//! click)`. The handler's signature, `async fn click(&Playwright, &OpScope<'_>, ClickInput) ->
//! Result<Option<Clicked>, _>`, is the only place the op's input and result shapes are written.
//! The same match is walked two ways:
//!
//! ```text
//!  Playwright::invoke ─▶ family::invoke ─▶ route(op, Run { .. })   decode I, call, encode O
//!  Op::wire()         ─▶ family::wire   ─▶ route(op, Describe)     OpWire: I's and O's TypeScript
//!                                                                    (ts-rs), the option keys,
//!                                                                    the upstream row
//!  tests/wire_generated.rs: Op::all() × wire() ─▶ neo-playwright-js/src/prelude/generated/
//!                                                  (rewritten and failing when stale)
//! ```
//!
//! Shapes only. Nothing here authorizes: `OpSpec` keeps every policy fact, and the door's rules
//! never read an `OpWire`.

mod op_wire;
mod option_keys;
mod route;
mod upstream;

pub use op_wire::{OpWire, WireType};
pub use option_keys::OptionNesting;
pub(crate) use option_keys::{ADDRESSING, OptionKeys, options, serde_fields};
pub(crate) use route::{Describe, Nothing, Route, Run};
pub use upstream::{Command, Upstream};
pub(crate) use upstream::{command, neo_only};
// No op replaces its upstream command since the expect ops carry `Frame.expect`'s params and
// interception carries `Page.updateSubscription`'s; the row stays for an op whose params cannot,
// and the machinery's own tests still build one.
#[cfg(test)]
pub(crate) use upstream::replaces;
