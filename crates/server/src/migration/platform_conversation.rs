use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m
            .has_column("platform_state", "conversation_config")
            .await?
        {
            m.alter_table(
                Table::alter()
                    .table(Alias::new("platform_state"))
                    .add_column(
                        ColumnDef::new(Alias::new("conversation_config"))
                            .text()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m
            .has_column("platform_state", "conversation_config")
            .await?
        {
            return Ok(());
        }
        let configured = m
            .get_connection()
            .query_one_raw(sea_orm::Statement::from_string(
                m.get_database_backend(),
                "SELECT id FROM platform_state WHERE conversation_config IS NOT NULL LIMIT 1"
                    .to_owned(),
            ))
            .await?;
        if configured.is_some() {
            return Err(DbErr::Custom(
                "Retain configured conversation instructions and approval policy".into(),
            ));
        }
        m.alter_table(
            Table::alter()
                .table(Alias::new("platform_state"))
                .drop_column(Alias::new("conversation_config"))
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
    async fn repeatable_migration_preserves_existing_settings_and_refuses_data_loss() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared(
            "CREATE TABLE platform_state (id BIGINT PRIMARY KEY, site_config TEXT)",
        )
        .await
        .unwrap();
        db.execute_unprepared("INSERT INTO platform_state VALUES (1, 'keep')")
            .await
            .unwrap();
        let m = SchemaManager::new(&db);
        Migration.up(&m).await.unwrap();
        Migration.up(&m).await.unwrap();
        Migration.down(&m).await.unwrap();
        Migration.up(&m).await.unwrap();
        db.execute_unprepared("UPDATE platform_state SET conversation_config='{}'")
            .await
            .unwrap();
        assert!(Migration.down(&m).await.is_err());
        let row = db
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DbBackend::Sqlite,
                "SELECT site_config,conversation_config FROM platform_state".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<String>("", "site_config").unwrap(), "keep");
        assert_eq!(
            row.try_get::<String>("", "conversation_config").unwrap(),
            "{}"
        );
    }
}
