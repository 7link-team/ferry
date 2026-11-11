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
    ("marketplace_entries", "package_version", Kind::Text(64, "0.0.0")),
    ("marketplace_entries", "kind", Kind::Text(16, "app")),
    ("marketplace_entries", "meta", Kind::LongText),
    ("marketplace_entries", "files", Kind::LongText),
    ("marketplace_entries", "readme", Kind::LongText),
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
    ("marketplace_entries", "idx_marketplace_entries_source", &["workspace_id", "source_id"]),
    ("marketplace_entries", "idx_marketplace_entries_key", &["workspace_id", "entry_key", "package_version"]),
    ("marketplace_sources", "idx_marketplace_sources_due", &["workspace_id", "enabled", "next_sync_at"]),
];

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
            m.alter_table(Table::alter().table(Alias::new(*table)).add_column(column).to_owned()).await?;
        }
        for (table, name, columns) in INDEXES {
            if m.has_index(*table, *name).await? {
                continue;
            }
            let mut index = Index::create();
            index.name(*name).table(Alias::new(*table));
            for column in *columns {
                index.col(Alias::new(*column));
            }
            m.create_index(index.to_owned()).await?;
        }
        Ok(())
    }
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, _) in INDEXES {
            if m.has_index(*table, *name).await? {
                m.drop_index(Index::drop().name(*name).table(Alias::new(*table)).to_owned()).await?;
            }
        }
        for (table, name, _) in COLUMNS.iter().rev() {
            if m.has_column(*table, *name).await? {
                m.alter_table(Table::alter().table(Alias::new(*table)).drop_column(Alias::new(*name)).to_owned()).await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, EntityTrait, PaginatorTrait, Statement};
    #[tokio::test]
    async fn marketplace_platform_migration_roundtrip_preserves_rows() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let manager = SchemaManager::new(&s.db);
        Migration.down(&manager).await.unwrap();
        Migration.up(&manager).await.unwrap();
        assert_eq!(crate::entities::users::Entity::find().count(&s.db).await.unwrap(), 2);
        let backend = s.db.get_database_backend();
        s.db.execute_raw(Statement::from_string(backend, "INSERT INTO marketplace_sources (id,workspace_id,name,repository_id,config,state,last_error,synced_at,run_as,version,created_at,updated_at,trust) VALUES (91,0,'Official',0,'{}','idle','',0,0,1,1,1,'official')")).await.unwrap();
        let row = s.db.query_one_raw(Statement::from_string(backend, "SELECT trust, enabled, sync_owner, sync_lease_until FROM marketplace_sources WHERE id = 91")).await.unwrap().unwrap();
        assert_eq!(row.try_get::<String>("", "trust").unwrap(), "official");
        assert_eq!(row.try_get::<i64>("", "enabled").unwrap(), 1);
        assert_eq!(row.try_get::<String>("", "sync_owner").unwrap(), "");
        s.db.execute_raw(Statement::from_string(backend, "INSERT INTO marketplace_entries (id,workspace_id,source_id,entry_key,name,description,content,parameters,version,created_at,updated_at) VALUES (92,0,91,'k','n','','{}','[]',1,1,1)")).await.unwrap();
        let entry = s.db.query_one_raw(Statement::from_string(backend, "SELECT package_version, kind, compatible FROM marketplace_entries WHERE id = 92")).await.unwrap().unwrap();
        assert_eq!(entry.try_get::<String>("", "package_version").unwrap(), "0.0.0");
        assert_eq!(entry.try_get::<String>("", "kind").unwrap(), "app");
        assert_eq!(entry.try_get::<i64>("", "compatible").unwrap(), 1);
        for table in ["app_templates", "config_templates"] {
            let row = s.db.query_one_raw(Statement::from_string(backend, format!("SELECT COUNT(origin_source_id) AS n FROM {table}"))).await.unwrap().unwrap();
            assert!(row.try_get::<i64>("", "n").unwrap() >= 0);
        }
    }
}
