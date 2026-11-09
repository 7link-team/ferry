use super::*;

/// Who opened a preview and its review outcome (docs/preview-automation-design.md
/// section 3). Existing rows become manual previews with no review.
pub struct Migration;
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "holes_v2"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let add = |column: &mut ColumnDef| {
            Table::alter()
                .table(Alias::new("holes"))
                .add_column(column)
                .to_owned()
        };
        let columns = [
            (
                "source_kind",
                add(ColumnDef::new(Alias::new("source_kind"))
                    .string_len(16)
                    .not_null()
                    .default("manual")),
            ),
            (
                "source_id",
                add(ColumnDef::new(Alias::new("source_id"))
                    .big_integer()
                    .not_null()
                    .default(0_i64)),
            ),
            (
                "source_ref",
                add(ColumnDef::new(Alias::new("source_ref"))
                    .string_len(128)
                    .not_null()
                    .default("")),
            ),
            (
                "review_state",
                add(ColumnDef::new(Alias::new("review_state"))
                    .string_len(16)
                    .not_null()
                    .default("none")),
            ),
            (
                "reviewed_at",
                add(ColumnDef::new(Alias::new("reviewed_at"))
                    .big_integer()
                    .not_null()
                    .default(0_i64)),
            ),
            (
                "reviewed_by",
                add(ColumnDef::new(Alias::new("reviewed_by"))
                    .big_integer()
                    .not_null()
                    .default(0_i64)),
            ),
        ];
        for (name, statement) in columns {
            execute_schema(m, SchemaStatement::Column("holes", name, statement)).await?;
        }
        execute_schema(
            m,
            SchemaStatement::Index(
                "holes",
                "holes_source",
                Index::create()
                    .name("holes_source")
                    .table(Alias::new("holes"))
                    .col(Alias::new("source_kind"))
                    .col(Alias::new("source_id"))
                    .to_owned(),
            ),
        )
        .await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
