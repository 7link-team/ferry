//! Durable code reviews orchestrated through the existing Actions execution engine.
use crate::{
    accounts::workspace::Scope,
    db,
    error::{ApiError, Json, Path, Query, Result},
    snowflake::Id,
    state::AppState,
};
use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{get, patch, post},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
    QuerySelect, Set,
    sea_query::{Expr, ExprTrait},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
mod audit;
pub(crate) mod entities;
pub(crate) mod migration;
mod template;
mod worker;
use entities::{definitions as defs, findings, jobs};
pub(crate) use worker::scheduler;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/{workspace}/code-reviews", get(list).post(create))
        .route(
            "/{workspace}/code-reviews/{id}",
            get(detail).patch(update).delete(remove),
        )
        .route("/{workspace}/code-reviews/{id}/run", post(run))
        .route("/{workspace}/code-reviews/{id}/runs", get(run_list))
        .route(
            "/{workspace}/code-reviews/{id}/runs/{run_id}",
            get(run_detail),
        )
        .route(
            "/{workspace}/code-reviews/{id}/runs/{run_id}/cancel",
            post(run_cancel),
        )
        .route("/{workspace}/code-reviews/{id}/findings", get(finding_list))
        .route(
            "/{workspace}/code-reviews/{id}/findings/{finding_id}",
            patch(finding_update),
        )
}
fn bad(reason: &'static str) -> ApiError {
    let key = match reason {
        "config" => Some("reviews.error.config"),
        "testCommand" => Some("reviews.error.tests"),
        "modelEndpoint" => Some("reviews.error.modelEndpoint"),
        "modelCredential" => Some("reviews.error.modelCredential"),
        "shaRequired" => Some("reviews.error.sha"),
        "selection"
        | "selectionEmpty"
        | "connection"
        | "selectionRevoked"
        | "repositoryListing"
        | "repositoryListingLimit" => Some("reviews.error.selection"),
        "gitSelection" => Some("repository.monitor.gitRepositoriesRequired"),
        "junitMissing" | "junitInvalid" => Some("reviews.error.junit"),
        "artifactContract" | "artifactIntegrity" | "artifactArchive" | "artifactLimit"
        | "inputMissing" | "inputContract" => Some("reviews.error.artifact"),
        "budget" => Some("reviews.error.budget"),
        "modelRequest" | "modelHttp" | "modelContract" | "modelIncomplete" => {
            Some("reviews.error.model")
        }
        "branchMissing" => Some("reviews.error.branch"),
        "disabled" => Some("reviews.error.disabled"),
        "launchUncertain" => Some("reviews.error.launch"),
        _ => None,
    };
    if let Some(key) = key {
        return ApiError::localized(StatusCode::BAD_REQUEST, crate::i18n::Message::new(key));
    }
    ApiError::localized(
        StatusCode::BAD_REQUEST,
        crate::i18n::Message::new("reviews.invalid").with("reason", reason),
    )
}
fn conflict() -> ApiError {
    ApiError::localized(
        StatusCode::CONFLICT,
        crate::i18n::Message::new("execution.versionConflict"),
    )
}
fn encode<T: Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(|_| bad("serialization"))
}
fn decode<T: serde::de::DeserializeOwned>(s: &str) -> Result<T> {
    serde_json::from_str(s).map_err(|_| bad("storedContract"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Config {
    enabled: bool,
    trigger: String,
    branch: String,
    repositories: Vec<crate::repositories::analysis::insights::Selection>,
    tests_enabled: bool,
    ai_enabled: bool,
    test_command: String,
    junit_path: String,
    runner_kind: String,
    runner_ref: String,
    runner_labels: Vec<String>,
    ai_endpoint: String,
    ai_model: String,
    ai_credential_id: String,
    ai_timeout_seconds: u64,
    ai_max_tokens: u32,
    ai_concurrency: u32,
    max_files: usize,
    max_bytes: usize,
    include: Vec<String>,
    exclude: Vec<String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            trigger: "manual".into(),
            branch: "main".into(),
            repositories: vec![],
            tests_enabled: false,
            ai_enabled: false,
            test_command: String::new(),
            junit_path: "junit.xml".into(),
            runner_kind: "pool".into(),
            runner_ref: String::new(),
            runner_labels: vec!["self-hosted".into()],
            ai_endpoint: String::new(),
            ai_model: String::new(),
            ai_credential_id: String::new(),
            ai_timeout_seconds: 60,
            ai_max_tokens: 32768,
            ai_concurrency: 1,
            max_files: 50,
            max_bytes: 24000,
            include: vec![],
            exclude: vec!["**/node_modules/**".into(), "**/vendor/**".into()],
        }
    }
}
#[derive(Deserialize)]
struct Body {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    version: i64,
    #[serde(flatten)]
    config: Config,
}

async fn validate(s: &AppState, scope: &Scope, b: &Body) -> Result<()> {
    let c = &b.config;
    scope.require("repositories.read")?;
    scope.require("credentials.read")?;
    scope.require("pipelines.run")?;
    if b.name.trim().is_empty()
        || b.name.chars().count() > 60
        || b.description.len() > 2000
        || !["manual", "push"].contains(&c.trigger.as_str())
        || c.branch.is_empty()
        || c.branch.len() > 255
        || c.branch.chars().any(char::is_control)
        || c.repositories.is_empty()
        || c.repositories.len() > 100
        || (!c.tests_enabled && !c.ai_enabled)
        || !["pool", "host", "host-group"].contains(&c.runner_kind.as_str())
        || c.max_files == 0
        || c.max_files > 1000
        || c.max_bytes == 0
        || c.max_bytes > 2_000_000
        || c.ai_concurrency == 0
        || c.ai_concurrency > 16
        || c.ai_timeout_seconds == 0
        || c.ai_timeout_seconds > 300
        || c.ai_max_tokens < 1024
        || c.ai_max_tokens > 1_000_000
        || c.include.len() + c.exclude.len() > 100
        || c.include
            .iter()
            .chain(&c.exclude)
            .any(|p| p.len() > 1024 || p.chars().any(char::is_control))
    {
        return Err(bad("config"));
    }
    if c.tests_enabled {
        if c.test_command.trim().is_empty() || c.test_command.len() > 65536 {
            return Err(bad("testCommand"));
        }
        if !c.junit_path.is_empty() {
            crate::pipelines::artifacts::validate_paths(&[c.junit_path.clone()])?;
        }
    }
    if c.ai_enabled {
        let url = reqwest::Url::parse(&c.ai_endpoint).map_err(|_| bad("modelEndpoint"))?;
        if !["http", "https"].contains(&url.scheme())
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || c.ai_model.trim().is_empty()
            || c.ai_model.len() > 255
        {
            return Err(bad("modelEndpoint"));
        }
        let id: Id = c
            .ai_credential_id
            .parse()
            .map_err(|_| bad("modelCredential"))?;
        crate::credentials::unseal(s, &scope.workspace_id, &id).await?;
    }
    let mut seen = BTreeSet::new();
    for selection in &c.repositories {
        let connection = crate::repositories::auth::owned_connection(
            &s.db,
            scope,
            selection
                .connection_id
                .parse()
                .map_err(|_| bad("connection"))?,
        )
        .await?;
        if connection.state != "connected" {
            return Err(ApiError::forbidden());
        }
        if !selection.pipeline_id.is_empty()
            || !seen.insert((
                &selection.connection_id,
                &selection.repository,
                selection.all_repositories,
            ))
        {
            return Err(bad("selection"));
        }
        if connection.provider == "git" {
            if selection.all_repositories || selection.repository.ends_with("/*") {
                return Err(bad("gitSelection"));
            }
            crate::repositories::auth::resolve_connection(
                s,
                &s.db,
                scope,
                connection.id.into(),
                &selection.repository,
            )
            .await?;
        } else if !selection.all_repositories
            && (selection.repository.is_empty()
                || selection.repository.len() > 400
                || !selection.repository.contains('/'))
        {
            return Err(bad("selection"));
        }
    }
    let d = template::document(c)?;
    // URL is resolved from the authorized selection on every run; check executor
    // references now without inventing a repository or sharing a creator's token.
    let mut check = d.clone();
    check.checkout_bindings.clear();
    crate::pipelines::actions::check_bindings(s, &s.db, scope, &check).await?;
    Ok(())
}
async fn owned(s: &AppState, scope: &Scope, id: Id) -> Result<defs::Model> {
    defs::Entity::find_by_id(i64::from(id))
        .filter(defs::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(defs::Column::UserId.eq(scope.user.id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)
}
fn definition_json(row: &defs::Model) -> Result<Value> {
    let mut v: Value = decode(&row.config)?;
    let baselines: BTreeMap<String, String> = decode(&row.baselines)?;
    v["trigger_error"] = json!(crate::i18n::display_stored(
        baselines.get("_error").map(String::as_str).unwrap_or("")
    ));
    for (k, x) in [
        ("id", json!(Id::from(row.id))),
        ("name", json!(row.name)),
        ("description", json!(row.description)),
        ("version", json!(row.version)),
        ("pipeline_id", json!(Id::from(row.pipeline_id))),
        ("created_at", json!(row.created_at)),
        ("updated_at", json!(row.updated_at)),
    ] {
        v[k] = x;
    }
    Ok(v)
}
const SPEC: crate::resources::resource::Spec = crate::resources::resource::Spec {
    search: &["name"],
    filters: &[],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "name",
};
async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("code-reviews.read")?;
    let q = crate::resources::resource::ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) = crate::resources::resource::list(
        &s.db,
        &SPEC,
        defs::Entity::find().filter(defs::Column::UserId.eq(scope.user.id)),
        &scope.workspace_id,
        &q,
    )
    .await?;
    let mut items = Vec::new();
    for r in rows {
        items.push(definition_json(&r)?);
    }
    Ok(Json(
        json!({"items":items,"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
async fn detail(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("code-reviews.read")?;
    let r = owned(&s, &scope, id).await?;
    authorize_config(&s, &scope, &decode(&r.config)?).await?;
    Ok(Json(definition_json(&r)?))
}
async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Json(b): Json<Body>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("code-reviews.create")?;
    scope.require("pipelines.create")?;
    Ok((StatusCode::CREATED, Json(save(&s, &scope, None, b).await?)))
}
async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(b): Json<Body>,
) -> Result<Json<Value>> {
    scope.require("code-reviews.update")?;
    Ok(Json(save(&s, &scope, Some(id), b).await?))
}
async fn save(s: &AppState, scope: &Scope, id: Option<Id>, b: Body) -> Result<Value> {
    validate(s, scope, &b).await?;
    let old = match id {
        Some(id) => Some(owned(s, scope, id).await?),
        None => None,
    };
    if old.as_ref().map(|r| r.version).unwrap_or(0) != b.version {
        return Err(conflict());
    }
    let now = db::now();
    let review_id = match id {
        Some(id) => id,
        None => s.next_id()?,
    };
    // Keep the generated workflow editable with a real representative source.
    // Every execution replaces this checkout with its own fixed source and SHA.
    let mut generated = template::document(&b.config)?;
    let representative = worker::sources(s, scope, &b.config)
        .await?
        .into_iter()
        .next();
    let binding = &mut generated.checkout_bindings[0];
    if let Some((connection, _, url)) = representative {
        binding.connection_id = connection.to_string();
        binding.repository_url = url;
    } else {
        binding.connection_id.clear();
        binding.repository_url.clear();
    }
    crate::pipelines::actions::parse(&generated)?;
    let tx = crate::variables::transaction(s, scope.workspace_id).await?;
    let pipeline = if let Some(row) = &old {
        row.pipeline_id
    } else {
        let id = s.next_id()?;
        crate::entities::pipelines::Entity::insert(crate::entities::pipelines::ActiveModel {
            id: Set(id.into()),
            workspace_id: Set(scope.workspace_id.into()),
            name: Set(b.name.trim().to_owned()),
            group_id: Set(0),
            application_id: Set(0),
            repository_id: Set(0),
            definition: Set(encode(&generated)?),
            target_kind: Set(String::new()),
            target_ref: Set(0),
            description: Set("Code review template".into()),
            created_at: Set(now),
            updated_at: Set(now),
            version: Set(1),
            ..Default::default()
        })
        .exec(&tx)
        .await?;
        i64::from(id)
    };
    // An edited dedicated workflow remains editable. Rebuild only when test or
    // executor configuration changes, never on description/AI-only edits.
    if let Some(row) = &old {
        let previous: Config = decode(&row.config)?;
        let a = template::document(&previous)?;
        let bdoc = template::document(&b.config)?;
        if encode(&a)? != encode(&bdoc)? {
            scope.require("pipelines.update")?;
            crate::entities::pipelines::Entity::update_many()
                .col_expr(
                    crate::entities::pipelines::Column::Definition,
                    Expr::value(encode(&generated)?),
                )
                .col_expr(
                    crate::entities::pipelines::Column::Version,
                    Expr::col(crate::entities::pipelines::Column::Version).add(1),
                )
                .filter(crate::entities::pipelines::Column::Id.eq(pipeline))
                .filter(crate::entities::pipelines::Column::WorkspaceId.eq(scope.workspace_id))
                .exec(&tx)
                .await?;
        }
    }
    let model = defs::Model {
        id: review_id.into(),
        workspace_id: scope.workspace_id.into(),
        user_id: scope.user.id.into(),
        name: b.name.trim().into(),
        description: b.description,
        config: encode(&b.config)?,
        pipeline_id: pipeline,
        version: b.version + 1,
        cursor: old.as_ref().map(|r| r.cursor).unwrap_or(now),
        baselines: old
            .as_ref()
            .map(|r| r.baselines.clone())
            .unwrap_or_else(|| "{}".into()),
        created_at: old.as_ref().map(|r| r.created_at).unwrap_or(now),
        updated_at: now,
    };
    if old.is_some() {
        let n = defs::Entity::update_many()
            .col_expr(defs::Column::Name, Expr::value(model.name.clone()))
            .col_expr(
                defs::Column::Description,
                Expr::value(model.description.clone()),
            )
            .col_expr(defs::Column::Config, Expr::value(model.config.clone()))
            .col_expr(defs::Column::Version, Expr::value(model.version))
            .col_expr(defs::Column::UpdatedAt, Expr::value(model.updated_at))
            .filter(defs::Column::Id.eq(review_id))
            .filter(defs::Column::Version.eq(b.version))
            .exec(&tx)
            .await?
            .rows_affected;
        if n != 1 {
            return Err(conflict());
        }
    } else {
        defs::Entity::insert(model.clone().into_active_model())
            .exec(&tx)
            .await?;
    }
    crate::hosts::activity_in(
        s,
        &tx,
        scope,
        if old.is_some() { "update" } else { "create" },
        "code-reviews",
        &review_id,
        json!({"name":model.name}),
    )
    .await?;
    tx.commit().await?;
    definition_json(&model)
}
async fn remove(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("code-reviews.delete")?;
    let row = owned(&s, &scope, id).await?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    // Retain immutable run evidence. Prevent deletion while an existing pipeline is active.
    let active = jobs::Entity::find()
        .filter(jobs::Column::ReviewId.eq(row.id))
        .filter(jobs::Column::State.is_in([
            "queued",
            "launching",
            "waiting",
            "running",
            "cancelling",
        ]))
        .one(&tx)
        .await?;
    if active.is_some() {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new("reviews.error.activeJobs"),
        ));
    }
    defs::Entity::delete_by_id(row.id).exec(&tx).await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn authorize_config(s: &AppState, scope: &Scope, c: &Config) -> Result<()> {
    scope.require("repositories.read")?;
    scope.require("credentials.read")?;
    for r in &c.repositories {
        let connection = crate::repositories::auth::owned_connection(
            &s.db,
            scope,
            r.connection_id.parse().map_err(|_| bad("connection"))?,
        )
        .await?;
        if connection.state != "connected" {
            return Err(ApiError::forbidden());
        }
    }
    Ok(())
}

#[derive(Default, Deserialize)]
struct RunBody {
    repository: Option<String>,
    base_sha: Option<String>,
    head_sha: Option<String>,
    #[serde(default)]
    full_scan: bool,
}
#[derive(Clone, Serialize, Deserialize)]
struct Snapshot {
    config: Config,
    config_version: i64,
    connection_id: i64,
    repository: String,
    url: String,
    base_sha: String,
    head_sha: String,
    full_scan: bool,
    pipeline_id: i64,
    user_id: i64,
    #[serde(default)]
    definition: String,
}
fn valid_sha(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64)
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        && s.bytes().any(|b| b != b'0')
}
/// Invoked inside the pipeline engine's insertion transaction. Holding the SQL
/// fence until the run commits closes the stale-owner launch window.
pub(crate) async fn authorize_launch(
    conn: &impl sea_orm::ConnectionTrait,
    workspace: i64,
    user: i64,
    pipeline: i64,
    job: i64,
    owner: &str,
    head: &str,
    definition: &str,
) -> Result<()> {
    let lock = crate::entities::locks::Entity::find_by_id(format!("code-review-job:{job}"))
        .lock_exclusive()
        .one(conn)
        .await?
        .ok_or_else(ApiError::forbidden)?;
    if lock.owner != owner || lock.expires_at <= db::now() {
        return Err(ApiError::forbidden());
    }
    let row = jobs::Entity::find_by_id(job)
        .filter(jobs::Column::WorkspaceId.eq(workspace))
        .lock_exclusive()
        .one(conn)
        .await?
        .ok_or_else(ApiError::forbidden)?;
    let snap: Snapshot = decode(&row.snapshot)?;
    if row.lease_owner != owner
        || row.lease_until <= db::now()
        || row.state != "launching"
        || snap.user_id != user
        || snap.pipeline_id != pipeline
        || snap.head_sha != head
        || snap.definition.is_empty()
        || snap.definition != definition
    {
        return Err(ApiError::forbidden());
    }
    Ok(())
}
fn matches_selection(r: &crate::repositories::analysis::insights::Selection, path: &str) -> bool {
    r.all_repositories
        || r.repository == path
        || r.repository
            .strip_suffix('*')
            .is_some_and(|p| path.starts_with(p))
}
async fn run(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<RunBody>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("code-reviews.run")?;
    let row = owned(&s, &scope, id).await?;
    let c: Config = decode(&row.config)?;
    authorize_config(&s, &scope, &c).await?;
    let mut items = vec![];
    for (conn, path, url) in worker::sources(&s, &scope, &c).await? {
        if body.repository.as_ref().is_some_and(|p| p != &path) {
            continue;
        }
        let (base, head) = worker::resolve_shas(
            &s,
            &scope,
            &row,
            &c,
            conn,
            &path,
            body.base_sha.as_deref(),
            body.head_sha.as_deref(),
            body.full_scan,
        )
        .await?;
        let snapshot = Snapshot {
            config: c.clone(),
            config_version: row.version,
            connection_id: conn,
            repository: path,
            url,
            base_sha: base,
            head_sha: head,
            full_scan: body.full_scan,
            pipeline_id: row.pipeline_id,
            user_id: row.user_id,
            definition: String::new(),
        };
        let mut job = worker::enqueue(&s, &row, snapshot).await?;
        if job.state == "failed" || job.state == "cancelled" {
            // Manual resubmission explicitly retries the same durable logical job.
            let mut result: Value = decode(&job.result)?;
            let mut attempts = result["attempts"].as_array().cloned().unwrap_or_default();
            attempts.push(json!({"pipeline_run_id":Id::from(job.pipeline_run_id),"finished_at":job.finished_at,"state":job.state,"error":result["error"],"test_error":result["test_error"],"ai_error":result["ai_error"]}));
            result = json!({"attempts":attempts,"test_state":if c.tests_enabled{"pending"}else{"disabled"},"ai_state":if c.ai_enabled{"pending"}else{"disabled"},"coverage":{"scanned":[],"skipped":[]},"error":""});
            let n = jobs::Entity::update_many()
                .col_expr(jobs::Column::State, Expr::value("queued"))
                .col_expr(jobs::Column::PipelineRunId, Expr::value(0i64))
                .col_expr(jobs::Column::Result, Expr::value(encode(&result)?))
                .col_expr(jobs::Column::FinishedAt, Expr::value(0i64))
                .filter(jobs::Column::Id.eq(job.id))
                .filter(jobs::Column::State.eq(job.state.clone()))
                .exec(&s.db)
                .await?
                .rows_affected;
            if n == 1 {
                job.state = "queued".into();
                job.pipeline_run_id = 0;
                job.finished_at = 0;
                job.result = encode(&result)?;
            }
        }
        items.push(job_json(&job)?);
    }
    if items.is_empty() {
        return Err(bad("selectionEmpty"));
    }
    Ok((StatusCode::ACCEPTED, Json(json!({"items":items}))))
}
fn job_json(r: &jobs::Model) -> Result<Value> {
    let snap: Snapshot = decode(&r.snapshot)?;
    let mut v: Value = decode(&r.result)?;
    display_result(&mut v);
    for (k, x) in [
        ("id", json!(Id::from(r.id))),
        ("review_id", json!(Id::from(r.review_id))),
        ("repository", json!(snap.repository)),
        ("connection_id", json!(Id::from(snap.connection_id))),
        ("base_sha", json!(snap.base_sha)),
        ("head_sha", json!(snap.head_sha)),
        ("full_scan", json!(snap.full_scan)),
        ("state", json!(r.state)),
        ("pipeline_run_id", json!(Id::from(r.pipeline_run_id))),
        ("pipeline_id", json!(Id::from(snap.pipeline_id))),
        ("created_at", json!(r.created_at)),
        ("finished_at", json!(r.finished_at)),
        ("config_version", json!(snap.config_version)),
    ] {
        v[k] = x;
    }
    Ok(v)
}
fn display_result(v: &mut Value) {
    for key in ["error", "test_error", "ai_error", "waiting_reason"] {
        if let Some(text) = v[key].as_str() {
            v[key] = json!(crate::i18n::display_stored(text));
        }
    }
    if let Some(attempts) = v["attempts"].as_array_mut() {
        for attempt in attempts {
            display_result(attempt);
        }
    }
}

/// A report reads review evidence only for its authorized commit cohort. This
/// deliberately omits workflow/configuration snapshots and credential references.
pub(crate) async fn report_summary(
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    repositories: &[(i64, String)],
    commits: &BTreeSet<String>,
) -> Result<Value> {
    if scope.require("code-reviews.read").is_err()
        || scope.require("repositories.read").is_err()
        || scope.require("credentials.read").is_err()
    {
        return Ok(json!({"available":false,"reason":"permission"}));
    }
    let mut allowed = BTreeSet::new();
    let mut unavailable = Vec::new();
    for (connection, path) in repositories {
        match crate::repositories::auth::owned_connection(conn,scope,(*connection).into()).await {
            Ok(row) if row.state=="connected"=>{
                let credential_available=if row.credential_id==0{true}else{crate::entities::credentials::Entity::find_by_id(row.credential_id).filter(crate::entities::credentials::Column::WorkspaceId.eq(scope.workspace_id)).one(conn).await?.is_some()};
                if credential_available {allowed.insert((*connection,path.clone()));}else{unavailable.push(json!({"connection_id":Id::from(*connection),"repository":path,"reason":"credential"}));}
            },_=>unavailable.push(json!({"connection_id":Id::from(*connection),"repository":path,"reason":"authorization"})),
        }
    }
    let definitions = defs::Entity::find()
        .filter(defs::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(defs::Column::UserId.eq(scope.user.id))
        .all(conn)
        .await?;
    let ids = definitions.into_iter().map(|r| r.id).collect::<Vec<_>>();
    let mut runs = vec![];
    let (mut passed, mut failed, mut findings_count) = (0u64, 0u64, 0u64);
    if !ids.is_empty() && !commits.is_empty() {
        let rows = jobs::Entity::find()
            .filter(jobs::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(jobs::Column::ReviewId.is_in(ids))
            .order_by_desc(jobs::Column::CreatedAt)
            .all(conn)
            .await?;
        for row in rows {
            let snapshot: Snapshot = decode(&row.snapshot)?;
            if snapshot.user_id != i64::from(scope.user.id)
                || !allowed.contains(&(snapshot.connection_id, snapshot.repository.clone()))
                || !commits.contains(&snapshot.head_sha)
            {
                continue;
            }
            let result: Value = decode(&row.result)?;
            if result["test_state"] == "passed" {
                passed += 1;
            }
            if result["test_state"] == "failed" {
                failed += 1;
            }
            let count = result["finding_count"].as_u64().unwrap_or(0);
            findings_count += count;
            runs.push(json!({"id":Id::from(row.id),"review_id":Id::from(row.review_id),"pipeline_run_id":Id::from(row.pipeline_run_id),"connection_id":Id::from(snapshot.connection_id),"head_sha":snapshot.head_sha,"repository":snapshot.repository,"state":row.state,"test_state":result["test_state"],"ai_state":result["ai_state"],"finding_count":count,"created_at":row.created_at,"finished_at":row.finished_at}));
        }
    }
    Ok(
        json!({"available":true,"runs":runs,"total":runs.len(),"tests_passed":passed,"tests_failed":failed,"findings":findings_count,"unavailable_repositories":unavailable}),
    )
}
async fn readable(s: &AppState, scope: &Scope, id: Id) -> Result<defs::Model> {
    scope.require("code-reviews.read")?;
    let row = owned(s, scope, id).await?;
    authorize_config(s, scope, &decode(&row.config)?).await?;
    Ok(row)
}
fn page(raw: &HashMap<String, String>) -> (usize, usize) {
    (
        raw.get("page")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1usize)
            .clamp(1, 1000000),
        raw.get("page_size")
            .and_then(|s| s.parse().ok())
            .unwrap_or(20usize)
            .clamp(1, 200),
    )
}
async fn run_list(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let row = readable(&s, &scope, id).await?;
    let rows = jobs::Entity::find()
        .filter(jobs::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(jobs::Column::ReviewId.eq(row.id))
        .order_by_desc(jobs::Column::CreatedAt)
        .all(&s.db)
        .await?;
    let (p, size) = page(&raw);
    let mut items = vec![];
    for r in rows.iter().skip((p - 1) * size).take(size) {
        worker::authorize_snapshot(&s, &scope, &decode(&r.snapshot)?).await?;
        items.push(job_json(r)?);
    }
    Ok(Json(
        json!({"items":items,"total":rows.len(),"page":p,"page_size":size}),
    ))
}
async fn run_detail(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, run_id)): Path<(String, Id, Id)>,
) -> Result<Json<Value>> {
    let row = readable(&s, &scope, id).await?;
    let r = jobs::Entity::find_by_id(i64::from(run_id))
        .filter(jobs::Column::ReviewId.eq(row.id))
        .filter(jobs::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let snap: Snapshot = decode(&r.snapshot)?;
    worker::authorize_snapshot(&s, &scope, &snap).await?;
    let mut v = job_json(&r)?;
    v["config_snapshot"] = serde_json::to_value(snap.config).map_err(|_| bad("storedContract"))?;
    Ok(Json(v))
}
async fn run_cancel(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, run_id)): Path<(String, Id, Id)>,
) -> Result<Json<Value>> {
    scope.require("code-reviews.run")?;
    scope.require("pipelines.run")?;
    let definition = owned(&s, &scope, id).await?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let name = format!("code-review-job:{run_id}");
    // Same workspace -> SQL job lock -> job ordering as launch/publication.
    crate::entities::locks::Entity::find_by_id(name.clone())
        .lock_exclusive()
        .one(&tx)
        .await?;
    let mut row = jobs::Entity::find_by_id(i64::from(run_id))
        .filter(jobs::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(jobs::Column::ReviewId.eq(definition.id))
        .lock_exclusive()
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let snap: Snapshot = decode(&row.snapshot)?;
    if snap.user_id != i64::from(scope.user.id) || snap.pipeline_id != definition.pipeline_id {
        return Err(ApiError::forbidden());
    }
    if !["queued", "launching", "waiting", "running", "cancelling"].contains(&row.state.as_str()) {
        tx.commit().await?;
        return Ok(Json(job_json(&row)?));
    }
    // Recover the engine commit -> job link window using immutable source binding.
    let candidates = crate::entities::pipeline_runs::Entity::find()
        .filter(crate::entities::pipeline_runs::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(crate::entities::pipeline_runs::Column::PipelineId.eq(snap.pipeline_id))
        .filter(crate::entities::pipeline_runs::Column::TriggeredBy.eq(snap.user_id))
        .filter(crate::entities::pipeline_runs::Column::CreatedAt.gte(row.created_at))
        .all(&tx)
        .await?;
    let mut queued = Vec::new();
    let mut linked = false;
    for candidate in candidates {
        let inputs: Value = candidate
            .inputs
            .as_deref()
            .and_then(|v| serde_json::from_str(v).ok())
            .unwrap_or(json!({}));
        if candidate.commit != snap.head_sha || inputs["review_job_id"] != row.id.to_string() {
            continue;
        }
        if row.pipeline_run_id != 0 && row.pipeline_run_id != candidate.id {
            continue;
        }
        linked = true;
        row.pipeline_run_id = candidate.id;
        if crate::pipelines::runs::mark_cancelling(&tx, &candidate).await? {
            queued.push(candidate.id);
        }
    }
    if row.pipeline_run_id != 0 && !linked {
        return Err(bad("runContract"));
    }
    row.state = if linked { "cancelling" } else { "cancelled" }.into();
    row.lease_owner.clear();
    row.lease_until = 0;
    row.finished_at = if linked { 0 } else { db::now() };
    let mut result: Value = decode(&row.result)?;
    result["waiting_reason"] = json!("");
    if !linked {
        cancel_result(&mut result);
    }
    row.result = encode(&result)?;
    jobs::Entity::update(row.clone().into_active_model().reset_all())
        .exec(&tx)
        .await?;
    crate::entities::locks::Entity::delete_by_id(name)
        .exec(&tx)
        .await?;
    tx.commit().await?;
    for run in queued {
        crate::pipelines::runs::finish(&s, run, "cancelled").await?;
    }
    Ok(Json(job_json(&row)?))
}
fn cancel_result(result: &mut Value) {
    for key in ["test_state", "ai_state"] {
        if ["pending", "running"].contains(&result[key].as_str().unwrap_or_default()) {
            result[key] = json!("cancelled");
        }
    }
    result["error"] = json!(crate::i18n::store_message(&crate::i18n::Message::new(
        "reviews.cancelled"
    )));
}
fn finding_json(r: &findings::Model) -> Result<Value> {
    let mut v: Value = decode(&r.detail)?;
    for (k, x) in [
        ("id", json!(Id::from(r.id))),
        ("repository", json!(r.repository)),
        ("status", json!(r.status)),
        ("version", json!(r.version)),
        ("first_seen", json!(r.first_seen)),
        ("last_seen", json!(r.last_seen)),
        ("history", decode::<Value>(&r.history)?),
    ] {
        v[k] = x;
    }
    Ok(v)
}
async fn finding_list(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let row = readable(&s, &scope, id).await?;
    let mut query = findings::Entity::find()
        .filter(findings::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(findings::Column::ReviewId.eq(row.id));
    if let Some(status) = raw.get("status").filter(|x| !x.is_empty()) {
        query = query.filter(findings::Column::Status.eq(status));
    }
    let rows = query
        .order_by_desc(findings::Column::LastSeen)
        .all(&s.db)
        .await?;
    let (p, size) = page(&raw);
    let mut items = vec![];
    for r in rows.iter().skip((p - 1) * size).take(size) {
        crate::repositories::auth::resolve_connection(
            &s,
            &s.db,
            &scope,
            r.connection_id.into(),
            &r.repository_url,
        )
        .await?;
        items.push(finding_json(r)?);
    }
    Ok(Json(
        json!({"items":items,"total":rows.len(),"page":p,"page_size":size}),
    ))
}
#[derive(Deserialize)]
struct FindingUpdate {
    status: String,
    version: i64,
}
async fn finding_update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, finding_id)): Path<(String, Id, Id)>,
    Json(b): Json<FindingUpdate>,
) -> Result<Json<Value>> {
    scope.require("code-reviews.update")?;
    let row = readable(&s, &scope, id).await?;
    if !["open", "fixed", "ignored", "false_positive"].contains(&b.status.as_str()) {
        return Err(bad("findingStatus"));
    }
    let r = findings::Entity::find_by_id(i64::from(finding_id))
        .filter(findings::Column::ReviewId.eq(row.id))
        .filter(findings::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    crate::repositories::auth::resolve_connection(
        &s,
        &s.db,
        &scope,
        r.connection_id.into(),
        &r.repository_url,
    )
    .await?;
    if r.version != b.version {
        return Err(conflict());
    }
    let mut history: Vec<Value> = decode(&r.history)?;
    history.push(json!({"from":r.status,"to":b.status,"at":db::now(),"actor":scope.user.id}));
    let n = findings::Entity::update_many()
        .col_expr(findings::Column::Status, Expr::value(b.status))
        .col_expr(findings::Column::History, Expr::value(encode(&history)?))
        .col_expr(
            findings::Column::Version,
            Expr::col(findings::Column::Version).add(1),
        )
        .filter(findings::Column::Id.eq(r.id))
        .filter(findings::Column::Version.eq(b.version))
        .exec(&s.db)
        .await?
        .rows_affected;
    if n != 1 {
        return Err(conflict());
    }
    Ok(Json(json!({"ok":true})))
}

#[cfg(test)]
mod tests;
