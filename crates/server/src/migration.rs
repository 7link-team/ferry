//! Schema migrations built with SeaQuery, applied in order and never edited once
//! shipped. The full target schema is in docs/ferry-运维平台产品与技术方案.md;
//! `Initial` carries only the identity and tenancy bedrock every module needs.

use sea_orm::DbBackend;
use sea_orm_migration::prelude::*;

// Keep names alongside builders for existence checks; no SQL parsing is needed.
enum SchemaStatement {
    Table(TableCreateStatement),
    Index(&'static str, &'static str, IndexCreateStatement),
    Column(&'static str, &'static str, TableAlterStatement),
}

/// MySQL commits DDL independently. Checking indexes and added columns makes
/// a partially applied migration safe to retry on every backend.
async fn execute_schema(m: &SchemaManager<'_>, statement: SchemaStatement) -> Result<(), DbErr> {
    match statement {
        SchemaStatement::Table(table) => m.create_table(table).await,
        SchemaStatement::Index(table, name, index) => {
            if !m.has_index(table, name).await? {
                m.create_index(index).await?;
            }
            Ok(())
        }
        SchemaStatement::Column(table, name, column) => {
            if !m.has_column(table, name).await? {
                m.alter_table(column).await?;
            }
            Ok(())
        }
    }
}

fn create_table(name: &str, backend: DbBackend) -> TableCreateStatement {
    let mut table = Table::create();
    table.table(Alias::new(name)).if_not_exists();
    if backend == DbBackend::MySql {
        // Preserve case/accent-sensitive unique keys (Admin != admin).
        table.character_set("utf8mb4").collate("utf8mb4_bin");
    }
    table
}

fn long_text(name: &str, backend: DbBackend) -> ColumnDef {
    let mut column = ColumnDef::new(Alias::new(name));
    column.text();
    if backend == DbBackend::MySql {
        // SeaQuery 1.0 emits TEXT for ColumnType::Text, not LONGTEXT. Keep the
        // original capacity with its custom-type builder, without rewriting SQL.
        column.custom("longtext");
    }
    column
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

// Optional references use 0, including activity.resource_id for bulk actions.
fn initial(backend: DbBackend) -> Vec<SchemaStatement> {
    vec![
        SchemaStatement::Table(
            create_table("users", backend)
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key())
                .col(
                    ColumnDef::new(Alias::new("email"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("name"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("password_hash"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("active"))
                        .big_integer()
                        .not_null()
                        .default(1_i64),
                )
                .col(
                    ColumnDef::new(Alias::new("created_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("updated_at"))
                        .big_integer()
                        .not_null(),
                )
                .to_owned(),
        ),
        SchemaStatement::Index(
            "users",
            "users_email",
            Index::create()
                .name("users_email")
                .table(Alias::new("users"))
                .unique()
                .col(Alias::new("email"))
                .to_owned(),
        ),
        // `id` identifies the row; `token_hash` authenticates the caller. They must
        // stay separate: a snowflake encodes its own creation time and a small
        // counter, so using it as the cookie would make a session guessable from
        // roughly when its owner signed in. Only the hash is stored, as with
        // runner keys.
        SchemaStatement::Table(
            create_table("sessions", backend)
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key())
                .col(
                    ColumnDef::new(Alias::new("token_hash"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("user_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("workspace_id"))
                        .big_integer()
                        .not_null()
                        .default(0_i64),
                )
                .col(
                    ColumnDef::new(Alias::new("created_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("seen_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("expires_at"))
                        .big_integer()
                        .not_null(),
                )
                .to_owned(),
        ),
        SchemaStatement::Index(
            "sessions",
            "sessions_user",
            Index::create()
                .name("sessions_user")
                .table(Alias::new("sessions"))
                .col(Alias::new("user_id"))
                .to_owned(),
        ),
        SchemaStatement::Index(
            "sessions",
            "sessions_token",
            Index::create()
                .name("sessions_token")
                .table(Alias::new("sessions"))
                .unique()
                .col(Alias::new("token_hash"))
                .to_owned(),
        ),
        SchemaStatement::Table(
            create_table("workspaces", backend)
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key())
                .col(
                    ColumnDef::new(Alias::new("slug"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("name"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("owner_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("join_code"))
                        .string_len(255)
                        .not_null()
                        .default(""),
                )
                .col(
                    ColumnDef::new(Alias::new("created_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("updated_at"))
                        .big_integer()
                        .not_null(),
                )
                .to_owned(),
        ),
        SchemaStatement::Index(
            "workspaces",
            "workspaces_slug",
            Index::create()
                .name("workspaces_slug")
                .table(Alias::new("workspaces"))
                .unique()
                .col(Alias::new("slug"))
                .to_owned(),
        ),
        SchemaStatement::Table(
            create_table("workspace_roles", backend)
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key())
                .col(
                    ColumnDef::new(Alias::new("workspace_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("name"))
                        .string_len(255)
                        .not_null(),
                )
                .col(long_text("permissions", backend).not_null())
                .col(
                    ColumnDef::new(Alias::new("builtin"))
                        .big_integer()
                        .not_null()
                        .default(0_i64),
                )
                .col(
                    ColumnDef::new(Alias::new("created_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("updated_at"))
                        .big_integer()
                        .not_null(),
                )
                .to_owned(),
        ),
        SchemaStatement::Index(
            "workspace_roles",
            "workspace_roles_workspace",
            Index::create()
                .name("workspace_roles_workspace")
                .table(Alias::new("workspace_roles"))
                .col(Alias::new("workspace_id"))
                .to_owned(),
        ),
        SchemaStatement::Table(
            create_table("workspace_members", backend)
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key())
                .col(
                    ColumnDef::new(Alias::new("workspace_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("user_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("role_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("state"))
                        .string_len(255)
                        .not_null()
                        .default("active"),
                )
                .col(
                    ColumnDef::new(Alias::new("created_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("updated_at"))
                        .big_integer()
                        .not_null(),
                )
                .to_owned(),
        ),
        SchemaStatement::Index(
            "workspace_members",
            "workspace_members_unique",
            Index::create()
                .name("workspace_members_unique")
                .table(Alias::new("workspace_members"))
                .unique()
                .col(Alias::new("workspace_id"))
                .col(Alias::new("user_id"))
                .to_owned(),
        ),
        SchemaStatement::Table(
            create_table("activity", backend)
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key())
                .col(
                    ColumnDef::new(Alias::new("workspace_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("actor_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("actor_name"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("action"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("resource"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("resource_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(long_text("details", backend).not_null())
                .col(
                    ColumnDef::new(Alias::new("created_at"))
                        .big_integer()
                        .not_null(),
                )
                .to_owned(),
        ),
        SchemaStatement::Index(
            "activity",
            "activity_workspace",
            Index::create()
                .name("activity_workspace")
                .table(Alias::new("activity"))
                .col(Alias::new("workspace_id"))
                .col(Alias::new("created_at"))
                .to_owned(),
        ),
    ]
}

struct Initial;
impl MigrationName for Initial {
    fn name(&self) -> &str {
        "initial_v1"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Initial {
    async fn up(&self, m: &SchemaManager) -> std::result::Result<(), DbErr> {
        for statement in initial(m.get_database_backend()) {
            execute_schema(m, statement).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> std::result::Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

fn host_resources(backend: DbBackend) -> Vec<SchemaStatement> {
    vec![
        SchemaStatement::Table(
            create_table("host_groups", backend)
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key())
                .col(
                    ColumnDef::new(Alias::new("workspace_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("name"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("description"))
                        .string_len(255)
                        .not_null()
                        .default(""),
                )
                .col(
                    ColumnDef::new(Alias::new("parent_id"))
                        .big_integer()
                        .not_null()
                        .default(0_i64),
                )
                .col(
                    ColumnDef::new(Alias::new("created_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("updated_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("version"))
                        .big_integer()
                        .not_null()
                        .default(1_i64),
                )
                .to_owned(),
        ),
        SchemaStatement::Index(
            "host_groups",
            "host_groups_workspace",
            Index::create()
                .name("host_groups_workspace")
                .table(Alias::new("host_groups"))
                .col(Alias::new("workspace_id"))
                .col(Alias::new("name"))
                .to_owned(),
        ),
        SchemaStatement::Table(
            create_table("hosts", backend)
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key())
                .col(
                    ColumnDef::new(Alias::new("workspace_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("name"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("address"))
                        .string_len(255)
                        .not_null()
                        .default(""),
                )
                .col(
                    ColumnDef::new(Alias::new("port"))
                        .big_integer()
                        .not_null()
                        .default(22_i64),
                )
                .col(
                    ColumnDef::new(Alias::new("username"))
                        .string_len(255)
                        .not_null()
                        .default(""),
                )
                .col(
                    ColumnDef::new(Alias::new("connection"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("state"))
                        .string_len(255)
                        .not_null()
                        .default("unknown"),
                )
                .col(
                    ColumnDef::new(Alias::new("group_id"))
                        .big_integer()
                        .not_null()
                        .default(0_i64),
                )
                .col(
                    ColumnDef::new(Alias::new("description"))
                        .string_len(255)
                        .not_null()
                        .default(""),
                )
                .col(
                    ColumnDef::new(Alias::new("host_key"))
                        .string_len(255)
                        .not_null()
                        .default(""),
                )
                .col(
                    ColumnDef::new(Alias::new("runner_key_hash"))
                        .string_len(255)
                        .not_null()
                        .default(""),
                )
                .col(
                    ColumnDef::new(Alias::new("seen_at"))
                        .big_integer()
                        .not_null()
                        .default(0_i64),
                )
                .col(
                    ColumnDef::new(Alias::new("created_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("updated_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("version"))
                        .big_integer()
                        .not_null()
                        .default(1_i64),
                )
                .to_owned(),
        ),
        SchemaStatement::Index(
            "hosts",
            "hosts_workspace",
            Index::create()
                .name("hosts_workspace")
                .table(Alias::new("hosts"))
                .col(Alias::new("workspace_id"))
                .col(Alias::new("name"))
                .to_owned(),
        ),
        SchemaStatement::Index(
            "hosts",
            "hosts_group",
            Index::create()
                .name("hosts_group")
                .table(Alias::new("hosts"))
                .col(Alias::new("workspace_id"))
                .col(Alias::new("group_id"))
                .to_owned(),
        ),
        SchemaStatement::Table(
            create_table("tags", backend)
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key())
                .col(
                    ColumnDef::new(Alias::new("workspace_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("name"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("color"))
                        .string_len(255)
                        .not_null()
                        .default(""),
                )
                .col(
                    ColumnDef::new(Alias::new("created_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("updated_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("version"))
                        .big_integer()
                        .not_null()
                        .default(1_i64),
                )
                .to_owned(),
        ),
        SchemaStatement::Index(
            "tags",
            "tags_unique",
            Index::create()
                .name("tags_unique")
                .table(Alias::new("tags"))
                .unique()
                .col(Alias::new("workspace_id"))
                .col(Alias::new("name"))
                .to_owned(),
        ),
        SchemaStatement::Table(
            create_table("host_tags", backend)
                .col(
                    ColumnDef::new(Alias::new("host_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("tag_id"))
                        .big_integer()
                        .not_null(),
                )
                .primary_key(
                    Index::create()
                        .col(Alias::new("host_id"))
                        .col(Alias::new("tag_id")),
                )
                .to_owned(),
        ),
    ]
}

struct HostResources;
impl MigrationName for HostResources {
    fn name(&self) -> &str {
        "host_resources_v1"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for HostResources {
    async fn up(&self, m: &SchemaManager) -> std::result::Result<(), DbErr> {
        for statement in host_resources(m.get_database_backend()) {
            execute_schema(m, statement).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> std::result::Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

fn runner_enrolment(backend: DbBackend) -> Vec<SchemaStatement> {
    vec![
        // A host is added to a workspace by running one command on it. The token in
        // that command is single-use and short-lived, and only its hash is stored,
        // so the table is not a list of working credentials.
        //
        // `host_id` 0 means "whichever host enrols with this", which is the
        // one-command flow for a machine that has no row yet. Nonzero binds the
        // token to a host the operator already created.
        SchemaStatement::Table(
            create_table("runner_tokens", backend)
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key())
                .col(
                    ColumnDef::new(Alias::new("workspace_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("host_id"))
                        .big_integer()
                        .not_null()
                        .default(0_i64),
                )
                .col(
                    ColumnDef::new(Alias::new("token_hash"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("expires_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("used_at"))
                        .big_integer()
                        .not_null()
                        .default(0_i64),
                )
                .col(
                    ColumnDef::new(Alias::new("used_by"))
                        .big_integer()
                        .not_null()
                        .default(0_i64),
                )
                .col(
                    ColumnDef::new(Alias::new("created_by"))
                        .big_integer()
                        .not_null()
                        .default(0_i64),
                )
                .col(
                    ColumnDef::new(Alias::new("created_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("updated_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("version"))
                        .big_integer()
                        .not_null()
                        .default(1_i64),
                )
                .to_owned(),
        ),
        // Unique so two tokens can never collide, and so consuming one is a single
        // conditional UPDATE rather than a read followed by a write.
        SchemaStatement::Index(
            "runner_tokens",
            "runner_tokens_hash",
            Index::create()
                .name("runner_tokens_hash")
                .table(Alias::new("runner_tokens"))
                .unique()
                .col(Alias::new("token_hash"))
                .to_owned(),
        ),
        SchemaStatement::Index(
            "runner_tokens",
            "runner_tokens_workspace",
            Index::create()
                .name("runner_tokens_workspace")
                .table(Alias::new("runner_tokens"))
                .col(Alias::new("workspace_id"))
                .col(Alias::new("created_at"))
                .to_owned(),
        ),
        // Facts the runner reports about itself, for the host page and dashboard.
        SchemaStatement::Column(
            "hosts",
            "os",
            Table::alter()
                .table(Alias::new("hosts"))
                .add_column(
                    ColumnDef::new(Alias::new("os"))
                        .string_len(255)
                        .not_null()
                        .default(""),
                )
                .to_owned(),
        ),
        SchemaStatement::Column(
            "hosts",
            "arch",
            Table::alter()
                .table(Alias::new("hosts"))
                .add_column(
                    ColumnDef::new(Alias::new("arch"))
                        .string_len(255)
                        .not_null()
                        .default(""),
                )
                .to_owned(),
        ),
        SchemaStatement::Column(
            "hosts",
            "agent_version",
            Table::alter()
                .table(Alias::new("hosts"))
                .add_column(
                    ColumnDef::new(Alias::new("agent_version"))
                        .string_len(255)
                        .not_null()
                        .default(""),
                )
                .to_owned(),
        ),
        // The latest heartbeat's load/memory/disk as a small JSON object. MySQL
        // stores this as VARCHAR(255) (a TEXT column cannot carry a DEFAULT), which
        // is why the keys are short — see `runner::metrics_json`.
        // ponytail: last sample only, no history table. Add one when someone asks
        // for a graph rather than a number.
        SchemaStatement::Column(
            "hosts",
            "metrics",
            Table::alter()
                .table(Alias::new("hosts"))
                .add_column(
                    ColumnDef::new(Alias::new("metrics"))
                        .string_len(255)
                        .not_null()
                        .default(""),
                )
                .to_owned(),
        ),
    ]
}

struct RunnerEnrolment;
impl MigrationName for RunnerEnrolment {
    fn name(&self) -> &str {
        "runner_enrolment_v1"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for RunnerEnrolment {
    async fn up(&self, m: &SchemaManager) -> std::result::Result<(), DbErr> {
        for statement in runner_enrolment(m.get_database_backend()) {
            execute_schema(m, statement).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> std::result::Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

fn credentials(backend: DbBackend) -> Vec<SchemaStatement> {
    vec![
        // `secret` holds a sealed SSH private key, so it must not land in MySQL's
        // default VARCHAR(255); its builder explicitly selects a long column.
        SchemaStatement::Table(
            create_table("credentials", backend)
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key())
                .col(
                    ColumnDef::new(Alias::new("workspace_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("name"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("kind"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("username"))
                        .string_len(255)
                        .not_null()
                        .default(""),
                )
                .col(
                    ColumnDef::new(Alias::new("description"))
                        .string_len(255)
                        .not_null()
                        .default(""),
                )
                .col(long_text("secret", backend).not_null())
                .col(
                    ColumnDef::new(Alias::new("created_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("updated_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("version"))
                        .big_integer()
                        .not_null()
                        .default(1_i64),
                )
                .to_owned(),
        ),
        SchemaStatement::Index(
            "credentials",
            "credentials_workspace",
            Index::create()
                .name("credentials_workspace")
                .table(Alias::new("credentials"))
                .col(Alias::new("workspace_id"))
                .col(Alias::new("name"))
                .to_owned(),
        ),
        SchemaStatement::Column(
            "hosts",
            "credential_id",
            Table::alter()
                .table(Alias::new("hosts"))
                .add_column(
                    ColumnDef::new(Alias::new("credential_id"))
                        .big_integer()
                        .not_null()
                        .default(0_i64),
                )
                .to_owned(),
        ),
    ]
}

struct Credentials;
impl MigrationName for Credentials {
    fn name(&self) -> &str {
        "credentials_v1"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Credentials {
    async fn up(&self, m: &SchemaManager) -> std::result::Result<(), DbErr> {
        for statement in credentials(m.get_database_backend()) {
            execute_schema(m, statement).await?;
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
    fn all_statements() -> Vec<SchemaStatement> {
        [
            initial,
            host_resources,
            runner_enrolment,
            credentials,
            repositories,
            pipelines,
            id_infrastructure,
        ]
        .into_iter()
        .flat_map(|schema| schema(DbBackend::MySql))
        .collect()
    }

    fn mysql_sql(statement: SchemaStatement) -> String {
        match statement {
            SchemaStatement::Table(s) => s.to_string(MysqlQueryBuilder),
            SchemaStatement::Index(_, _, s) => s.to_string(MysqlQueryBuilder),
            SchemaStatement::Column(_, _, s) => s.to_string(MysqlQueryBuilder),
        }
    }

    /// Check the applied schema (including ALTERs), not just the CREATE text.
    #[tokio::test]
    async fn every_business_id_column_is_bigint() {
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let inventory = [
            ("users", "id"),
            ("sessions", "id user_id workspace_id"),
            ("workspaces", "id owner_id"),
            ("workspace_roles", "id workspace_id"),
            ("workspace_members", "id workspace_id user_id role_id"),
            ("activity", "id workspace_id actor_id resource_id"),
            ("host_groups", "id workspace_id parent_id"),
            ("hosts", "id workspace_id group_id credential_id"),
            ("tags", "id workspace_id"),
            ("host_tags", "host_id tag_id"),
            (
                "runner_tokens",
                "id workspace_id host_id used_by created_by",
            ),
            ("credentials", "id workspace_id"),
            ("repositories", "id workspace_id credential_id"),
            ("pipelines", "id workspace_id repository_id target_ref"),
        ];
        let mysql: Vec<String> = all_statements().into_iter().map(mysql_sql).collect();
        let mut count = 0;
        for (table, columns) in inventory {
            // SeaORM's entity/query builders do not expose SQLite PRAGMA metadata.
            // This schema-inspection test intentionally uses the native command.
            let rows = db
                .query_all_raw(sea_orm::Statement::from_string(
                    db.get_database_backend(),
                    format!("PRAGMA table_info({table})"),
                ))
                .await
                .unwrap();
            let mut actual = std::collections::BTreeSet::new();
            for row in rows {
                let column = row.try_get::<String>("", "name").unwrap();
                if column == "id"
                    || column.ends_with("_id")
                    || ["target_ref", "used_by", "created_by"].contains(&column.as_str())
                {
                    assert_eq!(
                        row.try_get::<String>("", "type").unwrap(),
                        // SeaQuery renders BigInteger as INTEGER on SQLite;
                        // SQLite integers are always signed 64-bit. Also check
                        // MySQL below, where INTEGER would silently narrow it.
                        "INTEGER",
                        "{table}.{column}"
                    );
                    assert!(
                        mysql.iter().any(|sql| {
                            (sql.starts_with(&format!("CREATE TABLE IF NOT EXISTS `{table}` ("))
                                || sql.starts_with(&format!("ALTER TABLE `{table}` ")))
                                && sql.contains(&format!("`{column}` bigint"))
                        }),
                        "{table}.{column} must remain BIGINT on MySQL"
                    );
                    actual.insert(column);
                    count += 1;
                }
            }
            assert_eq!(
                actual,
                columns.split_whitespace().map(str::to_owned).collect(),
                "{table}"
            );
        }
        assert_eq!(count, 41);
    }

    #[test]
    fn pipeline_definition_is_longtext_without_default() {
        let sql = mysql_sql(pipelines(DbBackend::MySql).remove(0));
        assert!(sql.contains("`definition` longtext NOT NULL,"), "{sql}");
    }

    #[test]
    fn no_long_column_carries_a_default() {
        // Inspect individual column clauses: generated SQL is a single line,
        // and defaults on neighbouring VARCHAR/BIGINT columns are legitimate.
        for statement in all_statements() {
            let sql = mysql_sql(statement);
            for clause in sql.split(',') {
                let clause = clause.to_uppercase();
                assert!(
                    !(clause.contains("TEXT") && clause.contains("DEFAULT")),
                    "MySQL rejects a DEFAULT on a TEXT/LONGTEXT column: {clause}\nin: {sql}"
                );
            }
        }
    }

    #[test]
    fn no_text_column_survives_into_mysql() {
        for statement in all_statements() {
            let sql = mysql_sql(statement);
            assert!(
                !sql.to_uppercase().contains(" TEXT "),
                "a TEXT column reached MySQL unsized: {sql}"
            );
        }
    }

    #[test]
    fn mysql_tables_preserve_binary_collation() {
        for statement in all_statements() {
            if let SchemaStatement::Table(table) = statement {
                let sql = table.to_string(MysqlQueryBuilder);
                assert!(sql.contains("DEFAULT CHARSET=utf8mb4"), "{sql}");
                assert!(sql.contains("COLLATE=utf8mb4_bin"), "{sql}");
            }
        }
    }
}

fn repositories(backend: DbBackend) -> Vec<SchemaStatement> {
    vec![
        SchemaStatement::Table(
            create_table("repositories", backend)
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key())
                .col(
                    ColumnDef::new(Alias::new("workspace_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("name"))
                        .string_len(255)
                        .not_null(),
                )
                .col(ColumnDef::new(Alias::new("url")).string_len(255).not_null())
                .col(
                    ColumnDef::new(Alias::new("default_branch"))
                        .string_len(255)
                        .not_null()
                        .default("main"),
                )
                .col(
                    ColumnDef::new(Alias::new("credential_id"))
                        .big_integer()
                        .not_null()
                        .default(0_i64),
                )
                .col(
                    ColumnDef::new(Alias::new("description"))
                        .string_len(255)
                        .not_null()
                        .default(""),
                )
                .col(
                    ColumnDef::new(Alias::new("created_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("updated_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("version"))
                        .big_integer()
                        .not_null()
                        .default(1_i64),
                )
                .to_owned(),
        ),
        SchemaStatement::Index(
            "repositories",
            "repositories_workspace",
            Index::create()
                .name("repositories_workspace")
                .table(Alias::new("repositories"))
                .col(Alias::new("workspace_id"))
                .col(Alias::new("name"))
                .to_owned(),
        ),
    ]
}

struct Repositories;
impl MigrationName for Repositories {
    fn name(&self) -> &str {
        "repositories_v1"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Repositories {
    async fn up(&self, m: &SchemaManager) -> std::result::Result<(), DbErr> {
        for statement in repositories(m.get_database_backend()) {
            execute_schema(m, statement).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> std::result::Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

// target_ref is a polymorphic row ID; 0 means no reference (validation requires a target).
fn pipelines(backend: DbBackend) -> Vec<SchemaStatement> {
    vec![
        SchemaStatement::Table(
            create_table("pipelines", backend)
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key())
                .col(
                    ColumnDef::new(Alias::new("workspace_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("name"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("repository_id"))
                        .big_integer()
                        .not_null()
                        .default(0_i64),
                )
                .col(long_text("definition", backend).not_null())
                .col(
                    ColumnDef::new(Alias::new("target_kind"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("target_ref"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("description"))
                        .string_len(255)
                        .not_null()
                        .default(""),
                )
                .col(
                    ColumnDef::new(Alias::new("created_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("updated_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("version"))
                        .big_integer()
                        .not_null()
                        .default(1_i64),
                )
                .to_owned(),
        ),
        SchemaStatement::Index(
            "pipelines",
            "pipelines_workspace",
            Index::create()
                .name("pipelines_workspace")
                .table(Alias::new("pipelines"))
                .col(Alias::new("workspace_id"))
                .col(Alias::new("name"))
                .to_owned(),
        ),
    ]
}

struct Pipelines;
impl MigrationName for Pipelines {
    fn name(&self) -> &str {
        "pipelines_v1"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Pipelines {
    async fn up(&self, m: &SchemaManager) -> std::result::Result<(), DbErr> {
        for statement in pipelines(m.get_database_backend()) {
            execute_schema(m, statement).await?;
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
        for statement in id_infrastructure(m.get_database_backend()) {
            execute_schema(m, statement).await?;
        }
        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除 worker 租约".into()))
    }
}

fn id_infrastructure(backend: DbBackend) -> Vec<SchemaStatement> {
    vec![
        SchemaStatement::Table(
            create_table("locks", backend)
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
        ),
        SchemaStatement::Table(
            create_table("worker_ids", backend)
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
        ),
    ]
}
