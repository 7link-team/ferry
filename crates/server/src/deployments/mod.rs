//! Deployments use the shared tenant resource and host channel contracts.
pub(crate) mod manifest;
pub(crate) mod modes;
use crate::i18n::{Message, StoredText};
mod groups;
use crate::error::{Json, Path, Query};
use crate::hosts::channel::HostChannel;
use crate::{
    accounts::workspace::Scope,
    db,
    entities::{deployment_revisions as revisions, deployments as apps, hosts},
    error::{ApiError, Result},
    hosts::remote,
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{extract::State, http::StatusCode};
use sea_orm::{
    ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, QueryTrait,
    TransactionTrait,
    sea_query::{Expr, ExprTrait},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    time::Duration,
};

const SPEC: Spec = Spec {
    search: &["name"],
    filters: &["state", "category"],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "-created_at",
};
const REV_SPEC: Spec = Spec {
    search: &[],
    filters: &["deployment_id", "state"],
    sortable: &["number", "created_at"],
    // Revision numbers are local to each deployment.
    default_sort: "-created_at",
};
fn app_json(r: &apps::Model) -> Value {
    json!({"id":Id::from(r.id),"name":r.name,"category":r.category,"manifest":r.manifest,"mode":r.mode,"target_kind":r.target_kind,
        "target_ref":Id::from(r.target_ref),"workdir":r.workdir,"description":r.description,"state":r.state,
        "revision_number":r.revision_number,"current_revision":Id::from(r.current_revision),
        "created_at":r.created_at,"updated_at":r.updated_at,"version":r.version})
}
fn revision_json(r: &revisions::Model) -> Value {
    json!({"id":Id::from(r.id),"deployment_id":Id::from(r.deployment_id),"number":r.number,
        "action":r.action,"source_revision":Id::from(r.source_revision),"state":r.state,"output":crate::i18n::display_text_stored(&r.output),
        "rendered":r.rendered,"artifact":r.artifact,"triggered_by":Id::from(r.triggered_by),
        "created_at":r.created_at,"started_at":r.started_at,"finished_at":r.finished_at})
}
async fn scoped(
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    id: &str,
) -> Result<apps::Model> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    apps::Entity::find_by_id(i64::from(id))
        .filter(apps::Column::WorkspaceId.eq(scope.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
#[derive(Deserialize)]
pub struct Body {
    name: String,
    #[serde(default)]
    category: Option<String>,
    manifest: String,
    mode: String,
    target_kind: String,
    target_ref: String,
    workdir: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    version: i64,
}
async fn check_category(
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    category: &str,
    previous: Option<&str>,
) -> Result<()> {
    if category.is_empty() || previous == Some(category) {
        return Ok(());
    }
    if !crate::maintenance::dictionary_contains(conn, scope.workspace_id, "app-category", category)
        .await?
    {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("deployment.categoryInvalid"),
        ));
    }
    Ok(())
}
async fn check(
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    b: &Body,
) -> Result<(String, String, Id)> {
    if b.name.trim().is_empty() || b.name.chars().count() > 60 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.001"),
        ));
    }
    if !["compose", "docker", "native", "stack"].contains(&b.mode.as_str())
        || !["host", "host-group"].contains(&b.target_kind.as_str())
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.002"),
        ));
    }
    let workdir = crate::hosts::channel::check_path(&b.workdir)?;
    if b.mode == "native"
        && (workdir.chars().any(char::is_control) || workdir.contains(['[', ']', '*', '?', '\\']))
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.003"),
        ));
    }
    if workdir.len() > 900 || workdir == "/" {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.004"),
        ));
    }
    let host_id = Id::reference(&b.target_ref)?;
    if b.target_kind == "host-group" {
        groups::members(conn, scope.workspace_id, i64::from(host_id)).await?;
    } else {
        hosts::Entity::find_by_id(i64::from(host_id))
            .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .one(conn)
            .await?
            .ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("deployment.error.005"),
                )
            })?;
    }
    let mut manifest = manifest::parse_for(&b.manifest, &b.mode)?;
    for service in &mut manifest.services {
        if let Some(artifact) = &mut service.artifact {
            scope.require("pipelines.read")?;
            artifact.pipeline = crate::pipelines::artifacts::pipeline_id(
                conn,
                scope.workspace_id,
                &artifact.pipeline,
            )
            .await?
            .to_string();
        }
    }
    if manifest.services.iter().any(|s| !s.files.is_empty()) {
        scope.require("config-templates.read")?;
    }

    Ok((
        b.name.trim().into(),
        serde_json::to_string_pretty(&manifest).map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.006"),
            )
        })?,
        host_id,
    ))
}
pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("deployments.read")?;
    let q = ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) =
        resource::list(&s.db, &SPEC, apps::Entity::find(), &scope.workspace_id, &q).await?;
    let mut items: Vec<Value> = rows.iter().map(app_json).collect();
    crate::resources::names::attach(
        &s.db,
        scope.workspace_id,
        &mut items,
        &[crate::resources::names::target(
            "target_ref",
            "target_name",
            "target_kind",
        )],
    )
    .await?;
    Ok(Json(
        json!({"items":items,"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
pub async fn get(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    scope.require("deployments.read")?;
    let r = scoped(&s.db, &scope, &id).await?;
    let latest = revisions::Entity::find()
        .filter(revisions::Column::DeploymentId.eq(r.id))
        .filter(revisions::Column::WorkspaceId.eq(scope.workspace_id))
        .order_by_desc(revisions::Column::Number)
        .one(&s.db)
        .await?;
    let mut out = app_json(&r);
    out["latest_revision"] = latest.as_ref().map(revision_json).unwrap_or(Value::Null);
    if let Some(latest) = &latest {
        out["latest_revision"]["targets"] =
            groups::details(&s.db, scope.workspace_id, latest.id).await?;
    }
    out["current_targets"] = groups::current(&s.db, scope.workspace_id, r.id).await?;
    Ok(Json(out))
}
pub async fn revision_get(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    scope.require("deployments.read")?;
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    let r = revisions::Entity::find_by_id(i64::from(id))
        .filter(revisions::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let mut out = revision_json(&r);
    out["targets"] = groups::details(&s.db, scope.workspace_id, r.id).await?;
    Ok(Json(out))
}
pub async fn revision_list(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("deployments.read")?;
    let q = ListQuery::parse(&REV_SPEC, &raw)?;
    let (rows, total) = resource::list(
        &s.db,
        &REV_SPEC,
        revisions::Entity::find(),
        &scope.workspace_id,
        &q,
    )
    .await?;
    let mut items: Vec<Value> = rows.iter().map(revision_json).collect();
    crate::resources::names::attach(
        &s.db,
        scope.workspace_id,
        &mut items,
        &[
            crate::resources::names::of(
                "deployment_id",
                "deployment_name",
                crate::resources::names::Kind::Deployment,
            ),
            crate::resources::names::of(
                "triggered_by",
                "triggered_by_name",
                crate::resources::names::Kind::User,
            ),
        ],
    )
    .await?;
    Ok(Json(
        json!({"items":items,"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
pub async fn revisions_for_deployment(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Query(mut raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("deployments.read")?;
    let app = scoped(&s.db, &scope, &id).await?;
    raw.insert("deployment_id".into(), app.id.to_string());
    revision_list(State(s), scope, Query(raw)).await
}
pub async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Json(b): Json<Body>,
) -> Result<(StatusCode, Json<Value>)> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let result = create_in(&s, &tx, &scope, b).await?;
    tx.commit().await?;
    Ok(result)
}
pub(crate) async fn create_in(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    scope: &Scope,
    b: Body,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("deployments.create")?;
    let (name, manifest, target_ref) = check(tx, &scope, &b).await?;
    check_category(tx, scope, b.category.as_deref().unwrap_or_default(), None).await?;
    let (id, now) = (s.next_id()?, db::now());
    let row = apps::Model {
        id: id.into(),
        workspace_id: scope.workspace_id.into(),
        name: name.clone(),
        category: b.category.unwrap_or_default(),
        manifest,
        preview_artifact: String::new(),
        mode: b.mode,
        target_kind: b.target_kind,
        target_ref: target_ref.into(),
        workdir: b.workdir.trim_end_matches('/').into(),
        description: b.description,
        state: "idle".into(),
        revision_number: 0,
        current_revision: 0,
        created_at: now,
        updated_at: now,
        version: 1,
    };
    if row.target_kind == "host-group" {
        groups::members(tx, scope.workspace_id, row.target_ref).await?;
    }
    bindings(tx, &row, &row.manifest).await?;
    apps::Entity::insert(row.into_active_model())
        .exec(tx)
        .await?;
    crate::hosts::activity_in(
        &s,
        tx,
        &scope,
        "create",
        "deployments",
        &id,
        json!({"name":name}),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id}))))
}
pub async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Json(b): Json<Body>,
) -> Result<Json<Value>> {
    scope.require("deployments.update")?;
    let r = scoped(&s.db, &scope, &id).await?;
    let (name, manifest, host) = check(&s.db, &scope, &b).await?;
    if i64::from(host) != r.target_ref
        || b.workdir.trim_end_matches('/') != r.workdir
        || b.mode != r.mode
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.007"),
        ));
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let current = scoped(&tx, &scope, &id).await?;
    let category = b.category.as_deref().unwrap_or(&current.category);
    check_category(&tx, &scope, category, Some(&current.category)).await?;
    bindings(&tx, &r, &manifest).await?;
    let n = apps::Entity::update_many()
        .col_expr(apps::Column::Name, Expr::value(name.clone()))
        .col_expr(apps::Column::Category, Expr::value(category))
        .col_expr(apps::Column::Manifest, Expr::value(manifest))
        .col_expr(apps::Column::PreviewArtifact, Expr::value(""))
        .col_expr(apps::Column::Description, Expr::value(b.description))
        .col_expr(
            apps::Column::Version,
            Expr::col(apps::Column::Version).add(1),
        )
        .col_expr(apps::Column::UpdatedAt, Expr::value(db::now()))
        .filter(apps::Column::Id.eq(r.id))
        .filter(apps::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(apps::Column::Version.eq(b.version))
        .filter(apps::Column::State.is_not_in(["queued", "running", "needs-review"]))
        .exec(&tx)
        .await?
        .rows_affected;
    if n == 0 {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("deployment.error.008"),
        ));
    }
    tx.commit().await?;
    crate::hosts::activity(
        &s,
        &scope,
        "update",
        "deployments",
        &Id::from(r.id),
        json!({"name":name}),
    )
    .await?;
    Ok(Json(json!({"ok":true})))
}
pub async fn remove(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<StatusCode> {
    scope.require("deployments.delete")?;
    let r = scoped(&s.db, &scope, &id).await?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    crate::notifications::alerts::ensure_unreferenced(
        &tx,
        scope.workspace_id,
        "deployment",
        &[Id::from(r.id)],
    )
    .await?;
    crate::cron_jobs::ensure_unreferenced(&tx, scope.workspace_id, "deployment", &[Id::from(r.id)])
        .await?;
    crate::variables::check_object_removal(&tx, scope.workspace_id, "deploy", r.id.into()).await?;
    crate::pipelines::contexts::ensure_unreferenced(
        &tx,
        scope.workspace_id,
        "application",
        &[Id::from(r.id)],
    )
    .await?;
    let n = apps::Entity::delete_many()
        .filter(apps::Column::Id.eq(r.id))
        .filter(apps::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(apps::Column::State.is_not_in(["queued", "running", "needs-review"]))
        .exec(&tx)
        .await?
        .rows_affected;
    if n == 0 {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("deployment.error.009"),
        ));
    }
    groups::remove(&tx, scope.workspace_id, r.id).await?;
    crate::entities::deployment_artifact_refs::Entity::delete_many()
        .filter(
            crate::entities::deployment_artifact_refs::Column::WorkspaceId.eq(scope.workspace_id),
        )
        .filter(
            crate::entities::deployment_artifact_refs::Column::RevisionId.in_subquery(
                revisions::Entity::find()
                    .select_only()
                    .column(revisions::Column::Id)
                    .filter(revisions::Column::DeploymentId.eq(r.id))
                    .filter(revisions::Column::WorkspaceId.eq(scope.workspace_id))
                    .into_query(),
            ),
        )
        .exec(&tx)
        .await?;
    revisions::Entity::delete_many()
        .filter(revisions::Column::DeploymentId.eq(r.id))
        .filter(revisions::Column::WorkspaceId.eq(scope.workspace_id))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    crate::hosts::activity(
        &s,
        &scope,
        "delete-record-only",
        "deployments",
        &Id::from(r.id),
        json!({"name":r.name}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn render(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    scope.require("deployments.read")?;
    let r = scoped(&s.db, &scope, &id).await?;
    Ok(Json(
        json!({"artifact":manifest::render_for(&manifest::parse_for(&r.manifest,&r.mode)?,&r.mode)?,"version":r.version}),
    ))
}
pub(crate) fn directory(r: &apps::Model) -> String {
    format!("{}/{}/{}", r.workdir, r.workspace_id, r.id)
}
pub(crate) fn project(r: &apps::Model) -> String {
    format!("deployment-{}-{}", r.workspace_id, r.id)
}
fn hash(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}
pub(crate) fn quote(v: &str) -> String {
    format!("'{}'", v.replace('\'', "'\\''"))
}
async fn remote_artifact(ch: &dyn HostChannel, r: &apps::Model) -> Result<String> {
    let path = quote(&format!("{}/compose.json", directory(r)));
    let out = ch
        .exec(
            &format!("if [ -e {path} ]; then cat -- {path}; fi"),
            Duration::from_secs(15),
        )
        .await?;
    if out.code != 0 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.010"),
        ));
    }
    Ok(out.stdout)
}
// Public bindings contain identities/versions only; values never enter Compose JSON.
pub(crate) async fn references_variable(
    conn: &impl sea_orm::ConnectionTrait,
    app: &apps::Model,
    id: i64,
) -> Result<bool> {
    Ok(bindings(conn, app, &app.manifest)
        .await?
        .1
        .values()
        .any(|row| row.id == id))
}
type ConfigContents = BTreeMap<(String, String), String>;
async fn bindings(
    conn: &impl sea_orm::ConnectionTrait,
    r: &apps::Model,
    manifest: &str,
) -> Result<(
    Value,
    BTreeMap<String, crate::entities::variables::Model>,
    ConfigContents,
)> {
    use crate::entities::config_templates as templates;
    let manifest = manifest::parse_for(manifest, &r.mode)?;
    let refs = manifest.references();
    let mut names: std::collections::BTreeSet<String> = refs.values().cloned().collect();
    let mut metadata = Vec::new();
    let mut configs = ConfigContents::new();
    let mut size = 0usize;
    for service in &manifest.services {
        if let Some(artifact) = &service.artifact {
            crate::pipelines::artifacts::pipeline_id(
                conn,
                r.workspace_id.into(),
                &artifact.pipeline,
            )
            .await?;
        }
        for file in &service.files {
            let query =
                templates::Entity::find().filter(templates::Column::WorkspaceId.eq(r.workspace_id));
            let matches = if let Ok(id) = file.from_template.parse::<Id>() {
                query.filter(templates::Column::Id.eq(id)).all(conn).await?
            } else {
                query
                    .filter(templates::Column::Name.eq(&file.from_template))
                    .all(conn)
                    .await?
            };
            if matches.len() != 1 {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("deployment.error.011"),
                ));
            }
            let row = &matches[0];
            let text =
                crate::templates::config_content(&row.content, &row.parameters, &file.values)?;
            size += text.len();
            if size > 4 * 1024 * 1024 {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("deployment.error.012"),
                ));
            }
            for (i, part) in text.split('\0').enumerate() {
                if i % 2 == 1 {
                    names.insert(
                        part.strip_prefix("variable/")
                            .ok_or_else(|| {
                                ApiError::localized(
                                    axum::http::StatusCode::BAD_REQUEST,
                                    crate::i18n::Message::new("deployment.error.013"),
                                )
                            })?
                            .to_owned(),
                    );
                }
            }
            metadata.push(json!({"kind":"file","service":service.name,"path":file.path,"template_id":Id::from(row.id),"template_version":row.version,"content":crate::templates::config_preview(&text),"digest":hash(&text)}));
            configs.insert((service.name.clone(), file.path.clone()), text);
        }
    }
    let rows =
        crate::variables::resolve(conn, r.workspace_id.into(), "deploy", r.id.into(), &names)
            .await?;
    for ((service, env), name) in &refs {
        let row = &rows[name];
        metadata.push(json!({"service":service,"environment":env,"name":name,"id":Id::from(row.id),"version":row.version,"scope":row.scope,"scope_ref":Id::from(row.scope_ref),"secret":row.kind=="secret"}));
    }
    for ((service, path), text) in &configs {
        for (i, part) in text.split('\0').enumerate() {
            if i % 2 == 1 {
                let name = part.strip_prefix("variable/").unwrap();
                let row = &rows[name];
                metadata.push(json!({"kind":"file-variable","service":service,"path":path,"name":name,"id":Id::from(row.id),"version":row.version,"scope":row.scope,"scope_ref":Id::from(row.scope_ref),"secret":row.kind=="secret"}));
            }
        }
    }
    Ok((json!(metadata), rows, configs))
}
fn config_path(metadata: &Value, service: &str, path: &str) -> String {
    format!(
        ".config-{}.txt",
        hash(&format!("{metadata}:{service}:{path}"))
    )
}
fn config_files(
    s: &AppState,
    metadata: &Value,
    rows: &BTreeMap<String, crate::entities::variables::Model>,
    configs: &ConfigContents,
) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    for ((service, path), text) in configs {
        let mut content = String::new();
        for (i, part) in text.split('\0').enumerate() {
            if i % 2 == 0 {
                content.push_str(part);
            } else {
                let name = part.strip_prefix("variable/").ok_or_else(|| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("deployment.error.013"),
                    )
                })?;
                let row = rows.get(name).ok_or_else(|| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("deployment.error.014"),
                    )
                })?;
                let value = s.vault.open_secret(
                    &row.value,
                    &crate::vault::aad(
                        &row.workspace_id.to_string(),
                        "variables",
                        &row.id.to_string(),
                    ),
                )?;
                content.push_str(&value);
            }
        }
        files.insert(config_path(metadata, service, path), content);
    }
    Ok(files)
}

fn artifact_bindings(artifact: &str) -> Result<Value> {
    let document: Value = serde_json::from_str(artifact).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.015"),
        )
    })?;
    let mut metadata = document
        .get("x-variable-bindings")
        .cloned()
        .unwrap_or(json!([]));
    if let Some(items) = metadata.as_array_mut() {
        for item in items {
            if item["kind"] == "file" {
                if let Some(text) = item["content"].as_str() {
                    item["content"] = json!(text.replace("$$", "$"));
                }
            }
        }
    }
    Ok(metadata)
}
fn environment_path(metadata: &Value, service: &str) -> String {
    format!(".variables-{}-{service}.env", hash(&metadata.to_string()))
}
fn bind_artifact(artifact: &str, metadata: &Value) -> Result<String> {
    if metadata == &json!([]) {
        return Ok(artifact.to_owned());
    }
    let mut document: Value = serde_json::from_str(artifact).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.015"),
        )
    })?;
    for binding in metadata.as_array().ok_or_else(|| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.016"),
        )
    })? {
        if binding["kind"] == "file-variable" {
            continue;
        }
        let service = binding["service"].as_str().ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.017"),
            )
        })?;
        if !document["services"][service].is_object() {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.018"),
            ));
        }
        if binding["kind"] == "file" {
            let path = binding["path"].as_str().ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("deployment.error.019"),
                )
            })?;
            let source = format!("./{}", config_path(metadata, service, path));
            let entry = document["services"][service]
                .as_object_mut()
                .unwrap()
                .entry("volumes")
                .or_insert(json!([]));
            entry.as_array_mut().ok_or_else(||ApiError::localized(axum::http::StatusCode::BAD_REQUEST, crate::i18n::Message::new("deployment.error.020")))?.push(json!({"type":"bind","source":source,"target":path,"read_only":true,"bind":{"create_host_path":false}}));
        } else {
            document["services"][service]["env_file"] =
                json!([environment_path(metadata, service)]);
        }
    }
    document["x-variable-bindings"] = metadata.clone();
    for binding in document["x-variable-bindings"].as_array_mut().unwrap() {
        if binding["kind"] == "file" {
            if let Some(text) = binding["content"].as_str() {
                binding["content"] = json!(text.replace('$', "$$"));
            }
        }
    }
    serde_json::to_string_pretty(&document)
        .map(|s| s + "\n")
        .map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.021"),
            )
        })
}
fn dotenv_value(value: &str) -> String {
    let mut encoded = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => encoded.push_str("\\\\"),
            '"' => encoded.push_str("\\\""),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            '$' => encoded.push_str("$$"),
            other => encoded.push(other),
        }
    }
    encoded.push('"');
    encoded
}
fn environment_files(
    s: &AppState,
    metadata: &Value,
    rows: &BTreeMap<String, crate::entities::variables::Model>,
    mode: &str,
) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::<String, String>::new();
    for binding in metadata.as_array().ok_or_else(|| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.016"),
        )
    })? {
        if binding.get("kind").is_some() {
            continue;
        }
        let name = binding["name"].as_str().ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.022"),
            )
        })?;
        let row = rows.get(name).ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.023"),
            )
        })?;
        let value = s.vault.open_secret(
            &row.value,
            &crate::vault::aad(
                &row.workspace_id.to_string(),
                "variables",
                &row.id.to_string(),
            ),
        )?;
        let service = binding["service"].as_str().ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.017"),
            )
        })?;
        let env = binding["environment"].as_str().ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.024"),
            )
        })?;
        // Compose dotenv double quotes use JSON escapes; $$ disables interpolation.
        let encoded = dotenv_value(&value);
        files
            .entry(environment_path(metadata, service))
            .or_default()
            .push_str(&if mode == "docker" {
                format!("export {env}={}\n", quote(&value))
            } else if mode == "stack" {
                format!("{}\n", json!({env: value}))
            } else if mode == "native" {
                format!(
                    "{env}={}\n",
                    crate::deployments::modes::systemd_environment(&value)
                )
            } else {
                format!("{env}={encoded}\n")
            });
    }
    Ok(files)
}
fn snapshot_aad(workspace: i64, revision: i64) -> String {
    crate::vault::aad(
        &workspace.to_string(),
        "deployment-variables",
        &revision.to_string(),
    )
}
fn open_snapshot(
    vault: &crate::vault::Vault,
    rev: &revisions::Model,
) -> Result<BTreeMap<String, String>> {
    let sealed = rev.variable_snapshot.as_deref().unwrap_or_default();
    if sealed.is_empty() {
        if artifact_bindings(&rev.artifact)? != json!([]) {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.025"),
            ));
        }
        return Ok(BTreeMap::new());
    }
    let text = vault.open_secret(sealed, &snapshot_aad(rev.workspace_id, rev.id))?;
    serde_json::from_str(&text).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.026"),
        )
    })
}
fn seal_snapshot(
    vault: &crate::vault::Vault,
    workspace: i64,
    revision: i64,
    files: &BTreeMap<String, String>,
) -> Result<String> {
    if files.is_empty() {
        return Ok(String::new());
    }
    let text = serde_json::to_string(files).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.027"),
        )
    })?;
    vault.seal(&text, &snapshot_aad(workspace, revision))
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewBody {
    #[serde(default)]
    revision_id: Option<String>,
}
fn preview_token(artifact: &str, expected: &str, version: i64, source: i64) -> String {
    hash(&format!("{version}:{source}:{expected}:{artifact}"))
}
async fn old_revision(
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    app_id: i64,
    id: &str,
) -> Result<revisions::Model> {
    let id = Id::reference(id)?;
    revisions::Entity::find_by_id(i64::from(id))
        .filter(revisions::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(revisions::Column::DeploymentId.eq(app_id))
        .filter(revisions::Column::State.eq("succeeded"))
        .one(conn)
        .await?
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.028"),
            )
        })
}
pub async fn preview(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Json(b): Json<PreviewBody>,
) -> Result<Json<Value>> {
    scope.require("deployments.read")?;
    let r = scoped(&s.db, &scope, &id).await?;
    if r.target_kind == "host-group" {
        return groups::preview(&s, &scope, &r, b).await;
    }
    let ch = remote::open(&s, &scope.workspace_id, &Id::from(r.target_ref)).await?;
    let (desired, source) = if let Some(id) = b.revision_id {
        let old = old_revision(&s.db, &scope, r.id, &id).await?;
        (old.artifact, old.id)
    } else {
        let (metadata, _, _) = bindings(&s.db, &r, &r.manifest).await?;
        let artifact = prepare_artifact(&s, &scope, ch.as_ref(), &r, &metadata).await?;
        let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
        if bindings(&tx, &r, &r.manifest).await?.0 != metadata {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("deployment.error.029"),
            ));
        }
        save_preview(&tx, &scope, &r, &artifact).await?;
        tx.commit().await?;
        (artifact, 0)
    };
    let current = remote_artifact(ch.as_ref(), &r).await?;
    let expected = hash(&current);
    crate::hosts::activity(
        &s,
        &scope,
        "preview",
        "deployments",
        &Id::from(r.id),
        json!({}),
    )
    .await?;
    Ok(Json(
        json!({"desired":desired,"current":current,"expected_hash":expected,
        "preview_token":preview_token(&desired,&expected,r.version,source),"version":r.version,"changed":current!=desired,
        "variables": artifact_bindings(&desired)?,
        "notice": crate::i18n::message(if source != 0 {"deployment.notice.rollback"} else {"deployment.notice.preview"})}),
    ))
}
async fn prepare_artifact(
    s: &AppState,
    scope: &Scope,
    ch: &dyn HostChannel,
    r: &apps::Model,
    metadata: &Value,
) -> Result<String> {
    let m = manifest::parse_for(&r.manifest, &r.mode)?;
    if m.services.iter().any(|s| !s.files.is_empty()) {
        scope.require("config-templates.read")?;
    }
    if r.mode == "native" {
        scope.require("pipelines.read")?;
    }
    let rendered = manifest::render_for(&m, &r.mode)?;
    let artifact = bind_artifact(&resolve_artifact(s, ch, r, &rendered).await?, metadata)?;
    match r.mode.as_str() {
        "native" => crate::deployments::modes::native_finalize(r, &artifact),
        "stack" => crate::deployments::modes::stack_finalize(r, &artifact),
        _ => Ok(artifact),
    }
}
async fn save_preview(
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    r: &apps::Model,
    artifact: &str,
) -> Result<()> {
    let n = apps::Entity::update_many()
        .col_expr(apps::Column::PreviewArtifact, Expr::value(artifact))
        .filter(apps::Column::Id.eq(r.id))
        .filter(apps::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(apps::Column::Version.eq(r.version))
        .exec(conn)
        .await?
        .rows_affected;
    if n == 0 {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("deployment.error.030"),
        ));
    }
    Ok(())
}
async fn resolve_artifact(
    s: &AppState,
    ch: &dyn HostChannel,
    r: &apps::Model,
    rendered: &str,
) -> Result<String> {
    if r.mode == "native" {
        return crate::deployments::modes::native_resolve(s, r, rendered).await;
    }
    if r.mode == "docker" || r.mode == "stack" {
        return crate::deployments::modes::docker_resolve(ch, rendered).await;
    }
    let mut public: Value = serde_json::from_str(rendered).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.015"),
        )
    })?;
    public
        .as_object_mut()
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.015"),
            )
        })?
        .remove("x-variable-references");
    public
        .as_object_mut()
        .unwrap()
        .remove("x-config-references");
    let rendered = serde_json::to_string(&public).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.015"),
        )
    })?;
    let dir = directory(r);
    let path = format!("{dir}/preview-{}.json", s.next_id()?);
    let prepare=ch.exec(&format!("set -eu; command -v flock >/dev/null; command -v sha256sum >/dev/null; docker compose version; umask 077; mkdir -p -- {}",quote(&dir)),Duration::from_secs(30)).await?;
    if prepare.code != 0 {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.mode.037").with("detail", prepare.stderr),
        ));
    }
    upload(ch, &path, rendered.to_owned()).await?;
    let result=ch.exec(&format!("docker compose --project-name {} -f {} config --resolve-image-digests --format json",quote(&project(r)),quote(&path)),Duration::from_secs(120)).await;
    ch.delete_file(&path).await?;
    let out = result?;
    if out.code != 0 {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.mode.038").with("detail", out.stderr),
        ));
    }
    let value: Value = serde_json::from_str(&out.stdout).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.031"),
        )
    })?;
    if value["services"].as_object().is_none_or(|services| {
        services.is_empty()
            || services.values().any(|v| {
                !v["image"]
                    .as_str()
                    .is_some_and(|image| image.contains("@sha256:"))
            })
    }) {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.032"),
        ));
    }
    Ok(out.stdout)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunBody {
    version: i64,
    expected_hash: String,
    preview_token: String,
    #[serde(default)]
    target_hashes: BTreeMap<String, String>,
    #[serde(default)]
    revision_id: String,
}
pub async fn apply(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Json(b): Json<RunBody>,
) -> Result<(StatusCode, Json<Value>)> {
    trigger(&s, &scope, &id, b, false, false).await
}
/// Reuses exact preview/version/variable checks for an authorized scheduled action.
pub async fn scheduled(s: &AppState, scope: Scope, id: &str, restart: bool) -> Result<Id> {
    scope.require("deployments.apply")?;
    let row = scoped(&s.db, &scope, id).await?;
    if restart && row.current_revision == 0 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.033"),
        ));
    }
    let revision_id = if restart {
        Some(Id::from(row.current_revision).to_string())
    } else {
        None
    };
    let Json(preview) = preview(
        State(s.clone()),
        scope.clone(),
        Path((String::new(), id.to_owned())),
        Json(PreviewBody {
            revision_id: revision_id.clone(),
        }),
    )
    .await?;
    let body = RunBody {
        version: preview["version"].as_i64().ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.034"),
            )
        })?,
        expected_hash: preview["expected_hash"].as_str().unwrap_or_default().into(),
        preview_token: preview["preview_token"].as_str().unwrap_or_default().into(),
        target_hashes: serde_json::from_value(
            preview.get("target_hashes").cloned().unwrap_or(json!({})),
        )
        .map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.035"),
            )
        })?,
        revision_id: revision_id.unwrap_or_default(),
    };
    let (_, Json(result)) = trigger(s, &scope, id, body, false, restart).await?;
    Id::reference(result["id"].as_str().unwrap_or_default())
}
pub async fn rollback(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Json(b): Json<RunBody>,
) -> Result<(StatusCode, Json<Value>)> {
    trigger(&s, &scope, &id, b, true, false).await
}
async fn trigger(
    s: &AppState,
    scope: &Scope,
    id: &str,
    b: RunBody,
    rollback: bool,
    restart: bool,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require(if rollback {
        "deployments.rollback"
    } else {
        "deployments.apply"
    })?;
    let tx = crate::variables::transaction(s, scope.workspace_id).await?;
    let r = scoped(&tx, scope, id).await?;
    if restart && r.state != "succeeded" {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("deployment.error.036"),
        ));
    }
    if b.version != r.version {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("deployment.error.030"),
        ));
    }
    if b.expected_hash.len() != 64 || !b.expected_hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.037"),
        ));
    }
    let (source, manifest, rendered, artifact, files) = if rollback || restart {
        let old = old_revision(&tx, scope, r.id, &b.revision_id).await?;
        if restart && old.id != r.current_revision {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("deployment.error.038"),
            ));
        }
        let source = Id::from(old.id);
        if old.artifact.is_empty() {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.039"),
            ));
        }
        let files = open_snapshot(&s.vault, &old)?;
        (
            i64::from(source),
            old.manifest,
            old.rendered,
            old.artifact,
            files,
        )
    } else {
        if manifest::parse_for(&r.manifest, &r.mode)?
            .services
            .iter()
            .any(|s| !s.files.is_empty())
        {
            scope.require("config-templates.read")?;
        }
        let (metadata, rows, configs) = bindings(&tx, &r, &r.manifest).await?;
        if r.preview_artifact.is_empty() || artifact_bindings(&r.preview_artifact)? != metadata {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("deployment.error.040"),
            ));
        }
        let mut files = environment_files(s, &metadata, &rows, &r.mode)?;
        files.extend(config_files(s, &metadata, &rows, &configs)?);
        (
            0,
            r.manifest.clone(),
            manifest::render_for(&manifest::parse_for(&r.manifest, &r.mode)?, &r.mode)?,
            r.preview_artifact.clone(),
            files,
        )
    };
    if r.mode == "stack"
        && files
            .iter()
            .any(|(name, text)| name.starts_with(".config-") && text.len() > 500 * 1024)
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.041"),
        ));
    }
    if artifact.is_empty()
        || b.preview_token != preview_token(&artifact, &b.expected_hash, r.version, source)
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("deployment.error.042"),
        ));
    }
    let (id, now) = (s.next_id()?, db::now());
    if r.target_kind == "host-group" {
        groups::enqueue(s, &tx, &r, id, &artifact, &b, source).await?;
    } else {
        hosts::Entity::find_by_id(r.target_ref)
            .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .one(&tx)
            .await?
            .ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("deployment.error.043"),
                )
            })?;
    }
    crate::pipelines::artifacts::freeze(s, &tx, scope.workspace_id, id, &artifact).await?;
    let variable_snapshot = seal_snapshot(&s.vault, r.workspace_id, i64::from(id), &files)?;
    let n = apps::Entity::update_many()
        .col_expr(apps::Column::State, Expr::value("queued"))
        .col_expr(
            apps::Column::RevisionNumber,
            Expr::col(apps::Column::RevisionNumber).add(1),
        )
        .col_expr(
            apps::Column::Version,
            Expr::col(apps::Column::Version).add(1),
        )
        .col_expr(apps::Column::UpdatedAt, Expr::value(now))
        .filter(apps::Column::Id.eq(r.id))
        .filter(apps::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(apps::Column::Version.eq(b.version))
        .filter(apps::Column::State.is_not_in(["queued", "running", "needs-review"]))
        .exec(&tx)
        .await?
        .rows_affected;
    if n == 0 {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("deployment.error.044"),
        ));
    }
    let current = apps::Entity::find_by_id(r.id)
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let action = if restart {
        "restart"
    } else if rollback {
        "rollback"
    } else {
        "apply"
    };
    revisions::Entity::insert(
        revisions::Model {
            id: id.into(),
            workspace_id: scope.workspace_id.into(),
            deployment_id: r.id,
            number: current.revision_number,
            action: action.into(),
            source_revision: source,
            manifest,
            rendered,
            artifact,
            variable_snapshot: Some(variable_snapshot),
            expected_hash: b.expected_hash,
            state: "queued".into(),
            output: String::new(),
            triggered_by: scope.user.id.into(),
            started_at: 0,
            finished_at: 0,
            created_at: now,
            updated_at: now,
            version: 1,
        }
        .into_active_model(),
    )
    .exec(&tx)
    .await?;
    tx.commit().await?;
    crate::hosts::activity(
        s,
        scope,
        action,
        "deployments",
        &Id::from(r.id),
        json!({"revision_id":id}),
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(json!({"id":id}))))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewBody {
    version: i64,
    note: String,
}
pub async fn acknowledge(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Json(b): Json<ReviewBody>,
) -> Result<Json<Value>> {
    scope.require("deployments.apply")?;
    if b.note.trim().is_empty() || b.note.chars().count() > 500 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.045"),
        ));
    }
    let r = scoped(&s.db, &scope, &id).await?;
    let n = apps::Entity::update_many()
        .col_expr(apps::Column::State, Expr::value("failed"))
        .col_expr(
            apps::Column::Version,
            Expr::col(apps::Column::Version).add(1),
        )
        .filter(apps::Column::Id.eq(r.id))
        .filter(apps::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(apps::Column::State.eq("needs-review"))
        .filter(apps::Column::Version.eq(b.version))
        .exec(&s.db)
        .await?
        .rows_affected;
    if n == 0 {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("deployment.error.046"),
        ));
    }
    crate::hosts::activity(
        &s,
        &scope,
        "acknowledge-interrupted",
        "deployments",
        &Id::from(r.id),
        json!({"note":b.note}),
    )
    .await?;
    Ok(Json(json!({"ok":true})))
}

async fn finish(s: &AppState, rev: &revisions::Model, state: &str, output: &str) -> Result<()> {
    let now = db::now();
    let tx = s.db.begin().await?;
    groups::settle(&tx, rev.id).await?;
    revisions::Entity::update_many()
        .col_expr(revisions::Column::State, Expr::value(state))
        .col_expr(revisions::Column::Output, Expr::value(output))
        .col_expr(revisions::Column::FinishedAt, Expr::value(now))
        .col_expr(revisions::Column::UpdatedAt, Expr::value(now))
        .filter(revisions::Column::Id.eq(rev.id))
        .exec(&tx)
        .await?;
    let mut update = apps::Entity::update_many()
        .col_expr(apps::Column::State, Expr::value(state))
        .col_expr(
            apps::Column::Version,
            Expr::col(apps::Column::Version).add(1),
        )
        .col_expr(apps::Column::UpdatedAt, Expr::value(now));
    if state == "succeeded" {
        update = update.col_expr(apps::Column::CurrentRevision, Expr::value(rev.id));
    }
    update
        .filter(apps::Column::Id.eq(rev.deployment_id))
        .filter(apps::Column::WorkspaceId.eq(rev.workspace_id))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
pub async fn scheduler(
    s: AppState,
    mut stop: tokio::sync::watch::Receiver<bool>,
    mut drain: tokio::sync::watch::Receiver<bool>,
) {
    // Same single-service-instance execution scope as pipelines. Never replay an uncertain remote mutation.
    let recover = async {
        let rows = revisions::Entity::find()
            .filter(revisions::Column::State.is_in(["running", "queued"]))
            .all(&s.db)
            .await?;
        for r in rows {
            finish(
                &s,
                &r,
                "needs-review",
                &StoredText::from_message(Message::new("deployment.output.recovered")).stored(),
            )
            .await?;
        }
        Ok::<_, ApiError>(())
    }
    .await;
    if let Err(e) = recover {
        tracing::error!(%e,"部署恢复失败，暂停调度");
        return;
    }
    let mut next_cleanup = std::time::Instant::now();
    loop {
        if *stop.borrow() || *drain.borrow() {
            break;
        }
        if std::time::Instant::now() >= next_cleanup {
            if let Err(error) = crate::maintenance::prune_history(&s, true).await {
                tracing::error!(%error,"历史清理失败");
            }
            next_cleanup = std::time::Instant::now() + Duration::from_secs(60);
        }
        if let Err(e) = dispatch(&s, &mut stop).await {
            tracing::error!(%e,"部署调度失败");
        }
        tokio::select! {_=drain.changed()=>break,_=stop.changed()=>break,_=tokio::time::sleep(Duration::from_millis(200))=>{}}
    }
}
async fn dispatch(s: &AppState, stop: &mut tokio::sync::watch::Receiver<bool>) -> Result<()> {
    let Some(rev) = revisions::Entity::find()
        .filter(revisions::Column::State.eq("queued"))
        .order_by_asc(revisions::Column::CreatedAt)
        .one(&s.db)
        .await?
    else {
        return Ok(());
    };
    let n = revisions::Entity::update_many()
        .col_expr(revisions::Column::State, Expr::value("running"))
        .col_expr(revisions::Column::StartedAt, Expr::value(db::now()))
        .filter(revisions::Column::Id.eq(rev.id))
        .filter(revisions::Column::State.eq("queued"))
        .exec(&s.db)
        .await?
        .rows_affected;
    if n == 0 {
        return Ok(());
    }
    apps::Entity::update_many()
        .col_expr(apps::Column::State, Expr::value("running"))
        .filter(apps::Column::Id.eq(rev.deployment_id))
        .exec(&s.db)
        .await?;
    let outcome = tokio::select! {
        v=execute(s,&rev)=>v,
        _=stop.changed()=>Err(ApiError::localized(axum::http::StatusCode::BAD_REQUEST, crate::i18n::Message::new("deployment.error.047"))),
    };
    match outcome {
        Ok((success, output)) => {
            finish(
                s,
                &rev,
                if success { "succeeded" } else { "failed" },
                &output,
            )
            .await?
        }
        Err(e) => {
            let mut output =
                StoredText::from_message(Message::new("deployment.output.uncertainPrefix"));
            if let Some(message) = e.2 {
                output.push_message(message);
            } else {
                output.push_raw(e.1);
            }
            output.push_message(Message::new("deployment.output.verifyRemote"));
            finish(s, &rev, "needs-review", &output.stored()).await?
        }
    }
    Ok(())
}
pub(crate) async fn upload(ch: &dyn HostChannel, path: &str, text: String) -> Result<()> {
    ch.write_file(
        path,
        0o600,
        Box::pin(futures::stream::once(async move {
            Ok(bytes::Bytes::from(text))
        })),
    )
    .await
}
async fn execute(s: &AppState, rev: &revisions::Model) -> Result<(bool, String)> {
    let protected = rev
        .variable_snapshot
        .as_ref()
        .is_some_and(|v| !v.is_empty())
        || artifact_bindings(&rev.artifact)? != json!([]);
    match execute_inner(s, rev, protected).await {
        Err(_) if protected => Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.048"),
        )),
        result => result,
    }
}
async fn execute_inner(
    s: &AppState,
    rev: &revisions::Model,
    protected: bool,
) -> Result<(bool, String)> {
    let r = apps::Entity::find_by_id(rev.deployment_id)
        .filter(apps::Column::WorkspaceId.eq(rev.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    if r.target_kind == "host-group" {
        return groups::execute(s, &r, rev, protected).await;
    }
    execute_host(s, rev, protected, &r).await
}
async fn execute_host(
    s: &AppState,
    rev: &revisions::Model,
    protected: bool,
    r: &apps::Model,
) -> Result<(bool, String)> {
    let ch = remote::open(s, &Id::from(r.workspace_id), &Id::from(r.target_ref)).await?;
    if r.mode == "stack" {
        let files = open_snapshot(&s.vault, rev)?;
        return crate::deployments::modes::stack_execute(ch.as_ref(), &r, rev, &files, protected)
            .await;
    }
    if r.mode == "native" {
        let files = open_snapshot(&s.vault, rev)?;
        return crate::deployments::modes::native_execute(
            s,
            ch.as_ref(),
            &r,
            rev,
            &files,
            protected,
        )
        .await;
    }
    if r.mode == "docker" {
        let files = open_snapshot(&s.vault, rev)?;
        return crate::deployments::modes::docker_execute(ch.as_ref(), &r, rev, &files, protected)
            .await;
    }
    let current = remote_artifact(ch.as_ref(), &r).await?;
    if hash(&current) != rev.expected_hash {
        return Ok((
            false,
            StoredText::from_message(Message::new("deployment.output.artifactChanged")).stored(),
        ));
    }
    let dir = directory(&r);
    let stage = format!("{dir}/revision-{}.json", rev.id);
    let prepare=ch.exec(&format!("set -eu; command -v flock >/dev/null; docker compose version; umask 077; mkdir -p -- {}",quote(&dir)),Duration::from_secs(30)).await?;
    if prepare.code != 0 {
        return Ok((false, {
            let mut text = StoredText::new();
            text.push_message(if protected {
                Message::new("deployment.output.preflightProtected")
                    .with("code", prepare.code.to_string())
            } else {
                Message::new("deployment.output.preflightCompose")
            });
            if !protected {
                text.push_raw(prepare.stdout);
                text.push_raw(prepare.stderr);
            }
            text.stored()
        }));
    }
    let artifact = rev.artifact.clone();
    // Persist the exact artifact BEFORE mutating containers. Rollback never re-renders it.
    revisions::Entity::update_many()
        .col_expr(revisions::Column::Artifact, Expr::value(artifact.clone()))
        .filter(revisions::Column::Id.eq(rev.id))
        .exec(&s.db)
        .await?;
    upload(ch.as_ref(), &stage, artifact).await?;
    let timeout = manifest::parse(&rev.manifest)?
        .strategy
        .map(|s| s.health_timeout)
        .unwrap_or(120);
    let files = open_snapshot(&s.vault, rev)?;
    let mut uploaded: Vec<String> = Vec::new();
    for (name, text) in &files {
        // Snapshot paths are generated internally, never supplied by a client.
        if name.contains('/')
            || !(name.starts_with(".variables-") && name.ends_with(".env")
                || name.starts_with(".config-") && name.ends_with(".txt"))
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.049"),
            ));
        }
        let path = format!("{dir}/{name}");
        if let Err(error) = upload(ch.as_ref(), &path, text.clone()).await {
            for path in &uploaded {
                let _ = ch.delete_file(path).await;
            }
            let _ = ch.delete_file(&path).await;
            return Err(error);
        }
        if name.starts_with(".variables-") {
            uploaded.push(path);
        }
    }
    let current_path = format!("{dir}/compose.json");
    // Remote lock survives a lost server connection. An early manual acknowledgement cannot overlap commands.
    let check_hash = format!(
        "actual=$(if [ -e {path} ]; then cat -- {path}; fi | sha256sum); [ \"${{actual%% *}}\" = {expected} ] || {{ echo '目标产物已变化，请重新预览。' >&2; exit 73; }}; ",
        path = quote(&current_path),
        expected = quote(&rev.expected_hash)
    );
    let cleanup = if uploaded.is_empty() {
        String::new()
    } else {
        format!(
            "trap {} EXIT; ",
            quote(&format!(
                "rm -f -- {}",
                uploaded
                    .iter()
                    .map(|p| quote(p))
                    .collect::<Vec<_>>()
                    .join(" ")
            ))
        )
    };
    let mut readiness = String::new();
    let probes = manifest::readiness(&manifest::parse(&rev.manifest)?)?;
    if !probes.is_empty() {
        readiness.push_str(&format!("deadline=$(( $(date +%s) + {timeout} )); "));
        for (service, command, period) in probes {
            readiness.push_str(&format!(
                "while :; do ready=1; ids=$(docker compose --project-name {} -f {} ps -q {}); [ -n \"$ids\" ] || ready=0; for container in $ids; do docker exec \"$container\" sh -c {} >/dev/null 2>&1 || ready=0; done; [ \"$ready\" = 1 ] && break; [ $(date +%s) -lt \"$deadline\" ] || {{ echo {} >&2; exit 74; }}; next=$(( $(date +%s) + {period} )); while [ $(date +%s) -lt \"$next\" ] && [ $(date +%s) -lt \"$deadline\" ]; do sleep 1; done; done; ",
                quote(&project(&r)),quote(&stage),quote(&service),quote(&command),quote(&format!("服务 {service} readiness 检查超时。"))
            ));
        }
    }
    let inner = if rev.action == "restart" {
        format!(
            "set -eu; {cleanup}{check_hash} docker compose --project-name {} -f {} restart; {readiness}cp -- {} {}",
            quote(&project(&r)),
            quote(&stage),
            quote(&stage),
            quote(&current_path)
        )
    } else {
        format!(
            "set -eu; {cleanup}{check_hash} docker compose --project-name {} -f {} up -d --wait --wait-timeout {} --remove-orphans; {readiness}cp -- {} {}",
            quote(&project(&r)),
            quote(&stage),
            timeout,
            quote(&stage),
            quote(&current_path)
        )
    };
    let out = ch
        .exec(
            &format!(
                "flock -n {} sh -c {}",
                quote(&format!("{dir}/execution.lock")),
                quote(&inner)
            ),
            Duration::from_secs(u64::from(timeout) * 2 + 180),
        )
        .await?;
    if !uploaded.is_empty() {
        let cleanup = ch
            .exec(
                &format!(
                    "rm -f -- {}",
                    uploaded
                        .iter()
                        .map(|p| quote(p))
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
                Duration::from_secs(15),
            )
            .await?;
        if cleanup.code != 0 {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.050"),
            ));
        }
    }
    Ok((
        out.code == 0,
        execution_output(
            &out,
            protected.then(|| {
                Message::new("deployment.output.deploymentProtected")
                    .with_message(
                        "result",
                        Message::new(if out.code == 0 {
                            "deployment.output.succeeded"
                        } else {
                            "deployment.output.failed"
                        }),
                    )
                    .with("code", out.code.to_string())
            }),
        ),
    ))
}

/// Preserve literal command output while keeping application status messages reader-localized.
pub(crate) fn execution_output(
    out: &crate::hosts::channel::Output,
    protected: Option<Message>,
) -> String {
    let mut text = StoredText::new();
    if let Some(message) = protected {
        text.push_message(message);
    } else {
        text.push_raw(out.stdout.clone());
        text.push_raw(out.stderr.clone());
        text.push_message(
            Message::new("deployment.output.exitCode").with("code", out.code.to_string()),
        );
    }
    text.stored()
}

#[cfg(test)]
mod variable_snapshot_tests {
    use super::*;
    #[tokio::test]
    async fn deployment_outputs_and_validation_use_each_http_readers_language() {
        use crate::i18n::{Locale, render};
        use sea_orm::{ConnectionTrait, Database, DatabaseConnection, Schema};
        let conn = Database::connect("sqlite::memory:").await.unwrap();
        let backend = conn.get_database_backend();
        let schema = Schema::new(backend);
        for table in [
            schema.create_table_from_entity(revisions::Entity),
            schema.create_table_from_entity(crate::entities::deployment_revision_targets::Entity),
        ] {
            conn.execute_raw(backend.build(&table)).await.unwrap();
        }
        let literal = format!(
            "{}\n{{\"$system_message\":{{\"version\":1}}}}\n{{code}} 中文 사용자",
            "x".repeat(300 * 1024)
        );
        let mut row = revision();
        row.output = execution_output(
            &crate::hosts::channel::Output {
                code: 7,
                stdout: literal.clone(),
                stderr: "literal stderr".into(),
            },
            None,
        );
        let stored = row.output.clone();
        revisions::Entity::insert(row.clone().into_active_model())
            .exec(&conn)
            .await
            .unwrap();
        for (id, state) in [(10, "running"), (11, "pending")] {
            crate::entities::deployment_revision_targets::Entity::insert(
                crate::entities::deployment_revision_targets::Model {
                    id,
                    workspace_id: row.workspace_id,
                    revision_id: row.id,
                    host_id: id,
                    position: id,
                    artifact: "{}".into(),
                    expected_hash: String::new(),
                    state: state.into(),
                    output: String::new(),
                    started_at: 0,
                    finished_at: 0,
                }
                .into_active_model(),
            )
            .exec(&conn)
            .await
            .unwrap();
        }
        groups::settle(&conn, row.id).await.unwrap();
        async fn project(State(conn): State<DatabaseConnection>) -> Json<Value> {
            let row = revisions::Entity::find().one(&conn).await.unwrap().unwrap();
            let mut value = revision_json(&row);
            value["targets"] = groups::details(&conn, row.workspace_id.into(), row.id)
                .await
                .unwrap();
            Json(value)
        }
        async fn validate(State(conn): State<DatabaseConnection>) -> Result<Json<Value>> {
            let scope = Scope {
                workspace_id: 1.into(),
                user: crate::accounts::auth::User {
                    id: 1.into(),
                    name: "reader".into(),
                    email: String::new(),
                    is_superadmin: false,
                },
                permissions: Default::default(),
            };
            check(
                &conn,
                &scope,
                &Body {
                    name: String::new(),
                    category: None,
                    manifest: String::new(),
                    mode: "compose".into(),
                    target_kind: "host".into(),
                    target_ref: "1".into(),
                    workdir: "/srv".into(),
                    description: String::new(),
                    version: 1,
                },
            )
            .await?;
            Ok(Json(json!({})))
        }
        let router = axum::Router::new()
            .route("/status", axum::routing::get(project))
            .route("/validate", axum::routing::get(validate))
            .with_state(conn.clone())
            .layer(axum::middleware::from_fn(crate::i18n::middleware));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let root = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = reqwest::Client::new();
        for locale in Locale::ALL {
            let value: Value = client
                .get(format!("{root}/status"))
                .header("Accept-Language", locale.tag())
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(
                value["output"],
                format!(
                    "{literal}literal stderr{}",
                    render(
                        locale,
                        &Message::new("deployment.output.exitCode").with("code", "7")
                    )
                )
            );
            assert_eq!(
                value["targets"][0]["output"],
                render(locale, &Message::new("deployment.output.targetUncertain"))
            );
            assert_eq!(
                value["targets"][1]["output"],
                render(locale, &Message::new("deployment.output.skipped"))
            );
            let response = client
                .get(format!("{root}/validate"))
                .header("Accept-Language", locale.tag())
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let error: Value = response.json().await.unwrap();
            assert_eq!(
                error["error"],
                render(locale, &Message::new("deployment.error.001"))
            );
        }
        assert_eq!(
            revisions::Entity::find_by_id(row.id)
                .one(&conn)
                .await
                .unwrap()
                .unwrap()
                .output,
            stored
        );
        let mut legacy = row.clone();
        legacy.output=r#"{"$system_message":{"version":1,"message":{"key":"deployment.error.001","params":{}}}}"#.into();
        assert_eq!(revision_json(&legacy)["output"], legacy.output);
        task.abort();
    }

    #[test]
    fn protected_deployment_output_never_retains_command_output() {
        use crate::i18n::{Locale, render_text_stored};
        let out = crate::hosts::channel::Output {
            code: 9,
            stdout: "secret stdout".into(),
            stderr: "secret stderr".into(),
        };
        for key in [
            "deployment.output.dockerProtected",
            "deployment.output.nativeProtected",
            "deployment.output.swarmProtected",
        ] {
            let stored = execution_output(&out, Some(Message::new(key).with("code", "9")));
            assert!(!stored.contains("secret"));
            for locale in Locale::ALL {
                let rendered = render_text_stored(locale, &stored);
                assert!(rendered.contains('9'));
                assert!(!rendered.contains("secret"));
                assert!(!rendered.contains("$system_text"));
            }
        }
    }
    fn revision() -> revisions::Model {
        revisions::Model {
            id: 10,
            workspace_id: 20,
            deployment_id: 30,
            number: 1,
            action: "apply".into(),
            source_revision: 0,
            manifest: "{}".into(),
            rendered: "{}".into(),
            artifact: "{}".into(),
            variable_snapshot: None,
            expected_hash: String::new(),
            state: "succeeded".into(),
            output: String::new(),
            triggered_by: 1,
            started_at: 0,
            finished_at: 0,
            created_at: 0,
            updated_at: 0,
            version: 1,
        }
    }
    #[test]
    fn historical_snapshot_is_encrypted_bound_and_resealed_for_rollback() {
        let directory = tempfile::tempdir().unwrap();
        let vault = crate::vault::Vault::open(directory.path(), false).unwrap();
        let files = BTreeMap::from([(
            ".variables-test-web.env".into(),
            "TOKEN=\"private-fixture\"\n".into(),
        )]);
        let mut old = revision();
        old.variable_snapshot =
            Some(seal_snapshot(&vault, old.workspace_id, old.id, &files).unwrap());
        assert!(
            !old.variable_snapshot
                .as_ref()
                .unwrap()
                .as_str()
                .contains("private-fixture")
        );
        assert_eq!(open_snapshot(&vault, &old).unwrap(), files);
        let mut moved = old.clone();
        moved.workspace_id += 1;
        assert!(open_snapshot(&vault, &moved).is_err());
        moved = old.clone();
        moved.id += 1;
        assert!(open_snapshot(&vault, &moved).is_err());
        moved.variable_snapshot = Some(
            seal_snapshot(
                &vault,
                moved.workspace_id,
                moved.id,
                &open_snapshot(&vault, &old).unwrap(),
            )
            .unwrap(),
        );
        assert_eq!(open_snapshot(&vault, &moved).unwrap(), files);
        assert_ne!(moved.variable_snapshot, old.variable_snapshot);
        assert!(revision_json(&old).get("variable_snapshot").is_none());
        assert!(!revision_json(&old).to_string().contains("private-fixture"));
    }
    #[test]
    fn legacy_revisions_work_but_missing_or_corrupt_bound_snapshots_are_refused() {
        let directory = tempfile::tempdir().unwrap();
        let vault = crate::vault::Vault::open(directory.path(), false).unwrap();
        let mut row = revision();
        assert!(open_snapshot(&vault, &row).unwrap().is_empty());
        row.artifact = json!({"x-variable-bindings":[{"service":"web"}]}).to_string();
        assert!(open_snapshot(&vault, &row).is_err());
        row.variable_snapshot = Some("corrupt".into());
        assert!(open_snapshot(&vault, &row).is_err());
    }
    #[test]
    fn public_artifacts_reference_private_files_and_versions_invalidate_preview() {
        let metadata =
            json!([{"service":"web","environment":"TOKEN","name":"token","id":"1","version":1}]);
        let document = r#"{"services":{"web":{"image":"busybox@sha256:fixture"}}}"#;
        let first = bind_artifact(document, &metadata).unwrap();
        let parsed: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(parsed["services"]["web"]["environment"], Value::Null);
        assert_eq!(artifact_bindings(&first).unwrap(), metadata);
        let mut rotated = metadata.clone();
        rotated[0]["version"] = json!(2);
        let second = bind_artifact(document, &rotated).unwrap();
        assert_ne!(
            preview_token(&first, "hash", 1, 0),
            preview_token(&second, "hash", 1, 0)
        );
        assert_ne!(
            environment_path(&metadata, "web"),
            environment_path(&rotated, "web")
        );
    }
    #[test]
    fn config_preview_literals_survive_binding_without_changing_snapshot_metadata() {
        let metadata = json!([{"kind":"file","service":"web","path":"/etc/app.conf","content":"${HOST} $$ $plain\n"}]);
        let artifact = bind_artifact(r#"{"services":{"web":{}}}"#, &metadata).unwrap();
        assert_eq!(artifact_bindings(&artifact).unwrap(), metadata);
        let doc: Value = serde_json::from_str(&artifact).unwrap();
        assert_eq!(
            doc["x-variable-bindings"][0]["content"],
            "$${HOST} $$$$ $$plain\n"
        );
    }
}
