use super::*;
use crate::entities::{
    ai_agents as agents, ai_model_profiles as profiles, definition_revisions as revisions,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
    QuerySelect,
};
use serde_json::json;

pub struct Migration;
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "ai_agent_model_profiles_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m.has_column("ai_agents", "model_profile_id").await? {
            m.alter_table(
                Table::alter()
                    .table(Alias::new("ai_agents"))
                    .add_column(
                        ColumnDef::new(Alias::new("model_profile_id"))
                            .big_integer()
                            .not_null()
                            .default(0i64),
                    )
                    .to_owned(),
            )
            .await?;
        }
        let conn = m.get_connection();
        let mut after = 0;
        loop {
            let rows = agents::Entity::find()
                .filter(agents::Column::Id.gt(after))
                .order_by_asc(agents::Column::Id)
                .limit(100)
                .all(conn)
                .await?;
            if rows.is_empty() {
                break;
            }
            for agent in rows {
                after = agent.id;
                if agent.model_profile_id != 0 {
                    continue;
                }
                let old = profiles::Entity::find()
                    .filter(profiles::Column::WorkspaceId.eq(agent.workspace_id))
                    .filter(profiles::Column::Kind.eq("cli"))
                    .filter(profiles::Column::Runtime.eq(&agent.runtime))
                    .filter(profiles::Column::DefaultModel.eq(&agent.model))
                    .filter(profiles::Column::ProviderId.eq(0i64))
                    .filter(profiles::Column::State.eq("enabled"))
                    .order_by_asc(profiles::Column::Id)
                    .one(conn)
                    .await?;
                let profile = if let Some(profile) = old {
                    profile
                } else {
                    let name = if agent.runtime == "claude" {
                        "Claude Code"
                    } else {
                        "Codex"
                    };
                    let models = if agent.model.is_empty() {
                        vec![]
                    } else {
                        vec![agent.model.clone()]
                    };
                    let definition=json!({"name":name,"kind":"cli","runtime":agent.runtime,"provider_id":"0","models":models,"default_model":agent.model,"tool_version":"","state":"enabled"}).to_string();
                    // Agent IDs have never been revision IDs; using the original issued
                    // ID gives resumable migration stable identities without a worker lease.
                    if revisions::Entity::find_by_id(agent.id)
                        .one(conn)
                        .await?
                        .is_none()
                    {
                        revisions::Entity::insert(
                            revisions::Model {
                                id: agent.id,
                                workspace_id: agent.workspace_id,
                                subject_kind: "model".into(),
                                subject_id: agent.id,
                                number: 1,
                                definition,
                                message: None,
                                created_by: agent.owner_id,
                                created_at: agent.created_at,
                                version: 1,
                            }
                            .into_active_model(),
                        )
                        .exec(conn)
                        .await?;
                    }
                    let row = profiles::Model {
                        id: agent.id,
                        workspace_id: agent.workspace_id,
                        name: name.into(),
                        kind: "cli".into(),
                        runtime: agent.runtime.clone(),
                        provider_id: 0,
                        models: serde_json::to_string(&models).unwrap(),
                        default_model: agent.model.clone(),
                        tool_version: String::new(),
                        state: "enabled".into(),
                        revision_id: agent.id,
                        version: 1,
                        created_at: agent.created_at,
                        updated_at: agent.updated_at,
                    };
                    profiles::Entity::insert(row.clone().into_active_model())
                        .exec(conn)
                        .await?;
                    row
                };
                let mut row = agent.into_active_model();
                row.model_profile_id = sea_orm::Set(profile.id);
                row.update(conn).await?;
            }
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
