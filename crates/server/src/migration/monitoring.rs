use super::*;

pub struct Migration;
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "monitoring_health_v1"
    }
}

pub(crate) fn tables(backend: DbBackend) -> Vec<TableCreateStatement> {
    let mut checks = create_table("health_checks", backend);
    checks.col(
        ColumnDef::new(Alias::new("id"))
            .big_integer()
            .not_null()
            .primary_key(),
    );
    for name in [
        "workspace_id",
        "deployment_id",
        "host_id",
        "enabled",
        "run_as",
        "created_at",
        "created_by",
        "updated_at",
        "updated_by",
        "checked_at",
        "checked_by",
        "due_at",
        "due_by",
    ] {
        checks.col(
            ColumnDef::new(Alias::new(name))
                .big_integer()
                .not_null()
                .default(0),
        );
    }
    checks.col(
        ColumnDef::new(Alias::new("version"))
            .big_integer()
            .not_null()
            .default(1),
    );
    for (name, length) in [
        ("name", 120),
        ("kind", 16),
        ("role", 16),
        ("service_name", 120),
    ] {
        checks.col(
            ColumnDef::new(Alias::new(name))
                .string_len(length)
                .not_null(),
        );
    }
    for name in ["description", "settings", "result"] {
        checks.col(long_text(name, backend).not_null());
    }
    let mut observations = create_table("health_observations", backend);
    observations.col(
        ColumnDef::new(Alias::new("id"))
            .big_integer()
            .not_null()
            .primary_key(),
    );
    for name in ["workspace_id", "check_id", "observed_at", "observed_by"] {
        observations.col(
            ColumnDef::new(Alias::new(name))
                .big_integer()
                .not_null()
                .default(0),
        );
    }
    observations.col(long_text("result", backend).not_null());
    vec![checks.to_owned(), observations.to_owned()]
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for table in tables(m.get_database_backend()) {
            m.create_table(table).await?;
        }
        for (table, name, columns) in [
            (
                "health_checks",
                "idx_health_checks_workspace_created",
                vec!["workspace_id", "created_at"],
            ),
            (
                "health_checks",
                "idx_health_checks_due",
                vec!["enabled", "due_at"],
            ),
            (
                "health_checks",
                "idx_health_checks_application",
                vec!["workspace_id", "deployment_id"],
            ),
            (
                "health_checks",
                "idx_health_checks_host",
                vec!["workspace_id", "host_id"],
            ),
            (
                "health_observations",
                "idx_health_observations_check",
                vec!["workspace_id", "check_id", "observed_at"],
            ),
        ] {
            let mut index = Index::create();
            index.name(name).table(Alias::new(table));
            for column in columns {
                index.col(Alias::new(column));
            }
            execute_schema(m, SchemaStatement::Index(table, name, index.to_owned())).await?;
        }
        Ok(())
    }
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        use sea_orm::{EntityTrait, PaginatorTrait};
        if crate::entities::health_checks::Entity::find()
            .count(m.get_connection())
            .await?
            != 0
            || crate::entities::health_observations::Entity::find()
                .count(m.get_connection())
                .await?
                != 0
        {
            return Err(DbErr::Custom("Monitoring history must be retained; restore a verified backup for destructive rollback".into()));
        }
        for name in ["health_observations", "health_checks"] {
            m.drop_table(Table::drop().table(Alias::new(name)).to_owned())
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
    async fn monitoring_migration_preserves_existing_data_and_refuses_destructive_down() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("CREATE TABLE previous_data (id BIGINT PRIMARY KEY, value TEXT NOT NULL); INSERT INTO previous_data VALUES (1,'retained')").await.unwrap();
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.unwrap();
        Migration.up(&manager).await.unwrap();
        for (table, at, by) in [
            ("health_checks", "created_at", "created_by"),
            ("health_checks", "updated_at", "updated_by"),
            ("health_checks", "checked_at", "checked_by"),
            ("health_checks", "due_at", "due_by"),
            ("health_observations", "observed_at", "observed_by"),
        ] {
            assert!(manager.has_column(table, at).await.unwrap());
            assert!(manager.has_column(table, by).await.unwrap());
        }
        for (table, index) in [
            ("health_checks", "idx_health_checks_workspace_created"),
            ("health_checks", "idx_health_checks_due"),
            ("health_checks", "idx_health_checks_application"),
            ("health_checks", "idx_health_checks_host"),
            ("health_observations", "idx_health_observations_check"),
        ] {
            assert!(manager.has_index(table, index).await.unwrap());
        }
        db.execute_unprepared("INSERT INTO health_observations (id,workspace_id,check_id,result,observed_at,observed_by) VALUES(1,1,2,'{}',1,0)").await.unwrap();
        assert!(Migration.down(&manager).await.is_err());
        assert!(manager.has_table("health_observations").await.unwrap());
        db.execute_unprepared("DELETE FROM health_observations")
            .await
            .unwrap();
        Migration.down(&manager).await.unwrap();
        assert!(!manager.has_table("health_checks").await.unwrap());
        let value = db
            .query_one_raw(sea_orm::Statement::from_string(
                DbBackend::Sqlite,
                "SELECT value FROM previous_data WHERE id=1",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(value.try_get::<String>("", "value").unwrap(), "retained");
    }
    #[test]
    fn monitoring_schema_sql_has_portable_audit_constraints() {
        for backend in [DbBackend::Sqlite, DbBackend::Postgres, DbBackend::MySql] {
            for table in tables(backend) {
                let sql = match backend {
                    DbBackend::Sqlite => table.to_string(SqliteQueryBuilder),
                    DbBackend::Postgres => table.to_string(PostgresQueryBuilder),
                    DbBackend::MySql => table.to_string(MysqlQueryBuilder),
                    _ => panic!("Unexpected backend in the explicit schema test matrix"),
                };
                assert!(sql.contains("PRIMARY KEY"));
                assert!(sql.contains("NOT NULL"));
                assert!(!sql.contains("AUTOINCREMENT"));
                println!("MONITORING_DDL {backend:?} {sql};");
            }
        }
    }
}
