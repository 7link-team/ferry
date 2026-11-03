use super::entities::{bindings, definitions, runs};
use sea_orm::Schema;
use sea_orm_migration::prelude::*;

pub struct InsightReports;
impl MigrationName for InsightReports {
    fn name(&self) -> &str {
        "m20261104_insight_reports"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for InsightReports {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let schema = Schema::new(backend);
        for mut table in [
            schema.create_table_from_entity(definitions::Entity),
            schema.create_table_from_entity(runs::Entity),
            schema.create_table_from_entity(bindings::Entity),
        ] {
            table.if_not_exists();
            if backend == sea_orm::DbBackend::MySql {
                table.character_set("utf8mb4").collate("utf8mb4_bin");
            }
            manager.create_table(table).await?;
        }
        // Reports contain full evidence arrays, beyond MySQL TEXT's 64 KiB limit.
        if backend == sea_orm::DbBackend::MySql {
            for (table, column) in [
                ("insight_report_definitions", "config"),
                ("insight_report_runs", "config_snapshot"),
                ("insight_report_runs", "snapshot"),
                ("insight_report_runs", "error"),
            ] {
                manager
                    .alter_table(
                        Table::alter()
                            .table(Alias::new(table))
                            .modify_column(
                                ColumnDef::new(Alias::new(column))
                                    .custom("longtext")
                                    .not_null(),
                            )
                            .to_owned(),
                    )
                    .await?;
            }
        }
        for (table, name, columns, unique) in [
            (
                "insight_report_definitions",
                "idx_insight_reports_owner",
                vec!["workspace_id", "user_id", "next_run_at"],
                false,
            ),
            (
                "insight_report_runs",
                "idx_insight_report_idempotency",
                vec!["idempotency_key"],
                true,
            ),
            (
                "insight_report_runs",
                "idx_insight_report_queue",
                vec!["state", "lease_until"],
                false,
            ),
            (
                "insight_report_runs",
                "idx_insight_report_history",
                vec!["workspace_id", "user_id", "definition_id", "created_at"],
                false,
            ),
            (
                "application_repository_bindings",
                "idx_application_repository_owner",
                vec!["workspace_id", "user_id", "application_id"],
                false,
            ),
        ] {
            if !manager.has_index(table, name).await? {
                let mut index = Index::create();
                index.name(name).table(Alias::new(table));
                for column in columns {
                    index.col(Alias::new(column));
                }
                if unique {
                    index.unique();
                }
                manager.create_index(index.to_owned()).await?;
            }
        }
        Ok(())
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for name in [
            "insight_report_runs",
            "insight_report_definitions",
            "application_repository_bindings",
        ] {
            manager
                .drop_table(Table::drop().table(Alias::new(name)).to_owned())
                .await?;
        }
        Ok(())
    }
}
