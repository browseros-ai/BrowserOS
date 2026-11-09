//! The `Log` domain as neo-runtime uses it: the browser's log entries, which Playwright reports
//! as console messages, on a run that listens to them (`chromium_bridge/page_events.rs`).
//!
//! Neo: local types, each with its reason in its doc.

use serde::Serialize;

use super::{Ignored, OnSession, command, event};

/// `Log.entryAdded`, decoded as `browser_representations/console.rs` reads it. Not the generated
/// event: its `RemoteObject` cannot tell `"value": null` from no value, and a console message's
/// text depends on that (`console.rs` `RemoteObject`).
pub(crate) use crate::browser_representations::console::LogEntryAdded as EntryAdded;

/// `Log.enable`. Local: it takes no params, and browseros-cdp generates no params type for such a
/// command.
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
