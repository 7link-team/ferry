use super::*;

pub struct Migration;
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "ai_model_profiles_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        let mut table = create_table("ai_model_profiles", backend);
        table.col(
            ColumnDef::new(Alias::new("id"))
                .big_integer()
                .not_null()
                .primary_key(),
        );
        for name in [
            "workspace_id",
            "provider_id",
            "revision_id",
            "version",
            "created_at",
            "updated_at",
        ] {
            table.col(ColumnDef::new(Alias::new(name)).big_integer().not_null());
        }
        for (name, len) in [
            ("name", 128),
            ("kind", 16),
            ("runtime", 16),
            ("default_model", 256),
            ("tool_version", 64),
            ("state", 16),
        ] {
            table.col(ColumnDef::new(Alias::new(name)).string_len(len).not_null());
        }
        table.col(long_text("models", backend).not_null().to_owned());
        m.create_table(table.to_owned()).await?;
        m.create_index(
            Index::create()
                .name("ai_model_profiles_workspace")
                .table(Alias::new("ai_model_profiles"))
                .col(Alias::new("workspace_id"))
                .col(Alias::new("id"))
                .to_owned(),
        )
        .await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
