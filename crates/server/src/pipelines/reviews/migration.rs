//! Append-only module migration, registered by the server migrator.
use sea_orm_migration::prelude::*;
pub struct Migration;
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
