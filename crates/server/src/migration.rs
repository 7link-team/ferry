//! Schema migrations built with SeaQuery, applied in order and never edited once
//! shipped. The full target schema is in docs/ferry-运维平台产品与技术方案.md;
//! `Initial` carries only the identity and tenancy bedrock every module needs.

use sea_orm::DbBackend;
use sea_orm_migration::prelude::*;
mod access_addresses;
mod preview_ingress;
mod account_otp;
mod account_preferences;
mod identity_avatars;
mod agent_models;
mod ai_agents;
mod ai_models;
mod chat;
mod chat_interactions;
mod chat_messages;
mod conversations;
mod holes;
mod holes_sources;
mod login_guard;
mod notification_center;
mod monitoring;
mod platform_monitor;
mod platform_storage;
mod workflow_node_logs;

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

#[cfg(test)]
pub(crate) mod workflow_tests;

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
            Box::new(RepositoryEventDetail),
            Box::new(PipelineRunNames),
            Box::new(PipelineRunInputs),
            Box::new(EnvironmentApproval),
            Box::new(LegacyPipelineRemoval),
            Box::new(TypedMaintenanceValues),
            Box::new(RepositoryAnalysis),
            Box::new(RepositoryAnalysisIdentity),
            Box::new(CodeInsights),
            Box::new(CodeInsightSchedules),
            Box::new(RunnerAutoUpdates),
            Box::new(RepositoryEventDeliveryMessages),
            Box::new(HostExecutionCapacity),
            Box::new(DeploymentCategory),
            Box::new(crate::repositories::reports::migration::InsightReports),
            Box::new(crate::pipelines::reviews::migration::Migration),
            Box::new(ExecutionLeases),
            Box::new(DefinitionRevisions),
            Box::new(WorkflowStorage),
            Box::new(DefinitionAcl),
            Box::new(ContentAssets),
            Box::new(DefinitionPublications),
            Box::new(PipelineOwnership),
            Box::new(AiProviders),
            Box::new(IntegrationCalls),
            Box::new(platform_storage::Migration),
            Box::new(UserLoginMetadata),
            Box::new(PlatformFooter),
            Box::new(PlatformScripts),
            Box::new(PlatformMail),
            Box::new(FixedStateDictionaryRemoval),
            Box::new(PipelineCreatorOwnership),
            Box::new(platform_monitor::Migration),
            Box::new(platform_monitor::LimitsMigration),
            Box::new(login_guard::Migration),
            Box::new(workflow_node_logs::Migration),
            Box::new(ai_agents::Migration),
            Box::new(conversations::Migration),
            Box::new(chat::Migration),
            Box::new(ai_models::Migration),
            Box::new(chat_messages::Migration),
            Box::new(agent_models::Migration),
            Box::new(chat_interactions::Migration),
            Box::new(holes::Migration),
            Box::new(holes_sources::Migration),
            Box::new(account_preferences::Migration),
            Box::new(access_addresses::Migration),
            Box::new(PlatformSite),
            Box::new(notification_center::Migration),
            Box::new(account_otp::Migration),
            Box::new(identity_avatars::Migration),
            Box::new(monitoring::Migration),
            Box::new(preview_ingress::Migration),
        ]
    }
}

pub(crate) struct PlatformSite;
impl MigrationName for PlatformSite {
    fn name(&self) -> &str {
        "platform_site_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for PlatformSite {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m.has_column("platform_state", "site_config").await? {
            m.alter_table(
                Table::alter()
                    .table(Alias::new("platform_state"))
                    .add_column(long_text("site_config", m.get_database_backend()).null())
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Platform site configuration must be retained".into(),
        ))
    }
}

struct FixedStateDictionaryRemoval;
impl MigrationName for FixedStateDictionaryRemoval {
    fn name(&self) -> &str {
        "fixed_state_dictionary_removal_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for FixedStateDictionaryRemoval {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        use crate::entities::{dictionaries, dictionary_items};
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect, QueryTrait};
        let ids = dictionaries::Entity::find()
            .select_only()
            .column(dictionaries::Column::Id)
            .filter(
                dictionaries::Column::Key.is_in(
                    crate::maintenance::RETIRED_STATE_DICTIONARIES
                        .iter()
                        .copied(),
                ),
            );
        dictionary_items::Entity::delete_many()
            .filter(dictionary_items::Column::DictionaryId.in_subquery(ids.into_query()))
            .exec(m.get_connection())
            .await?;
        dictionaries::Entity::delete_many()
            .filter(
                dictionaries::Column::Key.is_in(
                    crate::maintenance::RETIRED_STATE_DICTIONARIES
                        .iter()
                        .copied(),
                ),
            )
            .exec(m.get_connection())
            .await?;
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "fixed workflow states are no longer dictionaries".into(),
        ))
    }
}

struct PipelineCreatorOwnership;
impl MigrationName for PipelineCreatorOwnership {
    fn name(&self) -> &str {
        "pipeline_creator_ownership_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for PipelineCreatorOwnership {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        // Only an unambiguous creation audit is evidence of ownership. Updates,
        // executions, workspace ownership and unrelated tenants never qualify.
        m.get_connection()
            .execute_unprepared(
                "UPDATE pipelines SET owner_id = (SELECT MIN(a.actor_id) FROM activity a
             WHERE a.workspace_id = pipelines.workspace_id AND a.resource = 'pipelines'
             AND a.resource_id = pipelines.id AND a.action = 'create' AND a.actor_id > 0)
             WHERE owner_id IS NULL AND
             (SELECT COUNT(DISTINCT a.actor_id) FROM activity a
             WHERE a.workspace_id = pipelines.workspace_id AND a.resource = 'pipelines'
             AND a.resource_id = pipelines.id AND a.action = 'create' AND a.actor_id > 0) = 1
             AND EXISTS (SELECT 1 FROM users u WHERE u.id =
             (SELECT MIN(a.actor_id) FROM activity a WHERE a.workspace_id = pipelines.workspace_id
             AND a.resource = 'pipelines' AND a.resource_id = pipelines.id
             AND a.action = 'create' AND a.actor_id > 0))",
            )
            .await?;
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "verified pipeline ownership must be retained".into(),
        ))
    }
}

struct PlatformFooter;
#[cfg(test)]
mod compatibility_cleanup_tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database};

    #[tokio::test]
    async fn removes_fixed_state_rows_and_items_only_and_is_repeatable() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("CREATE TABLE dictionaries (id BIGINT PRIMARY KEY, workspace_id BIGINT, key TEXT); CREATE TABLE dictionary_items (id BIGINT PRIMARY KEY, dictionary_id BIGINT, workspace_id BIGINT);
            INSERT INTO dictionaries VALUES (1,11,'host-state'),(2,12,'pipeline-state'),(3,11,'environment'),(4,12,'custom');
            INSERT INTO dictionary_items VALUES (1,1,11),(2,2,12),(3,3,11),(4,4,12)").await.unwrap();
        let manager = SchemaManager::new(&db);
        for _ in 0..2 {
            FixedStateDictionaryRemoval.up(&manager).await.unwrap();
        }
        for table in ["dictionaries", "dictionary_items"] {
            let rows = db
                .query_all_raw(sea_orm::Statement::from_string(
                    DbBackend::Sqlite,
                    format!("SELECT id FROM {table} ORDER BY id"),
                ))
                .await
                .unwrap();
            let ids: Vec<i64> = rows
                .into_iter()
                .map(|row| row.try_get("", "id").unwrap())
                .collect();
            assert_eq!(ids, vec![3, 4]);
        }
    }

    #[tokio::test]
    async fn recovers_only_unique_creation_audits_without_reassigning_existing_owners() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("CREATE TABLE pipelines (id BIGINT PRIMARY KEY, workspace_id BIGINT, owner_id BIGINT); CREATE TABLE activity (workspace_id BIGINT, resource TEXT, resource_id BIGINT, action TEXT, actor_id BIGINT); CREATE TABLE users (id BIGINT PRIMARY KEY);
            INSERT INTO users VALUES (3),(4);
            INSERT INTO pipelines VALUES (1,11,NULL),(2,11,NULL),(3,11,NULL),(4,11,NULL),(5,11,4),(6,11,NULL),(7,11,NULL);
            INSERT INTO activity VALUES (11,'pipelines',1,'create',3),(11,'pipelines',1,'create',3),
            (11,'pipelines',2,'update',3),(12,'pipelines',3,'create',3),
            (11,'pipelines',4,'create',3),(11,'pipelines',4,'create',4),
            (11,'pipelines',5,'create',3),(11,'pipelines',6,'create',99),(11,'workflows',7,'create',3)").await.unwrap();
        let manager = SchemaManager::new(&db);
        for _ in 0..2 {
            PipelineCreatorOwnership.up(&manager).await.unwrap();
        }
        let rows = db
            .query_all_raw(sea_orm::Statement::from_string(
                DbBackend::Sqlite,
                "SELECT owner_id FROM pipelines ORDER BY id",
            ))
            .await
            .unwrap();
        let owners: Vec<Option<i64>> = rows
            .into_iter()
            .map(|row| row.try_get("", "owner_id").unwrap())
            .collect();
        assert_eq!(owners, vec![Some(3), None, None, None, Some(4), None, None]);
    }
}

struct PlatformMail;
impl MigrationName for PlatformMail {
    fn name(&self) -> &str {
        "platform_mail_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for PlatformMail {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m.has_column("platform_state", "mail_config").await? {
            m.alter_table(
                Table::alter()
                    .table(Alias::new("platform_state"))
                    .add_column(long_text("mail_config", m.get_database_backend()).null())
                    .to_owned(),
            )
            .await?;
        }
        m.create_table(
            Table::create()
                .table(Alias::new("email_codes"))
                .if_not_exists()
                .col(
                    ColumnDef::new(Alias::new("id"))
                        .string_len(320)
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(Alias::new("code_hash"))
                        .string_len(255)
                        .not_null(),
                )
                .col(ColumnDef::new(Alias::new("count")).big_integer().not_null())
                .col(
                    ColumnDef::new(Alias::new("expires_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("updated_at"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("config_tag"))
                        .string_len(64)
                        .not_null(),
                )
                .to_owned(),
        )
        .await?;
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Mail verification state must be retained".into(),
        ))
    }
}
struct PlatformScripts;
impl MigrationName for PlatformScripts {
    fn name(&self) -> &str {
        "platform_scripts_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for PlatformScripts {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m.has_column("platform_state", "page_scripts").await? {
            m.alter_table(
                Table::alter()
                    .table(Alias::new("platform_state"))
                    .add_column(long_text("page_scripts", m.get_database_backend()).null())
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Platform script configuration must be retained".into(),
        ))
    }
}
impl MigrationName for PlatformFooter {
    fn name(&self) -> &str {
        "platform_footer_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for PlatformFooter {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m.has_column("platform_state", "footer_html").await? {
            m.alter_table(
                Table::alter()
                    .table(Alias::new("platform_state"))
                    .add_column(long_text("footer_html", m.get_database_backend()).null())
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Platform footer configuration must be retained".into(),
        ))
    }
}

struct UserLoginMetadata;
impl MigrationName for UserLoginMetadata {
    fn name(&self) -> &str {
        "user_login_metadata_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for UserLoginMetadata {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for mut column in [
            ColumnDef::new(Alias::new("last_login_at"))
                .big_integer()
                .null()
                .to_owned(),
            ColumnDef::new(Alias::new("last_login_ip"))
                .string_len(45)
                .null()
                .to_owned(),
        ] {
            let name = column.get_column_name();
            if !m.has_column("users", &name).await? {
                m.alter_table(
                    Table::alter()
                        .table(Alias::new("users"))
                        .add_column(&mut column)
                        .to_owned(),
                )
                .await?;
            }
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("User login metadata must be retained".into()))
    }
}

pub(crate) struct IntegrationCalls;
impl MigrationName for IntegrationCalls {
    fn name(&self) -> &str {
        "workflow_integration_calls_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for IntegrationCalls {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        let mut table = create_table("workflow_integration_calls", backend);
        table.col(
            ColumnDef::new(Alias::new("id"))
                .big_integer()
                .not_null()
                .primary_key(),
        );
        for field in [
            "workspace_id",
            "execution_id",
            "node_run_id",
            "item_index",
            "credential_id",
            "created_at",
            "created_by",
            "updated_at",
            "updated_by",
            "version",
        ] {
            table.col(ColumnDef::new(Alias::new(field)).big_integer().not_null());
        }
        for field in ["state", "request_hash"] {
            table.col(ColumnDef::new(Alias::new(field)).string_len(64).not_null());
        }
        table.col(long_text("result", backend).null());
        execute_schema(m, SchemaStatement::Table(table.to_owned())).await?;
        let name = "workflow_integration_call_item";
        let mut index = Index::create();
        index
            .name(name)
            .table(Alias::new("workflow_integration_calls"))
            .unique();
        for field in ["workspace_id", "execution_id", "node_run_id", "item_index"] {
            index.col(Alias::new(field));
        }
        execute_schema(
            m,
            SchemaStatement::Index("workflow_integration_calls", name, index.to_owned()),
        )
        .await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Integration request outcomes must be retained".into(),
        ))
    }
}

struct AiProviders;
impl MigrationName for AiProviders {
    fn name(&self) -> &str {
        "workflow_ai_providers_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for AiProviders {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        for (name, strings, numbers, texts) in [
            (
                "ai_providers",
                vec!["name", "base_url", "protocol", "currency", "timezone"],
                vec![
                    "workspace_id",
                    "credential_id",
                    "monthly_limit_micros",
                    "qps",
                    "concurrency",
                    "created_at",
                    "created_by",
                    "updated_at",
                    "updated_by",
                    "version",
                ],
                vec!["prices"],
            ),
            (
                "ai_calls",
                vec!["operation_key", "currency", "period_key", "state"],
                vec![
                    "workspace_id",
                    "provider_id",
                    "credential_id",
                    "execution_id",
                    "node_run_id",
                    "reserved_micros",
                    "actual_micros",
                    "created_at",
                    "created_by",
                    "updated_at",
                    "updated_by",
                    "version",
                ],
                vec!["price_snapshot"],
            ),
        ] {
            let mut table = create_table(name, backend);
            table.col(
                ColumnDef::new(Alias::new("id"))
                    .big_integer()
                    .not_null()
                    .primary_key(),
            );
            for field in strings {
                table.col(
                    ColumnDef::new(Alias::new(field))
                        .string_len(match field {
                            "currency" => 3,
                            "period_key" => 7,
                            "protocol" | "state" => 32,
                            "operation_key" => 128,
                            _ => 512,
                        })
                        .not_null(),
                );
            }
            for field in numbers {
                table.col(ColumnDef::new(Alias::new(field)).big_integer().not_null());
            }
            for field in texts {
                table.col(long_text(field, backend).not_null());
            }
            if name == "ai_calls" {
                table.col(long_text("result", backend).null());
            }
            execute_schema(m, SchemaStatement::Table(table.to_owned())).await?;
        }
        for (table, name, columns, unique) in [
            (
                "ai_providers",
                "ai_provider_credential",
                vec!["workspace_id", "credential_id"],
                true,
            ),
            (
                "ai_calls",
                "ai_call_operation",
                vec!["workspace_id", "operation_key"],
                true,
            ),
            (
                "ai_calls",
                "ai_call_budget",
                vec!["workspace_id", "credential_id", "currency", "period_key"],
                false,
            ),
            (
                "ai_calls",
                "ai_call_execution",
                vec!["workspace_id", "execution_id"],
                false,
            ),
        ] {
            let mut index = Index::create();
            index.name(name).table(Alias::new(table));
            if unique {
                index.unique();
            }
            for column in columns {
                index.col(Alias::new(column));
            }
            execute_schema(m, SchemaStatement::Index(table, name, index.to_owned())).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "AI costs and reservations must be retained".into(),
        ))
    }
}

struct PipelineOwnership;
impl MigrationName for PipelineOwnership {
    fn name(&self) -> &str {
        "pipeline_ownership_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for PipelineOwnership {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        // Historical editors are not evidence of ownership. Unknown stays NULL.
        execute_schema(m, pipeline_ownership()).await
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "pipeline ownership and grants must be retained".into(),
        ))
    }
}

fn pipeline_ownership() -> SchemaStatement {
    SchemaStatement::Column(
        "pipelines",
        "owner_id",
        Table::alter()
            .table(Alias::new("pipelines"))
            .add_column(ColumnDef::new(Alias::new("owner_id")).big_integer().null())
            .to_owned(),
    )
}

struct LegacyPipelineRemoval;
impl MigrationName for LegacyPipelineRemoval {
    fn name(&self) -> &str {
        "legacy_pipeline_removal_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for LegacyPipelineRemoval {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        use sea_orm::ConnectionTrait;
        let backend = m.get_database_backend();
        let conn = m.get_connection();
        for (table, field) in [
            ("pipelines", "definition"),
            ("pipeline_templates", "content"),
        ] {
            execute_schema(
                m,
                SchemaStatement::Column(
                    table,
                    "legacy_definition_backup",
                    Table::alter()
                        .table(Alias::new(table))
                        .add_column(long_text("legacy_definition_backup", backend))
                        .to_owned(),
                ),
            )
            .await?;
            let mut select = Query::select();
            select
                .from(Alias::new(table))
                .columns([Alias::new("id"), Alias::new(field)]);
            if table == "pipelines" {
                select.columns([Alias::new("target_kind"), Alias::new("target_ref")]);
            }
            for row in conn.query_all_raw(backend.build(&select)).await? {
                let id: i64 = row.try_get("", "id")?;
                let original: String = row.try_get("", field)?;
                if crate::pipelines::actions::document(&original).is_some() {
                    continue;
                }
                let converted = if table == "pipelines" {
                    let kind: String = row.try_get("", "target_kind")?;
                    let target: i64 = row.try_get("", "target_ref")?;
                    crate::pipelines::convert_legacy(&original, &kind, &target.to_string())
                } else {
                    crate::templates::convert_legacy_content(&original)
                };
                // One unconvertible row must not keep the server from starting:
                // keep it unchanged (it fails clearly when used) and say so.
                let converted = match converted {
                    Ok(converted) => converted,
                    Err(error) => {
                        tracing::warn!(table, id, error = %error.1, "旧格式流水线定义无法自动转换，已保持原样");
                        continue;
                    }
                };
                // Definition and backup change atomically. Reapplying after a DDL
                // interruption sees Actions and leaves the original backup intact.
                let update = Query::update()
                    .table(Alias::new(table))
                    .values([
                        (Alias::new(field), converted.into()),
                        (Alias::new("legacy_definition_backup"), original.into()),
                    ])
                    .and_where(Expr::col(Alias::new("id")).eq(id))
                    .to_owned();
                conn.execute_raw(backend.build(&update)).await?;
            }
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Legacy backups must be retained".into()))
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
            repository_event_detail,
            repository_analysis_identity,
            code_insights,
            code_insight_schedules,
            execution_leases,
            definition_revisions,
            workflow_storage,
            definition_acl,
        ]
        .into_iter()
        .flat_map(|schema| schema(DbBackend::MySql))
        .chain(std::iter::once(host_jump()))
        .chain(std::iter::once(pipeline_ownership()))
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
                "id workspace_id owner_id repository_id target_ref group_id application_id",
            ),
            ("pipeline_contexts", "id workspace_id target_ref"),
            ("pipeline_links", "pipeline_id reference_id"),
            (
                "pipeline_runs",
                "id workspace_id pipeline_id host_id triggered_by credential_id environment_id revision_id",
            ),
            (
                "definition_revisions",
                "id workspace_id subject_id created_by",
            ),
            (
                "workflows",
                "id workspace_id group_id draft_revision_id published_revision_id owner_id trigger_run_as archived_by created_by updated_by",
            ),
            (
                "workflow_executions",
                "id workspace_id workflow_id revision_id triggered_by parent_execution_id retry_of_id root_execution_id created_by started_by finished_by response_deadline_by response_delivered_by",
            ),
            (
                "workflow_node_runs",
                "id execution_id output_asset cache_hit_from started_by finished_by",
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
                    || (column.ends_with("_id") && column != "node_id")
                    || column.ends_with("_by")
                    || [
                        "target_ref",
                        "scope_ref",
                        "used_by",
                        "created_by",
                        "triggered_by",
                        "current_revision",
                        "source_revision",
                        "trigger_run_as",
                        "output_asset",
                        "cache_hit_from",
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
        assert_eq!(count, 107);
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

struct EnvironmentApproval;
impl MigrationName for EnvironmentApproval {
    fn name(&self) -> &str {
        "pipeline_environment_approval_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for EnvironmentApproval {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        execute_schema(
            m,
            SchemaStatement::Column(
                "pipeline_contexts",
                "requires_approval",
                Table::alter()
                    .table(Alias::new("pipeline_contexts"))
                    .add_column(
                        ColumnDef::new(Alias::new("requires_approval"))
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
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

struct PipelineRunInputs;
impl MigrationName for PipelineRunInputs {
    fn name(&self) -> &str {
        "pipeline_run_inputs_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for PipelineRunInputs {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        // Nullable: earlier runs had no inputs, and MySQL text cannot carry a default.
        execute_schema(
            m,
            SchemaStatement::Column(
                "pipeline_runs",
                "inputs",
                Table::alter()
                    .table(Alias::new("pipeline_runs"))
                    .add_column(long_text("inputs", m.get_database_backend()).null())
                    .to_owned(),
            ),
        )
        .await?;
        execute_schema(
            m,
            SchemaStatement::Column(
                "pipeline_runs",
                "concurrency_group",
                Table::alter()
                    .table(Alias::new("pipeline_runs"))
                    .add_column(
                        ColumnDef::new(Alias::new("concurrency_group"))
                            .string_len(255)
                            .not_null()
                            .default(""),
                    )
                    .to_owned(),
            ),
        )
        .await?;
        execute_schema(
            m,
            SchemaStatement::Column(
                "pipeline_run_jobs",
                "concurrency_group",
                Table::alter()
                    .table(Alias::new("pipeline_run_jobs"))
                    .add_column(
                        ColumnDef::new(Alias::new("concurrency_group"))
                            .string_len(255)
                            .not_null()
                            .default(""),
                    )
                    .to_owned(),
            ),
        )
        .await?;
        // 0 means the history policy decides; otherwise upload-artifact's
        // retention-days expiry in milliseconds.
        execute_schema(
            m,
            SchemaStatement::Column(
                "pipeline_artifacts",
                "expires_at",
                Table::alter()
                    .table(Alias::new("pipeline_artifacts"))
                    .add_column(
                        ColumnDef::new(Alias::new("expires_at"))
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
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

struct PipelineRunNames;
impl MigrationName for PipelineRunNames {
    fn name(&self) -> &str {
        "pipeline_run_names_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for PipelineRunNames {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for statement in pipeline_run_names() {
            execute_schema(m, statement).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Business data is not automatically removed".into(),
        ))
    }
}

/// Names as they were when the run started, like `environment_name`. Runs
/// recorded before these columns existed keep "" and are named live.
fn pipeline_run_names() -> Vec<SchemaStatement> {
    ["pipeline_name", "host_name", "triggered_by_name"]
        .into_iter()
        .map(|column| {
            SchemaStatement::Column(
                "pipeline_runs",
                column,
                Table::alter()
                    .table(Alias::new("pipeline_runs"))
                    .add_column(
                        ColumnDef::new(Alias::new(column))
                            .string_len(255)
                            .not_null()
                            .default(""),
                    )
                    .to_owned(),
            )
        })
        .collect()
}

struct RepositoryEventDetail;
impl MigrationName for RepositoryEventDetail {
    fn name(&self) -> &str {
        "repository_event_detail_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for RepositoryEventDetail {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for statement in repository_event_detail(m.get_database_backend()) {
            execute_schema(m, statement).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Business data is not automatically removed".into(),
        ))
    }
}

/// Structured event detail (commits, merge request, operator). Nullable: rows
/// recorded before it existed have none, and MySQL text cannot carry a default.
fn repository_event_detail(backend: DbBackend) -> Vec<SchemaStatement> {
    vec![SchemaStatement::Column(
        "repository_events",
        "detail",
        Table::alter()
            .table(Alias::new("repository_events"))
            .add_column(long_text("detail", backend).null())
            .to_owned(),
    )]
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
    async fn contribution_upgrade_preserves_existing_monitor_and_artifact_state() {
        use crate::entities::{pipeline_artifacts, repository_monitors, repository_poll_states};
        use sea_orm::IntoActiveModel;

        let db = Database::connect("sqlite::memory:").await.unwrap();
        let before = Migrator::migrations()
            .iter()
            .position(|migration| migration.name() == RepositoryAnalysis.name())
            .unwrap();
        Migrator::up(&db, Some(before as u32)).await.unwrap();
        let manager = SchemaManager::new(&db);
        assert!(
            manager
                .has_column("pipelines", "legacy_definition_backup")
                .await
                .unwrap()
        );
        assert!(
            !manager
                .has_table("repository_analysis_configs")
                .await
                .unwrap()
        );
        let monitor = repository_monitors::Model {
            id: 91,
            workspace_id: 11,
            config: "opaque-existing-config".into(),
            version: 7,
            created_at: 10,
            updated_at: 20,
        };
        let baseline = repository_poll_states::Model {
            id: 91,
            workspace_id: 11,
            config_version: 7,
            snapshot: "opaque-existing-baseline".into(),
            last_polled_at: 30,
            next_poll_at: 40,
            last_error: "previous status".into(),
        };
        let artifact = pipeline_artifacts::Model {
            id: 92,
            workspace_id: 11,
            pipeline_id: 93,
            run_id: 94,
            path: "artifacts/existing.zip".into(),
            sha256: "a".repeat(64),
            size: 123,
            mode: 0o644,
            created_at: 10,
            expires_at: 999,
        };
        repository_monitors::Entity::insert(monitor.clone().into_active_model())
            .exec(&db)
            .await
            .unwrap();
        repository_poll_states::Entity::insert(baseline.clone().into_active_model())
            .exec(&db)
            .await
            .unwrap();
        pipeline_artifacts::Entity::insert(artifact.clone().into_active_model())
            .exec(&db)
            .await
            .unwrap();
        Migrator::up(&db, None).await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        assert!(
            manager
                .has_table("repository_analysis_configs")
                .await
                .unwrap()
        );
        assert!(
            manager
                .has_table("repository_analysis_commit_authors")
                .await
                .unwrap()
        );
        assert_eq!(
            repository_monitors::Entity::find_by_id(91)
                .one(&db)
                .await
                .unwrap(),
            Some(monitor)
        );
        assert_eq!(
            repository_poll_states::Entity::find_by_id(91)
                .one(&db)
                .await
                .unwrap(),
            Some(baseline)
        );
        assert_eq!(
            pipeline_artifacts::Entity::find_by_id(92)
                .one(&db)
                .await
                .unwrap(),
            Some(artifact)
        );
    }

    #[tokio::test]
    async fn legacy_pipeline_upgrade_keeps_backups_and_history_and_is_idempotent() {
        use sea_orm::{ActiveModelTrait, Set};
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let before = Migrator::migrations()
            .iter()
            .position(|migration| migration.name() == LegacyPipelineRemoval.name())
            .unwrap();
        Migrator::up(&db, Some(before as u32)).await.unwrap();
        let original = serde_json::json!({"workdir":"/tmp/build", "steps":[
            {"name":"source","checkout":true}, {"name":"build","run":"echo {{run}}"}]})
        .to_string();
        crate::entities::pipelines::ActiveModel {
            id: Set(1),
            workspace_id: Set(10),
            owner_id: sea_orm::ActiveValue::NotSet,
            name: Set("legacy".into()),
            definition: Set(original.clone()),
            repository_id: Set(0),
            target_kind: Set("host".into()),
            target_ref: Set(42),
            description: Set(String::new()),
            created_at: Set(1),
            updated_at: Set(2),
            version: Set(3),
            group_id: Set(0),
            application_id: Set(0),
        }
        .insert(&db)
        .await
        .unwrap();
        crate::entities::pipeline_templates::ActiveModel {
            id: Set(2),
            workspace_id: Set(10),
            name: Set("legacy template".into()),
            content: Set(original.clone()),
            parameters: Set("[]".into()),
            description: Set(String::new()),
            created_at: Set(1),
            updated_at: Set(2),
            version: Set(3),
        }
        .insert(&db)
        .await
        .unwrap();
        Migrator::up(&db, None).await.unwrap();
        // Exercise the migration itself twice, including a retry after columns exist.
        LegacyPipelineRemoval
            .up(&SchemaManager::new(&db))
            .await
            .unwrap();
        for (table, field) in [
            ("pipelines", "definition"),
            ("pipeline_templates", "content"),
        ] {
            let query = Query::select()
                .from(Alias::new(table))
                .columns([
                    Alias::new(field),
                    Alias::new("legacy_definition_backup"),
                    Alias::new("version"),
                ])
                .to_owned();
            let row = db
                .query_one_raw(db.get_database_backend().build(&query))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                row.try_get::<String>("", "legacy_definition_backup")
                    .unwrap(),
                original
            );
            assert_eq!(row.try_get::<i64>("", "version").unwrap(), 3);
            let document =
                crate::pipelines::actions::document(&row.try_get::<String>("", field).unwrap())
                    .unwrap();
            assert!(document.workflow.contains("github.run_number"));
            if table == "pipelines" {
                assert_eq!(document.executor_bindings[0].target_ref, "42");
            }
        }
    }

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

struct RepositoryAnalysis;
impl MigrationName for RepositoryAnalysis {
    fn name(&self) -> &str {
        "repository_analysis_v1"
    }
}

fn repository_analysis_identity(backend: DbBackend) -> Vec<SchemaStatement> {
    let mut statements = Vec::new();
    let mut people = create_table("repository_analysis_people", backend);
    for column in ["id", "workspace_id", "user_id", "version"] {
        let mut definition = ColumnDef::new(Alias::new(column));
        definition.big_integer().not_null();
        if column == "id" {
            definition.primary_key();
        }
        people.col(definition);
    }
    people.col(
        ColumnDef::new(Alias::new("display_name"))
            .string_len(255)
            .not_null(),
    );
    people.col(ColumnDef::new(Alias::new("raw_identity_id")).big_integer());
    statements.push(SchemaStatement::Table(people));
    let mut identities = create_table("repository_analysis_identities", backend);
    identities.col(
        ColumnDef::new(Alias::new("id"))
            .big_integer()
            .not_null()
            .primary_key(),
    );
    identities.col(
        ColumnDef::new(Alias::new("workspace_id"))
            .big_integer()
            .not_null(),
    );
    identities.col(
        ColumnDef::new(Alias::new("fingerprint"))
            .string_len(64)
            .not_null(),
    );
    identities.col(
        ColumnDef::new(Alias::new("raw_name"))
            .string_len(255)
            .not_null(),
    );
    identities.col(
        ColumnDef::new(Alias::new("raw_email"))
            .string_len(255)
            .not_null(),
    );
    statements.push(SchemaStatement::Table(identities));
    let mut authors = create_table("repository_analysis_commit_authors", backend);
    for column in [
        "commit_id",
        "user_id",
        "workspace_id",
        "repository_id",
        "identity_id",
        "contributor_id",
    ] {
        authors.col(ColumnDef::new(Alias::new(column)).big_integer().not_null());
    }
    authors.primary_key(
        Index::create()
            .col(Alias::new("commit_id"))
            .col(Alias::new("user_id")),
    );
    statements.push(SchemaStatement::Table(authors));
    for (table, name, columns, unique) in [
        (
            "repository_analysis_people",
            "analysis_person_raw",
            &["workspace_id", "user_id", "raw_identity_id"][..],
            true,
        ),
        (
            "repository_analysis_identities",
            "analysis_identity_fingerprint",
            &["workspace_id", "fingerprint"][..],
            true,
        ),
        (
            "repository_analysis_commit_authors",
            "analysis_author_repository",
            &[
                "workspace_id",
                "user_id",
                "repository_id",
                "contributor_id",
                "commit_id",
            ][..],
            false,
        ),
        (
            "repository_analysis_commit_authors",
            "analysis_author_identity",
            &["workspace_id", "user_id", "identity_id"][..],
            false,
        ),
    ] {
        let mut index = Index::create();
        index.name(name).table(Alias::new(table));
        for column in columns {
            index.col(Alias::new(*column));
        }
        if unique {
            index.unique();
        }
        statements.push(SchemaStatement::Index(table, name, index));
    }
    statements
}

struct RepositoryAnalysisIdentity;
impl MigrationName for RepositoryAnalysisIdentity {
    fn name(&self) -> &str {
        "repository_analysis_identity_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for RepositoryAnalysisIdentity {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for statement in repository_analysis_identity(m.get_database_backend()) {
            execute_schema(m, statement).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Business data is not automatically removed".into(),
        ))
    }
}
#[async_trait::async_trait]
impl MigrationTrait for RepositoryAnalysis {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        let mut table = create_table("repository_analysis_configs", backend);
        table.col(
            ColumnDef::new(Alias::new("id"))
                .big_integer()
                .not_null()
                .primary_key(),
        );
        table.col(
            ColumnDef::new(Alias::new("workspace_id"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(long_text("config", backend).not_null());
        table.col(
            ColumnDef::new(Alias::new("coverage_start"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("next_reconcile_at"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(long_text("last_error", backend).not_null());
        table.col(
            ColumnDef::new(Alias::new("version"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("created_at"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("updated_at"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        m.create_table(table.to_owned()).await?;
        execute_schema(
            m,
            SchemaStatement::Index(
                "repository_analysis_configs",
                "analysis_0_0",
                Index::create()
                    .name("analysis_0_0")
                    .table(Alias::new("repository_analysis_configs"))
                    .col(Alias::new("workspace_id"))
                    .col(Alias::new("id"))
                    .unique()
                    .to_owned(),
            ),
        )
        .await?;
        let mut table = create_table("repository_analysis_repositories", backend);
        table.col(
            ColumnDef::new(Alias::new("id"))
                .big_integer()
                .not_null()
                .primary_key(),
        );
        table.col(
            ColumnDef::new(Alias::new("workspace_id"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("identity"))
                .string_len(64)
                .not_null(),
        );
        table.col(
            ColumnDef::new(Alias::new("provider"))
                .string_len(32)
                .not_null(),
        );
        table.col(long_text("path", backend).not_null());
        table.col(long_text("url", backend).not_null());
        table.col(
            ColumnDef::new(Alias::new("created_at"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        m.create_table(table.to_owned()).await?;
        execute_schema(
            m,
            SchemaStatement::Index(
                "repository_analysis_repositories",
                "analysis_1_0",
                Index::create()
                    .name("analysis_1_0")
                    .table(Alias::new("repository_analysis_repositories"))
                    .col(Alias::new("workspace_id"))
                    .col(Alias::new("identity"))
                    .unique()
                    .to_owned(),
            ),
        )
        .await?;
        let mut table = create_table("repository_analysis_sources", backend);
        table.col(
            ColumnDef::new(Alias::new("id"))
                .big_integer()
                .not_null()
                .primary_key(),
        );
        table.col(
            ColumnDef::new(Alias::new("workspace_id"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("connection_id"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("repository_id"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(long_text("remote_path", backend).not_null());
        table.col(
            ColumnDef::new(Alias::new("coverage_start"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("config_version"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("state"))
                .string_len(32)
                .not_null(),
        );
        table.col(long_text("last_error", backend).not_null());
        table.col(
            ColumnDef::new(Alias::new("synced_at"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        m.create_table(table.to_owned()).await?;
        execute_schema(
            m,
            SchemaStatement::Index(
                "repository_analysis_sources",
                "analysis_2_0",
                Index::create()
                    .name("analysis_2_0")
                    .table(Alias::new("repository_analysis_sources"))
                    .col(Alias::new("connection_id"))
                    .col(Alias::new("repository_id"))
                    .unique()
                    .to_owned(),
            ),
        )
        .await?;
        execute_schema(
            m,
            SchemaStatement::Index(
                "repository_analysis_sources",
                "analysis_2_1",
                Index::create()
                    .name("analysis_2_1")
                    .table(Alias::new("repository_analysis_sources"))
                    .col(Alias::new("workspace_id"))
                    .col(Alias::new("repository_id"))
                    .to_owned(),
            ),
        )
        .await?;
        let mut table = create_table("repository_analysis_refs", backend);
        table.col(
            ColumnDef::new(Alias::new("id"))
                .big_integer()
                .not_null()
                .primary_key(),
        );
        table.col(
            ColumnDef::new(Alias::new("workspace_id"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("source_id"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("name"))
                .string_len(255)
                .not_null(),
        );
        table.col(ColumnDef::new(Alias::new("head")).string_len(64).not_null());
        table.col(
            ColumnDef::new(Alias::new("present"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("generation"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("updated_at"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        m.create_table(table.to_owned()).await?;
        execute_schema(
            m,
            SchemaStatement::Index(
                "repository_analysis_refs",
                "analysis_3_0",
                Index::create()
                    .name("analysis_3_0")
                    .table(Alias::new("repository_analysis_refs"))
                    .col(Alias::new("source_id"))
                    .col(Alias::new("name"))
                    .unique()
                    .to_owned(),
            ),
        )
        .await?;
        let mut table = create_table("repository_analysis_commits", backend);
        table.col(
            ColumnDef::new(Alias::new("id"))
                .big_integer()
                .not_null()
                .primary_key(),
        );
        table.col(
            ColumnDef::new(Alias::new("workspace_id"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("repository_id"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(ColumnDef::new(Alias::new("sha")).string_len(64).not_null());
        table.col(long_text("parents", backend).not_null());
        table.col(long_text("message", backend).not_null());
        table.col(
            ColumnDef::new(Alias::new("message_type"))
                .string_len(64)
                .not_null(),
        );
        table.col(
            ColumnDef::new(Alias::new("author_name"))
                .string_len(255)
                .not_null(),
        );
        table.col(
            ColumnDef::new(Alias::new("author_email"))
                .string_len(255)
                .not_null(),
        );
        table.col(
            ColumnDef::new(Alias::new("author_date"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("committer_date"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("is_merge"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("additions"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("deletions"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("effectives"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(long_text("language_stats", backend).not_null());
        table.col(
            ColumnDef::new(Alias::new("analysis_version"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("complete"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        m.create_table(table.to_owned()).await?;
        execute_schema(
            m,
            SchemaStatement::Index(
                "repository_analysis_commits",
                "analysis_4_0",
                Index::create()
                    .name("analysis_4_0")
                    .table(Alias::new("repository_analysis_commits"))
                    .col(Alias::new("workspace_id"))
                    .col(Alias::new("repository_id"))
                    .col(Alias::new("sha"))
                    .unique()
                    .to_owned(),
            ),
        )
        .await?;
        execute_schema(
            m,
            SchemaStatement::Index(
                "repository_analysis_commits",
                "analysis_4_1",
                Index::create()
                    .name("analysis_4_1")
                    .table(Alias::new("repository_analysis_commits"))
                    .col(Alias::new("repository_id"))
                    .col(Alias::new("committer_date"))
                    .to_owned(),
            ),
        )
        .await?;
        execute_schema(
            m,
            SchemaStatement::Index(
                "repository_analysis_commits",
                "analysis_4_2",
                Index::create()
                    .name("analysis_4_2")
                    .table(Alias::new("repository_analysis_commits"))
                    .col(Alias::new("repository_id"))
                    .col(Alias::new("author_email"))
                    .to_owned(),
            ),
        )
        .await?;
        let mut table = create_table("repository_analysis_ref_commits", backend);
        table.col(
            ColumnDef::new(Alias::new("id"))
                .big_integer()
                .not_null()
                .primary_key(),
        );
        table.col(
            ColumnDef::new(Alias::new("workspace_id"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("source_id"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("ref_id"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("commit_id"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("generation"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("current"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        m.create_table(table.to_owned()).await?;
        execute_schema(
            m,
            SchemaStatement::Index(
                "repository_analysis_ref_commits",
                "analysis_5_0",
                Index::create()
                    .name("analysis_5_0")
                    .table(Alias::new("repository_analysis_ref_commits"))
                    .col(Alias::new("ref_id"))
                    .col(Alias::new("commit_id"))
                    .unique()
                    .to_owned(),
            ),
        )
        .await?;
        execute_schema(
            m,
            SchemaStatement::Index(
                "repository_analysis_ref_commits",
                "analysis_5_1",
                Index::create()
                    .name("analysis_5_1")
                    .table(Alias::new("repository_analysis_ref_commits"))
                    .col(Alias::new("source_id"))
                    .col(Alias::new("commit_id"))
                    .to_owned(),
            ),
        )
        .await?;
        let mut table = create_table("repository_analysis_jobs", backend);
        table.col(
            ColumnDef::new(Alias::new("id"))
                .big_integer()
                .not_null()
                .primary_key(),
        );
        table.col(
            ColumnDef::new(Alias::new("workspace_id"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("generation"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("completed_generation"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("state"))
                .string_len(32)
                .not_null(),
        );
        table.col(long_text("cursor", backend).not_null());
        table.col(
            ColumnDef::new(Alias::new("retry_at"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("lease_owner"))
                .string_len(64)
                .not_null(),
        );
        table.col(
            ColumnDef::new(Alias::new("lease_until"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        table.col(long_text("last_error", backend).not_null());
        table.col(
            ColumnDef::new(Alias::new("updated_at"))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
        m.create_table(table.to_owned()).await?;
        execute_schema(
            m,
            SchemaStatement::Index(
                "repository_analysis_jobs",
                "analysis_6_0",
                Index::create()
                    .name("analysis_6_0")
                    .table(Alias::new("repository_analysis_jobs"))
                    .col(Alias::new("state"))
                    .col(Alias::new("retry_at"))
                    .to_owned(),
            ),
        )
        .await?;
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Business data is not automatically removed".into(),
        ))
    }
}

struct CodeInsightSchedules;
impl MigrationName for CodeInsightSchedules {
    fn name(&self) -> &str {
        "code_insight_schedules_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for CodeInsightSchedules {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for statement in code_insight_schedules(m.get_database_backend()) {
            execute_schema(m, statement).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Business data is not automatically removed".into(),
        ))
    }
}
fn code_insight_schedules(_: DbBackend) -> Vec<SchemaStatement> {
    vec![
        SchemaStatement::Column(
            "code_insights",
            "next_run_at",
            Table::alter()
                .table(Alias::new("code_insights"))
                .add_column(
                    ColumnDef::new(Alias::new("next_run_at"))
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .to_owned(),
        ),
        SchemaStatement::Index(
            "code_insights",
            "code_insight_schedule_due",
            Index::create()
                .name("code_insight_schedule_due")
                .table(Alias::new("code_insights"))
                .col(Alias::new("next_run_at"))
                .to_owned(),
        ),
    ]
}

struct CodeInsights;
impl MigrationName for CodeInsights {
    fn name(&self) -> &str {
        "code_insights_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for CodeInsights {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for statement in code_insights(m.get_database_backend()) {
            execute_schema(m, statement).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Business data is not automatically removed".into(),
        ))
    }
}

fn code_insights(backend: DbBackend) -> Vec<SchemaStatement> {
    let mut statements = Vec::new();
    let mut table = create_table("code_insights", backend);
    table.col(
        ColumnDef::new(Alias::new("id"))
            .big_integer()
            .not_null()
            .primary_key(),
    );
    table.col(
        ColumnDef::new(Alias::new("workspace_id"))
            .big_integer()
            .not_null(),
    );
    table.col(
        ColumnDef::new(Alias::new("user_id"))
            .big_integer()
            .not_null(),
    );
    table.col(
        ColumnDef::new(Alias::new("name"))
            .string_len(255)
            .not_null(),
    );
    table.col(long_text("description", backend).not_null());
    table.col(long_text("config", backend).not_null());
    table.col(
        ColumnDef::new(Alias::new("coverage_start"))
            .big_integer()
            .not_null(),
    );
    table.col(
        ColumnDef::new(Alias::new("version"))
            .big_integer()
            .not_null(),
    );
    table.col(
        ColumnDef::new(Alias::new("created_at"))
            .big_integer()
            .not_null(),
    );
    table.col(
        ColumnDef::new(Alias::new("updated_at"))
            .big_integer()
            .not_null(),
    );
    statements.push(SchemaStatement::Table(table.to_owned()));
    let mut table = create_table("code_insight_repositories", backend);
    table.col(
        ColumnDef::new(Alias::new("id"))
            .big_integer()
            .not_null()
            .primary_key(),
    );
    table.col(
        ColumnDef::new(Alias::new("workspace_id"))
            .big_integer()
            .not_null(),
    );
    table.col(
        ColumnDef::new(Alias::new("insight_id"))
            .big_integer()
            .not_null(),
    );
    table.col(
        ColumnDef::new(Alias::new("connection_id"))
            .big_integer()
            .not_null(),
    );
    table.col(long_text("repository", backend).not_null());
    table.col(
        ColumnDef::new(Alias::new("all_repositories"))
            .big_integer()
            .not_null(),
    );
    table.col(
        ColumnDef::new(Alias::new("pipeline_id"))
            .string_len(255)
            .not_null(),
    );
    statements.push(SchemaStatement::Table(table.to_owned()));
    let mut table = create_table("repository_analysis_commit_identities", backend);
    table.col(
        ColumnDef::new(Alias::new("commit_id"))
            .big_integer()
            .not_null()
            .primary_key(),
    );
    table.col(
        ColumnDef::new(Alias::new("workspace_id"))
            .big_integer()
            .not_null(),
    );
    table.col(
        ColumnDef::new(Alias::new("identity_id"))
            .big_integer()
            .not_null(),
    );
    statements.push(SchemaStatement::Table(table.to_owned()));
    let mut table = create_table("code_insight_people", backend);
    table.col(
        ColumnDef::new(Alias::new("insight_id"))
            .big_integer()
            .not_null(),
    );
    table.col(ColumnDef::new(Alias::new("id")).big_integer().not_null());
    table.col(
        ColumnDef::new(Alias::new("workspace_id"))
            .big_integer()
            .not_null(),
    );
    table.col(
        ColumnDef::new(Alias::new("display_name"))
            .string_len(255)
            .not_null(),
    );
    table.col(
        ColumnDef::new(Alias::new("version"))
            .big_integer()
            .not_null(),
    );
    table.col(
        ColumnDef::new(Alias::new("raw_identity_id"))
            .big_integer()
            .null(),
    );
    table.primary_key(
        Index::create()
            .col(Alias::new("insight_id"))
            .col(Alias::new("id")),
    );
    statements.push(SchemaStatement::Table(table.to_owned()));
    let mut table = create_table("code_insight_identity_links", backend);
    table.col(
        ColumnDef::new(Alias::new("insight_id"))
            .big_integer()
            .not_null(),
    );
    table.col(
        ColumnDef::new(Alias::new("identity_id"))
            .big_integer()
            .not_null(),
    );
    table.col(
        ColumnDef::new(Alias::new("workspace_id"))
            .big_integer()
            .not_null(),
    );
    table.col(
        ColumnDef::new(Alias::new("contributor_id"))
            .big_integer()
            .not_null(),
    );
    table.primary_key(
        Index::create()
            .col(Alias::new("insight_id"))
            .col(Alias::new("identity_id")),
    );
    statements.push(SchemaStatement::Table(table.to_owned()));
    statements.push(SchemaStatement::Index(
        "code_insights",
        "insights_owner",
        Index::create()
            .name("insights_owner")
            .table(Alias::new("code_insights"))
            .col(Alias::new("workspace_id"))
            .col(Alias::new("user_id"))
            .to_owned(),
    ));
    statements.push(SchemaStatement::Index(
        "code_insight_repositories",
        "insight_members",
        Index::create()
            .name("insight_members")
            .table(Alias::new("code_insight_repositories"))
            .col(Alias::new("insight_id"))
            .col(Alias::new("connection_id"))
            .to_owned(),
    ));
    statements.push(SchemaStatement::Index(
        "code_insight_repositories",
        "insight_connection",
        Index::create()
            .name("insight_connection")
            .table(Alias::new("code_insight_repositories"))
            .col(Alias::new("workspace_id"))
            .col(Alias::new("connection_id"))
            .to_owned(),
    ));
    statements.push(SchemaStatement::Index(
        "repository_analysis_commit_identities",
        "insight_raw_identity",
        Index::create()
            .name("insight_raw_identity")
            .table(Alias::new("repository_analysis_commit_identities"))
            .col(Alias::new("workspace_id"))
            .col(Alias::new("identity_id"))
            .to_owned(),
    ));
    statements.push(SchemaStatement::Index(
        "code_insight_people",
        "insight_default_person",
        Index::create()
            .name("insight_default_person")
            .table(Alias::new("code_insight_people"))
            .col(Alias::new("insight_id"))
            .col(Alias::new("raw_identity_id"))
            .unique()
            .to_owned(),
    ));
    statements.push(SchemaStatement::Index(
        "code_insight_identity_links",
        "insight_person",
        Index::create()
            .name("insight_person")
            .table(Alias::new("code_insight_identity_links"))
            .col(Alias::new("insight_id"))
            .col(Alias::new("contributor_id"))
            .to_owned(),
    ));
    statements
}

struct TypedMaintenanceValues;
impl MigrationName for TypedMaintenanceValues {
    fn name(&self) -> &str {
        "typed_maintenance_values_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for TypedMaintenanceValues {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for table in ["settings", "dictionaries"] {
            for (column, fallback) in [
                ("category", "general"),
                (
                    "value_type",
                    if table == "settings" {
                        "num"
                    } else {
                        "selection"
                    },
                ),
            ] {
                execute_schema(
                    m,
                    SchemaStatement::Column(
                        table,
                        column,
                        Table::alter()
                            .table(Alias::new(table))
                            .add_column(
                                ColumnDef::new(Alias::new(column))
                                    .string_len(100)
                                    .not_null()
                                    .default(fallback),
                            )
                            .to_owned(),
                    ),
                )
                .await?;
            }
            for column in ["value_json", "schema_json"] {
                execute_schema(
                    m,
                    SchemaStatement::Column(
                        table,
                        column,
                        Table::alter()
                            .table(Alias::new(table))
                            .add_column(long_text(column, m.get_database_backend()))
                            .to_owned(),
                    ),
                )
                .await?;
            }
            let index = if table == "settings" {
                "settings_typed_filter_idx"
            } else {
                "dictionaries_typed_filter_idx"
            };
            execute_schema(
                m,
                SchemaStatement::Index(
                    table,
                    index,
                    Index::create()
                        .name(index)
                        .table(Alias::new(table))
                        .col(Alias::new("workspace_id"))
                        .col(Alias::new("category"))
                        .col(Alias::new("value_type"))
                        .to_owned(),
                ),
            )
            .await?;
        }
        for column in ["name", "description"] {
            execute_schema(
                m,
                SchemaStatement::Column(
                    "settings",
                    column,
                    Table::alter()
                        .table(Alias::new("settings"))
                        .add_column(
                            ColumnDef::new(Alias::new(column))
                                .string_len(if column == "name" { 100 } else { 10000 })
                                .not_null()
                                .default(""),
                        )
                        .to_owned(),
                ),
            )
            .await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Typed maintenance data cannot be automatically removed".into(),
        ))
    }
}

struct RepositoryEventDeliveryMessages;
impl MigrationName for RepositoryEventDeliveryMessages {
    fn name(&self) -> &str {
        "repository_event_delivery_messages_v1"
    }
}

fn repository_event_delivery_messages(backend: DbBackend) -> Option<TableAlterStatement> {
    // SQLite does not enforce the old VARCHAR length and cannot alter a type
    // without rebuilding the table. Its existing text affinity already suffices.
    (backend != DbBackend::Sqlite).then(|| {
        Table::alter()
            .table(Alias::new("repository_events"))
            .modify_column(long_text("delivery", backend).not_null())
            .to_owned()
    })
}

#[async_trait::async_trait]
impl MigrationTrait for RepositoryEventDeliveryMessages {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if let Some(statement) = repository_event_delivery_messages(m.get_database_backend()) {
            // Repeating the same widening is safe after interrupted MySQL DDL.
            m.alter_table(statement).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Delivery message storage cannot be narrowed without data loss".into(),
        ))
    }
}

#[cfg(test)]
mod delivery_message_tests {
    use super::*;
    use crate::{
        entities::repository_events,
        i18n::{self, Locale, Message},
    };
    use sea_orm::{Database, EntityTrait, IntoActiveModel};

    #[test]
    fn delivery_storage_widens_without_rewriting_existing_values() {
        let mysql = repository_event_delivery_messages(DbBackend::MySql)
            .unwrap()
            .to_string(MysqlQueryBuilder);
        assert!(
            mysql.contains("MODIFY COLUMN `delivery` longtext NOT NULL"),
            "{mysql}"
        );
        let postgres = repository_event_delivery_messages(DbBackend::Postgres)
            .unwrap()
            .to_string(PostgresQueryBuilder);
        assert!(
            postgres.contains("ALTER COLUMN \"delivery\" TYPE text"),
            "{postgres}"
        );
        assert!(repository_event_delivery_messages(DbBackend::Sqlite).is_none());
    }

    #[tokio::test]
    async fn upgraded_event_delivery_roundtrips_quoted_unicode_diagnostics_for_each_reader() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let before = Migrator::migrations()
            .iter()
            .position(|migration| migration.name() == RepositoryEventDeliveryMessages.name())
            .unwrap();
        Migrator::up(&db, Some(before as u32)).await.unwrap();
        let legacy = repository_events::Model {
            id: 1,
            workspace_id: 1,
            connection_id: 1,
            delivery_key: "legacy".into(),
            event: "push".into(),
            repository: "user/repository".into(),
            reference: "main".into(),
            actor: String::new(),
            action: "user action".into(),
            delivery: "旧状态 raw \"quoted\"".into(),
            created_at: 0,
            detail: None,
        };
        repository_events::Entity::insert(legacy.clone().into_active_model())
            .exec(&db)
            .await
            .unwrap();
        Migrator::up(&db, None).await.unwrap();
        RepositoryEventDeliveryMessages
            .up(&SchemaManager::new(&db))
            .await
            .unwrap();
        assert_eq!(
            repository_events::Entity::find_by_id(1)
                .one(&db)
                .await
                .unwrap()
                .unwrap(),
            legacy
        );
        let diagnostic = format!("{}{}", "\"".repeat(150), "漢".repeat(50));
        let message = Message::new("webhook.failed")
            .with("provider", "Google Chat")
            .with_message(
                "reason",
                Message::new("webhook.code")
                    .with("code", i64::MIN.to_string())
                    .with("detail", &diagnostic),
            );
        let stored = i18n::store_message(&message);
        // JSON escaping is part of the column's character count.
        assert!(
            stored.chars().count() > 512,
            "{} characters",
            stored.chars().count()
        );
        let mut updated = legacy.clone();
        updated.id = 2;
        updated.delivery_key = "typed".into();
        updated.delivery = stored.clone();
        repository_events::Entity::insert(updated.into_active_model())
            .exec(&db)
            .await
            .unwrap();
        let read = repository_events::Entity::find_by_id(2)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.delivery, stored);
        for locale in Locale::ALL {
            let text = i18n::render_stored(locale, &read.delivery);
            assert!(text.contains(&diagnostic));
            assert!(!text.contains("$system_message"));
            assert_eq!(text, i18n::render(locale, &message));
        }
        assert_ne!(
            i18n::render_stored(Locale::EnUs, &stored),
            i18n::render_stored(Locale::JaJp, &stored)
        );
    }
}

struct RunnerAutoUpdates;
impl MigrationName for RunnerAutoUpdates {
    fn name(&self) -> &str {
        "runner_auto_updates_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for RunnerAutoUpdates {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for (table, column) in [
            (
                "platform_state",
                ColumnDef::new(Alias::new("runner_auto_update"))
                    .big_integer()
                    .not_null()
                    .default(1_i64)
                    .to_owned(),
            ),
            (
                "hosts",
                ColumnDef::new(Alias::new("runner_auto_update"))
                    .string()
                    .not_null()
                    .default("inherit")
                    .to_owned(),
            ),
        ] {
            execute_schema(
                m,
                SchemaStatement::Column(
                    table,
                    "runner_auto_update",
                    Table::alter()
                        .table(Alias::new(table))
                        .add_column(column)
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

struct HostExecutionCapacity;
struct DeploymentCategory;
impl MigrationName for DeploymentCategory {
    fn name(&self) -> &str {
        "deployment_category_v1"
    }
}
fn deployment_category_column() -> TableAlterStatement {
    Table::alter()
        .table(Alias::new("deployments"))
        .add_column(
            ColumnDef::new(Alias::new("category"))
                .string_len(100)
                .not_null()
                .default(""),
        )
        .to_owned()
}
#[async_trait::async_trait]
impl MigrationTrait for DeploymentCategory {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        execute_schema(
            m,
            SchemaStatement::Column("deployments", "category", deployment_category_column()),
        )
        .await
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

#[cfg(test)]
mod deployment_category_tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database, Statement};

    #[test]
    fn category_column_uses_portable_default_empty_ddl() {
        for sql in [
            deployment_category_column().to_string(MysqlQueryBuilder),
            deployment_category_column().to_string(PostgresQueryBuilder),
            deployment_category_column().to_string(SqliteQueryBuilder),
        ] {
            assert!(
                sql.contains("category") && sql.contains("NOT NULL") && sql.contains("DEFAULT ''"),
                "{sql}"
            );
        }
    }

    #[tokio::test]
    async fn existing_applications_upgrade_without_data_loss_and_migration_retries() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared(
            "CREATE TABLE deployments (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
        )
        .await
        .unwrap();
        db.execute_unprepared(
            "INSERT INTO deployments (id,name) VALUES (1,'existing application')",
        )
        .await
        .unwrap();
        let manager = SchemaManager::new(&db);
        DeploymentCategory.up(&manager).await.unwrap();
        DeploymentCategory.up(&manager).await.unwrap();
        let row = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT name,category FROM deployments WHERE id=1",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.try_get::<String>("", "name").unwrap(),
            "existing application"
        );
        assert_eq!(row.try_get::<String>("", "category").unwrap(), "");
        db.execute_unprepared("UPDATE deployments SET category='web' WHERE id=1")
            .await
            .unwrap();
        DeploymentCategory.up(&manager).await.unwrap();
        let row = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT category FROM deployments WHERE id=1",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<String>("", "category").unwrap(), "web");
    }
}

impl MigrationName for HostExecutionCapacity {
    fn name(&self) -> &str {
        "host_execution_capacity_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for HostExecutionCapacity {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        m.create_table(
            create_table("host_execution_policies", backend)
                .col(
                    ColumnDef::new(Alias::new("host_id"))
                        .big_integer()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(Alias::new("workspace_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("enabled"))
                        .boolean()
                        .not_null()
                        .default(true),
                )
                .col(
                    ColumnDef::new(Alias::new("shared"))
                        .boolean()
                        .not_null()
                        .default(false),
                )
                .col(
                    ColumnDef::new(Alias::new("capacity"))
                        .big_integer()
                        .not_null()
                        .default(1_i64),
                )
                .to_owned(),
        )
        .await?;
        m.create_table(
            create_table("host_execution_slots", backend)
                .col(
                    ColumnDef::new(Alias::new("host_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(ColumnDef::new(Alias::new("slot")).big_integer().not_null())
                .col(
                    ColumnDef::new(Alias::new("owner"))
                        .string_len(36)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("run_id"))
                        .big_integer()
                        .not_null(),
                )
                .col(ColumnDef::new(Alias::new("job_key")).string().not_null())
                .col(
                    ColumnDef::new(Alias::new("expires_at"))
                        .big_integer()
                        .not_null(),
                )
                .primary_key(
                    Index::create()
                        .col(Alias::new("host_id"))
                        .col(Alias::new("slot")),
                )
                .to_owned(),
        )
        .await?;
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("?????????".into()))
    }
}

struct ExecutionLeases;
impl MigrationName for ExecutionLeases {
    fn name(&self) -> &str {
        "execution_leases_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for ExecutionLeases {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for statement in execution_leases(m.get_database_backend()) {
            execute_schema(m, statement).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Execution ownership history must not be removed by an automatic rollback".into(),
        ))
    }
}

fn execution_leases(_: DbBackend) -> Vec<SchemaStatement> {
    let mut columns = vec![];
    for (name, column) in [
        (
            "lease_owner",
            ColumnDef::new(Alias::new("lease_owner"))
                .string_len(128)
                .not_null()
                .default("")
                .to_owned(),
        ),
        (
            "lease_epoch",
            ColumnDef::new(Alias::new("lease_epoch"))
                .big_integer()
                .not_null()
                .default(0_i64)
                .to_owned(),
        ),
        (
            "not_before",
            ColumnDef::new(Alias::new("not_before"))
                .big_integer()
                .null()
                .to_owned(),
        ),
    ] {
        columns.push(SchemaStatement::Column(
            "pipeline_runs",
            name,
            Table::alter()
                .table(Alias::new("pipeline_runs"))
                .add_column(column)
                .to_owned(),
        ));
    }
    columns.push(SchemaStatement::Index(
        "pipeline_runs",
        "pipeline_runs_lease_recovery",
        Index::create()
            .name("pipeline_runs_lease_recovery")
            .table(Alias::new("pipeline_runs"))
            .col(Alias::new("state"))
            .col(Alias::new("lease_owner"))
            .to_owned(),
    ));
    columns
}

struct ContentAssets;
impl MigrationName for ContentAssets {
    fn name(&self) -> &str {
        "content_assets_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for ContentAssets {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for statement in content_assets(m.get_database_backend()) {
            execute_schema(m, statement).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "asset references and pending object deletions must be retained".into(),
        ))
    }
}
fn content_assets(backend: DbBackend) -> Vec<SchemaStatement> {
    let mut assets = create_table("assets", backend);
    assets.col(
        ColumnDef::new(Alias::new("id"))
            .big_integer()
            .not_null()
            .primary_key(),
    );
    for name in ["workspace_id", "size", "pinned", "created_at"] {
        assets.col(ColumnDef::new(Alias::new(name)).big_integer().not_null());
    }
    for name in [
        "width",
        "height",
        "duration_ms",
        "thumbnail_asset_id",
        "execution_id",
    ] {
        assets.col(ColumnDef::new(Alias::new(name)).big_integer().null());
    }
    for (name, len) in [("sha256", 64), ("mime", 255), ("kind", 32)] {
        assets.col(ColumnDef::new(Alias::new(name)).string_len(len).not_null());
    }
    assets.col(ColumnDef::new(Alias::new("node_id")).string_len(255).null());
    assets.col(long_text("provenance", backend).null());
    assets.col(
        ColumnDef::new(Alias::new("created_by"))
            .big_integer()
            .not_null()
            .default(0_i64),
    );
    assets.col(
        ColumnDef::new(Alias::new("version"))
            .big_integer()
            .not_null()
            .default(1_i64),
    );
    let mut refs = create_table("asset_refs", backend);
    refs.col(
        ColumnDef::new(Alias::new("asset_id"))
            .big_integer()
            .not_null(),
    )
    .col(
        ColumnDef::new(Alias::new("owner_kind"))
            .string_len(32)
            .not_null(),
    )
    .col(
        ColumnDef::new(Alias::new("owner_id"))
            .big_integer()
            .not_null(),
    )
    .col(
        ColumnDef::new(Alias::new("version"))
            .big_integer()
            .not_null()
            .default(1_i64),
    );
    let mut tags = create_table("asset_tags", backend);
    tags.col(
        ColumnDef::new(Alias::new("asset_id"))
            .big_integer()
            .not_null(),
    )
    .col(ColumnDef::new(Alias::new("tag")).string_len(255).not_null())
    .col(
        ColumnDef::new(Alias::new("version"))
            .big_integer()
            .not_null()
            .default(1_i64),
    );
    let mut gc = create_table("asset_gc_jobs", backend);
    gc.col(
        ColumnDef::new(Alias::new("id"))
            .big_integer()
            .not_null()
            .primary_key(),
    );
    for name in ["workspace_id", "asset_id", "eligible_at", "updated_at"] {
        gc.col(ColumnDef::new(Alias::new(name)).big_integer().not_null());
    }
    for name in ["eligible_by", "updated_by", "attempt"] {
        gc.col(
            ColumnDef::new(Alias::new(name))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
    }
    gc.col(long_text("storage_key", backend).not_null())
        .col(
            ColumnDef::new(Alias::new("state"))
                .string_len(16)
                .not_null()
                .default("candidate"),
        )
        .col(long_text("error", backend).null())
        .col(
            ColumnDef::new(Alias::new("version"))
                .big_integer()
                .not_null()
                .default(1_i64),
        );
    let mut statements = vec![
        SchemaStatement::Table(assets.to_owned()),
        SchemaStatement::Table(refs.to_owned()),
        SchemaStatement::Table(tags.to_owned()),
        SchemaStatement::Table(gc.to_owned()),
    ];
    for (table, name, columns, unique) in [
        (
            "assets",
            "assets_sha256",
            &["workspace_id", "sha256"][..],
            true,
        ),
        (
            "asset_refs",
            "asset_refs_owner",
            &["asset_id", "owner_kind", "owner_id"][..],
            true,
        ),
        (
            "asset_refs",
            "asset_refs_lookup",
            &["owner_kind", "owner_id"][..],
            false,
        ),
        ("asset_tags", "asset_tags_tag", &["tag"][..], false),
        (
            "asset_gc_jobs",
            "gc_asset",
            &["workspace_id", "asset_id"][..],
            true,
        ),
        (
            "asset_gc_jobs",
            "gc_dispatch",
            &["state", "eligible_at", "id"][..],
            false,
        ),
    ] {
        let mut index = Index::create();
        index.name(name).table(Alias::new(table));
        for column in columns {
            index.col(Alias::new(*column));
        }
        if unique {
            index.unique();
        }
        statements.push(SchemaStatement::Index(table, name, index.to_owned()));
    }
    statements
}

struct DefinitionPublications;
impl MigrationName for DefinitionPublications {
    fn name(&self) -> &str {
        "definition_publications_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for DefinitionPublications {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let mut table = create_table("definition_publications", m.get_database_backend());
        table.col(
            ColumnDef::new(Alias::new("id"))
                .big_integer()
                .not_null()
                .primary_key(),
        );
        for field in [
            "workspace_id",
            "subject_id",
            "revision_id",
            "run_as",
            "created_at",
        ] {
            table.col(ColumnDef::new(Alias::new(field)).big_integer().not_null());
        }
        table
            .col(
                ColumnDef::new(Alias::new("subject_kind"))
                    .string_len(16)
                    .not_null(),
            )
            .col(ColumnDef::new(Alias::new("name")).string_len(64).not_null())
            .col(
                ColumnDef::new(Alias::new("name_key"))
                    .string_len(64)
                    .not_null(),
            )
            .col(ColumnDef::new(Alias::new("change_note")).string_len(200))
            .col(ColumnDef::new(Alias::new("request_key")).string_len(64))
            .col(
                ColumnDef::new(Alias::new("created_by"))
                    .big_integer()
                    .not_null()
                    .default(0_i64),
            )
            .col(
                ColumnDef::new(Alias::new("version"))
                    .big_integer()
                    .not_null()
                    .default(1_i64),
            )
            .check(Expr::col(Alias::new("subject_kind")).is_in(["workflow", "pipeline"]));
        execute_schema(m, SchemaStatement::Table(table.to_owned())).await?;
        for (name, key) in [
            ("publication_name", "name_key"),
            ("publication_request", "request_key"),
        ] {
            let mut index = Index::create();
            index
                .name(name)
                .table(Alias::new("definition_publications"))
                .unique();
            for field in ["workspace_id", "subject_kind", "subject_id", key] {
                index.col(Alias::new(field));
            }
            execute_schema(
                m,
                SchemaStatement::Index("definition_publications", name, index.to_owned()),
            )
            .await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("publication history must be retained".into()))
    }
}

#[cfg(test)]
mod publication_schema_tests {
    use super::*;
    use crate::entities::definition_publications as publications;
    use sea_orm::{Database, EntityTrait, IntoActiveModel, PaginatorTrait};

    #[tokio::test]
    async fn publication_upgrade_is_repeatable_preserves_history_and_enforces_subject_uniqueness() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);
        DefinitionPublications.up(&manager).await.unwrap();
        let row = publications::Model {
            id: 1,
            workspace_id: 11,
            subject_kind: "workflow".into(),
            subject_id: 7,
            revision_id: 8,
            name: "A".into(),
            name_key: "A".into(),
            change_note: Some("Original".into()),
            run_as: 3,
            request_key: Some("request-a".into()),
            created_at: 10,
            created_by: 3,
            version: 1,
        };
        publications::Entity::insert(row.clone().into_active_model())
            .exec(&db)
            .await
            .unwrap();
        DefinitionPublications.up(&manager).await.unwrap();
        assert_eq!(
            publications::Entity::find_by_id(1)
                .one(&db)
                .await
                .unwrap()
                .unwrap(),
            row
        );
        let mut changed = row.clone();
        changed.id = 2;
        changed.request_key = Some("request-b".into());
        assert!(
            publications::Entity::insert(changed.clone().into_active_model())
                .exec(&db)
                .await
                .is_err()
        );
        changed.name = "a".into();
        changed.name_key = "a".into();
        changed.request_key = row.request_key.clone();
        assert!(
            publications::Entity::insert(changed.clone().into_active_model())
                .exec(&db)
                .await
                .is_err()
        );
        changed.request_key = None;
        publications::Entity::insert(changed.clone().into_active_model())
            .exec(&db)
            .await
            .unwrap();
        changed.id = 3;
        changed.subject_id = 9;
        changed.name = "A".into();
        changed.name_key = "A".into();
        changed.request_key = row.request_key.clone();
        publications::Entity::insert(changed.into_active_model())
            .exec(&db)
            .await
            .unwrap();
        assert_eq!(publications::Entity::find().count(&db).await.unwrap(), 3);
        assert!(DefinitionPublications.down(&manager).await.is_err());
        assert_eq!(publications::Entity::find().count(&db).await.unwrap(), 3);
    }
}

struct DefinitionAcl;
impl MigrationName for DefinitionAcl {
    fn name(&self) -> &str {
        "definition_acl_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for DefinitionAcl {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for statement in definition_acl(m.get_database_backend()) {
            execute_schema(m, statement).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("definition grants must be retained".into()))
    }
}
fn definition_acl(backend: DbBackend) -> Vec<SchemaStatement> {
    let mut table = create_table("definition_acl", backend);
    table.col(
        ColumnDef::new(Alias::new("id"))
            .big_integer()
            .not_null()
            .primary_key(),
    );
    for column in ["workspace_id", "subject_id", "user_id", "created_at"] {
        table.col(ColumnDef::new(Alias::new(column)).big_integer().not_null());
    }
    table
        .col(
            ColumnDef::new(Alias::new("subject_kind"))
                .string_len(16)
                .not_null(),
        )
        .col(ColumnDef::new(Alias::new("role")).string_len(16).not_null())
        .col(
            ColumnDef::new(Alias::new("created_by"))
                .big_integer()
                .not_null()
                .default(0_i64),
        )
        .col(
            ColumnDef::new(Alias::new("version"))
                .big_integer()
                .not_null()
                .default(1_i64),
        )
        .check(Expr::col(Alias::new("role")).is_in(["all", "run", "view"]))
        .check(Expr::col(Alias::new("subject_kind")).is_in([
            "pipeline",
            "workflow",
            "pipeline_group",
            "workflow_group",
        ]));
    let mut statements = vec![SchemaStatement::Table(table.to_owned())];
    for (table, name, columns, unique) in [
        (
            "definition_acl",
            "acl_identity",
            &[
                "workspace_id",
                "subject_kind",
                "subject_id",
                "user_id",
                "role",
            ][..],
            true,
        ),
        (
            "definition_acl",
            "acl_principal",
            &["workspace_id", "user_id", "subject_kind", "subject_id"][..],
            false,
        ),
        (
            "workflows",
            "workflow_authorized_list",
            &["workspace_id", "lifecycle", "updated_at", "id"][..],
            false,
        ),
        (
            "workflows",
            "workflow_group_lookup",
            &["workspace_id", "group_id", "id"][..],
            false,
        ),
        (
            "pipeline_contexts",
            "workflow_context_groups",
            &["workspace_id", "kind", "position", "id"][..],
            false,
        ),
    ] {
        let mut index = Index::create();
        index.name(name).table(Alias::new(table));
        for column in columns {
            index.col(Alias::new(*column));
        }
        if unique {
            index.unique();
        }
        statements.push(SchemaStatement::Index(table, name, index.to_owned()));
    }
    statements
}

struct WorkflowStorage;
impl MigrationName for WorkflowStorage {
    fn name(&self) -> &str {
        "workflow_storage_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for WorkflowStorage {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for statement in workflow_storage(m.get_database_backend()) {
            execute_schema(m, statement).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "workflow definitions, execution provenance and node attempts must be retained".into(),
        ))
    }
}

/// Full storage target from the workflow model, including its §10.11 increments.
/// No legacy pipeline is published or assigned an inferred owner by this migration.
fn workflow_storage(backend: DbBackend) -> Vec<SchemaStatement> {
    let mut definitions = create_table("workflows", backend);
    definitions
        .col(
            ColumnDef::new(Alias::new("id"))
                .big_integer()
                .not_null()
                .primary_key(),
        )
        .check(Expr::col(Alias::new("lifecycle")).is_in(["active", "archived"]));
    for name in [
        "workspace_id",
        "group_id",
        "draft_revision_id",
        "created_at",
        "updated_at",
    ] {
        definitions.col(ColumnDef::new(Alias::new(name)).big_integer().not_null());
    }
    for name in [
        "published_revision_id",
        "owner_id",
        "trigger_run_as",
        "archived_at",
    ] {
        definitions.col(ColumnDef::new(Alias::new(name)).big_integer().null());
    }
    for name in ["created_by", "updated_by", "archived_by"] {
        definitions.col(
            ColumnDef::new(Alias::new(name))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
    }
    definitions
        .col(
            ColumnDef::new(Alias::new("name"))
                .string_len(255)
                .not_null(),
        )
        .col(
            ColumnDef::new(Alias::new("lifecycle"))
                .string_len(16)
                .not_null()
                .default("active"),
        )
        .col(
            ColumnDef::new(Alias::new("version"))
                .big_integer()
                .not_null()
                .default(1_i64),
        );
    for name in ["description", "definition", "settings"] {
        definitions.col(long_text(name, backend).not_null());
    }

    let mut executions = create_table("workflow_executions", backend);
    executions.col(
        ColumnDef::new(Alias::new("id"))
            .big_integer()
            .not_null()
            .primary_key(),
    );
    for name in [
        "workspace_id",
        "workflow_id",
        "revision_id",
        "number",
        "triggered_by",
        "parent_execution_id",
        "retry_of_id",
        "root_execution_id",
        "cost_micros",
        "data_bytes",
        "created_at",
        "started_at",
        "finished_at",
    ] {
        executions.col(ColumnDef::new(Alias::new(name)).big_integer().not_null());
    }
    for name in [
        "created_by",
        "started_by",
        "finished_by",
        "lease_epoch",
        "priority",
        "concurrency_limit",
        "waiting_ms",
        "response_deadline_by",
        "response_delivered_by",
    ] {
        executions.col(
            ColumnDef::new(Alias::new(name))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
    }
    for name in [
        "not_before",
        "response_deadline_at",
        "response_delivered_at",
    ] {
        executions.col(ColumnDef::new(Alias::new(name)).big_integer().null());
    }
    for (name, length) in [
        ("mode", 32),
        ("state", 32),
        ("trigger_kind", 32),
        ("trigger_ref", 64),
    ] {
        executions.col(
            ColumnDef::new(Alias::new(name))
                .string_len(length)
                .not_null(),
        );
    }
    for (name, length) in [
        ("lease_owner", 128),
        ("end_reason", 64),
        ("concurrency_key", 128),
        ("queue_reason", 255),
    ] {
        executions.col(
            ColumnDef::new(Alias::new(name))
                .string_len(length)
                .not_null()
                .default(""),
        );
    }
    executions
        .col(long_text("context", backend).null())
        .col(
            ColumnDef::new(Alias::new("response_state"))
                .string_len(32)
                .not_null()
                .default("not_applicable"),
        )
        .col(
            ColumnDef::new(Alias::new("version"))
                .big_integer()
                .not_null()
                .default(1_i64),
        )
        .check(Expr::col(Alias::new("waiting_ms")).gte(0_i64));

    let mut attempts = create_table("workflow_node_runs", backend);
    attempts.col(
        ColumnDef::new(Alias::new("id"))
            .big_integer()
            .not_null()
            .primary_key(),
    );
    for name in [
        "execution_id",
        "attempt",
        "items_in",
        "items_out",
        "cost_micros",
        "started_at",
        "finished_at",
    ] {
        attempts.col(ColumnDef::new(Alias::new(name)).big_integer().not_null());
    }
    for name in ["output_asset", "cache_hit_from", "seed"] {
        attempts.col(ColumnDef::new(Alias::new(name)).big_integer().null());
    }
    for name in ["started_by", "finished_by", "redo_round"] {
        attempts.col(
            ColumnDef::new(Alias::new(name))
                .big_integer()
                .not_null()
                .default(0_i64),
        );
    }
    for (name, length) in [
        ("node_id", 255),
        ("iteration_hash", 64),
        ("state", 32),
        ("cache_key", 64),
    ] {
        attempts.col(
            ColumnDef::new(Alias::new(name))
                .string_len(length)
                .not_null(),
        );
    }
    for name in ["iteration_path", "output", "error"] {
        attempts.col(long_text(name, backend).not_null());
    }
    attempts
        .col(long_text("resolved_params", backend).null())
        .col(
            ColumnDef::new(Alias::new("error_policy"))
                .string_len(16)
                .not_null()
                .default("stop"),
        )
        .col(
            ColumnDef::new(Alias::new("version"))
                .big_integer()
                .not_null()
                .default(1_i64),
        )
        .check(Expr::col(Alias::new("redo_round")).gte(0_i64));

    let mut statements = vec![
        SchemaStatement::Table(definitions.to_owned()),
        SchemaStatement::Table(executions.to_owned()),
        SchemaStatement::Table(attempts.to_owned()),
        SchemaStatement::Column(
            "workspaces",
            "workflow_execution_number",
            Table::alter()
                .table(Alias::new("workspaces"))
                .add_column(
                    ColumnDef::new(Alias::new("workflow_execution_number"))
                        .big_integer()
                        .not_null()
                        .default(0_i64),
                )
                .to_owned(),
        ),
    ];
    for (table, name, columns, unique) in [
        (
            "workflow_executions",
            "workflow_executions_dispatch",
            &["state", "priority", "created_at"][..],
            false,
        ),
        (
            "workflow_executions",
            "workflow_executions_workflow",
            &["workspace_id", "workflow_id", "created_at"][..],
            false,
        ),
        (
            "workflow_executions",
            "workflow_execution_number",
            &["workspace_id", "number"][..],
            true,
        ),
        (
            "workflow_executions",
            "workflow_execution_statistics",
            &["workspace_id", "workflow_id", "state", "finished_at"][..],
            false,
        ),
        (
            "workflow_node_runs",
            "workflow_node_runs_attempt",
            &["execution_id", "node_id", "iteration_hash", "attempt"][..],
            true,
        ),
    ] {
        let mut index = Index::create();
        index.name(name).table(Alias::new(table));
        for column in columns {
            index.col(Alias::new(*column));
        }
        if unique {
            index.unique();
        }
        statements.push(SchemaStatement::Index(table, name, index.to_owned()));
    }
    statements
}

struct DefinitionRevisions;
impl MigrationName for DefinitionRevisions {
    fn name(&self) -> &str {
        "definition_revisions_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for DefinitionRevisions {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        use sea_orm::{ConnectionTrait, TransactionTrait};
        for statement in definition_revisions(m.get_database_backend()) {
            execute_schema(m, statement).await?;
        }
        let tx = m.get_connection().begin().await?;
        // Reuse each existing pipeline's Snowflake for its first revision in
        // this separate table. Do not invent historical authors or publications.
        let select = Query::select()
            .column((Alias::new("pipelines"), Alias::new("id")))
            .column(Alias::new("workspace_id"))
            .expr(Expr::val("pipeline"))
            .column((Alias::new("pipelines"), Alias::new("id")))
            .expr(Expr::val(1_i64))
            .column(Alias::new("definition"))
            .expr(Expr::val(crate::db::now()))
            .from(Alias::new("pipelines"))
            .and_where(
                Expr::exists(
                    Query::select()
                        .expr(Expr::val(1))
                        .from(Alias::new("definition_revisions"))
                        .and_where(Expr::col(Alias::new("subject_kind")).eq("pipeline"))
                        .and_where(
                            Expr::col(Alias::new("subject_id"))
                                .equals((Alias::new("pipelines"), Alias::new("id"))),
                        )
                        .to_owned(),
                )
                .not(),
            )
            .to_owned();
        let mut insert = Query::insert();
        insert
            .into_table(Alias::new("definition_revisions"))
            .columns(
                [
                    "id",
                    "workspace_id",
                    "subject_kind",
                    "subject_id",
                    "number",
                    "definition",
                    "created_at",
                ]
                .map(Alias::new),
            )
            .select_from(select)
            .map_err(|e| DbErr::Custom(e.to_string()))?;
        tx.execute_raw(tx.get_database_backend().build(&insert))
            .await?;
        tx.commit().await?;
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "definition revisions and run provenance must be retained".into(),
        ))
    }
}

fn definition_revisions(backend: DbBackend) -> Vec<SchemaStatement> {
    let mut table = create_table("definition_revisions", backend);
    table.check(Expr::cust(if backend == DbBackend::MySql {
        "message IS NULL OR CHAR_LENGTH(message) <= 200"
    } else {
        "message IS NULL OR length(message) <= 200"
    }));
    table.col(
        ColumnDef::new(Alias::new("id"))
            .big_integer()
            .not_null()
            .primary_key(),
    );
    for name in ["workspace_id", "subject_id", "number", "created_at"] {
        table.col(ColumnDef::new(Alias::new(name)).big_integer().not_null());
    }
    table
        .col(
            ColumnDef::new(Alias::new("subject_kind"))
                .string_len(16)
                .not_null(),
        )
        .col(long_text("definition", backend).not_null())
        .col(ColumnDef::new(Alias::new("message")).string_len(200).null())
        .col(
            ColumnDef::new(Alias::new("created_by"))
                .big_integer()
                .not_null()
                .default(0_i64),
        )
        .col(
            ColumnDef::new(Alias::new("version"))
                .big_integer()
                .not_null()
                .default(1_i64),
        );
    vec![
        SchemaStatement::Table(table.to_owned()),
        SchemaStatement::Index(
            "definition_revisions",
            "definition_revisions_number",
            Index::create()
                .name("definition_revisions_number")
                .table(Alias::new("definition_revisions"))
                .col(Alias::new("subject_kind"))
                .col(Alias::new("subject_id"))
                .col(Alias::new("number"))
                .unique()
                .to_owned(),
        ),
        SchemaStatement::Column(
            "pipeline_runs",
            "revision_id",
            Table::alter()
                .table(Alias::new("pipeline_runs"))
                .add_column(
                    ColumnDef::new(Alias::new("revision_id"))
                        .big_integer()
                        .not_null()
                        .default(0_i64),
                )
                .to_owned(),
        ),
    ]
}

#[cfg(test)]
mod execution_lease_schema_tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database, Statement};

    #[tokio::test]
    async fn revision_upgrade_preserves_history_and_repeated_upgrade_never_overwrites_it() {
        use crate::entities::definition_revisions as revisions;
        use sea_orm::EntityTrait;
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("CREATE TABLE pipelines (id BIGINT PRIMARY KEY, workspace_id BIGINT NOT NULL, definition TEXT NOT NULL)").await.unwrap();
        db.execute_unprepared(
            "CREATE TABLE pipeline_runs (id BIGINT PRIMARY KEY, state TEXT NOT NULL)",
        )
        .await
        .unwrap();
        db.execute_unprepared("INSERT INTO pipelines VALUES (7,11,'{\"original\":true}')")
            .await
            .unwrap();
        db.execute_unprepared("INSERT INTO pipeline_runs VALUES (8,'succeeded')")
            .await
            .unwrap();
        let manager = SchemaManager::new(&db);
        DefinitionRevisions.up(&manager).await.unwrap();
        let first = revisions::Entity::find_by_id(7)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.number, 1);
        assert_eq!(first.created_by, 0);
        assert_eq!(first.definition, r#"{"original":true}"#);
        db.execute_unprepared("UPDATE pipelines SET definition='{\"later\":true}'")
            .await
            .unwrap();
        DefinitionRevisions.up(&manager).await.unwrap();
        assert_eq!(
            revisions::Entity::find_by_id(7)
                .one(&db)
                .await
                .unwrap()
                .unwrap(),
            first
        );
        let row = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT state,revision_id FROM pipeline_runs WHERE id=8",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<String>("", "state").unwrap(), "succeeded");
        assert_eq!(row.try_get::<i64>("", "revision_id").unwrap(), 0);
        assert!(DefinitionRevisions.down(&manager).await.is_err());
    }

    #[tokio::test]
    async fn lease_upgrade_is_repeatable_and_preserves_legacy_ownership() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("CREATE TABLE locks (name TEXT PRIMARY KEY, owner TEXT NOT NULL, acquired_at BIGINT NOT NULL, expires_at BIGINT NOT NULL)").await.unwrap();
        db.execute_unprepared("CREATE TABLE pipeline_runs (id BIGINT PRIMARY KEY, workspace_id BIGINT NOT NULL, state TEXT NOT NULL, started_at BIGINT NOT NULL)").await.unwrap();
        db.execute_unprepared("INSERT INTO pipeline_runs VALUES (7,11,'running',123)")
            .await
            .unwrap();
        let manager = SchemaManager::new(&db);
        ExecutionLeases.up(&manager).await.unwrap();
        ExecutionLeases.up(&manager).await.unwrap();
        let row = db.query_one_raw(Statement::from_string(DbBackend::Sqlite,
            "SELECT state,started_at,lease_owner,lease_epoch,not_before FROM pipeline_runs WHERE id=7"
        )).await.unwrap().unwrap();
        assert_eq!(row.try_get::<String>("", "state").unwrap(), "running");
        assert_eq!(row.try_get::<i64>("", "started_at").unwrap(), 123);
        assert_eq!(row.try_get::<String>("", "lease_owner").unwrap(), "");
        assert_eq!(row.try_get::<i64>("", "lease_epoch").unwrap(), 0);
        assert_eq!(row.try_get::<Option<i64>>("", "not_before").unwrap(), None);
        use crate::executions::lease::{Execution, Kind, Lease};
        let execution = Execution {
            kind: Kind::Pipeline,
            workspace: 11,
            id: 7,
        };
        // A migration never assigns legacy in-flight work to this new process.
        assert!(
            Lease::claim(&db, execution, 0, "new:boot", 500)
                .await
                .unwrap()
                .is_none()
        );
        db.execute_unprepared("UPDATE pipeline_runs SET state='queued'")
            .await
            .unwrap();
        let lease = Lease::claim(&db, execution, 0, "new:boot", 500)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(lease.epoch(), 1);
        ExecutionLeases.up(&manager).await.unwrap();
        assert!(lease.renew(&db, 501).await.unwrap());
        // Automatic rollback cannot erase the epoch needed to fence old writers.
        assert!(ExecutionLeases.down(&manager).await.is_err());
        assert!(lease.renew(&db, 502).await.unwrap());
    }
}
