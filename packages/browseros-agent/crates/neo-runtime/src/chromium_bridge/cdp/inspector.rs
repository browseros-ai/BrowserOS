//! The `Inspector` domain as neo-runtime uses it: the events that end a run's session on a tab.
//!
//! Neo: payloads Neo never reads. The method alone says the session is gone, and
//! `Inspector.targetCrashed` carries no params at all.

use serde::Deserialize;

use super::event;

/// `Inspector.detached`: the session's inspector went away, with the tab or to another client.
#[derive(Deserialize)]
pub(crate) struct Detached {}

/// `Inspector.targetCrashed`: the tab's renderer crashed.
#[derive(Deserialize)]
pub(crate) struct TargetCrashed {}

event! {
    // Neo: the session's inspector detached, so the tab is gone for this run; Playwright learns the same from its browser session's `Target.detachedFromTarget`.
    Detached => "Inspector.detached";
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addBrowserListeners`: `eventsHelper.addEventListener(this._client, 'Inspector.targetCrashed', event => this._onTargetCrashed()),`
    TargetCrashed => "Inspector.targetCrashed";
}
