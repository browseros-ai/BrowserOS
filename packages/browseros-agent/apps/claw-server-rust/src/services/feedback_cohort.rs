//! Membership of the top-user cohort, answered from memory.
//!
//! The decision this serves runs while the cockpit is painting, so nothing here may touch
//! the network when asked. A scheduled job publishes a small document of install ids; this
//! fetches it on a timer and answers from a set held in memory.
//!
//! Every failure is silent and means "not eligible": no document, an unreachable host,
//! malformed JSON, or a document older than [`MAX_AGE`]. A feature that quietly invites
//! nobody is correct; one that puts an error in front of somebody is not.

use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{sync::RwLock, task::JoinHandle};
use tokio_util::sync::CancellationToken;

/// Where the published document lives. Absent at both build and run time means the feature
/// is off, which is the same silent posture as an unreachable document.
const BUILD_COHORT_URL: Option<&str> = option_env!("CLAW_FEEDBACK_COHORT_URL");

/// How long a single fetch may take before it is abandoned.
///
/// A hung request must not pin the refresh task; the previous document stays in place and
/// the next tick tries again.
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// Refuses a response larger than this without reading it.
///
/// The document is a few kilobytes of identifiers. Anything far larger is a misconfigured
/// URL or something hostile, and neither is worth buffering.
const MAX_DOCUMENT_BYTES: u64 = 1024 * 1024;

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
    /// A set lookup behind a read lock, with no I/O of its own.
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

/// The configured document location, runtime taking precedence over the build default.
#[must_use]
pub fn configured_url() -> Option<String> {
    resolve_url(
        std::env::var("CLAW_FEEDBACK_COHORT_URL").ok(),
        BUILD_COHORT_URL,
    )
}

/// Split from [`configured_url`] so the precedence rule can be tested without mutating the
/// process environment, which this crate forbids.
fn resolve_url(runtime: Option<String>, build: Option<&str>) -> Option<String> {
    runtime
        .filter(|url| !url.trim().is_empty())
        .or_else(|| build.map(str::to_owned))
        .map(|url| url.trim().to_owned())
        .filter(|url| !url.is_empty())
}

/// Fetches the published document once and adopts it if it is usable.
///
/// Returns whether the cohort changed. Every failure path is a warning and a `false`: a
/// refused or unreachable document leaves the previous one in place rather than emptying
/// the cohort, so a brief outage does not stop invitations that were already permitted.
pub async fn fetch_once(cohort: &FeedbackCohort, url: &str, now_ms: i64) -> bool {
    let client = match reqwest::Client::builder().timeout(FETCH_TIMEOUT).build() {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(%error, "feedback cohort client unavailable");
            return false;
        }
    };
    let response = match client.get(url).send().await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(%error, "feedback cohort fetch failed");
            return false;
        }
    };
    if !response.status().is_success() {
        tracing::warn!(status = %response.status(), "feedback cohort fetch rejected");
        return false;
    }
    // Checked before the body is buffered, so an oversized document costs nothing.
    if response
        .content_length()
        .is_some_and(|len| len > MAX_DOCUMENT_BYTES)
    {
        tracing::warn!("feedback cohort document is larger than expected; ignoring it");
        return false;
    }
    let document = match response.json::<CohortDocument>().await {
        Ok(document) => document,
        Err(error) => {
            tracing::warn!(%error, "feedback cohort document unreadable");
            return false;
        }
    };
    cohort.adopt(document, now_ms).await
}

/// Refreshes the cohort on a timer until the server shuts down.
///
/// Fetches immediately rather than waiting a full interval, so a restart picks up a new
/// document without a six hour delay.
pub fn spawn_refresh_loop(
    cohort: FeedbackCohort,
    url: String,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(REFRESH_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                _ = ticker.tick() => {
                    fetch_once(&cohort, &url, now_ms()).await;
                }
            }
        }
    })
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

    /// A served document reaches the cohort. Uses a real listener rather than mocking the
    /// client, so the JSON contract and the HTTP path are both exercised.
    #[tokio::test]
    async fn a_served_document_is_fetched_and_adopted() -> anyhow::Result<()> {
        let now = now_ms();
        let body = serde_json::to_string(&serde_json::json!({
            "generated_at_ms": now,
            "installs": ["install-a"],
            "copy": { "book_url": "https://cal.example/book" }
        }))?;
        let url = serve_once(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ))
        .await?;

        let cohort = FeedbackCohort::new();
        assert!(fetch_once(&cohort, &url, now).await);
        assert!(cohort.contains("install-a", now).await);
        Ok(())
    }

    /// A body that is not a cohort document leaves the previous answer alone rather than
    /// emptying it, so a bad deploy of the publisher does not stop invitations mid-flight.
    #[tokio::test]
    async fn an_unreadable_body_leaves_the_previous_cohort_in_place() -> anyhow::Result<()> {
        let now = now_ms();
        let cohort = FeedbackCohort::new();
        assert!(cohort.adopt(document(now, &["install-a"]), now).await);

        let body = "not json at all";
        let url = serve_once(format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ))
        .await?;
        assert!(!fetch_once(&cohort, &url, now).await);
        assert!(
            cohort.contains("install-a", now).await,
            "a bad fetch must not empty the cohort"
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_failing_response_is_ignored() -> anyhow::Result<()> {
        let url = serve_once(
            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n".to_string(),
        )
        .await?;
        let cohort = FeedbackCohort::new();
        assert!(!fetch_once(&cohort, &url, now_ms()).await);
        assert!(!cohort.is_loaded().await);
        Ok(())
    }

    /// The document is a few kilobytes of identifiers. A declared size far beyond that is a
    /// misconfigured URL or something hostile, and is refused without buffering the body.
    #[tokio::test]
    async fn an_oversized_document_is_refused_before_it_is_read() -> anyhow::Result<()> {
        let url = serve_once(format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            MAX_DOCUMENT_BYTES + 1
        ))
        .await?;
        let cohort = FeedbackCohort::new();
        assert!(!fetch_once(&cohort, &url, now_ms()).await);
        Ok(())
    }

    #[test]
    fn no_configured_url_means_the_feature_is_off() {
        assert_eq!(resolve_url(None, None), None);
        assert_eq!(resolve_url(Some("   ".to_string()), None), None);
        assert_eq!(resolve_url(Some(String::new()), Some("   ")), None);
    }

    #[test]
    fn a_runtime_url_overrides_the_build_default() {
        assert_eq!(
            resolve_url(
                Some("  https://run.test/c.json ".to_string()),
                Some("https://build.test/c.json")
            )
            .as_deref(),
            Some("https://run.test/c.json")
        );
        // A blank runtime value falls through rather than disabling a built-in default.
        assert_eq!(
            resolve_url(Some("  ".to_string()), Some("https://build.test/c.json")).as_deref(),
            Some("https://build.test/c.json")
        );
    }

    /// Serves one canned response and returns the URL to ask for it.
    async fn serve_once(response: String) -> anyhow::Result<String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/cohort.json", listener.local_addr()?);
        tokio::spawn(async move {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut scratch = [0_u8; 1024];
            let _ = socket.read(&mut scratch).await;
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
        Ok(url)
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
