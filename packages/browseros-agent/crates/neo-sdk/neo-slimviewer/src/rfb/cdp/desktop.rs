//! Pixel wheel input for the focused browser page; chrome and unavailable CDP use RFB.

use super::{BUDGET, CDP_PORT, Client, DeliveryError, Instant, json};

const GEOMETRY: &str =
    "[screenX,screenY,outerWidth,outerHeight,innerWidth,innerHeight,devicePixelRatio]";

pub(in crate::rfb) fn wheel(
    x: u16,
    y: u16,
    dx: f32,
    dy: f32,
    modifiers: u8,
    buttons: u8,
) -> Result<(), DeliveryError> {
    let mut client =
        Client::connect(CDP_PORT, Instant::now() + BUDGET).map_err(DeliveryError::NotSent)?;
    let page = client.focused_page().map_err(DeliveryError::NotSent)?;
    let result = client.call("Runtime.evaluate", json!({
        "expression": GEOMETRY, "contextId":page.context, "returnByValue":true, "timeout":100,
    }), Some(&page.session)).map_err(DeliveryError::NotSent)?;
    let geometry = result["result"]["value"]
        .as_array()
        .and_then(|v| v.iter().map(|n| n.as_f64()).collect::<Option<Vec<_>>>())
        .and_then(|v| <[f64; 7]>::try_from(v).ok())
        .ok_or(DeliveryError::NotSent("invalid viewport geometry"))?;
    let layout = client
        .call("Page.getLayoutMetrics", json!({}), Some(&page.session))
        .map_err(DeliveryError::NotSent)?;
    let zoom = layout["cssVisualViewport"]["zoom"].as_f64().unwrap_or(1.0);
    let (px, py, ratio) = viewport_point(geometry, zoom, x, y)
        .ok_or(DeliveryError::NotSent("pointer outside browser content"))?;
    if !client
        .still_selected(&page)
        .map_err(DeliveryError::NotSent)?
    {
        return Err(DeliveryError::NotSent("focus changed"));
    }
    client
        .call(
            "Input.dispatchMouseEvent",
            json!({
                "type":"mouseWheel", "x":px, "y":py,
                "deltaX":f64::from(dx)/ratio, "deltaY":f64::from(dy)/ratio,
                "modifiers":modifiers, "buttons":buttons,
            }),
            Some(&page.session),
        )
        .map_err(|_| DeliveryError::Uncertain)?;
    Ok(())
}

fn viewport_point(g: [f64; 7], zoom: f64, x: u16, y: u16) -> Option<(f64, f64, f64)> {
    let [sx, sy, ow, oh, iw, ih, ratio] = g;
    if !g.into_iter().all(f64::is_finite)
        || !zoom.is_finite()
        || zoom <= 0.0
        || ratio <= 0.0
        || iw <= 0.0
        || ih <= 0.0
    {
        return None;
    }
    // outer/screen dimensions are desktop DIPs; inner dimensions include page zoom.
    let scale = ratio / zoom;
    let border = ((ow - iw * zoom) / 2.0).max(0.0);
    let top = (oh - ih * zoom - border).max(0.0);
    let px = (f64::from(x) / scale - sx - border) / zoom;
    let py = (f64::from(y) / scale - sy - top) / zoom;
    (px >= 0.0 && py >= 0.0 && px < iw && py < ih).then_some((px, py, ratio))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_pixels_map_to_content_including_window_borders_zoom_and_dpi() {
        assert_eq!(
            viewport_point(
                [0.0, 0.0, 1920.0, 1080.0, 1920.0, 993.0, 1.0],
                1.0,
                400,
                287
            ),
            Some((400.0, 200.0, 1.0))
        );
        assert_eq!(
            viewport_point(
                [100.0, 50.0, 1002.0, 702.0, 1000.0, 600.0, 1.0],
                1.0,
                201,
                251
            ),
            Some((100.0, 100.0, 1.0))
        );
        assert_eq!(
            viewport_point([0.0, 0.0, 1920.0, 1080.0, 960.0, 496.5, 2.0], 2.0, 400, 287),
            Some((200.0, 100.0, 2.0))
        );
        assert_eq!(
            viewport_point([0.0, 0.0, 960.0, 540.0, 960.0, 453.0, 2.0], 1.0, 400, 374),
            Some((200.0, 100.0, 2.0))
        );
    }

    #[test]
    fn browser_chrome_and_invalid_geometry_are_not_sent_to_a_page() {
        let g = [0.0, 0.0, 1920.0, 1080.0, 1920.0, 993.0, 1.0];
        assert!(viewport_point(g, 1.0, 100, 50).is_none());
        assert!(viewport_point(g, 1.0, 2000, 300).is_none());
        assert!(viewport_point(g, f64::NAN, 100, 300).is_none());
        assert!(viewport_point(g, 0.0, 100, 300).is_none());
    }
}
