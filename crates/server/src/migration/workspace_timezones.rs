//! NULL preserves historical behavior until an administrator confirms the zone.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager.has_column("workspaces", "timezone").await? {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new("workspaces"))
                        .add_column(ColumnDef::new(Alias::new("timezone")).string_len(64).null())
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Workspace business-time configuration is retained; automatic rollback is not supported".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, sea_query::Expr};

    #[tokio::test]
    async fn workspace_timezone_migration_preserves_history_and_refuses_destructive_rollback() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let manager = SchemaManager::new(&s.db);
        Migration.up(&manager).await.unwrap();
        crate::entities::workspaces::Entity::update_many()
            .col_expr(
                crate::entities::workspaces::Column::Timezone,
                Expr::value("Asia/Kuala_Lumpur"),
            )
            .filter(crate::entities::workspaces::Column::Id.eq(1_i64))
            .exec(&s.db)
            .await
            .unwrap();
        assert!(Migration.down(&manager).await.is_err());
        Migration.up(&manager).await.unwrap();
        let rows =
            s.db.query_all_raw(sea_orm::Statement::from_string(
                s.db.get_database_backend(),
                "SELECT id,timezone FROM workspaces ORDER BY id".to_owned(),
            ))
            .await
            .unwrap();
        assert_eq!(
            rows[0]
                .try_get::<Option<String>>("", "timezone")
                .unwrap()
                .as_deref(),
            Some("Asia/Kuala_Lumpur")
        );
        assert!(
            rows[1]
                .try_get::<Option<String>>("", "timezone")
                .unwrap()
                .is_none()
        );
    }
}
