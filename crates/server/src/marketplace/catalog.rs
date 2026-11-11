//! Workspace market: own entries plus enabled platform entries; install copies a package into the workspace.
use super::platform::PLATFORM;
use crate::{
    accounts::workspace::Scope,
    entities::{app_templates, config_templates, marketplace_entries as entries, marketplace_sources as sources},
    error::{ApiError, Json, Path, Query, Result},
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{extract::State, http::StatusCode};
use sea_orm::{ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter, sea_query::Expr};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

const ENTRIES: Spec = Spec {
    search: &["name", "description", "search_text"],
    filters: &["source_id", "package_version"],
    sortable: &["name", "updated_at"],
    default_sort: "name",
};
const MAX_NAME_SUFFIX: usize = 20;

struct Visible {
    condition: Condition,
    sources: HashMap<i64, (String, String)>,
}

/// Entries a workspace may see: its own sources and enabled platform sources, optionally narrowed by trust.
async fn visible(conn: &impl ConnectionTrait, scope: &Scope, trust: Option<&[String]>) -> Result<Visible> {
    let workspace = i64::from(scope.workspace_id);
    let rows = sources::Entity::find()
        .filter(
            Condition::any()
                .add(sources::Column::WorkspaceId.eq(workspace))
                .add(Condition::all().add(sources::Column::WorkspaceId.eq(PLATFORM)).add(sources::Column::Enabled.eq(1))),
        )
        .all(conn)
        .await?;
    let sources: HashMap<i64, (String, String)> = rows
        .into_iter()
        .filter(|r| trust.is_none_or(|t| t.contains(&r.trust)))
        .map(|r| (r.id, (r.name, r.trust)))
        .collect();
    let platform: Vec<i64> = sources.iter().filter(|(_, (_, t))| t != "workspace").map(|(id, _)| *id).collect();
    let mut condition = Condition::any().add(Condition::all().add(entries::Column::WorkspaceId.eq(PLATFORM)).add(entries::Column::SourceId.is_in(platform)));
    if trust.is_none_or(|t| t.iter().any(|v| v == "workspace")) {
        condition = condition.add(entries::Column::WorkspaceId.eq(workspace));
    }
    Ok(Visible { condition, sources })
}

fn localize(row: &entries::Model, meta: &Value) -> (String, String) {
    let locale = crate::i18n::current_locale().tag();
    let name = super::index::localized(&meta["name"], locale).unwrap_or_else(|| row.name.clone());
    let description = super::index::localized(&meta["description"], locale).unwrap_or_else(|| row.description.clone());
    (name, description)
}

fn entry_json(row: entries::Model, sources: &HashMap<i64, (String, String)>, detail: bool) -> Value {
    let mut meta: Value = row.meta.as_deref().and_then(|m| serde_json::from_str(m).ok()).unwrap_or_else(|| json!({}));
    let files: Value = row.files.as_deref().and_then(|f| serde_json::from_str(f).ok()).unwrap_or_else(|| json!([]));
    let file_names: Vec<Value> = files.as_array().map(|a| a.iter().map(|f| f["name"].clone()).collect()).unwrap_or_default();
    let (name, description) = localize(&row, &meta);
    let (source_name, trust) = sources.get(&row.source_id).cloned().unwrap_or_default();
    let reason = meta["incompatible"].as_str().map(|key| crate::i18n::render(crate::i18n::current_locale(), &crate::i18n::Message::new(key)));
    if !detail {
        if let Some(object) = meta.as_object_mut() {
            object.remove("icon");
        }
    }
    let mut value = json!({
        "id": Id::from(row.id), "source_id": Id::from(row.source_id), "source_name": source_name, "trust": trust,
        "entry_key": row.entry_key, "name": name, "description": description, "version": row.version,
        "updated_at": row.updated_at, "package_version": row.package_version, "kind": row.kind,
        "compatible": row.compatible != 0, "incompatible_reason": reason, "meta": meta, "files": file_names,
    });
    if detail {
        value["content"] = json!(row.content);
        value["parameters"] = json!(row.parameters);
        value["readme"] = json!(row.readme.unwrap_or_default());
    }
    value
}

pub async fn list(State(s): State<AppState>, scope: Scope, Query(mut raw): Query<HashMap<String, String>>) -> Result<Json<Value>> {
    scope.require("marketplace.read")?;
    let trust: Option<Vec<String>> = raw.remove("trust").map(|t| t.split(',').map(str::to_owned).collect());
    let q = ListQuery::parse(&ENTRIES, &raw)?;
    let visible = visible(&s.db, &scope, trust.as_deref()).await?;
    let (rows, total) = resource::list_with::<entries::Entity, entries::Model>(&s.db, &ENTRIES, entries::Entity::find(), visible.condition.clone(), &q).await?;
    let items: Vec<Value> = rows.into_iter().map(|r| entry_json(r, &visible.sources, false)).collect();
    Ok(Json(json!({"items":items,"total":total,"page":q.page,"page_size":q.page_size})))
}

async fn visible_entry(conn: &impl ConnectionTrait, scope: &Scope, id: Id) -> Result<(entries::Model, Visible)> {
    let visible = visible(conn, scope, None).await?;
    let row = entries::Entity::find_by_id(i64::from(id)).filter(visible.condition.clone()).one(conn).await?.ok_or_else(ApiError::missing)?;
    Ok((row, visible))
}

pub async fn get(State(s): State<AppState>, scope: Scope, Path((_, id)): Path<(String, Id)>) -> Result<Json<Value>> {
    scope.require("marketplace.read")?;
    let (row, visible) = visible_entry(&s.db, &scope, id).await?;
    Ok(Json(entry_json(row, &visible.sources, true)))
}

#[derive(Deserialize)]
pub struct Install {
    pub(super) version: i64,
}
#[derive(Deserialize)]
struct PackageFile {
    name: String,
    content: String,
    #[serde(default)]
    parameters: Value,
}

fn bad(status: StatusCode, key: &str) -> ApiError {
    ApiError::localized(status, crate::i18n::Message::new(key))
}

/// First free name among `name`, `name-2` … `name-20` in this workspace.
async fn free_config_name(conn: &impl ConnectionTrait, workspace: Id, name: &str) -> Result<String> {
    let candidates: Vec<String> = std::iter::once(name.to_owned()).chain((2..=MAX_NAME_SUFFIX).map(|n| format!("{name}-{n}"))).collect();
    let taken: HashSet<String> = config_templates::Entity::find()
        .filter(config_templates::Column::WorkspaceId.eq(workspace))
        .filter(config_templates::Column::Name.is_in(candidates.clone()))
        .all(conn)
        .await?
        .into_iter()
        .map(|r| r.name)
        .collect();
    candidates.into_iter().find(|c| !taken.contains(c)).ok_or_else(|| bad(StatusCode::CONFLICT, "marketplace.configNameExhausted"))
}

/// Points `fromTemplate` references that name a package file at the imported config template ID.
fn rewrite_references(content: &mut Value, imported: &HashMap<String, String>) {
    if let Some(services) = content.get_mut("services").and_then(Value::as_array_mut) {
        for service in services {
            if let Some(files) = service.get_mut("files").and_then(Value::as_array_mut) {
                for file in files {
                    if let Some(id) = file.get("fromTemplate").and_then(Value::as_str).and_then(|n| imported.get(n)) {
                        file["fromTemplate"] = json!(id);
                    }
                }
            }
        }
    }
}

pub async fn install(State(s): State<AppState>, scope: Scope, Path((_, id)): Path<(String, Id)>, Json(b): Json<Install>) -> Result<(StatusCode, Json<Value>)> {
    scope.require("marketplace.install")?;
    let (row, _) = visible_entry(&s.db, &scope, id).await?;
    if row.version != b.version {
        return Err(bad(StatusCode::CONFLICT, "marketplace.catalogChanged"));
    }
    if row.compatible == 0 {
        return Err(bad(StatusCode::BAD_REQUEST, "marketplace.incompatible"));
    }
    let corrupt = || bad(StatusCode::BAD_REQUEST, "marketplace.templateCorrupt");
    let files: Vec<PackageFile> = serde_json::from_str(row.files.as_deref().unwrap_or("[]")).map_err(|_| corrupt())?;
    if !files.is_empty() {
        scope.require("config-templates.create")?;
    }
    let meta: Value = row.meta.as_deref().and_then(|m| serde_json::from_str(m).ok()).unwrap_or_else(|| json!({}));
    let (name, description) = localize(&row, &meta);
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut imported = HashMap::new();
    for file in files {
        let config_name = free_config_name(&tx, scope.workspace_id, &file.name).await?;
        let parameters = if file.parameters.is_null() { "[]".to_owned() } else { file.parameters.to_string() };
        let body = serde_json::from_value(json!({"name":config_name,"description":"","content":file.content,"parameters":parameters})).map_err(|_| corrupt())?;
        let (_, Json(created)) = crate::templates::configs::create_in(&s, &tx, &scope, body).await?;
        let config_id: Id = serde_json::from_value(created["id"].clone()).map_err(|_| corrupt())?;
        config_templates::Entity::update_many()
            .col_expr(config_templates::Column::OriginSourceId, Expr::value(row.source_id))
            .col_expr(config_templates::Column::OriginKey, Expr::value(row.entry_key.clone()))
            .col_expr(config_templates::Column::OriginVersion, Expr::value(row.package_version.clone()))
            .col_expr(config_templates::Column::OriginDigest, Expr::value(row.digest.clone()))
            .filter(config_templates::Column::Id.eq(config_id))
            .exec(&tx)
            .await?;
        imported.insert(file.name, config_id.to_string());
    }
    let mut content: Value = serde_json::from_str(&row.content).map_err(|_| corrupt())?;
    rewrite_references(&mut content, &imported);
    let body = serde_json::from_value(json!({"name":name,"description":description,"content":serde_json::to_string_pretty(&content).unwrap(),"parameters":row.parameters})).map_err(|_| corrupt())?;
    let (status, Json(created)) = crate::templates::apps::create_in(&s, &tx, &scope, body).await?;
    let template_id: Id = serde_json::from_value(created["id"].clone()).map_err(|_| corrupt())?;
    app_templates::Entity::update_many()
        .col_expr(app_templates::Column::OriginSourceId, Expr::value(row.source_id))
        .col_expr(app_templates::Column::OriginKey, Expr::value(row.entry_key.clone()))
        .col_expr(app_templates::Column::OriginVersion, Expr::value(row.package_version.clone()))
        .col_expr(app_templates::Column::OriginDigest, Expr::value(row.digest.clone()))
        .filter(app_templates::Column::Id.eq(template_id))
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(&s, &tx, &scope, "install", "marketplace", &id, json!({"source_id":Id::from(row.source_id),"key":row.entry_key,"version":row.package_version,"template_id":template_id})).await?;
    tx.commit().await?;
    Ok((status, Json(created)))
}
