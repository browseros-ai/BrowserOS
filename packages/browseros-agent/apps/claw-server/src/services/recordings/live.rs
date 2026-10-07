//! In-process fan-out of freshly ingested rrweb batches to live-preview
//! subscribers, keyed by recording document id. Publishing is best-effort and
//! never blocks ingest; a subscriber that falls behind is dropped and expected
//! to reconnect (which re-bootstraps from stored events).

use super::RecordingEventInput;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, broadcast};

/// Per-document broadcast depth. A subscriber more than this many batches behind
/// receives `Lagged` and is closed so it reconnects rather than replay stale
/// frames.
const LIVE_CHANNEL_CAPACITY: usize = 256;

/// One freshly accepted batch of rrweb events for a single recording document.
#[derive(Clone)]
pub struct LiveBatch {
    /// The document's committed byte length after this batch. A subscriber
    /// forwards the batch only when this exceeds the length its bootstrap
    /// already read, so a batch the bootstrap captured is never re-applied and
    /// none is skipped: batches are atomic and contiguous, so the cutoff is
    /// exact.
    pub end_offset: i64,
    pub events: Arc<[RecordingEventInput]>,
}

/// Routes newly accepted rrweb batches from the ingest path to any live-preview
/// subscribers watching the same document.
#[derive(Default)]
pub struct LiveRecordingBus {
    channels: Mutex<HashMap<String, broadcast::Sender<LiveBatch>>>,
}

impl LiveRecordingBus {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Subscribes to a document's live batches, creating the channel on first
    /// interest. The returned receiver keeps the channel alive until dropped.
    pub async fn subscribe(&self, document_id: &str) -> broadcast::Receiver<LiveBatch> {
        let mut channels = self.channels.lock().await;
        channels
            .entry(document_id.to_string())
            .or_insert_with(|| broadcast::channel(LIVE_CHANNEL_CAPACITY).0)
            .subscribe()
    }

    /// Publishes a batch to a document's subscribers. A no-op when nobody is
    /// listening; the channel is dropped once its last receiver is gone so idle
    /// documents do not accumulate.
    pub async fn publish(
        &self,
        document_id: &str,
        end_offset: i64,
        events: Arc<[RecordingEventInput]>,
    ) {
        let mut channels = self.channels.lock().await;
        let Some(sender) = channels.get(document_id) else {
            return;
        };
        let batch = LiveBatch { end_offset, events };
        if sender.send(batch).is_err() {
            channels.remove(document_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(ts: i64) -> RecordingEventInput {
        RecordingEventInput {
            ts,
            event_type: Some(json!(3)),
            data: Some(json!({ "ts": ts })),
        }
    }

    #[tokio::test]
    async fn delivers_batches_with_their_offset_to_a_document_subscriber() -> anyhow::Result<()> {
        let bus = LiveRecordingBus::new();
        let mut rx = bus.subscribe("doc").await;
        bus.publish("doc", 64, Arc::from(vec![event(1), event(2)]))
            .await;
        let batch = rx.recv().await?;
        assert_eq!(batch.end_offset, 64);
        assert_eq!(batch.events.len(), 2);
        assert_eq!(batch.events[0].ts, 1);
        Ok(())
    }

    #[tokio::test]
    async fn publish_without_subscribers_is_a_noop() -> anyhow::Result<()> {
        let bus = LiveRecordingBus::new();
        bus.publish("doc", 32, Arc::from(vec![event(1)])).await;
        // A later subscriber only sees batches published after it subscribes.
        let mut rx = bus.subscribe("doc").await;
        bus.publish("doc", 64, Arc::from(vec![event(2)])).await;
        assert_eq!(rx.recv().await?.events[0].ts, 2);
        Ok(())
    }

    #[tokio::test]
    async fn a_document_is_isolated_from_other_documents() -> anyhow::Result<()> {
        let bus = LiveRecordingBus::new();
        let mut rx = bus.subscribe("a").await;
        bus.publish("b", 32, Arc::from(vec![event(1)])).await;
        bus.publish("a", 32, Arc::from(vec![event(2)])).await;
        assert_eq!(rx.recv().await?.events[0].ts, 2);
        Ok(())
    }
}
