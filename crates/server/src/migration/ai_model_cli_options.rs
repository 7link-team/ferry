//! Per-CLI options (API format, model roles, reasoning...) for CLI model profiles.
use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m.has_column("ai_model_profiles", "cli_options").await? {
            m.alter_table(
                Table::alter()
                    .table(Alias::new("ai_model_profiles"))
                    .add_column(ColumnDef::new(Alias::new("cli_options")).text().null())
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.alter_table(
            Table::alter()
                .table(Alias::new("ai_model_profiles"))
                .drop_column(Alias::new("cli_options"))
                .to_owned(),
        )
        .await
    }
}
