//! Room threads for pipelines/workflows created or run from a room, the runs
//! started there, and which room messages belong to a thread. Conversation-side
//! only: pipeline and workflow tables are unchanged, so deleting a room never
//! touches them.
use super::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        let big = |name: &str| {
            ColumnDef::new(Alias::new(name))
                .big_integer()
                .not_null()
                .to_owned()
        };
        let mut threads = create_table("ai_chat_threads", backend);
        threads
            .col(big("id").primary_key().to_owned())
            .col(big("workspace_id"))
            .col(big("conversation_id"))
            .col(ColumnDef::new(Alias::new("kind")).string_len(16).not_null())
            .col(big("resource_id"))
            .col(
                ColumnDef::new(Alias::new("linked"))
                    .small_integer()
                    .not_null()
                    .default(0),
            )
            .col(big("created_by"))
            .col(big("created_at"));
        m.create_table(threads.to_owned()).await?;
        let mut runs = create_table("ai_chat_run_links", backend);
        runs.col(big("id").primary_key().to_owned())
            .col(big("workspace_id"))
            .col(big("conversation_id"))
            .col(big("thread_id"))
            .col(ColumnDef::new(Alias::new("kind")).string_len(24).not_null())
            .col(big("turn_id"))
            .col(big("chat_agent_id"))
            .col(big("hop"))
            .col(
                ColumnDef::new(Alias::new("notified_at"))
                    .big_integer()
                    .null(),
            );
        m.create_table(runs.to_owned()).await?;
        let mut items = create_table("ai_chat_thread_items", backend);
        items
            .col(big("message_id").primary_key().to_owned())
            .col(big("thread_id"));
        m.create_table(items.to_owned()).await?;
        for (table, name, columns, unique) in [
            (
                "ai_chat_threads",
                "ai_chat_threads_resource",
                vec!["conversation_id", "kind", "resource_id"],
                true,
            ),
            (
                "ai_chat_threads",
                "ai_chat_threads_origin",
                vec!["workspace_id", "kind", "resource_id"],
                false,
            ),
            (
                "ai_chat_run_links",
                "ai_chat_run_links_pending",
                vec!["notified_at"],
                false,
            ),
            (
                "ai_chat_thread_items",
                "ai_chat_thread_items_thread",
                vec!["thread_id"],
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
        Err(DbErr::Custom(
            "Room threads are retained; automatic destructive rollback is not supported".into(),
        ))
    }
}
