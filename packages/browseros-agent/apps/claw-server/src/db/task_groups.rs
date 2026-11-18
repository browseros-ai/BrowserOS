use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, Value};

use crate::{db::Database, error::AppResult};

/// Which client model opened a task's tab group.
///
/// The distinction is load bearing: a CLI task's lifetime follows its group, while an MCP
/// session keeps the lifetime it has always had. Every behaviour that reads this column
/// exists so the MCP paths stay exactly as they were.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskOrigin {
    /// A `browseros` command line. The group is the task's only identity.
    Cli,
    /// An MCP client. The binding is recorded for correlation and nothing else.
    Mcp,
}

impl TaskOrigin {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Mcp => "mcp",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "cli" => Some(Self::Cli),
            "mcp" => Some(Self::Mcp),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_cli(self) -> bool {
        self == Self::Cli
    }
}

/// A live or ended binding between a tab group and the task that owns it.
#[derive(Debug, Clone, PartialEq, Eq, FromQueryResult)]
pub struct Binding {
    pub group_id: String,
    pub session_id: String,
    pub agent_id: String,
    pub origin: String,
    pub window_id: Option<String>,
    pub profile_id: Option<String>,
    pub created_at: i64,
    pub ended_at: Option<i64>,
    /// The task's name, kept here because a session's name lives only in memory and the
    /// task outlives the process.
    pub name: Option<String>,
}

impl Binding {
    #[must_use]
    pub fn origin(&self) -> Option<TaskOrigin> {
        TaskOrigin::parse(&self.origin)
    }

    #[must_use]
    pub fn is_cli(&self) -> bool {
        self.origin().is_some_and(TaskOrigin::is_cli)
    }
}

/// What happened when a group was offered to a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindOutcome {
    /// The group had no live owner, so this task took it.
    Bound,
    /// The group is already owned by a live task, named here. The caller decides whether
    /// that is an adoption or a refusal; this store never silently reassigns one.
    AlreadyBound { session_id: String },
}

/// A task asking to own a tab group.
#[derive(Debug, Clone, Copy)]
pub struct NewBinding<'a> {
    pub group_id: &'a str,
    pub session_id: &'a str,
    pub agent_id: &'a str,
    pub origin: TaskOrigin,
    pub window_id: Option<&'a str>,
    pub profile_id: Option<&'a str>,
}

/// The durable `group -> task` index.
#[derive(Clone)]
pub struct TaskGroupStore {
    db: Database,
}

impl TaskGroupStore {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Records `session_id` as the owner of `group_id`.
    ///
    /// A group whose previous task has ended is taken over, which is what makes a closed
    /// task's group reusable for new work. A group with a live owner is left alone and
    /// reported back, so two tasks can never both believe they own one group.
    pub async fn bind(&self, request: NewBinding<'_>, now_ms: i64) -> AppResult<BindOutcome> {
        let NewBinding {
            group_id,
            session_id,
            agent_id,
            origin,
            window_id,
            profile_id,
        } = request;
        let bound = self
            .db
            .connection()
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "INSERT INTO task_groups \
                 (group_id, session_id, agent_id, origin, window_id, profile_id, created_at, ended_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, NULL) \
                 ON CONFLICT(group_id) DO UPDATE SET \
                 session_id = excluded.session_id, agent_id = excluded.agent_id, \
                 origin = excluded.origin, window_id = excluded.window_id, \
                 profile_id = excluded.profile_id, created_at = excluded.created_at, \
                 ended_at = NULL, name = NULL \
                 WHERE task_groups.ended_at IS NOT NULL",
                [
                    Value::from(group_id),
                    Value::from(session_id),
                    Value::from(agent_id),
                    Value::from(origin.as_str()),
                    Value::from(window_id.map(str::to_string)),
                    Value::from(profile_id.map(str::to_string)),
                    Value::from(now_ms),
                ],
            ))
            .await?
            .rows_affected();

        if bound > 0 {
            return Ok(BindOutcome::Bound);
        }

        // Either a live owner blocked the upsert, or the row vanished between the two
        // statements. Reading it back is what tells those apart.
        match self.lookup(group_id).await? {
            Some(existing) if existing.ended_at.is_none() => {
                if existing.session_id == session_id {
                    Ok(BindOutcome::Bound)
                } else {
                    Ok(BindOutcome::AlreadyBound {
                        session_id: existing.session_id,
                    })
                }
            }
            _ => Ok(BindOutcome::Bound),
        }
    }

    /// The binding for `group_id`, ended or not.
    pub async fn lookup(&self, group_id: &str) -> AppResult<Option<Binding>> {
        Ok(Binding::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT group_id, session_id, agent_id, origin, window_id, profile_id, created_at, ended_at, name \
             FROM task_groups WHERE group_id = ?",
            [Value::from(group_id)],
        ))
        .one(self.db.connection())
        .await?)
    }

    /// The binding for `group_id` only while its task is still open.
    ///
    /// An ended binding reads as absent on purpose: reviving a closed task would corrupt
    /// the duration and status already written for it, so the next command in that group
    /// starts new work instead.
    pub async fn lookup_live(&self, group_id: &str) -> AppResult<Option<Binding>> {
        Ok(self
            .lookup(group_id)
            .await?
            .filter(|binding| binding.ended_at.is_none()))
    }

    /// Ends the binding for `group_id`, if one is live. True when this call ended it.
    pub async fn end(&self, group_id: &str, now_ms: i64) -> AppResult<bool> {
        let ended = self
            .db
            .connection()
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "UPDATE task_groups SET ended_at = ? WHERE group_id = ? AND ended_at IS NULL",
                [Value::from(now_ms), Value::from(group_id)],
            ))
            .await?
            .rows_affected();
        Ok(ended > 0)
    }

    /// Every group a task still owns.
    pub async fn live_groups_for_session(&self, session_id: &str) -> AppResult<Vec<Binding>> {
        Ok(Binding::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT group_id, session_id, agent_id, origin, window_id, profile_id, created_at, ended_at, name \
             FROM task_groups WHERE session_id = ? AND ended_at IS NULL ORDER BY created_at",
            [Value::from(session_id)],
        ))
        .all(self.db.connection())
        .await?)
    }

    /// Records the name of the task `session_id` is, on every group it still owns.
    pub async fn set_name_for_session(&self, session_id: &str, name: &str) -> AppResult<u64> {
        Ok(self
            .db
            .connection()
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "UPDATE task_groups SET name = ? WHERE session_id = ? AND ended_at IS NULL",
                [Value::from(name), Value::from(session_id)],
            ))
            .await?
            .rows_affected())
    }

    /// Every live CLI binding, for the sweep that notices a group the user closed.
    pub async fn live_cli_bindings(&self) -> AppResult<Vec<Binding>> {
        Ok(Binding::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT group_id, session_id, agent_id, origin, window_id, profile_id, created_at, ended_at, name \
             FROM task_groups WHERE ended_at IS NULL AND origin = ? ORDER BY created_at",
            [Value::from(TaskOrigin::Cli.as_str())],
        ))
        .all(self.db.connection())
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const T0: i64 = 1_789_560_000_000;

    fn cli(group_id: &'static str, session_id: &'static str) -> NewBinding<'static> {
        NewBinding {
            group_id,
            session_id,
            agent_id: "agent",
            origin: TaskOrigin::Cli,
            window_id: None,
            profile_id: None,
        }
    }

    async fn live(store: &TaskGroupStore, group_id: &str) -> anyhow::Result<Binding> {
        store
            .lookup_live(group_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("{group_id} has no live binding"))
    }

    async fn store(dir: &std::path::Path) -> anyhow::Result<TaskGroupStore> {
        let db = Database::open(dir.join("browserclaw.sqlite")).await?;
        Ok(TaskGroupStore::new(db))
    }

    #[tokio::test]
    async fn a_free_group_binds_and_reads_back_live() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = store(dir.path()).await?;

        let outcome = store
            .bind(
                NewBinding {
                    window_id: Some("w1"),
                    ..cli("G1", "s1")
                },
                T0,
            )
            .await?;
        assert_eq!(outcome, BindOutcome::Bound);

        let binding = live(&store, "G1").await?;
        assert_eq!(binding.session_id, "s1");
        assert_eq!(binding.window_id.as_deref(), Some("w1"));
        assert!(binding.is_cli());
        Ok(())
    }

    #[tokio::test]
    async fn a_live_group_is_never_reassigned_to_another_task() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = store(dir.path()).await?;
        store.bind(cli("G1", "s1"), T0).await?;

        let outcome = store.bind(cli("G1", "s2"), T0 + 1).await?;
        assert_eq!(
            outcome,
            BindOutcome::AlreadyBound {
                session_id: "s1".to_string()
            }
        );
        let binding = live(&store, "G1").await?;
        assert_eq!(binding.session_id, "s1", "the first owner keeps the group");
        Ok(())
    }

    #[tokio::test]
    async fn rebinding_the_same_task_is_idempotent() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = store(dir.path()).await?;
        store.bind(cli("G1", "s1"), T0).await?;
        let again = store.bind(cli("G1", "s1"), T0 + 5).await?;
        assert_eq!(again, BindOutcome::Bound);
        Ok(())
    }

    #[tokio::test]
    async fn an_ended_binding_reads_as_absent_and_frees_the_group() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = store(dir.path()).await?;
        store.bind(cli("G1", "s1"), T0).await?;

        assert!(store.end("G1", T0 + 10).await?);
        assert!(!store.end("G1", T0 + 11).await?, "ending twice is a no-op");
        assert!(store.lookup_live("G1").await?.is_none());
        assert_eq!(
            store
                .lookup("G1")
                .await?
                .and_then(|binding| binding.ended_at),
            Some(T0 + 10),
            "the ended row is kept for the record"
        );

        let outcome = store.bind(cli("G1", "s2"), T0 + 20).await?;
        assert_eq!(
            outcome,
            BindOutcome::Bound,
            "new work takes over a closed group"
        );
        let binding = live(&store, "G1").await?;
        assert_eq!(binding.session_id, "s2");
        assert_eq!(binding.created_at, T0 + 20);
        Ok(())
    }

    #[tokio::test]
    async fn live_listings_separate_sessions_and_origins() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = store(dir.path()).await?;
        store.bind(cli("G1", "s1"), T0).await?;
        store.bind(cli("G2", "s1"), T0 + 1).await?;
        store
            .bind(
                NewBinding {
                    origin: TaskOrigin::Mcp,
                    ..cli("G3", "s3")
                },
                T0 + 2,
            )
            .await?;
        store.end("G2", T0 + 3).await?;

        let for_s1: Vec<_> = store
            .live_groups_for_session("s1")
            .await?
            .into_iter()
            .map(|binding| binding.group_id)
            .collect();
        assert_eq!(for_s1, vec!["G1".to_string()]);

        let cli: Vec<_> = store
            .live_cli_bindings()
            .await?
            .into_iter()
            .map(|binding| binding.group_id)
            .collect();
        assert_eq!(cli, vec!["G1".to_string()], "an MCP binding is never swept");
        Ok(())
    }

    #[tokio::test]
    async fn a_tasks_name_is_kept_on_its_open_groups_and_dropped_when_rebound() -> anyhow::Result<()>
    {
        let dir = tempdir()?;
        let store = store(dir.path()).await?;
        store.bind(cli("G1", "s1"), T0).await?;
        store.bind(cli("G2", "s1"), T0).await?;

        assert_eq!(store.set_name_for_session("s1", "price check").await?, 2);
        assert_eq!(
            live(&store, "G2").await?.name.as_deref(),
            Some("price check")
        );

        store.end("G1", T0 + 1).await?;
        store.bind(cli("G1", "s2"), T0 + 2).await?;
        assert_eq!(
            live(&store, "G1").await?.name,
            None,
            "new work in a closed task's group starts unnamed"
        );
        Ok(())
    }
}
