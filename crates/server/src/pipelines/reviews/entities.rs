pub mod definitions {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "code_reviews")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub user_id: i64,
        pub name: String,
        pub description: String,
        pub config: String,
        pub pipeline_id: i64,
        pub version: i64,
        pub cursor: i64,
        pub baselines: String,
        pub created_at: i64,
        pub updated_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod jobs {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "code_review_runs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub review_id: i64,
        pub dedupe: String,
        pub state: String,
        pub snapshot: String,
        pub result: String,
        pub pipeline_run_id: i64,
        pub lease_until: i64,
        pub lease_owner: String,
        pub created_at: i64,
        pub finished_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
pub mod findings {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "code_review_findings")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub workspace_id: i64,
        pub review_id: i64,
        pub connection_id: i64,
        pub repository: String,
        pub repository_url: String,
        pub fingerprint: String,
        pub file: String,
        pub detail: String,
        pub status: String,
        pub history: String,
        pub first_seen: i64,
        pub last_seen: i64,
        pub version: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
