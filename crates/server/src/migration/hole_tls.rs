//! Preview targets may speak HTTPS, optionally with a certificate Ferry does
//! not verify (self-signed devices such as PVE or a NAS).
use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for (column, definition) in [
            (
                "target_scheme",
                ColumnDef::new(Alias::new("target_scheme"))
                    .string_len(8)
                    .not_null()
                    .default("http")
                    .to_owned(),
            ),
            (
                "target_tls_insecure",
                ColumnDef::new(Alias::new("target_tls_insecure"))
                    .boolean()
                    .not_null()
                    .default(false)
                    .to_owned(),
            ),
        ] {
            if !m.has_column("holes", column).await? {
                m.alter_table(
                    Table::alter()
                        .table(Alias::new("holes"))
                        .add_column(definition)
                        .to_owned(),
                )
                .await?;
            }
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}
