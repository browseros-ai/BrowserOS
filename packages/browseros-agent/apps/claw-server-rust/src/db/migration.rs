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
        ]
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
