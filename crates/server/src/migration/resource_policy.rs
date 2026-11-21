use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m.has_column("platform_state", "resource_config").await? {
            m.alter_table(
                Table::alter()
                    .table(Alias::new("platform_state"))
                    .add_column(ColumnDef::new(Alias::new("resource_config")).text().null())
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m.has_column("platform_state", "resource_config").await? {
            return Ok(());
        }
        if m.get_connection()
            .query_one_raw(sea_orm::Statement::from_string(
                m.get_database_backend(),
                "SELECT id FROM platform_state WHERE resource_config IS NOT NULL LIMIT 1",
            ))
            .await?
            .is_some()
        {
            return Err(DbErr::Custom("Retain configured resource policy".into()));
        }
        m.alter_table(
            Table::alter()
                .table(Alias::new("platform_state"))
                .drop_column(Alias::new("resource_config"))
                .to_owned(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database};
    #[tokio::test]
    async fn resource_policy_migration_is_repeatable_and_preserves_legacy_and_new_configuration() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared(
            "CREATE TABLE platform_state (id BIGINT PRIMARY KEY, conversation_config TEXT)",
        )
        .await
        .unwrap();
        db.execute_unprepared("INSERT INTO platform_state VALUES (1, 'legacy')")
            .await
            .unwrap();
        let m = SchemaManager::new(&db);
        Migration.up(&m).await.unwrap();
        Migration.up(&m).await.unwrap();
        let row = db
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DbBackend::Sqlite,
                "SELECT conversation_config,resource_config FROM platform_state",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.try_get::<String>("", "conversation_config").unwrap(),
            "legacy"
        );
        assert_eq!(
            row.try_get::<Option<String>>("", "resource_config")
                .unwrap(),
            None
        );
        Migration.down(&m).await.unwrap();
        Migration.down(&m).await.unwrap();
        Migration.up(&m).await.unwrap();
        db.execute_unprepared("UPDATE platform_state SET resource_config='{\"file_count\":3}'")
            .await
            .unwrap();
        assert!(Migration.down(&m).await.is_err());
        assert!(
            m.has_column("platform_state", "resource_config")
                .await
                .unwrap()
        );
    }
}
