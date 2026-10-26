//! The only place that sends CDP mouse events. Callers pass CSS px; on a page session in
//! mobile mode the coordinates go out in its zoomed view space. Wheel deltas stay as they
//! are: Blink divides the position by the zoom but not the delta (measured live).
use crate::{
    CoreError, ProtocolSession,
    input::{ClickOptions, Point},
    mobile_mode::input_scale,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    #[default]
    Left,
    Middle,
    Right,
}

impl MouseButton {
    fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Middle => "middle",
            Self::Right => "right",
        }
    }
}

pub async fn dispatch_click(
    session: &ProtocolSession,
    x: f64,
    y: f64,
    button: MouseButton,
    click_count: i64,
    modifiers: i64,
) -> Result<(), CoreError> {
    dispatch_click_with_options(
        session,
        x,
        y,
        ClickOptions {
            button: Some(button),
            click_count: Some(click_count),
            ..Default::default()
        },
        modifiers,
    )
    .await
}

pub async fn dispatch_click_with_options(
    session: &ProtocolSession,
    x: f64,
    y: f64,
    opts: ClickOptions,
    modifiers: i64,
) -> Result<(), CoreError> {
    if let Some(from) = opts.humanlike_from {
        for step in 1..=4 {
            let fraction = f64::from(step) / 4.0;
            dispatch_hover(
                session,
                from.x + (x - from.x) * fraction,
                from.y + (y - from.y) * fraction,
            )
            .await?;
            tokio::time::sleep(Duration::from_millis(12)).await;
        }
    } else {
        dispatch_hover(session, x, y).await?;
    }
    let button = opts.button.unwrap_or(MouseButton::Left);
    let click_count = opts.click_count.unwrap_or(1);
    let scale = input_scale(session);
    let (x, y) = (x * scale, y * scale);
    let _: Value = session
        .send(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mousePressed",
                "x": x,
                "y": y,
                "button": button.as_str(),
                "clickCount": click_count,
                "modifiers": modifiers
            }),
        )
        .await?;
    if opts.humanlike_from.is_some() {
        tokio::time::sleep(Duration::from_millis(90)).await;
    }
    let _: Value = session
        .send(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseReleased",
                "x": x,
                "y": y,
                "button": button.as_str(),
                "clickCount": click_count,
                "modifiers": modifiers
            }),
        )
        .await?;
    Ok(())
}

pub async fn dispatch_hover(session: &ProtocolSession, x: f64, y: f64) -> Result<(), CoreError> {
    let scale = input_scale(session);
    let _: Value = session
        .send(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseMoved", "x": x * scale, "y": y * scale }),
        )
        .await?;
    Ok(())
}

pub async fn dispatch_scroll(
    session: &ProtocolSession,
    x: f64,
    y: f64,
    delta_x: f64,
    delta_y: f64,
) -> Result<(), CoreError> {
    let scale = input_scale(session);
    let _: Value = session
        .send(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseWheel",
                "x": x * scale,
                "y": y * scale,
                "deltaX": delta_x,
                "deltaY": delta_y
            }),
        )
        .await?;
    Ok(())
}

pub async fn dispatch_drag(
    session: &ProtocolSession,
    from: Point,
    to: Point,
) -> Result<(), CoreError> {
    let scale = input_scale(session);
    let from = Point {
        x: from.x * scale,
        y: from.y * scale,
    };
    let to = Point {
        x: to.x * scale,
        y: to.y * scale,
    };
    let _: Value = session
        .send(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseMoved", "x": from.x, "y": from.y }),
        )
        .await?;
    let _: Value = session
        .send(
            "Input.dispatchMouseEvent",
            json!({ "type": "mousePressed", "x": from.x, "y": from.y, "button": "left", "clickCount": 1 }),
        )
        .await?;
    let _: Value = session
        .send(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseMoved", "x": to.x, "y": to.y }),
        )
        .await?;
    let _: Value = session
        .send(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseReleased", "x": to.x, "y": to.y, "button": "left", "clickCount": 1 }),
        )
        .await?;
    Ok(())
}
