use crate::{
    CoreError, ProtocolSession,
    input::{ClickOptions, Point},
};
use browseros_cdp::CdpEvent;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::broadcast::{
    self,
    error::{RecvError, TryRecvError},
};

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
    let mut events = session.events();
    // Arm cleanup before the enable is sent: a cancellation can land after Chrome applied it.
    let mut drag = DragGuard::new(session);
    let intercepting = session
        .send::<_, Value>("Input.setInterceptDrags", json!({ "enabled": true }))
        .await
        .is_ok();
    drag.intercepting = intercepting;
    let result = drag_gesture(
        session,
        &mut drag,
        from,
        to,
        intercepting.then_some(&mut events),
    )
    .await;
    drag.finish(session).await;
    result
}

async fn drag_gesture(
    session: &ProtocolSession,
    drag: &mut DragGuard,
    from: Point,
    to: Point,
    mut intercepted: Option<&mut broadcast::Receiver<CdpEvent>>,
) -> Result<(), CoreError> {
    dispatch_hover(session, from.x, from.y).await?;
    drag.gesture = Gesture::Held(from);
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
            drag.gesture = Gesture::Dragging(point, data.clone());
            dispatch_drag_event(session, "dragOver", point, data).await?;
            continue;
        }
        drag.gesture = Gesture::Held(point);
        dispatch_held(session, "mouseMoved", point).await?;
        if let Some(events) = intercepted.as_deref_mut() {
            let grace = if step == 1 {
                crate::timeouts::DRAG_START_GRACE
            } else {
                Duration::ZERO
            };
            drag_data = next_intercepted_drag(session, events, grace).await;
            if let Some(data) = &drag_data {
                drag.gesture = Gesture::Dragging(point, data.clone());
                dispatch_drag_event(session, "dragEnter", point, data).await?;
            }
        }
    }
    match drag_data {
        // The drop ends the browser's drag; a release on top of it would also click there.
        Some(data) => dispatch_drag_event(session, "drop", to, &data).await?,
        None => release_left(session, to).await?,
    }
    drag.gesture = Gesture::Ended;
    Ok(())
}

/// How far a drag got, so one that stops early is ended the way it was started.
enum Gesture {
    NotStarted,
    /// The left button is down at this point; the page may be running a script-driven drag.
    Held(Point),
    /// Chromium intercepted a native drag carrying this data; only a drag event ends it.
    Dragging(Point, Value),
    Ended,
}

/// Ends a drag however its future ends. A run timeout, or a dialog the gesture opens, drops
/// the future mid-gesture; without this the page keeps the button held, and with interception
/// still on, Chromium keeps swallowing every later drag in that tab, the user's own included.
struct DragGuard {
    /// Reaches the page even after its admission is revoked; used only by the dropped path.
    endpoint: ProtocolSession,
    gesture: Gesture,
    intercepting: bool,
    finished: bool,
}

impl DragGuard {
    fn new(session: &ProtocolSession) -> Self {
        Self {
            endpoint: session.cancellation_endpoint(),
            gesture: Gesture::NotStarted,
            intercepting: true,
            finished: false,
        }
    }

    /// Releases whatever the gesture still holds and turns interception off. The gesture's own
    /// error, if any, is what the caller reports, so a cleanup failure is not.
    async fn finish(&mut self, session: &ProtocolSession) {
        self.finished = true;
        let gesture = std::mem::replace(&mut self.gesture, Gesture::Ended);
        end_drag(session, gesture, self.intercepting).await;
    }
}

impl Drop for DragGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let gesture = std::mem::replace(&mut self.gesture, Gesture::Ended);
        let endpoint = self.endpoint.clone();
        let intercepting = self.intercepting;
        tokio::spawn(async move { end_drag(&endpoint, gesture, intercepting).await });
    }
}

async fn end_drag(session: &ProtocolSession, gesture: Gesture, intercepting: bool) {
    let released = match gesture {
        Gesture::Held(at) => release_left(session, at).await,
        Gesture::Dragging(at, data) => dispatch_drag_event(session, "dragCancel", at, &data).await,
        Gesture::NotStarted | Gesture::Ended => Ok(()),
    };
    if let Err(error) = released {
        tracing::debug!(%error, "could not end an unfinished drag");
    }
    if intercepting {
        let _ = session
            .send::<_, Value>("Input.setInterceptDrags", json!({ "enabled": false }))
            .await;
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
///
/// The connection carries every page's events, so a drag on another page is skipped here
/// rather than taken. Events lost to a lagging receiver read as no drag, which ends the
/// gesture with a mouse release instead of a drop.
async fn next_intercepted_drag(
    session: &ProtocolSession,
    events: &mut broadcast::Receiver<CdpEvent>,
    grace: Duration,
) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + grace;
    loop {
        let event = match events.try_recv() {
            Ok(event) => event,
            Err(TryRecvError::Lagged(_)) => continue,
            Err(TryRecvError::Empty) if tokio::time::Instant::now() < deadline => {
                match tokio::time::timeout_at(deadline, events.recv()).await {
                    Ok(Ok(event)) => event,
                    Ok(Err(RecvError::Lagged(_))) => continue,
                    Ok(Err(RecvError::Closed)) | Err(_) => return None,
                }
            }
            Err(TryRecvError::Empty | TryRecvError::Closed) => return None,
        };
        if event.method == "Input.dragIntercepted"
            && event.session_id.as_ref() == session.session_id()
        {
            return event.params.get("data").cloned();
        }
    }
}
