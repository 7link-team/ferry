//! Platform-level marketplace sources and versioned packages. Additive only.
use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
enum Kind {
    Text(usize, &'static str),
    Number(i64),
    /// MySQL TEXT cannot carry a default; readers treat NULL as the documented default.
    LongText,
}
const COLUMNS: &[(&str, &str, Kind)] = &[
    ("marketplace_sources", "trust", Kind::Text(16, "workspace")),
    ("marketplace_sources", "enabled", Kind::Number(1)),
    ("marketplace_sources", "sync_interval", Kind::Number(0)),
    ("marketplace_sources", "next_sync_at", Kind::Number(0)),
    ("marketplace_sources", "sync_owner", Kind::Text(128, "")),
    ("marketplace_sources", "sync_lease_until", Kind::Number(0)),
    ("marketplace_sources", "created_by", Kind::Number(0)),
    ("marketplace_sources", "updated_by", Kind::Number(0)),
    ("marketplace_sources", "synced_by", Kind::Number(0)),
    (
        "marketplace_entries",
        "package_version",
        Kind::Text(64, "0.0.0"),
    ),
    ("marketplace_entries", "kind", Kind::Text(16, "app")),
    ("marketplace_entries", "meta", Kind::LongText),
    ("marketplace_entries", "files", Kind::LongText),
    ("marketplace_entries", "readme", Kind::LongText),
    ("marketplace_entries", "search_text", Kind::LongText),
    ("marketplace_entries", "compatible", Kind::Number(1)),
    ("marketplace_entries", "digest", Kind::Text(64, "")),
    ("marketplace_entries", "created_by", Kind::Number(0)),
    ("marketplace_entries", "updated_by", Kind::Number(0)),
    ("app_templates", "origin_source_id", Kind::Number(0)),
    ("app_templates", "origin_key", Kind::Text(100, "")),
    ("app_templates", "origin_version", Kind::Text(64, "")),
    ("app_templates", "origin_digest", Kind::Text(64, "")),
    ("config_templates", "origin_source_id", Kind::Number(0)),
    ("config_templates", "origin_key", Kind::Text(100, "")),
    ("config_templates", "origin_version", Kind::Text(64, "")),
    ("config_templates", "origin_digest", Kind::Text(64, "")),
];
const INDEXES: &[(&str, &str, &[&str])] = &[
    (
        "marketplace_entries",
        "idx_marketplace_entries_source",
        &["workspace_id", "source_id"],
    ),
    (
        "marketplace_entries",
        "idx_marketplace_entries_key",
        &["workspace_id", "entry_key", "package_version"],
    ),
    (
        "marketplace_sources",
        "idx_marketplace_sources_due",
        &["workspace_id", "enabled", "next_sync_at"],
    ),
];

/// Columns created as TEXT by `external_repository_marketplace_v1` (LONGTEXT on MySQL).
const TEXT_COLUMNS: &[&str] = &["entry_key"];

/// Index statements for this backend. MySQL cannot index TEXT columns without a prefix length
/// (error 1170), so indexes covering them are skipped there; `idx_marketplace_entries_source`
/// still serves the per-source queries. Applied SQLite/PostgreSQL databases keep theirs.
fn indexes(backend: sea_orm::DbBackend) -> Vec<(&'static str, &'static str, IndexCreateStatement)> {
    INDEXES
        .iter()
        .filter(|(_, _, columns)| {
            backend != sea_orm::DbBackend::MySql
                || !columns.iter().any(|c| TEXT_COLUMNS.contains(c))
        })
        .map(|(table, name, columns)| {
            let mut index = Index::create();
            index.name(*name).table(Alias::new(*table));
            for column in *columns {
                index.col(Alias::new(*column));
            }
            (*table, *name, index.to_owned())
        })
        .collect()
}

/// Built-in official source (user decision 2026-11-11): the public templates in this repository.
/// Inserted once by this migration; a superadmin may disable or delete it and it is not re-created.
const OFFICIAL_SOURCE_ID: i64 = 1;
const OFFICIAL_SOURCE_CONFIG: &str = r#"{"provider":"github","api_base":"https://api.github.com","project":"7link-team/ferry","organization":"","branch":"main","index_path":"marketplace/index.json"}"#;

async fn seed_official_source(m: &SchemaManager<'_>) -> Result<(), DbErr> {
    use sea_orm::{ConnectionTrait, Statement};
    let db = m.get_connection();
    let backend = m.get_database_backend();
    let exists = db
        .query_one_raw(Statement::from_string(
            backend,
            format!("SELECT id FROM marketplace_sources WHERE id = {OFFICIAL_SOURCE_ID}"),
        ))
        .await?
        .is_some();
    if exists {
        return Ok(());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let mut insert = Query::insert();
    insert
        .into_table(Alias::new("marketplace_sources"))
        .columns(
            [
                "id",
                "workspace_id",
                "name",
                "repository_id",
                "config",
                "state",
                "last_error",
                "synced_at",
                "run_as",
                "version",
                "created_at",
                "updated_at",
                "trust",
                "enabled",
                "sync_interval",
                "next_sync_at",
            ]
            .map(Alias::new),
        )
        .values_panic([
            OFFICIAL_SOURCE_ID.into(),
            0_i64.into(),
            "Ferry official templates".into(),
            0_i64.into(),
            OFFICIAL_SOURCE_CONFIG.into(),
            "idle".into(),
            "".into(),
            0_i64.into(),
            0_i64.into(),
            1_i64.into(),
            now.into(),
            now.into(),
            "official".into(),
            1_i64.into(),
            (6 * 3600_i64).into(),
            0_i64.into(),
        ]);
    m.exec_stmt(insert).await
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        for (table, name, kind) in COLUMNS {
            if m.has_column(*table, *name).await? {
                continue;
            }
            let mut column = ColumnDef::new(Alias::new(*name));
            match kind {
                Kind::Text(len, default) => {
                    column.string_len(*len as u32).not_null().default(*default);
                }
                Kind::Number(default) => {
                    column.big_integer().not_null().default(*default);
                }
                Kind::LongText => {
                    column.text().null();
                    if backend == sea_orm::DbBackend::MySql {
                        column.custom(Alias::new("longtext"));
                    }
                }
            }
            m.alter_table(
                Table::alter()
                    .table(Alias::new(*table))
                    .add_column(column)
                    .to_owned(),
            )
            .await?;
        }
        for (table, name, index) in indexes(backend) {
            if !m.has_index(table, name).await? {
                m.create_index(index).await?;
            }
        }
        seed_official_source(m).await
    }
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let db = m.get_connection();
        db.execute_unprepared(&format!("DELETE FROM marketplace_entries WHERE workspace_id = 0 AND source_id = {OFFICIAL_SOURCE_ID}")).await?;
        db.execute_unprepared(&format!(
            "DELETE FROM marketplace_sources WHERE id = {OFFICIAL_SOURCE_ID}"
        ))
        .await?;
        for (table, name, _) in INDEXES {
            if m.has_index(*table, *name).await? {
                m.drop_index(
                    Index::drop()
                        .name(*name)
                        .table(Alias::new(*table))
                        .to_owned(),
                )
                .await?;
            }
        }
        for (table, name, _) in COLUMNS.iter().rev() {
            if m.has_column(*table, *name).await? {
                m.alter_table(
                    Table::alter()
                        .table(Alias::new(*table))
                        .drop_column(Alias::new(*name))
                        .to_owned(),
                )
                .await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_indexes_never_cover_text_columns() {
        // MySQL rejects TEXT/BLOB columns in a key without a prefix length (error 1170);
        // marketplace_entries.entry_key is TEXT from the original marketplace migration.
        let sql = |backend| {
            indexes(backend)
                .into_iter()
                .map(|(_, _, index)| match backend {
                    sea_orm::DbBackend::MySql => index.to_string(MysqlQueryBuilder),
                    sea_orm::DbBackend::Postgres => index.to_string(PostgresQueryBuilder),
                    _ => index.to_string(SqliteQueryBuilder),
                })
                .collect::<Vec<_>>()
        };
        let mysql = sql(sea_orm::DbBackend::MySql);
        assert!(mysql.iter().all(|s| !s.contains("entry_key")), "{mysql:?}");
        assert_eq!(mysql.len(), 2);
        for backend in [sea_orm::DbBackend::Sqlite, sea_orm::DbBackend::Postgres] {
            let statements = sql(backend);
            assert_eq!(statements.len(), 3);
            assert!(statements.iter().any(|s| s.contains("entry_key")));
        }
    }
    use sea_orm::{ConnectionTrait, EntityTrait, PaginatorTrait, Statement};
    #[tokio::test]
    async fn marketplace_platform_migration_roundtrip_preserves_rows() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let manager = SchemaManager::new(&s.db);
        Migration.down(&manager).await.unwrap();
        Migration.up(&manager).await.unwrap();
        assert_eq!(
            crate::entities::users::Entity::find()
                .count(&s.db)
                .await
                .unwrap(),
            2
        );
        let backend = s.db.get_database_backend();
        s.db.execute_raw(Statement::from_string(backend, "INSERT INTO marketplace_sources (id,workspace_id,name,repository_id,config,state,last_error,synced_at,run_as,version,created_at,updated_at,trust) VALUES (91,0,'Official',0,'{}','idle','',0,0,1,1,1,'official')")).await.unwrap();
        let row = s.db.query_one_raw(Statement::from_string(backend, "SELECT trust, enabled, sync_owner, sync_lease_until FROM marketplace_sources WHERE id = 91")).await.unwrap().unwrap();
        assert_eq!(row.try_get::<String>("", "trust").unwrap(), "official");
        assert_eq!(row.try_get::<i64>("", "enabled").unwrap(), 1);
        assert_eq!(row.try_get::<String>("", "sync_owner").unwrap(), "");
        s.db.execute_raw(Statement::from_string(backend, "INSERT INTO marketplace_entries (id,workspace_id,source_id,entry_key,name,description,content,parameters,version,created_at,updated_at) VALUES (92,0,91,'k','n','','{}','[]',1,1,1)")).await.unwrap();
        let entry =
            s.db.query_one_raw(Statement::from_string(
                backend,
                "SELECT package_version, kind, compatible FROM marketplace_entries WHERE id = 92",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            entry.try_get::<String>("", "package_version").unwrap(),
            "0.0.0"
        );
        assert_eq!(entry.try_get::<String>("", "kind").unwrap(), "app");
        assert_eq!(entry.try_get::<i64>("", "compatible").unwrap(), 1);
        let official = s.db.query_one_raw(Statement::from_string(backend, "SELECT workspace_id, trust, enabled, sync_interval, config FROM marketplace_sources WHERE id = 1")).await.unwrap().expect("default official source");
        assert_eq!(official.try_get::<i64>("", "workspace_id").unwrap(), 0);
        assert_eq!(official.try_get::<String>("", "trust").unwrap(), "official");
        assert_eq!(
            official.try_get::<i64>("", "sync_interval").unwrap(),
            6 * 3600
        );
        let config: serde_json::Value =
            serde_json::from_str(&official.try_get::<String>("", "config").unwrap()).unwrap();
        assert_eq!(config["project"], "7link-team/ferry");
        assert_eq!(config["index_path"], "marketplace/index.json");
        for table in ["app_templates", "config_templates"] {
            let row =
                s.db.query_one_raw(Statement::from_string(
                    backend,
                    format!("SELECT COUNT(origin_source_id) AS n FROM {table}"),
                ))
                .await
                .unwrap()
                .unwrap();
            assert!(row.try_get::<i64>("", "n").unwrap() >= 0);
        }
    }
}
