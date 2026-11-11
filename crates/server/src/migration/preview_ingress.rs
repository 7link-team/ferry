use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m.has_column("holes", "ingress_config").await? {
            m.alter_table(
                Table::alter()
                    .table(Alias::new("holes"))
                    .add_column(ColumnDef::new(Alias::new("ingress_config")).text().null())
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Preview ingress snapshots must be retained; restore a pre-upgrade backup to downgrade"
                .into(),
        ))
    }
}
