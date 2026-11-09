//! The `Log` domain as neo-runtime uses it: the browser's log entries, which Playwright reports
//! as console messages, on a run that listens to them (`chromium_bridge/page_events.rs`).
//!
//! Neo: a local struct, since browseros-cdp's surface has no `Log` domain; the entry is
//! decoded as `browser_representations/console.rs` reads it.

use serde::Serialize;

use super::{Ignored, OnSession, command, event};

pub(crate) use crate::browser_representations::console::LogEntryAdded as EntryAdded;

#[derive(Serialize)]
pub(crate) struct Enable {}

command! {
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `this._client.send('Log.enable', {}),`
    Enable => "Log.enable", OnSession, Ignored;
}

event! {
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addRendererListeners`: `eventsHelper.addEventListener(this._client, 'Log.entryAdded', event => this._onLogEntryAdded(event)),`
    EntryAdded => "Log.entryAdded";
}
