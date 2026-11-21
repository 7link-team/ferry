use super::*;

pub struct Migration;
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "ai_model_max_output_tokens_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m
            .has_column("ai_model_profiles", "max_output_tokens")
            .await?
        {
            m.alter_table(
                Table::alter()
                    .table(Alias::new("ai_model_profiles"))
                    .add_column(
                        ColumnDef::new(Alias::new("max_output_tokens"))
                            .big_integer()
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
            .has_column("ai_model_profiles", "max_output_tokens")
            .await?
        {
            return Ok(());
        }
        if m.get_connection()
            .query_one_raw(sea_orm::Statement::from_string(
                m.get_database_backend(),
                "SELECT id FROM ai_model_profiles WHERE max_output_tokens IS NOT NULL LIMIT 1"
                    .to_owned(),
            ))
            .await?
            .is_some()
        {
            return Err(DbErr::Custom(
                "Retain configured model output token limits before rollback".into(),
            ));
        }
        m.alter_table(
            Table::alter()
                .table(Alias::new("ai_model_profiles"))
                .drop_column(Alias::new("max_output_tokens"))
                .to_owned(),
        )
        .await
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, EntityTrait};
    #[tokio::test]
    async fn nullable_output_setting_migrates_existing_profiles_without_values_and_guards_downgrade()
     {
        let (_dir, s) = crate::agents::tests::setup().await;
        let m = SchemaManager::new(&s.db);
        let rows = crate::entities::ai_model_profiles::Entity::find()
            .all(&s.db)
            .await
            .unwrap();
        assert!(!rows.is_empty());
        assert!(
            rows.iter()
                .all(|profile| profile.max_output_tokens.is_none())
        );
        s.db.execute_unprepared("UPDATE ai_model_profiles SET max_output_tokens=1")
            .await
            .unwrap();
        assert!(
            Migration
                .down(&m)
                .await
                .unwrap_err()
                .to_string()
                .contains("output token")
        );
        assert!(
            m.has_column("ai_model_profiles", "max_output_tokens")
                .await
                .unwrap()
        );
    }
    #[tokio::test]
    async fn existing_profile_rows_gain_null_output_setting_without_rewrite_and_empty_setting_can_rollback()
     {
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        let m = SchemaManager::new(&db);
        super::super::ai_models::Migration.up(&m).await.unwrap();
        db.execute_unprepared("INSERT INTO ai_model_profiles (id,workspace_id,provider_id,revision_id,version,created_at,updated_at,name,kind,runtime,default_model,tool_version,state,models) VALUES (1,1,0,0,7,11,12,'Existing API','api','','m','','enabled','[\"m\"]')").await.unwrap();
        Migration.up(&m).await.unwrap();
        Migration.up(&m).await.unwrap();
        let row=db.query_one_raw(sea_orm::Statement::from_string(DbBackend::Sqlite,"SELECT max_output_tokens,version,created_at,updated_at FROM ai_model_profiles WHERE id=1".to_owned())).await.unwrap().unwrap();
        assert_eq!(
            row.try_get::<Option<i64>>("", "max_output_tokens").unwrap(),
            None
        );
        assert_eq!(row.try_get::<i64>("", "version").unwrap(), 7);
        assert_eq!(row.try_get::<i64>("", "created_at").unwrap(), 11);
        assert_eq!(row.try_get::<i64>("", "updated_at").unwrap(), 12);
        Migration.down(&m).await.unwrap();
        assert!(
            !m.has_column("ai_model_profiles", "max_output_tokens")
                .await
                .unwrap()
        );
    }
}
