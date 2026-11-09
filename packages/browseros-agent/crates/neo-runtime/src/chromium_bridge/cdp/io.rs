//! The `IO` domain as neo-runtime uses it: reading a PDF's stream and a refetched response body,
//! a chunk at a time, and closing the stream (`playwright_methods/pdf.rs`,
//! `browser_representations/responses.rs`).
//!
//! Neo: local structs, since browseros-cdp's surface has no `IO` domain. A chunk stays JSON:
//! each reader decodes it with its own errors.

use serde::Serialize;
use serde_json::Value;

use super::{Ignored, OnSession, command};

#[derive(Serialize)]
pub(crate) struct Close {
    pub(crate) handle: String,
}

#[derive(Serialize)]
pub(crate) struct Read {
    pub(crate) handle: String,
}

command! {
    // Playwright: `server/chromium/crProtocolHelper.ts` `readProtocolStream`: `await client.send('IO.close', { handle });`
    Close => "IO.close", OnSession, Ignored;
    // Playwright: `server/chromium/crProtocolHelper.ts` `readProtocolStream`: `const response = await client.send('IO.read', { handle });`
    Read => "IO.read", OnSession, Value;
}
