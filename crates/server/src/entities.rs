//! Table mirrors. HTTP responses explicitly wrap identifiers in `Id`.

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

// The schema includes this junction table, but no current endpoint reads or
// writes tag associations. Keep its required table mirror without inventing a caller.
#[expect(dead_code, reason = "host_tags has no data-access caller yet")]
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
        pub host_id: i64,
        pub number: i64,
        pub state: String,
        pub trigger_kind: String,
        pub triggered_by: i64,
        pub definition: String,
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
