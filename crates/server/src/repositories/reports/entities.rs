//! Report-owned persistence. Snapshots are append-only; mutable queue state is separate.
pub mod definitions {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "insight_report_definitions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub user_id: i64,
        pub name: String,
        pub insight_id: i64,
        #[sea_orm(column_type = "Text")]
        pub config: String,
        pub enabled: i64,
        pub version: i64,
        pub next_run_at: i64,
        pub last_generated_at: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod runs {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "insight_report_runs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub user_id: i64,
        pub definition_id: i64,
        pub period: String,
        pub period_start: i64,
        pub period_end: i64,
        pub definition_version: i64,
        pub state: String,
        pub trigger_kind: String,
        pub idempotency_key: String,
        pub lease_token: String,
        pub lease_until: i64,
        #[sea_orm(column_type = "Text")]
        pub config_snapshot: String,
        #[sea_orm(column_type = "Text")]
        pub snapshot: String,
        #[sea_orm(column_type = "Text")]
        pub error: String,
        pub created_at: i64,
        pub generated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod bindings {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "application_repository_bindings")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub user_id: i64,
        pub application_id: i64,
        pub connection_id: i64,
        pub repository: String,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
