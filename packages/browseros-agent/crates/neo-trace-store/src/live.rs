//! Session change signals for feeds. Only the writer publishes them, after commit; readers
//! seeing a new version can immediately read its rows through the independent WAL read pool.

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

use neo_runtime::{CallId, SessionKey};
use sea_orm::{
    ColumnTrait, DatabaseConnection, DbErr, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
};
use tokio::sync::watch;

use crate::db::entities::{trace_calls, trace_runs, trace_sessions};
use crate::db::unsigned;
use crate::{LastCall, SessionHead};

/// The mutex protects watch subscription/registration only, never a runtime sink or emit.
#[derive(Default)]
pub(crate) struct Heads(Mutex<HashMap<SessionKey, watch::Sender<SessionHead>>>);

impl Heads {
    pub(crate) fn watch(&self, key: &SessionKey) -> watch::Receiver<SessionHead> {
        let Ok(mut heads) = self.0.lock() else {
            return watch::channel(SessionHead::default()).1;
        };
        heads
            .entry(key.clone())
            .or_insert_with(|| watch::channel(SessionHead::default()).0)
            .subscribe()
    }

    pub(crate) async fn initialize(&self, db: &DatabaseConnection, now: u64) -> Result<(), DbErr> {
        let sessions = trace_sessions::Entity::find().all(db).await?;
        let mut keys: BTreeSet<String> = sessions
            .into_iter()
            .map(|session| session.session_key)
            .collect();
        if let Ok(heads) = self.0.lock() {
            keys.extend(heads.keys().map(|key| key.as_str().to_string()));
        }
        for key in keys {
            self.publish(db, &key, now).await?;
        }
        if let Ok(mut heads) = self.0.lock() {
            heads.retain(|_, sender| sender.receiver_count() > 0 || sender.borrow().runs > 0);
        }
        Ok(())
    }

    pub(crate) async fn publish(
        &self,
        db: &DatabaseConnection,
        key: &str,
        now: u64,
    ) -> Result<(), DbErr> {
        let runs = trace_runs::Entity::find().filter(trace_runs::Column::SessionKey.eq(key));
        let count = runs.clone().count(db).await?;
        let live = runs
            .clone()
            .filter(trace_runs::Column::Status.is_null())
            .count(db)
            .await?;
        let unsettled = runs
            .clone()
            .filter(trace_runs::Column::SettledAtMs.is_null())
            .count(db)
            .await?;
        let last = runs
            .order_by_desc(trace_runs::Column::RunId)
            .one(db)
            .await?;
        let last_run = last
            .as_ref()
            .map(|run| run.run_id.parse())
            .transpose()
            .map_err(|error| DbErr::Custom(format!("invalid stored run id: {error}")))?;
        let last_call = if let Some(last) = last {
            trace_calls::Entity::find()
                .filter(trace_calls::Column::RunId.eq(last.run_id))
                .order_by_desc(trace_calls::Column::CallNo)
                .one(db)
                .await?
                .map(|call| {
                    call.run_id.parse().map(|run| LastCall {
                        id: CallId {
                            run,
                            no: unsigned(call.call_no),
                        },
                        state: call.state.view(),
                    })
                })
                .transpose()
                .map_err(|error| DbErr::Custom(format!("invalid stored call id: {error}")))?
        } else {
            None
        };
        let mut heads = self
            .0
            .lock()
            .map_err(|_| DbErr::Custom("trace heads poisoned".into()))?;
        let sender = heads
            .entry(SessionKey::new(key))
            .or_insert_with(|| watch::channel(SessionHead::default()).0);
        let version = sender.borrow().version.saturating_add(1);
        sender.send_replace(SessionHead {
            version,
            runs: count,
            live_runs: live,
            unsettled_runs: unsettled,
            last_run,
            last_call,
            changed_at_ms: now,
        });
        Ok(())
    }
}
