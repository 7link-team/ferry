use super::*;

pub(crate) struct Migration;
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "ai_chat_api_context_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        let mut table = create_table("ai_chat_api_sessions", backend);
        table.col(
            ColumnDef::new(Alias::new("id"))
                .big_integer()
                .not_null()
                .primary_key(),
        );
        for name in [
            "workspace_id",
            "conversation_id",
            "agent_id",
            "model_profile_id",
            "provider_id",
        ] {
            table.col(ColumnDef::new(Alias::new(name)).big_integer().not_null());
        }
        table.col(
            ColumnDef::new(Alias::new("model"))
                .string_len(256)
                .not_null(),
        );
        table.col(long_text("encrypted_history", backend).not_null());
        for name in [
            "last_turn_id",
            "created_at",
            "created_by",
            "updated_at",
            "updated_by",
        ] {
            table.col(
                ColumnDef::new(Alias::new(name))
                    .big_integer()
                    .not_null()
                    .default(0),
            );
        }
        table.col(
            ColumnDef::new(Alias::new("version"))
                .big_integer()
                .not_null()
                .default(1),
        );
        m.create_table(table.to_owned()).await?;
        if !m
            .has_index("ai_chat_api_sessions", "ai_chat_api_sessions_identity")
            .await?
        {
            m.create_index(
                Index::create()
                    .name("ai_chat_api_sessions_identity")
                    .table(Alias::new("ai_chat_api_sessions"))
                    .unique()
                    .col(Alias::new("workspace_id"))
                    .col(Alias::new("conversation_id"))
                    .col(Alias::new("agent_id"))
                    .to_owned(),
            )
            .await?;
        }
        for name in ["conversation_id", "conversation_turn_id"] {
            if !m.has_column("ai_calls", name).await? {
                m.alter_table(
                    Table::alter()
                        .table(Alias::new("ai_calls"))
                        .add_column(
                            ColumnDef::new(Alias::new(name))
                                .big_integer()
                                .not_null()
                                .default(0),
                        )
                        .to_owned(),
                )
                .await?;
            }
        }
        Ok(())
    }
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let conn = m.get_connection();
        for (table, predicate) in [
            ("ai_chat_api_sessions", ""),
            ("ai_calls", " WHERE conversation_id <> 0"),
        ] {
            if !m.has_table(table).await?
                || (table == "ai_calls" && !m.has_column(table, "conversation_id").await?)
            {
                continue;
            }
            if conn
                .query_one_raw(sea_orm::Statement::from_string(
                    m.get_database_backend(),
                    format!("SELECT id FROM {table}{predicate} LIMIT 1"),
                ))
                .await?
                .is_some()
            {
                return Err(DbErr::Custom(
                    "Retain API conversation history and call provenance before rollback".into(),
                ));
            }
        }
        m.drop_table(
            Table::drop()
                .if_exists()
                .table(Alias::new("ai_chat_api_sessions"))
                .to_owned(),
        )
        .await?;
        for name in ["conversation_id", "conversation_turn_id"] {
            if m.has_column("ai_calls", name).await? {
                m.alter_table(
                    Table::alter()
                        .table(Alias::new("ai_calls"))
                        .drop_column(Alias::new(name))
                        .to_owned(),
                )
                .await?;
            }
        }
        Ok(())
    }
}
