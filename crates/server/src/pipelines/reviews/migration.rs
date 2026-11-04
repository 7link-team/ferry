//! Append-only module migration, registered by the server migrator.
use sea_orm_migration::prelude::*;

/// Old managed pipelines remain stored, but the retired feature cannot run them.
pub(crate) async fn owns_pipeline(
    db: &impl sea_orm::ConnectionTrait,
    pipeline: i64,
) -> Result<bool, sea_orm::DbErr> {
    use sea_orm::sea_query::{Alias, Expr, Query};
    let query = Query::select()
        .column(Alias::new("id"))
        .from(Alias::new("code_reviews"))
        .and_where(Expr::col(Alias::new("pipeline_id")).eq(pipeline))
        .limit(1)
        .to_owned();
    Ok(db.query_one(&query).await?.is_some())
}
pub struct Migration;

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::ConnectionTrait;

    #[tokio::test]
    async fn retired_managed_pipeline_is_identified_without_removing_history() {
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        Migration.up(&SchemaManager::new(&db)).await.unwrap();
        db.execute_unprepared("INSERT INTO code_reviews (id,workspace_id,user_id,pipeline_id,version,cursor,created_at,updated_at,name,description,config,baselines) VALUES (1,1,1,42,1,0,0,0,'Historic review','','{}','{}')").await.unwrap();
        assert!(owns_pipeline(&db, 42).await.unwrap());
        assert!(!owns_pipeline(&db, 43).await.unwrap());
        // Reapplying the old migration leaves historical rows intact.
        Migration.up(&SchemaManager::new(&db)).await.unwrap();
        assert!(owns_pipeline(&db, 42).await.unwrap());
    }
}
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20261104_code_reviews"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for (table, numbers, texts) in [
            (
                "code_reviews",
                vec![
                    "workspace_id",
                    "user_id",
                    "pipeline_id",
                    "version",
                    "cursor",
                    "created_at",
                    "updated_at",
                ],
                vec!["name", "description", "config", "baselines"],
            ),
            (
                "code_review_runs",
                vec![
                    "workspace_id",
                    "review_id",
                    "pipeline_run_id",
                    "lease_until",
                    "created_at",
                    "finished_at",
                ],
                vec!["dedupe", "state", "snapshot", "result", "lease_owner"],
            ),
            (
                "code_review_findings",
                vec![
                    "workspace_id",
                    "review_id",
                    "connection_id",
                    "first_seen",
                    "last_seen",
                    "version",
                ],
                vec![
                    "repository",
                    "repository_url",
                    "fingerprint",
                    "file",
                    "detail",
                    "status",
                    "history",
                ],
            ),
        ] {
            let mut t = Table::create();
            t.table(Alias::new(table)).if_not_exists().col(
                ColumnDef::new(Alias::new("id"))
                    .big_integer()
                    .not_null()
                    .primary_key(),
            );
            for field in numbers {
                t.col(ColumnDef::new(Alias::new(field)).big_integer().not_null());
            }
            for field in texts {
                let mut c = ColumnDef::new(Alias::new(field));
                if field == "dedupe" || field == "fingerprint" {
                    c.string_len(64);
                } else if field == "state" || field == "status" {
                    c.string_len(32);
                } else if m.get_database_backend() == sea_orm::DbBackend::MySql {
                    c.custom("longtext");
                } else {
                    c.text();
                }
                t.col(c.not_null());
            }
            m.create_table(t.to_owned()).await?;
        }
        for (table, name, fields, unique) in [
            (
                "code_reviews",
                "idx_code_reviews_owner",
                vec!["workspace_id", "user_id"],
                false,
            ),
            (
                "code_review_runs",
                "idx_code_reviews_dedupe",
                vec!["dedupe"],
                true,
            ),
            (
                "code_review_runs",
                "idx_code_reviews_queue",
                vec!["state", "lease_until"],
                false,
            ),
            (
                "code_review_findings",
                "idx_code_reviews_findings",
                vec!["review_id", "fingerprint"],
                true,
            ),
        ] {
            if !m.has_index(table, name).await? {
                let mut i = Index::create();
                i.name(name).table(Alias::new(table));
                for f in fields {
                    i.col(Alias::new(f));
                }
                if unique {
                    i.unique();
                }
                m.create_index(i.to_owned()).await?;
            }
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
