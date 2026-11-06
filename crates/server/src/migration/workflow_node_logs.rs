use super::*;

pub struct Migration;
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "workflow_node_logs_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for (name, column) in [
            (
                "logs",
                long_text("logs", m.get_database_backend())
                    .null()
                    .to_owned(),
            ),
            (
                "logs_truncated",
                ColumnDef::new(Alias::new("logs_truncated"))
                    .boolean()
                    .not_null()
                    .default(false)
                    .to_owned(),
            ),
            (
                "logs_updated_at",
                ColumnDef::new(Alias::new("logs_updated_at"))
                    .big_integer()
                    .not_null()
                    .default(0)
                    .to_owned(),
            ),
        ] {
            if !m.has_column("workflow_node_runs", name).await? {
                m.alter_table(
                    Table::alter()
                        .table(Alias::new("workflow_node_runs"))
                        .add_column(column)
                        .to_owned(),
                )
                .await?;
            }
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
