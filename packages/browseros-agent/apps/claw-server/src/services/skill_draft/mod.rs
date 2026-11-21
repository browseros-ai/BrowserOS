//! Making a skill from a finished task. The app asks for a draft; this module builds a redacted
//! digest of the task, has the decisions worker write the draft (the worker owns the model, its
//! key and the per-licence limits), checks it, and keeps it so every view of the task shares one
//! state. Nothing becomes a skill until the person reviews the draft and saves it, which goes
//! through the regular skill create path.
//!
//! ```text
//! app ──POST /sessions/{id}/skill-draft──▶ SkillDraftService::draft ──▶ decisions worker
//!       (per-task lock: a second POST waits      (request id kept on the row, so a
//!        and gets the same draft)                  retry is counted once)
//! ```

pub mod checks;
pub mod digest;
pub mod redact;

use std::{collections::HashMap, future::Future, sync::Arc};

use claw_api::models::{
    SkillDraft, SkillDraftFailure, SkillDraftFailureCode, SkillDraftState, SkillDraftStatus,
};
use tokio::sync::Mutex;

use crate::{
    db::{SkillsRepository, entities::skill_drafts},
    error::AppResult,
    services::skills::neo_prefixed,
};
use digest::DraftRequest;

/// How long a draft that was not saved is kept: reopening it within this costs nothing.
pub const DRAFT_TTL_MS: i64 = 24 * 60 * 60 * 1000;

/// A `writing` row older than this has no request behind it any more (the server restarted
/// mid-draft), so it reads as failed and may be retried. Above the worker call's own timeout.
const STALE_WRITING_MS: i64 = 90 * 1000;

const WRITING: &str = "writing";
const READY: &str = "ready";
const FAILED: &str = "failed";

/// Keeps skill drafts per task and makes sure only one is ever being written for a task.
pub struct SkillDraftService {
    repo: SkillsRepository,
    /// One lock per task, held while its draft is written, so concurrent requests for the same
    /// task wait for the first one and then read its result instead of paying for a second.
    writing: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl SkillDraftService {
    #[must_use]
    pub fn new(repo: SkillsRepository) -> Self {
        Self {
            repo,
            writing: Mutex::new(HashMap::new()),
        }
    }

    /// Where making a skill from this task stands. `skill_name` is the skill already saved from
    /// it, if any, which wins over any draft.
    pub async fn status(
        &self,
        session_id: &str,
        skill_name: Option<String>,
        now: i64,
    ) -> AppResult<SkillDraftStatus> {
        let row = self.repo.get_draft(session_id).await?;
        Ok(status_of(row.as_ref(), skill_name, now))
    }

    /// Writes a draft for this task through `call`, or returns the one already written. A draft
    /// that failed is retried with the same request id, so the worker never counts it twice.
    /// `request` is the task's digest, or `None` when the task gives nothing to draft from.
    pub async fn draft<C, F>(
        &self,
        session_id: &str,
        now: i64,
        request: Option<DraftRequest>,
        call: C,
    ) -> AppResult<SkillDraftStatus>
    where
        C: FnOnce(DraftRequest) -> F,
        F: Future<Output = Result<SkillDraft, SkillDraftFailure>>,
    {
        let lock = self.lock_for(session_id).await;
        let _held = lock.lock().await;
        let previous = self.repo.get_draft(session_id).await?;
        if let Some(row) = previous.as_ref()
            && row.status == READY
            && now - row.created_at < DRAFT_TTL_MS
        {
            return Ok(status_of(previous.as_ref(), None, now));
        }
        // Holding the lock means no request is still writing a `writing` row, so it is retried.
        let (request_id, created_at) = match previous {
            Some(row) if row.status != READY && now - row.created_at < DRAFT_TTL_MS => {
                (row.request_id, row.created_at)
            }
            _ => (uuid::Uuid::new_v4().simple().to_string(), now),
        };
        let mut row = skill_drafts::Model {
            session_id: session_id.to_string(),
            status: WRITING.to_string(),
            request_id: request_id.clone(),
            draft_json: None,
            failure_json: None,
            created_at,
            updated_at: now,
        };
        self.repo.put_draft(row.clone()).await?;

        let outcome = match request {
            None => Err(SkillDraftFailure::new(SkillDraftFailureCode::DraftUnusable)),
            Some(mut request) => {
                request.request_id = request_id;
                let hosts = request.hosts();
                match call(request).await {
                    Ok(draft) => checks::checked(draft, &hosts).ok_or_else(|| {
                        SkillDraftFailure::new(SkillDraftFailureCode::DraftUnusable)
                    }),
                    Err(failure) => Err(failure),
                }
            }
        };
        match outcome {
            Ok(mut draft) => {
                draft.name = self.free_name(&draft.name).await?;
                row.status = READY.to_string();
                row.draft_json = serde_json::to_string(&draft).ok();
            }
            Err(failure) => {
                row.status = FAILED.to_string();
                row.failure_json = serde_json::to_string(&failure).ok();
            }
        }
        row.updated_at = crate::clock::now_epoch_ms();
        self.repo.put_draft(row.clone()).await?;
        Ok(status_of(Some(&row), None, now))
    }

    async fn lock_for(&self, session_id: &str) -> Arc<Mutex<()>> {
        self.writing
            .lock()
            .await
            .entry(session_id.to_string())
            .or_default()
            .clone()
    }

    /// The draft's name, or the first `-2`, `-3`, ... form of it no skill uses yet.
    async fn free_name(&self, name: &str) -> AppResult<String> {
        if self.repo.get(&neo_prefixed(name)).await?.is_none() {
            return Ok(name.to_string());
        }
        for suffix in 2..100 {
            let candidate = format!("{name}-{suffix}");
            if self.repo.get(&neo_prefixed(&candidate)).await?.is_none() {
                return Ok(candidate);
            }
        }
        Ok(format!("{name}-{}", uuid::Uuid::new_v4().simple()))
    }
}

/// The contract shape for a draft row, read at `now`.
fn status_of(
    row: Option<&skill_drafts::Model>,
    skill_name: Option<String>,
    now: i64,
) -> SkillDraftStatus {
    let fresh = row.filter(|row| now - row.created_at < DRAFT_TTL_MS);
    let draft = fresh
        .and_then(|row| row.draft_json.as_deref())
        .and_then(|json| serde_json::from_str::<SkillDraft>(json).ok());
    let mut status = if let Some(name) = skill_name {
        let mut status = SkillDraftStatus::new(SkillDraftState::Saved);
        status.skill_name = Some(name);
        status
    } else {
        match fresh {
            None => return SkillDraftStatus::new(SkillDraftState::None),
            Some(row) if row.status == READY && draft.is_some() => {
                SkillDraftStatus::new(SkillDraftState::Ready)
            }
            Some(row) if row.status == WRITING && now - row.updated_at < STALE_WRITING_MS => {
                SkillDraftStatus::new(SkillDraftState::Writing)
            }
            Some(row) => {
                let mut status = SkillDraftStatus::new(SkillDraftState::Failed);
                let failure = row
                    .failure_json
                    .as_deref()
                    .and_then(|json| serde_json::from_str::<SkillDraftFailure>(json).ok())
                    .unwrap_or_else(|| {
                        SkillDraftFailure::new(SkillDraftFailureCode::UpstreamError)
                    });
                status.failure = Some(Box::new(failure));
                status
            }
        }
    };
    if let Some(row) = fresh {
        status.draft_id = Some(row.request_id.clone());
        status.expires_at = Some(row.created_at + DRAFT_TTL_MS);
    }
    status.draft = draft.map(Box::new);
    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{DATABASE_FILENAME, Database};
    use claw_api::models::{
        SkillDraftConfidence, SkillDraftEffects, SkillDraftInput, SkillDraftInputKind,
        SkillDraftLimitScope,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    const NOW: i64 = 1_800_000_000_000;

    async fn service() -> anyhow::Result<(SkillDraftService, tempfile::TempDir)> {
        let dir = tempfile::tempdir()?;
        let db = Database::open(dir.path().join(DATABASE_FILENAME)).await?;
        Ok((SkillDraftService::new(SkillsRepository::new(db)), dir))
    }

    fn request() -> DraftRequest {
        DraftRequest {
            request_id: String::new(),
            goal: "Find phones under 30000 on Flipkart".into(),
            outcome: "done",
            site: "flipkart.com".into(),
            agent: "claude-code",
            steps: vec!["Opened Flipkart".into(), "Read Mobiles".into()],
            neo: None,
            pages: vec![],
            fields: vec![],
        }
    }

    fn written() -> SkillDraft {
        SkillDraft::new(
            "flipkart-phones-under-budget".into(),
            "Find phones under a budget on Flipkart".into(),
            "Lists popular phones under a price you choose.".into(),
            "flipkart.com".into(),
            vec![
                "Open flipkart.com and search for phones.".into(),
                "Set the price filter to a maximum of {budget}.".into(),
            ],
            vec![SkillDraftInput::new(
                "budget".into(),
                "Highest price".into(),
                "30000".into(),
                SkillDraftInputKind::Number,
            )],
            vec![],
            SkillDraftEffects::Read,
            SkillDraftConfidence::High,
        )
    }

    #[tokio::test]
    async fn concurrent_requests_for_one_task_share_one_draft() -> anyhow::Result<()> {
        let (service, _dir) = service().await?;
        let calls = AtomicUsize::new(0);
        let call = |sent: DraftRequest| {
            calls.fetch_add(1, Ordering::SeqCst);
            async move {
                assert_eq!(sent.request_id.len(), 32);
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                Ok(written())
            }
        };
        let (first, second) = tokio::join!(
            service.draft("sess", NOW, Some(request()), call),
            service.draft("sess", NOW, Some(request()), call),
        );
        let (first, second) = (first?, second?);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "the worker is asked once");
        assert_eq!(first.status, SkillDraftState::Ready);
        assert_eq!(first.draft_id, second.draft_id);
        assert_eq!(first.draft, second.draft);

        let later = service
            .draft("sess", NOW + 1000, Some(request()), call)
            .await?;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "a kept draft costs nothing"
        );
        assert_eq!(later.draft_id, first.draft_id);
        assert_eq!(later.expires_at, Some(NOW + DRAFT_TTL_MS));
        Ok(())
    }

    #[tokio::test]
    async fn a_failed_draft_is_retried_with_the_same_request_id() -> anyhow::Result<()> {
        let (service, _dir) = service().await?;
        let seen = std::sync::Mutex::new(Vec::new());
        let failed = service
            .draft("sess", NOW, Some(request()), |sent| {
                if let Ok(mut seen) = seen.lock() {
                    seen.push(sent.request_id);
                }
                async {
                    let mut failure = SkillDraftFailure::new(SkillDraftFailureCode::LimitReached);
                    failure.scope = Some(SkillDraftLimitScope::Day);
                    failure.resets_at = Some("2027-01-16T00:00:00.000Z".into());
                    Err(failure)
                }
            })
            .await?;
        assert_eq!(failed.status, SkillDraftState::Failed);
        let failure = failed
            .failure
            .ok_or_else(|| anyhow::anyhow!("no failure"))?;
        assert_eq!(failure.code, SkillDraftFailureCode::LimitReached);
        assert_eq!(failure.scope, Some(SkillDraftLimitScope::Day));

        let status = service.status("sess", None, NOW + 10).await?;
        assert_eq!(status.status, SkillDraftState::Failed, "every view sees it");

        let retried = service
            .draft("sess", NOW + 60_000, Some(request()), |sent| {
                if let Ok(mut seen) = seen.lock() {
                    seen.push(sent.request_id);
                }
                async { Ok(written()) }
            })
            .await?;
        assert_eq!(retried.status, SkillDraftState::Ready);
        let seen = seen.lock().map_err(|_| anyhow::anyhow!("poisoned"))?;
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], seen[1], "the worker counts a retry once");
        Ok(())
    }

    #[tokio::test]
    async fn an_expired_draft_is_written_again_under_a_new_id() -> anyhow::Result<()> {
        let (service, _dir) = service().await?;
        let first = service
            .draft("sess", NOW, Some(request()), |_| async { Ok(written()) })
            .await?;
        let expired = service.status("sess", None, NOW + DRAFT_TTL_MS + 1).await?;
        assert_eq!(expired.status, SkillDraftState::None);
        let again = service
            .draft("sess", NOW + DRAFT_TTL_MS + 1, Some(request()), |_| async {
                Ok(written())
            })
            .await?;
        assert_ne!(again.draft_id, first.draft_id);
        Ok(())
    }

    #[tokio::test]
    async fn an_unsafe_or_missing_draft_is_unusable() -> anyhow::Result<()> {
        let (service, _dir) = service().await?;
        let off_site = service
            .draft("a", NOW, Some(request()), |_| async {
                let mut draft = written();
                draft.site = "attacker.example".into();
                Ok(draft)
            })
            .await?;
        assert_eq!(
            off_site.failure.map(|failure| failure.code),
            Some(SkillDraftFailureCode::DraftUnusable)
        );
        let nothing = service
            .draft("b", NOW, None, |_| async { Ok(written()) })
            .await?;
        assert_eq!(
            nothing.failure.map(|failure| failure.code),
            Some(SkillDraftFailureCode::DraftUnusable)
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_saved_skill_wins_and_a_writing_row_goes_stale() -> anyhow::Result<()> {
        let (service, _dir) = service().await?;
        service
            .repo
            .put_draft(skill_drafts::Model {
                session_id: "sess".into(),
                status: WRITING.into(),
                request_id: "0123456789abcdef".into(),
                draft_json: None,
                failure_json: None,
                created_at: NOW,
                updated_at: NOW,
            })
            .await?;
        assert_eq!(
            service.status("sess", None, NOW + 1000).await?.status,
            SkillDraftState::Writing
        );
        assert_eq!(
            service
                .status("sess", None, NOW + STALE_WRITING_MS + 1)
                .await?
                .status,
            SkillDraftState::Failed
        );
        let saved = service
            .status("sess", Some("neo-flipkart".into()), NOW)
            .await?;
        assert_eq!(saved.status, SkillDraftState::Saved);
        assert_eq!(saved.skill_name.as_deref(), Some("neo-flipkart"));
        Ok(())
    }
}
