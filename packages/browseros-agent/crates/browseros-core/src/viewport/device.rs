//! The fixed devices, the CDP that puts a tab in one, and how the page proves it took.
use super::{DEFAULT_ZOOM, PageMetrics, Size, ViewportMode};
use crate::{CoreError, PageId, PageJs, ProtocolSession};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::time::{Instant, sleep};

const SETTLE_LIMIT: Duration = Duration::from_millis(2_000);
const SETTLE_POLL: Duration = Duration::from_millis(25);

const METRICS_EXPRESSION: &str = "(() => ({ width: innerWidth, height: innerHeight, \
    screenWidth: screen.width, screenHeight: screen.height, deviceScaleFactor: devicePixelRatio, \
    touchPoints: navigator.maxTouchPoints, \
    mobile: navigator.userAgentData ? navigator.userAgentData.mobile : null }))()";

/// The sizes each fixed mode uses unless a request names its own.
pub(super) const MOBILE_SIZE: Size = Size {
    width: 400,
    height: 906,
};
pub(super) const DESKTOP_SIZE: Size = Size {
    width: 1920,
    height: 1080,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Fixed {
    Mobile { zoom: f64, size: Size },
    Desktop { size: Size },
}

impl Fixed {
    /// The device viewport in CSS px.
    pub(super) fn size(self) -> Size {
        match self {
            Self::Mobile { size, .. } | Self::Desktop { size } => size,
        }
    }

    fn mobile(self) -> bool {
        matches!(self, Self::Mobile { .. })
    }

    fn scale_factor(self) -> f64 {
        if self.mobile() { 2.0 } else { 1.0 }
    }

    pub(super) fn mode(self) -> ViewportMode {
        match self {
            Self::Mobile { .. } => ViewportMode::Mobile,
            Self::Desktop { .. } => ViewportMode::Desktop,
        }
    }

    /// The emulation scale: the preview zoom as a factor.
    pub(super) fn scale(self) -> f64 {
        match self {
            Self::Mobile { zoom, .. } => zoom / DEFAULT_ZOOM,
            Self::Desktop { .. } => 1.0,
        }
    }

    /// The on-screen size of the rendered preview, which the tab's view is pinned to.
    pub(super) fn preview(self) -> Size {
        let size = self.size();
        Size {
            width: (size.width as f64 * self.scale()).round() as i64,
            height: (size.height as f64 * self.scale()).round() as i64,
        }
    }

    /// Commands that put the tab in this mode. Desktop restores the browser's own identity.
    pub(super) fn commands(self, chrome_version: &str) -> Vec<(&'static str, Value)> {
        let size = self.size();
        let mut metrics = json!({
            "width": size.width,
            "height": size.height,
            "deviceScaleFactor": self.scale_factor(),
            "mobile": self.mobile(),
            "scale": self.scale(),
            "screenWidth": size.width,
            "screenHeight": size.height,
            // Otherwise a hidden tab's view shrinks to the unscaled device size, and zoomed
            // wheel input aimed beyond it misses the scroller under the pointer.
            "dontSetVisibleSize": true,
        });
        if self.mobile() {
            metrics["screenOrientation"] = json!({"type": "portraitPrimary", "angle": 0});
        }
        let (touch, identity) = match self {
            Self::Mobile { .. } => (
                json!({"enabled": true, "maxTouchPoints": 5}),
                mobile_identity(chrome_version),
            ),
            Self::Desktop { .. } => (json!({"enabled": false}), json!({"userAgent": ""})),
        };
        vec![
            ("Emulation.setDeviceMetricsOverride", metrics),
            ("Emulation.setTouchEmulationEnabled", touch),
            ("Emulation.setUserAgentOverride", identity),
        ]
    }

    /// Sizes the tab's view to the preview. A hidden tab's view otherwise keeps whatever size
    /// an earlier override or capture left, and zoomed input beyond it misses.
    pub(super) fn pin(self) -> (&'static str, Value) {
        let preview = self.preview();
        (
            "Emulation.setVisibleSize",
            json!({"width": preview.width, "height": preview.height}),
        )
    }

    fn reported_by(self, metrics: &PageMetrics) -> bool {
        let size = self.size();
        let screen = metrics.screen_width == size.width as f64
            && metrics.screen_height == size.height as f64
            && metrics.device_scale_factor == self.scale_factor();
        match self {
            Self::Mobile { .. } => {
                screen && metrics.touch_points > 0 && metrics.mobile != Some(false)
            }
            Self::Desktop { .. } => screen,
        }
    }
}

/// Whether a raw protocol call replaces something a fixed mode set on the page session.
pub(super) fn overrides(method: &str) -> bool {
    matches!(
        method,
        "Emulation.setDeviceMetricsOverride"
            | "Emulation.clearDeviceMetricsOverride"
            | "Emulation.setTouchEmulationEnabled"
            | "Emulation.setUserAgentOverride"
            | "Emulation.setVisibleSize"
    )
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

/// What Google Chrome on an Android phone sends for this browser's Chromium version: the reduced
/// user agent and its client hints. The device's own model and Android version are high-entropy
/// hints, so they stay plausible rather than the frozen "K" and "10" of the user agent.
fn mobile_identity(chrome_version: &str) -> Value {
    let major = major_version(chrome_version).unwrap_or_default();
    json!({
        "userAgent": format!(
            "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{major}.0.0.0 Mobile Safari/537.36"
        ),
        // navigator.platform: Chromium's reduced value on Android phones ("armv81" as written
        // in navigator_base.cc), never the host's.
        "platform": "Linux armv81",
        "userAgentMetadata": {
            "brands": chrome_brands(major, None),
            "fullVersionList": chrome_brands(major, Some(chrome_version)),
            "fullVersion": chrome_version,
            "platform": "Android",
            "platformVersion": "14.0.0",
            "architecture": "",
            "model": "Pixel 7",
            "mobile": true,
            "bitness": "",
            "wow64": false,
            "formFactors": ["Mobile"]
        }
    })
}

/// Chrome's own brand list (Chromium's `GenerateBrandVersionList`): a GREASE brand, Chromium and
/// Google Chrome, named, versioned and ordered by values seeded with the major version.
fn chrome_brands(major: u64, full_version: Option<&str>) -> Value {
    const GREASE_CHARS: [&str; 11] = [" ", "(", ":", "-", ".", "/", ")", ";", "=", "?", "_"];
    const GREASE_VERSIONS: [&str; 3] = ["8", "99", "24"];
    const ORDERS: [[usize; 3]; 6] = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let seed = usize::try_from(major).unwrap_or_default();
    let grease = format!(
        "Not{}A{}Brand",
        GREASE_CHARS[seed % GREASE_CHARS.len()],
        GREASE_CHARS[(seed + 1) % GREASE_CHARS.len()]
    );
    let grease_version = GREASE_VERSIONS[seed % GREASE_VERSIONS.len()];
    let (grease_version, chrome) = match full_version {
        Some(full) => (format!("{grease_version}.0.0.0"), full.to_string()),
        None => (grease_version.to_string(), major.to_string()),
    };
    let generated = [
        (grease, grease_version),
        ("Chromium".to_string(), chrome.clone()),
        ("Google Chrome".to_string(), chrome),
    ];
    let order = ORDERS[seed % ORDERS.len()];
    let mut brands = vec![Value::Null; generated.len()];
    for (index, (brand, version)) in generated.into_iter().enumerate() {
        brands[order[index]] = json!({"brand": brand, "version": version});
    }
    Value::Array(brands)
}

fn major_version(chrome_version: &str) -> Option<u64> {
    chrome_version.split('.').next()?.parse().ok()
}

/// The Chromium version from `Browser.getVersion`, e.g. `155.0.8315.26`.
pub(super) fn chrome_version(reply: &Value) -> Option<String> {
    let from = |value: &Value| {
        value
            .as_str()
            .and_then(|text| text.split("Chrome/").nth(1))
            .and_then(|rest| rest.split_whitespace().next())
            .filter(|version| major_version(version).is_some())
            .map(ToString::to_string)
    };
    from(&reply["product"]).or_else(|| from(&reply["userAgent"]))
}

/// Reads the metrics in an isolated world: page scripts may redefine `screen` or
/// `devicePixelRatio` in their own world, which must neither fake nor fail this check.
pub(super) async fn measure(session: &ProtocolSession) -> Result<PageMetrics, CoreError> {
    PageJs::isolated_only(session)
        .eval(METRICS_EXPRESSION)
        .await
        .map_err(|err| CoreError::Message(format!("page metrics were unreadable: {err}")))
}

/// Waits until the page reports `fixed`'s device. Emulation reaches the renderer
/// asynchronously, and a page that never reports it did not take it.
pub(super) async fn settle(
    session: &ProtocolSession,
    page: &PageId,
    fixed: Fixed,
) -> Result<PageMetrics, CoreError> {
    let deadline = Instant::now() + SETTLE_LIMIT;
    loop {
        let metrics = measure(session).await;
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
