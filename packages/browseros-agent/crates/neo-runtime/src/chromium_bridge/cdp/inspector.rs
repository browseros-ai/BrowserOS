//! The `Inspector` domain as neo-runtime uses it: the events that end a run's session on a tab.
//!
//! Neo: browseros-cdp's generated events, made from BrowserOS's pinned protocol. Readers match
//! the method alone: the event says the session is gone.

use browseros_cdp::inspector as generated;

use super::event;

pub(crate) use generated::{DetachedEvent as Detached, TargetCrashedEvent as TargetCrashed};

event! {
    // Neo: the session's inspector detached, so the tab is gone for this run; Playwright learns the same from its browser session's `Target.detachedFromTarget`.
    Detached => "Inspector.detached";
    // Playwright: `server/chromium/crPage.ts` `FrameSession._addBrowserListeners`: `eventsHelper.addEventListener(this._client, 'Inspector.targetCrashed', event => this._onTargetCrashed()),`
    TargetCrashed => "Inspector.targetCrashed";
}
