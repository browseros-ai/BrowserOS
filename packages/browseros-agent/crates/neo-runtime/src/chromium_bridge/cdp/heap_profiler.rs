//! The `HeapProfiler` domain as neo-runtime uses it: `page.requestGC()`
//! (`browser_representations/emulation.rs`).
//!
//! Neo: a local struct, since browseros-cdp's surface has no `HeapProfiler` domain.

use serde::Serialize;

use super::{Ignored, OnSession, command};

/// `HeapProfiler.collectGarbage`, which takes no params.
#[derive(Serialize)]
pub(crate) struct CollectGarbage {}

command! {
    // Playwright: `server/chromium/crPage.ts` `CRPage.requestGC`: `await this._mainFrameSession._client.send('HeapProfiler.collectGarbage');`
    CollectGarbage => "HeapProfiler.collectGarbage", OnSession, Ignored;
}
