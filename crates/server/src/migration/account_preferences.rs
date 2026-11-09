use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(Alias::new("account_preferences"))
                .if_not_exists()
                .col(
                    ColumnDef::new(Alias::new("user_id"))
                        .big_integer()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(Alias::new("theme")).text().null())
                .col(ColumnDef::new(Alias::new("locale")).string_len(16).null())
                .col(
                    ColumnDef::new(Alias::new("updated_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("updated_by"))
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(Alias::new("account_preferences"), Alias::new("user_id"))
                        .to(Alias::new("users"), Alias::new("id"))
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await
    }
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(
            Table::drop()
                .table(Alias::new("account_preferences"))
                .to_owned(),
        )
        .await
    }
}
