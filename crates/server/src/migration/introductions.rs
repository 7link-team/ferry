use super::*;

pub(crate) struct Migration;
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "ai_conversation_introductions_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for (name, column) in [
            (
                "purpose",
                ColumnDef::new(Alias::new("purpose"))
                    .string_len(16)
                    .not_null()
                    .default("message")
                    .to_owned(),
            ),
            (
                "introduction_id",
                ColumnDef::new(Alias::new("introduction_id"))
                    .big_integer()
                    .not_null()
                    .default(0i64)
                    .to_owned(),
            ),
            (
                "introduction_locale",
                ColumnDef::new(Alias::new("introduction_locale"))
                    .string_len(16)
                    .not_null()
                    .default("")
                    .to_owned(),
            ),
        ] {
            if !m.has_column("ai_turns", name).await? {
                m.alter_table(
                    Table::alter()
                        .table(Alias::new("ai_turns"))
                        .add_column(column.to_owned())
                        .to_owned(),
                )
                .await?;
            }
        }
        if !m.has_index("ai_turns", "ai_turns_introduction").await? {
            m.create_index(
                Index::create()
                    .name("ai_turns_introduction")
                    .table(Alias::new("ai_turns"))
                    .col(Alias::new("conversation_id"))
                    .col(Alias::new("introduction_id"))
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        // Check each existing column independently after a partially applied upgrade.
        for (column, predicate) in [
            ("purpose", "purpose <> 'message'"),
            ("introduction_id", "introduction_id <> 0"),
            ("introduction_locale", "introduction_locale <> ''"),
        ] {
            if m.has_column("ai_turns", column).await?
                && m.get_connection()
                    .query_one_raw(sea_orm::Statement::from_string(
                        m.get_database_backend(),
                        format!("SELECT id FROM ai_turns WHERE {predicate} LIMIT 1"),
                    ))
                    .await?
                    .is_some()
            {
                return Err(DbErr::Custom(
                    "Retain introduction tasks and membership provenance before rollback".into(),
                ));
            }
        }
        if m.has_index("ai_turns", "ai_turns_introduction").await? {
            m.drop_index(
                Index::drop()
                    .name("ai_turns_introduction")
                    .table(Alias::new("ai_turns"))
                    .to_owned(),
            )
            .await?;
        }
        for name in ["purpose", "introduction_id", "introduction_locale"] {
            if m.has_column("ai_turns", name).await? {
                m.alter_table(
                    Table::alter()
                        .table(Alias::new("ai_turns"))
                        .drop_column(Alias::new(name))
                        .to_owned(),
                )
                .await?;
            }
        }
        Ok(())
    }
}
