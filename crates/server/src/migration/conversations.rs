use super::*;

pub struct Migration;
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "ai_conversations_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        for (name, numbers, strings, texts) in [
            (
                "ai_conversations",
                &[
                    "workspace_id",
                    "owner_id",
                    "agent_id",
                    "revision_id",
                    "next_sequence",
                    "version",
                    "created_at",
                    "updated_at",
                ][..],
                &[("title", 256), ("state", 24)][..],
                &["config", "binding", "native_session_id"][..],
            ),
            (
                "ai_turns",
                &[
                    "workspace_id",
                    "conversation_id",
                    "owner_id",
                    "expires_at",
                    "version",
                    "created_at",
                    "updated_at",
                ][..],
                &[("state", 24), ("lease_owner", 128), ("client_id", 128)][..],
                &[
                    "prompt",
                    "attachments",
                    "native_session_id",
                    "candidate_session",
                    "error",
                    "usage",
                    "decisions",
                ][..],
            ),
            (
                "ai_events",
                &[
                    "workspace_id",
                    "conversation_id",
                    "turn_id",
                    "sequence",
                    "created_at",
                ][..],
                &[][..],
                &["payload"][..],
            ),
        ] {
            let mut table = create_table(name, backend);
            table.col(
                ColumnDef::new(Alias::new("id"))
                    .big_integer()
                    .not_null()
                    .primary_key(),
            );
            for key in numbers {
                table.col(ColumnDef::new(Alias::new(*key)).big_integer().not_null());
            }
            for (key, size) in strings {
                table.col(
                    ColumnDef::new(Alias::new(*key))
                        .string_len(*size)
                        .not_null(),
                );
            }
            for key in texts {
                table.col(long_text(key, backend).not_null().to_owned());
            }
            m.create_table(table.to_owned()).await?;
        }
        for (table, name, columns, unique) in [
            (
                "ai_conversations",
                "ai_conversations_owner",
                &["workspace_id", "owner_id", "id"][..],
                false,
            ),
            (
                "ai_turns",
                "ai_turns_state",
                &["state", "expires_at"][..],
                false,
            ),
            (
                "ai_turns",
                "ai_turns_client",
                &["conversation_id", "client_id"][..],
                true,
            ),
            (
                "ai_events",
                "ai_events_sequence",
                &["conversation_id", "sequence"][..],
                true,
            ),
        ] {
            if !m.has_index(table, name).await? {
                let mut index = Index::create();
                index.name(name).table(Alias::new(table));
                for column in columns {
                    index.col(Alias::new(*column));
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
