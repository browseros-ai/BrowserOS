//! The `Emulation` domain as neo-runtime uses it: focus emulation, so a background tab paints,
//! and what a page emulates for a run: its viewport and screen, media, user agent, locale,
//! timezone, touch, script execution, position and screenshot background
//! (`browser_representations/emulation.rs`, `cookies_and_storage.rs`, `screenshot.rs`).
//!
//! Neo: browseros-cdp's generated params, made from BrowserOS's pinned protocol, under each
//! method's name; a local type only where its doc says why. Every reply is `{}` and none is
//! read.

use browseros_cdp::emulation as generated;
use serde::Serialize;

use super::{Ignored, OnSession, command};

pub(crate) use generated::{
    MediaFeature, ScreenOrientation,
    SetDefaultBackgroundColorOverrideParams as SetDefaultBackgroundColorOverride,
    SetDeviceMetricsOverrideParams as SetDeviceMetricsOverride,
    SetEmulatedMediaParams as SetEmulatedMedia,
    SetFocusEmulationEnabledParams as SetFocusEmulationEnabled,
    SetGeolocationOverrideParams as SetGeolocationOverride,
    SetLocaleOverrideParams as SetLocaleOverride,
    SetScriptExecutionDisabledParams as SetScriptExecutionDisabled,
    SetTimezoneOverrideParams as SetTimezoneOverride,
    SetTouchEmulationEnabledParams as SetTouchEmulationEnabled,
    SetUserAgentOverrideParams as SetUserAgentOverride, UserAgentMetadata,
};

/// `Emulation.clearGeolocationOverride`. Local: it takes no params, and browseros-cdp generates
/// no params type for such a command.
#[derive(Serialize)]
pub(crate) struct ClearGeolocationOverride {}

command! {
    // Neo: a run takes the position it set off its session again when it releases the session's state (`cookies_and_storage.rs` `Overrides::release`), before the session detaches; Playwright's sessions end with their browser context instead.
    ClearGeolocationOverride => "Emulation.clearGeolocationOverride", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `CRPage.setBackgroundColor`: `await this._mainFrameSession._client.send('Emulation.setDefaultBackgroundColorOverride', { color });`
    SetDefaultBackgroundColorOverride => "Emulation.setDefaultBackgroundColorOverride", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._updateViewport`: `promises.push(this._client.send('Emulation.setDeviceMetricsOverride', metricsOverride));`
    SetDeviceMetricsOverride => "Emulation.setDeviceMetricsOverride", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._updateEmulateMedia`: `await this._client.send('Emulation.setEmulatedMedia', { media, features });`
    SetEmulatedMedia => "Emulation.setEmulatedMedia", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `promises.push(this._client.send('Emulation.setFocusEmulationEnabled', { enabled: true }));`
    SetFocusEmulationEnabled => "Emulation.setFocusEmulationEnabled", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._updateGeolocation`: `await this._client.send('Emulation.setGeolocationOverride', geolocation || {});`
    SetGeolocationOverride => "Emulation.setGeolocationOverride", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `emulateLocale`: `await session.send('Emulation.setLocaleOverride', { locale });`
    SetLocaleOverride => "Emulation.setLocaleOverride", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `promises.push(this._client.send('Emulation.setScriptExecutionDisabled', { value: true }));`
    SetScriptExecutionDisabled => "Emulation.setScriptExecutionDisabled", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `emulateTimezone`: `await session.send('Emulation.setTimezoneOverride', { timezoneId: timezoneId });`
    SetTimezoneOverride => "Emulation.setTimezoneOverride", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `promises.push(this._client.send('Emulation.setTouchEmulationEnabled', { enabled: true }));`
    SetTouchEmulationEnabled => "Emulation.setTouchEmulationEnabled", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._updateUserAgent`: `await session.send('Emulation.setUserAgentOverride', {`
    SetUserAgentOverride => "Emulation.setUserAgentOverride", OnSession, Ignored;
}
