use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for (table, column) in [("platform_state", "file_storage"), ("worker_ids", "info")] {
            if !m.has_column(table, column).await? {
                m.alter_table(
                    Table::alter()
                        .table(Alias::new(table))
                        .add_column(ColumnDef::new(Alias::new(column)).text().null())
                        .to_owned(),
                )
                .await?;
            }
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Storage configuration and node identity must be retained".into(),
        ))
    }
}
