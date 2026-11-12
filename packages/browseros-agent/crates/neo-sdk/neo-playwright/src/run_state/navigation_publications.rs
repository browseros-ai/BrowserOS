//! Run-local navigation publication readiness. A load observed in the journal can finish a
//! navigation's required browser work before its task completes optional bounded metadata.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use browseros_core::FrameId;

use crate::events::{EventHub, PageEvent, PageEventKind};
use crate::page::LoadState;

/// Loader-correlated main-document waits owned by a run and released by client acknowledgments.
#[derive(Debug, Default)]
pub(crate) struct NavigationPublications(Mutex<HashMap<u64, AwaitedDocument>>);

/// The requested loader and call-time journal fence prevent stale or replaced documents from lending readiness.
#[derive(Debug)]
struct AwaitedDocument {
    hub: Arc<EventHub>,
    frame: FrameId,
    loader: String,
    state: LoadState,
    after: u64,
}

impl NavigationPublications {
    pub(crate) fn acknowledge(&self, ids: &[u64]) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|id, _| !ids.contains(id));
    }

    /// Only main-frame load events are public journal facts. Child and network-idle waits
    /// retain the stricter native-reply receipt path instead of borrowing another state.
    pub(crate) fn watch(
        &self,
        id: u64,
        hub: Arc<EventHub>,
        frame: FrameId,
        loader: String,
        state: LoadState,
        after: u64,
    ) {
        if state == LoadState::NetworkIdle {
            return;
        }
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                id,
                AwaitedDocument {
                    hub,
                    frame,
                    loader,
                    state,
                    after,
                },
            );
    }

    pub(crate) fn ready(&self) -> Vec<u64> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter_map(|(id, document)| document.ready().then_some(*id))
            .collect()
    }
}

impl AwaitedDocument {
    fn ready(&self) -> bool {
        let read = self.hub.read_after(self.after);
        if read.lost > 0 {
            return false;
        }
        let mut current = None;
        let mut reached = self.state == LoadState::Commit;
        for record in read.records {
            match record.event {
                PageEvent::FrameLifecycle(event)
                    if event.frame.id == self.frame
                        && event.kind == PageEventKind::FrameNavigated =>
                {
                    current = event.frame.loader_id;
                }
                PageEvent::Load { kind, frame, .. }
                    if frame.id == self.frame
                        && frame.loader_id.as_deref() == Some(&self.loader) =>
                {
                    reached |= match self.state {
                        LoadState::Load => kind == PageEventKind::Load,
                        LoadState::DomContentLoaded => {
                            matches!(kind, PageEventKind::DomContentLoaded | PageEventKind::Load)
                        }
                        LoadState::Commit => true,
                        LoadState::NetworkIdle => false,
                    };
                }
                PageEvent::Closed(target) if target.as_str() == self.frame.as_str() => {
                    return false;
                }
                _ => {}
            }
        }
        // A newer committed document cannot lend readiness to this producer. Pending
        // replacements are handled by the navigation's existing bounded metadata race.
        reached && current.as_deref() == Some(&self.loader)
    }
}
