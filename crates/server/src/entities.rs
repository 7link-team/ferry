//! Table mirrors. HTTP responses explicitly wrap identifiers in `Id`.

pub mod platform_state {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "platform_state")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub initialized: i64,
        pub registration_enabled: i64,
        pub repository_oauth: Option<String>,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod users {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "users")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub email: String,
        pub name: String,
        pub password_hash: String,
        pub is_superadmin: i64,
        pub version: i64,
        pub active: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod sessions {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "sessions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub token_hash: String,
        pub user_id: i64,
        pub workspace_id: i64,
        pub created_at: i64,
        pub seen_at: i64,
        pub expires_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod workspaces {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "workspaces")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub slug: String,
        pub name: String,
        pub owner_id: i64,
        pub join_code: String,
        pub pipeline_run_number: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod workspace_roles {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "workspace_roles")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub permissions: String,
        pub builtin: i64,
        pub created_at: i64,
        pub updated_at: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod workspace_members {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "workspace_members")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub user_id: i64,
        pub role_id: i64,
        pub state: String,
        pub created_at: i64,
        pub updated_at: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod activity {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "activity")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub actor_id: i64,
        pub actor_name: String,
        pub action: String,
        pub resource: String,
        pub resource_id: i64,
        pub details: String,
        pub created_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod host_groups {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "host_groups")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub description: String,
        pub parent_id: i64,
        pub created_at: i64,
        pub updated_at: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod hosts {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "hosts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub address: String,
        pub port: i64,
        pub username: String,
        pub connection: String,
        pub state: String,
        pub group_id: i64,
        pub description: String,
        pub host_key: String,
        pub runner_key_hash: String,
        pub seen_at: i64,
        pub created_at: i64,
        pub updated_at: i64,
        pub version: i64,
        pub os: String,
        pub arch: String,
        pub agent_version: String,
        pub metrics: String,
        pub credential_id: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod tags {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "tags")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub color: String,
        pub created_at: i64,
        pub updated_at: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod host_tags {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "host_tags")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub host_id: i64,
        #[sea_orm(primary_key, auto_increment = false)]
        pub tag_id: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod runner_tokens {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "runner_tokens")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub host_id: i64,
        pub token_hash: String,
        pub expires_at: i64,
        pub used_at: i64,
        pub used_by: i64,
        pub created_by: i64,
        pub created_at: i64,
        pub updated_at: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod credentials {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "credentials")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub kind: String,
        pub username: String,
        pub description: String,
        pub secret: String,
        pub created_at: i64,
        pub updated_at: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod repositories {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "repositories")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub provider: String,
        pub url: String,
        pub default_branch: String,
        pub credential_id: i64,
        pub description: String,
        pub created_at: i64,
        pub updated_at: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod pipelines {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "pipelines")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub group_id: i64,
        pub application_id: i64,
        pub repository_id: i64,
        pub definition: String,
        pub target_kind: String,
        pub target_ref: i64,
        pub description: String,
        pub created_at: i64,
        pub updated_at: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod locks {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "locks")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub name: String,
        pub owner: String,
        pub acquired_at: i64,
        pub expires_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod worker_ids {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "worker_ids")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub worker_id: i64,
        pub instance_id: String,
        pub heartbeat_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod pipeline_runs {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "pipeline_runs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub pipeline_id: i64,
        pub environment_id: i64,
        pub environment_name: String,
        pub host_id: i64,
        pub number: i64,
        pub state: String,
        pub trigger_kind: String,
        pub triggered_by: i64,
        pub definition: String,
        pub variable_snapshot: Option<String>,
        pub repo_url: String,
        pub branch: String,
        pub commit: String,
        pub credential_id: i64,
        pub created_at: i64,
        pub started_at: i64,
        pub finished_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod pipeline_run_steps {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "pipeline_run_steps")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub run_id: i64,
        pub step_index: i64,
        pub name: String,
        pub state: String,
        pub exit_code: Option<i32>,
        pub started_at: i64,
        pub finished_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod pipeline_run_logs {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "pipeline_run_logs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub run_id: i64,
        pub seq: i64,
        pub step_index: i64,
        pub stream: String,
        pub chunk: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod deployments {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "deployments")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub manifest: String,
        pub preview_artifact: String,
        pub mode: String,
        pub target_kind: String,
        pub target_ref: i64,
        pub workdir: String,
        pub description: String,
        pub state: String,
        pub revision_number: i64,
        pub current_revision: i64,
        pub created_at: i64,
        pub updated_at: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod deployment_revisions {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "deployment_revisions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub deployment_id: i64,
        pub number: i64,
        pub action: String,
        pub source_revision: i64,
        pub manifest: String,
        pub rendered: String,
        pub artifact: String,
        pub variable_snapshot: Option<String>,
        pub expected_hash: String,
        pub state: String,
        pub output: String,
        pub triggered_by: i64,
        pub started_at: i64,
        pub finished_at: i64,
        pub created_at: i64,
        pub updated_at: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod variables {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "variables")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub scope: String,
        pub scope_ref: i64,
        pub kind: String,
        pub description: String,
        pub value: String,
        pub created_at: i64,
        pub updated_at: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod app_templates {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "app_templates")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub description: String,
        pub content: String,
        pub parameters: String,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod pipeline_templates {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "pipeline_templates")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub description: String,
        pub content: String,
        pub parameters: String,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod settings {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "settings")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub key: String,
        pub value: i64,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod dictionaries {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "dictionaries")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub key: String,
        pub name: String,
        pub description: String,
        pub builtin: i64,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod dictionary_items {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "dictionary_items")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub dictionary_id: i64,
        pub value: String,
        pub label: String,
        pub position: i64,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod cron_jobs {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "cron_jobs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub description: String,
        pub scope: String,
        pub config: String,
        pub target_kind: String,
        pub target_ref: i64,
        pub enabled: i64,
        pub run_as: i64,
        pub run_as_name: String,
        pub next_run_at: i64,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod cron_runs {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "cron_runs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub job_id: i64,
        pub job_name: String,
        pub scope: String,
        pub config: String,
        pub target_kind: String,
        pub target_ref: i64,
        pub state: String,
        pub trigger_kind: String,
        pub triggered_by: i64,
        pub scheduled_at: i64,
        pub started_at: i64,
        pub finished_at: i64,
        pub attempt: i64,
        pub output: String,
        pub downstream_id: i64,
        pub downstream_kind: String,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod alert_rules {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "alert_rules")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub description: String,
        pub target_kind: String,
        pub target_ref: i64,
        pub metric: String,
        pub threshold: i64,
        pub enabled: i64,
        pub run_as: i64,
        pub webhook: String,
        pub last_error: String,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod alert_events {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "alert_events")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub rule_id: i64,
        pub rule_name: String,
        pub target_kind: String,
        pub target_ref: i64,
        pub state: String,
        pub message: String,
        pub triggered_at: i64,
        pub recovered_at: i64,
        pub trigger_delivery: String,
        pub recovery_delivery: String,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod marketplace_sources {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "marketplace_sources")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub repository_id: i64,
        pub config: String,
        pub state: String,
        pub last_error: String,
        pub synced_at: i64,
        pub run_as: i64,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod marketplace_entries {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "marketplace_entries")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub source_id: i64,
        pub entry_key: String,
        pub name: String,
        pub description: String,
        pub content: String,
        pub parameters: String,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod repository_oauth_flows {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "repository_oauth_flows")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub user_id: i64,
        pub provider: String,
        pub state_hash: String,
        pub proof: String,
        pub expires_at: i64,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod repository_connections {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "repository_connections")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub user_id: i64,
        pub provider: String,
        pub credential_id: i64,
        pub account: String,
        pub refresh_secret: String,
        pub expires_at: i64,
        pub state: String,
        pub last_error: String,
        pub client_id: String,
        pub config_fingerprint: String,
        pub token_config: Option<String>,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod pipeline_artifacts {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "pipeline_artifacts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub pipeline_id: i64,
        pub run_id: i64,
        pub path: String,
        pub sha256: String,
        pub size: i64,
        pub mode: i64,
        pub created_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod config_templates {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "config_templates")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub description: String,
        pub content: String,
        pub parameters: String,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod deployment_artifact_refs {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "deployment_artifact_refs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub revision_id: i64,
        pub artifact_id: i64,
        pub run_id: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod deployment_revision_targets {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "deployment_revision_targets")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub revision_id: i64,
        pub host_id: i64,
        pub position: i64,
        pub artifact: String,
        pub expected_hash: String,
        pub state: String,
        pub output: String,
        pub started_at: i64,
        pub finished_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod deployment_target_states {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "deployment_target_states")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub deployment_id: i64,
        pub host_id: i64,
        pub current_revision: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

/// Encrypted, actor-bound pending imports. Consumed rows retain only result IDs.
pub mod resource_imports {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "resource_imports")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub actor_id: i64,
        pub resource: String,
        pub payload: String,
        pub result: String,
        pub created_at: i64,
        pub committed_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod pipeline_run_jobs {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "pipeline_run_jobs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub run_id: i64,
        pub job_key: String,
        pub host_id: i64,
        pub state: String,
        pub outputs: Option<String>,
        pub started_at: i64,
        pub finished_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod pipeline_contexts {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "pipeline_contexts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub kind: String,
        pub name: String,
        pub description: String,
        pub target_kind: String,
        pub target_ref: i64,
        pub created_at: i64,
        pub updated_at: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod pipeline_links {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "pipeline_links")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub pipeline_id: i64,
        #[sea_orm(primary_key, auto_increment = false)]
        pub kind: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub reference_id: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
