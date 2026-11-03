pub(crate) mod actions;
pub(crate) mod actions_run;
pub(crate) mod artifacts;
pub(crate) mod concurrency;
pub(crate) mod contexts;
pub(crate) mod reviews;
pub(crate) mod runs;
use crate::entities::{host_groups, hosts, pipelines, repositories};
use crate::error::{Json, Path, Query};
use crate::i18n::Message;
use crate::snowflake::Id;
use crate::{
    accounts::workspace::Scope,
    db,
    error::{ApiError, Result},
    resources::resource::{self, ListQuery, Spec},
    state::AppState,
};
use axum::{extract::State, http::StatusCode};
use sea_orm::{
    ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QuerySelect, QueryTrait, Set,
    sea_query::{Expr, ExprTrait},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

const SPEC: Spec = Spec {
    search: &["name"],
    filters: &["repository_id", "target_kind", "group_id", "application_id"],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "name",
};

fn to_json(row: &pipelines::Model) -> Value {
    let mut value = json!({
        "id": Id::from(row.id),
        "name": row.name,
        "repository_id": Id::from(row.repository_id),
        "group_id": Id::from(row.group_id),
        "application_id": Id::from(row.application_id),
        "definition": row.definition,
        "target_kind": row.target_kind,
        "target_ref": Id::from(row.target_ref),
        "description": row.description,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
        "version": row.version,
    });
    crate::pipelines::actions::display(&mut value, &row.definition);
    value
}

#[derive(Deserialize)]
pub struct Body {
    name: String,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    application_id: Option<String>,
    #[serde(default)]
    tag_ids: Option<Vec<Id>>,
    #[serde(default)]
    environment_ids: Option<Vec<Id>>,
    definition: String,
    #[serde(default)]
    definition_format: String,
    #[serde(default)]
    executor_bindings: Vec<crate::pipelines::actions::Binding>,
    #[serde(default)]
    checkout_bindings: Vec<crate::pipelines::actions::CheckoutBinding>,
    #[serde(default)]
    graph_layout: Value,
    #[serde(default)]
    repository_id: String,
    #[serde(default)]
    repository_selection: Option<crate::repositories::Body>,
    #[serde(default)]
    target_kind: String,
    #[serde(default)]
    target_ref: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    version: i64,
}

impl Body {
    fn stored_definition(&self) -> Result<String> {
        // Scripts that read a pipeline and write back an old JSON definition
        // keep the format label they read; the definition's shape decides.
        let format = if legacy_shaped(&self.definition) {
            "legacy"
        } else {
            self.definition_format.as_str()
        };
        match format {
            "" | "legacy" => {
                validate_definition(&self.definition)?;
                convert_legacy(&self.definition, &self.target_kind, &self.target_ref)
            }
            crate::pipelines::actions::ENGINE => {
                serde_json::to_string(&crate::pipelines::actions::Document {
                    engine: crate::pipelines::actions::ENGINE.into(),
                    workflow: self.definition.clone(),
                    executor_bindings: self.executor_bindings.clone(),
                    checkout_bindings: self.checkout_bindings.clone(),
                    graph_layout: self.graph_layout.clone(),
                })
                .map_err(|_| {
                    ApiError::localized(
                        StatusCode::BAD_REQUEST,
                        Message::new("execution.saveDefinition"),
                    )
                })
            }
            _ => Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.unknownFormat"),
            )),
        }
    }
}

/// An old pipeline definition: a JSON object with steps and no Actions jobs.
fn legacy_shaped(definition: &str) -> bool {
    serde_json::from_str::<Value>(definition).is_ok_and(|value| {
        value.get("steps").is_some_and(Value::is_array) && value.get("jobs").is_none()
    })
}

async fn check(
    s: &AppState,
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    body: &Body,
) -> Result<String> {
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.nameRequired"),
        ));
    }
    crate::pipelines::contexts::check_single(
        conn,
        scope.workspace_id,
        "group",
        Id::reference(body.group_id.as_deref().unwrap_or(""))?,
    )
    .await?;
    crate::pipelines::contexts::check_single(
        conn,
        scope.workspace_id,
        "application",
        Id::reference(body.application_id.as_deref().unwrap_or(""))?,
    )
    .await?;
    let stored = body.stored_definition()?;
    validate_definition(&stored)?;
    if let Some(document) = crate::pipelines::actions::document(&stored) {
        crate::pipelines::actions::check_bindings(s, conn, scope, &document).await?;
        if document
            .checkout_bindings
            .iter()
            .any(crate::pipelines::actions::CheckoutBinding::pipeline_repository)
            && Id::reference(&body.repository_id)? == Id::from(0)
        {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.repositoryRequired"),
            ));
        }
    }
    let node_only = crate::pipelines::actions::document(&stored).is_some()
        && Id::reference(&body.target_ref)? == Id::from(0);
    if node_only {
        let document =
            crate::pipelines::actions::document(&stored).expect("validated Actions document");
        let (jobs, _) = crate::pipelines::actions::parse(&document)?;
        for job in jobs {
            if job.manual {
                continue;
            }
            if !document.executor_bindings.iter().any(|binding| {
                binding.job_id == job.key
                    && ((binding.target_kind == "pool" && binding.target_ref.is_empty())
                        || (!binding.target_ref.is_empty() && binding.target_ref != "0"))
            }) {
                return Err(ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.jobHostRequired").with("name", job.name.clone()),
                ));
            }
        }
    }
    let target_table = match body.target_kind.as_str() {
        "host" => "hosts",
        "host-group" => "host_groups",
        "" if node_only => "hosts",
        _ => {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.targetKind"),
            ));
        }
    };
    for (table, id, noun) in [
        (
            "repositories",
            body.repository_id.trim(),
            "execution.repositoryNoun",
        ),
        (target_table, body.target_ref.trim(), "execution.targetNoun"),
    ] {
        if (table == "repositories" || node_only) && Id::reference(id)? == Id::from(0) {
            continue;
        }
        let id = Id::reference(id)?;
        let exists = match table {
            "repositories" => {
                repositories::Entity::find()
                    .filter(repositories::Column::WorkspaceId.eq(scope.workspace_id))
                    .filter(repositories::Column::Id.eq(id))
                    .count(conn)
                    .await?
            }
            "hosts" => {
                hosts::Entity::find()
                    .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
                    .filter(hosts::Column::Id.eq(id))
                    .count(conn)
                    .await?
            }
            "host_groups" => {
                host_groups::Entity::find()
                    .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
                    .filter(host_groups::Column::Id.eq(id))
                    .count(conn)
                    .await?
            }
            _ => unreachable!("validated target table"),
        };
        if exists == 0 {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.referenceMissing").with_message("noun", Message::new(noun)),
            ));
        }
    }
    Ok(name.to_owned())
}

pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(mut raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("pipelines.read")?;
    let mut select = pipelines::Entity::find();
    for (key, kind) in [("tag_id", "tag"), ("environment_id", "environment")] {
        if let Some(id) = raw.remove(key).filter(|id| !id.is_empty()) {
            use crate::entities::pipeline_links as links;
            let id = Id::reference(&id)?;
            select = select.filter(
                pipelines::Column::Id.in_subquery(
                    links::Entity::find()
                        .select_only()
                        .column(links::Column::PipelineId)
                        .filter(links::Column::Kind.eq(kind))
                        .filter(links::Column::ReferenceId.eq(id))
                        .into_query(),
                ),
            );
        }
    }
    let query = ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) = resource::list(&s.db, &SPEC, select, &scope.workspace_id, &query).await?;
    let mut items = crate::pipelines::contexts::decorate(
        &s.db,
        scope.workspace_id,
        rows.iter().map(to_json).collect(),
    )
    .await?;
    crate::resources::names::attach(
        &s.db,
        scope.workspace_id,
        &mut items,
        &[
            crate::resources::names::of(
                "repository_id",
                "repository_name",
                crate::resources::names::Kind::Repository,
            ),
            crate::resources::names::target("target_ref", "target_name", "target_kind"),
        ],
    )
    .await?;
    Ok(Json(json!({
        "items": items,
        "total": total, "page": query.page, "page_size": query.page_size,
    })))
}

pub async fn get(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("pipelines.read")?;
    let row = pipelines::Entity::find()
        .filter(pipelines::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(pipelines::Column::Id.eq(id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok(Json(
        crate::pipelines::contexts::decorate(&s.db, scope.workspace_id, vec![to_json(&row)])
            .await?
            .remove(0),
    ))
}

pub async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<Body>,
) -> Result<(StatusCode, Json<Value>)> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let result = create_in(&s, &tx, &scope, body).await?;
    tx.commit().await?;
    Ok(result)
}
pub(crate) async fn create_in(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    scope: &Scope,
    mut body: Body,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("pipelines.create")?;
    if let Some(selection) = body.repository_selection.take() {
        body.repository_id = crate::repositories::bind_selection(s, tx, scope, selection).await?;
    }
    let name = check(s, tx, &scope, &body).await?;
    let (id, now) = (s.next_id()?, db::now());
    pipelines::Entity::insert(pipelines::ActiveModel {
        id: Set(i64::from(id)),
        workspace_id: Set(i64::from(scope.workspace_id)),
        name: Set(name.clone()),
        group_id: Set(Id::reference(body.group_id.as_deref().unwrap_or(""))?.into()),
        application_id: Set(Id::reference(body.application_id.as_deref().unwrap_or(""))?.into()),
        repository_id: Set(i64::from(Id::reference(&body.repository_id)?)),
        definition: Set(body.stored_definition()?),
        target_kind: Set(body.target_kind.clone()),
        target_ref: Set(i64::from(Id::reference(&body.target_ref)?)),
        description: Set(body.description.trim().to_owned()),
        created_at: Set(now),
        updated_at: Set(now),
        version: Set(1),
        ..Default::default()
    })
    .exec(tx)
    .await?;
    crate::pipelines::contexts::replace_links(
        tx,
        scope.workspace_id,
        id,
        "tag",
        body.tag_ids.as_deref(),
    )
    .await?;
    crate::pipelines::contexts::replace_links(
        tx,
        scope.workspace_id,
        id,
        "environment",
        body.environment_ids.as_deref(),
    )
    .await?;
    // The details of an audit entry are readable by anyone with activity.read,
    // so they carry the name and never the secret.
    crate::hosts::activity_in(
        &s,
        tx,
        &scope,
        "create",
        "pipelines",
        &id,
        json!({"name": name}),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

pub async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Json(mut body): Json<Body>,
) -> Result<Json<Value>> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("pipelines.update")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let current = pipelines::Entity::find()
        .filter(pipelines::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(pipelines::Column::Id.eq(id))
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    if current.version != body.version {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            Message::new("execution.versionConflict"),
        ));
    }
    if body.environment_ids.is_none() {
        use crate::entities::pipeline_links as links;
        body.environment_ids = Some(
            links::Entity::find()
                .filter(links::Column::PipelineId.eq(current.id))
                .filter(links::Column::Kind.eq("environment"))
                .all(&tx)
                .await?
                .into_iter()
                .map(|link| Id::from(link.reference_id))
                .collect(),
        );
    }
    if body.group_id.is_none() {
        body.group_id = Some(current.group_id.to_string());
    }
    if body.application_id.is_none() {
        body.application_id = Some(current.application_id.to_string());
    }
    if let Some(selection) = body.repository_selection.take() {
        body.repository_id =
            crate::repositories::bind_selection(&s, &tx, &scope, selection).await?;
    }
    let name = check(&s, &tx, &scope, &body).await?;
    let affected = pipelines::Entity::update_many()
        .col_expr(
            pipelines::Column::GroupId,
            Expr::value(Id::reference(body.group_id.as_deref().unwrap_or(""))?),
        )
        .col_expr(
            pipelines::Column::ApplicationId,
            Expr::value(Id::reference(body.application_id.as_deref().unwrap_or(""))?),
        )
        .col_expr(pipelines::Column::Name, Expr::value(name.clone()))
        .col_expr(
            pipelines::Column::RepositoryId,
            Expr::value(Id::reference(&body.repository_id)?),
        )
        .col_expr(
            pipelines::Column::Definition,
            Expr::value(body.stored_definition()?),
        )
        .col_expr(
            pipelines::Column::TargetKind,
            Expr::value(body.target_kind.clone()),
        )
        .col_expr(
            pipelines::Column::TargetRef,
            Expr::value(Id::reference(&body.target_ref)?),
        )
        .col_expr(
            pipelines::Column::Description,
            Expr::value(body.description.trim()),
        )
        .col_expr(pipelines::Column::UpdatedAt, Expr::value(db::now()))
        .col_expr(
            pipelines::Column::Version,
            Expr::col(pipelines::Column::Version).add(1),
        )
        .filter(pipelines::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(pipelines::Column::Id.eq(id))
        .filter(pipelines::Column::Version.eq(body.version))
        .exec(&tx)
        .await?
        .rows_affected;
    if affected == 0 {
        return Err(crate::hosts::stale(
            &s,
            pipelines::Entity::find()
                .filter(pipelines::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(pipelines::Column::Id.eq(id)),
        )
        .await?);
    }
    crate::pipelines::contexts::replace_links(
        &tx,
        scope.workspace_id,
        id,
        "tag",
        body.tag_ids.as_deref(),
    )
    .await?;
    crate::pipelines::contexts::replace_links(
        &tx,
        scope.workspace_id,
        id,
        "environment",
        body.environment_ids.as_deref(),
    )
    .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "pipelines",
        &id,
        json!({"name": name}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn remove(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<StatusCode> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("pipelines.delete")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    crate::cron_jobs::ensure_unreferenced(&tx, scope.workspace_id, "pipeline", &[id]).await?;
    crate::notifications::alerts::ensure_unreferenced(&tx, scope.workspace_id, "pipeline", &[id])
        .await?;
    crate::variables::check_object_removal(&tx, scope.workspace_id, "build", id).await?;
    for app in crate::entities::deployments::Entity::find()
        .filter(crate::entities::deployments::Column::WorkspaceId.eq(scope.workspace_id))
        .all(&tx)
        .await?
    {
        let manifest = crate::deployments::manifest::parse_for(&app.manifest, &app.mode)?;
        if manifest.services.iter().any(|s| {
            s.artifact
                .as_ref()
                .is_some_and(|a| a.pipeline == id.to_string())
        }) {
            return Err(ApiError::localized(
                StatusCode::CONFLICT,
                Message::new("execution.artifactReferenced"),
            ));
        }
    }
    let affected = pipelines::Entity::delete_many()
        .filter(pipelines::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(pipelines::Column::Id.eq(id))
        .exec(&tx)
        .await?
        .rows_affected;
    if affected == 0 {
        return Err(ApiError::missing());
    }
    crate::entities::pipeline_links::Entity::delete_many()
        .filter(crate::entities::pipeline_links::Column::PipelineId.eq(id))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    crate::hosts::activity(&s, &scope, "delete", "pipelines", &id, json!({})).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Compatibility boundary for old exports and scripts. The stored representation
/// is always an Actions document; values are serialized rather than interpolated
/// into YAML so shell text and creation-template parameters remain intact.
pub(crate) fn convert_legacy(input: &str, target_kind: &str, target_ref: &str) -> Result<String> {
    if crate::pipelines::actions::document(input).is_some() {
        return Ok(input.to_owned());
    }
    if !input.contains(".Values.") {
        validate_definition(input)?;
    }
    let legacy: Value = serde_json::from_str(input).map_err(|_| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.definitionJson"),
        )
    })?;
    let object = legacy.as_object().ok_or_else(|| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.definitionObject"),
        )
    })?;
    if object
        .keys()
        .any(|key| !["workdir", "env", "steps", "artifacts"].contains(&key.as_str()))
    {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.definitionUnknownFields"),
        ));
    }
    let workdir = legacy
        .get("workdir")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.workdirString"),
            )
        })?;
    if !workdir.contains(".Values.") {
        crate::hosts::channel::check_path(workdir).map_err(|error| {
            let message = crate::i18n::Message::new("pipelines.workdirInvalid");
            let message = match error.2 {
                Some(cause) => message.with_message("error", cause),
                None => message.with("error", error.1),
            };
            ApiError::localized(axum::http::StatusCode::BAD_REQUEST, message)
        })?;
    }
    let normalized_workdir;
    let workdir = if workdir.starts_with("\\\\") || workdir.as_bytes().get(1) == Some(&b':') {
        normalized_workdir = workdir.replace('\\', "/");
        normalized_workdir.as_str()
    } else {
        workdir
    };
    let source_steps = legacy
        .get("steps")
        .and_then(Value::as_array)
        .filter(|steps| !steps.is_empty())
        .ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.stepsNonempty"),
            )
        })?;
    let mut steps = vec![json!({"id":"prepare_workspace", "name":"Prepare workspace",
        "run":"[ -d \"$FERRY_LEGACY_WORKDIR\" ] || mkdir -p -- \"$FERRY_LEGACY_WORKDIR\"", "shell":"sh",
        "working-directory":"${{ github.workspace }}",
        "env":{"FERRY_LEGACY_WORKDIR":legacy_expressions(workdir)}})];
    let mut checkouts = Vec::new();
    for (index, source) in source_steps.iter().enumerate() {
        if source.as_object().is_none_or(|step| {
            step.keys()
                .any(|key| !["name", "run", "checkout", "timeout_seconds"].contains(&key.as_str()))
        }) {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.stepUnknownFields"),
            ));
        }
        let name = source.get("name").and_then(Value::as_str).ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.stepNameRequired"),
            )
        })?;
        let mut step = json!({"name": legacy_expressions(name)});
        if source
            .get("checkout")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let id = format!("checkout_{}", index + 1);
            step["id"] = json!(id);
            step["uses"] = json!("actions/checkout@v4");
            checkouts.push(crate::pipelines::actions::CheckoutBinding {
                job_id: "build".into(),
                step_id: id,
                connection_id: String::new(),
                repository_url: String::new(),
                git_ref: String::new(),
                path: legacy_expressions(workdir),
            });
        } else {
            let command = source.get("run").and_then(Value::as_str).ok_or_else(|| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.stepCommandRequired"),
                )
            })?;
            step["run"] = json!(legacy_expressions(command));
            step["id"] = json!(format!("legacy_{}", index + 1));
        }
        if let Some(timeout) = source.get("timeout_seconds") {
            step["timeout-minutes"] = if let Some(seconds) = timeout.as_u64() {
                json!(std::cmp::max(seconds.div_ceil(60), 1))
            } else if let Some(token) = timeout
                .as_str()
                .and_then(|text| text.trim().strip_prefix("{{"))
                .and_then(|text| text.strip_suffix("}}"))
            {
                json!(format!("{{{{ ceilMinutes {} }}}}", token.trim()))
            } else {
                return Err(ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.stepTimeoutInvalid"),
                ));
            };
        } else {
            step["timeout-minutes"] = json!(30);
        }
        steps.push(step);
    }
    let mut env = serde_json::Map::new();
    for entry in legacy
        .get("env")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if entry.as_object().is_none_or(|env| {
            env.keys()
                .any(|key| !["name", "value", "valueFrom"].contains(&key.as_str()))
        }) {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.envUnknownFields"),
            ));
        }
        let name = entry.get("name").and_then(Value::as_str).ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.envNameRequired"),
            )
        })?;
        let value = if let Some(reference) = entry.get("valueFrom").and_then(Value::as_str) {
            if let Some(name) = reference.strip_prefix("variable/") {
                // Explicit secrets references use the same scoped variable lookup
                // for both ordinary and encrypted values, including env overrides.
                format!("${{{{ secrets['{}'] }}}}", name.replace('\'', "''"))
            } else if reference.contains(".Values.") {
                reference.to_owned()
            } else {
                return Err(ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.variableReferenceInvalid"),
                ));
            }
        } else {
            legacy_expressions(entry.get("value").and_then(Value::as_str).ok_or_else(|| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.envValueRequired"),
                )
            })?)
        };
        env.insert(name.into(), json!(value));
    }
    for path in legacy
        .get("artifacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let path = path.as_str().ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.artifactPathString"),
            )
        })?;
        steps.push(json!({"name":format!("Archive {path}"), "if":"success()",
        "uses":"actions/upload-artifact@v4", "with": {
            "name":crate::pipelines::artifacts::legacy_artifact_name(path),
            "path":format!("{}/{}", legacy_expressions(workdir).trim_end_matches('/'), path),
            "if-no-files-found":"error", "include-hidden-files":true
        }}));
    }
    let timeout_minutes: u64 = source_steps
        .iter()
        .map(|step| {
            step["timeout_seconds"]
                .as_u64()
                .unwrap_or(1800)
                .div_ceil(60)
        })
        .sum::<u64>()
        + 30;
    let workflow = json!({"jobs":{"build":{"runs-on":"self-hosted", "env":env,
        "timeout-minutes":timeout_minutes,
        "defaults":{"run":{"working-directory":legacy_expressions(workdir), "shell":"sh {0}"}},
        "steps":steps}}});
    let document = crate::pipelines::actions::Document {
        engine: crate::pipelines::actions::ENGINE.into(),
        workflow: serde_yaml_ng::to_string(&workflow).map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.convertFailed"),
            )
        })?,
        executor_bindings: vec![crate::pipelines::actions::Binding {
            job_id: "build".into(),
            target_kind: target_kind.into(),
            target_ref: target_ref.into(),
            image: "host".into(),
        }],
        checkout_bindings: checkouts,
        graph_layout: Value::Null,
    };
    serde_json::to_string(&document).map_err(|_| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.convertFailed"),
        )
    })
}

fn legacy_expressions(value: &str) -> String {
    // One scan: inserted expressions must never be interpreted as input tokens.
    let mut output = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let token = &rest[start..];
        let replacement = [
            ("{{run}}", "${{ github.run_number }}"),
            ("{{branch}}", "${{ github.ref_name }}"),
            ("{{commit}}", "${{ github.sha }}"),
            ("{{repo_url}}", "${{ env.FERRY_REPOSITORY_URL }}"),
        ]
        .into_iter()
        .find(|(old, _)| token.starts_with(old));
        if let Some((old, new)) = replacement {
            output.push_str(new);
            rest = &token[old.len()..];
        } else {
            output.push_str("{{");
            rest = &token[2..];
        }
    }
    output.push_str(rest);
    output
}

#[derive(Debug, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub workdir: String,
    #[serde(default)]
    pub env: Vec<Environment>,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub artifacts: Vec<String>,
}

#[derive(Debug, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, rename = "valueFrom", skip_serializing_if = "Option::is_none")]
    pub value_from: Option<String>,
}

#[derive(Debug, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub run: String,
    #[serde(default)]
    pub checkout: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
}

/// Only exact tokens are supported; no whitespace folding, nesting or expressions.
fn check_templates(input: &str, context: &Message) -> Result<()> {
    let mut remaining = input;
    while let Some(start) = remaining.find("{{") {
        let token = &remaining[start + 2..];
        let Some(end) = token.find("}}") else {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.templateUnclosed")
                    .with_message("context", context.clone())
                    .with("token", format!("{{{{{token}")),
            ));
        };
        let variable = &token[..end];
        if !["run", "branch", "repo_url", "commit"].contains(&variable) {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.templateUnsupported")
                    .with_message("context", context.clone())
                    .with("token", format!("{{{{{variable}}}}}")),
            ));
        }
        remaining = &token[end + 2..];
    }
    Ok(())
}

pub fn validate_definition(input: &str) -> Result<Definition> {
    if let Some(document) = crate::pipelines::actions::document(input) {
        return crate::pipelines::actions::summary(&document);
    }
    if input.len() > 64 * 1024 {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.definitionTooLarge"),
        ));
    }
    let value: Value = serde_json::from_str(input).map_err(|_| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.definitionJson"),
        )
    })?;
    let object = value.as_object().ok_or_else(|| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.definitionObject"),
        )
    })?;
    let steps = object
        .get("steps")
        .and_then(Value::as_array)
        .filter(|steps| !steps.is_empty() && steps.len() <= 50)
        .ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.stepsRange"),
            )
        })?;
    for (index, step) in steps.iter().enumerate() {
        let name = step.get("name").and_then(Value::as_str).unwrap_or("");
        let context = Message::new("execution.stepContext")
            .with("index", (index + 1).to_string())
            .with("name", name);
        if name.trim().is_empty() || name.chars().count() > 60 {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.stepNameInvalid").with_message("context", context.clone()),
            ));
        }
        let run = step.get("run").and_then(Value::as_str).unwrap_or("");
        let checkout = step
            .get("checkout")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if checkout && step.get("run").is_some() {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.stepCheckoutRun").with_message("context", context.clone()),
            ));
        }
        if !checkout && (run.trim().is_empty() || run.len() > 8 * 1024 || run.contains('\0')) {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.stepCommandInvalid")
                    .with_message("context", context.clone()),
            ));
        }
        if let Some(timeout) = step.get("timeout_seconds")
            && !timeout.as_u64().is_some_and(|n| (1..=86400).contains(&n))
        {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.stepTimeoutRange").with_message("context", context.clone()),
            ));
        }
        check_templates(name, &context)?;
        check_templates(run, &context)?;
        serde_json::from_value::<Step>(step.clone()).map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.stepFieldsInvalid")
                    .with_message("context", context.clone()),
            )
        })?;
    }
    let workdir = object
        .get("workdir")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.workdirString"),
            )
        })?;
    crate::hosts::channel::check_path(workdir).map_err(|error| {
        let message = crate::i18n::Message::new("pipelines.workdirInvalid");
        let message = match error.2 {
            Some(cause) => message.with_message("error", cause),
            None => message.with("error", error.1),
        };
        ApiError::localized(axum::http::StatusCode::BAD_REQUEST, message)
    })?;
    check_templates(workdir, &Message::new("execution.workdirContext"))?;
    if let Some(env) = object.get("env") {
        let env = env.as_array().ok_or_else(|| {
            ApiError::localized(StatusCode::BAD_REQUEST, Message::new("execution.envArray"))
        })?;
        let mut names = std::collections::HashSet::new();
        for (index, item) in env.iter().enumerate() {
            let context =
                Message::new("execution.envContext").with("index", (index + 1).to_string());
            let entry: Environment = serde_json::from_value(item.clone()).map_err(|_| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.envEntryInvalid")
                        .with_message("context", context.clone()),
                )
            })?;
            let mut bytes = entry.name.bytes();
            let valid = bytes
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_');
            if !valid {
                return Err(ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.envNameInvalid")
                        .with_message("context", context.clone()),
                ));
            }
            if !names.insert(entry.name.clone()) {
                return Err(ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.envNameDuplicate")
                        .with_message("context", context.clone())
                        .with("name", entry.name.clone()),
                ));
            }
            match (&entry.value, &entry.value_from) {
                (Some(value), None) => {
                    if value.contains('\0') {
                        return Err(ApiError::localized(
                            StatusCode::BAD_REQUEST,
                            Message::new("execution.envNull")
                                .with_message("context", context.clone()),
                        ));
                    }
                    check_templates(value, &context)?;
                }
                (None, Some(reference))
                    if reference
                        .strip_prefix("variable/")
                        .is_some_and(crate::variables::valid_name) => {}
                _ => {
                    return Err(ApiError::localized(
                        StatusCode::BAD_REQUEST,
                        Message::new("execution.envValueChoice")
                            .with_message("context", context.clone()),
                    ));
                }
            }
        }
    }
    let definition: Definition = serde_json::from_value(value).map_err(|_| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.definitionFieldsInvalid"),
        )
    })?;
    crate::pipelines::artifacts::validate_paths(&definition.artifacts)?;
    Ok(definition)
}

pub fn variable_names(definition: &Definition) -> std::collections::BTreeSet<String> {
    definition
        .env
        .iter()
        .filter_map(|env| {
            env.value_from
                .as_deref()
                .and_then(|reference| reference.strip_prefix("variable/"))
                .map(str::to_owned)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn template_errors_preserve_nested_identity_and_literal_parameters() {
        use crate::i18n::{Locale, Message};
        let name = "user {token} {{commit}}";
        let context = Message::new("execution.stepContext")
            .with("index", "2")
            .with("name", name);
        let error = super::check_templates("{{unknown}}", &context).unwrap_err();
        let message = error.2.as_ref().unwrap();
        assert_eq!(message.key, "execution.templateUnsupported");
        assert_eq!(message.children["context"].key, "execution.stepContext");
        assert_eq!(message.children["context"].params["name"], name);
        assert_eq!(message.params["token"], "{{unknown}}");
        for locale in Locale::ALL {
            let displayed = error.render(locale);
            assert!(displayed.contains(name), "{}: {displayed}", locale.tag());
            assert!(displayed.contains("{{unknown}}"));
            assert_eq!(
                crate::i18n::render_stored(locale, &error.stored()),
                displayed
            );
        }
        assert_ne!(error.render(Locale::EnUs), error.render(Locale::JaJp));
    }

    use super::*;

    #[test]
    fn legacy_definitions_are_recognised_by_shape() {
        assert!(super::legacy_shaped(
            r#"{"workdir":"/w","steps":[{"name":"a","run":"true"}]}"#
        ));
        assert!(!super::legacy_shaped(
            "on: workflow_dispatch
jobs:
  a:
    runs-on: self-hosted
    steps: []
"
        ));
        assert!(!super::legacy_shaped(
            r#"{"on":"workflow_dispatch","jobs":{"a":{"steps":[]}},"steps":[]}"#
        ));
        assert!(!super::legacy_shaped("{{ .Values.x }}"));
    }

    #[test]
    fn legacy_conversion_is_deterministic_complete_and_idempotent() {
        let legacy = json!({"workdir":"/opt/build/{{run}}", "env":[
            {"name":"TOKEN","valueFrom":"variable/token-with-dash"},
            {"name":"LITERAL","value":"{{branch}} {{commit}} {{repo_url}}"}],
            "steps":[{"name":"Source {{branch}}","checkout":true},
                {"name":"Build {{run}}","run":"printf '%s' '{{commit}}'", "timeout_seconds":61}],
            "artifacts":["bin/app", ".hidden"]})
        .to_string();
        let converted = convert_legacy(&legacy, "host-group", "42").unwrap();
        assert_eq!(
            convert_legacy(&legacy, "host-group", "42").unwrap(),
            converted
        );
        assert_eq!(convert_legacy(&converted, "host", "99").unwrap(), converted);
        let doc = crate::pipelines::actions::document(&converted).unwrap();
        let (jobs, _) = crate::pipelines::actions::parse(&doc).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(doc.executor_bindings[0].target_kind, "host-group");
        assert_eq!(doc.executor_bindings[0].target_ref, "42");
        assert_eq!(doc.executor_bindings[0].image, "host");
        let value: Value = serde_yaml_ng::from_str(&doc.workflow).unwrap();
        let job = &value["jobs"]["build"];
        assert_eq!(
            job["defaults"]["run"]["working-directory"],
            "/opt/build/${{ github.run_number }}"
        );
        assert_eq!(job["env"]["TOKEN"], "${{ secrets['token-with-dash'] }}");
        assert_eq!(
            job["env"]["LITERAL"],
            "${{ github.ref_name }} ${{ github.sha }} ${{ env.FERRY_REPOSITORY_URL }}"
        );
        assert!(doc.checkout_bindings[0].pipeline_repository());
        assert_eq!(
            doc.checkout_bindings[0].path,
            "/opt/build/${{ github.run_number }}"
        );
        let steps = job["steps"].as_array().unwrap();
        assert_eq!(steps[2]["run"], "printf '%s' '${{ github.sha }}'");
        assert_eq!(steps[1]["timeout-minutes"], 30);
        assert_eq!(steps[2]["timeout-minutes"], 2);
        assert_eq!(
            steps[3]["with"]["name"],
            crate::pipelines::artifacts::legacy_artifact_name("bin/app")
        );
        assert_eq!(steps[3]["if"], "success()");
        assert_eq!(steps[4]["with"]["include-hidden-files"], true);
    }

    #[test]
    fn legacy_timeout_conversion_rounds_up_and_keeps_windows_absolute_path() {
        for (seconds, minutes) in [(1, 1), (60, 1), (61, 2), (86400, 1440)] {
            let legacy = json!({"workdir":"C:/build", "steps":[{"name":"test", "run":"true", "timeout_seconds":seconds}]});
            let doc = crate::pipelines::actions::document(
                &convert_legacy(&legacy.to_string(), "host", "1").unwrap(),
            )
            .unwrap();
            let value: Value = serde_yaml_ng::from_str(&doc.workflow).unwrap();
            assert_eq!(
                value["jobs"]["build"]["steps"][1]["timeout-minutes"],
                minutes
            );
            assert_eq!(
                value["jobs"]["build"]["defaults"]["run"]["working-directory"],
                "C:/build"
            );
        }
    }

    fn valid() -> Value {
        json!({"workdir":"/tmp/{{run}}","env":[{"name":"LOG_LEVEL","value":"info"}],
            "steps":[{"name":"构建","run":"echo {{branch}} {{repo_url}} {{commit}}","timeout_seconds":1800}]})
    }
    fn accepts(value: &Value) {
        validate_definition(&value.to_string()).unwrap();
    }
    fn rejects(value: &Value, message: &str) {
        let error = validate_definition(&value.to_string()).unwrap_err();
        let displayed = error.render(crate::i18n::Locale::ZhCn);
        assert!(displayed.contains(message), "{displayed}");
        assert!(displayed.ends_with('。'), "{displayed}");
    }

    #[test]
    fn actions_envelope_preserves_yaml_and_validates_dependencies() {
        let workflow = "# keep this comment\njobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: echo hello\n";
        let envelope = json!({"engine":"github-actions", "workflow":workflow});
        let parsed = validate_definition(&envelope.to_string()).unwrap();
        assert_eq!(parsed.steps.len(), 1);
        let invalid = json!({"engine":"github-actions", "workflow":workflow.replace("    runs-on:", "    needs: missing\n    runs-on:")});
        assert!(
            validate_definition(&invalid.to_string())
                .unwrap_err()
                .render(crate::i18n::Locale::ZhCn)
                .contains("不存在")
        );
    }

    #[test]
    fn valid_definition_and_optional_fields() {
        accepts(&valid());
        let mut v = valid();
        v.as_object_mut().unwrap().remove("env");
        v["steps"][0]
            .as_object_mut()
            .unwrap()
            .remove("timeout_seconds");
        accepts(&v);
    }

    #[test]
    fn checkout_steps_round_trip_and_exclude_commands() {
        let mut value = valid();
        value["steps"][0] = json!({"name":"拉取代码","checkout":true});
        let parsed = validate_definition(&value.to_string()).unwrap();
        assert!(parsed.steps[0].checkout);
        accepts(&serde_json::to_value(parsed).unwrap());
        value["steps"][0]["run"] = json!("echo forbidden");
        rejects(&value, "不能同时填写");
        value["steps"][0].as_object_mut().unwrap().remove("run");
        value["steps"][0]["checkout"] = json!(false);
        rejects(&value, "命令");
        value["steps"][0]["checkout"] = json!("true");
        rejects(&value, "命令");
    }

    #[test]
    fn invalid_json_and_root_shapes() {
        assert!(validate_definition("{").unwrap_err().1.contains("JSON"));
        for v in [json!(null), json!([]), json!("text"), json!(1)] {
            rejects(&v, "对象");
        }
    }

    #[test]
    fn steps_required_array_and_count_boundary() {
        let mut v = valid();
        v.as_object_mut().unwrap().remove("steps");
        rejects(&v, "steps");
        for bad in [json!(null), json!({}), json!("x"), json!([])] {
            v["steps"] = bad;
            rejects(&v, "steps");
        }
        v["steps"] = json!(vec![json!({"name":"构建","run":"true"}); 50]);
        accepts(&v);
        v["steps"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"第51步","run":"true"}));
        rejects(&v, "50");
    }

    #[test]
    fn step_fields_required_and_typed_with_step_number() {
        for field in ["name", "run"] {
            let mut v = valid();
            v["steps"]
                .as_array_mut()
                .unwrap()
                .insert(0, json!({"name":"第一步","run":"true"}));
            v["steps"][1].as_object_mut().unwrap().remove(field);
            rejects(&v, "第 2 步");
            for bad in [json!(null), json!(1), json!([]), json!(""), json!(" \n\t")] {
                v["steps"][1][field] = bad;
                rejects(&v, "第 2 步");
            }
        }
        let mut v = valid();
        v["steps"][0] = json!(null);
        rejects(&v, "第 1 步");
    }

    #[test]
    fn name_character_and_run_byte_boundaries() {
        let mut v = valid();
        v["steps"][0]["name"] = json!("构".repeat(60));
        v["steps"][0]["run"] = json!("a".repeat(8192));
        accepts(&v);
        v["steps"][0]["name"] = json!("构".repeat(61));
        rejects(&v, "名称");
        v["steps"][0]["name"] = json!("构建");
        v["steps"][0]["run"] = json!("a".repeat(8193));
        rejects(&v, "8KB");
        v["steps"][0]["run"] = json!("构".repeat(2731));
        rejects(&v, "8KB");
        v["steps"][0]["run"] = json!("echo\0x");
        rejects(&v, "空字符");
    }

    #[test]
    fn timeout_integer_and_bounds() {
        let mut v = valid();
        for n in [json!(1), json!(86400)] {
            v["steps"][0]["timeout_seconds"] = n;
            accepts(&v);
        }
        for n in [
            json!(0),
            json!(-1),
            json!(86401),
            json!(1.5),
            json!(1.0),
            json!("10"),
            json!(null),
            json!(true),
            json!(u64::MAX),
        ] {
            v["steps"][0]["timeout_seconds"] = n;
            rejects(&v, "第 1 步「构建」的超时时间");
        }
    }

    #[test]
    fn workdir_uses_shared_path_rules() {
        let mut v = valid();
        v.as_object_mut().unwrap().remove("workdir");
        rejects(&v, "workdir");
        for path in [
            "",
            "relative",
            "../tmp",
            "/tmp/../etc",
            "/tmp/\0x",
            &format!("/{}", "a".repeat(4096)),
        ] {
            v["workdir"] = json!(path);
            rejects(&v, "工作目录");
        }
        v["workdir"] = json!(1);
        rejects(&v, "workdir");
    }

    #[test]
    fn environment_shape_names_duplicates_and_values() {
        let mut v = valid();
        for bad in [json!(null), json!({}), json!("x")] {
            v["env"] = bad;
            rejects(&v, "env");
        }
        for name in ["", "1A", "A-B", "A B", "中文", "A\n", "A=x"] {
            v["env"] = json!([{"name":name,"value":"x"}]);
            rejects(&v, "名称");
        }
        for bad in [
            json!({}),
            json!({"name":"A"}),
            json!({"name":"A","value":1}),
        ] {
            v["env"] = json!([bad]);
            rejects(&v, "环境变量");
        }
        v["env"] = json!([{"name":"A","value":"x"},{"name":"A","value":"y"}]);
        rejects(&v, "重复");
        v["env"] = json!([{"name":"_A1","value":"x"},{"name":"a","value":"y"}]);
        accepts(&v);
        v["env"][0]["value"] = json!("a\0b");
        rejects(&v, "空字符");
    }

    #[test]
    fn variable_references_are_explicit_and_exclusive() {
        let mut v = valid();
        for invalid in [
            json!({"name":"A","value":"x","valueFrom":"variable/token"}),
            json!({"name":"A","valueFrom":"token"}),
            json!({"name":"A","valueFrom":"variable/"}),
            json!({"name":"A","valueFrom":"variable/token with spaces"}),
        ] {
            v["env"] = json!([invalid]);
            rejects(&v, "环境变量");
        }
        v["env"] = json!([{"name":"A","valueFrom":"variable/token"},{"name":"B","value":""}]);
        let definition = validate_definition(&v.to_string()).unwrap();
        assert_eq!(
            variable_names(&definition),
            std::collections::BTreeSet::from(["token".into()])
        );
    }

    #[test]
    fn template_whitelist_rejects_adversarial_tokens_everywhere() {
        for token in [
            "{{unknown}}",
            "{{ run }}",
            "{{RUN}}",
            "{{a{{b}}}}",
            "{{}}",
            "{{run",
        ] {
            for field in ["run", "name", "workdir", "env"] {
                let mut v = valid();
                match field {
                    "run" | "name" => v["steps"][0][field] = json!(token),
                    "workdir" => v["workdir"] = json!(format!("/tmp/{token}")),
                    _ => v["env"][0]["value"] = json!(token),
                }
                let error = validate_definition(&v.to_string()).unwrap_err();
                let displayed = error.render(crate::i18n::Locale::ZhCn);
                assert!(displayed.contains("模板变量"), "{field}: {error}");
                assert!(displayed.contains(token.trim_end_matches('}')), "{error}");
                if field == "run" || field == "name" {
                    assert!(displayed.contains("第 1 步"), "{error}");
                }
            }
        }
        let mut v = valid();
        v["steps"][0]["run"] = json!("{{run}}{{branch}}{{repo_url}}{{commit}} {{typo}}");
        rejects(&v, "{{typo}}");
    }

    #[test]
    fn shell_syntax_is_intentionally_allowed() {
        // run is an operator-authored shell program, not a single argv value.
        // Newlines, semicolons, backticks and $() must remain usable; banning
        // them would break normal build scripts, not establish a sandbox.
        let mut v = valid();
        v["steps"][0]["run"] = json!("echo hello\necho world; echo `date`; echo $(pwd)");
        accepts(&v);
    }

    #[test]
    fn definition_size_exact_boundary() {
        let mut v = valid();
        v["env"][0]["value"] = json!("");
        let padding = 65536 - v.to_string().len();
        v["env"][0]["value"] = json!("x".repeat(padding));
        assert_eq!(v.to_string().len(), 65536);
        accepts(&v);
        v["env"][0]["value"] = json!("x".repeat(padding + 1));
        rejects(&v, "64KB");
    }

    #[test]
    fn unknown_fields_are_not_silently_ignored() {
        let mut v = valid();
        v["steps"][0]["timeout_second"] = json!(10);
        rejects(&v, "第 1 步");
        let mut v = valid();
        v["parallel"] = json!(true);
        rejects(&v, "字段");
    }
}
