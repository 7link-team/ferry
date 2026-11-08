use super::*;

pub struct Migration;
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "ai_agents_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        let mut table = create_table("ai_agents", backend);
        table.col(
            ColumnDef::new(Alias::new("id"))
                .big_integer()
                .not_null()
                .primary_key(),
        );
        for name in [
            "workspace_id",
            "host_id",
            "owner_id",
            "revision_id",
            "version",
            "created_at",
            "updated_at",
        ] {
            table.col(ColumnDef::new(Alias::new(name)).big_integer().not_null());
        }
        for (name, size) in [
            ("name", 128),
            ("role", 256),
            ("runtime", 16),
            ("model", 256),
            ("permission", 24),
            ("state", 16),
            ("visibility", 16),
        ] {
            table.col(ColumnDef::new(Alias::new(name)).string_len(size).not_null());
        }
        for name in ["description", "cwd", "system_prompt"] {
            table.col(long_text(name, backend).not_null().to_owned());
        }
        m.create_table(table.to_owned()).await?;
        m.create_index(
            Index::create()
                .name("ai_agents_workspace")
                .table(Alias::new("ai_agents"))
                .col(Alias::new("workspace_id"))
                .col(Alias::new("id"))
                .to_owned(),
        )
        .await?;
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
