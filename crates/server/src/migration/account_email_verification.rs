use super::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            create_table("account_email_verifications", m.get_database_backend())
                .col(
                    ColumnDef::new(Alias::new("user_id"))
                        .big_integer()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(Alias::new("email")).text().not_null())
                .col(
                    ColumnDef::new(Alias::new("verified_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("verified_by"))
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(
                            Alias::new("account_email_verifications"),
                            Alias::new("user_id"),
                        )
                        .to(Alias::new("users"), Alias::new("id"))
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m.has_table("account_email_verifications").await? {
            return Ok(());
        }
        if m.get_connection()
            .query_one_raw(sea_orm::Statement::from_string(
                m.get_database_backend(),
                "SELECT user_id FROM account_email_verifications LIMIT 1".to_owned(),
            ))
            .await?
            .is_some()
        {
            return Err(DbErr::Custom(
                "Retain account email verification evidence before rollback".into(),
            ));
        }
        m.drop_table(
            Table::drop()
                .table(Alias::new("account_email_verifications"))
                .to_owned(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::ConnectionTrait;
    #[tokio::test]
    async fn verification_migration_is_idempotent_and_refuses_to_erase_evidence() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let manager = SchemaManager::new(&s.db);
        Migration.up(&manager).await.unwrap();
        s.db.execute_unprepared("INSERT INTO account_email_verifications (user_id,email,verified_at,verified_by) VALUES (1,'user1@example.invalid',123,1)").await.unwrap();
        assert!(Migration.down(&manager).await.is_err());
        assert!(
            s.db.query_one_raw(sea_orm::Statement::from_string(
                s.db.get_database_backend(),
                "SELECT user_id FROM account_email_verifications".to_owned()
            ))
            .await
            .unwrap()
            .is_some()
        );
        s.db.execute_unprepared("DELETE FROM account_email_verifications")
            .await
            .unwrap();
        Migration.down(&manager).await.unwrap();
        assert!(
            !manager
                .has_table("account_email_verifications")
                .await
                .unwrap()
        );
        Migration.up(&manager).await.unwrap();
        assert!(
            manager
                .has_table("account_email_verifications")
                .await
                .unwrap()
        );
    }
}
