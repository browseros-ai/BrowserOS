//! The `Security` domain as neo-runtime uses it: a created context's `ignoreHTTPSErrors` on each
//! of its frame sessions (`browser_representations/emulation.rs`).
//!
//! Neo: browseros-cdp's generated params, made from BrowserOS's pinned protocol.

use browseros_cdp::security as generated;

use super::{Ignored, OnSession, command};

pub(crate) use generated::SetIgnoreCertificateErrorsParams as SetIgnoreCertificateErrors;

command! {
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `promises.push(this._client.send('Security.setIgnoreCertificateErrors', { ignore: true }));`
    SetIgnoreCertificateErrors => "Security.setIgnoreCertificateErrors", OnSession, Ignored;
}
