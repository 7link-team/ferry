//! Retired insight-report schema retained for released database migrations.
mod entities;
pub(crate) mod migration;

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectionTrait, Database, EntityTrait};
    use sea_orm_migration::{MigrationTrait, SchemaManager};

    #[tokio::test]
    async fn retired_report_migration_preserves_historical_data() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);
        let migration = super::migration::InsightReports;
        migration.up(&manager).await.unwrap();
        db.execute_unprepared(
            "INSERT INTO insight_report_definitions \
             (id,workspace_id,user_id,name,insight_id,config,enabled,version,\
              next_run_at,last_generated_at,created_at,updated_at) \
             VALUES (1,2,3,'historical',4,'{}',1,1,0,0,0,0)",
        )
        .await
        .unwrap();
        migration.up(&manager).await.unwrap();
        let row = super::entities::definitions::Entity::find_by_id(1)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.name, "historical");
    }
}
