//! Workspace presentation; existing rows keep their neutral default identity.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager.has_column("workspaces", "avatar").await? {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new("workspaces"))
                        .add_column(
                            super::long_text("avatar", manager.get_database_backend()).null(),
                        )
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Workspace avatars are retained; automatic destructive rollback is not supported"
                .into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};
    #[tokio::test]
    async fn workspace_avatar_migration_preserves_existing_rows_and_rejects_destructive_down() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let manager = SchemaManager::new(&s.db);
        Migration.up(&manager).await.unwrap();
        use crate::entities::workspaces;
        assert!(
            workspaces::Entity::find_by_id(1_i64)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap()
                .avatar
                .is_none()
        );
        let avatar = r#"{"style":"initials"}"#;
        workspaces::Entity::update_many()
            .col_expr(workspaces::Column::Avatar, Expr::value(Some(avatar)))
            .filter(workspaces::Column::Id.eq(1_i64))
            .exec(&s.db)
            .await
            .unwrap();
        assert!(Migration.down(&manager).await.is_err());
        Migration.up(&manager).await.unwrap();
        assert_eq!(
            workspaces::Entity::find_by_id(1_i64)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap()
                .avatar
                .as_deref(),
            Some(avatar)
        );
    }
}
