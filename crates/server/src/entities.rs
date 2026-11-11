//! Table mirrors. HTTP responses explicitly wrap identifiers in `Id`.

pub mod ai_model_profiles {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "ai_model_profiles")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub kind: String,
        pub runtime: String,
        pub provider_id: i64,
        pub models: String,
        pub default_model: String,
        pub tool_version: String,
        pub state: String,
        pub revision_id: i64,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod ai_chat_members {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "ai_chat_members")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub conversation_id: i64,
        pub user_id: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod ai_chat_reads {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "ai_chat_reads")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub conversation_id: i64,
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: i64,
        pub workspace_id: i64,
        pub message_id: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod ai_chat_saves {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "ai_chat_saves")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub message_id: i64,
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: i64,
        pub workspace_id: i64,
        pub conversation_id: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod ai_chat_agents {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "ai_chat_agents")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub conversation_id: i64,
        pub agent_id: i64,
        pub revision_id: i64,
        pub config: String,
        pub binding: String,
        pub native_session_id: String,
        pub state: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod ai_chat_messages {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "ai_chat_messages")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub conversation_id: i64,
        pub owner_id: i64,
        pub created_at: i64,
        pub client_id: String,
        pub prompt: String,
        pub attachments: String,
        pub targets: String,
        pub role: String,
        pub agent_id: i64,
        pub turn_id: Option<i64>,
        pub reply_to: Option<i64>,
        pub sender_name: String,
        pub text_items: Option<String>,
        pub version: i64,
        pub edited_at: Option<i64>,
        pub retracted_at: Option<i64>,
        pub mentions: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod ai_chat_turns {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "ai_chat_turns")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub conversation_id: i64,
        pub message_id: i64,
        pub chat_agent_id: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod ai_chat_files {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "ai_chat_files")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub conversation_id: i64,
        pub owner_id: i64,
        pub asset_id: i64,
        pub size: i64,
        pub created_at: i64,
        pub name: String,
        pub mime: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod ai_conversations {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "ai_conversations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub owner_id: i64,
        pub agent_id: i64,
        pub revision_id: i64,
        pub next_sequence: i64,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
        pub title: String,
        pub state: String,
        pub config: String,
        pub binding: String,
        pub native_session_id: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod ai_turns {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "ai_turns")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub conversation_id: i64,
        pub owner_id: i64,
        pub expires_at: i64,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
        pub state: String,
        pub lease_owner: String,
        pub client_id: String,
        pub prompt: String,
        pub attachments: String,
        pub native_session_id: String,
        pub candidate_session: String,
        pub error: String,
        pub usage: String,
        pub decisions: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod ai_events {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "ai_events")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub conversation_id: i64,
        pub turn_id: i64,
        pub sequence: i64,
        pub created_at: i64,
        pub payload: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod holes {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "holes")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub host_id: i64,
        pub name: String,
        pub target_port: i64,
        pub strip_prefix: bool,
        pub token_hash: String,
        pub token_sealed: String,
        pub expires_at: i64,
        pub revoked_at: i64,
        pub revoked_by: i64,
        pub probe_state: String,
        pub probe_error: String,
        pub probed_at: i64,
        pub probed_by: i64,
        pub created_at: i64,
        pub created_by: i64,
        pub updated_at: i64,
        pub updated_by: i64,
        pub version: i64,
        pub source_kind: String,
        pub source_id: i64,
        pub source_ref: String,
        pub review_state: String,
        pub reviewed_at: i64,
        pub reviewed_by: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod ai_agents {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "ai_agents")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub model_profile_id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub avatar: Option<String>,
        pub role: String,
        pub description: String,
        pub runtime: String,
        pub model: String,
        pub host_id: i64,
        pub owner_id: i64,
        pub cwd: String,
        pub system_prompt: String,
        pub permission: String,
        pub visibility: String,
        pub state: String,
        pub revision_id: i64,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod platform_state {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "platform_state")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub initialized: i64,
        pub registration_enabled: i64,
        pub runner_auto_update: i64,
        pub repository_oauth: Option<String>,
        pub access_config: Option<String>,
        pub file_storage: Option<String>,
        pub footer_html: Option<String>,
        pub page_scripts: Option<String>,
        pub site_config: Option<String>,
        pub mail_config: Option<String>,
        pub monitor_config: Option<String>,
        pub login_security: Option<String>,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod login_guards {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "login_guards")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub count: i64,
        pub payload: Option<String>,
        pub expires_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod email_codes {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "email_codes")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub code_hash: String,
        pub count: i64,
        pub expires_at: i64,
        pub updated_at: i64,
        pub config_tag: String,
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
        pub last_login_at: Option<i64>,
        pub last_login_ip: Option<String>,
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

pub mod user_otp {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "user_otp")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: i64,
        pub payload: String,
        pub created_at: i64,
        pub created_by: i64,
        pub updated_at: i64,
        pub updated_by: i64,
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
        pub workflow_execution_number: i64,
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
        pub position: i64,
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
        pub enroll_token: String,
        pub enroll_token_hash: String,
        pub jump_host_id: i64,
        pub seen_at: i64,
        pub created_at: i64,
        pub updated_at: i64,
        pub version: i64,
        pub os: String,
        pub arch: String,
        pub agent_version: String,
        pub agent_update: Option<String>,
        pub runner_auto_update: String,
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
        pub scope: String,
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

pub mod definition_acl {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "definition_acl")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub subject_kind: String,
        pub subject_id: i64,
        pub user_id: i64,
        pub role: String,
        pub created_at: i64,
        pub created_by: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod workflows {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "workflows")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub group_id: i64,
        pub description: String,
        pub definition: String,
        pub draft_revision_id: i64,
        pub published_revision_id: Option<i64>,
        pub settings: String,
        pub owner_id: Option<i64>,
        pub trigger_run_as: Option<i64>,
        pub lifecycle: String,
        pub archived_at: Option<i64>,
        pub archived_by: i64,
        pub created_by: i64,
        pub created_at: i64,
        pub updated_at: i64,
        pub updated_by: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod workflow_executions {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "workflow_executions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub workflow_id: i64,
        pub revision_id: i64,
        pub number: i64,
        pub mode: String,
        pub state: String,
        pub trigger_kind: String,
        pub trigger_ref: String,
        pub triggered_by: i64,
        pub parent_execution_id: i64,
        pub retry_of_id: i64,
        pub root_execution_id: i64,
        pub cost_micros: i64,
        pub data_bytes: i64,
        pub created_at: i64,
        pub created_by: i64,
        pub started_at: i64,
        pub started_by: i64,
        pub finished_at: i64,
        pub finished_by: i64,
        pub lease_owner: String,
        pub lease_epoch: i64,
        pub priority: i64,
        pub not_before: Option<i64>,
        pub end_reason: String,
        pub concurrency_key: String,
        pub concurrency_limit: i64,
        pub queue_reason: String,
        pub context: Option<String>,
        pub waiting_ms: i64,
        pub response_state: String,
        pub response_deadline_at: Option<i64>,
        pub response_deadline_by: i64,
        pub response_delivered_at: Option<i64>,
        pub response_delivered_by: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod workflow_integration_calls {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "workflow_integration_calls")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub execution_id: i64,
        pub node_run_id: i64,
        pub item_index: i64,
        pub credential_id: i64,
        pub request_hash: String,
        pub state: String,
        pub result: Option<String>,
        pub created_at: i64,
        pub created_by: i64,
        pub updated_at: i64,
        pub updated_by: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod workflow_node_runs {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "workflow_node_runs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub execution_id: i64,
        pub node_id: String,
        pub iteration_path: String,
        pub iteration_hash: String,
        pub attempt: i64,
        pub state: String,
        pub items_in: i64,
        pub items_out: i64,
        pub output: String,
        pub output_asset: Option<i64>,
        pub logs: Option<String>,
        pub logs_truncated: bool,
        pub logs_updated_at: i64,
        pub error: String,
        pub cache_key: String,
        pub cache_hit_from: Option<i64>,
        pub cost_micros: i64,
        pub started_at: i64,
        pub started_by: i64,
        pub finished_at: i64,
        pub finished_by: i64,
        pub resolved_params: Option<String>,
        pub seed: Option<i64>,
        pub redo_round: i64,
        pub error_policy: String,
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
        pub owner_id: Option<i64>,
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
        pub info: Option<String>,
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
        pub revision_id: i64,
        pub environment_id: i64,
        pub environment_name: String,
        pub host_id: i64,
        /// Names when the run started; "" for runs recorded before snapshots.
        pub pipeline_name: String,
        pub host_name: String,
        pub triggered_by_name: String,
        /// workflow_dispatch inputs as JSON; None when the run had none.
        pub inputs: Option<String>,
        /// Evaluated GitHub concurrency group; "" when the workflow has none.
        pub concurrency_group: String,
        pub lease_owner: String,
        pub lease_epoch: i64,
        pub not_before: Option<i64>,
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

pub mod definition_publications {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "definition_publications")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub subject_kind: String,
        pub subject_id: i64,
        pub revision_id: i64,
        pub name: String,
        pub name_key: String,
        pub change_note: Option<String>,
        pub run_as: i64,
        pub request_key: Option<String>,
        pub created_at: i64,
        pub created_by: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod definition_revisions {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "definition_revisions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub subject_kind: String,
        pub subject_id: i64,
        pub number: i64,
        pub definition: String,
        pub message: Option<String>,
        pub created_by: i64,
        pub created_at: i64,
        pub version: i64,
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
        pub category: String,
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
        pub origin_source_id: i64,
        pub origin_key: String,
        pub origin_version: String,
        pub origin_digest: String,
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
        pub category: String,
        pub value_type: String,
        pub value_json: Option<String>,
        pub schema_json: Option<String>,
        pub value: i64,
        pub name: String,
        pub description: String,
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
        pub category: String,
        pub value_type: String,
        pub value_json: Option<String>,
        pub schema_json: Option<String>,
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
        pub notification_channel_id: i64,
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
        pub trust: String,
        pub enabled: i64,
        pub sync_interval: i64,
        pub next_sync_at: i64,
        pub sync_owner: String,
        pub sync_lease_until: i64,
        pub created_by: i64,
        pub updated_by: i64,
        pub synced_by: i64,
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
        pub package_version: String,
        pub kind: String,
        pub meta: Option<String>,
        pub files: Option<String>,
        pub readme: Option<String>,
        pub compatible: i64,
        pub digest: String,
        pub created_by: i64,
        pub updated_by: i64,
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
        pub name: String,
        pub description: String,
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

pub mod repository_monitors {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "repository_monitors")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub config: String,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod repository_poll_states {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "repository_poll_states")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub config_version: i64,
        pub snapshot: String,
        pub last_polled_at: i64,
        pub next_poll_at: i64,
        pub last_error: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod repository_events {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "repository_events")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub connection_id: i64,
        pub delivery_key: String,
        pub event: String,
        pub repository: String,
        pub reference: String,
        pub actor: String,
        pub action: String,
        pub delivery: String,
        pub created_at: i64,
        /// Structured detail JSON; absent for events recorded before it existed.
        pub detail: Option<String>,
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
        pub expires_at: i64,
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
        pub origin_source_id: i64,
        pub origin_key: String,
        pub origin_version: String,
        pub origin_digest: String,
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
        /// `<workspace>:<group>` of the job's GitHub concurrency; "" for none.
        pub concurrency_group: String,
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
        pub position: i64,
        pub target_kind: String,
        pub target_ref: i64,
        /// Environments only: 1 when jobs declaring it wait for approval.
        pub requires_approval: i64,
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

pub mod repository_analysis_configs {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "repository_analysis_configs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub config: String,
        pub coverage_start: i64,
        pub next_reconcile_at: i64,
        pub last_error: String,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod repository_analysis_repositories {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "repository_analysis_repositories")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub identity: String,
        pub provider: String,
        pub path: String,
        pub url: String,
        pub created_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod repository_analysis_sources {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "repository_analysis_sources")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub connection_id: i64,
        pub repository_id: i64,
        pub remote_path: String,
        pub coverage_start: i64,
        pub config_version: i64,
        pub state: String,
        pub last_error: String,
        pub synced_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod repository_analysis_refs {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "repository_analysis_refs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub source_id: i64,
        pub name: String,
        pub head: String,
        pub present: i64,
        pub generation: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod repository_analysis_commits {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "repository_analysis_commits")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub repository_id: i64,
        pub sha: String,
        pub parents: String,
        pub message: String,
        pub message_type: String,
        pub author_name: String,
        pub author_email: String,
        pub author_date: i64,
        pub committer_date: i64,
        pub is_merge: i64,
        pub additions: i64,
        pub deletions: i64,
        pub effectives: i64,
        pub language_stats: String,
        pub analysis_version: i64,
        pub complete: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod repository_analysis_ref_commits {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "repository_analysis_ref_commits")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub source_id: i64,
        pub ref_id: i64,
        pub commit_id: i64,
        pub generation: i64,
        pub current: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod repository_analysis_jobs {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "repository_analysis_jobs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub generation: i64,
        pub completed_generation: i64,
        pub state: String,
        pub cursor: String,
        pub retry_at: i64,
        pub lease_owner: String,
        pub lease_until: i64,
        pub last_error: String,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod repository_analysis_people {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "repository_analysis_people")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub user_id: i64,
        pub display_name: String,
        pub version: i64,
        pub raw_identity_id: Option<i64>,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod repository_analysis_identities {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "repository_analysis_identities")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub fingerprint: String,
        pub raw_name: String,
        pub raw_email: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod repository_analysis_commit_authors {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "repository_analysis_commit_authors")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub commit_id: i64,
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: i64,
        pub workspace_id: i64,
        pub repository_id: i64,
        pub identity_id: i64,
        pub contributor_id: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod code_insights {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "code_insights")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub user_id: i64,
        pub name: String,
        pub description: String,
        pub config: String,
        pub coverage_start: i64,
        pub next_run_at: i64,
        pub version: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod code_insight_repositories {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "code_insight_repositories")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub insight_id: i64,
        pub connection_id: i64,
        pub repository: String,
        pub all_repositories: i64,
        pub pipeline_id: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod repository_analysis_commit_identities {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "repository_analysis_commit_identities")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub commit_id: i64,
        pub workspace_id: i64,
        pub identity_id: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod code_insight_people {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "code_insight_people")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub insight_id: i64,
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub display_name: String,
        pub version: i64,
        pub raw_identity_id: Option<i64>,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod code_insight_identity_links {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "code_insight_identity_links")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub insight_id: i64,
        #[sea_orm(primary_key, auto_increment = false)]
        pub identity_id: i64,
        pub workspace_id: i64,
        pub contributor_id: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

/// Execution policy is independent of transport/enrolment state.
pub mod host_execution_policies {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "host_execution_policies")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub host_id: i64,
        pub workspace_id: i64,
        pub enabled: bool,
        pub shared: bool,
        pub capacity: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod host_execution_slots {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "host_execution_slots")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub host_id: i64,
        #[sea_orm(primary_key, auto_increment = false)]
        pub slot: i64,
        pub owner: String,
        pub run_id: i64,
        pub job_key: String,
        pub expires_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod ai_providers {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "ai_providers")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub credential_id: i64,
        pub base_url: String,
        pub protocol: String,
        pub currency: String,
        pub timezone: String,
        pub monthly_limit_micros: i64,
        pub qps: i64,
        pub concurrency: i64,
        pub prices: String,
        pub created_at: i64,
        pub created_by: i64,
        pub updated_at: i64,
        pub updated_by: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod ai_calls {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "ai_calls")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub provider_id: i64,
        pub credential_id: i64,
        pub execution_id: i64,
        pub node_run_id: i64,
        pub operation_key: String,
        pub currency: String,
        pub period_key: String,
        pub state: String,
        pub reserved_micros: i64,
        pub actual_micros: i64,
        pub price_snapshot: String,
        pub result: Option<String>,
        pub created_at: i64,
        pub created_by: i64,
        pub updated_at: i64,
        pub updated_by: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod platform_observations {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "platform_observations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub at: i64,
        pub event_day: i64,
        pub user_id: i64,
        pub instance_id: String,
        pub kind: String,
        pub method: String,
        pub route: String,
        pub file_path: String,
        pub status: i32,
        pub elapsed_ms: i64,
        pub response_bytes: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod content_assets {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "assets")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub sha256: String,
        pub size: i64,
        pub mime: String,
        pub kind: String,
        pub width: Option<i64>,
        pub height: Option<i64>,
        pub duration_ms: Option<i64>,
        pub thumbnail_asset_id: Option<i64>,
        pub execution_id: Option<i64>,
        pub node_id: Option<String>,
        pub pinned: i64,
        pub created_at: i64,
        pub created_by: i64,
        pub version: i64,
        pub provenance: Option<String>,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod asset_refs {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "asset_refs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub asset_id: i64,
        #[sea_orm(primary_key, auto_increment = false)]
        pub owner_kind: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub owner_id: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod asset_gc_jobs {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "asset_gc_jobs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub asset_id: i64,
        pub storage_key: String,
        pub state: String,
        pub eligible_at: i64,
        pub eligible_by: i64,
        pub attempt: i64,
        pub error: Option<String>,
        pub updated_at: i64,
        pub updated_by: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod notification_channels {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "notification_channels")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub description: String,
        pub provider: String,
        pub enabled: i64,
        pub config: String,
        pub role_ids: String,
        pub version: i64,
        pub created_at: i64,
        pub created_by: i64,
        pub updated_at: i64,
        pub updated_by: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod notification_templates {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "notification_templates")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub name: String,
        pub language: String,
        pub subject: String,
        pub body: String,
        pub format: String,
        pub version: i64,
        pub created_at: i64,
        pub created_by: i64,
        pub updated_at: i64,
        pub updated_by: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod notification_deliveries {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "notification_deliveries")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub channel_id: i64,
        pub template_id: i64,
        pub actor_id: i64,
        pub agent_id: i64,
        pub source: String,
        pub source_id: String,
        pub client_id: String,
        pub request_hash: String,
        pub state: String,
        pub subject: String,
        pub body: String,
        pub result: String,
        pub version: i64,
        pub created_at: i64,
        pub created_by: i64,
        pub updated_at: i64,
        pub updated_by: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
