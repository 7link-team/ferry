use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for name in ["account_preferences", "ai_agents"] {
            m.alter_table(Table::alter().table(Alias::new(name)).add_column(ColumnDef::new(Alias::new("avatar")).text().null()).to_owned()).await?;
        }
        Ok(())
    }
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for name in ["ai_agents", "account_preferences"] {
            m.alter_table(Table::alter().table(Alias::new(name)).drop_column(Alias::new("avatar")).to_owned()).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{EntityTrait, PaginatorTrait};
    #[tokio::test]
    async fn avatar_migration_roundtrip_preserves_existing_records() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let manager = SchemaManager::new(&s.db);
        Migration.down(&manager).await.unwrap();
        Migration.up(&manager).await.unwrap();
        assert_eq!(crate::entities::users::Entity::find().count(&s.db).await.unwrap(), 2);
        let avatar = serde_json::from_value(serde_json::json!({"style":"pixel"})).unwrap();
        crate::accounts::preferences::save_avatar(&s.db, 1, &avatar).await.unwrap();
        assert_eq!(crate::accounts::preferences::avatars(&s.db, vec![1]).await.unwrap()[&1]["style"], "pixel");
    }
}
