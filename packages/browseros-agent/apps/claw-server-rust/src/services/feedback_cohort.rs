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

/// The PostHog remote configuration flag carrying the cohort.
///
/// Remote configuration delivers the same payload to every caller, so membership is still
/// decided here rather than by PostHog. That matters: PostHog answers targeting questions
/// from person profiles, and this product deliberately sends `$process_person_profile:
/// false`, so no profile exists to target.
const COHORT_FLAG_KEY: &str = "feedback-call-cohort";

/// The identity used when reading remote configuration.
///
/// Deliberately constant rather than the installation id. The payload is identical for
/// everyone, so sending the real id would tell PostHog which installations are polling
/// without changing the answer.
const REMOTE_CONFIG_IDENTITY: &str = "browserclaw-remote-config";

/// Overrides the PostHog source with a plain JSON document. For local testing, and as an
/// escape hatch if remote configuration is ever unavailable.
const BUILD_COHORT_URL: Option<&str> = option_env!("CLAW_FEEDBACK_COHORT_URL");

/// Where the invitation points when the published document does not override it.
pub const DEFAULT_BOOK_URL: &str = "https://cal.com/team/felafax/browseros";

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

    /// Where to point this installation's invitation, or `None` if it is not in the
    /// current cohort.
    ///
    /// Membership and the link come from one snapshot under one lock. Reading them
    /// separately would let a refresh land in between and admit an installation on the
    /// strength of one document while handing it another document's link.
    ///
    /// A set lookup behind a read lock, with no I/O of its own.
    pub async fn invitation_url(&self, install_id: &str, now_ms: i64) -> Option<String> {
        let guard = self.state.read().await;
        let state = guard.as_ref()?;
        if is_stale(state.generated_at_ms, now_ms) {
            return None;
        }
        if !state.installs.contains(install_id) {
            return None;
        }
        Some(
            state
                .book_url
                .clone()
                .unwrap_or_else(|| DEFAULT_BOOK_URL.to_owned()),
        )
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

/// Where the cohort is read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CohortSource {
    /// A plain JSON document served at a URL.
    Document(String),
    /// PostHog remote configuration, read with the credentials this build already ships.
    RemoteConfig { host: String, project_key: String },
}

/// The configured source, or `None` when this build can reach neither.
///
/// An explicit document URL wins so a developer can point at a local file; otherwise the
/// analytics credentials the build already embeds are enough, which is the whole point:
/// shipping this needs no second thing to configure.
#[must_use]
pub fn configured_source(posthog: Option<(String, String)>) -> Option<CohortSource> {
    resolve_source(
        std::env::var("CLAW_FEEDBACK_COHORT_URL").ok(),
        BUILD_COHORT_URL,
        posthog,
    )
}

/// Split from [`configured_source`] so the precedence rule can be tested without mutating
/// the process environment, which this crate forbids.
fn resolve_source(
    runtime: Option<String>,
    build: Option<&str>,
    posthog: Option<(String, String)>,
) -> Option<CohortSource> {
    let document = runtime
        .filter(|url| !url.trim().is_empty())
        .or_else(|| build.map(str::to_owned))
        .map(|url| url.trim().to_owned())
        .filter(|url| !url.is_empty());
    if let Some(url) = document {
        return Some(CohortSource::Document(url));
    }
    let (host, project_key) = posthog?;
    let host = host.trim().trim_end_matches('/').to_owned();
    let project_key = project_key.trim().to_owned();
    (!host.is_empty() && !project_key.is_empty())
        .then_some(CohortSource::RemoteConfig { host, project_key })
}

/// PostHog's flag evaluation response. Only the one payload is read; everything else about
/// the envelope is ignored so PostHog can extend it freely.
#[derive(Debug, serde::Deserialize)]
struct FlagsEnvelope {
    #[serde(default)]
    flags: std::collections::HashMap<String, FlagEntry>,
}

#[derive(Debug, serde::Deserialize)]
struct FlagEntry {
    #[serde(default)]
    metadata: FlagMetadata,
}

#[derive(Debug, Default, serde::Deserialize)]
struct FlagMetadata {
    /// The remote configuration payload, delivered as a JSON-encoded string.
    #[serde(default)]
    payload: Option<String>,
}

/// Pulls the cohort document out of a flag evaluation response.
fn document_from_flags(body: &[u8]) -> Option<CohortDocument> {
    let envelope: FlagsEnvelope = serde_json::from_slice(body)
        .inspect_err(|error| tracing::warn!(%error, "feedback cohort flags response unreadable"))
        .ok()?;
    let payload = envelope
        .flags
        .get(COHORT_FLAG_KEY)?
        .metadata
        .payload
        .as_ref()?;
    serde_json::from_str(payload)
        .inspect_err(|error| tracing::warn!(%error, "feedback cohort payload unreadable"))
        .ok()
}

/// Fetches the published document once and adopts it if it is usable.
///
/// Returns whether the cohort changed. Every failure path is a warning and a `false`: a
/// refused or unreachable document leaves the previous one in place rather than emptying
/// the cohort, so a brief outage does not stop invitations that were already permitted.
pub async fn fetch_once(cohort: &FeedbackCohort, source: &CohortSource, now_ms: i64) -> bool {
    let client = match reqwest::Client::builder().timeout(FETCH_TIMEOUT).build() {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(%error, "feedback cohort client unavailable");
            return false;
        }
    };
    let request = match source {
        CohortSource::Document(url) => client.get(url),
        CohortSource::RemoteConfig { host, project_key } => client
            .post(format!("{host}/flags?v=2"))
            .json(&serde_json::json!({
                "api_key": project_key,
                "distinct_id": REMOTE_CONFIG_IDENTITY,
            })),
    };
    let response = match request.send().await {
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
    let Some(body) = read_capped(response).await else {
        return false;
    };
    let document = match source {
        CohortSource::Document(_) => match serde_json::from_slice::<CohortDocument>(&body) {
            Ok(document) => Some(document),
            Err(error) => {
                tracing::warn!(%error, "feedback cohort document unreadable");
                None
            }
        },
        CohortSource::RemoteConfig { .. } => document_from_flags(&body),
    };
    let Some(document) = document else {
        return false;
    };
    cohort.adopt(document, now_ms).await
}

/// Reads at most [`MAX_DOCUMENT_BYTES`] of the body, refusing rather than buffering more.
///
/// The ceiling is enforced against the bytes actually read rather than against
/// `Content-Length`, which is advisory and absent altogether on a chunked response.
async fn read_capped(mut response: reqwest::Response) -> Option<Vec<u8>> {
    let cap = usize::try_from(MAX_DOCUMENT_BYTES).unwrap_or(usize::MAX);
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len().saturating_add(chunk.len()) > cap {
                    tracing::warn!("feedback cohort document is larger than expected; ignoring it");
                    return None;
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => return Some(body),
            Err(error) => {
                tracing::warn!(%error, "feedback cohort body unreadable");
                return None;
            }
        }
    }
}

/// Refreshes the cohort on a timer until the server shuts down.
///
/// Fetches immediately rather than waiting a full interval, so a restart picks up a new
/// document without a six hour delay.
pub fn spawn_refresh_loop(
    cohort: FeedbackCohort,
    source: CohortSource,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(REFRESH_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                _ = ticker.tick() => {
                    fetch_once(&cohort, &source, now_ms()).await;
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
            cohort.invitation_url("install-a", now).await.is_none(),
            "empty until adopted"
        );

        assert!(
            cohort
                .adopt(document(now, &["install-a", "install-b"]), now)
                .await
        );
        assert!(cohort.invitation_url("install-a", now).await.is_some());
        assert!(cohort.invitation_url("install-c", now).await.is_none());
        assert_eq!(
            cohort.invitation_url("install-a", now).await.as_deref(),
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
        assert!(cohort.invitation_url("install-a", now).await.is_none());
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
        assert!(cohort.invitation_url("install-a", now).await.is_some());

        let much_later = now + i64::try_from(MAX_AGE.as_millis()).unwrap_or(i64::MAX) + 1;
        assert!(
            cohort
                .invitation_url("install-a", much_later)
                .await
                .is_none()
        );
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
        assert!(fetch_once(&cohort, &CohortSource::Document(url), now).await);
        assert!(cohort.invitation_url("install-a", now).await.is_some());
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
        assert!(!fetch_once(&cohort, &CohortSource::Document(url), now).await);
        assert!(
            cohort.invitation_url("install-a", now).await.is_some(),
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
        assert!(!fetch_once(&cohort, &CohortSource::Document(url), now_ms()).await);
        assert!(!cohort.is_loaded().await);
        Ok(())
    }

    /// The document is a few kilobytes of identifiers. Anything far beyond that is a
    /// misconfigured URL or something hostile, and the read stops at the ceiling rather
    /// than buffering whatever arrives.
    ///
    /// The body is a document that would parse and be adopted if it were read in full, so
    /// the refusal can only come from the ceiling.
    #[tokio::test]
    async fn an_oversized_document_is_refused() -> anyhow::Result<()> {
        let body = oversized_but_valid_document(now_ms())?;
        let url = serve_once(format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ))
        .await?;
        let cohort = FeedbackCohort::new();
        assert!(!fetch_once(&cohort, &CohortSource::Document(url), now_ms()).await);
        assert!(!cohort.is_loaded().await);
        Ok(())
    }

    /// A document just inside the ceiling still gets through, so the cap is not simply
    /// refusing everything large.
    #[tokio::test]
    async fn a_document_just_inside_the_ceiling_is_adopted() -> anyhow::Result<()> {
        let now = now_ms();
        let body = padded_document(now, usize::try_from(MAX_DOCUMENT_BYTES)? - 4096)?;
        assert!(body.len() <= usize::try_from(MAX_DOCUMENT_BYTES)?);
        let url = serve_once(format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ))
        .await?;

        let cohort = FeedbackCohort::new();
        assert!(fetch_once(&cohort, &CohortSource::Document(url), now).await);
        assert!(cohort.invitation_url("install-a", now).await.is_some());
        Ok(())
    }

    /// A valid document padded past the ceiling. Unknown fields are ignored by the parser,
    /// so this deserializes cleanly whenever it is read in full.
    fn oversized_but_valid_document(generated_at_ms: i64) -> anyhow::Result<String> {
        padded_document(generated_at_ms, usize::try_from(MAX_DOCUMENT_BYTES)? + 4096)
    }

    fn padded_document(generated_at_ms: i64, target_bytes: usize) -> anyhow::Result<String> {
        let skeleton = serde_json::to_string(&json!({
            "generated_at_ms": generated_at_ms,
            "installs": ["install-a"],
            "padding": "",
        }))?;
        let padding = "a".repeat(target_bytes.saturating_sub(skeleton.len()));
        Ok(serde_json::to_string(&json!({
            "generated_at_ms": generated_at_ms,
            "installs": ["install-a"],
            "padding": padding,
        }))?)
    }

    fn posthog() -> Option<(String, String)> {
        Some((
            "https://us.i.posthog.com/".to_string(),
            "phc_example".to_string(),
        ))
    }

    /// The point of reading PostHog remote configuration: a build that ships analytics
    /// credentials needs nothing else configured to reach a cohort.
    #[test]
    fn the_shipped_analytics_credentials_are_enough() {
        assert_eq!(
            resolve_source(None, None, posthog()),
            Some(CohortSource::RemoteConfig {
                host: "https://us.i.posthog.com".to_string(),
                project_key: "phc_example".to_string(),
            })
        );
    }

    #[test]
    fn a_build_without_analytics_credentials_reaches_no_source() {
        assert_eq!(resolve_source(None, None, None), None);
        assert_eq!(
            resolve_source(None, None, Some((String::new(), "phc_example".to_string()))),
            None
        );
        assert_eq!(
            resolve_source(
                None,
                None,
                Some(("https://us.i.posthog.com".to_string(), "  ".to_string()))
            ),
            None
        );
    }

    /// An explicit document wins so a developer can point at a local file.
    #[test]
    fn an_explicit_document_overrides_remote_configuration() {
        assert_eq!(
            resolve_source(
                Some("  http://127.0.0.1:8787/cohort.json ".to_string()),
                None,
                posthog()
            ),
            Some(CohortSource::Document(
                "http://127.0.0.1:8787/cohort.json".to_string()
            ))
        );
        // A blank runtime value falls through rather than disabling a built-in default.
        assert_eq!(
            resolve_source(
                Some("  ".to_string()),
                Some("https://build.test/c.json"),
                posthog()
            ),
            Some(CohortSource::Document(
                "https://build.test/c.json".to_string()
            ))
        );
    }

    /// The exact envelope PostHog returns, captured from a live remote configuration flag.
    #[test]
    fn a_remote_configuration_payload_is_read_out_of_the_envelope() {
        let body = serde_json::to_vec(&json!({
            "errorsWhileComputingFlags": false,
            "flags": {
                "feedback-call-cohort": {
                    "key": "feedback-call-cohort",
                    "enabled": true,
                    "variant": null,
                    "reason": { "code": "condition_match", "condition_index": 0 },
                    "metadata": {
                        "id": 902303,
                        "version": 1,
                        "payload": "{\"generated_at_ms\": 1790101086157, \"installs\": [\"install-a\"]}",
                        "has_experiment": false
                    }
                }
            },
            "requestId": "01e09120-8f07-4a70-bc77-bc29223b2911"
        }))
        .unwrap_or_else(|error| panic!("fixture: {error}"));

        let Some(document) = document_from_flags(&body) else {
            panic!("the payload was not read out of the envelope");
        };
        assert_eq!(document.installs, vec!["install-a".to_string()]);
        assert_eq!(document.generated_at_ms, 1_790_101_086_157);
    }

    /// The flag being absent, disabled or payload-free all mean the same thing: no cohort.
    #[test]
    fn an_envelope_without_our_flag_yields_no_document() {
        let empty = serde_json::to_vec(&json!({ "flags": {} }))
            .unwrap_or_else(|error| panic!("fixture: {error}"));
        assert!(document_from_flags(&empty).is_none());

        let no_payload = serde_json::to_vec(&json!({
            "flags": { "feedback-call-cohort": { "metadata": { "id": 1 } } }
        }))
        .unwrap_or_else(|error| panic!("fixture: {error}"));
        assert!(document_from_flags(&no_payload).is_none());

        assert!(document_from_flags(b"not json").is_none());
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

    /// A document that supplies no link still invites, pointed at the built-in default.
    #[tokio::test]
    async fn a_document_without_copy_falls_back_to_the_default_link() {
        let now = 1_700_000_000_000;
        let cohort = FeedbackCohort::new();
        let document: CohortDocument = serde_json::from_value(json!({
            "generated_at_ms": now,
            "installs": ["install-a"],
        }))
        .unwrap_or_else(|error| panic!("fixture: {error}"));

        assert!(cohort.adopt(document, now).await);
        assert_eq!(
            cohort.invitation_url("install-a", now).await.as_deref(),
            Some(DEFAULT_BOOK_URL)
        );
    }

    /// `Content-Length` is advisory and absent on a chunked response, so the ceiling has
    /// to hold against the bytes actually read.
    #[tokio::test]
    async fn an_oversized_chunked_document_is_refused_mid_read() -> anyhow::Result<()> {
        // No Content-Length at all, which is exactly the case the old check could not see.
        let body = oversized_but_valid_document(now_ms())?;
        let url = serve_once(format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",
            body.len()
        ))
        .await?;

        let cohort = FeedbackCohort::new();
        assert!(!fetch_once(&cohort, &CohortSource::Document(url), now_ms()).await);
        assert!(!cohort.is_loaded().await);
        Ok(())
    }

    /// The cap must not break the ordinary chunked path.
    #[tokio::test]
    async fn a_chunked_document_within_the_cap_is_adopted() -> anyhow::Result<()> {
        let now = now_ms();
        let body = serde_json::to_string(&json!({
            "generated_at_ms": now,
            "installs": ["install-a"],
        }))?;
        let url = serve_once(format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",
            body.len()
        ))
        .await?;

        let cohort = FeedbackCohort::new();
        assert!(fetch_once(&cohort, &CohortSource::Document(url), now).await);
        assert!(cohort.invitation_url("install-a", now).await.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn a_refresh_replaces_the_previous_cohort() {
        let now = 1_700_000_000_000;
        let cohort = FeedbackCohort::new();
        assert!(cohort.adopt(document(now, &["install-a"]), now).await);
        assert!(cohort.adopt(document(now, &["install-b"]), now).await);

        assert!(
            cohort.invitation_url("install-a", now).await.is_none(),
            "dropped on refresh"
        );
        assert!(cohort.invitation_url("install-b", now).await.is_some());
    }
}
