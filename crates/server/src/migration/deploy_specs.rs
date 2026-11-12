//! Blueprint deployments created from a deployment spec keep that spec beside the manifest.
use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::ConnectionTrait;
#[derive(DeriveMigrationName)]
pub struct Migration;

const TABLE: &str = "deployments";
const COLUMN: &str = "deploy_spec";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if m.has_column(TABLE, COLUMN).await? {
            return Ok(());
        }
        // NULL for deployments without a spec; MySQL TEXT cannot carry a default or an index.
        let mut column = ColumnDef::new(Alias::new(COLUMN));
        column.text().null();
        if m.get_database_backend() == sea_orm::DbBackend::MySql {
            column.custom(Alias::new("longtext"));
        }
        m.alter_table(
            Table::alter()
                .table(Alias::new(TABLE))
                .add_column(column)
                .to_owned(),
        )
        .await
    }
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if m.has_column(TABLE, COLUMN).await? {
            // Spec §9.4: a rollback never deletes business data; stored specs must survive.
            let stored = Query::select()
                .expr(Expr::val(1))
                .from(Alias::new(TABLE))
                .and_where(Expr::col(Alias::new(COLUMN)).is_not_null())
                .limit(1)
                .to_owned();
            let db = m.get_connection();
            if db.query_one(&stored).await?.is_some() {
                return Err(DbErr::Custom("不自动删除业务数据".into()));
            }
            m.alter_table(
                Table::alter()
                    .table(Alias::new(TABLE))
                    .drop_column(Alias::new(COLUMN))
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Statement};

    fn read(row: &sea_orm::QueryResult) -> Option<String> {
        row.try_get::<Option<String>>("", COLUMN).unwrap()
    }

    #[tokio::test]
    async fn spec_migration_roundtrip() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let manager = SchemaManager::new(&s.db);
        let backend = s.db.get_database_backend();
        s.db.execute_raw(Statement::from_string(backend, "INSERT INTO deployments (id,workspace_id,name,category,manifest,preview_artifact,mode,target_kind,target_ref,workdir,description,state,revision_number,current_revision,created_at,updated_at,version) VALUES (73,1,'web','','{}','','compose','host',1,'/srv','','idle',0,0,1,1,1)")).await.unwrap();
        // Idempotent in both directions while no spec is stored.
        Migration.down(&manager).await.unwrap();
        assert!(!manager.has_column(TABLE, COLUMN).await.unwrap());
        Migration.down(&manager).await.unwrap();
        Migration.up(&manager).await.unwrap();
        Migration.up(&manager).await.unwrap();
        let select = "SELECT name, deploy_spec FROM deployments WHERE id = 73";
        let row =
            s.db.query_one_raw(Statement::from_string(backend, select))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(row.try_get::<String>("", "name").unwrap(), "web");
        assert_eq!(read(&row), None);
        let spec = "apiVersion: ferry/deploy-v1\ndriver: docker\n";
        s.db.execute_raw(Statement::from_string(
            backend,
            format!("UPDATE deployments SET deploy_spec = '{spec}' WHERE id = 73"),
        ))
        .await
        .unwrap();
        let row =
            s.db.query_one_raw(Statement::from_string(backend, select))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(read(&row).as_deref(), Some(spec));
        // A stored spec is business data: down refuses and keeps it.
        let err = Migration.down(&manager).await.expect_err("must refuse");
        assert!(err.to_string().contains("不自动删除业务数据"), "{err}");
        assert!(manager.has_column(TABLE, COLUMN).await.unwrap());
        let row =
            s.db.query_one_raw(Statement::from_string(backend, select))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(read(&row).as_deref(), Some(spec));
    }
}
