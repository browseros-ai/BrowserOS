use crate::{CoreError, ProtocolSession, input::Point};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
    let _: Value = session
        .send(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseMoved", "x": x, "y": y }),
        )
        .await?;
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
    let _: Value = session
        .send(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseMoved", "x": x, "y": y }),
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
    let _: Value = session
        .send(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseWheel", "x": x, "y": y, "deltaX": delta_x, "deltaY": delta_y }),
        )
        .await?;
    Ok(())
}

pub async fn dispatch_drag(
    session: &ProtocolSession,
    from: Point,
    to: Point,
) -> Result<(), CoreError> {
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
