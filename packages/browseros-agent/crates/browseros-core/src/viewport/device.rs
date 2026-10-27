//! The two fixed devices, the CDP that puts a tab in one, and how the page proves it took.
use super::{DEFAULT_ZOOM, PageMetrics, Size, ViewportMode};
use crate::{BrowserView, CoreError, PageId, SessionId};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::time::{Instant, sleep};

const SETTLE_LIMIT: Duration = Duration::from_millis(2_000);
const SETTLE_POLL: Duration = Duration::from_millis(25);

const METRICS_EXPRESSION: &str = "(() => ({ width: innerWidth, height: innerHeight, \
    screenWidth: screen.width, screenHeight: screen.height, deviceScaleFactor: devicePixelRatio, \
    touchPoints: navigator.maxTouchPoints, \
    mobile: navigator.userAgentData ? navigator.userAgentData.mobile : null }))()";

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Fixed {
    Mobile { zoom: f64 },
    Desktop,
}

struct Device {
    width: i64,
    height: i64,
    scale_factor: f64,
    mobile: bool,
}

const MOBILE: Device = Device {
    width: 400,
    height: 906,
    scale_factor: 2.0,
    mobile: true,
};
const DESKTOP: Device = Device {
    width: 1920,
    height: 1080,
    scale_factor: 1.0,
    mobile: false,
};

impl Fixed {
    fn device(self) -> &'static Device {
        match self {
            Self::Mobile { .. } => &MOBILE,
            Self::Desktop => &DESKTOP,
        }
    }

    pub(super) fn mode(self) -> ViewportMode {
        match self {
            Self::Mobile { .. } => ViewportMode::Mobile,
            Self::Desktop => ViewportMode::Desktop,
        }
    }

    /// The emulation scale: the preview zoom as a factor.
    pub(super) fn scale(self) -> f64 {
        match self {
            Self::Mobile { zoom } => zoom / DEFAULT_ZOOM,
            Self::Desktop => 1.0,
        }
    }

    /// The on-screen size of the rendered preview, which the window is fitted around.
    pub(super) fn preview(self) -> Size {
        let device = self.device();
        Size {
            width: (device.width as f64 * self.scale()).round() as i64,
            height: (device.height as f64 * self.scale()).round() as i64,
        }
    }

    /// Commands that put the tab in this mode. Desktop restores the browser's own identity.
    pub(super) fn commands(self, chrome_version: &str) -> Vec<(&'static str, Value)> {
        let device = self.device();
        let mut metrics = json!({
            "width": device.width,
            "height": device.height,
            "deviceScaleFactor": device.scale_factor,
            "mobile": device.mobile,
            "scale": self.scale(),
            "screenWidth": device.width,
            "screenHeight": device.height,
        });
        if device.mobile {
            metrics["screenOrientation"] = json!({"type": "portraitPrimary", "angle": 0});
        }
        let (touch, identity) = match self {
            Self::Mobile { .. } => (
                json!({"enabled": true, "maxTouchPoints": 5}),
                mobile_identity(chrome_version),
            ),
            Self::Desktop => (json!({"enabled": false}), json!({"userAgent": ""})),
        };
        vec![
            ("Emulation.setDeviceMetricsOverride", metrics),
            ("Emulation.setTouchEmulationEnabled", touch),
            ("Emulation.setUserAgentOverride", identity),
        ]
    }

    fn reported_by(self, metrics: &PageMetrics) -> bool {
        let device = self.device();
        let screen = metrics.screen_width == device.width as f64
            && metrics.screen_height == device.height as f64
            && metrics.device_scale_factor == device.scale_factor;
        match self {
            Self::Mobile { .. } => {
                screen && metrics.touch_points > 0 && metrics.mobile != Some(false)
            }
            Self::Desktop => screen,
        }
    }
}

/// Undoes exactly what [`Fixed::commands`] set.
pub(super) fn clear_commands() -> Vec<(&'static str, Value)> {
    vec![
        ("Emulation.clearDeviceMetricsOverride", json!({})),
        (
            "Emulation.setTouchEmulationEnabled",
            json!({"enabled": false}),
        ),
        ("Emulation.setUserAgentOverride", json!({"userAgent": ""})),
    ]
}

/// Android Chrome's reduced user agent and client hints for this browser's Chromium version.
fn mobile_identity(chrome_version: &str) -> Value {
    let major = chrome_version.split('.').next().unwrap_or(chrome_version);
    json!({
        "userAgent": format!(
            "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{major}.0.0.0 Mobile Safari/537.36"
        ),
        "userAgentMetadata": {
            "brands": [
                {"brand": "Chromium", "version": major},
                {"brand": "Not/A)Brand", "version": "24"}
            ],
            "fullVersionList": [
                {"brand": "Chromium", "version": chrome_version},
                {"brand": "Not/A)Brand", "version": "24.0.0.0"}
            ],
            "platform": "Android",
            "platformVersion": "10.0.0",
            "architecture": "",
            "model": "K",
            "mobile": true,
            "bitness": "",
            "wow64": false
        }
    })
}

/// The Chromium version from `Browser.getVersion`, e.g. `155.0.8315.26`.
pub(super) fn chrome_version(reply: &Value) -> Option<String> {
    let from = |value: &Value| {
        value
            .as_str()
            .and_then(|text| text.split("Chrome/").nth(1))
            .and_then(|rest| rest.split_whitespace().next())
            .map(ToString::to_string)
    };
    from(&reply["product"]).or_else(|| from(&reply["userAgent"]))
}

pub(super) async fn measure(
    view: &BrowserView,
    session: &SessionId,
) -> Result<PageMetrics, CoreError> {
    let reply = view
        .send_at_capture(
            "Runtime.evaluate",
            json!({"expression": METRICS_EXPRESSION, "returnByValue": true}),
            Some(session),
        )
        .await?;
    serde_json::from_value(reply["result"]["value"].clone())
        .map_err(|err| CoreError::Message(format!("page metrics were unreadable: {err}")))
}

/// Waits until the page reports `fixed`'s device. Emulation reaches the renderer
/// asynchronously, and a page that never reports it did not take it.
pub(super) async fn settle(
    view: &BrowserView,
    session: &SessionId,
    page: &PageId,
    fixed: Fixed,
) -> Result<PageMetrics, CoreError> {
    let deadline = Instant::now() + SETTLE_LIMIT;
    loop {
        let metrics = measure(view, session).await;
        if let Ok(metrics) = &metrics
            && fixed.reported_by(metrics)
        {
            return Ok(metrics.clone());
        }
        if Instant::now() >= deadline {
            let shown = match metrics {
                Ok(metrics) => format!(
                    "it reports {}x{} at device scale {}",
                    metrics.screen_width, metrics.screen_height, metrics.device_scale_factor
                ),
                Err(error) => format!("its metrics could not be read: {error}"),
            };
            let mode = match fixed.mode() {
                ViewportMode::Mobile => "mobile",
                ViewportMode::Desktop | ViewportMode::Default => "desktop",
            };
            return Err(CoreError::Message(format!(
                "page {page} did not take the {mode} viewport; {shown}"
            )));
        }
        sleep(SETTLE_POLL).await;
    }
}
