use super::*;

pub struct Migration;
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "account_otp_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            create_table("user_otp", m.get_database_backend())
                .col(
                    ColumnDef::new(Alias::new("user_id"))
                        .big_integer()
                        .not_null()
                        .primary_key(),
                )
                .col(long_text("payload", m.get_database_backend()).not_null())
                .col(
                    ColumnDef::new(Alias::new("created_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("created_by"))
                        .big_integer()
                        .not_null()
                        .default(0),
                )
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
                .to_owned(),
        )
        .await
    }
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(
            Table::drop()
                .table(Alias::new("user_otp"))
                .if_exists()
                .to_owned(),
        )
        .await
    }
}
