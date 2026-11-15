//! Time zones for processes a runner starts: a host setting ("" = the host's
//! own zone), what the runner detected and whether it can apply one, and an
//! optional zone a CLI model requires.
use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for (table, column, definition) in [
            (
                "hosts",
                "timezone",
                ColumnDef::new(Alias::new("timezone"))
                    .string_len(64)
                    .not_null()
                    .default("")
                    .to_owned(),
            ),
            (
                "hosts",
                "detected_timezone",
                ColumnDef::new(Alias::new("detected_timezone"))
                    .string_len(64)
                    .not_null()
                    .default("")
                    .to_owned(),
            ),
            (
                "hosts",
                "timezone_supported",
                ColumnDef::new(Alias::new("timezone_supported"))
                    .big_integer()
                    .not_null()
                    .default(0_i64)
                    .to_owned(),
            ),
            (
                "ai_model_profiles",
                "timezone",
                ColumnDef::new(Alias::new("timezone"))
                    .string_len(64)
                    .not_null()
                    .default("")
                    .to_owned(),
            ),
        ] {
            if !m.has_column(table, column).await? {
                m.alter_table(
                    Table::alter()
                        .table(Alias::new(table))
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
