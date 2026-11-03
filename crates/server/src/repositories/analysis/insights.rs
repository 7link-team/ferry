//! Standalone analysis scopes over shared, connection-authorized collection facts.
use super::*;
use crate::{
    entities::{code_insight_repositories as selections, code_insights as insights},
    resources::resource::{self, ListQuery, Spec},
};
use axum::http::StatusCode;
use sea_orm::QueryTrait;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InsightConfig {
    /// Server-maintained local projection status; never a collection error.
    pub identity_error: String,
    pub schedule_error: String,
    pub enabled: bool,
    pub update_mode: String,
    pub schedule: String,
    pub timezone: String,
    pub full_history: bool,
    pub initial_days: u32,
    pub reconcile_minutes: u32,
    pub aliases: Vec<Alias>,
}
impl Default for InsightConfig {
    fn default() -> Self {
        Self {
            identity_error: String::new(),
            schedule_error: String::new(),
            enabled: true,
            update_mode: "scheduled".into(),
            schedule: "0 */4 * * *".into(),
            timezone: String::new(),
            full_history: false,
            initial_days: 90,
            reconcile_minutes: 1440,
            aliases: vec![],
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Selection {
    pub connection_id: String,
    pub repository: String,
    pub all_repositories: bool,
    pub pipeline_id: String,
}
#[derive(Deserialize)]
pub struct Body {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: i64,
    #[serde(flatten)]
    pub config: InsightConfig,
    #[serde(default)]
    pub repositories: Vec<Selection>,
}
pub(super) fn decode(row: &insights::Model) -> Result<InsightConfig> {
    let mut value: Value = serde_json::from_str(&row.config).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.analysis.insightConfigRead"),
        )
    })?;
    // Persisted configurations created before scheduling keep their event behavior.
    value
        .as_object_mut()
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.analysis.insightConfigRead"),
            )
        })?
        .entry("update_mode")
        .or_insert_with(|| json!("event"));
    serde_json::from_value(value).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.analysis.insightConfigRead"),
        )
    })
}

fn next_run(config: &InsightConfig, after: i64) -> Result<i64> {
    match config.update_mode.as_str() {
        "event" => Ok(0),
        "scheduled" => {
            let timezone = if config.timezone.trim().is_empty() {
                "UTC"
            } else {
                config.timezone.trim()
            };
            let next = crate::cron_jobs::next_schedule(&config.schedule, timezone, after)?;
            Ok(if config.enabled { next } else { 0 })
        }
        _ => Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.analysis.updateModeInvalid"),
        )),
    }
}
pub(super) async fn owned(
    tx: &impl ConnectionTrait,
    scope: &Scope,
    id: Id,
) -> Result<insights::Model> {
    insights::Entity::find_by_id(i64::from(id))
        .filter(insights::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(insights::Column::UserId.eq(scope.user.id))
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)
}
pub(super) async fn members(
    tx: &impl ConnectionTrait,
    row: &insights::Model,
) -> Result<Vec<selections::Model>> {
    Ok(selections::Entity::find()
        .filter(selections::Column::WorkspaceId.eq(row.workspace_id))
        .filter(selections::Column::InsightId.eq(row.id))
        .order_by_asc(selections::Column::Id)
        .all(tx)
        .await?)
}
fn selection(row: &selections::Model) -> Selection {
    Selection {
        connection_id: Id::from(row.connection_id).to_string(),
        repository: row.repository.clone(),
        all_repositories: row.all_repositories != 0,
        pipeline_id: row.pipeline_id.clone(),
    }
}

/// Missing/revoked selections remain visible as coverage gaps, never as usable source IDs.
pub(super) async fn selected_sources(
    tx: &impl ConnectionTrait,
    scope: &Scope,
    insight: &insights::Model,
) -> Result<Vec<sources::Model>> {
    selected_sources_mode(tx, scope, insight, false).await
}

async fn selected_sources_mode(
    tx: &impl ConnectionTrait,
    scope: &Scope,
    insight: &insights::Model,
    retain_unavailable: bool,
) -> Result<Vec<sources::Model>> {
    scope.require("repositories.read")?;
    let members = members(tx, insight).await?;
    if members.is_empty() {
        return Ok(vec![]);
    }
    let connection_ids: BTreeSet<_> = members.iter().map(|m| m.connection_id).collect();
    let authorized: BTreeSet<_> = connections::Entity::find()
        .filter(connections::Column::Id.is_in(connection_ids))
        .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(connections::Column::UserId.eq(scope.user.id))
        .filter(connections::Column::State.eq("connected"))
        .all(tx)
        .await?
        .into_iter()
        .map(|c| c.id)
        .collect();
    let mut paths: BTreeMap<i64, (bool, BTreeSet<String>)> = BTreeMap::new();
    for member in &members {
        if authorized.contains(&member.connection_id) {
            let entry = paths.entry(member.connection_id).or_default();
            entry.0 |= member.all_repositories != 0;
            entry.1.insert(member.repository.clone());
        }
    }
    let mut by_connection: BTreeMap<i64, Vec<sources::Model>> = BTreeMap::new();
    if !paths.is_empty() {
        let mut selected = sea_orm::Condition::any();
        for (connection_id, (all, repositories)) in paths {
            let mut group =
                sea_orm::Condition::all().add(sources::Column::ConnectionId.eq(connection_id));
            if !all {
                group = group.add(sources::Column::RemotePath.is_in(repositories));
            }
            selected = selected.add(group);
        }
        for source in sources::Entity::find()
            .filter(sources::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(selected)
            .all(tx)
            .await?
        {
            by_connection
                .entry(source.connection_id)
                .or_default()
                .push(source);
        }
    }
    let mut result = BTreeMap::new();
    let mut gaps = Vec::new();
    for member in members {
        let mut found: Vec<_> = by_connection
            .get(&member.connection_id)
            .into_iter()
            .flatten()
            .filter(|source| {
                member.all_repositories != 0 || source.remote_path == member.repository
            })
            .cloned()
            .collect();
        if found.is_empty() {
            gaps.push(sources::Model {
                id: 0,
                workspace_id: insight.workspace_id,
                connection_id: member.connection_id,
                repository_id: 0,
                remote_path: member.repository.clone(),
                coverage_start: insight.coverage_start,
                config_version: 0,
                state: "unavailable".into(),
                last_error: crate::i18n::store_message(&crate::i18n::Message::new(
                    "repository.analysis.undiscovered",
                )),
                synced_at: 0,
            });
        }
        for source in &mut found {
            // An unavailable source must not expose its previously collected membership.
            if source.state == "unavailable" && !retain_unavailable {
                source.id = 0;
                source.repository_id = 0;
                gaps.push(source.clone());
                continue;
            }
            source.coverage_start =
                coverage_intersection(source.coverage_start, insight.coverage_start);
            result.insert(source.id, source.clone());
        }
    }
    Ok(result.into_values().chain(gaps).collect())
}
const SPEC: Spec = Spec {
    search: &["name", "description"],
    filters: &[],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "-created_at",
};
fn summary(row: &insights::Model, count: usize) -> Result<Value> {
    let config = decode(row)?;
    Ok(
        json!({"id":Id::from(row.id),"name":row.name,"description":row.description,"enabled":config.enabled,
        "repository_count":count,"coverage_start":row.coverage_start,"update_mode":config.update_mode,
        "schedule":config.schedule,"timezone":config.timezone,"next_run_at":row.next_run_at,
        "version":row.version,"created_at":row.created_at,"updated_at":row.updated_at}),
    )
}
pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(mut raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("code-insights.read")?;
    let connection = raw
        .remove("connection_id")
        .filter(|v| !v.is_empty())
        .map(|v| v.parse::<Id>().map_err(|_| ApiError::missing()))
        .transpose()?;
    let query = ListQuery::parse(&SPEC, &raw)?;
    let mut select = insights::Entity::find().filter(insights::Column::UserId.eq(scope.user.id));
    if let Some(connection) = connection {
        owned_connection(&s.db, &scope, connection).await?;
        let subquery = selections::Entity::find()
            .select_only()
            .column(selections::Column::InsightId)
            .filter(selections::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(selections::Column::ConnectionId.eq(connection))
            .into_query();
        select = select.filter(insights::Column::Id.in_subquery(subquery));
    }
    let (rows, total) = resource::list::<insights::Entity, insights::Model>(
        &s.db,
        &SPEC,
        select,
        &scope.workspace_id,
        &query,
    )
    .await?;
    let selections = selections::Entity::find()
        .filter(selections::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(selections::Column::InsightId.is_in(rows.iter().map(|r| r.id)))
        .all(&s.db)
        .await?;
    let items = rows
        .iter()
        .map(|row| {
            summary(
                row,
                selections.iter().filter(|m| m.insight_id == row.id).count(),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Json(
        json!({"items":items,"total":total,"page":query.page,"page_size":query.page_size}),
    ))
}
pub async fn get(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("code-insights.read")?;
    let row = owned(&s.db, &scope, id).await?;
    let mut config = decode(&row)?;
    insight_identity::hydrate_aliases(&s.db, &row, &mut config.aliases).await?;
    let members = members(&s.db, &row).await?;
    let connections: HashMap<i64, connections::Model> = connections::Entity::find()
        .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(connections::Column::UserId.eq(scope.user.id))
        .filter(connections::Column::Id.is_in(members.iter().map(|m| m.connection_id)))
        .all(&s.db)
        .await?
        .into_iter()
        .map(|c| (c.id, c))
        .collect();
    let sources = selected_sources(&s.db, &scope, &row).await?;
    let jobs = jobs::Entity::find()
        .filter(jobs::Column::Id.is_in(sources.iter().filter(|r| r.id > 0).map(|r| r.id)))
        .all(&s.db)
        .await?;
    let collection_configs: HashMap<_, _> = configs::Entity::find()
        .filter(configs::Column::Id.is_in(sources.iter().map(|source| source.connection_id)))
        .all(&s.db)
        .await?
        .into_iter()
        .map(|row| Ok((row.id, super::decode(&row)?)))
        .collect::<Result<_>>()?;
    let mut value = summary(&row, members.len())?;
    value.as_object_mut().unwrap().extend(
        serde_json::to_value(config)
            .map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.analysis.insightConfigEncode"),
                )
            })?
            .as_object()
            .unwrap()
            .clone(),
    );
    let config = decode(&row)?;
    value["identity_error"] = json!(crate::i18n::display_stored(&config.identity_error));
    value["schedule_error"] = json!(crate::i18n::display_stored(&config.schedule_error));
    value["last_error"] = json!(
        [
            crate::i18n::display_stored(&config.identity_error),
            crate::i18n::display_stored(&config.schedule_error)
        ]
        .into_iter()
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
    );
    value["repositories"] = json!(
        members
            .iter()
            .map(|m| {
                let mut v = json!(selection(m));
                v["connection_name"] = json!(
                    connections
                        .get(&m.connection_id)
                        .map(|c| c.name.as_str())
                        .unwrap_or("")
                );
                v["provider"] = json!(
                    connections
                        .get(&m.connection_id)
                        .map(|c| c.provider.as_str())
                        .unwrap_or("")
                );
                v
            })
            .collect::<Vec<_>>()
    );
    value["sources"] = json!(
        sources
            .iter()
            .map(|source| {
                let job = jobs.iter().find(|job| job.id == source.id);
                let pipeline_configured = collection_configs
                    .get(&source.connection_id)
                    .is_some_and(|config| {
                        !config
                            .pipeline_bindings
                            .get(&source.remote_path)
                            .unwrap_or(&config.pipeline_id)
                            .is_empty()
                    });
                source_status(source, job, pipeline_configured, db::now())
            })
            .collect::<Vec<_>>()
    );
    Ok(Json(value))
}

/// A failed collection and a newly queued revision can coexist. Expose both the
/// persisted result and the effective task state instead of treating revisions
/// as a progress counter or implying that a blocked task is actively collecting.
fn source_status(
    source: &sources::Model,
    job: Option<&jobs::Model>,
    pipeline_configured: bool,
    now: i64,
) -> Value {
    let error = job
        .filter(|job| !job.last_error.is_empty())
        .map_or(source.last_error.as_str(), |job| job.last_error.as_str());
    let pipeline_errors = [
        "repository.collector.filesMissing",
        "repository.collector.pipelineRequired",
    ];
    let pipeline_required = !pipeline_configured
        && match crate::i18n::decode_stored(error) {
            Some(message) => pipeline_errors.contains(&message.key.as_str()),
            // Older installations persisted the rendered diagnostic. Recognize
            // exact known messages, never infer setup requirements from substrings.
            None => pipeline_errors.iter().any(|key| {
                crate::i18n::Locale::ALL.iter().any(|locale| {
                    error == crate::i18n::render(*locale, &crate::i18n::Message::new(*key))
                })
            }),
        };
    json!({
        "id":Id::from(source.id), "connection_id":Id::from(source.connection_id),
        "repository":source.remote_path, "state":source.state,
        "last_error":crate::i18n::display_stored(&source.last_error),
        "coverage_start":source.coverage_start, "synced_at":source.synced_at,
        "pipeline_required":pipeline_required,
        "job":job.map(|job| {
            let effective_state = if job.state == "running" && job.lease_until > now {
                "running"
            } else if source.state == "unavailable" {
                "unavailable"
            } else if pipeline_required {
                "blocked"
            } else if job.generation > job.completed_generation {
                if job.retry_at > now { "retry_wait" } else { "queued" }
            } else {
                job.state.as_str()
            };
            let pipeline_run_id = serde_json::from_str::<Value>(&job.cursor).ok()
                .and_then(|cursor| cursor["run_id"].as_i64()).filter(|id| *id > 0)
                .map(Id::from);
            json!({"state":job.state,"effective_state":effective_state,
                "generation":job.generation,"completed_generation":job.completed_generation,
                "last_error":crate::i18n::display_stored(&job.last_error),
                "updated_at":job.updated_at,"retry_at":job.retry_at,
                "lease_until":job.lease_until,"pipeline_run_id":pipeline_run_id})
        })
    })
}
async fn validate(
    s: &AppState,
    tx: &impl ConnectionTrait,
    scope: &Scope,
    body: &mut Body,
) -> Result<()> {
    body.name = body.name.trim().to_owned();
    if body.name.is_empty()
        || body.name.chars().count() > 255
        || body.config.initial_days == 0
        || body.config.reconcile_minutes == 0
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.analysis.insightSettingsInvalid"),
        ));
    }
    if body.repositories.len() > 1000 || (body.config.enabled && body.repositories.is_empty()) {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.analysis.insightRepositoriesRequired"),
        ));
    }
    scope.require("repositories.read")?;
    scope.require("credentials.read")?;
    let mut seen = BTreeSet::new();
    for member in &mut body.repositories {
        member.repository = member.repository.trim().to_owned();
        let connection_id: Id = member.connection_id.parse().map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.analysis.connectionRequired"),
            )
        })?;
        let connection = owned_connection(tx, scope, connection_id).await?;
        if body.config.enabled && connection.state != "connected" {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.analysis.connectionUnavailable"),
            ));
        }
        if connection.provider == "git" {
            if member.all_repositories {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.analysis.gitUrlRequired"),
                ));
            }
            if body.config.enabled && member.pipeline_id.is_empty() {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.analysis.gitInsightPipeline"),
                ));
            }
            crate::repositories::auth::resolve_connection(
                s,
                tx,
                scope,
                connection_id,
                &member.repository,
            )
            .await?;
        } else if !member.all_repositories && !member.repository.contains('/') {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.analysis.platformPathRequired"),
            ));
        }
        if member.repository.len() > 400 || member.repository.chars().any(char::is_control) {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.analysis.repositoryUrlInvalid"),
            ));
        }
        if member.all_repositories {
            member.repository.clear();
        }
        if !seen.insert((
            member.connection_id.clone(),
            member.repository.clone(),
            member.all_repositories,
        )) {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.analysis.duplicateInsightRepository"),
            ));
        }
        if !member.pipeline_id.is_empty() {
            scope.require("pipelines.run")?;
            let id: Id = member.pipeline_id.parse().map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.analysis.pipelineRequired"),
                )
            })?;
            if crate::entities::pipelines::Entity::find_by_id(i64::from(id))
                .filter(crate::entities::pipelines::Column::WorkspaceId.eq(scope.workspace_id))
                .one(tx)
                .await?
                .is_none()
            {
                return Err(ApiError::missing());
            }
        }
    }
    Ok(())
}
pub async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<Body>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("code-insights.create")?;
    let id = s.next_id()?;
    let value = save(&s, &scope, id, body, true).await?;
    Ok((StatusCode::CREATED, Json(value)))
}
pub async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<Body>,
) -> Result<Json<Value>> {
    scope.require("code-insights.update")?;
    Ok(Json(save(&s, &scope, id, body, false).await?))
}
async fn save(
    s: &AppState,
    scope: &Scope,
    id: Id,
    mut body: Body,
    creating: bool,
) -> Result<Value> {
    let tx = crate::variables::transaction(s, scope.workspace_id).await?;
    let old = if creating {
        None
    } else {
        Some(owned(&tx, scope, id).await?)
    };
    if old.as_ref().map(|r| r.version).unwrap_or(0) != body.version {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("repository.analysis.insightChanged"),
        ));
    }
    let previous = old.as_ref().map(decode).transpose()?.unwrap_or_default();
    let planned_next = next_run(&body.config, db::now())?;
    let schedule_changed = creating
        || previous.enabled != body.config.enabled
        || previous.update_mode != body.config.update_mode
        || previous.schedule != body.config.schedule
        || previous.timezone != body.config.timezone;
    body.config.schedule_error = if schedule_changed {
        String::new()
    } else {
        previous.schedule_error.clone()
    };
    let old_members = if let Some(row) = &old {
        members(&tx, row).await?
    } else {
        vec![]
    };
    let before = old_members.iter().map(selection).collect::<Vec<_>>();
    let same_members = before.len() == body.repositories.len()
        && before.iter().all(|r| body.repositories.contains(r));
    let collection_changed = creating
        || !same_members
        || previous.enabled != body.config.enabled
        || previous.full_history != body.config.full_history
        || previous.initial_days != body.config.initial_days
        || previous.reconcile_minutes != body.config.reconcile_minutes;
    if collection_changed {
        validate(s, &tx, scope, &mut body).await?;
    } else if body.name.trim().is_empty() || body.name.chars().count() > 255 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.analysis.insightNameInvalid"),
        ));
    }
    let requested = if body.config.full_history {
        0
    } else {
        db::now() - i64::from(body.config.initial_days) * 86_400_000
    };
    let start = old
        .as_ref()
        .map(|r| {
            if previous.full_history == body.config.full_history
                && previous.initial_days == body.config.initial_days
            {
                r.coverage_start
            } else {
                coverage_union(r.coverage_start, requested)
            }
        })
        .unwrap_or(requested);
    let mut row = insights::Model {
        id: id.into(),
        workspace_id: scope.workspace_id.into(),
        user_id: scope.user.id.into(),
        name: body.name.trim().into(),
        description: body.description,
        config: String::new(),
        coverage_start: start,
        next_run_at: if schedule_changed {
            planned_next
        } else {
            old.as_ref().map(|r| r.next_run_at).unwrap_or(planned_next)
        },
        version: body.version + 1,
        created_at: old.as_ref().map(|r| r.created_at).unwrap_or_else(db::now),
        updated_at: db::now(),
    };
    // Clients cannot hide a runtime mapping conflict by posting a status field.
    body.config.identity_error.clear();
    insight_identity::prepare_aliases(s, &tx, &row, &previous.aliases, &mut body.config.aliases)
        .await?;
    row.config = encode(&body.config)?;
    if creating {
        insights::Entity::insert(row.clone().into_active_model())
            .exec(&tx)
            .await?;
    } else {
        insights::Entity::update(row.clone().into_active_model().reset_all())
            .exec(&tx)
            .await?;
    }
    let mut affected = old_members
        .iter()
        .map(|m| m.connection_id)
        .collect::<BTreeSet<_>>();
    if !same_members {
        selections::Entity::delete_many()
            .filter(selections::Column::InsightId.eq(id))
            .exec(&tx)
            .await?;
        for member in body.repositories {
            let connection: Id = member
                .connection_id
                .parse()
                .map_err(|_| ApiError::missing())?;
            affected.insert(connection.into());
            selections::Entity::insert(
                selections::Model {
                    id: s.next_id()?.into(),
                    workspace_id: row.workspace_id,
                    insight_id: row.id,
                    connection_id: connection.into(),
                    repository: member.repository,
                    all_repositories: i64::from(member.all_repositories),
                    pipeline_id: member.pipeline_id,
                }
                .into_active_model(),
            )
            .exec(&tx)
            .await?;
        }
    }
    if collection_changed || previous.update_mode != body.config.update_mode {
        for connection in affected {
            aggregate(s, &tx, scope, connection).await?;
        }
    }
    insight_identity::rebuild(s, &tx, &row).await?;
    crate::hosts::activity_in(
        s,
        &tx,
        scope,
        if creating { "create" } else { "update" },
        "code-insights",
        &id,
        json!({"name":row.name}),
    )
    .await?;
    tx.commit().await?;
    Ok(json!({"id":id,"version":row.version}))
}

/// A single connection/source job services the union of enabled insight requirements.
async fn aggregate(
    s: &AppState,
    tx: &impl ConnectionTrait,
    scope: &Scope,
    connection: i64,
) -> Result<()> {
    let old = configs::Entity::find_by_id(connection).one(tx).await?;
    // Keep legacy configured collection as an independent demand until explicitly migrated.
    if old
        .as_ref()
        .map(super::decode)
        .transpose()?
        .is_some_and(|c| !c.insights_managed)
    {
        import_connection(s, tx, scope, connection).await?;
    }
    let old = configs::Entity::find_by_id(connection).one(tx).await?;
    let previous = old
        .as_ref()
        .map(super::decode)
        .transpose()?
        .unwrap_or_default();
    let rows = selections::Entity::find()
        .filter(selections::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(selections::Column::ConnectionId.eq(connection))
        .all(tx)
        .await?;
    let parents: HashMap<i64, insights::Model> = insights::Entity::find()
        .filter(insights::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(insights::Column::UserId.eq(scope.user.id))
        .filter(insights::Column::Id.is_in(rows.iter().map(|r| r.insight_id)))
        .all(tx)
        .await?
        .into_iter()
        .map(|r| (r.id, r))
        .collect();
    let mut config = Config {
        insights_managed: true,
        event_repositories: Some(vec![]),
        enabled: false,
        all_repositories: false,
        aliases: previous.aliases.clone(),
        initial_days: previous.initial_days,
        ..Default::default()
    };
    config.reconcile_minutes = u32::MAX;
    let mut starts: BTreeMap<String, i64> = BTreeMap::new();
    let mut pipelines: BTreeMap<String, String> = BTreeMap::new();
    let mut start = old.as_ref().map(|r| r.coverage_start).unwrap_or(db::now());
    for member in rows {
        let Some(parent) = parents.get(&member.insight_id) else {
            continue;
        };
        let settings = decode(parent)?;
        if !settings.enabled {
            continue;
        }
        config.enabled = true;
        config.reconcile_minutes =
            std::cmp::min(config.reconcile_minutes, settings.reconcile_minutes);
        start = coverage_union(start, parent.coverage_start);
        let key = if member.all_repositories != 0 {
            String::new()
        } else {
            member.repository.clone()
        };
        config.all_repositories |= member.all_repositories != 0;
        if settings.update_mode == "event" {
            config
                .event_repositories
                .as_mut()
                .unwrap()
                .push(key.clone());
        }
        if member.all_repositories == 0 {
            config.repositories.push(key.clone());
        }
        starts
            .entry(key.clone())
            .and_modify(|v| *v = coverage_union(*v, parent.coverage_start))
            .or_insert(parent.coverage_start);
        if !member.pipeline_id.is_empty() {
            if pipelines
                .get(&key)
                .is_some_and(|p| p != &member.pipeline_id)
            {
                return Err(ApiError::localized(
                    axum::http::StatusCode::CONFLICT,
                    crate::i18n::Message::new("repository.analysis.sharedPipelineMismatch"),
                ));
            }
            pipelines.insert(key, member.pipeline_id);
        }
    }
    config.repositories.sort();
    config.repositories.dedup();
    if let Some(paths) = &mut config.event_repositories {
        paths.sort();
        paths.dedup();
    }
    if !config.enabled {
        config.reconcile_minutes = previous.reconcile_minutes;
    }
    config.pipeline_id = pipelines.remove("").unwrap_or_default();
    if !config.pipeline_id.is_empty() && pipelines.values().any(|p| p != &config.pipeline_id) {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("repository.analysis.sharedScopeMismatch"),
        ));
    }
    config.pipeline_bindings = pipelines;
    for (path, value) in &mut starts {
        if let Some(before) = previous.coverage_starts.get(path) {
            *value = coverage_union(*value, *before);
        }
    }
    config.coverage_starts = starts;
    config.full_history = start == 0;
    let changed = old.is_none() || !previous.same_collection(&config);
    if !changed {
        if previous.event_repositories != config.event_repositories {
            let mut updated = previous;
            updated.event_repositories = config.event_repositories;
            configs::Entity::update_many()
                .col_expr(configs::Column::Config, Expr::value(encode(&updated)?))
                .filter(configs::Column::Id.eq(connection))
                .exec(tx)
                .await?;
        }
        return Ok(());
    }
    let version = old.as_ref().map(|r| r.version + 1).unwrap_or(1);
    config.collection_version = version;
    config.identity_version = previous.identity_version;
    let row = configs::Model {
        id: connection,
        workspace_id: scope.workspace_id.into(),
        config: encode(&config)?,
        coverage_start: start,
        next_reconcile_at: 0,
        last_error: String::new(),
        version,
        created_at: old.as_ref().map(|r| r.created_at).unwrap_or_else(db::now),
        updated_at: db::now(),
    };
    if old.is_some() {
        configs::Entity::update(row.into_active_model().reset_all())
            .exec(tx)
            .await?;
    } else {
        configs::Entity::insert(row.into_active_model())
            .exec(tx)
            .await?;
    }
    for mut source in sources::Entity::find()
        .filter(sources::Column::ConnectionId.eq(connection))
        .all(tx)
        .await?
    {
        source.config_version = version;
        if config.selects(&source.remote_path) {
            let required = config.required_coverage_start(&source.remote_path, start);
            let widened = coverage_union(source.coverage_start, required);
            if widened != source.coverage_start {
                source.coverage_start = widened;
                if source.state != "unavailable" {
                    source.state = "pending".into();
                }
            }
            let id = source.id;
            sources::Entity::update(source.into_active_model().reset_all())
                .exec(tx)
                .await?;
            dirty(s, tx, id, scope.workspace_id.into()).await?;
        } else {
            sources::Entity::update(source.into_active_model().reset_all())
                .exec(tx)
                .await?;
        }
    }
    Ok(())
}

/// Import persisted connection configurations locally, without re-fetching any commits.
async fn import_connection(
    s: &AppState,
    tx: &impl ConnectionTrait,
    scope: &Scope,
    connection: i64,
) -> Result<()> {
    let Some(mut legacy) = configs::Entity::find_by_id(connection).one(tx).await? else {
        return Ok(());
    };
    let mut config = super::decode(&legacy)?;
    if config.insights_managed {
        return Ok(());
    }
    let conn = owned_connection(tx, scope, connection.into()).await?;
    let has_sources = sources::Entity::find()
        .filter(sources::Column::ConnectionId.eq(connection))
        .one(tx)
        .await?
        .is_some();
    if (config.enabled || has_sources)
        && insights::Entity::find_by_id(connection)
            .one(tx)
            .await?
            .is_none()
    {
        let mut insight = insights::Model {
            id: connection,
            workspace_id: legacy.workspace_id,
            user_id: conn.user_id,
            name: crate::i18n::render(
                crate::i18n::current_locale(),
                &crate::i18n::Message::new("repository.analysis.importedInsightName")
                    .with("name", &conn.name),
            ),
            description: String::new(),
            config: String::new(),
            coverage_start: legacy.coverage_start,
            next_run_at: 0,
            version: 1,
            created_at: legacy.created_at,
            updated_at: legacy.updated_at,
        };
        let mut settings = InsightConfig {
            identity_error: String::new(),
            schedule_error: String::new(),
            enabled: config.enabled,
            update_mode: "event".into(),
            schedule: "0 */4 * * *".into(),
            timezone: String::new(),
            full_history: config.full_history,
            initial_days: config.initial_days,
            reconcile_minutes: config.reconcile_minutes,
            aliases: vec![],
        };
        settings.aliases =
            insight_identity::import_aliases(s, tx, &insight, &config.aliases).await?;
        insight.config = encode(&settings)?;
        insights::Entity::insert(insight.clone().into_active_model())
            .exec(tx)
            .await?;
        let paths = if config.all_repositories {
            vec![String::new()]
        } else {
            config.repositories.clone()
        };
        for path in paths {
            selections::Entity::insert(
                selections::Model {
                    id: s.next_id()?.into(),
                    workspace_id: legacy.workspace_id,
                    insight_id: insight.id,
                    connection_id: connection,
                    pipeline_id: config
                        .pipeline_bindings
                        .get(&path)
                        .cloned()
                        .unwrap_or_else(|| config.pipeline_id.clone()),
                    repository: path,
                    all_repositories: i64::from(config.all_repositories),
                }
                .into_active_model(),
            )
            .exec(tx)
            .await?;
        }
    }
    config.insights_managed = true;
    legacy.config = encode(&config)?;
    configs::Entity::update(legacy.into_active_model().reset_all())
        .exec(tx)
        .await?;
    Ok(())
}
pub(crate) async fn recover(s: &AppState) -> Result<()> {
    for row in configs::Entity::find().all(&s.db).await? {
        if super::decode(&row)?.insights_managed {
            continue;
        }
        let Some(conn) = connections::Entity::find_by_id(row.id).one(&s.db).await? else {
            continue;
        };
        let Ok(scope) = Scope::for_actor(s, conn.workspace_id.into(), conn.user_id.into()).await
        else {
            continue;
        };
        let tx = crate::variables::transaction(s, scope.workspace_id).await?;
        import_connection(s, &tx, &scope, conn.id).await?;
        tx.commit().await?;
    }
    insight_identity::recover(s).await
}
pub async fn delete(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("code-insights.delete")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let row = owned(&tx, &scope, id).await?;
    let affected = members(&tx, &row)
        .await?
        .into_iter()
        .map(|m| m.connection_id)
        .collect::<BTreeSet<_>>();
    selections::Entity::delete_many()
        .filter(selections::Column::InsightId.eq(id))
        .exec(&tx)
        .await?;
    insights::Entity::delete_by_id(i64::from(id))
        .exec(&tx)
        .await?;
    crate::entities::code_insight_identity_links::Entity::delete_many()
        .filter(crate::entities::code_insight_identity_links::Column::InsightId.eq(id))
        .exec(&tx)
        .await?;
    crate::entities::code_insight_people::Entity::delete_many()
        .filter(crate::entities::code_insight_people::Column::InsightId.eq(id))
        .exec(&tx)
        .await?;
    for connection in affected {
        aggregate(&s, &tx, &scope, connection).await?;
    }
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "delete",
        "code-insights",
        &id,
        json!({"name":row.name}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id})))
}
pub async fn retry(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("code-insights.sync")?;
    scope.require("credentials.read")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let row = owned(&tx, &scope, id).await?;
    if !decode(&row)?.enabled {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.analysis.insightEnableFirst"),
        ));
    }
    for source in selected_sources(&tx, &scope, &row)
        .await?
        .into_iter()
        .filter(|r| r.id > 0)
    {
        if jobs::Entity::find_by_id(source.id)
            .one(&tx)
            .await?
            .is_some_and(|j| j.lease_until > db::now())
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("repository.analysis.sharedAnalysisRunning"),
            ));
        }
        jobs::Entity::update_many()
            .col_expr(jobs::Column::Cursor, Expr::value(""))
            .filter(jobs::Column::Id.eq(source.id))
            .exec(&tx)
            .await?;
        dirty(&s, &tx, source.id, source.workspace_id).await?;
    }
    for member in members(&tx, &row).await? {
        owned_connection(&tx, &scope, member.connection_id.into()).await?;
        configs::Entity::update_many()
            .col_expr(configs::Column::NextReconcileAt, Expr::value(0_i64))
            .filter(configs::Column::Id.eq(member.connection_id))
            .exec(&tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"id":id,"queued":true})))
}

/// Claim persisted schedules under the same workspace lock as configuration edits.
/// Missed occurrences collapse into one enqueue; active collectors retain their leases.
pub(super) async fn schedule_due(s: &AppState) -> Result<()> {
    let now = db::now();
    let due = insights::Entity::find()
        .filter(insights::Column::NextRunAt.gt(0))
        .filter(insights::Column::NextRunAt.lte(now))
        .order_by_asc(insights::Column::NextRunAt)
        .limit(100)
        .all(&s.db)
        .await?;
    let mut queued = BTreeSet::new();
    for candidate in due {
        let tx = crate::variables::transaction(s, candidate.workspace_id.into()).await?;
        let Some(row) = insights::Entity::find_by_id(candidate.id).one(&tx).await? else {
            continue;
        };
        if row.next_run_at <= 0 || row.next_run_at > now {
            continue;
        }
        let mut config = decode(&row)?;
        let next = next_run(&config, now)?;
        let outcome = if config.enabled && config.update_mode == "scheduled" {
            enqueue_scheduled(s, &tx, &row, &mut queued).await
        } else {
            Ok(())
        };
        match outcome {
            Ok(()) => config.schedule_error.clear(),
            Err(error) if error.0.is_client_error() => {
                let message = crate::i18n::Message::new("repository.analysis.scheduledNotRun");
                let message = match error.2 {
                    Some(cause) => message.with_message("error", cause),
                    None => message.with("error", error.1),
                };
                config.schedule_error = crate::i18n::store_message(&message);
            }
            Err(error) => return Err(error),
        }
        insights::Entity::update_many()
            .col_expr(insights::Column::NextRunAt, Expr::value(next))
            .col_expr(insights::Column::Config, Expr::value(encode(&config)?))
            .filter(insights::Column::Id.eq(row.id))
            .exec(&tx)
            .await?;
        tx.commit().await?;
    }
    Ok(())
}

async fn enqueue_scheduled(
    s: &AppState,
    tx: &impl ConnectionTrait,
    row: &insights::Model,
    queued: &mut BTreeSet<i64>,
) -> Result<()> {
    let scope = Scope::for_actor_in(tx, row.workspace_id.into(), row.user_id.into()).await?;
    scope.require("repositories.read")?;
    scope.require("credentials.read")?;
    // Queue unavailable sources too: discovery must restore authorization before
    // try_claim permits them to run. This avoids waiting another cron period.
    for source in selected_sources_mode(tx, &scope, row, true)
        .await?
        .into_iter()
        .filter(|v| v.id > 0)
    {
        if !queued.contains(&source.id) {
            dirty(s, tx, source.id, source.workspace_id).await?;
            queued.insert(source.id);
        }
    }
    // Refresh access/discover missing selected repositories before the worker claims jobs.
    let ids: BTreeSet<_> = members(tx, row)
        .await?
        .into_iter()
        .map(|m| m.connection_id)
        .collect();
    let authorized = connections::Entity::find()
        .filter(connections::Column::Id.is_in(ids))
        .filter(connections::Column::WorkspaceId.eq(row.workspace_id))
        .filter(connections::Column::UserId.eq(row.user_id))
        .filter(connections::Column::State.eq("connected"))
        .all(tx)
        .await?;
    configs::Entity::update_many()
        .col_expr(configs::Column::NextReconcileAt, Expr::value(0_i64))
        .filter(configs::Column::Id.is_in(authorized.into_iter().map(|c| c.id)))
        .exec(tx)
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "insights_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "insights_schedule_tests.rs"]
mod schedule_tests;
