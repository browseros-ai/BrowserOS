//! Jev decisions for blind Neo scripts: which element, is this true, which value.
//!
//! The page arrives as snapshot TEXT (the prelude's `page.ariaSnapshot(..)`); nothing here touches
//! a browser. One call flows:
//!
//! ```text
//! PageText ─▶ view (parse, twin groups, batches) ─▶ client (HTTP to Jev) ─▶ gate (verdict)
//! ```
//!
//! `view` and `gate` are pure, so the saved eval cases replay without a network. `client` owns the
//! key, the long-context header and its retries (long context once; a 401 once, only when the key
//! file holds a new bearer). Verdicts are values; only transport faults are errors (`JevFault`).
//! The gate labels a verdict clear, unsure or ambiguous; stopping the script on it is the
//! prelude's job in `neo-server`.

mod client;
mod config;
mod fault;
pub mod gate;
mod jev;
mod verdict;
pub mod view;

pub use config::JevConfig;
pub use fault::JevFault;
pub use jev::Jev;
pub use verdict::{
    CheckVerdict, ChooseVerdict, Cost, EarlierPick, FindAllVerdict, FindOpts, FindOutcome,
    FindVerdict, Flags, Obstacle, PageText, Pick, Saw,
};
