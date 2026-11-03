//! Pipeline run history, immutable snapshots and Actions scheduling.
use crate::error::{Json, Path, Query};
use crate::i18n::Message;
use crate::{
    accounts::workspace::Scope,
    db,
    entities::{
        host_groups, hosts, pipeline_run_logs as logs, pipeline_run_steps as steps,
        pipeline_runs as runs, pipelines, repositories, workspaces,
    },
    error::{ApiError, Result},
    hosts::channel::HostChannel,
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{extract::State, http::StatusCode};
use bytes::Bytes;
use sea_orm::{
    ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, Set,
    TransactionTrait,
    sea_query::{Expr, ExprTrait},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    time::Duration,
};

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct VariableSnapshot {
    pub(crate) values: BTreeMap<String, String>,
    pub(crate) sources: Vec<Value>,
    /// Actions runs: the secrets and vars contexts every job receives,
    /// including everything authorized when the workflow indexes them
    /// dynamically. Absent in snapshots taken before this existed.
    #[serde(default)]
    pub(crate) secrets: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) vars: Option<BTreeMap<String, String>>,
    /// Actions jobs that declare `environment`, by job key.
    #[serde(default)]
    pub(crate) environments: BTreeMap<String, JobEnvironment>,
    /// Actions jobs with `concurrency`: job key -> cancel-in-progress.
    #[serde(default)]
    pub(crate) job_cancel_in_progress: BTreeMap<String, bool>,
}

/// A job's GitHub `environment`: its Ferry environment's contexts (None when
/// no Ferry environment has that name) and whether the job awaits approval.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct JobEnvironment {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) approval: bool,
    pub(crate) secrets: Option<BTreeMap<String, String>>,
    pub(crate) vars: Option<BTreeMap<String, String>>,
}
fn snapshot_aad(workspace: i64, run: i64) -> String {
    crate::vault::aad(
        &workspace.to_string(),
        "pipeline-run-variables",
        &run.to_string(),
    )
}
pub(crate) fn open_variables(s: &AppState, run: &runs::Model) -> Result<VariableSnapshot> {
    match &run.variable_snapshot {
        Some(sealed) => serde_json::from_str(
            &s.vault
                .open_secret(sealed, &snapshot_aad(run.workspace_id, run.id))?,
        )
        .map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.variableSnapshotInvalid"),
            )
        }),
        None => Ok(VariableSnapshot::default()),
    }
}

const SPEC: Spec = Spec {
    search: &[],
    filters: &["pipeline_id", "state", "environment_id"],
    sortable: &["number", "created_at", "state"],
    default_sort: "-number",
};
const RUN_NAMES: [crate::resources::names::Ref; 3] = [
    crate::resources::names::of(
        "pipeline_id",
        "pipeline_name",
        crate::resources::names::Kind::Pipeline,
    ),
    crate::resources::names::of(
        "triggered_by",
        "triggered_by_name",
        crate::resources::names::Kind::User,
    ),
    crate::resources::names::of("host_id", "host_name", crate::resources::names::Kind::Host),
];
fn run_json(r: &runs::Model) -> Value {
    json!({"id":Id::from(r.id),"pipeline_id":Id::from(r.pipeline_id),"host_id":Id::from(r.host_id),
        "environment_id":Id::from(r.environment_id),"environment_name":r.environment_name,
        "number":r.number,"state":r.state,"trigger_kind":r.trigger_kind,
        "definition_format":if crate::pipelines::actions::snapshot(&r.definition).is_some(){"github-actions"}else{"legacy"},
        "triggered_by":Id::from(r.triggered_by),"branch":r.branch,"commit":r.commit,
        "pipeline_name":r.pipeline_name,"host_name":r.host_name,"triggered_by_name":r.triggered_by_name,
        "inputs":r.inputs.as_deref().and_then(|text| serde_json::from_str::<Value>(text).ok()).unwrap_or_else(|| json!({})),
        "created_at":r.created_at,"started_at":r.started_at,"finished_at":r.finished_at})
}
async fn scoped(s: &AppState, scope: &Scope, id: &str) -> Result<runs::Model> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    runs::Entity::find_by_id(i64::from(id))
        .filter(runs::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)
}
pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("pipelines.read")?;
    let q = ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) =
        resource::list(&s.db, &SPEC, runs::Entity::find(), &scope.workspace_id, &q).await?;
    let mut items: Vec<Value> = rows.iter().map(run_json).collect();
    crate::resources::names::attach(&s.db, scope.workspace_id, &mut items, &RUN_NAMES).await?;
    Ok(Json(
        json!({"items":items,"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
pub async fn get(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    scope.require("pipelines.read")?;
    let r = scoped(&s, &scope, &id).await?;
    let rows = steps::Entity::find()
        .filter(steps::Column::RunId.eq(r.id))
        .order_by_asc(steps::Column::StepIndex)
        .all(&s.db)
        .await?;
    let mut v = run_json(&r);
    crate::resources::names::attach(
        &s.db,
        scope.workspace_id,
        std::slice::from_mut(&mut v),
        &RUN_NAMES,
    )
    .await?;
    if r.variable_snapshot.is_some() {
        let opened = open_variables(&s, &r)?;
        v["variables"] = json!(opened.sources);
        // Names and urls only; the environment's values stay sealed.
        v["environments"] = json!(
            opened
                .environments
                .iter()
                .map(|(key, environment)| (key.clone(), json!({"name": environment.name, "url": environment.url, "approval": environment.approval})))
                .collect::<serde_json::Map<_, _>>()
        );
    }
    let snapshot = crate::pipelines::actions::snapshot(&r.definition);
    // Only the authored document: resolved credentials and sealed variable values stay private.
    v["definition_snapshot"] = json!(snapshot.as_ref().map(|snapshot| &snapshot.document));
    v["steps"] = json!(
        rows.iter()
            .map(
                |v| json!({"index":v.step_index,"name":display_step_name(snapshot.as_ref(), v.step_index, &v.name),"state":v.state,
        "exit_code":v.exit_code,"started_at":v.started_at,"finished_at":v.finished_at})
            )
            .collect::<Vec<_>>()
    );
    let jobs = crate::entities::pipeline_run_jobs::Entity::find()
        .filter(crate::entities::pipeline_run_jobs::Column::RunId.eq(r.id))
        .all(&s.db)
        .await?;
    v["jobs"] = json!(jobs.iter().map(|job| {
        let definition = snapshot.as_ref().and_then(|s| s.jobs.iter().find(|item| item.key == job.job_key));
        json!({"key":job.job_key,"name":definition.map(|item| item.name.as_str()).unwrap_or(&job.job_key),
            "step_indices":definition.map(|item| item.steps.iter().map(|step| step.index).collect::<Vec<_>>()).unwrap_or_default(),
            "host_id":Id::from(job.host_id),"state":job.state,"started_at":job.started_at,"finished_at":job.finished_at})
    }).collect::<Vec<_>>());
    v["artifacts"] = crate::pipelines::artifacts::for_run(s.as_ref(), r.workspace_id, r.id).await?;
    Ok(Json(v))
}
fn display_step_name(
    snapshot: Option<&crate::pipelines::actions::Snapshot>,
    index: i64,
    name: &str,
) -> String {
    if name == "Run command"
        && snapshot.is_some_and(|snapshot| {
            snapshot
                .jobs
                .iter()
                .flat_map(|job| &job.steps)
                .any(|step| step.index == index && step.default_name)
        })
    {
        crate::i18n::message("actions.runCommand")
    } else {
        name.to_owned()
    }
}
pub async fn log_list(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("pipelines.read")?;
    let r = scoped(&s, &scope, &id).await?;
    let after = q
        .get("after")
        .map(|v| v.parse::<i64>())
        .transpose()
        .map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.afterInvalid"),
            )
        })?
        .unwrap_or(0);
    if after < 0 {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.afterInvalid"),
        ));
    }
    let rows = logs::Entity::find()
        .filter(logs::Column::RunId.eq(r.id))
        .filter(logs::Column::Seq.gt(after))
        .order_by_asc(logs::Column::Seq)
        .limit(200)
        .all(&s.db)
        .await?;
    Ok(Json(
        json!({"items":rows.iter().map(|v|json!({"seq":v.seq,"step_index":v.step_index,
        "stream":v.stream,"chunk":crate::i18n::display_text_stored(&v.chunk)})).collect::<Vec<_>>(),
        "next_seq":rows.last().map(|v|v.seq).unwrap_or(after)}),
    ))
}

#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerBody {
    branch: Option<String>,
    commit: Option<String>,
    environment_id: Option<String>,
    /// workflow_dispatch inputs (strings, booleans or numbers).
    #[serde(default)]
    inputs: BTreeMap<String, Value>,
}

/// GitHub semantics for workflow_dispatch inputs: undeclared names are
/// refused, missing ones take their default, required ones must be given,
/// and each value must match its declared type.
fn dispatch_inputs(
    specs: &[crate::pipelines::actions::InputSpec],
    given: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>> {
    if let Some(name) = given
        .keys()
        .find(|name| !specs.iter().any(|spec| &spec.name == *name))
    {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.inputUndeclared").with("name", name.clone()),
        ));
    }
    let text = |value: &Value| match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let mut values = BTreeMap::new();
    for spec in specs {
        let Some(value) = given.get(&spec.name).or(spec.default.as_ref()) else {
            if spec.required {
                return Err(ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.inputRequired").with("name", spec.name.clone()),
                ));
            }
            continue;
        };
        let invalid = |kind: Message| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.inputType")
                    .with("name", spec.name.clone())
                    .with_message("kind", kind),
            )
        };
        let value = match spec.r#type.as_str() {
            "boolean" => match value {
                Value::Bool(_) => value.clone(),
                Value::String(flag) if flag == "true" || flag == "false" => {
                    Value::Bool(flag == "true")
                }
                _ => return Err(invalid(Message::new("execution.booleanKind"))),
            },
            "number" => match value {
                Value::Number(_) => value.clone(),
                Value::String(number) => number
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .map(Value::Number)
                    .ok_or_else(|| invalid(Message::new("execution.numberKind")))?,
                _ => return Err(invalid(Message::new("execution.numberKind"))),
            },
            "choice" => {
                let choice = text(value);
                if !spec.options.contains(&choice) {
                    return Err(invalid(
                        Message::new("execution.choiceKind")
                            .with("options", spec.options.join("、")),
                    ));
                }
                Value::String(choice)
            }
            _ => Value::String(text(value)),
        };
        if spec.required && value == Value::String(String::new()) {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.inputRequired").with("name", spec.name.clone()),
            ));
        }
        values.insert(spec.name.clone(), value);
    }
    Ok(values)
}

pub async fn trigger(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>)> {
    trigger_as(s, scope, id, body, "manual").await
}

/// `kind` is the run's trigger: "manual" (workflow_dispatch) or "schedule".
/// Only server code chooses it; the public endpoint is always manual.
pub(crate) async fn trigger_as(
    s: AppState,
    scope: Scope,
    id: String,
    body: Bytes,
    kind: &str,
) -> Result<(StatusCode, Json<Value>)> {
    trigger_with_analysis_source(s, scope, id, body, kind, None).await
}

/// Analysis instantiates one checkout of its authorized connection from a saved
/// workflow template. The saved pipeline and ordinary/manual runs are unchanged.
pub(crate) async fn trigger_with_analysis_source(
    s: AppState,
    scope: Scope,
    id: String,
    body: Bytes,
    kind: &str,
    source: Option<(i64, String)>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("pipelines.run")?;
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut p = pipelines::Entity::find_by_id(i64::from(id))
        .filter(pipelines::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    if let Some((connection, url)) = source {
        let mut document = crate::pipelines::actions::document(&p.definition).ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.analysisActionsRequired"),
            )
        })?;
        let bindings: Vec<_> = document
            .checkout_bindings
            .iter_mut()
            .filter(|v| v.connection_id == connection.to_string())
            .collect();
        if bindings.len() != 1 {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.analysisSourceRequired"),
            ));
        }
        let binding = bindings.into_iter().next().unwrap();
        binding.repository_url = url;
        binding.git_ref = "+refs/heads/*:refs/remotes/origin/*".into();
        // Keep the YAML step's declarative repository/ref consistent with its
        // connection binding (the validator rejects conflicting values).
        let mut yaml: serde_json::Value =
            serde_yaml_ng::from_str(&document.workflow).map_err(|_| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.analysisTemplateRead"),
                )
            })?;
        if let Some(steps) = yaml["jobs"][&binding.job_id]["steps"].as_array_mut() {
            if let Some(step) = steps
                .iter_mut()
                .find(|v| v["id"].as_str() == Some(&binding.step_id))
            {
                if let Some(with) = step.get_mut("with").and_then(Value::as_object_mut) {
                    with.remove("repository");
                    with.remove("ref");
                }
            }
        }
        document.workflow = serde_yaml_ng::to_string(&yaml).map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.analysisTemplateEncode"),
            )
        })?;
        p.definition = serde_json::to_string(&document).map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.analysisTemplateEncode"),
            )
        })?;
    }
    let body: TriggerBody = if body.is_empty() {
        TriggerBody::default()
    } else {
        serde_json::from_slice(&body).map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.triggerObject"),
            )
        })?
    };
    let environment = crate::pipelines::contexts::selected(
        &tx,
        scope.workspace_id,
        id,
        body.environment_id.as_deref(),
    )
    .await?;
    if let Some(environment) = &environment {
        p.target_kind = environment.target_kind.clone();
        p.target_ref = environment.target_ref;
    }
    let definition = crate::pipelines::validate_definition(&p.definition)?;
    // Scope references again at execution time, not only when a definition is saved.
    let candidates =
        hosts::Entity::find().filter(hosts::Column::WorkspaceId.eq(scope.workspace_id));
    let node_only =
        crate::pipelines::actions::document(&p.definition).is_some() && p.target_ref == 0;
    let host = if node_only {
        None
    } else {
        Some(
            match p.target_kind.as_str() {
                "host" => {
                    candidates
                        .filter(hosts::Column::Id.eq(p.target_ref))
                        .one(&tx)
                        .await?
                }
                "host-group" => {
                    host_groups::Entity::find_by_id(p.target_ref)
                        .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
                        .one(&tx)
                        .await?
                        .ok_or_else(|| {
                            ApiError::localized(
                                StatusCode::BAD_REQUEST,
                                Message::new("execution.hostGroupMissing"),
                            )
                        })?;
                    // One run has one concrete host. Resolve direct online members once,
                    // deterministically, and retain that host even if group membership changes.
                    candidates
                        .filter(hosts::Column::GroupId.eq(p.target_ref))
                        .filter(hosts::Column::State.eq("online"))
                        .order_by_asc(hosts::Column::Id)
                        .one(&tx)
                        .await?
                }
                _ => {
                    return Err(ApiError::localized(
                        StatusCode::BAD_REQUEST,
                        Message::new("execution.unknownTarget"),
                    ));
                }
            }
            .ok_or_else(|| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.noExecutionHost"),
                )
            })?,
        )
    };
    let repo = if p.repository_id != 0 {
        Some(
            repositories::Entity::find_by_id(p.repository_id)
                .filter(repositories::Column::WorkspaceId.eq(scope.workspace_id))
                .one(&tx)
                .await?
                .ok_or_else(|| {
                    ApiError::localized(
                        StatusCode::BAD_REQUEST,
                        Message::new("execution.repositoryMissing"),
                    )
                })?,
        )
    } else {
        None
    };
    let (repo_url, branch, credential_id) = repo
        .map(|r| (r.url, r.default_branch, r.credential_id))
        .unwrap_or_default();
    let branch = body.branch.unwrap_or(branch);
    let commit = body.commit.unwrap_or_default();
    for (name, value) in &body.inputs {
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || !(value.is_string() || value.is_boolean() || value.is_number())
        {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.inputsMap"),
            ));
        }
    }
    let input_values = match crate::pipelines::actions::document(&p.definition) {
        Some(document) => dispatch_inputs(
            &crate::pipelines::actions::dispatch_inputs(&document),
            &body.inputs,
        )?,
        None if body.inputs.is_empty() => BTreeMap::new(),
        None => {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.inputsNotDeclared"),
            ));
        }
    };
    let inputs = (!input_values.is_empty())
        .then(|| serde_json::to_string(&input_values))
        .transpose()
        .map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.saveInputs"),
            )
        })?;
    for value in [&branch, &commit] {
        if value.chars().count() > 255 || value.contains('\0') {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.branchCommitInvalid"),
            ));
        }
    }
    let (run_id, now) = (i64::from(s.next_id()?), db::now());
    // UPDATE first acquires the workspace row lock, then read our own increment
    // inside that transaction. Concurrent triggers cannot receive the same number.
    workspaces::Entity::update_many()
        .col_expr(
            workspaces::Column::PipelineRunNumber,
            Expr::col(workspaces::Column::PipelineRunNumber).add(1),
        )
        .filter(workspaces::Column::Id.eq(scope.workspace_id))
        .exec(&tx)
        .await?;
    let number = workspaces::Entity::find_by_id(i64::from(scope.workspace_id))
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?
        .pipeline_run_number;
    let actions = match crate::pipelines::actions::document(&p.definition) {
        Some(document) => Some(
            crate::pipelines::actions::resolve(
                &s,
                &tx,
                &scope,
                document,
                host.as_ref().map(|row| row.id).unwrap_or(0),
            )
            .await?,
        ),
        None => None,
    };
    let mut snapshot = VariableSnapshot::default();
    let environment_id = Id::from(environment.as_ref().map(|e| e.id).unwrap_or(0));
    // Actions: GitHub semantics (dynamic indexing delivers every authorized
    // value; undefined names are "").
    let workflow_document = crate::pipelines::actions::document(&p.definition);
    let references = workflow_document
        .as_ref()
        .map(crate::pipelines::actions::references)
        .transpose()?;
    let actions_references = references.as_ref().ok_or_else(|| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.actionsDefinitionRequired"),
        )
    })?;
    let contexts = crate::variables::snapshot_for_actions(
        &s,
        &tx,
        scope.workspace_id,
        id,
        environment_id,
        actions_references,
    )
    .await?;
    // Server-side redaction masks every delivered secret.
    for (name, value) in &contexts.secrets {
        snapshot
            .values
            .insert(format!("secrets:{name}"), value.clone());
    }
    snapshot.sources = contexts.sources;
    snapshot.secrets = Some(contexts.secrets);
    snapshot.vars = Some(contexts.vars);
    // Contexts GitHub allows in workflow-level expressions (environment names,
    // concurrency groups): github, inputs and vars.
    let (server_url, repository) = crate::pipelines::actions_run::repository_parts(&repo_url);
    let expression_github = json!({
        "workflow": workflow_document
            .as_ref()
            .and_then(|document| serde_yaml_ng::from_str::<Value>(&document.workflow).ok())
            .and_then(|value| value.get("name").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| p.name.clone()),
        "ref": if branch.is_empty() { String::new() } else { format!("refs/heads/{branch}") },
        "ref_name": branch, "head_ref": "", "base_ref": "",
        "event_name": if kind == "schedule" { "schedule" } else { "workflow_dispatch" },
        "sha": commit, "repository": repository,
        "repository_owner": repository.split('/').next().unwrap_or(""),
        "actor": scope.user.name, "run_id": run_id.to_string(), "run_number": number,
        "run_attempt": 1, "server_url": server_url,
    });
    let no_vars = BTreeMap::new();
    let mut job_groups: BTreeMap<String, String> = BTreeMap::new();
    if let (Some(document), Some(references), Some(actions)) =
        (&workflow_document, &references, &actions)
    {
        let contexts = crate::pipelines::concurrency::Contexts {
            github: expression_github.clone(),
            inputs: &input_values,
            vars: snapshot.vars.as_ref().unwrap_or(&no_vars),
        };
        let environments = crate::entities::pipeline_contexts::Entity::find()
            .filter(crate::entities::pipeline_contexts::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(crate::entities::pipeline_contexts::Column::Kind.eq("environment"))
            .all(&tx)
            .await?;
        let mut declared = BTreeMap::new();
        for job in &actions.jobs {
            if let Some(raw) = crate::pipelines::actions::job_policies(document, &job.key)
                .and_then(|policy| policy.concurrency)
            {
                let (group, cancel) = match &raw {
                    Value::String(group) => (group.clone(), Value::Bool(false)),
                    Value::Object(object) => (
                        object
                            .get("group")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        object
                            .get("cancel-in-progress")
                            .cloned()
                            .unwrap_or(Value::Bool(false)),
                    ),
                    _ => {
                        return Err(ApiError::localized(
                            StatusCode::BAD_REQUEST,
                            Message::new("execution.concurrencyInvalid")
                                .with("name", job.key.clone()),
                        ));
                    }
                };
                let group = crate::pipelines::concurrency::template(&group, &contexts)?;
                if !group.is_empty() {
                    // Groups are per workspace, like GitHub's per repository.
                    let group = format!("{}:{group}", scope.workspace_id);
                    let group = if group.chars().count() > 255 {
                        format!("sha256:{}", crate::hosts::runner::fingerprint(&group))
                    } else {
                        group
                    };
                    job_groups.insert(job.key.clone(), group);
                    snapshot.job_cancel_in_progress.insert(
                        job.key.clone(),
                        crate::pipelines::concurrency::flag(&cancel, &contexts)?,
                    );
                }
            }
            let Some(raw) = crate::pipelines::actions::job_policies(document, &job.key)
                .and_then(|policy| policy.environment)
            else {
                continue;
            };
            let (name, url) = match &raw {
                Value::String(name) => (name.clone(), String::new()),
                Value::Object(object) => (
                    object
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    object
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                ),
                _ => {
                    return Err(ApiError::localized(
                        StatusCode::BAD_REQUEST,
                        Message::new("execution.environmentInvalid").with("name", job.key.clone()),
                    ));
                }
            };
            let name = crate::pipelines::concurrency::template(&name, &contexts)?;
            if name.is_empty() {
                continue;
            }
            // The url may use job outputs (evaluated by GitHub later); keep it as written then.
            let url = crate::pipelines::concurrency::template(&url, &contexts).unwrap_or(url);
            // GitHub environment names are case-insensitive.
            let row = environments
                .iter()
                .find(|row| row.name.eq_ignore_ascii_case(&name));
            let mut entry = JobEnvironment {
                name,
                url,
                approval: row.is_some_and(|row| row.requires_approval != 0),
                ..JobEnvironment::default()
            };
            if let Some(row) = row {
                let scoped = crate::variables::snapshot_for_actions(
                    &s,
                    &tx,
                    scope.workspace_id,
                    id,
                    Id::from(row.id),
                    references,
                )
                .await?;
                for (secret, value) in &scoped.secrets {
                    snapshot
                        .values
                        .insert(format!("secrets:{}:{secret}", job.key), value.clone());
                }
                entry.secrets = Some(scoped.secrets);
                entry.vars = Some(scoped.vars);
            }
            declared.insert(job.key.clone(), entry);
        }
        snapshot.environments = declared;
    }
    let variable_snapshot = if snapshot.values.is_empty() && snapshot.secrets.is_none() {
        None
    } else {
        Some(s.vault.seal(
            &serde_json::to_string(&snapshot).map_err(|_| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.saveVariableSnapshot"),
                )
            })?,
            &crate::vault::aad(
                &scope.workspace_id.to_string(),
                "pipeline-run-variables",
                &run_id.to_string(),
            ),
        )?)
    };
    let host_id = actions
        .as_ref()
        .and_then(|snapshot| snapshot.jobs.first())
        .map(|job| job.host_id)
        .or_else(|| host.as_ref().map(|row| row.id))
        .ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.hostMissing"),
            )
        })?;
    // Names as they are now, so history keeps them after a rename or delete.
    let host_name = crate::entities::hosts::Entity::find_by_id(host_id)
        .filter(crate::entities::hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&tx)
        .await?
        .map(|row| row.name)
        .unwrap_or_default();
    // GitHub concurrency, evaluated now with the contexts GitHub allows there.
    let policy = workflow_document
        .as_ref()
        .and_then(crate::pipelines::actions::concurrency);
    let (concurrency_group, cancel_in_progress) = match policy {
        Some(policy) => {
            let contexts = crate::pipelines::concurrency::Contexts {
                github: expression_github.clone(),
                inputs: &input_values,
                vars: snapshot.vars.as_ref().unwrap_or(&no_vars),
            };
            let group = crate::pipelines::concurrency::template(&policy.group, &contexts)?;
            let cancel =
                crate::pipelines::concurrency::flag(&policy.cancel_in_progress, &contexts)?;
            // Long group names are stored by digest; equal names still match.
            let group = if group.chars().count() > 255 {
                format!("sha256:{}", crate::hosts::runner::fingerprint(&group))
            } else {
                group
            };
            (group, cancel)
        }
        None => (String::new(), false),
    };
    let run = runs::Model {
        id: run_id,
        workspace_id: i64::from(scope.workspace_id),
        pipeline_id: p.id,
        concurrency_group,
        pipeline_name: p.name.clone(),
        host_id,
        host_name,
        triggered_by_name: scope.user.name.clone(),
        environment_id: environment.as_ref().map(|e| e.id).unwrap_or(0),
        environment_name: environment
            .as_ref()
            .map(|e| e.name.clone())
            .unwrap_or_default(),
        number: number,
        state: "queued".into(),
        trigger_kind: kind.into(),
        triggered_by: i64::from(scope.user.id),
        inputs,
        definition: match &actions {
            Some(snapshot) => serde_json::to_string(snapshot).map_err(|_| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.saveRunSnapshot"),
                )
            })?,
            None => p.definition,
        },
        variable_snapshot,
        repo_url: repo_url,
        branch: branch,
        commit,
        credential_id: credential_id,
        created_at: now,
        started_at: 0,
        finished_at: 0,
    };
    runs::Entity::insert(run.clone().into_active_model())
        .exec(&tx)
        .await?;
    for (index, step) in definition.steps.iter().enumerate() {
        steps::Entity::insert(steps::ActiveModel {
            id: Set(i64::from(s.next_id()?)),
            run_id: Set(run_id),
            step_index: Set(index as i64),
            name: Set(crate::pipelines::actions_run::runtime_path(
                &step.name, &run,
            )),
            state: Set("queued".into()),
            exit_code: Set(None),
            started_at: Set(0),
            finished_at: Set(0),
        })
        .exec(&tx)
        .await?;
    }
    if let Some(snapshot) = &actions {
        use crate::entities::pipeline_run_jobs as jobs;
        for job in &snapshot.jobs {
            jobs::Entity::insert(jobs::ActiveModel {
                id: Set(i64::from(s.next_id()?)),
                run_id: Set(run_id),
                job_key: Set(job.key.clone()),
                host_id: Set(job.host_id),
                state: Set("queued".into()),
                outputs: Set(None),
                concurrency_group: Set(job_groups.get(&job.key).cloned().unwrap_or_default()),
                started_at: Set(0),
                finished_at: Set(0),
            })
            .exec(&tx)
            .await?;
        }
    }
    tx.commit().await?;
    apply_concurrency(&s, &run, cancel_in_progress).await?;
    crate::hosts::activity(
        &s,
        &scope,
        "run",
        "pipelines",
        &id,
        json!({"run_id":Id::from(run_id),"number":number}),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id":Id::from(run_id),"number":number})),
    ))
}
pub async fn cancel(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    scope.require("pipelines.run")?;
    let r = scoped(&s, &scope, &id).await?;
    request_cancel(&s, &r).await?;
    crate::hosts::activity(
        &s,
        &scope,
        "cancel",
        "pipeline-runs",
        &Id::from(r.id),
        json!({}),
    )
    .await?;
    Ok(Json(json!({"ok":true})))
}
/// A queued run ends as cancelled; a running Actions run moves to
/// cancelling so its scheduler stops workers and cleans up.
pub(crate) async fn request_cancel(s: &AppState, r: &runs::Model) -> Result<()> {
    if crate::pipelines::actions::snapshot(&r.definition).is_some() {
        let queued = runs::Entity::update_many()
            .col_expr(runs::Column::State, Expr::value("cancelling"))
            .filter(runs::Column::Id.eq(r.id))
            .filter(runs::Column::State.eq("queued"))
            .exec(&s.db)
            .await?
            .rows_affected;
        if queued > 0 {
            finish(s, r.id, "cancelled").await?;
        } else {
            runs::Entity::update_many()
                .col_expr(runs::Column::State, Expr::value("cancelling"))
                .filter(runs::Column::Id.eq(r.id))
                .filter(runs::Column::State.eq("running"))
                .exec(&s.db)
                .await?;
        }
    } else {
        finish(s, r.id, "cancelled").await?;
    }
    Ok(())
}

/// GitHub concurrency for a new run: older pending runs of its group are
/// replaced, and with cancel-in-progress running ones are cancelled too.
async fn apply_concurrency(
    s: &AppState,
    run: &runs::Model,
    cancel_in_progress: bool,
) -> Result<()> {
    if run.concurrency_group.is_empty() {
        return Ok(());
    }
    let mut states = vec!["queued"];
    if cancel_in_progress {
        states.push("running");
    }
    let others = runs::Entity::find()
        .filter(runs::Column::WorkspaceId.eq(run.workspace_id))
        .filter(runs::Column::ConcurrencyGroup.eq(run.concurrency_group.clone()))
        .filter(runs::Column::Id.ne(run.id))
        .filter(runs::Column::State.is_in(states))
        .all(&s.db)
        .await?;
    for other in others {
        request_cancel(s, &other).await?;
    }
    Ok(())
}

pub async fn approve(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, key)): Path<(String, String, String)>,
) -> Result<Json<Value>> {
    scope.require("pipelines.run")?;
    let run = scoped(&s, &scope, &id).await?;
    let snapshot =
        crate::pipelines::actions::snapshot(&run.definition).ok_or_else(ApiError::missing)?;
    if run.state != "running" || !snapshot.jobs.iter().any(|job| job.key == key) {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.notWaitingApproval"),
        ));
    }
    use crate::entities::pipeline_run_jobs as jobs;
    let changed = jobs::Entity::update_many()
        .col_expr(jobs::Column::State, Expr::value("approved"))
        .filter(jobs::Column::RunId.eq(run.id))
        .filter(jobs::Column::JobKey.eq(&key))
        .filter(jobs::Column::State.eq("waiting"))
        .exec(&s.db)
        .await?
        .rows_affected;
    if changed == 0 {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.approvalProcessed"),
        ));
    }
    crate::hosts::activity(
        &s,
        &scope,
        "approve",
        "pipeline-runs",
        &Id::from(run.id),
        json!({"job":key}),
    )
    .await?;
    Ok(Json(json!({"ok":true})))
}
pub(crate) async fn finish(s: &AppState, id: i64, state: &str) -> Result<()> {
    let tx = s.db.begin().await?;
    let changed = runs::Entity::update_many()
        .col_expr(runs::Column::State, Expr::value(state))
        .col_expr(runs::Column::FinishedAt, Expr::value(db::now()))
        .filter(runs::Column::Id.eq(id))
        .filter(runs::Column::State.is_in(["queued", "running", "cancelling"]))
        .exec(&tx)
        .await?
        .rows_affected;
    if changed > 0 {
        // Recovery and queued cancellation also finish Actions job rows.
        // Normal orchestration already finalized its jobs; only unfinished
        // rows are changed here, in the same transaction as the run.
        use crate::entities::pipeline_run_jobs as jobs;
        jobs::Entity::update_many()
            .col_expr(
                jobs::Column::State,
                Expr::value(if state == "cancelled" {
                    "cancelled"
                } else {
                    "failed"
                }),
            )
            .col_expr(jobs::Column::FinishedAt, Expr::value(db::now()))
            .filter(jobs::Column::RunId.eq(id))
            .filter(jobs::Column::State.is_in(["running", "waiting", "approved"]))
            .exec(&tx)
            .await?;
        jobs::Entity::update_many()
            .col_expr(jobs::Column::State, Expr::value("skipped"))
            .col_expr(jobs::Column::FinishedAt, Expr::value(db::now()))
            .filter(jobs::Column::RunId.eq(id))
            .filter(jobs::Column::State.eq("queued"))
            .exec(&tx)
            .await?;
        steps::Entity::update_many()
            .col_expr(
                steps::Column::State,
                Expr::value(if state == "cancelled" {
                    "cancelled"
                } else {
                    "failed"
                }),
            )
            .col_expr(steps::Column::FinishedAt, Expr::value(db::now()))
            .filter(steps::Column::RunId.eq(id))
            .filter(steps::Column::State.eq("running"))
            .exec(&tx)
            .await?;
        steps::Entity::update_many()
            .col_expr(steps::Column::State, Expr::value("skipped"))
            .col_expr(steps::Column::FinishedAt, Expr::value(db::now()))
            .filter(steps::Column::RunId.eq(id))
            .filter(steps::Column::State.eq("queued"))
            .exec(&tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}
pub(crate) async fn active(s: &AppState, id: i64) -> Result<bool> {
    Ok(runs::Entity::find_by_id(id)
        .one(&s.db)
        .await?
        .is_some_and(|r| r.state == "running"))
}

pub async fn scheduler(
    s: AppState,
    mut stop: tokio::sync::watch::Receiver<bool>,
    mut drain: tokio::sync::watch::Receiver<bool>,
) {
    // No leases/multi-instance support yet. Interrupted runs cannot be resumed
    // safely: a remote command might have completed before the server crashed.
    if let Ok(rows) = runs::Entity::find()
        .filter(runs::Column::State.is_in(["running", "cancelling"]))
        .all(&s.db)
        .await
    {
        for r in rows {
            let _ = finish(&s, r.id, "failed").await;
        }
    }
    let mut next_cleanup = std::time::Instant::now();
    // Runs execute concurrently, at most `pipeline_concurrent_runs` per
    // workspace (0 = unlimited); finished tasks report their workspace back.
    let mut running = tokio::task::JoinSet::new();
    let mut busy: HashMap<i64, i64> = HashMap::new();
    loop {
        if *stop.borrow() || *drain.borrow() {
            break;
        }
        while let Some(done) = running.try_join_next() {
            if let Ok(workspace) = done
                && let Some(count) = busy.get_mut(&workspace)
            {
                *count -= 1;
            }
        }
        if std::time::Instant::now() >= next_cleanup {
            if let Err(error) = crate::maintenance::prune_history(&s, false).await {
                tracing::error!(%error,"历史清理失败");
            }
            if let Err(error) = crate::pipelines::artifacts::prune_expired(&s).await {
                tracing::error!(%error, "过期产物清理失败");
            }
            next_cleanup = std::time::Instant::now() + Duration::from_secs(60);
        }
        match claim(&s, &busy).await {
            Ok(Some(r)) => {
                *busy.entry(r.workspace_id).or_default() += 1;
                let (s, mut stop) = (s.clone(), stop.clone());
                running.spawn(async move {
                    let workspace = r.workspace_id;
                    if dispatch(&s, r, &mut stop).await.is_err() {
                        tracing::error!("流水线调度失败；稍后检查队列。");
                    }
                    workspace
                });
                // Claim the next eligible run without waiting.
                continue;
            }
            Ok(None) => {}
            Err(_) => tracing::error!("流水线调度失败；稍后检查队列。"),
        }
        tokio::select! { _=drain.changed()=>break, _=stop.changed()=>break, _=tokio::time::sleep(Duration::from_millis(100))=>{} }
    }
    // Shutdown waits for in-flight runs, as the sequential scheduler did.
    while running.join_next().await.is_some() {}
}

/// The oldest queued run whose workspace is below its concurrency limit,
/// atomically moved to running so other server instances cannot claim it.
async fn claim(s: &AppState, busy: &HashMap<i64, i64>) -> Result<Option<runs::Model>> {
    let queued = runs::Entity::find()
        .filter(runs::Column::State.eq("queued"))
        .order_by_asc(runs::Column::CreatedAt)
        .limit(500)
        .all(&s.db)
        .await?;
    let mut limits: HashMap<i64, i64> = HashMap::new();
    for r in queued {
        let limit = match limits.get(&r.workspace_id) {
            Some(limit) => *limit,
            None => {
                let limit = crate::maintenance::setting(
                    &s.db,
                    Id::from(r.workspace_id),
                    "pipeline_concurrent_runs",
                )
                .await?;
                limits.insert(r.workspace_id, limit);
                limit
            }
        };
        if limit > 0 && busy.get(&r.workspace_id).copied().unwrap_or(0) >= limit {
            continue;
        }
        if !r.concurrency_group.is_empty()
            && runs::Entity::find()
                .filter(runs::Column::WorkspaceId.eq(r.workspace_id))
                .filter(runs::Column::ConcurrencyGroup.eq(r.concurrency_group.clone()))
                .filter(runs::Column::State.is_in(["running", "cancelling"]))
                .one(&s.db)
                .await?
                .is_some()
        {
            continue;
        }
        let claimed = runs::Entity::update_many()
            .col_expr(runs::Column::State, Expr::value("running"))
            .col_expr(runs::Column::StartedAt, Expr::value(db::now()))
            .filter(runs::Column::Id.eq(r.id))
            .filter(runs::Column::State.eq("queued"))
            .exec(&s.db)
            .await?
            .rows_affected;
        if claimed == 1 {
            return Ok(Some(r));
        }
    }
    Ok(None)
}

async fn dispatch(
    s: &AppState,
    r: runs::Model,
    stop: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let r = &r;
    if execute(s, r, stop).await.is_err() {
        // Never log arbitrary remote errors here: they may contain credentials.
        append_message(
            s,
            r.id,
            -1,
            &mut logs::Entity::find()
                .filter(logs::Column::RunId.eq(r.id))
                .order_by_desc(logs::Column::Seq)
                .one(&s.db)
                .await?
                .map(|l| l.seq)
                .unwrap_or(0),
            "stderr",
            Message::new("execution.dispatchFailed"),
        )
        .await?;
        finish(s, r.id, "failed").await?;
    }
    Ok(())
}
pub(crate) fn quote(v: &str) -> String {
    format!("'{}'", v.replace('\'', "'\\''"))
}
pub(crate) async fn upload(
    ch: &dyn HostChannel,
    path: &str,
    value: String,
    mode: u32,
) -> Result<()> {
    ch.write_file(
        path,
        mode,
        Box::pin(futures::stream::once(async move { Ok(Bytes::from(value)) })),
    )
    .await
}
pub(crate) async fn append(
    s: &AppState,
    run: i64,
    index: i64,
    seq: &mut i64,
    stream: &str,
    text: &str,
) -> Result<()> {
    // UTF-8 boundaries must survive fragmentation. Sequence allocation is local
    // to the one claimed executor; the unique index is the database backstop.
    let mut rest = text;
    while !rest.is_empty() {
        let mut end = rest.len().min(16384);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        append_record(
            s,
            run,
            index,
            seq,
            stream,
            crate::i18n::store_raw(&rest[..end]),
        )
        .await?;
        rest = &rest[end..];
    }
    Ok(())
}

/// Application messages remain atomic; raw stdout/stderr always uses `append`.
pub(crate) async fn append_message(
    s: &AppState,
    run: i64,
    index: i64,
    seq: &mut i64,
    stream: &str,
    message: Message,
) -> Result<()> {
    append_record(
        s,
        run,
        index,
        seq,
        stream,
        crate::i18n::StoredText::from_message(message).stored(),
    )
    .await
}

async fn append_record(
    s: &AppState,
    run: i64,
    index: i64,
    seq: &mut i64,
    stream: &str,
    chunk: String,
) -> Result<()> {
    *seq += 1;
    logs::Entity::insert(logs::ActiveModel {
        id: Set(i64::from(s.next_id()?)),
        run_id: Set(run),
        seq: Set(*seq),
        step_index: Set(index),
        stream: Set(stream.into()),
        chunk: Set(chunk),
    })
    .exec(&s.db)
    .await?;
    Ok(())
}
async fn execute(
    s: &AppState,
    r: &runs::Model,
    stop: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    if let Some(snapshot) = crate::pipelines::actions::snapshot(&r.definition) {
        return crate::pipelines::actions_run::execute(s, r, snapshot, stop).await;
    }
    Err(ApiError::localized(
        StatusCode::BAD_REQUEST,
        Message::new("execution.historicalRun"),
    ))
}

pub(crate) fn checkout_script(r: &runs::Model) -> Result<String> {
    if r.repo_url.is_empty() {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.repositoryRequired"),
        ));
    }
    let target = if r.commit.is_empty() {
        format!("refs/remotes/origin/{}^{{commit}}", r.branch)
    } else {
        "FETCH_HEAD^{commit}".into()
    };
    let fetch = if r.commit.is_empty() {
        String::new()
    } else {
        format!(
            "git fetch --no-tags -- origin {} || exit $?\n",
            quote(&r.commit)
        )
    };
    Ok(format!(
        "git clone --no-checkout -- {} . || exit $?\n{fetch}revision=$(git rev-parse --verify --end-of-options {}) || exit $?\ngit checkout --detach \"$revision\" --\n",
        quote(&r.repo_url),
        quote(&target)
    ))
}

pub(crate) async fn credential_environment(
    ch: &dyn HostChannel,
    private: &str,
    credential: Option<&(String, String, String)>,
) -> Result<String> {
    let mut env = String::new();
    env.push_str("export GIT_TERMINAL_PROMPT=0\n");
    if let Some((kind, user, secret)) = credential {
        if kind == "ssh-key" {
            upload(ch, &format!("{private}/key"), secret.clone(), 0o600).await?;
            env.push_str(&format!(
                "export GIT_SSH_COMMAND={}\n",
                quote(&format!(
                    "ssh -o BatchMode=yes -o IdentitiesOnly=yes -i {}",
                    quote(&format!("{private}/key"))
                ))
            ));
        } else {
            upload(
                ch,
                &format!("{private}/username"),
                if user.is_empty() {
                    "git".into()
                } else {
                    user.clone()
                },
                0o600,
            )
            .await?;
            upload(ch, &format!("{private}/password"), secret.clone(), 0o600).await?;
            // ASKPASS receives Git's prompt as argv, including the credential username
            // on password prompts. The credential-helper protocol keeps both fields in pipes.
            upload(
                ch,
                &format!("{private}/credential-helper"),
                format!(
                    "#!/bin/sh\ncat >/dev/null\n[ \"$1\" = get ] || exit 0\nprintf 'username='\ncat {}\nprintf '\\npassword='\ncat {}\nprintf '\\n\\n'\n",
                    quote(&format!("{private}/username")),
                    quote(&format!("{private}/password")),
                ),
                0o700,
            ).await?;
            env.push_str(&format!(
                "unset GIT_ASKPASS SSH_ASKPASS\nexport GIT_CONFIG_COUNT=3\nexport GIT_CONFIG_KEY_0=credential.helper\nexport GIT_CONFIG_VALUE_0=''\nexport GIT_CONFIG_KEY_1=credential.helper\nexport GIT_CONFIG_VALUE_1={}\nexport GIT_CONFIG_KEY_2=core.askPass\nexport GIT_CONFIG_VALUE_2=''\n",
                quote(&format!("!{}", quote(&format!("{private}/credential-helper"))))
            ));
        }
    }
    Ok(env)
}

pub(crate) fn redact(
    mut text: String,
    credential: Option<&(String, String, String)>,
    variables: &BTreeMap<String, String>,
) -> String {
    let mut values: Vec<&str> = variables.values().map(String::as_str).collect();
    if let Some((_, user, secret)) = credential {
        values.extend([secret.as_str(), user.as_str()]);
    }
    values.extend(values.clone().into_iter().flat_map(str::lines));
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    values.dedup();
    for value in values {
        if !value.is_empty() {
            text = text.replace(value, "[REDACTED]");
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_reader_localizes_only_marked_unchanged_default_step_names() {
        use sea_orm::{ConnectionTrait, Schema};
        let dir = tempfile::tempdir().unwrap();
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        let schema = Schema::new(db.get_database_backend());
        for table in [
            schema.create_table_from_entity(runs::Entity),
            schema.create_table_from_entity(steps::Entity),
            schema.create_table_from_entity(crate::entities::pipeline_run_jobs::Entity),
            schema.create_table_from_entity(crate::entities::pipeline_artifacts::Entity),
        ] {
            db.execute(&table).await.unwrap();
        }
        let s = std::sync::Arc::new(crate::state::State {
            host_monitor: Default::default(),
            cluster: None,
            ids: std::sync::Arc::new(
                crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap(),
            ),
            db,
            data_dir: dir.path().into(),
            instance_id: "step-name-test".into(),
            server_mode: true,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), true).unwrap(),
        });
        let document = crate::pipelines::actions::document(&json!({"engine":"github-actions", "workflow":"jobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: echo default\n      - name: Run command\n        run: echo explicit\n      - run: echo engine\n"}).to_string()).unwrap();
        let (jobs, _) = crate::pipelines::actions::parse(&document).unwrap();
        let snapshot = crate::pipelines::actions::Snapshot {
            engine: "github-actions".into(),
            document,
            jobs,
            checkouts: vec![],
        };
        let definition = serde_json::to_string(&snapshot).unwrap();
        runs::Entity::insert(runs::ActiveModel {
            id: Set(1),
            workspace_id: Set(1),
            pipeline_id: Set(1),
            environment_id: Set(0),
            environment_name: Set(String::new()),
            inputs: Set(None),
            concurrency_group: Set(String::new()),
            pipeline_name: Set("literal pipeline".into()),
            host_name: Set("literal host".into()),
            triggered_by_name: Set("literal user".into()),
            host_id: Set(1),
            number: Set(1),
            state: Set("succeeded".into()),
            trigger_kind: Set("manual".into()),
            triggered_by: Set(1),
            definition: Set(definition.clone()),
            variable_snapshot: Set(None),
            repo_url: Set(String::new()),
            branch: Set(String::new()),
            commit: Set(String::new()),
            credential_id: Set(0),
            created_at: Set(0),
            started_at: Set(0),
            finished_at: Set(0),
        })
        .exec(&s.db)
        .await
        .unwrap();
        for (index, name) in [
            (0, "Run command"),
            (1, "Run command"),
            (2, "engine evaluated name {name}"),
        ] {
            steps::Entity::insert(steps::ActiveModel {
                id: Set(index + 2),
                run_id: Set(1),
                step_index: Set(index),
                name: Set(name.into()),
                state: Set("success".into()),
                exit_code: Set(Some(0)),
                started_at: Set(0),
                finished_at: Set(0),
            })
            .exec(&s.db)
            .await
            .unwrap();
        }
        for locale in crate::i18n::Locale::ALL {
            let scope = Scope {
                workspace_id: Id::from(1),
                user: crate::accounts::auth::User {
                    id: Id::from(1),
                    email: String::new(),
                    name: "literal user".into(),
                    is_superadmin: false,
                },
                permissions: std::collections::HashSet::from(["pipelines.read".into()]),
            };
            let Json(value) = crate::i18n::with_locale(
                locale,
                get(State(s.clone()), scope, Path(("1".into(), "1".into()))),
            )
            .await
            .unwrap();
            assert_eq!(value["definition_snapshot"], json!(snapshot.document));
            assert!(value["definition_snapshot"].get("checkouts").is_none());
            assert_eq!(
                value["steps"][0]["name"],
                crate::i18n::render(locale, &Message::new("actions.runCommand"))
            );
            assert_eq!(
                value["steps"][1]["name"], "Run command",
                "explicit identical user name stays literal"
            );
            assert_eq!(value["steps"][2]["name"], "engine evaluated name {name}");
        }
        let mut legacy: Value = serde_json::from_str(&definition).unwrap();
        for job in legacy["jobs"].as_array_mut().unwrap() {
            for step in job["steps"].as_array_mut().unwrap() {
                step.as_object_mut().unwrap().remove("default_name");
            }
        }
        let old = crate::pipelines::actions::snapshot(&legacy.to_string()).unwrap();
        for locale in crate::i18n::Locale::ALL {
            assert_eq!(
                crate::i18n::with_locale(locale, async {
                    display_step_name(Some(&old), 0, "Run command")
                })
                .await,
                "Run command"
            );
        }
        let stored = runs::Entity::find_by_id(1)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.definition, definition,
            "reader never rewrites the frozen snapshot"
        );
        let stored = steps::Entity::find()
            .order_by_asc(steps::Column::StepIndex)
            .all(&s.db)
            .await
            .unwrap();
        assert_eq!(stored[0].name, "Run command");
    }

    #[test]
    fn dispatch_input_errors_preserve_localized_type_and_raw_input_names() {
        for (kind, key) in [
            ("boolean", "execution.booleanKind"),
            ("number", "execution.numberKind"),
            ("choice", "execution.choiceKind"),
        ] {
            let name = "user {kind} {{name}}";
            let specs = [crate::pipelines::actions::InputSpec {
                name: name.into(),
                r#type: kind.into(),
                required: false,
                default: None,
                options: vec!["raw {option}".into()],
                description: String::new(),
            }];
            let error = dispatch_inputs(&specs, &BTreeMap::from([(name.into(), json!("invalid"))]))
                .unwrap_err();
            let message = error.2.as_ref().unwrap();
            assert_eq!(message.key, "execution.inputType");
            assert_eq!(message.params["name"], name);
            assert_eq!(message.children["kind"].key, key);
            for locale in crate::i18n::Locale::ALL {
                let displayed = error.render(locale);
                assert!(displayed.contains(name));
                if kind == "choice" {
                    assert!(displayed.contains("raw {option}"));
                }
                assert_eq!(
                    crate::i18n::render_stored(locale, &error.stored()),
                    displayed
                );
            }
        }
    }

    #[test]
    fn workflow_dispatch_inputs_follow_github_rules() {
        let spec =
            |name: &str, kind: &str, required: bool, default: Option<Value>, options: &[&str]| {
                crate::pipelines::actions::InputSpec {
                    name: name.into(),
                    r#type: kind.into(),
                    required,
                    default,
                    options: options.iter().map(|option| option.to_string()).collect(),
                    description: String::new(),
                }
            };
        let specs = vec![
            spec(
                "target",
                "choice",
                true,
                Some(json!("staging")),
                &["staging", "production"],
            ),
            spec("dry", "boolean", false, Some(json!(false)), &[]),
            spec("count", "number", false, None, &[]),
            spec("note", "string", true, None, &[]),
        ];
        let given = |value: Value| -> BTreeMap<String, Value> {
            value.as_object().unwrap().clone().into_iter().collect()
        };
        let values = dispatch_inputs(
            &specs,
            &given(json!({"note": "hi", "dry": "true", "count": "3"})),
        )
        .unwrap();
        assert_eq!(
            values["target"],
            json!("staging"),
            "default fills a missing input"
        );
        assert_eq!(values["dry"], json!(true));
        assert_eq!(values["count"], json!(3.0));
        assert_eq!(values["note"], json!("hi"));
        assert!(!values.contains_key("missing"));
        for bad in [
            json!({}),
            json!({"note": ""}),
            json!({"note": "x", "target": "dev"}),
            json!({"note": "x", "unknown": "y"}),
            json!({"note": "x", "dry": "yes"}),
            json!({"note": "x", "count": "many"}),
        ] {
            assert!(
                dispatch_inputs(&specs, &given(bad.clone())).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn shell_quote_preserves_single_quotes_without_evaluation() {
        assert_eq!(quote("a'b$(id)"), "'a'\\''b$(id)'");
    }

    #[test]
    fn redaction_masks_individual_lines_of_multiline_variables() {
        let variables = BTreeMap::from([(
            "TOKEN".into(),
            "first-secret-line\r\nsecond-secret-line".into(),
        )]);
        assert_eq!(
            redact("first-secret-line".into(), None, &variables),
            "[REDACTED]"
        );
        assert_eq!(
            redact("second-secret-line".into(), None, &variables),
            "[REDACTED]"
        );
    }

    #[test]
    fn redaction_precedes_chunking_and_covers_both_credential_parts() {
        let c = (
            "password".into(),
            "private-user".into(),
            "private-secret".into(),
        );
        let text = format!("{}private-secret\nprivate-user", "界".repeat(5460));
        let masked = redact(text, Some(&c), &BTreeMap::new());
        assert!(!masked.contains("private-secret"));
        assert!(!masked.contains("private-user"));
        assert_eq!(masked.matches("[REDACTED]").count(), 2);
    }

    #[test]
    fn expansion_does_not_recursively_interpret_values() {
        let r = runs::Model {
            id: 1,
            workspace_id: 1,
            pipeline_id: 1,
            environment_id: 0,
            environment_name: String::new(),
            inputs: None,
            concurrency_group: String::new(),
            pipeline_name: String::new(),
            host_name: String::new(),
            triggered_by_name: String::new(),
            host_id: 1,
            number: 12,
            state: "queued".into(),
            trigger_kind: "manual".into(),
            triggered_by: 1,
            definition: String::new(),
            variable_snapshot: None,
            repo_url: "https://example.invalid/{{run}}".into(),
            branch: "main".into(),
            commit: "abc123".into(),
            credential_id: 0,
            created_at: 0,
            started_at: 0,
            finished_at: 0,
        };
        assert_eq!(
            crate::pipelines::actions_run::runtime_path(
                "${{ github.run_number }} ${{ github.ref_name }} ${{ env.FERRY_REPOSITORY_URL }} ${{ github.sha }}",
                &r
            ),
            "12 main https://example.invalid/{{run}} abc123"
        );
    }
}
