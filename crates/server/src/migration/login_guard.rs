use super::*;

pub struct Migration;
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "login_guard_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m.has_column("platform_state", "login_security").await? {
            m.alter_table(
                Table::alter()
                    .table(Alias::new("platform_state"))
                    .add_column(long_text("login_security", m.get_database_backend()).null())
                    .to_owned(),
            )
            .await?;
        }
        m.create_table(
            create_table("login_guards", m.get_database_backend())
                .col(
                    ColumnDef::new(Alias::new("id"))
                        .string_len(160)
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(Alias::new("count")).big_integer().not_null())
                .col(ColumnDef::new(Alias::new("payload")).text().null())
                .col(
                    ColumnDef::new(Alias::new("expires_at"))
                        .big_integer()
                        .not_null(),
                )
                .to_owned(),
        )
        .await?;
        if !m.has_index("login_guards", "login_guard_expiry").await? {
            m.create_index(
                Index::create()
                    .name("login_guard_expiry")
                    .table(Alias::new("login_guards"))
                    .col(Alias::new("expires_at"))
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
