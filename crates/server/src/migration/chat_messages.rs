use super::*;
pub struct Migration;
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "ai_chat_messages_v3"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for (name, column) in [
            (
                "role",
                ColumnDef::new(Alias::new("role"))
                    .string_len(16)
                    .not_null()
                    .default("user")
                    .to_owned(),
            ),
            (
                "agent_id",
                ColumnDef::new(Alias::new("agent_id"))
                    .big_integer()
                    .not_null()
                    .default(0i64)
                    .to_owned(),
            ),
            (
                "turn_id",
                ColumnDef::new(Alias::new("turn_id"))
                    .big_integer()
                    .null()
                    .to_owned(),
            ),
            (
                "reply_to",
                ColumnDef::new(Alias::new("reply_to"))
                    .big_integer()
                    .null()
                    .to_owned(),
            ),
            (
                "sender_name",
                ColumnDef::new(Alias::new("sender_name"))
                    .string_len(128)
                    .not_null()
                    .default("")
                    .to_owned(),
            ),
            (
                "text_items",
                long_text("text_items", m.get_database_backend())
                    .null()
                    .to_owned(),
            ),
        ] {
            if !m.has_column("ai_chat_messages", name).await? {
                m.alter_table(
                    Table::alter()
                        .table(Alias::new("ai_chat_messages"))
                        .add_column(&mut column.to_owned())
                        .to_owned(),
                )
                .await?;
            }
        }
        if m.has_index("ai_chat_messages", "ai_chat_messages_client")
            .await?
        {
            m.drop_index(
                Index::drop()
                    .name("ai_chat_messages_client")
                    .table(Alias::new("ai_chat_messages"))
                    .to_owned(),
            )
            .await?;
        }
        for (name, columns) in [
            (
                "ai_chat_messages_actor_client",
                vec!["conversation_id", "owner_id", "role", "client_id"],
            ),
            (
                "ai_chat_messages_turn_role",
                vec!["conversation_id", "turn_id", "role"],
            ),
        ] {
            if !m.has_index("ai_chat_messages", name).await? {
                let mut index = Index::create();
                index
                    .name(name)
                    .table(Alias::new("ai_chat_messages"))
                    .unique();
                for column in columns {
                    index.col(Alias::new(column));
                }
                m.create_index(index.to_owned()).await?;
            }
        }
        if !m
            .has_index("ai_chat_messages", "ai_chat_messages_history")
            .await?
        {
            m.create_index(
                Index::create()
                    .name("ai_chat_messages_history")
                    .table(Alias::new("ai_chat_messages"))
                    .col(Alias::new("conversation_id"))
                    .col(Alias::new("id"))
                    .to_owned(),
            )
            .await?;
        }
        super::chat_interactions::columns(m).await?;
        crate::conversations::messages::backfill(m.get_connection()).await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
