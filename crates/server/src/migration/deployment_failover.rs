//! Failover timeline (`deployment_events`) and instance fences (`deployment_fences`).
use super::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

fn bigints(table: &mut TableCreateStatement, names: &[&str]) {
    for name in names {
        table.col(
            ColumnDef::new(Alias::new(*name))
                .big_integer()
                .not_null()
                .default(0),
        );
    }
}

pub(crate) fn tables(backend: DbBackend) -> Vec<TableCreateStatement> {
    let id = || {
        ColumnDef::new(Alias::new("id"))
            .big_integer()
            .not_null()
            .primary_key()
            .to_owned()
    };
    let mut events = create_table("deployment_events", backend);
    events.col(id());
    bigints(
        &mut events,
        &[
            "workspace_id",
            "deployment_id",
            "host_id",
            "revision_id",
            "created_at",
        ],
    );
    events.col(ColumnDef::new(Alias::new("kind")).string_len(32).not_null());
    events.col(
        ColumnDef::new(Alias::new("instance"))
            .string_len(120)
            .not_null()
            .default(""),
    );
    // MySQL TEXT cannot carry a default or an index.
    events.col(long_text("detail", backend).not_null());
    let mut fences = create_table("deployment_fences", backend);
    fences.col(id());
    bigints(
        &mut fences,
        &[
            "workspace_id",
            "deployment_id",
            "host_id",
            "created_at",
            "cleared_at",
        ],
    );
    fences.col(
        ColumnDef::new(Alias::new("instance"))
            .string_len(120)
            .not_null()
            .default(""),
    );
    vec![events.to_owned(), fences.to_owned()]
}

const INDEXES: [(&str, &str, &[&str]); 4] = [
    (
        "deployment_events",
        "idx_deployment_events_deployment",
        &["workspace_id", "deployment_id", "created_at"],
    ),
    (
        "deployment_events",
        "idx_deployment_events_instance",
        &["deployment_id", "instance", "created_at"],
    ),
    (
        "deployment_fences",
        "idx_deployment_fences_deployment",
        &["workspace_id", "deployment_id", "cleared_at"],
    ),
    (
        "deployment_fences",
        "idx_deployment_fences_host",
        &["host_id", "cleared_at"],
    ),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for table in tables(m.get_database_backend()) {
            execute_schema(m, SchemaStatement::Table(table)).await?;
        }
        for (table, name, columns) in INDEXES {
            let mut index = Index::create();
            index.name(name).table(Alias::new(table));
            for column in columns {
                index.col(Alias::new(*column));
            }
            execute_schema(m, SchemaStatement::Index(table, name, index.to_owned())).await?;
        }
        Ok(())
    }
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        use sea_orm::ConnectionTrait;
        let names = ["deployment_events", "deployment_fences"];
        for name in names {
            if !m.has_table(name).await? {
                continue;
            }
            let stored = Query::select()
                .expr(Expr::val(1))
                .from(Alias::new(name))
                .limit(1)
                .to_owned();
            if m.get_connection().query_one(&stored).await?.is_some() {
                // Spec §9.4: a rollback never deletes business data; the failover history stays.
                return Err(DbErr::Custom("不自动删除业务数据".into()));
            }
        }
        for name in names {
            m.drop_table(Table::drop().table(Alias::new(name)).if_exists().to_owned())
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database};

    #[tokio::test]
    async fn failover_tables_migrate_and_refuse_destructive_down() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.unwrap();
        Migration.up(&manager).await.unwrap();
        for (table, index) in [
            ("deployment_events", "idx_deployment_events_deployment"),
            ("deployment_events", "idx_deployment_events_instance"),
            ("deployment_fences", "idx_deployment_fences_deployment"),
            ("deployment_fences", "idx_deployment_fences_host"),
        ] {
            assert!(manager.has_index(table, index).await.unwrap(), "{index}");
        }
        // Defaults let a row omit every BIGINT column and the instance.
        db.execute_unprepared(
            "INSERT INTO deployment_events (id,kind,detail) VALUES (1,'fenced','{}')",
        )
        .await
        .unwrap();
        let err = Migration.down(&manager).await.expect_err("must refuse");
        assert!(err.to_string().contains("不自动删除业务数据"), "{err}");
        assert!(manager.has_table("deployment_events").await.unwrap());
        db.execute_unprepared("DELETE FROM deployment_events")
            .await
            .unwrap();
        db.execute_unprepared("INSERT INTO deployment_fences (id,instance) VALUES (1,'db-0')")
            .await
            .unwrap();
        assert!(Migration.down(&manager).await.is_err());
        db.execute_unprepared("DELETE FROM deployment_fences")
            .await
            .unwrap();
        Migration.down(&manager).await.unwrap();
        assert!(!manager.has_table("deployment_events").await.unwrap());
        assert!(!manager.has_table("deployment_fences").await.unwrap());
        Migration.down(&manager).await.unwrap();
    }

    #[test]
    fn failover_schema_keeps_mysql_text_without_default() {
        for backend in [DbBackend::Sqlite, DbBackend::Postgres, DbBackend::MySql] {
            assert_eq!(tables(backend).len(), 2);
            for table in tables(backend) {
                let sql = match backend {
                    DbBackend::Sqlite => table.to_string(SqliteQueryBuilder),
                    DbBackend::Postgres => table.to_string(PostgresQueryBuilder),
                    _ => table.to_string(MysqlQueryBuilder),
                };
                assert!(sql.contains("PRIMARY KEY"), "{sql}");
                assert!(!sql.contains("AUTOINCREMENT"), "{sql}");
                if backend == DbBackend::MySql && sql.contains("`detail`") {
                    assert!(sql.contains("`detail` longtext NOT NULL"), "{sql}");
                    assert!(!sql.contains("longtext NOT NULL DEFAULT"), "{sql}");
                }
            }
        }
    }
}
