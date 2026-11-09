//! The `Emulation` domain as neo-runtime uses it: focus emulation, so a background tab paints,
//! and what a page emulates for a run: its viewport and screen, media, user agent, locale,
//! timezone, touch, script execution, position and screenshot background
//! (`browser_representations/emulation.rs`, `cookies_and_storage.rs`, `screenshot.rs`).
//!
//! Neo: local structs, since browseros-cdp's surface has no `Emulation` domain. Every reply is
//! `{}` and none is read.

use serde::Serialize;
use serde_json::Value;

use super::{Ignored, OnSession, command};

/// `Emulation.clearGeolocationOverride`, which takes no params.
#[derive(Serialize)]
pub(crate) struct ClearGeolocationOverride {}

/// `Emulation.setDefaultBackgroundColorOverride`: `color` while a screenshot omits the
/// background, `{}` to end the override.
#[derive(Serialize)]
pub(crate) struct SetDefaultBackgroundColorOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) color: Option<Rgba>,
}

/// CDP's `DOM.RGBA`. Neo sends only transparent black, as the integers Playwright's JSON
/// carries.
#[derive(Serialize)]
pub(crate) struct Rgba {
    pub(crate) r: u8,
    pub(crate) g: u8,
    pub(crate) b: u8,
    pub(crate) a: u8,
}

/// `Emulation.setDeviceMetricsOverride`, as Playwright's `FrameSession._updateViewport` builds
/// it. The page's emulation keeps the last one it sent, to skip sending the same again.
#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetDeviceMetricsOverride {
    pub(crate) mobile: bool,
    pub(crate) width: i64,
    pub(crate) height: i64,
    pub(crate) screen_width: i64,
    pub(crate) screen_height: i64,
    pub(crate) device_scale_factor: f64,
    pub(crate) screen_orientation: ScreenOrientation,
}

/// CDP's `Emulation.ScreenOrientation`.
#[derive(Clone, PartialEq, Serialize)]
pub(crate) struct ScreenOrientation {
    pub(crate) angle: i64,
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
}

/// `Emulation.setEmulatedMedia`: the media type and Playwright's four media features, each `""`
/// where nothing overrides it. Sent to the tab's session and every frame session of it.
#[derive(Clone, Serialize)]
pub(crate) struct SetEmulatedMedia {
    pub(crate) media: &'static str,
    pub(crate) features: Vec<MediaFeature>,
}

/// CDP's `Emulation.MediaFeature`.
#[derive(Clone, Serialize)]
pub(crate) struct MediaFeature {
    pub(crate) name: &'static str,
    pub(crate) value: &'static str,
}

#[derive(Serialize)]
pub(crate) struct SetFocusEmulationEnabled {
    pub(crate) enabled: bool,
}

/// `Emulation.setGeolocationOverride`: a position, or `{}` (every field `None`), which makes
/// the position unavailable. A run's hold on a tab keeps the position it set, to put it back.
#[derive(Clone, Default, Serialize)]
pub(crate) struct SetGeolocationOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) latitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) longitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) accuracy: Option<f64>,
}

#[derive(Serialize)]
pub(crate) struct SetLocaleOverride {
    pub(crate) locale: String,
}

#[derive(Serialize)]
pub(crate) struct SetScriptExecutionDisabled {
    pub(crate) value: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetTimezoneOverride {
    pub(crate) timezone_id: String,
}

#[derive(Serialize)]
pub(crate) struct SetTouchEmulationEnabled {
    pub(crate) enabled: bool,
}

/// `Emulation.setUserAgentOverride`, as Playwright's `FrameSession._updateUserAgent` builds it.
/// `userAgentMetadata` is the client hints `calculateUserAgentMetadata` derives, built as JSON.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetUserAgentOverride {
    pub(crate) user_agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) accept_language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) user_agent_metadata: Option<Value>,
}

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
