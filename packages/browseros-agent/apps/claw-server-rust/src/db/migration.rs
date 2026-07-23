use sea_orm_migration::prelude::*;

/// Applies the shared BrowserClaw database schema migrations.
pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m0001_baseline::Migration),
            Box::new(m0002_add_recordings_and_claims::Migration),
            Box::new(m0003_document_recordings_and_tab_ownership::Migration),
            Box::new(m0004_atomic_recording_payloads::Migration),
            Box::new(m0005_reclassify_task_status::Migration),
            Box::new(m0006_add_tool_token_estimates::Migration),
            Box::new(m0007_add_session_efficiency_stats::Migration),
            Box::new(m0008_add_task_token_estimates::Migration),
            Box::new(m0009_rebase_screenshot_baseline::Migration),
            Box::new(m0010_sum_session_efficiency_durations::Migration),
        ]
    }
}

mod m0009_rebase_screenshot_baseline {
    use super::*;

    pub struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &str {
            "m0009_rebase_screenshot_baseline"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            // These formula versions and token baselines are frozen migration values, not
            // mutable runtime constants. The cap mirrors the projection's saturating multiply.
            manager
                .get_connection()
                .execute_unprepared(
                    r#"
                    UPDATE session_efficiency_stats SET
                        screenshot_baseline_token_estimate = CASE
                            WHEN dispatch_count <= 0 THEN 0
                            WHEN dispatch_count > 3074457345618258 THEN 9223372036854775807
                            ELSE dispatch_count * 3000
                        END,
                        efficiency_estimator_version = 2
                    WHERE efficiency_estimator_version = 1
                    "#,
                )
                .await?;
            Ok(())
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .get_connection()
                .execute_unprepared(
                    r#"
                    UPDATE session_efficiency_stats SET
                        screenshot_baseline_token_estimate = CASE
                            WHEN dispatch_count <= 0 THEN 0
                            WHEN dispatch_count > 6004799503160661 THEN 9223372036854775807
                            ELSE dispatch_count * 1536
                        END,
                        efficiency_estimator_version = 1
                    WHERE efficiency_estimator_version = 2
                    "#,
                )
                .await?;
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use sea_orm_migration::sea_orm::{
            ConnectionTrait, Database as SeaDatabase, DbBackend, Statement,
        };

        struct PreviousMigrator;

        #[async_trait::async_trait]
        impl MigratorTrait for PreviousMigrator {
            fn migrations() -> Vec<Box<dyn MigrationTrait>> {
                super::super::Migrator::migrations()
                    .into_iter()
                    .take(8)
                    .collect()
            }
        }

        async fn insert_projection(
            connection: &sea_orm_migration::sea_orm::DatabaseConnection,
            session_id: &str,
            dispatch_count: i64,
            screenshot_tokens: i64,
            version: i64,
        ) -> Result<(), DbErr> {
            connection
                .execute_unprepared(&format!(
                    "INSERT INTO session_efficiency_stats (session_id, ended_at, dispatch_count, active_duration_ms, tool_input_token_estimate, tool_output_token_estimate, screenshot_baseline_token_estimate, efficiency_estimator_version, computed_at) VALUES ('{session_id}', 1, {dispatch_count}, 1, 1, 1, {screenshot_tokens}, {version}, 1)"
                ))
                .await?;
            Ok(())
        }

        async fn projection(
            connection: &sea_orm_migration::sea_orm::DatabaseConnection,
            session_id: &str,
        ) -> Result<(i64, i64), DbErr> {
            let row = connection
                .query_one(Statement::from_string(
                    DbBackend::Sqlite,
                    format!(
                        "SELECT screenshot_baseline_token_estimate, efficiency_estimator_version FROM session_efficiency_stats WHERE session_id = '{session_id}'"
                    ),
                ))
                .await?
                .ok_or_else(|| DbErr::RecordNotFound(session_id.to_owned()))?;
            Ok((
                row.try_get("", "screenshot_baseline_token_estimate")?,
                row.try_get("", "efficiency_estimator_version")?,
            ))
        }

        #[tokio::test]
        async fn backfill_rebases_only_v1_projections_and_saturates() -> anyhow::Result<()> {
            let connection = SeaDatabase::connect("sqlite::memory:").await?;
            PreviousMigrator::up(&connection, None).await?;
            insert_projection(&connection, "v1", 2, 3_072, 1).await?;
            insert_projection(&connection, "v2", 2, 6_000, 2).await?;
            insert_projection(&connection, "saturated", i64::MAX, i64::MAX, 1).await?;

            super::super::Migrator::up(&connection, None).await?;

            assert_eq!(projection(&connection, "v1").await?, (6_000, 2));
            assert_eq!(projection(&connection, "v2").await?, (6_000, 2));
            assert_eq!(projection(&connection, "saturated").await?, (i64::MAX, 2));
            Ok(())
        }
    }
}

mod m0010_sum_session_efficiency_durations {
    use super::*;
    use sea_orm_migration::sea_orm::{DbBackend, Statement};
    use std::collections::BTreeMap;

    const SOURCE_EFFICIENCY_ESTIMATOR_VERSION: i64 = 2;
    const EFFICIENCY_ESTIMATOR_VERSION: i64 = 3;

    pub struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &str {
            "m0010_sum_session_efficiency_durations"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            let connection = manager.get_connection();
            let source_rows = connection
                .query_all(Statement::from_sql_and_values(
                    DbBackend::Sqlite,
                    r#"
                    SELECT stats.session_id, dispatch.duration_ms
                    FROM session_efficiency_stats AS stats
                    JOIN tool_dispatches AS dispatch
                        ON dispatch.session_id = stats.session_id
                        AND dispatch.created_at <= stats.ended_at
                    WHERE stats.efficiency_estimator_version = ?
                        AND stats.dispatch_count = (
                            SELECT COUNT(*)
                            FROM tool_dispatches AS retained
                            WHERE retained.session_id = stats.session_id
                                AND retained.created_at <= stats.ended_at
                        )
                    "#,
                    [SOURCE_EFFICIENCY_ESTIMATOR_VERSION.into()],
                ))
                .await?;
            let mut duration_sums = BTreeMap::<String, i64>::new();
            for row in source_rows {
                let session_id = row.try_get("", "session_id")?;
                let duration_ms = row
                    .try_get::<Option<i64>>("", "duration_ms")?
                    .unwrap_or_default()
                    .max(0);
                let sum = duration_sums.entry(session_id).or_default();
                *sum = sum.saturating_add(duration_ms);
            }

            // Retention can remove a v2 source, and session IDs can be reused after it ends.
            // Reproject only when the complete dispatch set from the original session remains.
            for (session_id, active_duration_ms) in duration_sums {
                connection
                    .execute(Statement::from_sql_and_values(
                        DbBackend::Sqlite,
                        "UPDATE session_efficiency_stats SET active_duration_ms = ?, efficiency_estimator_version = ? WHERE session_id = ? AND efficiency_estimator_version = ?",
                        [
                            active_duration_ms.into(),
                            EFFICIENCY_ESTIMATOR_VERSION.into(),
                            session_id.into(),
                            SOURCE_EFFICIENCY_ESTIMATOR_VERSION.into(),
                        ],
                    ))
                    .await?;
            }
            Ok(())
        }

        async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
            // Audit retention makes the original wall-clock span irrecoverable.
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use sea_orm_migration::sea_orm::{
            ConnectionTrait, Database as SeaDatabase, DbBackend, Statement,
        };

        #[tokio::test]
        async fn upgrade_rewrites_v2_spans_as_saturating_duration_sums() -> anyhow::Result<()> {
            let connection = SeaDatabase::connect("sqlite::memory:").await?;
            super::super::Migrator::up(&connection, Some(8)).await?;
            for statement in [
                "INSERT INTO session_efficiency_stats VALUES ('gap', 10, 2, 100, 0, 0, 0, 1, 0)",
                "INSERT INTO session_efficiency_stats VALUES ('overlap', 10, 2, 700, 0, 0, 0, 1, 0)",
                "INSERT INTO session_efficiency_stats VALUES ('partial', 10, 2, 300, 0, 0, 0, 1, 0)",
                "INSERT INTO session_efficiency_stats VALUES ('precise', 10, 2, 1, 0, 0, 0, 1, 0)",
                "INSERT INTO session_efficiency_stats VALUES ('retained', 10, 1, 250, 0, 0, 0, 1, 0)",
                "INSERT INTO session_efficiency_stats VALUES ('reused', 10, 2, 600, 0, 0, 0, 1, 0)",
                "INSERT INTO session_efficiency_stats VALUES ('saturated', 10, 2, 1, 0, 0, 0, 1, 0)",
                "INSERT INTO tool_dispatches (created_at, agent_id, slug, agent_label, session_id, tool_name, duration_ms, has_screenshot) VALUES (0, 'agent', 'agent', 'Agent', 'gap', 'navigate', NULL, 0), (1, 'agent', 'agent', 'Agent', 'gap', 'click', -10, 0)",
                "INSERT INTO tool_dispatches (created_at, agent_id, slug, agent_label, session_id, tool_name, duration_ms, has_screenshot) VALUES (0, 'agent', 'agent', 'Agent', 'overlap', 'navigate', 500, 0), (1, 'agent', 'agent', 'Agent', 'overlap', 'click', 400, 0)",
                "INSERT INTO tool_dispatches (created_at, agent_id, slug, agent_label, session_id, tool_name, duration_ms, has_screenshot) VALUES (0, 'agent', 'agent', 'Agent', 'partial', 'navigate', 10, 0)",
                "INSERT INTO tool_dispatches (created_at, agent_id, slug, agent_label, session_id, tool_name, duration_ms, has_screenshot) VALUES (0, 'agent', 'agent', 'Agent', 'precise', 'navigate', 9007199254740992, 0), (1, 'agent', 'agent', 'Agent', 'precise', 'click', 1, 0)",
                "INSERT INTO tool_dispatches (created_at, agent_id, slug, agent_label, session_id, tool_name, duration_ms, has_screenshot) VALUES (20, 'agent', 'agent', 'Agent', 'reused', 'navigate', 10, 0), (21, 'agent', 'agent', 'Agent', 'reused', 'click', 20, 0)",
                "INSERT INTO tool_dispatches (created_at, agent_id, slug, agent_label, session_id, tool_name, duration_ms, has_screenshot) VALUES (0, 'agent', 'agent', 'Agent', 'saturated', 'navigate', 9223372036854775807, 0), (1, 'agent', 'agent', 'Agent', 'saturated', 'click', 1, 0)",
            ] {
                connection.execute_unprepared(statement).await?;
            }
            super::super::Migrator::up(&connection, None).await?;
            let rows = connection
                .query_all(Statement::from_string(
                    DbBackend::Sqlite,
                    "SELECT session_id, active_duration_ms, efficiency_estimator_version FROM session_efficiency_stats ORDER BY session_id",
                ))
                .await?;
            assert_eq!(
                rows.into_iter()
                    .map(|row| Ok((
                        row.try_get::<String>("", "session_id")?,
                        row.try_get::<i64>("", "active_duration_ms")?,
                        row.try_get::<i64>("", "efficiency_estimator_version")?,
                    )))
                    .collect::<Result<Vec<_>, DbErr>>()?,
                [
                    ("gap".to_owned(), 0, 3),
                    ("overlap".to_owned(), 900, 3),
                    ("partial".to_owned(), 300, 2),
                    ("precise".to_owned(), 9_007_199_254_740_993, 3),
                    ("retained".to_owned(), 250, 2),
                    ("reused".to_owned(), 600, 2),
                    ("saturated".to_owned(), i64::MAX, 3)
                ]
            );
            Ok(())
        }
    }
}

mod m0008_add_task_token_estimates {
    use super::*;

    pub struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &str {
            "m0008_add_task_token_estimates"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            // Zero/false defaults preserve existing rows; the backfill below refines them.
            for column in ["tool_input_token_estimate", "tool_output_token_estimate"] {
                if !manager.has_column("tasks", column).await? {
                    manager
                        .alter_table(
                            Table::alter()
                                .table(Alias::new("tasks"))
                                .add_column(
                                    ColumnDef::new(Alias::new(column))
                                        .big_integer()
                                        .not_null()
                                        .default(0),
                                )
                                .to_owned(),
                        )
                        .await?;
                }
            }
            if !manager.has_column("tasks", "tokens_measured").await? {
                manager
                    .alter_table(
                        Table::alter()
                            .table(Alias::new("tasks"))
                            .add_column(
                                ColumnDef::new(Alias::new("tokens_measured"))
                                    .boolean()
                                    .not_null()
                                    .default(false),
                            )
                            .to_owned(),
                    )
                    .await?;
            }
            // Backfill the materialized per-session totals from the source dispatches. `tokens_measured`
            // mirrors the efficiency projection's eligibility: a session counts only when it has
            // dispatches and every one carries token-estimator v1 (0 is legacy/unmeasured). The version
            // literal is the value frozen at this migration, not a runtime constant.
            manager
                .get_connection()
                .execute_unprepared(
                    r#"
                    UPDATE tasks SET
                        tool_input_token_estimate = COALESCE((
                            SELECT SUM(MAX(d.tool_input_token_estimate, 0))
                            FROM tool_dispatches d
                            WHERE d.session_id = tasks.session_id
                        ), 0),
                        tool_output_token_estimate = COALESCE((
                            SELECT SUM(MAX(d.tool_output_token_estimate, 0))
                            FROM tool_dispatches d
                            WHERE d.session_id = tasks.session_id
                        ), 0),
                        tokens_measured = CASE
                            WHEN EXISTS (
                                SELECT 1 FROM tool_dispatches d
                                WHERE d.session_id = tasks.session_id
                            )
                            AND NOT EXISTS (
                                SELECT 1 FROM tool_dispatches d
                                WHERE d.session_id = tasks.session_id
                                  AND d.token_estimator_version != 1
                            )
                            THEN 1 ELSE 0 END
                    "#,
                )
                .await?;
            Ok(())
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            for column in [
                "tokens_measured",
                "tool_output_token_estimate",
                "tool_input_token_estimate",
            ] {
                if manager.has_column("tasks", column).await? {
                    manager
                        .alter_table(
                            Table::alter()
                                .table(Alias::new("tasks"))
                                .drop_column(Alias::new(column))
                                .to_owned(),
                        )
                        .await?;
                }
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use sea_orm_migration::sea_orm::{
            ConnectionTrait, Database as SeaDatabase, DatabaseConnection, DbBackend, Statement,
        };

        struct PreviousMigrator;

        #[async_trait::async_trait]
        impl MigratorTrait for PreviousMigrator {
            fn migrations() -> Vec<Box<dyn MigrationTrait>> {
                vec![
                    Box::new(super::super::m0001_baseline::Migration),
                    Box::new(super::super::m0002_add_recordings_and_claims::Migration),
                    Box::new(super::super::m0003_document_recordings_and_tab_ownership::Migration),
                    Box::new(super::super::m0004_atomic_recording_payloads::Migration),
                    Box::new(super::super::m0005_reclassify_task_status::Migration),
                    Box::new(super::super::m0006_add_tool_token_estimates::Migration),
                    Box::new(super::super::m0007_add_session_efficiency_stats::Migration),
                ]
            }
        }

        async fn insert_dispatch(
            conn: &DatabaseConnection,
            session_id: &str,
            input_tokens: i64,
            output_tokens: i64,
            version: i64,
        ) -> Result<(), DbErr> {
            conn.execute_unprepared(&format!(
                "INSERT INTO tool_dispatches (created_at, agent_id, slug, agent_label, session_id, tool_name, has_screenshot, tool_input_token_estimate, tool_output_token_estimate, token_estimator_version) VALUES (0, 'agent', 'agent', 'Agent', '{session_id}', 'navigate', 0, {input_tokens}, {output_tokens}, {version})"
            ))
            .await?;
            Ok(())
        }

        async fn insert_task(
            conn: &DatabaseConnection,
            session_id: &str,
            dispatch_count: i64,
        ) -> Result<(), DbErr> {
            conn.execute_unprepared(&format!(
                "INSERT INTO tasks (session_id, agent_id, slug, agent_label, title, started_at, duration_ms, dispatch_count, tool_sequence_json, status, error_count, cursor_id, has_screenshots, updated_at) VALUES ('{session_id}', 'agent', 'agent', 'Agent', 'Session', 0, 0, {dispatch_count}, '[]', 'done', 0, 0, 0, 0)"
            ))
            .await?;
            Ok(())
        }

        async fn token_row(
            conn: &DatabaseConnection,
            session_id: &str,
        ) -> Result<(i64, i64, bool), DbErr> {
            let row = conn
                .query_one(Statement::from_string(
                    DbBackend::Sqlite,
                    format!(
                        "SELECT tool_input_token_estimate, tool_output_token_estimate, tokens_measured FROM tasks WHERE session_id = '{session_id}'"
                    ),
                ))
                .await?
                .ok_or_else(|| DbErr::RecordNotFound(session_id.to_string()))?;
            Ok((
                row.try_get("", "tool_input_token_estimate")?,
                row.try_get("", "tool_output_token_estimate")?,
                row.try_get("", "tokens_measured")?,
            ))
        }

        #[tokio::test]
        async fn backfill_sums_tokens_and_marks_only_all_v1_sessions() -> anyhow::Result<()> {
            let conn = SeaDatabase::connect("sqlite::memory:").await?;
            PreviousMigrator::up(&conn, None).await?;

            insert_task(&conn, "measured", 2).await?;
            insert_dispatch(&conn, "measured", 10, 100, 1).await?;
            insert_dispatch(&conn, "measured", 20, 200, 1).await?;

            insert_task(&conn, "legacy", 1).await?;
            insert_dispatch(&conn, "legacy", 0, 0, 0).await?;

            // One legacy dispatch taints the session, but the sums still count every dispatch.
            insert_task(&conn, "mixed", 2).await?;
            insert_dispatch(&conn, "mixed", 5, 5, 1).await?;
            insert_dispatch(&conn, "mixed", 5, 5, 0).await?;

            insert_task(&conn, "empty", 0).await?;

            super::super::Migrator::up(&conn, None).await?;

            assert_eq!(token_row(&conn, "measured").await?, (30, 300, true));
            assert_eq!(token_row(&conn, "legacy").await?, (0, 0, false));
            assert_eq!(token_row(&conn, "mixed").await?, (10, 10, false));
            assert_eq!(token_row(&conn, "empty").await?, (0, 0, false));
            Ok(())
        }
    }
}

mod m0007_add_session_efficiency_stats {
    use super::*;

    pub struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &str {
            "m0007_add_session_efficiency_stats"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .create_table(
                    Table::create()
                        .table(SessionEfficiencyStats::Table)
                        .if_not_exists()
                        .col(
                            ColumnDef::new(SessionEfficiencyStats::SessionId)
                                .string()
                                .not_null()
                                .primary_key(),
                        )
                        .col(
                            ColumnDef::new(SessionEfficiencyStats::EndedAt)
                                .big_integer()
                                .not_null(),
                        )
                        .col(
                            ColumnDef::new(SessionEfficiencyStats::DispatchCount)
                                .big_integer()
                                .not_null(),
                        )
                        .col(
                            ColumnDef::new(SessionEfficiencyStats::ActiveDurationMs)
                                .big_integer()
                                .not_null(),
                        )
                        .col(
                            ColumnDef::new(SessionEfficiencyStats::ToolInputTokenEstimate)
                                .big_integer()
                                .not_null(),
                        )
                        .col(
                            ColumnDef::new(SessionEfficiencyStats::ToolOutputTokenEstimate)
                                .big_integer()
                                .not_null(),
                        )
                        .col(
                            ColumnDef::new(SessionEfficiencyStats::ScreenshotBaselineTokenEstimate)
                                .big_integer()
                                .not_null(),
                        )
                        .col(
                            ColumnDef::new(SessionEfficiencyStats::EfficiencyEstimatorVersion)
                                .big_integer()
                                .not_null(),
                        )
                        .col(
                            ColumnDef::new(SessionEfficiencyStats::ComputedAt)
                                .big_integer()
                                .not_null(),
                        )
                        .to_owned(),
                )
                .await?;
            manager
                .create_index(
                    Index::create()
                        .name("session_efficiency_stats_ended_at_idx")
                        .table(SessionEfficiencyStats::Table)
                        .col(SessionEfficiencyStats::EndedAt)
                        .if_not_exists()
                        .to_owned(),
                )
                .await?;
            Ok(())
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .drop_table(
                    Table::drop()
                        .table(SessionEfficiencyStats::Table)
                        .if_exists()
                        .to_owned(),
                )
                .await?;
            Ok(())
        }
    }

    #[derive(DeriveIden)]
    enum SessionEfficiencyStats {
        Table,
        SessionId,
        EndedAt,
        DispatchCount,
        ActiveDurationMs,
        ToolInputTokenEstimate,
        ToolOutputTokenEstimate,
        ScreenshotBaselineTokenEstimate,
        EfficiencyEstimatorVersion,
        ComputedAt,
    }
}

mod m0006_add_tool_token_estimates {
    use super::*;

    pub struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &str {
            "m0006_add_tool_token_estimates"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            // Zero defaults preserve existing rows while reserving version 0 for unmeasured data.
            for column in [
                "tool_input_token_estimate",
                "tool_output_token_estimate",
                "token_estimator_version",
            ] {
                if !manager.has_column("tool_dispatches", column).await? {
                    manager
                        .alter_table(
                            Table::alter()
                                .table(Alias::new("tool_dispatches"))
                                .add_column(
                                    ColumnDef::new(Alias::new(column))
                                        .big_integer()
                                        .not_null()
                                        .default(0),
                                )
                                .to_owned(),
                        )
                        .await?;
                }
            }
            Ok(())
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            for column in [
                "token_estimator_version",
                "tool_output_token_estimate",
                "tool_input_token_estimate",
            ] {
                if manager.has_column("tool_dispatches", column).await? {
                    manager
                        .alter_table(
                            Table::alter()
                                .table(Alias::new("tool_dispatches"))
                                .drop_column(Alias::new(column))
                                .to_owned(),
                        )
                        .await?;
                }
            }
            Ok(())
        }
    }
}

mod m0005_reclassify_task_status {
    use super::*;

    pub struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &str {
            "m0005_reclassify_task_status"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            // Task status is a persisted projection, so completed rows must be reclassified when
            // its semantics change instead of waiting for another dispatch that will never arrive.
            manager
                .get_connection()
                .execute_unprepared(
                    r#"
                    WITH first_ends AS (
                        SELECT session_id, kind
                        FROM (
                            SELECT
                                session_id,
                                kind,
                                ROW_NUMBER() OVER (
                                    PARTITION BY session_id
                                    ORDER BY id ASC
                                ) AS position
                            FROM agent_session_ends
                        )
                        WHERE position = 1
                    ),
                    dispatch_tails AS (
                        SELECT
                            session_id,
                            COUNT(*) AS dispatch_count,
                            SUM(
                                CASE WHEN json_valid(result_meta) THEN
                                    CASE WHEN
                                        json_extract(result_meta, '$.isError') = 1
                                        AND COALESCE(
                                            json_extract(result_meta, '$.cancelled'),
                                            0
                                        ) != 1
                                    THEN 1 ELSE 0 END
                                ELSE 0 END
                            ) AS error_count
                        FROM (
                            SELECT
                                session_id,
                                result_meta,
                                ROW_NUMBER() OVER (
                                    PARTITION BY session_id
                                    ORDER BY id DESC
                                ) AS position
                            FROM tool_dispatches
                        )
                        WHERE position <= 3
                        GROUP BY session_id
                    )
                    UPDATE tasks
                    SET status = CASE
                        WHEN NOT EXISTS (
                            SELECT 1 FROM first_ends
                            WHERE first_ends.session_id = tasks.session_id
                        ) THEN 'live'
                        WHEN (
                            SELECT kind FROM first_ends
                            WHERE first_ends.session_id = tasks.session_id
                        ) = 'cancelled' THEN 'cancelled'
                        WHEN dispatch_count = 0 AND (
                            SELECT kind FROM first_ends
                            WHERE first_ends.session_id = tasks.session_id
                        ) = 'errored' THEN 'failed'
                        WHEN dispatch_count = 0 AND (
                            SELECT kind FROM first_ends
                            WHERE first_ends.session_id = tasks.session_id
                        ) = 'closed' THEN 'done'
                        WHEN (
                            SELECT kind FROM first_ends
                            WHERE first_ends.session_id = tasks.session_id
                        ) IN ('closed', 'errored') AND EXISTS (
                            SELECT 1 FROM dispatch_tails
                            WHERE dispatch_tails.session_id = tasks.session_id
                                AND dispatch_tails.dispatch_count = 3
                                AND dispatch_tails.error_count = 3
                        ) THEN 'failed'
                        WHEN (
                            SELECT kind FROM first_ends
                            WHERE first_ends.session_id = tasks.session_id
                        ) IN ('closed', 'errored') THEN 'done'
                        ELSE 'live'
                    END
                    "#,
                )
                .await?;
            Ok(())
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .get_connection()
                .execute_unprepared(
                    r#"
                    WITH first_ends AS (
                        SELECT session_id, kind
                        FROM (
                            SELECT
                                session_id,
                                kind,
                                ROW_NUMBER() OVER (
                                    PARTITION BY session_id
                                    ORDER BY id ASC
                                ) AS position
                            FROM agent_session_ends
                        )
                        WHERE position = 1
                    )
                    UPDATE tasks
                    SET status = CASE
                        WHEN (
                            SELECT kind FROM first_ends
                            WHERE first_ends.session_id = tasks.session_id
                        ) = 'cancelled' THEN 'cancelled'
                        WHEN (
                            SELECT kind FROM first_ends
                            WHERE first_ends.session_id = tasks.session_id
                        ) = 'errored' THEN 'failed'
                        WHEN (
                            SELECT kind FROM first_ends
                            WHERE first_ends.session_id = tasks.session_id
                        ) = 'closed' AND error_count = 0 THEN 'done'
                        WHEN error_count > 0 THEN 'failed'
                        ELSE 'live'
                    END
                    "#,
                )
                .await?;
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use sea_orm_migration::sea_orm::{
            ConnectionTrait, Database as SeaDatabase, DatabaseConnection, DbBackend, Statement,
        };

        struct PreviousMigrator;

        #[async_trait::async_trait]
        impl MigratorTrait for PreviousMigrator {
            fn migrations() -> Vec<Box<dyn MigrationTrait>> {
                vec![
                    Box::new(super::super::m0001_baseline::Migration),
                    Box::new(super::super::m0002_add_recordings_and_claims::Migration),
                    Box::new(super::super::m0003_document_recordings_and_tab_ownership::Migration),
                    Box::new(super::super::m0004_atomic_recording_payloads::Migration),
                ]
            }
        }

        async fn insert_legacy_task(
            connection: &DatabaseConnection,
            session_id: &str,
            results: &[bool],
            end_kind: Option<&str>,
        ) -> Result<(), DbErr> {
            for (index, is_error) in results.iter().enumerate() {
                let result_meta = format!(
                    r#"{{"isError":{},"cancelled":false}}"#,
                    if *is_error { "true" } else { "false" }
                );
                connection
                    .execute_unprepared(&format!(
                        "INSERT INTO tool_dispatches (created_at, agent_id, slug, agent_label, session_id, tool_name, result_meta, has_screenshot) VALUES ({index}, 'agent', 'agent', 'Agent', '{session_id}', 'navigate', '{result_meta}', 0)"
                    ))
                    .await?;
            }
            if let Some(kind) = end_kind {
                connection
                    .execute_unprepared(&format!(
                        "INSERT INTO agent_session_ends (created_at, session_id, kind) VALUES (100, '{session_id}', '{kind}')"
                    ))
                    .await?;
            }
            let error_count = results.iter().filter(|is_error| **is_error).count();
            let old_status = match end_kind {
                Some("cancelled") => "cancelled",
                Some("errored") => "failed",
                Some("closed") if error_count == 0 => "done",
                _ if error_count > 0 => "failed",
                _ => "live",
            };
            let ended_at = end_kind.map_or_else(|| "NULL".to_string(), |_| "100".to_string());
            connection
                .execute_unprepared(&format!(
                    "INSERT INTO tasks (session_id, agent_id, slug, agent_label, title, started_at, ended_at, duration_ms, dispatch_count, tool_sequence_json, status, error_count, cursor_id, has_screenshots, updated_at) VALUES ('{session_id}', 'agent', 'agent', 'Agent', 'Session', 0, {ended_at}, 100, {}, '[]', '{old_status}', {error_count}, {}, 0, 100)",
                    results.len(),
                    results.len()
                ))
                .await?;
            Ok(())
        }

        async fn status_of(
            connection: &DatabaseConnection,
            session_id: &str,
        ) -> Result<String, DbErr> {
            connection
                .query_one(Statement::from_string(
                    DbBackend::Sqlite,
                    format!("SELECT status FROM tasks WHERE session_id = '{session_id}'"),
                ))
                .await?
                .ok_or_else(|| DbErr::RecordNotFound(session_id.to_string()))?
                .try_get("", "status")
        }

        #[tokio::test]
        async fn upgrade_reclassifies_existing_task_statuses() -> anyhow::Result<()> {
            let connection = SeaDatabase::connect("sqlite::memory:").await?;
            PreviousMigrator::up(&connection, None).await?;
            for (session_id, results, end_kind) in [
                ("recovered", vec![true, false], Some("errored")),
                ("two-errors", vec![false, true, true], Some("closed")),
                (
                    "three-errors",
                    vec![false, true, true, true],
                    Some("closed"),
                ),
                ("active", vec![true, true, true], None),
                ("empty-closed", vec![], Some("closed")),
                ("empty-errored", vec![], Some("errored")),
                ("cancelled", vec![true, true, true], Some("cancelled")),
            ] {
                insert_legacy_task(&connection, session_id, &results, end_kind).await?;
            }

            assert_eq!(status_of(&connection, "recovered").await?, "failed");
            assert_eq!(status_of(&connection, "two-errors").await?, "failed");
            assert_eq!(status_of(&connection, "active").await?, "failed");

            super::super::Migrator::up(&connection, None).await?;

            for (session_id, status) in [
                ("recovered", "done"),
                ("two-errors", "done"),
                ("three-errors", "failed"),
                ("active", "live"),
                ("empty-closed", "done"),
                ("empty-errored", "failed"),
                ("cancelled", "cancelled"),
            ] {
                assert_eq!(status_of(&connection, session_id).await?, status);
            }
            Ok(())
        }
    }
}

mod m0003_document_recordings_and_tab_ownership {
    use super::*;

    pub struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &str {
            "m0003_document_recordings_and_tab_ownership"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            let connection = manager.get_connection();
            for statement in [
                "CREATE TABLE IF NOT EXISTS session_tabs (id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL, session_id TEXT NOT NULL, agent_id TEXT NOT NULL, tab_id INTEGER NOT NULL, opened_target_id TEXT, claimed_at INTEGER NOT NULL, released_at INTEGER)",
                "CREATE INDEX IF NOT EXISTS session_tabs_session_idx ON session_tabs(session_id, claimed_at)",
                "CREATE INDEX IF NOT EXISTS session_tabs_tab_window_idx ON session_tabs(tab_id, claimed_at, released_at)",
                "CREATE UNIQUE INDEX IF NOT EXISTS session_tabs_one_live_owner_idx ON session_tabs(tab_id) WHERE released_at IS NULL",
                "CREATE TABLE IF NOT EXISTS recording_streams (document_id TEXT PRIMARY KEY NOT NULL, tab_id INTEGER NOT NULL, target_id TEXT, first_event_at INTEGER NOT NULL, last_event_at INTEGER NOT NULL, size_bytes INTEGER NOT NULL, event_count INTEGER NOT NULL, has_gap INTEGER NOT NULL DEFAULT 0)",
                "CREATE INDEX IF NOT EXISTS recording_streams_tab_time_idx ON recording_streams(tab_id, first_event_at, last_event_at)",
                "CREATE INDEX IF NOT EXISTS recording_streams_retention_idx ON recording_streams(last_event_at)",
                "CREATE TABLE IF NOT EXISTS recording_batches (document_id TEXT NOT NULL REFERENCES recording_streams(document_id) ON DELETE CASCADE, batch_id TEXT NOT NULL, accepted_at INTEGER NOT NULL, PRIMARY KEY(document_id, batch_id))",
            ] {
                connection.execute_unprepared(statement).await?;
            }
            if !manager.has_column("tool_dispatches", "tab_id").await? {
                manager
                    .alter_table(
                        Table::alter()
                            .table(Alias::new("tool_dispatches"))
                            .add_column(ColumnDef::new(Alias::new("tab_id")).big_integer())
                            .to_owned(),
                    )
                    .await?;
            }
            Ok(())
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            for table in ["recording_batches", "recording_streams", "session_tabs"] {
                manager
                    .drop_table(
                        Table::drop()
                            .table(Alias::new(table))
                            .if_exists()
                            .to_owned(),
                    )
                    .await?;
            }
            Ok(())
        }
    }
}

mod m0004_atomic_recording_payloads {
    use super::*;

    pub struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &str {
            "m0004_atomic_recording_payloads"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .get_connection()
                .execute_unprepared(
                    "CREATE TABLE IF NOT EXISTS recording_payloads (document_id TEXT PRIMARY KEY NOT NULL REFERENCES recording_streams(document_id) ON DELETE CASCADE, events_ndjson TEXT NOT NULL)",
                )
                .await?;
            Ok(())
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .drop_table(
                    Table::drop()
                        .table(Alias::new("recording_payloads"))
                        .if_exists()
                        .to_owned(),
                )
                .await?;
            Ok(())
        }
    }
}

mod m0002_add_recordings_and_claims {
    use super::*;

    pub struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &str {
            "m0002_add_recordings_and_claims"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .create_table(
                    Table::create()
                        .table(TabRecordings::Table)
                        .if_not_exists()
                        .col(
                            ColumnDef::new(TabRecordings::TargetId)
                                .string()
                                .not_null()
                                .primary_key(),
                        )
                        .col(
                            ColumnDef::new(TabRecordings::TabId)
                                .big_integer()
                                .not_null(),
                        )
                        .col(
                            ColumnDef::new(TabRecordings::FirstEventAt)
                                .big_integer()
                                .not_null(),
                        )
                        .col(
                            ColumnDef::new(TabRecordings::LastEventAt)
                                .big_integer()
                                .not_null(),
                        )
                        .col(
                            ColumnDef::new(TabRecordings::SizeBytes)
                                .big_integer()
                                .not_null(),
                        )
                        .col(
                            ColumnDef::new(TabRecordings::EventCount)
                                .big_integer()
                                .not_null(),
                        )
                        .to_owned(),
                )
                .await?;
            manager
                .create_index(
                    Index::create()
                        .name("tab_recordings_last_event_idx")
                        .table(TabRecordings::Table)
                        .col(TabRecordings::LastEventAt)
                        .if_not_exists()
                        .to_owned(),
                )
                .await?;
            manager
                .create_table(
                    Table::create()
                        .table(TabClaims::Table)
                        .if_not_exists()
                        .col(
                            ColumnDef::new(TabClaims::Id)
                                .big_integer()
                                .not_null()
                                .auto_increment()
                                .primary_key(),
                        )
                        .col(ColumnDef::new(TabClaims::TargetId).string().not_null())
                        .col(ColumnDef::new(TabClaims::SessionId).string().not_null())
                        .col(ColumnDef::new(TabClaims::AgentId).string().not_null())
                        .col(
                            ColumnDef::new(TabClaims::ClaimedAt)
                                .big_integer()
                                .not_null(),
                        )
                        .col(ColumnDef::new(TabClaims::ReleasedAt).big_integer())
                        .to_owned(),
                )
                .await?;
            manager
                .create_index(
                    Index::create()
                        .name("tab_claims_target_idx")
                        .table(TabClaims::Table)
                        .col(TabClaims::TargetId)
                        .col(TabClaims::ClaimedAt)
                        .if_not_exists()
                        .to_owned(),
                )
                .await?;
            manager
                .create_index(
                    Index::create()
                        .name("tab_claims_session_idx")
                        .table(TabClaims::Table)
                        .col(TabClaims::SessionId)
                        .if_not_exists()
                        .to_owned(),
                )
                .await?;
            Ok(())
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .drop_table(Table::drop().table(TabClaims::Table).if_exists().to_owned())
                .await?;
            manager
                .drop_table(
                    Table::drop()
                        .table(TabRecordings::Table)
                        .if_exists()
                        .to_owned(),
                )
                .await?;
            Ok(())
        }
    }

    #[derive(DeriveIden)]
    enum TabRecordings {
        Table,
        TargetId,
        TabId,
        FirstEventAt,
        LastEventAt,
        SizeBytes,
        EventCount,
    }

    #[derive(DeriveIden)]
    enum TabClaims {
        Table,
        Id,
        TargetId,
        SessionId,
        AgentId,
        ClaimedAt,
        ReleasedAt,
    }
}

mod m0001_baseline {
    use super::*;

    pub struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &str {
            "m0001_baseline"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            create_tables(manager).await?;
            add_rust_columns(manager).await?;
            create_indexes(manager).await?;
            manager
                .get_connection()
                .execute_unprepared("DROP TABLE IF EXISTS __drizzle_migrations")
                .await?;
            Ok(())
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            for table in [
                "tasks",
                "agent_session_ends",
                "agent_session_starts",
                "tool_dispatches",
            ] {
                manager
                    .drop_table(
                        Table::drop()
                            .table(Alias::new(table))
                            .if_exists()
                            .to_owned(),
                    )
                    .await?;
            }
            Ok(())
        }
    }

    async fn create_tables(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ToolDispatches::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ToolDispatches::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ToolDispatches::CreatedAt)
                            .big_integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ToolDispatches::AgentId).string().not_null())
                    .col(ColumnDef::new(ToolDispatches::Slug).string().not_null())
                    .col(
                        ColumnDef::new(ToolDispatches::AgentLabel)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ToolDispatches::SessionId)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ToolDispatches::ToolName).string().not_null())
                    .col(ColumnDef::new(ToolDispatches::PageId).big_integer())
                    .col(ColumnDef::new(ToolDispatches::TargetId).string())
                    .col(ColumnDef::new(ToolDispatches::Url).string())
                    .col(ColumnDef::new(ToolDispatches::Title).string())
                    .col(ColumnDef::new(ToolDispatches::ArgsJson).text())
                    .col(ColumnDef::new(ToolDispatches::ResultMeta).text())
                    .col(ColumnDef::new(ToolDispatches::DurationMs).big_integer())
                    .col(ColumnDef::new(ToolDispatches::DispatchId).string())
                    .col(
                        ColumnDef::new(ToolDispatches::HasScreenshot)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(AgentSessionStarts::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AgentSessionStarts::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AgentSessionStarts::CreatedAt)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AgentSessionStarts::SessionId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AgentSessionStarts::AgentId)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AgentSessionStarts::Slug).string().not_null())
                    .col(
                        ColumnDef::new(AgentSessionStarts::AgentLabel)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AgentSessionStarts::ClientName)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AgentSessionStarts::ClientVersion)
                            .string()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(AgentSessionEnds::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AgentSessionEnds::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AgentSessionEnds::CreatedAt)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AgentSessionEnds::SessionId)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AgentSessionEnds::Kind).string().not_null())
                    .col(ColumnDef::new(AgentSessionEnds::Reason).string())
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Tasks::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Tasks::SessionId)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Tasks::AgentId).string().not_null())
                    .col(ColumnDef::new(Tasks::Slug).string().not_null())
                    .col(ColumnDef::new(Tasks::AgentLabel).string().not_null())
                    .col(ColumnDef::new(Tasks::Title).string().not_null())
                    .col(ColumnDef::new(Tasks::Site).string())
                    .col(ColumnDef::new(Tasks::StartedAt).big_integer().not_null())
                    .col(ColumnDef::new(Tasks::EndedAt).big_integer())
                    .col(ColumnDef::new(Tasks::DurationMs).big_integer().not_null())
                    .col(
                        ColumnDef::new(Tasks::DispatchCount)
                            .big_integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Tasks::ToolSequenceJson).text().not_null())
                    .col(ColumnDef::new(Tasks::Status).string().not_null())
                    .col(ColumnDef::new(Tasks::ErrorCount).big_integer().not_null())
                    .col(ColumnDef::new(Tasks::LastScreenshotDispatchId).big_integer())
                    .col(ColumnDef::new(Tasks::CursorId).big_integer().not_null())
                    .col(
                        ColumnDef::new(Tasks::HasScreenshots)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(ColumnDef::new(Tasks::UpdatedAt).big_integer().not_null())
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn add_rust_columns(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        if !manager.has_column("tool_dispatches", "dispatch_id").await? {
            manager
                .alter_table(
                    Table::alter()
                        .table(ToolDispatches::Table)
                        .add_column(ColumnDef::new(ToolDispatches::DispatchId).string())
                        .to_owned(),
                )
                .await?;
        }
        if !manager
            .has_column("tool_dispatches", "has_screenshot")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(ToolDispatches::Table)
                        .add_column(
                            ColumnDef::new(ToolDispatches::HasScreenshot)
                                .boolean()
                                .not_null()
                                .default(false),
                        )
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn create_indexes(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        for index in [
            Index::create()
                .name("tool_dispatches_created_at_idx")
                .table(ToolDispatches::Table)
                .col(ToolDispatches::CreatedAt)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("tool_dispatches_agent_created_idx")
                .table(ToolDispatches::Table)
                .col(ToolDispatches::AgentId)
                .col(ToolDispatches::CreatedAt)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("tool_dispatches_session_idx")
                .table(ToolDispatches::Table)
                .col(ToolDispatches::SessionId)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("agent_session_starts_session_idx")
                .table(AgentSessionStarts::Table)
                .col(AgentSessionStarts::SessionId)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("agent_session_starts_created_at_idx")
                .table(AgentSessionStarts::Table)
                .col(AgentSessionStarts::CreatedAt)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("agent_session_ends_session_idx")
                .table(AgentSessionEnds::Table)
                .col(AgentSessionEnds::SessionId)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("agent_session_ends_created_at_idx")
                .table(AgentSessionEnds::Table)
                .col(AgentSessionEnds::CreatedAt)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("tasks_cursor_idx")
                .table(Tasks::Table)
                .col((Tasks::CursorId, IndexOrder::Desc))
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("tasks_agent_cursor_idx")
                .table(Tasks::Table)
                .col(Tasks::AgentId)
                .col((Tasks::CursorId, IndexOrder::Desc))
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("tasks_status_cursor_idx")
                .table(Tasks::Table)
                .col(Tasks::Status)
                .col((Tasks::CursorId, IndexOrder::Desc))
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("tasks_site_cursor_idx")
                .table(Tasks::Table)
                .col(Tasks::Site)
                .col((Tasks::CursorId, IndexOrder::Desc))
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("tasks_started_idx")
                .table(Tasks::Table)
                .col(Tasks::StartedAt)
                .if_not_exists()
                .to_owned(),
        ] {
            manager.create_index(index).await?;
        }
        Ok(())
    }

    #[derive(DeriveIden)]
    enum ToolDispatches {
        Table,
        Id,
        CreatedAt,
        AgentId,
        Slug,
        AgentLabel,
        SessionId,
        ToolName,
        PageId,
        TargetId,
        Url,
        Title,
        ArgsJson,
        ResultMeta,
        DurationMs,
        DispatchId,
        HasScreenshot,
    }

    #[derive(DeriveIden)]
    enum AgentSessionStarts {
        Table,
        Id,
        CreatedAt,
        SessionId,
        AgentId,
        Slug,
        AgentLabel,
        ClientName,
        ClientVersion,
    }

    #[derive(DeriveIden)]
    enum AgentSessionEnds {
        Table,
        Id,
        CreatedAt,
        SessionId,
        Kind,
        Reason,
    }

    #[derive(DeriveIden)]
    enum Tasks {
        Table,
        SessionId,
        AgentId,
        Slug,
        AgentLabel,
        Title,
        Site,
        StartedAt,
        EndedAt,
        DurationMs,
        DispatchCount,
        ToolSequenceJson,
        Status,
        ErrorCount,
        LastScreenshotDispatchId,
        CursorId,
        HasScreenshots,
        UpdatedAt,
    }
}
