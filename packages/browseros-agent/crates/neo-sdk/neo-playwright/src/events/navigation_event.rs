//! Internal navigation facts in the shared event journal. They keep a wait's cursor alive
//! across agent-side URL predicates without adding public frame or network events.

use browseros_core::FrameId;
use serde::Deserialize;
use serde_json::Value;

use crate::events::page_event::PageEvent;

/// One frame's committed navigation, including moves inside its existing document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NavigationEvent {
    pub(crate) frame: FrameId,
    pub(crate) url: String,
    pub(crate) new_document: bool,
    pub(crate) unreachable: Option<String>,
}

#[derive(Deserialize)]
struct Committed {
    frame: Document,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Document {
    id: FrameId,
    url: String,
    #[serde(default)]
    url_fragment: String,
    #[serde(default)]
    unreachable_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SameDocument {
    frame_id: FrameId,
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Detached {
    frame_id: FrameId,
    reason: String,
}

/// Decode only the Page-domain facts a navigation waiter needs. Renderer swaps retain the
/// frame's identity; only an actual removal ends its wait.
pub(crate) fn parse(method: &str, params: Value) -> Option<PageEvent> {
    match method {
        "Page.frameNavigated" => {
            let Committed { frame } = serde_json::from_value(params).ok()?;
            Some(PageEvent::Navigation(NavigationEvent {
                frame: frame.id,
                url: format!("{}{}", frame.url, frame.url_fragment),
                new_document: true,
                unreachable: frame.unreachable_url,
            }))
        }
        "Page.navigatedWithinDocument" => {
            let navigation: SameDocument = serde_json::from_value(params).ok()?;
            Some(PageEvent::Navigation(NavigationEvent {
                frame: navigation.frame_id,
                url: navigation.url,
                new_document: false,
                unreachable: None,
            }))
        }
        "Page.frameDetached" => {
            let detached: Detached = serde_json::from_value(params).ok()?;
            (detached.reason != "swap").then_some(PageEvent::FrameDetached(detached.frame_id))
        }
        _ => None,
    }
}
