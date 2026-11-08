use super::*;
pub struct Migration;
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "ai_chat_interactions_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        columns(m).await?;
        tables(m).await?;
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
pub(super) async fn columns(m: &SchemaManager<'_>) -> Result<(), DbErr> {
    for (name, column) in [
        (
            "version",
            ColumnDef::new(Alias::new("version"))
                .big_integer()
                .not_null()
                .default(1i64)
                .to_owned(),
        ),
        (
            "edited_at",
            ColumnDef::new(Alias::new("edited_at"))
                .big_integer()
                .null()
                .to_owned(),
        ),
        (
            "retracted_at",
            ColumnDef::new(Alias::new("retracted_at"))
                .big_integer()
                .null()
                .to_owned(),
        ),
        (
            "mentions",
            ColumnDef::new(Alias::new("mentions"))
                .string_len(4096)
                .not_null()
                .default("[]")
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
    Ok(())
}
async fn tables(m: &SchemaManager<'_>) -> Result<(), DbErr> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
    for (table, key) in [
        ("ai_chat_reads", "conversation_id"),
        ("ai_chat_saves", "message_id"),
    ] {
        let mut t = create_table(table, m.get_database_backend());
        for column in [key, "user_id", "workspace_id"] {
            t.col(ColumnDef::new(Alias::new(column)).big_integer().not_null());
        }
        t.col(
            ColumnDef::new(Alias::new(if table == "ai_chat_reads" {
                "message_id"
            } else {
                "conversation_id"
            }))
            .big_integer()
            .not_null(),
        );
        t.primary_key(
            Index::create()
                .col(Alias::new(key))
                .col(Alias::new("user_id")),
        );
        m.create_table(t.to_owned()).await?;
    }
    if !m.has_index("ai_chat_saves", "ai_chat_saves_reader").await? {
        m.create_index(
            Index::create()
                .name("ai_chat_saves_reader")
                .table(Alias::new("ai_chat_saves"))
                .col(Alias::new("conversation_id"))
                .col(Alias::new("user_id"))
                .to_owned(),
        )
        .await?;
    }
    // Preserve authors of old messages when they are later removed from a room.
    use crate::entities::{ai_chat_messages as chat, users};
    let mut after = 0;
    loop {
        let batch = chat::Entity::find()
            .filter(chat::Column::Id.gt(after))
            .filter(chat::Column::Role.eq("user"))
            .filter(chat::Column::SenderName.eq(""))
            .order_by_asc(chat::Column::Id)
            .limit(100)
            .all(m.get_connection())
            .await?;
        if batch.is_empty() {
            break;
        }
        for row in batch {
            after = row.id;
            if let Some(user) = users::Entity::find_by_id(row.owner_id)
                .one(m.get_connection())
                .await?
            {
                chat::Entity::update_many()
                    .col_expr(
                        chat::Column::SenderName,
                        sea_orm::sea_query::Expr::value(user.name),
                    )
                    .filter(chat::Column::Id.eq(row.id))
                    .exec(m.get_connection())
                    .await?;
            }
        }
    }
    Ok(())
}
