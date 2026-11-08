use super::*;
pub struct Migration;
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "ai_chat_v2"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        for (name, numbers, strings, texts) in [
            (
                "ai_chat_members",
                vec!["workspace_id", "conversation_id", "user_id"],
                vec![],
                vec![],
            ),
            (
                "ai_chat_agents",
                vec!["workspace_id", "conversation_id", "agent_id", "revision_id"],
                vec![],
                vec!["config", "binding", "native_session_id", "state"],
            ),
            (
                "ai_chat_messages",
                vec!["workspace_id", "conversation_id", "owner_id", "created_at"],
                vec![("client_id", 128)],
                vec!["prompt", "attachments", "targets"],
            ),
            (
                "ai_chat_turns",
                vec![
                    "workspace_id",
                    "conversation_id",
                    "message_id",
                    "chat_agent_id",
                ],
                vec![],
                vec![],
            ),
            (
                "ai_chat_files",
                vec![
                    "workspace_id",
                    "conversation_id",
                    "owner_id",
                    "asset_id",
                    "size",
                    "created_at",
                ],
                vec![("name", 255), ("mime", 128)],
                vec![],
            ),
        ] {
            let mut table = create_table(name, backend);
            table.col(
                ColumnDef::new(Alias::new("id"))
                    .big_integer()
                    .not_null()
                    .primary_key(),
            );
            for n in numbers {
                table.col(ColumnDef::new(Alias::new(n)).big_integer().not_null());
            }
            for (n, size) in strings {
                table.col(ColumnDef::new(Alias::new(n)).string_len(size).not_null());
            }
            for n in texts {
                table.col(long_text(n, backend).not_null().to_owned());
            }
            m.create_table(table.to_owned()).await?;
        }
        for (table, name, columns, unique) in [
            (
                "ai_chat_members",
                "ai_chat_members_identity",
                vec!["conversation_id", "user_id"],
                true,
            ),
            (
                "ai_chat_members",
                "ai_chat_members_user",
                vec!["workspace_id", "user_id", "conversation_id"],
                false,
            ),
            (
                "ai_chat_agents",
                "ai_chat_agents_identity",
                vec!["conversation_id", "agent_id"],
                true,
            ),
            (
                "ai_chat_messages",
                "ai_chat_messages_client",
                vec!["conversation_id", "owner_id", "client_id"],
                true,
            ),
            (
                "ai_chat_turns",
                "ai_chat_turns_message",
                vec!["conversation_id", "message_id"],
                false,
            ),
            (
                "ai_chat_files",
                "ai_chat_files_conversation",
                vec!["workspace_id", "conversation_id"],
                false,
            ),
        ] {
            if !m.has_index(table, name).await? {
                let mut index = Index::create();
                index.name(name).table(Alias::new(table));
                for c in columns {
                    index.col(Alias::new(c));
                }
                if unique {
                    index.unique();
                }
                m.create_index(index.to_owned()).await?;
            }
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
