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
            Box::new(PipelineExecution),
            Box::new(ComposeDeployments),
            Box::new(PlatformInitialization),
            Box::new(Variables),
            Box::new(WorkspaceAdministration),
            Box::new(DeploymentVariableSnapshots),
            Box::new(ResourceTemplates),
            Box::new(WorkspacePreferences),
            Box::new(ScheduledTasks),
            Box::new(AlertMonitoring),
            Box::new(Marketplace),
            Box::new(RepositoryAuthorization),
            Box::new(RepositoryAuthorizationBinding),
            Box::new(PipelineArtifacts),
            Box::new(ConfigTemplates),
            Box::new(DeploymentArtifacts),
            Box::new(DeploymentGroups),
            Box::new(ResourceImports),
            Box::new(PipelineVariableSnapshots),
            Box::new(PlatformRepositoryOAuth),
            Box::new(ActionsJobs),
            Box::new(PipelineClassification),
            Box::new(RunnerUpdateState),
            Box::new(NamedCodeConnections),
            Box::new(HostGroupOrder),
            Box::new(HostConnectionCommand),
            Box::new(HostJump),
            Box::new(PipelineGroupOrder),
            Box::new(CredentialScopes),
            Box::new(RepositoryMonitoring),
            Box::new(RepositoryPolling),
        ]
    }
}

struct PipelineGroupOrder;
impl MigrationName for PipelineGroupOrder {
    fn name(&self) -> &str {
        "pipeline_group_order_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for PipelineGroupOrder {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        execute_schema(
            m,
            SchemaStatement::Column(
                "pipeline_contexts",
                "position",
                Table::alter()
                    .table(Alias::new("pipeline_contexts"))
                    .add_column(
                        ColumnDef::new(Alias::new("position"))
                            .big_integer()
                            .not_null()
                            .default(0_i64),
                    )
                    .to_owned(),
            ),
        )
        .await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct HostJump;
fn host_jump() -> SchemaStatement {
    SchemaStatement::Column(
        "hosts",
        "jump_host_id",
        Table::alter()
            .table(Alias::new("hosts"))
            .add_column(
                ColumnDef::new(Alias::new("jump_host_id"))
                    .big_integer()
                    .not_null()
                    .default(0_i64),
            )
            .to_owned(),
    )
}
impl MigrationName for HostJump {
    fn name(&self) -> &str {
        "host_jump_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for HostJump {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        execute_schema(m, host_jump()).await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct HostConnectionCommand;
impl MigrationName for HostConnectionCommand {
    fn name(&self) -> &str {
        "host_connection_command_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for HostConnectionCommand {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for (name, column) in [
            (
                "enroll_token",
                ColumnDef::new(Alias::new("enroll_token"))
                    // A sealed 64-character token fits here. MySQL rejects a
                    // literal DEFAULT on TEXT when upgrading existing hosts.
                    .string_len(512)
                    .not_null()
                    .default("")
                    .to_owned(),
            ),
            (
                "enroll_token_hash",
                ColumnDef::new(Alias::new("enroll_token_hash"))
                    .string_len(64)
                    .not_null()
                    .default("")
                    .to_owned(),
            ),
        ] {
            execute_schema(
                m,
                SchemaStatement::Column(
                    "hosts",
                    name,
                    Table::alter()
                        .table(Alias::new("hosts"))
                        .add_column(&mut column.clone())
                        .to_owned(),
                ),
            )
            .await?;
        }
        execute_schema(
            m,
            SchemaStatement::Index(
                "hosts",
                "hosts_enroll_token_hash",
                Index::create()
                    .name("hosts_enroll_token_hash")
                    .table(Alias::new("hosts"))
                    .col(Alias::new("enroll_token_hash"))
                    .to_owned(),
            ),
        )
        .await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct HostGroupOrder;
impl MigrationName for HostGroupOrder {
    fn name(&self) -> &str {
        "host_group_order_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for HostGroupOrder {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        execute_schema(
            m,
            SchemaStatement::Column(
                "host_groups",
                "position",
                Table::alter()
                    .table(Alias::new("host_groups"))
                    .add_column(
                        ColumnDef::new(Alias::new("position"))
                            .big_integer()
                            .not_null()
                            .default(0_i64),
                    )
                    .to_owned(),
            ),
        )
        .await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
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
            pipeline_execution,
            compose_deployments,
            variables,
            pipeline_classification,
        ]
        .into_iter()
        .flat_map(|schema| schema(DbBackend::MySql))
        .chain(std::iter::once(host_jump()))
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
            (
                "hosts",
                "id workspace_id group_id credential_id jump_host_id",
            ),
            ("tags", "id workspace_id"),
            ("host_tags", "host_id tag_id"),
            (
                "runner_tokens",
                "id workspace_id host_id used_by created_by",
            ),
            ("credentials", "id workspace_id"),
            ("variables", "id workspace_id scope_ref"),
            ("repositories", "id workspace_id credential_id"),
            (
                "pipelines",
                "id workspace_id repository_id target_ref group_id application_id",
            ),
            ("pipeline_contexts", "id workspace_id target_ref"),
            ("pipeline_links", "pipeline_id reference_id"),
            (
                "pipeline_runs",
                "id workspace_id pipeline_id host_id triggered_by credential_id environment_id",
            ),
            ("pipeline_run_steps", "id run_id"),
            ("pipeline_run_logs", "id run_id"),
            ("deployments", "id workspace_id target_ref current_revision"),
            (
                "deployment_revisions",
                "id workspace_id deployment_id source_revision triggered_by",
            ),
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
                    || [
                        "target_ref",
                        "scope_ref",
                        "used_by",
                        "created_by",
                        "triggered_by",
                        "current_revision",
                        "source_revision",
                    ]
                    .contains(&column.as_str())
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
        assert_eq!(count, 72);
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
            // Table DEFAULT CHARSET is not a column default (the last column
            // may itself be LONGTEXT). Inspect only the column clauses.
            let columns = sql.split(") DEFAULT CHARSET=").next().unwrap();
            for clause in columns.split(',') {
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

fn pipeline_execution(backend: DbBackend) -> Vec<SchemaStatement> {
    vec![
        SchemaStatement::Column(
            "workspaces",
            "pipeline_run_number",
            Table::alter()
                .table(Alias::new("workspaces"))
                .add_column(
                    ColumnDef::new(Alias::new("pipeline_run_number"))
                        .big_integer()
                        .not_null()
                        .default(0_i64),
                )
                .to_owned(),
        ),
        SchemaStatement::Table(
            create_table("pipeline_runs", backend)
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key())
                .col(
                    ColumnDef::new(Alias::new("workspace_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("pipeline_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("host_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("number"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("state"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("trigger_kind"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("triggered_by"))
                        .big_integer()
                        .not_null(),
                )
                .col(long_text("definition", backend).not_null())
                .col(long_text("repo_url", backend).not_null())
                .col(
                    ColumnDef::new(Alias::new("branch"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("commit"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("credential_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("created_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("started_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("finished_at"))
                        .big_integer()
                        .not_null(),
                )
                .to_owned(),
        ),
        SchemaStatement::Table(
            create_table("pipeline_run_steps", backend)
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key())
                .col(
                    ColumnDef::new(Alias::new("run_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("step_index"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("name"))
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("state"))
                        .string_len(255)
                        .not_null(),
                )
                .col(ColumnDef::new(Alias::new("exit_code")).integer())
                .col(
                    ColumnDef::new(Alias::new("started_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("finished_at"))
                        .big_integer()
                        .not_null(),
                )
                .to_owned(),
        ),
        SchemaStatement::Table(
            create_table("pipeline_run_logs", backend)
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key())
                .col(
                    ColumnDef::new(Alias::new("run_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(ColumnDef::new(Alias::new("seq")).big_integer().not_null())
                .col(
                    ColumnDef::new(Alias::new("step_index"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("stream"))
                        .string_len(255)
                        .not_null(),
                )
                .col(long_text("chunk", backend).not_null())
                .to_owned(),
        ),
        SchemaStatement::Index(
            "pipeline_runs",
            "pipeline_runs_number",
            Index::create()
                .name("pipeline_runs_number")
                .table(Alias::new("pipeline_runs"))
                .unique()
                .col(Alias::new("workspace_id"))
                .col(Alias::new("number"))
                .to_owned(),
        ),
        SchemaStatement::Index(
            "pipeline_runs",
            "pipeline_runs_queue",
            Index::create()
                .name("pipeline_runs_queue")
                .table(Alias::new("pipeline_runs"))
                .col(Alias::new("state"))
                .col(Alias::new("created_at"))
                .to_owned(),
        ),
        SchemaStatement::Index(
            "pipeline_run_steps",
            "pipeline_run_steps_order",
            Index::create()
                .name("pipeline_run_steps_order")
                .table(Alias::new("pipeline_run_steps"))
                .unique()
                .col(Alias::new("run_id"))
                .col(Alias::new("step_index"))
                .to_owned(),
        ),
        SchemaStatement::Index(
            "pipeline_run_logs",
            "pipeline_run_logs_seq",
            Index::create()
                .name("pipeline_run_logs_seq")
                .table(Alias::new("pipeline_run_logs"))
                .unique()
                .col(Alias::new("run_id"))
                .col(Alias::new("seq"))
                .to_owned(),
        ),
    ]
}
struct PipelineExecution;
impl MigrationName for PipelineExecution {
    fn name(&self) -> &str {
        "pipeline_execution_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for PipelineExecution {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for statement in pipeline_execution(m.get_database_backend()) {
            execute_schema(m, statement).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

fn compose_deployments(backend: DbBackend) -> Vec<SchemaStatement> {
    let mut statements = Vec::new();
    for (name, strings, texts, integers) in [
        (
            "deployments",
            vec!["name", "mode", "target_kind", "workdir", "state"],
            vec!["manifest", "description", "preview_artifact"],
            vec!["target_ref", "revision_number", "current_revision"],
        ),
        (
            "deployment_revisions",
            vec!["action", "state", "expected_hash"],
            vec!["manifest", "rendered", "artifact", "output"],
            vec![
                "deployment_id",
                "number",
                "source_revision",
                "triggered_by",
                "started_at",
                "finished_at",
            ],
        ),
    ] {
        let mut table = create_table(name, backend);
        table.col(ColumnDef::new(Alias::new("id")).big_integer().primary_key());
        for field in ["workspace_id", "created_at", "updated_at", "version"]
            .into_iter()
            .chain(integers)
        {
            table.col(ColumnDef::new(Alias::new(field)).big_integer().not_null());
        }
        for field in strings {
            table.col(
                ColumnDef::new(Alias::new(field))
                    .string_len(if field == "workdir" { 1024 } else { 255 })
                    .not_null(),
            );
        }
        for field in texts {
            table.col(long_text(field, backend).not_null());
        }
        statements.push(SchemaStatement::Table(table));
    }
    statements.push(SchemaStatement::Index(
        "deployment_revisions",
        "deployment_revision_number",
        Index::create()
            .name("deployment_revision_number")
            .table(Alias::new("deployment_revisions"))
            .col(Alias::new("deployment_id"))
            .col(Alias::new("number"))
            .unique()
            .to_owned(),
    ));
    statements
}
struct ComposeDeployments;
impl MigrationName for ComposeDeployments {
    fn name(&self) -> &str {
        "compose_deployments_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for ComposeDeployments {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for statement in compose_deployments(m.get_database_backend()) {
            execute_schema(m, statement).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct PlatformInitialization;
impl MigrationName for PlatformInitialization {
    fn name(&self) -> &str {
        "platform_initialization_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for PlatformInitialization {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for (name, default) in [("is_superadmin", 0_i64), ("version", 1_i64)] {
            execute_schema(
                m,
                SchemaStatement::Column(
                    "users",
                    name,
                    Table::alter()
                        .table(Alias::new("users"))
                        .add_column(
                            ColumnDef::new(Alias::new(name))
                                .big_integer()
                                .not_null()
                                .default(default),
                        )
                        .to_owned(),
                ),
            )
            .await?;
        }
        m.create_table(
            create_table("platform_state", m.get_database_backend())
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key())
                .col(
                    ColumnDef::new(Alias::new("initialized"))
                        .big_integer()
                        .not_null()
                        .default(0_i64),
                )
                .col(
                    ColumnDef::new(Alias::new("registration_enabled"))
                        .big_integer()
                        .not_null()
                        .default(1_i64),
                )
                .col(
                    ColumnDef::new(Alias::new("version"))
                        .big_integer()
                        .not_null()
                        .default(1_i64),
                )
                .to_owned(),
        )
        .await?;
        use sea_orm::{EntityTrait, QuerySelect, Set};
        if crate::entities::platform_state::Entity::find_by_id(1)
            .select_only()
            .column(crate::entities::platform_state::Column::Id)
            .into_tuple::<i64>()
            .one(m.get_connection())
            .await?
            .is_none()
        {
            crate::entities::platform_state::Entity::insert(
                crate::entities::platform_state::ActiveModel {
                    id: Set(1),
                    initialized: Set(0),
                    registration_enabled: Set(1),
                    version: Set(1),
                    ..Default::default()
                },
            )
            .exec(m.get_connection())
            .await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

fn variables(backend: DbBackend) -> Vec<SchemaStatement> {
    let mut table = create_table("variables", backend);
    table.col(ColumnDef::new(Alias::new("id")).big_integer().primary_key());
    for field in [
        "workspace_id",
        "scope_ref",
        "created_at",
        "updated_at",
        "version",
    ] {
        table.col(ColumnDef::new(Alias::new(field)).big_integer().not_null());
    }
    for (field, length) in [("name", 96), ("scope", 16), ("kind", 16)] {
        table.col(
            ColumnDef::new(Alias::new(field))
                .string_len(length)
                .not_null(),
        );
    }
    for field in ["description", "value"] {
        table.col(long_text(field, backend).not_null());
    }
    vec![
        SchemaStatement::Table(table),
        SchemaStatement::Index(
            "variables",
            "variables_scope_name",
            Index::create()
                .name("variables_scope_name")
                .table(Alias::new("variables"))
                .col(Alias::new("workspace_id"))
                .col(Alias::new("scope"))
                .col(Alias::new("scope_ref"))
                .col(Alias::new("name"))
                .unique()
                .to_owned(),
        ),
    ]
}
struct Variables;
impl MigrationName for Variables {
    fn name(&self) -> &str {
        "variables_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Variables {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for statement in variables(m.get_database_backend()) {
            execute_schema(m, statement).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct WorkspaceAdministration;
impl MigrationName for WorkspaceAdministration {
    fn name(&self) -> &str {
        "workspace_administration_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for WorkspaceAdministration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for table in ["workspace_members", "workspace_roles"] {
            execute_schema(
                m,
                SchemaStatement::Column(
                    table,
                    "version",
                    Table::alter()
                        .table(Alias::new(table))
                        .add_column(
                            ColumnDef::new(Alias::new("version"))
                                .big_integer()
                                .not_null()
                                .default(1_i64),
                        )
                        .to_owned(),
                ),
            )
            .await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct DeploymentVariableSnapshots;
impl MigrationName for DeploymentVariableSnapshots {
    fn name(&self) -> &str {
        "deployment_variable_snapshots_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for DeploymentVariableSnapshots {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        execute_schema(
            m,
            SchemaStatement::Column(
                "deployment_revisions",
                "variable_snapshot",
                Table::alter()
                    .table(Alias::new("deployment_revisions"))
                    .add_column(long_text("variable_snapshot", m.get_database_backend()))
                    .to_owned(),
            ),
        )
        .await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct ResourceTemplates;
impl MigrationName for ResourceTemplates {
    fn name(&self) -> &str {
        "resource_templates_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for ResourceTemplates {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for name in ["app_templates", "pipeline_templates"] {
            let backend = m.get_database_backend();
            let mut table = create_table(name, backend);
            table.col(ColumnDef::new(Alias::new("id")).big_integer().primary_key());
            for key in ["workspace_id", "version", "created_at", "updated_at"] {
                table.col(ColumnDef::new(Alias::new(key)).big_integer().not_null());
            }
            table.col(
                ColumnDef::new(Alias::new("name"))
                    .string_len(255)
                    .not_null(),
            );
            for key in ["description", "content", "parameters"] {
                table.col(long_text(key, backend).not_null());
            }
            execute_schema(m, SchemaStatement::Table(table)).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct WorkspacePreferences;
impl MigrationName for WorkspacePreferences {
    fn name(&self) -> &str {
        "workspace_preferences_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for WorkspacePreferences {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        for name in ["settings", "dictionaries", "dictionary_items"] {
            let mut t = create_table(name, backend);
            t.col(ColumnDef::new(Alias::new("id")).big_integer().primary_key());
            for key in ["workspace_id", "version", "created_at", "updated_at"] {
                t.col(ColumnDef::new(Alias::new(key)).big_integer().not_null());
            }
            if name == "dictionary_items" {
                for key in ["dictionary_id", "position"] {
                    t.col(ColumnDef::new(Alias::new(key)).big_integer().not_null());
                }
                for key in ["value", "label"] {
                    t.col(ColumnDef::new(Alias::new(key)).string_len(100).not_null());
                }
                t.index(
                    Index::create()
                        .unique()
                        .col(Alias::new("dictionary_id"))
                        .col(Alias::new("value")),
                );
            } else {
                t.col(ColumnDef::new(Alias::new("key")).string_len(60).not_null());
                t.index(
                    Index::create()
                        .unique()
                        .col(Alias::new("workspace_id"))
                        .col(Alias::new("key")),
                );
                if name == "settings" {
                    t.col(ColumnDef::new(Alias::new("value")).big_integer().not_null());
                } else {
                    t.col(
                        ColumnDef::new(Alias::new("name"))
                            .string_len(100)
                            .not_null(),
                    );
                    t.col(long_text("description", backend).not_null());
                    t.col(
                        ColumnDef::new(Alias::new("builtin"))
                            .big_integer()
                            .not_null(),
                    );
                }
            }
            execute_schema(m, SchemaStatement::Table(t)).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct ScheduledTasks;
impl MigrationName for ScheduledTasks {
    fn name(&self) -> &str {
        "scheduled_tasks_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for ScheduledTasks {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        for name in ["cron_jobs", "cron_runs"] {
            let mut t = create_table(name, backend);
            t.col(ColumnDef::new(Alias::new("id")).big_integer().primary_key());
            for key in ["workspace_id", "version", "created_at", "updated_at"] {
                t.col(ColumnDef::new(Alias::new(key)).big_integer().not_null());
            }
            t.col(
                ColumnDef::new(Alias::new("scope"))
                    .string_len(20)
                    .not_null(),
            );
            t.col(long_text("config", backend).not_null());
            if name == "cron_jobs" {
                for key in ["name", "run_as_name", "target_kind"] {
                    t.col(ColumnDef::new(Alias::new(key)).string_len(100).not_null());
                }
                t.col(long_text("description", backend).not_null());
                for key in ["enabled", "run_as", "next_run_at", "target_ref"] {
                    t.col(ColumnDef::new(Alias::new(key)).big_integer().not_null());
                }
            } else {
                for key in [
                    "job_name",
                    "state",
                    "trigger_kind",
                    "downstream_kind",
                    "target_kind",
                ] {
                    t.col(ColumnDef::new(Alias::new(key)).string_len(100).not_null());
                }
                for key in [
                    "target_ref",
                    "job_id",
                    "triggered_by",
                    "scheduled_at",
                    "started_at",
                    "finished_at",
                    "attempt",
                    "downstream_id",
                ] {
                    t.col(ColumnDef::new(Alias::new(key)).big_integer().not_null());
                }
                t.col(long_text("output", backend).not_null());
            }
            execute_schema(m, SchemaStatement::Table(t)).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct AlertMonitoring;
impl MigrationName for AlertMonitoring {
    fn name(&self) -> &str {
        "alert_monitoring_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for AlertMonitoring {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        for name in ["alert_rules", "alert_events"] {
            let mut t = create_table(name, backend);
            t.col(ColumnDef::new(Alias::new("id")).big_integer().primary_key());
            for key in [
                "workspace_id",
                "target_ref",
                "version",
                "created_at",
                "updated_at",
            ] {
                t.col(ColumnDef::new(Alias::new(key)).big_integer().not_null());
            }
            t.col(
                ColumnDef::new(Alias::new("target_kind"))
                    .string_len(30)
                    .not_null(),
            );
            let (numbers, texts): (&[&str], &[&str]) = if name == "alert_rules" {
                (
                    &["threshold", "enabled", "run_as"],
                    &["name", "description", "metric", "webhook", "last_error"],
                )
            } else {
                (
                    &["rule_id", "triggered_at", "recovered_at"],
                    &[
                        "rule_name",
                        "state",
                        "message",
                        "trigger_delivery",
                        "recovery_delivery",
                    ],
                )
            };
            for key in numbers {
                t.col(ColumnDef::new(Alias::new(*key)).big_integer().not_null());
            }
            for key in texts {
                t.col(long_text(key, backend).not_null());
            }
            execute_schema(m, SchemaStatement::Table(t)).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct Marketplace;
impl MigrationName for Marketplace {
    fn name(&self) -> &str {
        "external_repository_marketplace_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Marketplace {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        for name in ["marketplace_sources", "marketplace_entries"] {
            let mut t = create_table(name, backend);
            t.col(ColumnDef::new(Alias::new("id")).big_integer().primary_key());
            for key in ["workspace_id", "version", "created_at", "updated_at"] {
                t.col(ColumnDef::new(Alias::new(key)).big_integer().not_null());
            }
            let (numbers, texts): (&[&str], &[&str]) = if name == "marketplace_sources" {
                (
                    &["repository_id", "synced_at", "run_as"],
                    &["name", "config", "state", "last_error"],
                )
            } else {
                (
                    &["source_id"],
                    &["entry_key", "name", "description", "content", "parameters"],
                )
            };
            for key in numbers {
                t.col(ColumnDef::new(Alias::new(*key)).big_integer().not_null());
            }
            for key in texts {
                t.col(long_text(key, backend).not_null());
            }
            execute_schema(m, SchemaStatement::Table(t)).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct RepositoryAuthorization;
impl MigrationName for RepositoryAuthorization {
    fn name(&self) -> &str {
        "repository_web_authorization_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for RepositoryAuthorization {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        for name in ["repository_oauth_flows", "repository_connections"] {
            let mut t = create_table(name, backend);
            t.col(ColumnDef::new(Alias::new("id")).big_integer().primary_key());
            for key in [
                "workspace_id",
                "user_id",
                "version",
                "expires_at",
                "created_at",
                "updated_at",
            ] {
                t.col(ColumnDef::new(Alias::new(key)).big_integer().not_null());
            }
            t.col(
                ColumnDef::new(Alias::new("provider"))
                    .string_len(20)
                    .not_null(),
            );
            if name == "repository_oauth_flows" {
                t.col(
                    ColumnDef::new(Alias::new("state_hash"))
                        .string_len(64)
                        .not_null()
                        .unique_key(),
                );
                t.col(long_text("proof", backend).not_null());
            } else {
                t.col(
                    ColumnDef::new(Alias::new("credential_id"))
                        .big_integer()
                        .not_null(),
                );
                for key in [
                    "account",
                    "refresh_secret",
                    "state",
                    "last_error",
                    "client_id",
                ] {
                    t.col(long_text(key, backend).not_null());
                }
                t.index(
                    Index::create()
                        .name("repository_connection_identity")
                        .unique()
                        .col(Alias::new("workspace_id"))
                        .col(Alias::new("user_id"))
                        .col(Alias::new("provider")),
                );
            }
            execute_schema(m, SchemaStatement::Table(t)).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct RepositoryAuthorizationBinding;
impl MigrationName for RepositoryAuthorizationBinding {
    fn name(&self) -> &str {
        "repository_authorization_binding_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for RepositoryAuthorizationBinding {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        execute_schema(
            m,
            SchemaStatement::Column(
                "repository_connections",
                "config_fingerprint",
                Table::alter()
                    .table(Alias::new("repository_connections"))
                    .add_column(
                        ColumnDef::new(Alias::new("config_fingerprint"))
                            .string_len(64)
                            .not_null()
                            .default(""),
                    )
                    .to_owned(),
            ),
        )
        .await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct PipelineArtifacts;
impl MigrationName for PipelineArtifacts {
    fn name(&self) -> &str {
        "pipeline_artifacts_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for PipelineArtifacts {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let mut table = create_table("pipeline_artifacts", m.get_database_backend());
        table.col(ColumnDef::new(Alias::new("id")).big_integer().primary_key());
        for name in [
            "workspace_id",
            "pipeline_id",
            "run_id",
            "size",
            "mode",
            "created_at",
        ] {
            table.col(ColumnDef::new(Alias::new(name)).big_integer().not_null());
        }
        table.col(long_text("path", m.get_database_backend()).not_null());
        table.col(
            ColumnDef::new(Alias::new("sha256"))
                .string_len(64)
                .not_null(),
        );
        execute_schema(m, SchemaStatement::Table(table)).await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct ConfigTemplates;
impl MigrationName for ConfigTemplates {
    fn name(&self) -> &str {
        "configuration_templates_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for ConfigTemplates {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for name in ["config_templates"] {
            let backend = m.get_database_backend();
            let mut table = create_table(name, backend);
            table.col(ColumnDef::new(Alias::new("id")).big_integer().primary_key());
            for key in ["workspace_id", "version", "created_at", "updated_at"] {
                table.col(ColumnDef::new(Alias::new(key)).big_integer().not_null());
            }
            table.col(
                ColumnDef::new(Alias::new("name"))
                    .string_len(255)
                    .not_null(),
            );
            for key in ["description", "content", "parameters"] {
                table.col(long_text(key, backend).not_null());
            }
            execute_schema(m, SchemaStatement::Table(table)).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct DeploymentArtifacts;
impl MigrationName for DeploymentArtifacts {
    fn name(&self) -> &str {
        "deployment_artifact_refs_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for DeploymentArtifacts {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let mut table = create_table("deployment_artifact_refs", m.get_database_backend());
        table.col(ColumnDef::new(Alias::new("id")).big_integer().primary_key());
        for key in ["workspace_id", "revision_id", "artifact_id", "run_id"] {
            table.col(ColumnDef::new(Alias::new(key)).big_integer().not_null());
        }
        execute_schema(m, SchemaStatement::Table(table)).await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct DeploymentGroups;
impl MigrationName for DeploymentGroups {
    fn name(&self) -> &str {
        "deployment_group_targets_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for DeploymentGroups {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let mut table = create_table("deployment_revision_targets", m.get_database_backend());
        table.col(ColumnDef::new(Alias::new("id")).big_integer().primary_key());
        for key in [
            "workspace_id",
            "revision_id",
            "host_id",
            "position",
            "started_at",
            "finished_at",
        ] {
            table.col(ColumnDef::new(Alias::new(key)).big_integer().not_null());
        }
        for key in ["artifact", "expected_hash", "state", "output"] {
            table.col(long_text(key, m.get_database_backend()).not_null());
        }
        execute_schema(m, SchemaStatement::Table(table)).await?;
        let mut table = create_table("deployment_target_states", m.get_database_backend());
        table.col(ColumnDef::new(Alias::new("id")).big_integer().primary_key());
        for key in [
            "workspace_id",
            "deployment_id",
            "host_id",
            "current_revision",
        ] {
            table.col(ColumnDef::new(Alias::new(key)).big_integer().not_null());
        }
        execute_schema(m, SchemaStatement::Table(table)).await?;
        execute_schema(
            m,
            SchemaStatement::Index(
                "deployment_target_states",
                "deployment_target_current_unique",
                Index::create()
                    .name("deployment_target_current_unique")
                    .table(Alias::new("deployment_target_states"))
                    .col(Alias::new("deployment_id"))
                    .col(Alias::new("host_id"))
                    .unique()
                    .to_owned(),
            ),
        )
        .await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct ResourceImports;
impl MigrationName for ResourceImports {
    fn name(&self) -> &str {
        "resource_imports_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for ResourceImports {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        let mut table = create_table("resource_imports", backend);
        table.col(ColumnDef::new(Alias::new("id")).big_integer().primary_key());
        for key in ["workspace_id", "actor_id", "created_at", "committed_at"] {
            table.col(ColumnDef::new(Alias::new(key)).big_integer().not_null());
        }
        for key in ["resource", "payload", "result"] {
            table.col(long_text(key, backend).not_null());
        }
        execute_schema(m, SchemaStatement::Table(table)).await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct PipelineVariableSnapshots;
impl MigrationName for PipelineVariableSnapshots {
    fn name(&self) -> &str {
        "pipeline_variable_snapshots_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for PipelineVariableSnapshots {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        execute_schema(
            m,
            SchemaStatement::Column(
                "pipeline_runs",
                "variable_snapshot",
                Table::alter()
                    .table(Alias::new("pipeline_runs"))
                    .add_column(long_text("variable_snapshot", m.get_database_backend()))
                    .to_owned(),
            ),
        )
        .await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct PlatformRepositoryOAuth;
impl MigrationName for PlatformRepositoryOAuth {
    fn name(&self) -> &str {
        "platform_repository_oauth_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for PlatformRepositoryOAuth {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        execute_schema(
            m,
            SchemaStatement::Column(
                "repository_connections",
                "token_config",
                Table::alter()
                    .table(Alias::new("repository_connections"))
                    .add_column(long_text("token_config", m.get_database_backend()))
                    .to_owned(),
            ),
        )
        .await?;
        execute_schema(
            m,
            SchemaStatement::Column(
                "repositories",
                "provider",
                Table::alter()
                    .table(Alias::new("repositories"))
                    .add_column(
                        ColumnDef::new(Alias::new("provider"))
                            .string_len(32)
                            .not_null()
                            .default("git"),
                    )
                    .to_owned(),
            ),
        )
        .await?;
        execute_schema(
            m,
            SchemaStatement::Column(
                "platform_state",
                "repository_oauth",
                Table::alter()
                    .table(Alias::new("platform_state"))
                    .add_column(long_text("repository_oauth", m.get_database_backend()))
                    .to_owned(),
            ),
        )
        .await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct ActionsJobs;
impl MigrationName for ActionsJobs {
    fn name(&self) -> &str {
        "actions_jobs_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for ActionsJobs {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let mut table = create_table("pipeline_run_jobs", m.get_database_backend());
        table.col(ColumnDef::new(Alias::new("id")).big_integer().primary_key());
        for name in ["run_id", "host_id", "started_at", "finished_at"] {
            table.col(ColumnDef::new(Alias::new(name)).big_integer().not_null());
        }
        for name in ["job_key", "state"] {
            table.col(ColumnDef::new(Alias::new(name)).string_len(255).not_null());
        }
        table.col(long_text("outputs", m.get_database_backend()));
        execute_schema(m, SchemaStatement::Table(table)).await?;
        execute_schema(
            m,
            SchemaStatement::Index(
                "pipeline_run_jobs",
                "pipeline_run_jobs_key",
                Index::create()
                    .name("pipeline_run_jobs_key")
                    .table(Alias::new("pipeline_run_jobs"))
                    .col(Alias::new("run_id"))
                    .col(Alias::new("job_key"))
                    .unique()
                    .to_owned(),
            ),
        )
        .await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct PipelineClassification;
impl MigrationName for PipelineClassification {
    fn name(&self) -> &str {
        "pipeline_classification_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for PipelineClassification {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for statement in pipeline_classification(m.get_database_backend()) {
            execute_schema(m, statement).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

fn pipeline_classification(backend: DbBackend) -> Vec<SchemaStatement> {
    let mut table = create_table("pipeline_contexts", backend);
    table.col(ColumnDef::new(Alias::new("id")).big_integer().primary_key());
    for name in [
        "workspace_id",
        "target_ref",
        "created_at",
        "updated_at",
        "version",
    ] {
        table.col(ColumnDef::new(Alias::new(name)).big_integer().not_null());
    }
    for name in ["kind", "name", "target_kind"] {
        table.col(ColumnDef::new(Alias::new(name)).string_len(255).not_null());
    }
    table.col(long_text("description", backend).not_null());
    let mut statements = vec![SchemaStatement::Table(table)];
    let mut links = create_table("pipeline_links", backend);
    links
        .col(
            ColumnDef::new(Alias::new("pipeline_id"))
                .big_integer()
                .not_null(),
        )
        .col(ColumnDef::new(Alias::new("kind")).string_len(32).not_null())
        .col(
            ColumnDef::new(Alias::new("reference_id"))
                .big_integer()
                .not_null(),
        )
        .primary_key(
            Index::create()
                .col(Alias::new("pipeline_id"))
                .col(Alias::new("kind"))
                .col(Alias::new("reference_id")),
        );
    statements.push(SchemaStatement::Table(links));
    for (table, column) in [
        ("pipelines", "group_id"),
        ("pipelines", "application_id"),
        ("pipeline_runs", "environment_id"),
    ] {
        statements.push(SchemaStatement::Column(
            table,
            column,
            Table::alter()
                .table(Alias::new(table))
                .add_column(
                    ColumnDef::new(Alias::new(column))
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .to_owned(),
        ));
    }
    statements.push(SchemaStatement::Column(
        "pipeline_runs",
        "environment_name",
        Table::alter()
            .table(Alias::new("pipeline_runs"))
            .add_column(
                ColumnDef::new(Alias::new("environment_name"))
                    .string_len(255)
                    .not_null()
                    .default(""),
            )
            .to_owned(),
    ));
    statements
}

struct RunnerUpdateState;
impl MigrationName for RunnerUpdateState {
    fn name(&self) -> &str {
        "runner_update_state_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for RunnerUpdateState {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        execute_schema(
            m,
            SchemaStatement::Column(
                "hosts",
                "agent_update",
                Table::alter()
                    .table(Alias::new("hosts"))
                    .add_column(long_text("agent_update", m.get_database_backend()).null())
                    .to_owned(),
            ),
        )
        .await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct NamedCodeConnections;
impl MigrationName for NamedCodeConnections {
    fn name(&self) -> &str {
        "named_code_connections_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for NamedCodeConnections {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for column in ["name", "description"] {
            execute_schema(
                m,
                SchemaStatement::Column(
                    "repository_connections",
                    column,
                    Table::alter()
                        .table(Alias::new("repository_connections"))
                        .add_column(
                            ColumnDef::new(Alias::new(column))
                                .string_len(255)
                                .not_null()
                                .default(""),
                        )
                        .to_owned(),
                ),
            )
            .await?;
        }
        match m.get_database_backend() {
            DbBackend::Sqlite => {
                // SQLite's table UNIQUE constraint is an implicit autoindex, which cannot
                // be dropped. Rebuild atomically, preserving every identity and secret.
                use sea_orm::{ConnectionTrait, TransactionTrait};
                let tx = m.get_connection().begin().await?;
                tx.execute_unprepared("CREATE TABLE repository_connections_named (
                    id INTEGER PRIMARY KEY, workspace_id INTEGER NOT NULL, user_id INTEGER NOT NULL,
                    version INTEGER NOT NULL, expires_at INTEGER NOT NULL, created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL, provider VARCHAR(20) NOT NULL, credential_id INTEGER NOT NULL,
                    account TEXT NOT NULL, refresh_secret TEXT NOT NULL, state TEXT NOT NULL,
                    last_error TEXT NOT NULL, client_id TEXT NOT NULL, config_fingerprint VARCHAR(64) NOT NULL DEFAULT '',
                    token_config TEXT, name VARCHAR(255) NOT NULL DEFAULT '', description VARCHAR(255) NOT NULL DEFAULT '')").await?;
                tx.execute_unprepared("INSERT INTO repository_connections_named
                    (id,workspace_id,user_id,version,expires_at,created_at,updated_at,provider,credential_id,
                     account,refresh_secret,state,last_error,client_id,config_fingerprint,token_config,name,description)
                    SELECT id,workspace_id,user_id,version,expires_at,created_at,updated_at,provider,credential_id,
                     account,refresh_secret,state,last_error,client_id,config_fingerprint,token_config,name,description
                    FROM repository_connections").await?;
                tx.execute_unprepared("DROP TABLE repository_connections")
                    .await?;
                tx.execute_unprepared(
                    "ALTER TABLE repository_connections_named RENAME TO repository_connections",
                )
                .await?;
                tx.commit().await?;
            }
            DbBackend::Postgres => {
                use sea_orm::ConnectionTrait;
                m.get_connection().execute_unprepared("ALTER TABLE repository_connections DROP CONSTRAINT IF EXISTS repository_connection_identity").await?;
            }
            DbBackend::MySql => {
                if m.has_index("repository_connections", "repository_connection_identity")
                    .await?
                {
                    m.drop_index(
                        Index::drop()
                            .table(Alias::new("repository_connections"))
                            .name("repository_connection_identity")
                            .to_owned(),
                    )
                    .await?;
                }
            }
            _ => return Err(DbErr::Custom("Unsupported database backend".into())),
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

struct CredentialScopes;
impl MigrationName for CredentialScopes {
    fn name(&self) -> &str {
        "credential_scopes_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for CredentialScopes {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        use sea_orm::ConnectionTrait;
        execute_schema(
            m,
            SchemaStatement::Column(
                "credentials",
                "scope",
                Table::alter()
                    .table(Alias::new("credentials"))
                    .add_column(
                        ColumnDef::new(Alias::new("scope"))
                            .string_len(20)
                            .not_null()
                            .default("build"),
                    )
                    .to_owned(),
            ),
        )
        .await?;
        // Preserve credential IDs, ciphertext/AAD and all historical references.
        m.get_connection().execute_unprepared(
            "UPDATE credentials SET scope = 'deploy' WHERE EXISTS (SELECT 1 FROM repositories r INNER JOIN marketplace_sources m ON m.repository_id = r.id AND m.workspace_id = r.workspace_id WHERE r.workspace_id = credentials.workspace_id AND r.credential_id = credentials.id)"
        ).await?;
        m.get_connection().execute_unprepared(
            "UPDATE credentials SET scope = 'host' WHERE EXISTS (SELECT 1 FROM hosts WHERE hosts.workspace_id = credentials.workspace_id AND hosts.credential_id = credentials.id)"
        ).await?;
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Business data is not automatically removed".into(),
        ))
    }
}

struct RepositoryPolling;
impl MigrationName for RepositoryPolling {
    fn name(&self) -> &str {
        "repository_polling_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for RepositoryPolling {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        let mut table = create_table("repository_poll_states", backend);
        for name in [
            "id",
            "workspace_id",
            "config_version",
            "last_polled_at",
            "next_poll_at",
        ] {
            let mut column = ColumnDef::new(Alias::new(name));
            column.big_integer().not_null();
            if name == "id" {
                column.primary_key();
            }
            table.col(&mut column);
        }
        table.col(long_text("snapshot", backend).not_null());
        table.col(ColumnDef::new(Alias::new("last_error")).text().not_null());
        m.create_table(table.to_owned()).await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Business data is not automatically removed".into(),
        ))
    }
}

struct RepositoryMonitoring;
impl MigrationName for RepositoryMonitoring {
    fn name(&self) -> &str {
        "repository_monitoring_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for RepositoryMonitoring {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        let mut monitors = create_table("repository_monitors", backend);
        for name in ["id", "workspace_id", "version", "created_at", "updated_at"] {
            let mut column = ColumnDef::new(Alias::new(name));
            column.big_integer().not_null();
            if name == "id" {
                column.primary_key();
            }
            monitors.col(&mut column);
        }
        monitors.col(long_text("config", backend).not_null());
        m.create_table(monitors.to_owned()).await?;
        let mut events = create_table("repository_events", backend);
        for name in ["id", "workspace_id", "connection_id", "created_at"] {
            let mut column = ColumnDef::new(Alias::new(name));
            column.big_integer().not_null();
            if name == "id" {
                column.primary_key();
            }
            events.col(&mut column);
        }
        for name in [
            "delivery_key",
            "event",
            "repository",
            "reference",
            "actor",
            "action",
            "delivery",
        ] {
            events.col(ColumnDef::new(Alias::new(name)).string_len(512).not_null());
        }
        m.create_table(events.to_owned()).await?;
        execute_schema(
            m,
            SchemaStatement::Index(
                "repository_events",
                "repository_event_delivery",
                Index::create()
                    .name("repository_event_delivery")
                    .table(Alias::new("repository_events"))
                    .col(Alias::new("connection_id"))
                    .col(Alias::new("delivery_key"))
                    .unique()
                    .to_owned(),
            ),
        )
        .await?;
        execute_schema(
            m,
            SchemaStatement::Index(
                "repository_events",
                "repository_event_history",
                Index::create()
                    .name("repository_event_history")
                    .table(Alias::new("repository_events"))
                    .col(Alias::new("workspace_id"))
                    .col(Alias::new("connection_id"))
                    .col(Alias::new("created_at"))
                    .to_owned(),
            ),
        )
        .await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Business data is not automatically removed".into(),
        ))
    }
}

#[cfg(test)]
mod credential_scope_tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database, EntityTrait, QueryOrder};

    #[tokio::test]
    async fn upgrade_preserves_legacy_credentials_and_host_bindings() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::connect(format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("upgrade.sqlite").display()
        ))
        .await
        .unwrap();
        let before_scopes = Migrator::migrations()
            .iter()
            .position(|migration| migration.name() == CredentialScopes.name())
            .unwrap();
        Migrator::up(&db, Some(before_scopes as u32)).await.unwrap();
        db.execute_unprepared("INSERT INTO credentials (id,workspace_id,name,kind,username,description,secret,created_at,updated_at,version) VALUES (1,10,'SSH','password','root','','sealed-host',1,2,3),(2,10,'Git','token','','','sealed-git',1,2,4),(3,20,'Other tenant','token','','','sealed-other',1,2,5)").await.unwrap();
        db.execute_unprepared("INSERT INTO hosts (id,workspace_id,name,address,connection,credential_id,created_at,updated_at) VALUES (4,10,'Node','127.0.0.1','ssh',1,1,2),(5,10,'Invalid tenant binding','127.0.0.2','ssh',3,1,2)").await.unwrap();
        db.execute_unprepared("INSERT INTO credentials (id,workspace_id,name,kind,username,description,secret,created_at,updated_at,version) VALUES (6,10,'Application','token','','','sealed-app',1,2,6)").await.unwrap();
        db.execute_unprepared("INSERT INTO repositories (id,workspace_id,name,provider,url,default_branch,credential_id,description,created_at,updated_at,version) VALUES (7,10,'Application repository','git','https://git.example.invalid/app.git','main',6,'',1,2,1)").await.unwrap();
        db.execute_unprepared("INSERT INTO marketplace_sources (id,workspace_id,name,repository_id,config,state,last_error,synced_at,run_as,version,created_at,updated_at) VALUES (8,10,'Application source',7,'{}','idle','',0,0,1,1,2)").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        use crate::entities::{credentials, hosts};
        let rows = credentials::Entity::find()
            .order_by_asc(credentials::Column::Id)
            .all(&db)
            .await
            .unwrap();
        assert_eq!(
            rows.iter().map(|v| v.scope.as_str()).collect::<Vec<_>>(),
            ["host", "build", "build", "deploy"]
        );
        assert_eq!(
            rows.iter()
                .map(|v| (v.id, v.secret.as_str(), v.version))
                .collect::<Vec<_>>(),
            [
                (1, "sealed-host", 3),
                (2, "sealed-git", 4),
                (3, "sealed-other", 5),
                (6, "sealed-app", 6)
            ]
        );
        assert_eq!(
            hosts::Entity::find_by_id(4)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .credential_id,
            1
        );
    }
}
