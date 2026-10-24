//! Schema migrations as raw SQL, applied in order and never edited once
//! shipped. The full target schema is in docs/ferry-运维平台产品与技术方案.md;
//! `Initial` carries only the identity and tenancy bedrock every module needs.

use crate::dialect;
use sea_orm_migration::prelude::*;

/// Applies one canonical (SQLite-flavoured) DDL statement to whichever backend
/// is connected.
///
/// MySQL has no `CREATE INDEX IF NOT EXISTS` and commits DDL independently, so
/// a migration that fails halfway must stay re-runnable: every statement is
/// checked for existence first rather than relying on the clause.
async fn execute_schema(m: &SchemaManager<'_>, sql: &str) -> Result<(), DbErr> {
    let backend = m.get_connection().get_database_backend();
    // No backend has `ADD COLUMN IF NOT EXISTS` in a spelling all three share,
    // so the column is looked up instead. Checked everywhere, not only on
    // MySQL: a re-run must be a no-op rather than an error on any of them.
    if let Some((table, column)) = added_column(sql)
        && m.has_column(&table, &column).await?
    {
        return Ok(());
    }
    if backend == sea_orm::DbBackend::MySql {
        if let Some((table, index)) = dialect::index_target(sql) {
            if m.has_index(&table, &index).await? {
                return Ok(());
            }
        } else if let Some(table) = sql
            .trim()
            .strip_prefix("CREATE TABLE IF NOT EXISTS ")
            .and_then(|rest| rest.split_whitespace().next())
            && m.has_table(table).await?
        {
            return Ok(());
        }
    }
    let Some(sql) = dialect::schema(backend, sql) else {
        return Ok(());
    };
    m.get_connection().execute_unprepared(&sql).await?;
    Ok(())
}

/// `ALTER TABLE t ADD COLUMN c ...` -> `(t, c)`, so the caller can ask whether
/// the column is already there before issuing it.
fn added_column(sql: &str) -> Option<(String, String)> {
    let words: Vec<&str> = sql.split_whitespace().collect();
    match words.as_slice() {
        ["ALTER", "TABLE", table, "ADD", "COLUMN", column, ..] => {
            Some(((*table).to_owned(), (*column).to_owned()))
        }
        _ => None,
    }
}

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(Initial),
            Box::new(HostResources),
            Box::new(RunnerEnrolment),
            Box::new(Credentials),
            Box::new(Repositories),
            Box::new(Pipelines),
            Box::new(IdInfrastructure),
        ]
    }
}

const INITIAL: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS users (
        id TEXT PRIMARY KEY,
        email TEXT NOT NULL,
        name TEXT NOT NULL,
        password_hash TEXT NOT NULL,
        active BIGINT NOT NULL DEFAULT 1,
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL)",
    "CREATE UNIQUE INDEX IF NOT EXISTS users_email ON users (email)",
    "CREATE TABLE IF NOT EXISTS sessions (
        id TEXT PRIMARY KEY,
        user_id TEXT NOT NULL,
        workspace_id TEXT NOT NULL DEFAULT '',
        created_at BIGINT NOT NULL,
        seen_at BIGINT NOT NULL,
        expires_at BIGINT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS sessions_user ON sessions (user_id)",
    "CREATE TABLE IF NOT EXISTS workspaces (
        id TEXT PRIMARY KEY,
        slug TEXT NOT NULL,
        name TEXT NOT NULL,
        owner_id TEXT NOT NULL,
        join_code TEXT NOT NULL DEFAULT '',
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL)",
    "CREATE UNIQUE INDEX IF NOT EXISTS workspaces_slug ON workspaces (slug)",
    "CREATE TABLE IF NOT EXISTS workspace_roles (
        id TEXT PRIMARY KEY,
        workspace_id TEXT NOT NULL,
        name TEXT NOT NULL,
        permissions TEXT NOT NULL,
        builtin BIGINT NOT NULL DEFAULT 0,
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS workspace_roles_workspace ON workspace_roles (workspace_id)",
    "CREATE TABLE IF NOT EXISTS workspace_members (
        id TEXT PRIMARY KEY,
        workspace_id TEXT NOT NULL,
        user_id TEXT NOT NULL,
        role_id TEXT NOT NULL,
        state TEXT NOT NULL DEFAULT 'active',
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL)",
    "CREATE UNIQUE INDEX IF NOT EXISTS workspace_members_unique ON workspace_members (workspace_id, user_id)",
    "CREATE TABLE IF NOT EXISTS activity (
        id TEXT PRIMARY KEY,
        workspace_id TEXT NOT NULL,
        actor_id TEXT NOT NULL,
        actor_name TEXT NOT NULL,
        action TEXT NOT NULL,
        resource TEXT NOT NULL,
        resource_id TEXT NOT NULL,
        details TEXT NOT NULL,
        created_at BIGINT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS activity_workspace ON activity (workspace_id, created_at)",
];

struct Initial;
impl MigrationName for Initial {
    fn name(&self) -> &str {
        "initial_v1"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Initial {
    async fn up(&self, m: &SchemaManager) -> std::result::Result<(), DbErr> {
        for sql in INITIAL {
            execute_schema(m, sql).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> std::result::Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

const HOST_RESOURCES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS host_groups (
        id TEXT PRIMARY KEY,
        workspace_id TEXT NOT NULL,
        name TEXT NOT NULL,
        description TEXT NOT NULL DEFAULT '',
        parent_id TEXT NOT NULL DEFAULT '',
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL,
        version BIGINT NOT NULL DEFAULT 1)",
    "CREATE INDEX IF NOT EXISTS host_groups_workspace ON host_groups (workspace_id, name)",
    "CREATE TABLE IF NOT EXISTS hosts (
        id TEXT PRIMARY KEY,
        workspace_id TEXT NOT NULL,
        name TEXT NOT NULL,
        address TEXT NOT NULL DEFAULT '',
        port BIGINT NOT NULL DEFAULT 22,
        username TEXT NOT NULL DEFAULT '',
        connection TEXT NOT NULL,
        state TEXT NOT NULL DEFAULT 'unknown',
        group_id TEXT NOT NULL DEFAULT '',
        description TEXT NOT NULL DEFAULT '',
        host_key TEXT NOT NULL DEFAULT '',
        runner_key_hash TEXT NOT NULL DEFAULT '',
        seen_at BIGINT NOT NULL DEFAULT 0,
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL,
        version BIGINT NOT NULL DEFAULT 1)",
    "CREATE INDEX IF NOT EXISTS hosts_workspace ON hosts (workspace_id, name)",
    "CREATE INDEX IF NOT EXISTS hosts_group ON hosts (workspace_id, group_id)",
    "CREATE TABLE IF NOT EXISTS tags (
        id TEXT PRIMARY KEY,
        workspace_id TEXT NOT NULL,
        name TEXT NOT NULL,
        color TEXT NOT NULL DEFAULT '',
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL,
        version BIGINT NOT NULL DEFAULT 1)",
    "CREATE UNIQUE INDEX IF NOT EXISTS tags_unique ON tags (workspace_id, name)",
    "CREATE TABLE IF NOT EXISTS host_tags (
        host_id TEXT NOT NULL,
        tag_id TEXT NOT NULL,
        PRIMARY KEY (host_id, tag_id))",
];

struct HostResources;
impl MigrationName for HostResources {
    fn name(&self) -> &str {
        "host_resources_v1"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for HostResources {
    async fn up(&self, m: &SchemaManager) -> std::result::Result<(), DbErr> {
        for sql in HOST_RESOURCES {
            execute_schema(m, sql).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> std::result::Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

const RUNNER_ENROLMENT: &[&str] = &[
    // A host is added to a workspace by running one command on it. The token in
    // that command is single-use and short-lived, and only its hash is stored,
    // so the table is not a list of working credentials.
    //
    // `host_id` empty means "whichever host enrols with this", which is the
    // one-command flow for a machine that has no row yet. Non-empty binds the
    // token to a host the operator already created.
    "CREATE TABLE IF NOT EXISTS runner_tokens (
        id TEXT PRIMARY KEY,
        workspace_id TEXT NOT NULL,
        host_id TEXT NOT NULL DEFAULT '',
        token_hash TEXT NOT NULL,
        expires_at BIGINT NOT NULL,
        used_at BIGINT NOT NULL DEFAULT 0,
        used_by TEXT NOT NULL DEFAULT '',
        created_by TEXT NOT NULL DEFAULT '',
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL,
        version BIGINT NOT NULL DEFAULT 1)",
    // Unique so two tokens can never collide, and so consuming one is a single
    // conditional UPDATE rather than a read followed by a write.
    "CREATE UNIQUE INDEX IF NOT EXISTS runner_tokens_hash ON runner_tokens (token_hash)",
    "CREATE INDEX IF NOT EXISTS runner_tokens_workspace ON runner_tokens (workspace_id, created_at)",
    // Facts the runner reports about itself, for the host page and dashboard.
    "ALTER TABLE hosts ADD COLUMN os TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE hosts ADD COLUMN arch TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE hosts ADD COLUMN agent_version TEXT NOT NULL DEFAULT ''",
    // The latest heartbeat's load/memory/disk as a small JSON object. MySQL
    // maps this to VARCHAR(255) (a TEXT column cannot carry a DEFAULT), which
    // is why the keys are short — see `runner::metrics_json`.
    // ponytail: last sample only, no history table. Add one when someone asks
    // for a graph rather than a number.
    "ALTER TABLE hosts ADD COLUMN metrics TEXT NOT NULL DEFAULT ''",
];

struct RunnerEnrolment;
impl MigrationName for RunnerEnrolment {
    fn name(&self) -> &str {
        "runner_enrolment_v1"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for RunnerEnrolment {
    async fn up(&self, m: &SchemaManager) -> std::result::Result<(), DbErr> {
        for sql in RUNNER_ENROLMENT {
            execute_schema(m, sql).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> std::result::Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::added_column;

    #[test]
    fn added_columns_are_recognised_and_nothing_else_is() {
        assert_eq!(
            added_column("ALTER TABLE hosts ADD COLUMN metrics TEXT NOT NULL DEFAULT ''"),
            Some(("hosts".into(), "metrics".into()))
        );
        assert!(added_column("CREATE TABLE IF NOT EXISTS hosts (id TEXT)").is_none());
        assert!(added_column("CREATE INDEX x ON hosts (id)").is_none());
        assert!(added_column("ALTER TABLE hosts RENAME TO boxes").is_none());
    }
}

const CREDENTIALS: &[&str] = &[
    // `secret` holds a sealed SSH private key, so it must not land in MySQL's
    // default VARCHAR(255) — dialect.rs lists it as a long column.
    "CREATE TABLE IF NOT EXISTS credentials (
        id TEXT PRIMARY KEY,
        workspace_id TEXT NOT NULL,
        name TEXT NOT NULL,
        kind TEXT NOT NULL,
        username TEXT NOT NULL DEFAULT '',
        description TEXT NOT NULL DEFAULT '',
        -- No DEFAULT: MySQL forbids one on TEXT, and every insert supplies it.
        secret TEXT NOT NULL,
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL,
        version BIGINT NOT NULL DEFAULT 1)",
    "CREATE INDEX IF NOT EXISTS credentials_workspace ON credentials (workspace_id, name)",
    "ALTER TABLE hosts ADD COLUMN credential_id TEXT NOT NULL DEFAULT ''",
];

struct Credentials;
impl MigrationName for Credentials {
    fn name(&self) -> &str {
        "credentials_v1"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Credentials {
    async fn up(&self, m: &SchemaManager) -> std::result::Result<(), DbErr> {
        for sql in CREDENTIALS {
            execute_schema(m, sql).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> std::result::Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

#[cfg(test)]
mod schema_tests {
    use super::*;

    /// Every DDL statement this crate ships, so a new migration is covered by
    /// simply being added to the list below.
    fn all_statements() -> Vec<&'static str> {
        [
            INITIAL,
            HOST_RESOURCES,
            RUNNER_ENROLMENT,
            CREDENTIALS,
            REPOSITORIES,
            PIPELINES,
        ]
        .concat()
    }

    #[test]
    fn pipeline_definition_is_longtext_without_default() {
        let sql = crate::dialect::mysql_schema(PIPELINES[0]);
        assert!(sql.contains("definition LONGTEXT NOT NULL,"));
    }

    #[test]
    fn no_long_column_carries_a_default() {
        // MySQL refuses `LONGTEXT ... DEFAULT ''` with error 1101, and the
        // migration then fails at startup rather than at review time. Columns
        // listed in dialect::LONG_COLUMNS must therefore declare no DEFAULT.
        for sql in all_statements() {
            let mysql = crate::dialect::mysql_schema(sql);
            for line in mysql.lines() {
                assert!(
                    !(line.contains("LONGTEXT") && line.to_uppercase().contains("DEFAULT")),
                    "MySQL rejects a DEFAULT on a LONGTEXT column:\n  {}\nin:\n{sql}",
                    line.trim()
                );
            }
        }
    }

    #[test]
    fn no_text_column_survives_into_mysql() {
        // A bare TEXT column cannot be indexed on MySQL; dialect.rs must have
        // given every one of them a width.
        for sql in all_statements() {
            let mysql = crate::dialect::mysql_schema(sql);
            assert!(
                !mysql.contains(" TEXT "),
                "a TEXT column reached MySQL unsized:\n{mysql}"
            );
        }
    }
}

const REPOSITORIES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS repositories (
        id TEXT PRIMARY KEY,
        workspace_id TEXT NOT NULL,
        name TEXT NOT NULL,
        url TEXT NOT NULL,
        default_branch TEXT NOT NULL DEFAULT 'main',
        credential_id TEXT NOT NULL DEFAULT '',
        description TEXT NOT NULL DEFAULT '',
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL,
        version BIGINT NOT NULL DEFAULT 1)",
    "CREATE INDEX IF NOT EXISTS repositories_workspace ON repositories (workspace_id, name)",
];

struct Repositories;
impl MigrationName for Repositories {
    fn name(&self) -> &str {
        "repositories_v1"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Repositories {
    async fn up(&self, m: &SchemaManager) -> std::result::Result<(), DbErr> {
        for sql in REPOSITORIES {
            execute_schema(m, sql).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> std::result::Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

const PIPELINES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS pipelines (
        id TEXT PRIMARY KEY,
        workspace_id TEXT NOT NULL,
        name TEXT NOT NULL,
        repository_id TEXT NOT NULL DEFAULT '',
        definition TEXT NOT NULL,
        target_kind TEXT NOT NULL,
        target_ref TEXT NOT NULL,
        description TEXT NOT NULL DEFAULT '',
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL,
        version BIGINT NOT NULL DEFAULT 1)",
    "CREATE INDEX IF NOT EXISTS pipelines_workspace ON pipelines (workspace_id, name)",
];

struct Pipelines;
impl MigrationName for Pipelines {
    fn name(&self) -> &str {
        "pipelines_v1"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Pipelines {
    async fn up(&self, m: &SchemaManager) -> std::result::Result<(), DbErr> {
        for sql in PIPELINES {
            execute_schema(m, sql).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> std::result::Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct IdInfrastructure;
impl MigrationName for IdInfrastructure {
    fn name(&self) -> &str {
        "id_infrastructure_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for IdInfrastructure {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(Alias::new("locks"))
                .if_not_exists()
                .col(
                    ColumnDef::new(Alias::new("name"))
                        .string_len(64)
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(Alias::new("owner"))
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("acquired_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("expires_at"))
                        .big_integer()
                        .not_null(),
                )
                .to_owned(),
        )
        .await?;
        m.create_table(
            Table::create()
                .table(Alias::new("worker_ids"))
                .if_not_exists()
                .col(
                    ColumnDef::new(Alias::new("worker_id"))
                        .big_integer()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(Alias::new("instance_id"))
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("heartbeat_at"))
                        .big_integer()
                        .not_null(),
                )
                .to_owned(),
        )
        .await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除 worker 租约".into()))
    }
}
