//! Preview tunnels may target a private-network address the Runner can reach.
use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m.has_column("holes", "target_host").await? {
            m.alter_table(
                Table::alter()
                    .table(Alias::new("holes"))
                    .add_column(
                        ColumnDef::new(Alias::new("target_host"))
                            .string_len(253)
                            .not_null()
                            .default(""),
                    )
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.alter_table(
            Table::alter()
                .table(Alias::new("holes"))
                .drop_column(Alias::new("target_host"))
                .to_owned(),
        )
        .await
    }
}
