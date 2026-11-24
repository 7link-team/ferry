//! Room Agents belong to the developer who added them (their sponsor), keep
//! their own approval mode, and relay hops are recorded. A held relay
//! delivery is a turn in state `held`, so it needs no extra column.
//! Existing room Agents are sponsored by the conversation owner and keep the
//! room's current approval mode.
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const OLD_IDENTITY: &str = "ai_chat_agents_identity";
const IDENTITY: &str = "ai_chat_agents_sponsor_identity";

fn number(name: &str) -> ColumnDef {
    ColumnDef::new(Alias::new(name))
        .big_integer()
        .not_null()
        .default(0)
        .to_owned()
}

async fn add(m: &SchemaManager<'_>, table: &str, column: ColumnDef) -> Result<(), DbErr> {
    let name = column.get_column_name();
    if !m.has_column(table, &name).await? {
        m.alter_table(
            Table::alter()
                .table(Alias::new(table))
                .add_column(column)
                .to_owned(),
        )
        .await?;
    }
    Ok(())
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        add(m, "ai_chat_agents", number("added_by")).await?;
        add(
            m,
            "ai_chat_agents",
            ColumnDef::new(Alias::new("approval_mode"))
                .string_len(16)
                .not_null()
                .default("request")
                .to_owned(),
        )
        .await?;
        add(m, "ai_chat_turns", number("hop")).await?;
        add(m, "ai_chat_turns", number("caller_chat_agent_id")).await?;

        let db = m.get_connection();
        db.execute_raw(Statement::from_string(
            backend,
            "UPDATE ai_chat_agents SET added_by = (SELECT c.owner_id FROM ai_conversations c \
             WHERE c.id = ai_chat_agents.conversation_id) WHERE added_by = 0",
        ))
        .await?;
        // Copy each room's approval mode onto its Agents once (config is JSON text).
        let rooms = db
            .query_all_raw(Statement::from_string(
                backend,
                "SELECT c.id, c.config FROM ai_conversations c WHERE c.id IN \
                 (SELECT DISTINCT conversation_id FROM ai_chat_agents)",
            ))
            .await?;
        for room in rooms {
            let id: i64 = room.try_get_by_index(0)?;
            let config: String = room.try_get_by_index(1)?;
            let mode = serde_json::from_str::<serde_json::Value>(&config)
                .ok()
                .and_then(|value| value["approval_mode"].as_str().map(str::to_owned))
                .filter(|mode| ["request", "assisted", "yolo"].contains(&mode.as_str()));
            if let Some(mode) = mode {
                let update = Query::update()
                    .table(Alias::new("ai_chat_agents"))
                    .value(Alias::new("approval_mode"), mode)
                    .and_where(Expr::col(Alias::new("conversation_id")).eq(id))
                    .and_where(Expr::col(Alias::new("approval_mode")).eq("request"))
                    .to_owned();
                db.execute_raw(backend.build(&update)).await?;
            }
        }

        if m.has_index("ai_chat_agents", OLD_IDENTITY).await? {
            m.drop_index(
                Index::drop()
                    .name(OLD_IDENTITY)
                    .table(Alias::new("ai_chat_agents"))
                    .to_owned(),
            )
            .await?;
        }
        if !m.has_index("ai_chat_agents", IDENTITY).await? {
            m.create_index(
                Index::create()
                    .name(IDENTITY)
                    .table(Alias::new("ai_chat_agents"))
                    .col(Alias::new("conversation_id"))
                    .col(Alias::new("agent_id"))
                    .col(Alias::new("added_by"))
                    .unique()
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Room Agent sponsors are retained; automatic destructive rollback is not supported"
                .into(),
        ))
    }
}
