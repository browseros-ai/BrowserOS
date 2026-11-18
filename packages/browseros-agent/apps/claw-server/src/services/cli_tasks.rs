//! A CLI task's lifetime follows its tab group: the task ends when its last group does.
//!
//! The decided model is user-driven. The agent creates a group when the user gives it a
//! task and closes it when the user says the task is done, and a user closing those tabs
//! by hand ends the task too. Only CLI bindings are ever acted on here, so an MCP session's
//! lifetime is exactly what it was.

use std::{collections::HashSet, time::Duration};

use tracing::warn;

use crate::{
    db::{Binding, TaskGroupStore},
    error::AppResult,
    ids::SessionId,
    services::{browser::BrowserService, sessions::Sessions},
};

/// Why a task ended, as the end row records it. Both read as done in history.
pub const REASON_TASK_DONE: &str = "task done";
pub const REASON_GROUP_CLOSED: &str = "group closed";

/// Ends the CLI task that owns `binding`'s group, if this call is the one that ends it.
///
/// A task can own more than one group, so it ends with its last. The binding is ended
/// first and the session after, so a command racing this one cannot resolve the task from
/// a group that is already closing.
pub async fn end_task_for_group(
    sessions: &Sessions,
    task_groups: &TaskGroupStore,
    binding: &Binding,
    reason: &str,
    now_ms: i64,
) -> AppResult<bool> {
    if !binding.is_cli() {
        return Ok(false);
    }
    if !task_groups.end(&binding.group_id, now_ms).await? {
        return Ok(false);
    }
    if !task_groups
        .live_groups_for_session(&binding.session_id)
        .await?
        .is_empty()
    {
        return Ok(true);
    }
    let session_id = SessionId::new(binding.session_id.clone());
    if sessions.lookup(&session_id).await.is_some() {
        sessions.remove(&session_id, "closed", Some(reason)).await?;
    }
    Ok(true)
}

/// The live CLI bindings whose group the browser no longer has.
///
/// A binding younger than `grace` is left for the next pass, so a sweep racing the
/// creation of a group cannot end the task that is just starting in it.
fn closed_groups<'a>(
    bindings: &'a [Binding],
    live_groups: &HashSet<&str>,
    now_ms: i64,
    grace: Duration,
) -> Vec<&'a Binding> {
    let grace_ms = i64::try_from(grace.as_millis()).unwrap_or(i64::MAX);
    bindings
        .iter()
        .filter(|binding| binding.is_cli())
        .filter(|binding| !live_groups.contains(binding.group_id.as_str()))
        .filter(|binding| now_ms.saturating_sub(binding.created_at) >= grace_ms)
        .collect()
}

/// Ends every CLI task whose group was closed outside the CLI, returning how many ended.
///
/// Groups are read from the native tab listing, which omits extension and devtools pages,
/// so a group holding only such a page reads as closed. An unreachable browser or a failed
/// listing ends nothing: a task is never ended because the browser could not be asked.
pub async fn sweep_closed_groups(
    sessions: &Sessions,
    task_groups: &TaskGroupStore,
    browser: &BrowserService,
    now_ms: i64,
    grace: Duration,
) -> AppResult<usize> {
    let bindings = task_groups.live_cli_bindings().await?;
    if bindings.is_empty() {
        return Ok(0);
    }
    let Some(session) = browser.session().await else {
        return Ok(0);
    };
    let pages = match session.pages.list().await {
        Ok(pages) => pages,
        Err(error) => {
            warn!(error = %error, "CLI task sweep could not list tabs");
            return Ok(0);
        }
    };
    let live_groups: HashSet<&str> = pages
        .iter()
        .filter_map(|page| page.group_id.as_deref())
        .collect();
    let mut ended = 0;
    for binding in closed_groups(&bindings, &live_groups, now_ms, grace) {
        if end_task_for_group(sessions, task_groups, binding, REASON_GROUP_CLOSED, now_ms).await? {
            ended += 1;
        }
    }
    Ok(ended)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::{AuditLog, Database, NewBinding, SessionTabLedger, TaskOrigin},
        identity::{ClientIdentity, ClientInfo},
        services::sessions::RetirementCause,
    };
    use std::sync::Arc;
    use tempfile::tempdir;

    const T0: i64 = 1_789_560_000_000;

    struct Fixture {
        sessions: Arc<Sessions>,
        task_groups: TaskGroupStore,
        _dir: tempfile::TempDir,
    }

    async fn fixture() -> anyhow::Result<Fixture> {
        let dir = tempdir()?;
        let db = Database::open(dir.path().join("browserclaw.sqlite")).await?;
        let sessions = Sessions::new(
            Arc::new(AuditLog::new(db.clone())),
            Arc::new(SessionTabLedger::new(db.clone())),
            Duration::from_secs(1800),
            Duration::from_secs(300),
            Duration::from_secs(3600),
            Duration::from_secs(60),
        );
        Ok(Fixture {
            sessions,
            task_groups: TaskGroupStore::new(db),
            _dir: dir,
        })
    }

    async fn cli_session(fixture: &Fixture) -> anyhow::Result<SessionId> {
        let session = fixture
            .sessions
            .mint(
                ClientIdentity::Ephemeral {
                    slug: "browseros".to_string(),
                    label: "browseros".to_string(),
                },
                ClientInfo {
                    name: "browseros-cli".to_string(),
                    version: "0.0.1".to_string(),
                    title: None,
                },
            )
            .await?;
        Ok(session.id().clone())
    }

    async fn bind(
        fixture: &Fixture,
        group_id: &str,
        session_id: &SessionId,
        origin: TaskOrigin,
        at: i64,
    ) -> anyhow::Result<Binding> {
        fixture
            .task_groups
            .bind(
                NewBinding {
                    group_id,
                    session_id: session_id.as_str(),
                    agent_id: "agent",
                    origin,
                    window_id: None,
                    profile_id: None,
                },
                at,
            )
            .await?;
        fixture
            .task_groups
            .lookup(group_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("{group_id} was not bound"))
    }

    #[tokio::test]
    async fn closing_a_cli_tasks_group_ends_the_task_as_done() -> anyhow::Result<()> {
        let fixture = fixture().await?;
        let id = cli_session(&fixture).await?;
        let binding = bind(&fixture, "G1", &id, TaskOrigin::Cli, T0).await?;

        let ended = end_task_for_group(
            &fixture.sessions,
            &fixture.task_groups,
            &binding,
            REASON_TASK_DONE,
            T0 + 1,
        )
        .await?;

        assert!(ended);
        assert!(fixture.sessions.lookup(&id).await.is_none());
        assert!(matches!(
            fixture.sessions.retirement_of(&id).await,
            Some(RetirementCause::Closed)
        ));
        assert!(fixture.task_groups.lookup_live("G1").await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn a_task_with_another_open_group_lives_on() -> anyhow::Result<()> {
        let fixture = fixture().await?;
        let id = cli_session(&fixture).await?;
        let first = bind(&fixture, "G1", &id, TaskOrigin::Cli, T0).await?;
        bind(&fixture, "G2", &id, TaskOrigin::Cli, T0).await?;

        assert!(
            end_task_for_group(
                &fixture.sessions,
                &fixture.task_groups,
                &first,
                REASON_GROUP_CLOSED,
                T0 + 1,
            )
            .await?
        );
        assert!(
            fixture.sessions.lookup(&id).await.is_some(),
            "the task still owns G2"
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_mcp_binding_never_ends_its_session() -> anyhow::Result<()> {
        let fixture = fixture().await?;
        let id = cli_session(&fixture).await?;
        let binding = bind(&fixture, "G1", &id, TaskOrigin::Mcp, T0).await?;

        let ended = end_task_for_group(
            &fixture.sessions,
            &fixture.task_groups,
            &binding,
            REASON_GROUP_CLOSED,
            T0 + 1,
        )
        .await?;

        assert!(!ended);
        assert!(fixture.sessions.lookup(&id).await.is_some());
        assert!(fixture.task_groups.lookup_live("G1").await?.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn ending_twice_is_a_no_op() -> anyhow::Result<()> {
        let fixture = fixture().await?;
        let id = cli_session(&fixture).await?;
        let binding = bind(&fixture, "G1", &id, TaskOrigin::Cli, T0).await?;
        let end = || {
            end_task_for_group(
                &fixture.sessions,
                &fixture.task_groups,
                &binding,
                REASON_TASK_DONE,
                T0 + 1,
            )
        };
        assert!(end().await?);
        assert!(!end().await?);
        Ok(())
    }

    fn binding(group_id: &str, origin: TaskOrigin, created_at: i64) -> Binding {
        Binding {
            group_id: group_id.to_string(),
            session_id: "s".to_string(),
            agent_id: "a".to_string(),
            origin: origin.as_str().to_string(),
            window_id: None,
            profile_id: None,
            created_at,
            ended_at: None,
        }
    }

    #[test]
    fn only_a_vanished_cli_group_past_its_grace_reads_as_closed() {
        let bindings = vec![
            binding("open", TaskOrigin::Cli, T0),
            binding("gone", TaskOrigin::Cli, T0),
            binding("just-made", TaskOrigin::Cli, T0 + 55_000),
            binding("mcp-gone", TaskOrigin::Mcp, T0),
        ];
        let live = HashSet::from(["open"]);

        let closed: Vec<_> = closed_groups(&bindings, &live, T0 + 60_000, Duration::from_secs(10))
            .into_iter()
            .map(|binding| binding.group_id.as_str())
            .collect();

        assert_eq!(closed, vec!["gone"]);
    }
}
