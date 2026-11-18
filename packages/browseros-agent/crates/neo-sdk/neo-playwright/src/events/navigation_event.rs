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
    /// The selected document, for a timing read that must never return a later response.
    pub(crate) loader: Option<String>,
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
    loader_id: String,
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
                loader: Some(frame.loader_id),
                unreachable: frame.unreachable_url,
            }))
        }
        "Page.navigatedWithinDocument" => {
            let navigation: SameDocument = serde_json::from_value(params).ok()?;
            Some(PageEvent::Navigation(NavigationEvent {
                frame: navigation.frame_id,
                url: navigation.url,
                new_document: false,
                loader: None,
                unreachable: None,
            }))
        }
        "Page.frameDetached" => {
            let detached: Detached = serde_json::from_value(params).ok()?;
            Some(if detached.reason == "swap" {
                PageEvent::FrameSwapped(detached.frame_id)
            } else {
                PageEvent::FrameDetached(detached.frame_id)
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use browseros_core::FrameId;
    use serde_json::json;
    use std::error::Error;

    use super::parse;
    use crate::events::PageEvent;

    #[test]
    fn tests_that_renderer_swaps_preserve_a_wait_but_removal_ends_it() {
        assert_eq!(
            parse(
                "Page.frameDetached",
                json!({"frameId":"child", "reason":"swap"})
            ),
            Some(PageEvent::FrameSwapped(FrameId("child".to_owned())))
        );
        assert_eq!(
            parse(
                "Page.frameDetached",
                json!({"frameId":"child", "reason":"remove"})
            ),
            Some(PageEvent::FrameDetached(FrameId("child".to_owned())))
        );
    }

    #[test]
    fn tests_that_commit_fragments_and_history_moves_keep_their_response_kind()
    -> Result<(), Box<dyn Error>> {
        let Some(PageEvent::Navigation(committed)) = parse(
            "Page.frameNavigated",
            json!({
                "frame": {"id":"main", "loaderId":"L1", "url":"https://a.test/", "urlFragment":"#part"}
            }),
        ) else {
            return Err("a commit must be journaled".into());
        };
        let Some(PageEvent::Navigation(history)) = parse(
            "Page.navigatedWithinDocument",
            json!({
                "frameId":"main", "url":"https://a.test/#other"
            }),
        ) else {
            return Err("a history move must be journaled".into());
        };
        assert_eq!(committed.url, "https://a.test/#part");
        assert!(
            committed.new_document,
            "a committed document can have a response"
        );
        assert!(!history.new_document, "a history move must return null");
        Ok(())
    }
}
