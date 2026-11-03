//! Shared click transaction for ref and backend-node callers. Preparation can
//! repeat on the resolved node; dispatch ends recovery, regardless of the acknowledgement.

use super::mouse::MouseButton;
use super::{ClickOptions, Input, InputTarget, Point, geometry};
use crate::pointer::PointerAction;
use crate::{CoreError, PageJs, ProtocolSession};
use serde::Deserialize;
use serde_json::json;
use std::{sync::LazyLock, time::Duration};
use tokio::time::Instant;

const RECOVERY_BUDGET: Duration = Duration::from_millis(1500);
const PROBE_INTERVAL: Duration = Duration::from_millis(50);
const ELIGIBILITY_JS: &str = include_str!("../assets/click-eligibility.js");
static PROBE_JS: LazyLock<String> =
    LazyLock::new(|| guarded_script(include_str!("../assets/click-probe.js")));
static DOM_JS: LazyLock<String> =
    LazyLock::new(|| guarded_script(include_str!("../assets/click-dom.js")));

/// Repeat the same eligibility guard inside the final activation call, so page
/// changes during preparation cannot bypass disabled/inert/ARIA-modal checks.
fn guarded_script(body: &str) -> String {
    format!(
        "function(...args){{const refused = ({ELIGIBILITY_JS}).call(this); if(refused) return {{refused}}; return ({body}).apply(this,args);}}"
    )
}

#[derive(Deserialize)]
struct Activation {
    refused: Option<String>,
}

/// Native AX reasons include implicit inertness behind top-layer modal dialogs,
/// even when the dialog lives inside an inaccessible closed shadow root.
#[derive(Deserialize)]
struct AxEligibility {
    nodes: Vec<AxEligibilityNode>,
}

#[derive(Deserialize)]
struct AxEligibilityNode {
    #[serde(rename = "backendDOMNodeId")]
    backend_node_id: Option<i64>,
    #[serde(default, rename = "ignoredReasons")]
    ignored_reasons: Vec<AxProperty>,
    #[serde(default)]
    properties: Vec<AxProperty>,
}

#[derive(Deserialize)]
struct AxProperty {
    name: String,
    #[serde(default)]
    value: serde_json::Value,
}

/// Size of the session viewport, used only to project an OOPIF point into its parent.
#[derive(Debug, Clone, Copy, Deserialize)]
struct Viewport {
    width: f64,
    height: f64,
}

/// One read of geometry and hit testing; a blocker can belong to a parent endpoint.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    #[serde(default)]
    points: Vec<Point>,
    blocker: Option<String>,
    blocked_point: Option<Point>,
    unavailable: Option<String>,
    refused: Option<String>,
    viewport: Option<Viewport>,
    #[serde(skip)]
    blocker_session: Option<ProtocolSession>,
}

impl Input {
    pub(super) async fn click_node(
        &self,
        session: &ProtocolSession,
        target: InputTarget,
        opts: ClickOptions,
    ) -> Result<Option<Point>, CoreError> {
        // The fallback branch surrounds preparation only. Once native dispatch
        // starts, neither a transport failure nor a lost ack can switch methods.
        let prepared = self.prepare_pointer_click(session, &target, &opts).await;
        let (point, at, dispatch_opts) = match prepared {
            Ok(prepared) => prepared,
            Err(
                native @ (CoreError::ElementCovered { .. } | CoreError::ClickPointUnavailable(_)),
            ) => {
                return self
                    .activate_after_preparation_failure(session, &target, &opts, native)
                    .await;
            }
            Err(error) => return Err(error),
        };
        super::mouse::dispatch_click_at_rest(session, at.x, at.y, dispatch_opts, 0).await?;
        Ok(Some(point))
    }

    async fn prepare_pointer_click(
        &self,
        session: &ProtocolSession,
        target: &InputTarget,
        opts: &ClickOptions,
    ) -> Result<(Point, Point, ClickOptions), CoreError> {
        let deadline = tokio::time::Instant::now() + RECOVERY_BUDGET;
        geometry::scroll_into_view(session, target.backend_node_id).await;
        loop {
            let point = self.prepare_click(session, target, deadline).await?;
            if let Some(pointer) = self.drawing_pointer().await
                && pointer.move_to(session, PointerAction::Click, point).await
            {
                if !self.click_point_still_ready(session, target, point).await? {
                    continue;
                }
                pointer.click_effect(session, point).await;
            }
            let at = self.dispatch_point(session, point).await;
            let dispatch_opts = self.dispatch_options(session, opts.clone()).await;
            super::mouse::move_to_click(session, at.x, at.y, &dispatch_opts).await?;
            if !self.click_point_still_ready(session, target, point).await? {
                continue;
            }
            return Ok((point, at, dispatch_opts));
        }
    }

    async fn activate_after_preparation_failure(
        &self,
        session: &ProtocolSession,
        target: &InputTarget,
        opts: &ClickOptions,
        native: CoreError,
    ) -> Result<Option<Point>, CoreError> {
        let refused = if opts
            .button
            .is_some_and(|button| button != MouseButton::Left)
            || opts.click_count.is_some_and(|count| count != 1)
        {
            Some("DOM fallback cannot reproduce a right, middle or repeated click.".to_string())
        } else {
            // Eligibility must be established on the captured endpoint and its
            // admitted parent chain. A failed query never authorizes activation.
            self.dom_fallback_refusal(session, target.backend_node_id)
                .await
                .unwrap_or_else(|error| Some(error.to_string()))
        };
        if let Some(reason) = refused {
            return Err(CoreError::ClickFallbackUnavailable {
                native: Box::new(native),
                reason,
            });
        }
        // This is the sole DOM dispatch. call_once forbids stale-context replay,
        // and no error after this handoff returns to native preparation.
        let activation: Activation = PageJs::isolated(session)
            .call_once(target.backend_node_id, &DOM_JS, &[])
            .await
            .map_err(|error| CoreError::ClickOutcomeUnknown(error.to_string()))?;
        if let Some(reason) = activation.refused {
            return Err(CoreError::ClickFallbackUnavailable {
                native: Box::new(native),
                reason,
            });
        }
        Ok(None)
    }

    async fn dom_fallback_refusal(
        &self,
        session: &ProtocolSession,
        backend_node_id: i64,
    ) -> Result<Option<String>, CoreError> {
        let mut endpoint = session.clone();
        let mut node_id = backend_node_id;
        for _ in 0..16 {
            let ax: AxEligibility = endpoint
                .send(
                    "Accessibility.getPartialAXTree",
                    json!({"backendNodeId": node_id, "fetchRelatives": false}),
                )
                .await?;
            let node = ax
                .nodes
                .iter()
                .find(|node| node.backend_node_id == Some(node_id))
                .ok_or_else(|| {
                    CoreError::Message("Could not establish click eligibility.".into())
                })?;
            if node.ignored_reasons.iter().any(|reason| {
                matches!(
                    reason.name.as_str(),
                    "activeModalDialog" | "activeAriaModalDialog" | "inertElement" | "inertSubtree"
                )
            }) {
                return Ok(Some(
                    "Element is inert or outside the active modal; handle the modal first.".into(),
                ));
            }
            if node.properties.iter().any(|property| {
                property.name == "disabled"
                    && property
                        .value
                        .get("value")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
            }) {
                return Ok(Some("Element is disabled or inert.".into()));
            }
            let refused: Option<String> = PageJs::isolated(&endpoint)
                .call(node_id, ELIGIBILITY_JS, &[])
                .await?;
            if refused.is_some() {
                return Ok(refused);
            }
            let Some((parent, owner)) = self.observer.click_parent(&endpoint).await? else {
                return Ok(None);
            };
            endpoint = parent;
            node_id = owner;
        }
        Ok(Some("Click frame nesting is too deep.".into()))
    }

    /// Wait for two matching observations, within one budget shared with pointer
    /// movement and the final recheck. A ref is never re-resolved during recovery.
    async fn prepare_click(
        &self,
        session: &ProtocolSession,
        target: &InputTarget,
        deadline: Instant,
    ) -> Result<Point, CoreError> {
        let mut previous = None;
        let mut centered = false;
        loop {
            let observed = self.observe_click(session, target, None).await?;
            if let Some(point) = observed.points.first().copied() {
                if previous.is_some_and(|last| same_point(last, point)) {
                    return Ok(point);
                }
                previous = Some(point);
            } else {
                previous = None;
            }
            if Instant::now() >= deadline {
                return Err(self.click_obstruction(session, target, observed).await);
            }
            if !centered && observed.points.is_empty() {
                // The browser's nearest alignment can leave a control behind a
                // sticky header. Center once; never dismiss page UI implicitly.
                let _: serde_json::Value = PageJs::isolated(session)
                    .call(target.backend_node_id,
                        "function(){this.scrollIntoView({block:'center',inline:'center',behavior:'instant'})}", &[])
                    .await?;
                centered = true;
            }
            tokio::time::sleep_until((Instant::now() + PROBE_INTERVAL).min(deadline)).await;
        }
    }

    /// Recheck the exact destination after pointer movement. A nearby newly
    /// exposed sample does not prove this coordinate still reaches the target.
    async fn click_point_still_ready(
        &self,
        session: &ProtocolSession,
        target: &InputTarget,
        point: Point,
    ) -> Result<bool, CoreError> {
        let observed = self.observe_click(session, target, Some(point)).await?;
        Ok(observed
            .points
            .into_iter()
            .any(|now| same_point(now, point)))
    }

    async fn observe_click(
        &self,
        session: &ProtocolSession,
        target: &InputTarget,
        at: Option<Point>,
    ) -> Result<Observation, CoreError> {
        let args = match at {
            Some(point) => vec![json!([]), json!([point])],
            None => vec![json!(
                geometry::get_element_quads(session, target.backend_node_id).await?
            )],
        };
        let mut observed: Observation = PageJs::isolated(session)
            .call(target.backend_node_id, &PROBE_JS, &args)
            .await?;
        if let Some(reason) = &observed.refused {
            return Err(CoreError::Message(reason.clone()));
        }
        if let Some(reason) = &observed.unavailable {
            return Err(CoreError::ClickPointUnavailable(reason.clone()));
        }
        let mut endpoint = session.clone();
        let mut viewport = observed.viewport;
        let mut mapped = observed.points.clone();
        // Input sent to an OOPIF uses its own coordinates, but an overlay in any
        // parent can still intercept it. Check every parent before declaring a
        // point usable, retaining its original coordinate for dispatch.
        for _ in 0..16 {
            if mapped.is_empty() {
                return Ok(observed);
            }
            let Some((parent, owner)) = self.observer.click_parent(&endpoint).await? else {
                return Ok(observed);
            };
            let size = viewport
                .filter(|size| size.width > 0.0 && size.height > 0.0)
                .ok_or_else(|| {
                    CoreError::ClickPointUnavailable("Could not read click frame viewport.".into())
                })?;
            let quads = geometry::get_element_quads(&parent, owner).await?;
            let quad = quads
                .first()
                .filter(|quad| quad.len() == 4)
                .ok_or_else(|| {
                    CoreError::ClickPointUnavailable("Could not read click frame geometry.".into())
                })?;
            if (quad[2].x - quad[1].x - quad[3].x + quad[0].x).abs() > 0.5
                || (quad[2].y - quad[1].y - quad[3].y + quad[0].y).abs() > 0.5
            {
                return Err(CoreError::ClickPointUnavailable(
                    "Perspective-transformed click frames are not supported. No click was sent."
                        .into(),
                ));
            }
            let projected: Vec<Point> = mapped
                .iter()
                .map(|point| Point {
                    x: quad[0].x
                        + (quad[1].x - quad[0].x) * point.x / size.width
                        + (quad[3].x - quad[0].x) * point.y / size.height,
                    y: quad[0].y
                        + (quad[1].y - quad[0].y) * point.x / size.width
                        + (quad[3].y - quad[0].y) * point.y / size.height,
                })
                .collect();
            let visibility: Observation = PageJs::isolated(&parent)
                .call(owner, &PROBE_JS, &[json!([]), json!(projected)])
                .await?;
            if let Some(reason) = visibility.refused {
                return Err(CoreError::Message(reason));
            }
            if let Some(reason) = visibility.unavailable {
                return Err(CoreError::ClickPointUnavailable(reason));
            }
            let keep: Vec<_> = projected
                .iter()
                .enumerate()
                .filter_map(|(i, point)| {
                    visibility
                        .points
                        .iter()
                        .any(|clear| same_point(*clear, *point))
                        .then_some(i)
                })
                .collect();
            observed.points = keep.iter().map(|&i| observed.points[i]).collect();
            mapped = keep.iter().map(|&i| projected[i]).collect();
            if observed.points.is_empty() {
                observed.blocker = visibility.blocker;
                observed.blocked_point = visibility.blocked_point;
                observed.blocker_session = Some(parent);
                return Ok(observed);
            }
            viewport = visibility.viewport;
            endpoint = parent;
        }
        Err(CoreError::ClickPointUnavailable(
            "Click frame nesting is too deep. No click was sent.".into(),
        ))
    }

    async fn click_obstruction(
        &self,
        session: &ProtocolSession,
        target: &InputTarget,
        observed: Observation,
    ) -> CoreError {
        if let Some(blocker) = observed.blocker {
            let blocker_ref = match observed.blocked_point {
                Some(point) => self
                    .observer
                    .reference_at_point(observed.blocker_session.as_ref().unwrap_or(session), point)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            };
            CoreError::ElementCovered {
                target: Box::new(target.error_target.clone()),
                blocker,
                blocker_ref,
            }
        } else {
            CoreError::ClickPointUnavailable("Element has no stable, uncovered click point; take a new snapshot. No click was sent.".to_string())
        }
    }
}

fn same_point(a: Point, b: Point) -> bool {
    (a.x - b.x).abs() < 0.5 && (a.y - b.y).abs() < 0.5
}
