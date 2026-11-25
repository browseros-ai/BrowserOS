//! What the server knows about legacy MCP transport sessions after they are gone, so a request
//! for one can say whether it expired or never existed. Ids are kept only as log fingerprints.

use crate::ids::log_fingerprint;
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tracing::info;

/// Bounds both maps; a busy server forgets the oldest entries first.
const REMEMBERED: usize = 1024;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum TransportFate {
    /// The transport ended this long ago: rmcp expired it or the client sent DELETE.
    Ended(Duration),
    /// Never seen, or ended so long ago it was forgotten.
    Unknown,
}

#[derive(Default)]
pub struct TransportLog {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    last_request: HashMap<String, Instant>,
    ended: VecDeque<(String, Instant)>,
}

impl TransportLog {
    /// Records a request carrying `transport_id`, the moment rmcp's idle clock restarts.
    pub async fn touch(&self, transport_id: &str) {
        let mut inner = self.inner.lock().await;
        if inner.last_request.len() >= REMEMBERED
            && let Some(oldest) = inner
                .last_request
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(fingerprint, _)| fingerprint.clone())
        {
            inner.last_request.remove(&oldest);
        }
        inner
            .last_request
            .insert(log_fingerprint(transport_id), Instant::now());
    }

    /// Logs the end of a transport with how long it had been idle.
    pub async fn ended(&self, transport_id: &str) {
        let fingerprint = log_fingerprint(transport_id);
        let now = Instant::now();
        let mut inner = self.inner.lock().await;
        let idle_for = inner
            .last_request
            .remove(&fingerprint)
            .map(|at| now.duration_since(at));
        info!(transport = %fingerprint, ?idle_for, "mcp transport session ended");
        if inner.ended.len() >= REMEMBERED {
            inner.ended.pop_front();
        }
        inner.ended.push_back((fingerprint, now));
    }

    pub async fn fate(&self, transport_id: &str) -> TransportFate {
        let fingerprint = log_fingerprint(transport_id);
        let inner = self.inner.lock().await;
        inner
            .ended
            .iter()
            .rev()
            .find(|(ended, _)| *ended == fingerprint)
            .map_or(TransportFate::Unknown, |(_, at)| {
                TransportFate::Ended(at.elapsed())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_ended_transport_is_told_apart_from_one_never_seen() {
        let log = TransportLog::default();
        log.touch("transport-a").await;
        log.ended("transport-a").await;
        assert!(matches!(
            log.fate("transport-a").await,
            TransportFate::Ended(_)
        ));
        assert_eq!(log.fate("transport-b").await, TransportFate::Unknown);
    }

    #[tokio::test]
    async fn the_oldest_ended_transport_is_forgotten_first() {
        let log = TransportLog::default();
        for index in 0..=REMEMBERED {
            log.ended(&format!("transport-{index}")).await;
        }
        assert_eq!(log.fate("transport-0").await, TransportFate::Unknown);
        assert!(matches!(
            log.fate(&format!("transport-{REMEMBERED}")).await,
            TransportFate::Ended(_)
        ));
    }
}
