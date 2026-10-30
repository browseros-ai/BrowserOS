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
/// a hover and a drag-and-drop list never sees the drag. An HTML5 drag needs nothing more:
/// once a CDP press has started one, Chromium's DevTools input handler drives it from the
/// following CDP moves and the release (`InputHandler::DragController`), delivering dragenter,
/// dragover and drop as a real drag would.
pub async fn dispatch_drag(
    session: &ProtocolSession,
    from: Point,
    to: Point,
) -> Result<(), CoreError> {
    dispatch_hover(session, from.x, from.y).await?;
    // Armed before the press is sent: a cancellation can land after Chromium applied it.
    let mut held = HeldButton::new(session, from);
    dispatch_held(session, "mousePressed", from).await?;
    let steps = crate::timeouts::DRAG_STEPS;
    for step in 1..=steps {
        let fraction = f64::from(step) / f64::from(steps);
        let point = Point {
            x: from.x + (to.x - from.x) * fraction,
            y: from.y + (to.y - from.y) * fraction,
        };
        held.at = point;
        dispatch_held(session, "mouseMoved", point).await?;
    }
    release_left(session, to).await?;
    held.released = true;
    Ok(())
}

/// Ends a drag that did not finish, however its future ends. An error part-way, a run timeout,
/// or a dialog the gesture opens leaves the left button down and possibly an HTML5 drag in
/// progress, and until something ends them the page treats every later move as part of the
/// drag. The cleanup runs on its own task, so the caller going away cannot interrupt it.
struct HeldButton {
    /// Reaches the page even after its admission is revoked.
    endpoint: ProtocolSession,
    at: Point,
    released: bool,
}

impl HeldButton {
    fn new(session: &ProtocolSession, at: Point) -> Self {
        Self {
            endpoint: session.cancellation_endpoint(),
            at,
            released: false,
        }
    }
}

impl Drop for HeldButton {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        let endpoint = self.endpoint.clone();
        let at = self.at;
        tokio::spawn(async move {
            // Cancel first: the browser process handles it, so a page stalled by a dialog
            // cannot hold it up, and the release that follows lands outside any drag.
            let _ = endpoint
                .send::<_, Value>("Input.cancelDragging", json!({}))
                .await;
            if let Err(error) = release_left(&endpoint, at).await {
                tracing::debug!(%error, "could not release the button of an unfinished drag");
            }
        });
    }
}

async fn release_left(session: &ProtocolSession, at: Point) -> Result<(), CoreError> {
    let _: Value = session
        .send(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseReleased", "x": at.x, "y": at.y, "button": "left", "buttons": 0, "clickCount": 1 }),
        )
        .await?;
    Ok(())
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
