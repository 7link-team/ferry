//! The ordered Runner hosts a CLI model prefers ("[]" = any ready Runner).
use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m.has_column("ai_model_profiles", "hosts").await? {
            m.alter_table(
                Table::alter()
                    .table(Alias::new("ai_model_profiles"))
                    .add_column(
                        ColumnDef::new(Alias::new("hosts"))
                            .string_len(1024)
                            .not_null()
                            .default("[]"),
                    )
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}
