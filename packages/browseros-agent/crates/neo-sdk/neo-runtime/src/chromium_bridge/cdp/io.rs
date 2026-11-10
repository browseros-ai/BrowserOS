//! The `IO` domain as neo-runtime uses it: reading a PDF's stream and a refetched response body,
//! a chunk at a time, and closing the stream (`playwright_methods/pdf.rs`,
//! `browser_representations/responses.rs`).
//!
//! Neo: browseros-cdp's generated params, made from BrowserOS's pinned protocol, under each
//! method's name.

use browseros_cdp::io as generated;
use serde_json::Value;

use super::{Ignored, OnSession, command};

pub(crate) use generated::{CloseParams as Close, ReadParams as Read};

command! {
    // Playwright: `server/chromium/crProtocolHelper.ts` `readProtocolStream`: `await client.send('IO.close', { handle });`
    Close => "IO.close", OnSession, Ignored;
    // Playwright: `server/chromium/crProtocolHelper.ts` `readProtocolStream`: `const response = await client.send('IO.read', { handle });`
    // The reply stays JSON: its two readers (`pdf.rs`, `responses.rs`) each read a chunk its own
    // way, with its own errors.
    Read => "IO.read", OnSession, Value;
}
