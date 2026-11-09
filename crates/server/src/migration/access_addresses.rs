use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m.has_column("platform_state", "access_config").await? {
            m.alter_table(
                Table::alter()
                    .table(Alias::new("platform_state"))
                    .add_column(ColumnDef::new(Alias::new("access_config")).text().null())
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.alter_table(
            Table::alter()
                .table(Alias::new("platform_state"))
                .drop_column(Alias::new("access_config"))
                .to_owned(),
        )
        .await
    }
}
