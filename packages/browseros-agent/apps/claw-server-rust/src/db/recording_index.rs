//! Durable recording index. Each newly accepted non-empty document batch commits stream metadata,
//! NDJSON payload, and its durable dedupe identity in one transaction.

use crate::{
    clock::now_epoch_ms,
    db::{
        Database,
        entities::{
            prelude::{
                RecordingBatches, RecordingPayloads, RecordingStreams, SessionTabs, TabClaims,
                TabRecordings,
            },
            recording_batches, recording_streams, session_tabs, tab_claims, tab_recordings,
        },
    },
    error::{AppError, AppResult},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DbBackend, EntityTrait,
    FromQueryResult, IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, Statement,
    TransactionTrait, sea_query::Expr,
};

#[derive(Clone)]
pub struct RecordingIndex {
    db: Database,
}

pub struct AppendDocumentBatch<'a> {
    pub document_id: &'a str,
    /// Chrome tab id permanently bound to the document by its first persisted non-empty batch.
    pub tab_id: i64,
    /// Best-effort target attribution: a later persisted batch may fill an initial absence but
    /// never replace a stored target id.
    pub target_id: Option<&'a str>,
    pub first_event_at: i64,
    pub last_event_at: i64,
    pub size_bytes: i64,
    pub event_count: i64,
    pub batch_id: &'a str,
    /// On a newly accepted non-empty batch, recorder gap evidence or malformed lines dropped by the
    /// server become sticky for the document stream; any replay selecting that stream is incomplete.
    pub has_gap: bool,
    /// New absolute committed length of the document's on-disk replay file after
    /// this batch's events were appended.
    pub payload_bytes: i64,
}

#[derive(Debug, Clone)]
pub struct RecordingStreamRow {
    pub document_id: String,
    pub tab_id: i64,
    pub target_id: Option<String>,
    pub first_event_at: i64,
    pub last_event_at: i64,
    pub size_bytes: i64,
    pub event_count: i64,
    pub has_gap: bool,
    pub payload_bytes: i64,
}

impl From<recording_streams::Model> for RecordingStreamRow {
    fn from(row: recording_streams::Model) -> Self {
        Self {
            document_id: row.document_id,
            tab_id: row.tab_id,
            target_id: row.target_id,
            first_event_at: row.first_event_at,
            last_event_at: row.last_event_at,
            size_bytes: row.size_bytes,
            event_count: row.event_count,
            has_gap: row.has_gap,
            payload_bytes: row.payload_bytes,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionTabWindow {
    pub tab_id: i64,
    pub claimed_at: i64,
    pub released_at: Option<i64>,
}

impl From<session_tabs::Model> for SessionTabWindow {
    fn from(row: session_tabs::Model) -> Self {
        Self {
            tab_id: row.tab_id,
            claimed_at: row.claimed_at,
            released_at: row.released_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LegacyRecordingRow {
    pub target_id: String,
    pub tab_id: i64,
    pub first_event_at: i64,
    pub last_event_at: i64,
    pub size_bytes: i64,
    pub event_count: i64,
}

impl From<tab_recordings::Model> for LegacyRecordingRow {
    fn from(row: tab_recordings::Model) -> Self {
        Self {
            target_id: row.target_id,
            tab_id: row.tab_id,
            first_event_at: row.first_event_at,
            last_event_at: row.last_event_at,
            size_bytes: row.size_bytes,
            event_count: row.event_count,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LegacyClaimRow {
    pub target_id: String,
    pub claimed_at: i64,
    pub released_at: Option<i64>,
}

impl From<tab_claims::Model> for LegacyClaimRow {
    fn from(row: tab_claims::Model) -> Self {
        Self {
            target_id: row.target_id,
            claimed_at: row.claimed_at,
            released_at: row.released_at,
        }
    }
}

#[derive(Debug, Clone, FromQueryResult)]
pub struct StreamMatchRow {
    pub document_id: String,
    pub tab_id: i64,
    pub target_id: Option<String>,
    pub first_event_at: i64,
    pub last_event_at: i64,
    pub size_bytes: i64,
    pub event_count: i64,
    pub has_gap: bool,
    pub claimed_at: i64,
    pub released_at: Option<i64>,
}

impl RecordingIndex {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Total bytes of rrweb recording payloads tracked in the stream index.
    pub async fn recording_bytes_total(&self) -> AppResult<i64> {
        let total = RecordingStreams::find()
            .select_only()
            .column_as(
                Expr::col(recording_streams::Column::SizeBytes).sum(),
                "total",
            )
            .into_tuple::<Option<i64>>()
            .one(self.db.connection())
            .await?
            .flatten();
        Ok(total.unwrap_or(0))
    }

    /// Whether this document has already durably accepted `batch_id`.
    pub async fn batch_accepted(&self, document_id: &str, batch_id: &str) -> AppResult<bool> {
        Ok(
            RecordingBatches::find_by_id((document_id.to_string(), batch_id.to_string()))
                .one(self.db.connection())
                .await?
                .is_some(),
        )
    }

    /// Committed byte length of the document's on-disk replay file (0 when the
    /// stream does not exist yet or predates the file migration).
    pub async fn committed_payload_bytes(&self, document_id: &str) -> AppResult<i64> {
        Ok(RecordingStreams::find_by_id(document_id.to_string())
            .one(self.db.connection())
            .await?
            .map_or(0, |row| row.payload_bytes))
    }

    /// The un-migrated document with the smallest DB-blob payload (smallest first,
    /// so extraction relieves the database quickly). Returns (document_id, bytes).
    pub async fn smallest_unmigrated_document(&self) -> AppResult<Option<(String, i64)>> {
        let row = self
            .db
            .connection()
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT document_id AS document_id, LENGTH(events_ndjson) AS sz \
                 FROM recording_payloads ORDER BY sz ASC LIMIT 1",
            ))
            .await?;
        match row {
            Some(row) => Ok(Some((
                row.try_get::<String>("", "document_id")?,
                row.try_get::<i64>("", "sz")?,
            ))),
            None => Ok(None),
        }
    }

    /// Number of documents whose payload still lives in the DB blob.
    pub async fn unmigrated_document_count(&self) -> AppResult<i64> {
        let row = self
            .db
            .connection()
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n FROM recording_payloads",
            ))
            .await?;
        Ok(row.map_or(0, |row| row.try_get::<i64>("", "n").unwrap_or(0)))
    }

    /// Marks a document's replay file authoritative in one transaction: sets the
    /// committed length and drops the now-redundant DB blob together. Atomic so a
    /// crash can never leave `payload_bytes` set while the blob survives, a state
    /// a later append plus background migration would resolve by truncating the
    /// appended events off the file.
    pub async fn finalize_extraction(&self, document_id: &str, bytes: i64) -> AppResult<()> {
        let txn = self.db.connection().begin().await?;
        async {
            RecordingStreams::update_many()
                .col_expr(recording_streams::Column::PayloadBytes, Expr::value(bytes))
                .filter(recording_streams::Column::DocumentId.eq(document_id))
                .exec(&txn)
                .await?;
            RecordingPayloads::delete_by_id(document_id.to_string())
                .exec(&txn)
                .await?;
            txn.commit().await?;
            Ok::<(), AppError>(())
        }
        .await
    }

    /// The oldest recording document, used to free space during disk-full recovery.
    pub async fn oldest_document_id(&self) -> AppResult<Option<String>> {
        Ok(RecordingStreams::find()
            .order_by_asc(recording_streams::Column::LastEventAt)
            .one(self.db.connection())
            .await?
            .map(|row| row.document_id))
    }

    /// Returns freed pages to the OS when the database is in incremental
    /// auto-vacuum mode; a no-op on a legacy database (which needs a full VACUUM).
    pub async fn incremental_reclaim(&self) -> AppResult<()> {
        self.db
            .connection()
            .execute(Statement::from_string(
                DbBackend::Sqlite,
                "PRAGMA incremental_vacuum",
            ))
            .await?;
        Ok(())
    }

    /// Seeds a pre-migration document (metadata with `payload_bytes = 0` plus a DB
    /// blob) to exercise the file migration in tests.
    #[cfg(test)]
    pub async fn seed_legacy_payload(
        &self,
        document_id: &str,
        tab_id: i64,
        events_ndjson: &str,
        first_event_at: i64,
        last_event_at: i64,
        event_count: i64,
    ) -> AppResult<()> {
        let conn = self.db.connection();
        conn.execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "INSERT INTO recording_streams (document_id, tab_id, target_id, first_event_at, \
             last_event_at, size_bytes, event_count, has_gap, payload_bytes) \
             VALUES (?, ?, NULL, ?, ?, ?, ?, 0, 0)",
            [
                document_id.to_string().into(),
                tab_id.into(),
                first_event_at.into(),
                last_event_at.into(),
                i64::try_from(events_ndjson.len())
                    .unwrap_or(i64::MAX)
                    .into(),
                event_count.into(),
            ],
        ))
        .await?;
        conn.execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "INSERT INTO recording_payloads (document_id, events_ndjson) VALUES (?, ?)",
            [
                document_id.to_string().into(),
                events_ndjson.to_string().into(),
            ],
        ))
        .await?;
        Ok(())
    }

    /// Records a batch's metadata after its events were appended to the document's
    /// on-disk replay file, where `input.payload_bytes` is the file's new committed
    /// length. Idempotent: an already-accepted batch is a no-op, so a duplicate
    /// append leaves only uncommitted trailing file bytes that the next append
    /// truncates away. Callers serialize per document.
    pub async fn commit_batch(&self, input: AppendDocumentBatch<'_>) -> AppResult<()> {
        let txn = self.db.connection().begin().await?;
        async {
            if RecordingBatches::find_by_id((
                input.document_id.to_string(),
                input.batch_id.to_string(),
            ))
            .one(&txn)
            .await?
            .is_some()
            {
                return Ok(());
            }
            if let Some(existing) = RecordingStreams::find_by_id(input.document_id.to_string())
                .one(&txn)
                .await?
            {
                if existing.tab_id != input.tab_id {
                    return Err(AppError::Internal(format!(
                        "recording document {} changed tab identity",
                        input.document_id
                    )));
                }
                let mut update = existing.into_active_model();
                if update.target_id.as_ref().is_none() && input.target_id.is_some() {
                    update.target_id = Set(input.target_id.map(str::to_string));
                }
                update.first_event_at =
                    Set(update.first_event_at.unwrap().min(input.first_event_at));
                update.last_event_at = Set(update.last_event_at.unwrap().max(input.last_event_at));
                update.size_bytes =
                    Set(update.size_bytes.unwrap().saturating_add(input.size_bytes));
                update.event_count = Set(update
                    .event_count
                    .unwrap()
                    .saturating_add(input.event_count));
                update.has_gap = Set(update.has_gap.unwrap() || input.has_gap);
                update.payload_bytes = Set(input.payload_bytes);
                update.update(&txn).await?;
            } else {
                RecordingStreams::insert(recording_streams::ActiveModel {
                    document_id: Set(input.document_id.to_string()),
                    tab_id: Set(input.tab_id),
                    target_id: Set(input.target_id.map(str::to_string)),
                    first_event_at: Set(input.first_event_at),
                    last_event_at: Set(input.last_event_at),
                    size_bytes: Set(input.size_bytes),
                    event_count: Set(input.event_count),
                    has_gap: Set(input.has_gap),
                    payload_bytes: Set(input.payload_bytes),
                })
                .exec(&txn)
                .await?;
            }
            RecordingBatches::insert(recording_batches::ActiveModel {
                document_id: Set(input.document_id.to_string()),
                batch_id: Set(input.batch_id.to_string()),
                accepted_at: Set(now_epoch_ms()),
            })
            .exec(&txn)
            .await?;
            txn.commit().await?;
            Ok::<(), AppError>(())
        }
        .await
    }

    pub async fn payload(&self, document_id: &str) -> AppResult<Option<String>> {
        Ok(RecordingPayloads::find_by_id(document_id.to_string())
            .one(self.db.connection())
            .await?
            .map(|row| row.events_ndjson))
    }

    pub async fn retention_snapshot(
        &self,
    ) -> AppResult<(Vec<SessionTabWindow>, Vec<RecordingStreamRow>)> {
        let claims = SessionTabs::find()
            .all(self.db.connection())
            .await?
            .into_iter()
            .map(SessionTabWindow::from)
            .collect();
        let streams = RecordingStreams::find()
            .all(self.db.connection())
            .await?
            .into_iter()
            .map(RecordingStreamRow::from)
            .collect();
        Ok((claims, streams))
    }

    /// Every live recording stream's document id, for the replay-file orphan
    /// sweep to distinguish tracked files from drift left by a crashed writer.
    pub async fn all_document_ids(&self) -> AppResult<Vec<String>> {
        Ok(RecordingStreams::find()
            .select_only()
            .column(recording_streams::Column::DocumentId)
            .into_tuple::<String>()
            .all(self.db.connection())
            .await?)
    }

    pub async fn legacy_recordings_before(
        &self,
        cutoff: i64,
    ) -> AppResult<Vec<LegacyRecordingRow>> {
        Ok(TabRecordings::find()
            .filter(tab_recordings::Column::LastEventAt.lt(cutoff))
            .all(self.db.connection())
            .await?
            .into_iter()
            .map(LegacyRecordingRow::from)
            .collect())
    }

    pub async fn delete_released_claims_before(&self, cutoff: i64) -> AppResult<u64> {
        let old_tab_claims = SessionTabs::find()
            .filter(session_tabs::Column::ReleasedAt.is_not_null())
            .filter(session_tabs::Column::ReleasedAt.lt(cutoff))
            .all(self.db.connection())
            .await?;
        let old_target_claims = TabClaims::find()
            .filter(tab_claims::Column::ReleasedAt.is_not_null())
            .filter(tab_claims::Column::ReleasedAt.lt(cutoff))
            .all(self.db.connection())
            .await?;
        SessionTabs::delete_many()
            .filter(session_tabs::Column::ReleasedAt.is_not_null())
            .filter(session_tabs::Column::ReleasedAt.lt(cutoff))
            .exec(self.db.connection())
            .await?;
        TabClaims::delete_many()
            .filter(tab_claims::Column::ReleasedAt.is_not_null())
            .filter(tab_claims::Column::ReleasedAt.lt(cutoff))
            .exec(self.db.connection())
            .await?;
        Ok(u64::try_from(old_tab_claims.len() + old_target_claims.len()).unwrap_or(u64::MAX))
    }

    pub async fn delete_document(&self, document_id: &str) -> AppResult<bool> {
        let Some(stream) = RecordingStreams::find_by_id(document_id.to_string())
            .one(self.db.connection())
            .await?
        else {
            return Ok(false);
        };
        RecordingStreams::delete_by_id(stream.document_id)
            .exec(self.db.connection())
            .await?;
        Ok(true)
    }

    pub async fn legacy_recording(&self, target_id: &str) -> AppResult<Option<LegacyRecordingRow>> {
        Ok(TabRecordings::find_by_id(target_id.to_string())
            .one(self.db.connection())
            .await?
            .map(LegacyRecordingRow::from))
    }

    pub async fn delete_legacy_recording(&self, target_id: &str) -> AppResult<()> {
        TabRecordings::delete_by_id(target_id.to_string())
            .exec(self.db.connection())
            .await?;
        Ok(())
    }

    pub async fn stream_matches(&self, session_id: &str) -> AppResult<Vec<StreamMatchRow>> {
        let statement = Statement::from_sql_and_values(
            DbBackend::Sqlite,
            r#"SELECT
                rs.document_id, rs.tab_id, rs.target_id,
                rs.first_event_at, rs.last_event_at, rs.size_bytes,
                rs.event_count, rs.has_gap,
                st.claimed_at, st.released_at
              FROM session_tabs st
              JOIN recording_streams rs
                ON rs.tab_id = st.tab_id
               AND rs.last_event_at >= st.claimed_at
               AND rs.first_event_at <= COALESCE(st.released_at, 9223372036854775807)
              WHERE st.session_id = ?
              ORDER BY rs.first_event_at"#,
            [session_id.into()],
        );
        Ok(StreamMatchRow::find_by_statement(statement)
            .all(self.db.connection())
            .await?)
    }

    pub async fn legacy_claims(&self, session_id: &str) -> AppResult<Vec<LegacyClaimRow>> {
        Ok(TabClaims::find()
            .filter(tab_claims::Column::SessionId.eq(session_id))
            .all(self.db.connection())
            .await?
            .into_iter()
            .map(LegacyClaimRow::from)
            .collect())
    }

    pub async fn legacy_recordings(&self) -> AppResult<Vec<LegacyRecordingRow>> {
        Ok(TabRecordings::find()
            .all(self.db.connection())
            .await?
            .into_iter()
            .map(LegacyRecordingRow::from)
            .collect())
    }

    #[cfg(test)]
    pub(crate) async fn insert_session_tab(
        &self,
        session_id: &str,
        agent_id: &str,
        tab_id: i64,
        opened_target_id: Option<&str>,
        claimed_at: i64,
        released_at: Option<i64>,
    ) -> AppResult<()> {
        use sea_orm::ActiveValue::{NotSet, Set};

        SessionTabs::insert(session_tabs::ActiveModel {
            id: NotSet,
            session_id: Set(session_id.to_string()),
            agent_id: Set(agent_id.to_string()),
            tab_id: Set(tab_id),
            opened_target_id: Set(opened_target_id.map(str::to_string)),
            claimed_at: Set(claimed_at),
            released_at: Set(released_at),
        })
        .exec(self.db.connection())
        .await?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn stream(&self, document_id: &str) -> AppResult<Option<RecordingStreamRow>> {
        Ok(RecordingStreams::find_by_id(document_id.to_string())
            .one(self.db.connection())
            .await?
            .map(RecordingStreamRow::from))
    }

    #[cfg(test)]
    pub(crate) async fn stream_count(&self) -> AppResult<usize> {
        Ok(RecordingStreams::find()
            .all(self.db.connection())
            .await?
            .len())
    }

    #[cfg(test)]
    pub(crate) async fn batch_exists(&self, document_id: &str, batch_id: &str) -> AppResult<bool> {
        Ok(
            RecordingBatches::find_by_id((document_id.to_string(), batch_id.to_string()))
                .one(self.db.connection())
                .await?
                .is_some(),
        )
    }

    #[cfg(test)]
    pub(crate) async fn payload_exists(&self, document_id: &str) -> AppResult<bool> {
        Ok(RecordingPayloads::find_by_id(document_id.to_string())
            .one(self.db.connection())
            .await?
            .is_some())
    }
}
