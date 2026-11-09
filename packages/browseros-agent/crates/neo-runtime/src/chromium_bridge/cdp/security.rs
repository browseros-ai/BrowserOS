//! The `Security` domain as neo-runtime uses it: a created context's `ignoreHTTPSErrors` on each
//! of its frame sessions (`browser_representations/emulation.rs`).
//!
//! Neo: a local struct, since browseros-cdp's surface has no `Security` domain.

use serde::Serialize;

use super::{Ignored, OnSession, command};

#[derive(Serialize)]
pub(crate) struct SetIgnoreCertificateErrors {
    pub(crate) ignore: bool,
}

command! {
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `promises.push(this._client.send('Security.setIgnoreCertificateErrors', { ignore: true }));`
    SetIgnoreCertificateErrors => "Security.setIgnoreCertificateErrors", OnSession, Ignored;
}
