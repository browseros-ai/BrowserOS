//! Membership of the top-user cohort, answered from memory.
//!
//! The decision this serves runs inside a tool response, on the latency path of every MCP
//! call an agent makes, so nothing here may touch the network or the database when asked.
//! A scheduled job publishes a small document of install ids; this fetches it on a timer and
//! answers from a set held in memory.
//!
//! Every failure is silent and means "not eligible": no document, an unreachable host,
//! malformed JSON, or a document older than [`MAX_AGE`]. A feature that quietly invites
//! nobody is correct; one that reports an error into somebody's task is not.

use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::RwLock;

/// How stale a published cohort may be before it stops being believed.
///
/// A cohort is a claim about who was active recently. If the job that publishes it has been
/// broken for a fortnight, the right behaviour is to invite nobody rather than to keep
/// inviting from a list nobody is maintaining.
pub const MAX_AGE: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// How often to re-fetch. The cohort changes daily at most, so this is deliberately slow.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// The published document. Unknown fields are ignored so the publisher can add copy
/// variants without every shipped server needing to understand them.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CohortDocument {
    /// Unix milliseconds. Absent or unparsable means the document is not trusted.
    pub generated_at_ms: i64,
    /// Server-minted installation UUIDs. Random v4 values that identify nobody.
    pub installs: Vec<String>,
    #[serde(default)]
    pub copy: CohortCopy,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct CohortCopy {
    #[serde(default)]
    pub book_url: Option<String>,
}

#[derive(Debug, Default)]
struct CohortState {
    installs: HashSet<String>,
    generated_at_ms: i64,
    book_url: Option<String>,
}

/// Holds the current cohort. Cheap to clone, cheap to ask.
#[derive(Clone, Default)]
pub struct FeedbackCohort {
    state: Arc<RwLock<Option<CohortState>>>,
}

impl FeedbackCohort {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether this installation is in the current cohort.
    ///
    /// A set lookup behind a read lock: no I/O, safe to call on every tool dispatch.
    pub async fn contains(&self, install_id: &str, now_ms: i64) -> bool {
        let guard = self.state.read().await;
        let Some(state) = guard.as_ref() else {
            return false;
        };
        if is_stale(state.generated_at_ms, now_ms) {
            return false;
        }
        state.installs.contains(install_id)
    }

    /// The booking link the publisher wants used, if it supplied one.
    pub async fn book_url(&self) -> Option<String> {
        self.state
            .read()
            .await
            .as_ref()
            .and_then(|state| state.book_url.clone())
    }

    /// Replaces the cohort. A document that is already stale is rejected rather than
    /// stored, so a publisher that stops running cannot be masked by a successful fetch.
    pub async fn adopt(&self, document: CohortDocument, now_ms: i64) -> bool {
        if is_stale(document.generated_at_ms, now_ms) {
            tracing::warn!(
                generated_at_ms = document.generated_at_ms,
                "feedback cohort is older than the freshness window; ignoring it"
            );
            return false;
        }
        let state = CohortState {
            installs: document.installs.into_iter().collect(),
            generated_at_ms: document.generated_at_ms,
            book_url: document.copy.book_url,
        };
        tracing::info!(installs = state.installs.len(), "feedback cohort adopted");
        *self.state.write().await = Some(state);
        true
    }

    /// True once a usable cohort has been adopted. Used only by diagnostics.
    pub async fn is_loaded(&self) -> bool {
        self.state.read().await.is_some()
    }
}

fn is_stale(generated_at_ms: i64, now_ms: i64) -> bool {
    let age_ms = now_ms.saturating_sub(generated_at_ms);
    // A document stamped in the future is as untrustworthy as one that is too old.
    age_ms < 0 || age_ms > i64::try_from(MAX_AGE.as_millis()).unwrap_or(i64::MAX)
}

#[must_use]
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document(generated_at_ms: i64, installs: &[&str]) -> CohortDocument {
        serde_json::from_value(json!({
            "generated_at_ms": generated_at_ms,
            "installs": installs,
            "copy": { "book_url": "https://cal.example/book" },
            // A field this build does not know about must not break the fetch, so the
            // publisher can add copy variants without waiting for every server to update.
            "unknown_future_field": { "anything": true }
        }))
        .unwrap_or_else(|error| panic!("fixture: {error}"))
    }

    #[tokio::test]
    async fn membership_is_answered_from_the_adopted_document() {
        let now = 1_700_000_000_000;
        let cohort = FeedbackCohort::new();
        assert!(
            !cohort.contains("install-a", now).await,
            "empty until adopted"
        );

        assert!(
            cohort
                .adopt(document(now, &["install-a", "install-b"]), now)
                .await
        );
        assert!(cohort.contains("install-a", now).await);
        assert!(!cohort.contains("install-c", now).await);
        assert_eq!(
            cohort.book_url().await.as_deref(),
            Some("https://cal.example/book")
        );
    }

    /// A cohort is a claim about who was active recently. If the job publishing it has been
    /// broken for a fortnight, inviting nobody is the right answer.
    #[tokio::test]
    async fn a_document_past_the_freshness_window_is_refused() {
        let now = 1_700_000_000_000;
        let stale = now - i64::try_from(MAX_AGE.as_millis()).unwrap_or(i64::MAX) - 1;
        let cohort = FeedbackCohort::new();

        assert!(!cohort.adopt(document(stale, &["install-a"]), now).await);
        assert!(!cohort.contains("install-a", now).await);
        assert!(
            !cohort.is_loaded().await,
            "a refused document is not stored"
        );
    }

    /// Adopted fresh, then time passes. Membership has to lapse on read as well as on
    /// adoption, or a long-running server keeps inviting from a list nobody maintains.
    #[tokio::test]
    async fn an_adopted_document_lapses_as_it_ages() {
        let now = 1_700_000_000_000;
        let cohort = FeedbackCohort::new();
        assert!(cohort.adopt(document(now, &["install-a"]), now).await);
        assert!(cohort.contains("install-a", now).await);

        let much_later = now + i64::try_from(MAX_AGE.as_millis()).unwrap_or(i64::MAX) + 1;
        assert!(!cohort.contains("install-a", much_later).await);
    }

    /// A clock skewed into the future is as untrustworthy as one that is too old.
    #[tokio::test]
    async fn a_document_stamped_in_the_future_is_refused() {
        let now = 1_700_000_000_000;
        let cohort = FeedbackCohort::new();
        assert!(
            !cohort
                .adopt(document(now + 60_000, &["install-a"]), now)
                .await
        );
    }

    #[tokio::test]
    async fn a_refresh_replaces_the_previous_cohort() {
        let now = 1_700_000_000_000;
        let cohort = FeedbackCohort::new();
        assert!(cohort.adopt(document(now, &["install-a"]), now).await);
        assert!(cohort.adopt(document(now, &["install-b"]), now).await);

        assert!(
            !cohort.contains("install-a", now).await,
            "dropped on refresh"
        );
        assert!(cohort.contains("install-b", now).await);
    }
}
