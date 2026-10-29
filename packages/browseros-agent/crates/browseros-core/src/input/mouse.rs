use crate::{
    CoreError, ProtocolSession,
    input::{ClickOptions, Point},
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

/// Presses at `from`, moves to `to` in steps with the left button held, and releases.
///
/// Blink starts a drag only from a move that reports the left button down
/// (`MouseEventManager::HandleMouseDraggedEvent`), so every move carries it; a bare move is
/// a hover and a drag-and-drop list never sees the drag. An element using HTML drag-and-drop
/// is different again: once its drag starts, the browser hands it to the OS drag loop, which
/// synthetic mouse events cannot steer. With drag interception on, Chromium reports that
/// drag as `Input.dragIntercepted` instead, and the rest of the gesture is replayed as drag
/// events (`dragEnter`, `dragOver`, `drop`), the way Playwright drives it.
pub async fn dispatch_drag(
    session: &ProtocolSession,
    from: Point,
    to: Point,
) -> Result<(), CoreError> {
    // Subscribe before enabling interception so an intercepted drag cannot be missed.
    let mut intercepted = session.events_targeted("Input.dragIntercepted");
    let intercepting = session
        .send::<_, Value>("Input.setInterceptDrags", json!({ "enabled": true }))
        .await
        .is_ok();
    let result = drag_gesture(session, from, to, intercepting.then_some(&mut intercepted)).await;
    if intercepting {
        let _ = session
            .send::<_, Value>("Input.setInterceptDrags", json!({ "enabled": false }))
            .await;
    }
    result
}

async fn drag_gesture(
    session: &ProtocolSession,
    from: Point,
    to: Point,
    mut intercepted: Option<&mut tokio::sync::mpsc::UnboundedReceiver<browseros_cdp::CdpEvent>>,
) -> Result<(), CoreError> {
    dispatch_hover(session, from.x, from.y).await?;
    dispatch_held(session, "mousePressed", from).await?;
    let mut drag_data: Option<Value> = None;
    let steps = crate::timeouts::DRAG_STEPS;
    for step in 1..=steps {
        let fraction = f64::from(step) / f64::from(steps);
        let point = Point {
            x: from.x + (to.x - from.x) * fraction,
            y: from.y + (to.y - from.y) * fraction,
        };
        if let Some(data) = &drag_data {
            dispatch_drag_event(session, "dragOver", point, data).await?;
            continue;
        }
        dispatch_held(session, "mouseMoved", point).await?;
        if let Some(events) = intercepted.as_deref_mut() {
            let grace = if step == 1 {
                crate::timeouts::DRAG_START_GRACE
            } else {
                Duration::ZERO
            };
            drag_data = next_intercepted_drag(session, events, grace).await;
            if let Some(data) = &drag_data {
                dispatch_drag_event(session, "dragEnter", point, data).await?;
            }
        }
    }
    match drag_data {
        // The drop ends the browser's drag; a release on top of it would also click there.
        Some(data) => dispatch_drag_event(session, "drop", to, &data).await,
        None => {
            let _: Value = session
                .send(
                    "Input.dispatchMouseEvent",
                    json!({ "type": "mouseReleased", "x": to.x, "y": to.y, "button": "left", "buttons": 0, "clickCount": 1 }),
                )
                .await?;
            Ok(())
        }
    }
}

/// A mouse event with the left button held, which is what makes a move part of a drag.
async fn dispatch_held(
    session: &ProtocolSession,
    kind: &str,
    point: Point,
) -> Result<(), CoreError> {
    let mut params =
        json!({ "type": kind, "x": point.x, "y": point.y, "button": "left", "buttons": 1 });
    if kind == "mousePressed" {
        params["clickCount"] = json!(1);
    }
    let _: Value = session.send("Input.dispatchMouseEvent", params).await?;
    Ok(())
}

async fn dispatch_drag_event(
    session: &ProtocolSession,
    kind: &str,
    point: Point,
    data: &Value,
) -> Result<(), CoreError> {
    let _: Value = session
        .send(
            "Input.dispatchDragEvent",
            json!({ "type": kind, "x": point.x, "y": point.y, "data": data }),
        )
        .await?;
    Ok(())
}

/// The drag data of a native drag this session started, waiting up to `grace` for it.
async fn next_intercepted_drag(
    session: &ProtocolSession,
    events: &mut tokio::sync::mpsc::UnboundedReceiver<browseros_cdp::CdpEvent>,
    grace: Duration,
) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + grace;
    loop {
        let event = match events.try_recv() {
            Ok(event) => event,
            Err(_empty) if tokio::time::Instant::now() < deadline => {
                tokio::time::timeout_at(deadline, events.recv())
                    .await
                    .ok()??
            }
            Err(_empty) => return None,
        };
        if event.session_id.as_ref() == session.session_id() {
            return event.params.get("data").cloned();
        }
    }
}
