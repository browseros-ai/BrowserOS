//! The `HeapProfiler` domain as neo-runtime uses it: `page.requestGC()`
//! (`browser_representations/emulation.rs`).
//!
//! Neo: a local type, since its one command takes no params and browseros-cdp generates no
//! params type for such a command.

use serde::Serialize;

use super::{Ignored, OnSession, command};

/// `HeapProfiler.collectGarbage`. Local: it takes no params, and browseros-cdp generates no
/// params type for such a command.
#[derive(Serialize)]
pub(crate) struct CollectGarbage {}

command! {
    // Playwright: `server/chromium/crPage.ts` `CRPage.requestGC`: `await this._mainFrameSession._client.send('HeapProfiler.collectGarbage');`
    CollectGarbage => "HeapProfiler.collectGarbage", OnSession, Ignored;
}
