//! Durable manual runs. The registered executors below are deliberately explicit:
//! a schema catalog entry alone never makes tenant code runnable.
use super::{
    graph,
    journal::{Activation, Journal, Output, Port, Source},
    plan,
};
use crate::{
    accounts::workspace::Scope,
    artifact_store::ArtifactStore,
    content_assets::{DEFAULT_QUOTA, Store},
    db,
    definitions::access::{self, Operation},
    entities::{
        asset_refs, definition_revisions, workflow_executions as runs, workflow_node_runs as nodes,
    },
    error::{ApiError, Json, Path, Query, Result},
    executions::{
        self,
        lease::{Execution, Kind, Lease},
    },
    i18n::Message,
    snowflake::Id,
    state::AppState,
};
use axum::{extract::State, http::StatusCode};
use ferry_protocol::sandbox::{InputData, Item, NodeOutput, Paired};
use sea_orm::{
    ColumnTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, TransactionTrait, sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, time::Duration};

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Manual,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Revision {
    Draft,
    Published,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    mode: Mode,
    revision: Revision,
    version: i64,
    entry_node_id: String,
    #[serde(default)]
    inputs: Map<String, Value>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    version: u8,
    revision_id: i64,
    plan: plan::Plan,
    inputs: Map<String, Value>,
    vars: BTreeMap<String, String>,
}
fn error(key: &'static str, path: &str) -> ApiError {
    ApiError::localized(
        StatusCode::BAD_REQUEST,
        Message::new("workflow.runNotReady")
            .with("path", path)
            .with_message("reason", Message::new(key)),
    )
}
fn context_aad(workspace: i64, id: i64) -> String {
    format!("workflow:{workspace}:{id}:execution:context")
}
fn supported(graph: &Value, plan: &plan::Plan) -> Result<()> {
    if graph
        .pointer("/settings/timeout_seconds")
        .and_then(Value::as_f64)
        .is_some_and(|seconds| seconds != 0.0)
        || graph.pointer("/settings/save_data").is_some()
    {
        return Err(error("workflow.runImplementation", "/settings"));
    }
    if graph
        .pointer("/settings/concurrency/scope")
        .and_then(Value::as_str)
        .is_some_and(|scope| scope != "none")
    {
        return Err(error("workflow.runImplementation", "/settings/concurrency"));
    }
    if graph
        .pointer("/settings/error_workflow_id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty())
    {
        return Err(error(
            "workflow.runImplementation",
            "/settings/error_workflow_id",
        ));
    }
    for node in &plan.nodes {
        let raw_spec = graph::node_at(graph, &node.path).ok_or_else(ApiError::missing)?;
        let effective_spec = super::host::effective(graph, raw_spec);
        let spec = &effective_spec;
        if spec["type"] == "host.command" {
            super::host::command_script(&spec["params"])?;
        }
        if super::host::requested(spec) && !super::integration::supports(spec) {
            return Err(error("workflow.runImplementation", &node.path));
        }
        if super::host::requested(spec)
            && spec
                .pointer("/settings/target_ref")
                .and_then(Value::as_str)
                .is_none()
        {
            return Err(error("workflow.hostTargetRequired", &node.path));
        }
        if (!matches!(
            spec["type"].as_str(),
            Some("core.trigger.manual" | "data.set" | "core.wait" | "core.stop" | "host.preview")
        ) && !super::transform::supports(spec)
            && !super::media::supports(spec)
            && !super::ai::supports(spec)
            && !super::integration::supports(spec)
            && !super::condition::supports(spec)
            && !(spec["type"] == "core.merge" && spec["params"]["mode"] == "append"))
            || (spec["type"] == "core.wait"
                && !matches!(spec["params"]["mode"].as_str(), Some("duration" | "until")))
            || spec
                .pointer("/settings/on_error")
                .and_then(Value::as_str)
                .is_some_and(|policy| policy != "stop")
            || spec
                .pointer("/settings/mode")
                .and_then(Value::as_str)
                .is_some_and(|mode| !matches!(mode, "active" | "mute" | "bypass"))
            || spec
                .pointer("/settings/retry/max_tries")
                .and_then(Value::as_f64)
                .is_some_and(|tries| tries != 1.0)
        {
            return Err(error("workflow.runImplementation", &node.path));
        }
        // Flat pins and bypass have no defined multi-port routing contract.
        if matches!(
            spec["type"].as_str(),
            Some("core.if" | "core.switch" | "host.preview")
        ) && (graph::pin_data(graph, spec, "manual").is_some()
            || spec.pointer("/settings/mode").and_then(Value::as_str) == Some("bypass")
            || spec
                .pointer("/settings/always_output")
                .and_then(Value::as_bool)
                == Some(true))
        {
            return Err(error("workflow.runImplementation", &node.path));
        }
        if spec["type"] == "core.merge"
            && spec.pointer("/settings/mode").and_then(Value::as_str) == Some("bypass")
        {
            return Err(error("workflow.runImplementation", &node.path));
        }
        if spec["type"] == "core.stop"
            && (graph::pin_data(graph, spec, "manual").is_some()
                || spec.pointer("/settings/mode").and_then(Value::as_str) == Some("bypass"))
        {
            return Err(error("workflow.runImplementation", &node.path));
        }
    }
    Ok(())
}
struct NoExternalSchema;
impl jsonschema::Retrieve for NoExternalSchema {
    fn retrieve(
        &self,
        _: &jsonschema::Uri<String>,
    ) -> std::result::Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err("external schema resources are not available".into())
    }
}
/// Publication checks current runtime support without executing tenant parameters.
/// Pins affect manual/test runs only and cannot supply production authority.
pub(super) fn publication_ready(graph: &Value) -> Result<()> {
    let mut production = graph.clone();
    production
        .as_object_mut()
        .ok_or_else(ApiError::missing)?
        .remove("pin_data");
    let nodes = production["nodes"]
        .as_array()
        .ok_or_else(ApiError::missing)?;
    let all = plan::Plan {
        version: 1,
        entry_node_id: String::new(),
        nodes: nodes
            .iter()
            .enumerate()
            .map(|(index, node)| plan::Node {
                id: node["id"].as_str().unwrap_or_default().into(),
                path: format!("/nodes/{index}"),
                inputs: vec![],
            })
            .collect(),
    };
    supported(&production, &all)?;
    let mut entries = 0;
    for node in nodes.iter().filter(|node| {
        node["type"]
            .as_str()
            .is_some_and(|kind| kind.starts_with("core.trigger."))
    }) {
        plan::compile(
            &production.to_string(),
            node["id"].as_str().unwrap_or_default(),
        )?;
        entries += 1;
    }
    if entries == 0 {
        return Err(error("workflow.runEntry", "/nodes"));
    }
    Ok(())
}

fn input(spec: &Value, inputs: &Map<String, Value>) -> Result<()> {
    let value = Value::Object(inputs.clone());
    if serde_json::to_vec(&json!({"json":value}))
        .map_err(|_| ApiError::missing())?
        .len()
        > 1024 * 1024
    {
        return Err(error("workflow.graphLimit", "/inputs"));
    }
    let validator = jsonschema::options()
        .with_retriever(NoExternalSchema)
        .build(&spec["params"]["input_schema"])
        .map_err(|_| error("workflow.runInputInvalid", "/inputs"))?;
    if !validator.is_valid(&value) {
        return Err(error("workflow.runInputInvalid", "/inputs"));
    }
    Ok(())
}
fn summary(run: &runs::Model) -> Value {
    json!({"id":Id::from(run.id),"workspace_id":Id::from(run.workspace_id),"workflow_id":Id::from(run.workflow_id),"revision_id":Id::from(run.revision_id),"number":run.number,"mode":run.mode,"state":run.state,"trigger_kind":run.trigger_kind,"trigger_ref":run.trigger_ref,"triggered_by":Id::from(run.triggered_by),"version":run.version,"created_at":run.created_at,"started_at":run.started_at,"finished_at":run.finished_at,"waiting_ms":run.waiting_ms,"data_bytes":run.data_bytes,"end_reason":run.end_reason,"queue_reason":run.queue_reason})
}
pub async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<Request>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("workflows.run")?;
    let tx = access::snapshot(&s.db).await?;
    let scope = crate::accounts::workspace_admin::authorize(&tx, &scope, "workflows.run").await?;
    let workflow = super::scoped(&tx, scope.workspace_id, id).await?;
    access::require(&tx, &scope, &workflow, Operation::Run).await?;
    if workflow.version != body.version {
        return Err(super::conflict("execution.versionConflict"));
    }
    if workflow.lifecycle != "active" {
        return Err(super::conflict("workflow.archived"));
    }
    let revision_id = match body.revision {
        Revision::Draft => workflow.draft_revision_id,
        Revision::Published => workflow
            .published_revision_id
            .ok_or_else(|| error("workflow.runImplementation", "/revision"))?,
    };
    let revision = definition_revisions::Entity::find_by_id(revision_id)
        .filter(definition_revisions::Column::WorkspaceId.eq(workflow.workspace_id))
        .filter(definition_revisions::Column::SubjectKind.eq("workflow"))
        .filter(definition_revisions::Column::SubjectId.eq(workflow.id))
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let plan = plan::compile(&revision.definition, &body.entry_node_id)?;
    let graph: Value =
        serde_json::from_str(&revision.definition).map_err(|_| ApiError::missing())?;
    supported(&graph, &plan)?;
    let entry = graph::node_at(
        &graph,
        &plan
            .nodes
            .iter()
            .find(|n| n.id == body.entry_node_id)
            .unwrap()
            .path,
    )
    .unwrap();
    if entry["type"] != "core.trigger.manual" {
        return Err(error("workflow.runEntry", "/entry_node_id"));
    }
    input(entry, &body.inputs)?;
    let vars = crate::variables::snapshot_for_workflow(&s, &tx, scope.workspace_id).await?;
    tx.commit().await?;
    let execution: i64 = s.next_id()?.into();
    let _mode = body.mode;
    let start = Start {
        version: 1,
        revision_id,
        plan,
        inputs: body.inputs,
        vars,
    };
    let plain = crate::definitions::canonical(
        &serde_json::to_string(&start).map_err(|_| ApiError::missing())?,
    )?;
    let backend = ArtifactStore::for_state(&s).await?;
    let store = Store {
        db: &s.db,
        backend: &backend,
        vault: &s.vault,
        ids: &s.ids,
        quota: DEFAULT_QUOTA,
    };
    let asset = store
        .stage(workflow.workspace_id, &plain, execution, "execution")
        .await?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let scope = crate::accounts::workspace_admin::authorize(&tx, &scope, "workflows.run").await?;
    let fresh = super::scoped(&tx, scope.workspace_id, id).await?;
    access::require(&tx, &scope, &fresh, Operation::Run).await?;
    let fresh_revision = match body.revision {
        Revision::Draft => Some(fresh.draft_revision_id),
        Revision::Published => fresh.published_revision_id,
    };
    if fresh.version != body.version || fresh_revision != Some(revision_id) {
        return Err(super::conflict("execution.versionConflict"));
    }
    if fresh.lifecycle != "active" {
        return Err(super::conflict("workflow.archived"));
    }
    store
        .attach(&tx, workflow.workspace_id, asset.id, "execution", execution)
        .await?;
    let number = executions::next_workflow_number(&tx, scope.workspace_id).await?;
    let now = db::now();
    let context = s.vault.seal(
        &json!({"version":1,"start_asset":asset.id,"origin_agent_id":crate::platform_tools::current_agent()}).to_string(),
        &context_aad(workflow.workspace_id, execution),
    )?;
    let run = runs::Model {
        id: execution,
        workspace_id: workflow.workspace_id,
        workflow_id: workflow.id,
        revision_id,
        number,
        mode: "manual".into(),
        state: "queued".into(),
        trigger_kind: "manual".into(),
        trigger_ref: body.entry_node_id,
        triggered_by: scope.user.id.into(),
        parent_execution_id: 0,
        retry_of_id: 0,
        root_execution_id: execution,
        cost_micros: 0,
        data_bytes: asset.size,
        created_at: now,
        created_by: scope.user.id.into(),
        started_at: 0,
        started_by: 0,
        finished_at: 0,
        finished_by: 0,
        lease_owner: String::new(),
        lease_epoch: 0,
        priority: 0,
        not_before: None,
        end_reason: String::new(),
        concurrency_key: String::new(),
        concurrency_limit: 0,
        queue_reason: String::new(),
        context: Some(context),
        waiting_ms: 0,
        response_state: "not_applicable".into(),
        response_deadline_at: None,
        response_deadline_by: 0,
        response_delivered_at: None,
        response_delivered_by: 0,
        version: 1,
    };
    runs::Entity::insert(run.clone().into_active_model())
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "run",
        "workflows",
        &id,
        json!({"execution_id":Id::from(execution),"revision_id":Id::from(revision_id)}),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(summary(&run))))
}
async fn authorized(
    tx: &sea_orm::DatabaseTransaction,
    scope: &Scope,
    id: Id,
) -> Result<runs::Model> {
    let scope = crate::accounts::workspace_admin::authorize(tx, scope, "workflows.read").await?;
    let run = runs::Entity::find_by_id(i64::from(id))
        .filter(runs::Column::WorkspaceId.eq(scope.workspace_id))
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let workflow = super::scoped(tx, scope.workspace_id, run.workflow_id.into()).await?;
    access::require(tx, &scope, &workflow, Operation::View).await?;
    Ok(run)
}
pub async fn get(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("workflows.read")?;
    let tx = access::snapshot(&s.db).await?;
    let run = authorized(&tx, &scope, id).await?;
    tx.commit().await?;
    Ok(Json(summary(&run)))
}

pub async fn cancel(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("workflows.run")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let scope = crate::accounts::workspace_admin::authorize(&tx, &scope, "workflows.run").await?;
    let run = runs::Entity::find_by_id(i64::from(id))
        .filter(runs::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let workflow = super::scoped(&tx, scope.workspace_id, run.workflow_id.into()).await?;
    access::require(&tx, &scope, &workflow, Operation::Run).await?;
    let revoked = request_cancel_in(&tx, &run, scope.user.id.into()).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "cancel",
        "workflow-executions",
        &id,
        json!({}),
    )
    .await?;
    let run = runs::Entity::find_by_id(run.id)
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    tx.commit().await?;
    if let Some(revoked) = revoked {
        super::preview::ended(run.id, &revoked);
    }
    Ok(Json(summary(&run)))
}

/// `Some(previews revoked)` when the execution ended now; the caller passes
/// them to `preview::ended` once the transaction committed.
async fn request_cancel_in(
    tx: &sea_orm::DatabaseTransaction,
    run: &runs::Model,
    actor: i64,
) -> Result<Option<Vec<i64>>> {
    if !matches!(run.state.as_str(), "queued" | "waiting" | "running") {
        return Ok(None);
    }
    let immediate = run.state != "running";
    let mut update = runs::Entity::update_many()
        .col_expr(
            runs::Column::State,
            Expr::value(if immediate { "cancelled" } else { "cancelling" }),
        )
        .col_expr(
            runs::Column::Version,
            Expr::value(run.version.checked_add(1).ok_or_else(ApiError::missing)?),
        );
    if immediate {
        update = update
            .col_expr(runs::Column::EndReason, Expr::value("user_cancelled"))
            .col_expr(runs::Column::FinishedAt, Expr::value(db::now()))
            .col_expr(runs::Column::FinishedBy, Expr::value(actor));
    }
    if update
        .filter(runs::Column::Id.eq(run.id))
        .filter(runs::Column::WorkspaceId.eq(run.workspace_id))
        .filter(runs::Column::State.eq(&run.state))
        .filter(runs::Column::Version.eq(run.version))
        .exec(tx)
        .await?
        .rows_affected
        != 1
    {
        return Err(super::conflict("execution.versionConflict"));
    }
    if !immediate {
        return Ok(None);
    }
    finish_nodes(tx, run.id, "cancelled", "user_cancelled", actor, db::now()).await?;
    Ok(Some(
        crate::holes::revoke_source(tx, "workflow", run.id, None).await?,
    ))
}

pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Query(raw): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>> {
    use crate::resources::resource::{self, ListQuery, Spec};
    scope.require("workflows.read")?;
    let tx = access::snapshot(&s.db).await?;
    let scope = crate::accounts::workspace_admin::authorize(&tx, &scope, "workflows.read").await?;
    let workflow = super::scoped(&tx, scope.workspace_id, id).await?;
    access::require(&tx, &scope, &workflow, Operation::View).await?;
    const SPEC: Spec = Spec {
        search: &[],
        filters: &["state", "mode"],
        sortable: &["number", "created_at", "started_at", "finished_at"],
        default_sort: "-number",
    };
    let query = ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) = resource::list(
        &tx,
        &SPEC,
        runs::Entity::find().filter(runs::Column::WorkflowId.eq(id)),
        &scope.workspace_id,
        &query,
    )
    .await?;
    let revision_ids: Vec<_> = rows
        .iter()
        .map(|row: &runs::Model| row.revision_id)
        .collect();
    let revisions: BTreeMap<i64, i64> = definition_revisions::Entity::find()
        .select_only()
        .column(definition_revisions::Column::Id)
        .column(definition_revisions::Column::Number)
        .filter(definition_revisions::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(definition_revisions::Column::SubjectKind.eq("workflow"))
        .filter(definition_revisions::Column::SubjectId.eq(id))
        .filter(definition_revisions::Column::Id.is_in(revision_ids))
        .into_tuple()
        .all(&tx)
        .await?
        .into_iter()
        .collect();
    use crate::entities::users;
    let actors: BTreeMap<i64, String> = users::Entity::find()
        .select_only()
        .column(users::Column::Id)
        .column(users::Column::Name)
        .filter(users::Column::Id.is_in(rows.iter().map(|row| row.triggered_by)))
        .into_tuple()
        .all(&tx)
        .await?
        .into_iter()
        .collect();
    let items: Vec<_> = rows
        .iter()
        .map(|row| {
            let mut value = summary(row);
            value["revision_number"] = json!(revisions.get(&row.revision_id));
            value["triggered_by_name"] = json!(actors.get(&row.triggered_by));
            value
        })
        .collect();
    tx.commit().await?;
    Ok(Json(json!({"items":items,
        "total":total,"page":query.page,"page_size":query.page_size})))
}

pub async fn node_output(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, node)): Path<(String, Id, Id)>,
) -> Result<Json<Value>> {
    scope.require("workflows.read")?;
    let tx = access::snapshot(&s.db).await?;
    let run = authorized(&tx, &scope, id).await?;
    let node = nodes::Entity::find_by_id(node)
        .filter(nodes::Column::ExecutionId.eq(run.id))
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    if !matches!(
        node.state.as_str(),
        "succeeded" | "skipped" | "bypassed" | "cached" | "pinned" | "reused"
    ) {
        return Err(ApiError::missing());
    }
    let backend = ArtifactStore::for_connection(&s, &tx).await?;
    let store = Store {
        db: &s.db,
        backend: &backend,
        vault: &s.vault,
        ids: &s.ids,
        quota: DEFAULT_QUOTA,
    };
    let output: Output = super::journal::read_packet(
        &s.vault,
        Some(&store),
        &tx,
        &run,
        node.id,
        "output",
        &node.output,
        node.output_asset,
    )
    .await?;
    output.validate(output.ports.is_empty())?;
    tx.commit().await?;
    Ok(Json(
        serde_json::to_value(output).map_err(|_| ApiError::missing())?,
    ))
}

/// Inspect frozen inputs and resolved parameters without running any node again.
pub async fn node_data(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, node)): Path<(String, Id, Id)>,
) -> Result<Json<Value>> {
    scope.require("workflows.read")?;
    let tx = access::snapshot(&s.db).await?;
    let run = authorized(&tx, &scope, id).await?;
    let node = nodes::Entity::find_by_id(node)
        .filter(nodes::Column::ExecutionId.eq(run.id))
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let backend = ArtifactStore::for_connection(&s, &tx).await?;
    let store = Store {
        db: &s.db,
        backend: &backend,
        vault: &s.vault,
        ids: &s.ids,
        quota: DEFAULT_QUOTA,
    };
    let snapshot = super::journal::read_snapshot(&s.vault, Some(&store), &tx, &run, &node).await?;
    let mut items = Vec::new();
    let mut upstream = Vec::new();
    let mut bytes = 0;
    for source in &snapshot.sources {
        let row = nodes::Entity::find_by_id(source.node_run)
            .filter(nodes::Column::ExecutionId.eq(run.id))
            .one(&tx)
            .await?
            .ok_or_else(ApiError::missing)?;
        if row.attempt != source.attempt || row.iteration_hash != source.iteration_hash {
            return Err(error("workflow.executionDataUnavailable", "/input"));
        }
        let source_snapshot =
            super::journal::read_snapshot(&s.vault, Some(&store), &tx, &run, &row).await?;
        let output: Output = super::journal::read_packet(
            &s.vault,
            Some(&store),
            &tx,
            &run,
            row.id,
            "output",
            &row.output,
            row.output_asset,
        )
        .await?;
        output.validate(source_snapshot.outputless)?;
        let port = output
            .ports
            .iter()
            .find(|p| p.name == source.port && p.index == source.port_index)
            .ok_or_else(ApiError::missing)?;
        let canonical = crate::definitions::canonical(
            &serde_json::to_string(port).map_err(|_| ApiError::missing())?,
        )?;
        if hex::encode(Sha256::digest(canonical.as_bytes())) != source.output_sha256 {
            return Err(error("workflow.executionDataUnavailable", "/input"));
        }
        let offset: usize = output
            .ports
            .iter()
            .take_while(|p| p.name != port.name || p.index != port.index)
            .map(|p| p.items.len())
            .sum();
        for (index, mut item) in port.items.iter().cloned().enumerate() {
            item.paired = vec![Paired {
                node: row.node_id.clone(),
                iteration_hash: row.iteration_hash.clone(),
                index: offset + index,
            }];
            super::transform::append(&mut items, &mut bytes, item)?;
        }
        upstream.push(json!({"node":row.node_id,"iteration_hash":row.iteration_hash,"port":source.port,"index":source.port_index,
            "items":output.ports.iter().flat_map(|port|port.items.iter()).collect::<Vec<_>>()}));
    }
    let output = if matches!(
        node.state.as_str(),
        "succeeded" | "skipped" | "bypassed" | "cached" | "pinned" | "reused"
    ) {
        let output: Output = super::journal::read_packet(
            &s.vault,
            Some(&store),
            &tx,
            &run,
            node.id,
            "output",
            &node.output,
            node.output_asset,
        )
        .await?;
        output.validate(snapshot.outputless)?;
        Some(output)
    } else {
        None
    };
    tx.commit().await?;
    Ok(Json(
        json!({"input":{"items":items,"sources":upstream},"output":output,
        "resolved_params":snapshot.params,"frozen_at":snapshot.frozen_at_ms,
        "revision_id":Id::from(run.revision_id),"state":node.state,"end_reason":run.end_reason,
        "logs":node.logs,"logs_truncated":node.logs_truncated,"logs_updated_at":node.logs_updated_at}),
    ))
}

pub async fn node_runs(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("workflows.read")?;
    let tx = access::snapshot(&s.db).await?;
    let run = authorized(&tx, &scope, id).await?;
    let rows = nodes::Entity::find()
        .filter(nodes::Column::ExecutionId.eq(run.id))
        .order_by_asc(nodes::Column::StartedAt)
        .order_by_asc(nodes::Column::Id)
        .all(&tx)
        .await?;
    let items:Vec<_>=rows.iter().map(|n|json!({"id":Id::from(n.id),"node_id":n.node_id,"iteration":n.iteration_path,"attempt":n.attempt,"state":n.state,"items_in":n.items_in,"items_out":n.items_out,"version":n.version,"started_at":n.started_at,"finished_at":n.finished_at,"logs_updated_at":n.logs_updated_at,"logs_truncated":n.logs_truncated})).collect();
    tx.commit().await?;
    Ok(Json(
        json!({"items":items,"total":items.len(),"page":1,"page_size":items.len()}),
    ))
}

async fn current(s: &AppState, id: i64) -> Result<runs::Model> {
    runs::Entity::find_by_id(id)
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)
}
pub(super) fn failure_reason(error: &ApiError) -> &'static str {
    if let Some(message) = &error.2 {
        match message.key.as_str() {
            "workflow.integrationUnknown" => return "integration_unknown",
            "workflow.integrationRemote" => return "integration_remote",
            "workflow.integrationInvalid" => return "integration_invalid",
            _ => {}
        }
    }
    if error
        .2
        .as_ref()
        .is_some_and(|message| message.key == "workflow.nodeExecutionLimit")
    {
        return "budget_exceeded";
    }
    match error.0 {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "permission_revoked",
        _ => "node_failed",
    }
}
async fn finish(
    db: &sea_orm::DatabaseConnection,
    lease: &Lease,
    state: &str,
    reason: &str,
) -> Result<()> {
    let tx = db.begin().await?;
    let run = current_in(&tx, lease).await?;
    let (state, reason) = if run.state == "cancelling" {
        ("cancelled", "user_cancelled")
    } else {
        (state, reason)
    };
    let mut version = run.version;
    let now = db::now();
    if matches!(state, "failed" | "cancelled") {
        // Keep Journal's parent-before-child lock order. A stale owner must not
        // finish attempts belonging to the current execution owner.
        version = version.checked_add(1).ok_or_else(ApiError::missing)?;
        if runs::Entity::update_many()
            .col_expr(runs::Column::Version, Expr::value(version))
            .filter(executions::workflow_state_condition(
                lease,
                run.version,
                &run.state,
            ))
            .exec(&tx)
            .await?
            .rows_affected
            != 1
        {
            return Err(executions::ownership_lost());
        }
        finish_nodes(&tx, run.id, state, reason, 0, now).await?;
    }
    if runs::Entity::update_many()
        .col_expr(runs::Column::State, Expr::value(state))
        .col_expr(runs::Column::EndReason, Expr::value(reason))
        .col_expr(runs::Column::FinishedAt, Expr::value(now))
        .col_expr(runs::Column::FinishedBy, Expr::value(0_i64))
        .col_expr(
            runs::Column::Version,
            Expr::value(version.checked_add(1).ok_or_else(ApiError::missing)?),
        )
        .filter(executions::workflow_state_condition(
            lease, version, &run.state,
        ))
        .exec(&tx)
        .await?
        .rows_affected
        != 1
    {
        return Err(executions::ownership_lost());
    }
    let revoked = crate::holes::revoke_source(&tx, "workflow", run.id, None).await?;
    tx.commit().await?;
    super::preview::ended(run.id, &revoked);
    Ok(())
}

async fn finish_nodes(
    tx: &sea_orm::DatabaseTransaction,
    run_id: i64,
    state: &str,
    reason: &str,
    actor: i64,
    now: i64,
) -> Result<()> {
    let active = nodes::Entity::find()
        .filter(nodes::Column::ExecutionId.eq(run_id))
        .filter(nodes::Column::State.is_in(["running", "waiting"]))
        .all(tx)
        .await?;
    for node in active {
        let changed = nodes::Entity::update_many()
            .col_expr(nodes::Column::State, Expr::value(state))
            .col_expr(nodes::Column::Error, Expr::value(reason))
            .col_expr(nodes::Column::FinishedAt, Expr::value(now))
            .col_expr(nodes::Column::FinishedBy, Expr::value(actor))
            .col_expr(
                nodes::Column::Version,
                Expr::value(node.version.checked_add(1).ok_or_else(ApiError::missing)?),
            )
            .filter(nodes::Column::Id.eq(node.id))
            .filter(nodes::Column::ExecutionId.eq(run_id))
            .filter(nodes::Column::State.eq(&node.state))
            .filter(nodes::Column::Version.eq(node.version))
            .exec(tx)
            .await?
            .rows_affected;
        if changed != 1 {
            return Err(executions::ownership_lost());
        }
    }
    Ok(())
}
async fn current_in(tx: &sea_orm::DatabaseTransaction, lease: &Lease) -> Result<runs::Model> {
    runs::Entity::find_by_id(lease.execution().id)
        .filter(runs::Column::WorkspaceId.eq(lease.execution().workspace))
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)
}

async fn dispatch(
    s: &AppState,
    backend: &ArtifactStore,
    lease: &Lease,
    stop: &tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let run = current(s, lease.execution().id).await?;
    let context: Value = serde_json::from_str(&s.vault.open_secret(
        run.context.as_deref().ok_or_else(ApiError::missing)?,
        &context_aad(run.workspace_id, run.id),
    )?)
    .map_err(|_| ApiError::missing())?;
    if let Some(id) = context
        .get("origin_agent_id")
        .filter(|value| !value.is_null())
    {
        let agent_id: Id = serde_json::from_value(id.clone()).map_err(|_| ApiError::forbidden())?;
        let scope = Scope::for_actor(s, run.workspace_id.into(), run.triggered_by.into()).await?;
        crate::platform_tools::delegated_future(
            s,
            scope,
            agent_id,
            dispatch_inner(s, backend, lease, stop),
        )
        .await
    } else {
        dispatch_inner(s, backend, lease, stop).await
    }
}

async fn dispatch_inner(
    s: &AppState,
    backend: &ArtifactStore,
    lease: &Lease,
    stop: &tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let store = Store {
        db: &s.db,
        backend,
        vault: &s.vault,
        ids: &s.ids,
        quota: DEFAULT_QUOTA,
    };
    let journal = Journal {
        db: &s.db,
        vault: &s.vault,
        lease,
        assets: Some(&store),
        authority: Some(s),
    };
    let mut run = current(s, lease.execution().id).await?;
    if run.state == "cancelling" {
        return finish(&s.db, lease, "cancelled", "user_cancelled").await;
    }
    let context: Value = serde_json::from_str(&s.vault.open_secret(
        run.context.as_deref().ok_or_else(ApiError::missing)?,
        &context_aad(run.workspace_id, run.id),
    )?)
    .map_err(|_| ApiError::missing())?;
    let asset = context["start_asset"]
        .as_i64()
        .ok_or_else(ApiError::missing)?;
    if asset_refs::Entity::find_by_id((asset, "execution".to_owned(), run.id))
        .one(&s.db)
        .await?
        .is_none()
    {
        return Err(error("workflow.executionDataUnavailable", "/context"));
    }
    let start: Start = serde_json::from_str(&store.read(&s.db, run.workspace_id, asset).await?)
        .map_err(|_| ApiError::missing())?;
    if start.version != 1
        || start.revision_id != run.revision_id
        || start.plan.version != 1
        || start.plan.entry_node_id != run.trigger_ref
    {
        return Err(error("workflow.executionDataUnavailable", "/context"));
    }
    let revision = definition_revisions::Entity::find_by_id(run.revision_id)
        .filter(definition_revisions::Column::WorkspaceId.eq(run.workspace_id))
        .filter(definition_revisions::Column::SubjectId.eq(run.workflow_id))
        .filter(definition_revisions::Column::SubjectKind.eq("workflow"))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let graph: Value =
        serde_json::from_str(&revision.definition).map_err(|_| ApiError::missing())?;
    if plan::compile(&revision.definition, &run.trigger_ref)? != start.plan {
        return Err(error("workflow.executionDataUnavailable", "/plan"));
    }
    supported(&graph, &start.plan)?;
    if context.pointer("/journal/parked_at").is_some() {
        let due = super::preview::due(&s.db, run.id, &graph).await?;
        journal.wake(run.version, &due).await?;
        run = current(s, run.id).await?;
    }
    let hash = hex::encode(Sha256::digest(b"[]"));
    let mut completed: BTreeMap<String, (nodes::Model, Output)> = BTreeMap::new();
    let mut waiting = false;
    for planned in &start.plan.nodes {
        if *stop.borrow() {
            return Ok(());
        }
        let raw_spec = graph::node_at(&graph, &planned.path).ok_or_else(ApiError::missing)?;
        let effective_spec = super::host::effective(&graph, raw_spec);
        let spec = &effective_spec;
        let existing = nodes::Entity::find()
            .filter(nodes::Column::ExecutionId.eq(run.id))
            .filter(nodes::Column::NodeId.eq(&planned.id))
            .filter(nodes::Column::IterationHash.eq(&hash))
            .filter(nodes::Column::Attempt.eq(1_i64))
            .one(&s.db)
            .await?;
        if let Some(node) = &existing {
            if matches!(
                node.state.as_str(),
                "succeeded" | "skipped" | "bypassed" | "pinned"
            ) {
                completed.insert(
                    planned.id.clone(),
                    (node.clone(), journal.output(&s.db, &run, node).await?),
                );
                continue;
            }
            if node.state == "waiting" {
                waiting = true;
                continue;
            }
        }
        if planned
            .inputs
            .iter()
            .any(|input| !completed.contains_key(&input.source.node))
        {
            continue;
        }
        let mut sources = Vec::new();
        let mut items = Vec::new();
        let mode = spec
            .pointer("/settings/mode")
            .and_then(Value::as_str)
            .unwrap_or("active");
        let mut skipped = mode == "mute";
        let mut any_skipped = false;
        let mut any_items = false;
        let mut input_bytes = 0;
        for input in &planned.inputs {
            let (node, output) = &completed[&input.source.node];
            let port = output
                .ports
                .iter()
                .find(|port| port.name == input.source.port && port.index == input.source.index)
                .ok_or_else(ApiError::missing)?;
            let canonical = crate::definitions::canonical(
                &serde_json::to_string(port).map_err(|_| ApiError::missing())?,
            )?;
            sources.push(Source {
                node_run: node.id,
                attempt: node.attempt,
                iteration_hash: node.iteration_hash.clone(),
                port: port.name.clone(),
                port_index: port.index,
                output_sha256: hex::encode(Sha256::digest(canonical.as_bytes())),
            });
            any_skipped |= port.skipped;
            any_items |= !port.items.is_empty();
            let offset: usize = output
                .ports
                .iter()
                .take_while(|candidate| {
                    candidate.name != port.name || candidate.index != port.index
                })
                .map(|candidate| candidate.items.len())
                .sum();
            for (index, mut item) in port.items.iter().cloned().enumerate() {
                item.paired = vec![Paired {
                    node: node.node_id.clone(),
                    iteration_hash: node.iteration_hash.clone(),
                    index: offset + index,
                }];
                super::transform::append(&mut items, &mut input_bytes, item)?;
            }
        }
        skipped |= if spec["type"] == "core.merge" && spec["params"]["wait"] == "any_items" {
            !any_items
        } else {
            any_skipped
        };
        let pin = graph::pin_data(&graph, spec, &run.mode);
        let selected = if spec
            .pointer("/settings/execute_once")
            .and_then(Value::as_bool)
            == Some(true)
        {
            items.len().min(1)
        } else {
            items.len()
        };
        let node_id = match &existing {
            Some(node) => node.id,
            None => s.next_id()?.into(),
        };
        let now = db::now();
        let params = if existing.is_some() {
            vec![]
        } else if skipped || mode == "bypass" || pin.is_some() {
            vec![]
        } else {
            let (clock, today) = super::expression::clock(&graph["settings"], now)?;
            let upstream = completed
                .values()
                .map(|(node, output)| NodeOutput {
                    node: node.node_id.clone(),
                    name: graph["nodes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|spec| spec["id"] == node.node_id)
                        .unwrap()["name"]
                        .as_str()
                        .unwrap()
                        .into(),
                    iteration_hash: node.iteration_hash.clone(),
                    items: output
                        .ports
                        .iter()
                        .flat_map(|port| port.items.iter().cloned())
                        .collect(),
                })
                .collect();
            let selections = if spec["type"] == "data.set"
                || super::media::supports(spec)
                || super::ai::supports(spec)
                || (super::integration::supports(spec) && spec["type"] != "host.command")
                || super::transform::per_item(spec)
                || super::condition::supports(spec)
            {
                (0..selected).map(Some).collect()
            } else {
                vec![None]
            };
            graph::resolve_attempt(spec,&planned.path,InputData{items:items.clone(),upstream},&selections,json!({"$vars":start.vars,"$node":{"id":spec["id"],"name":spec["name"],"type":spec["type"]},"$workflow":{"id":Id::from(run.workflow_id)},"$execution":{"id":Id::from(run.id),"mode":"manual"},"$now":clock,"$today":today}),true).await?
        };
        let active = journal
            .activate(
                run.version,
                Activation {
                    id: node_id,
                    node_path: planned.path.clone(),
                    iteration: json!([]),
                    attempt: 1,
                    redo_round: 0,
                    params,
                    frozen_at_ms: now,
                    sources,
                    seed: None,
                },
            )
            .await?;
        run = current(s, run.id).await?;
        let mut conditional =
            if super::condition::supports(spec) && !skipped && mode == "active" && pin.is_none() {
                Some(
                    super::condition::execute(spec, &items[..selected], &active.snapshot.params)
                        .await?,
                )
            } else {
                None
            };
        let result = if skipped || conditional.is_some() {
            vec![]
        } else if mode == "bypass" {
            items
        } else if let Some(pin) = pin {
            graph::pin_items(pin)?
        } else {
            match spec["type"].as_str().unwrap() {
                "core.trigger.manual" => vec![Item {
                    json: start.inputs.clone(),
                    binary: Default::default(),
                    paired: vec![],
                }],
                "data.set" => set(&items[..selected], &active.snapshot.params)?,
                _ if super::integration::supports(spec) => {
                    Box::pin(super::integration::execute(
                        s,
                        &run,
                        node_id,
                        spec,
                        &items[..selected],
                        &active.snapshot.params,
                    ))
                    .await?
                }
                _ if super::ai::supports(spec) => {
                    super::ai::execute(
                        s,
                        &store,
                        &run,
                        node_id,
                        spec,
                        &items[..selected],
                        &active.snapshot.params,
                    )
                    .await?
                }
                _ if super::media::supports(spec) => {
                    super::media::execute(
                        s,
                        &store,
                        run.workspace_id,
                        run.workflow_id,
                        run.id,
                        spec["id"].as_str().unwrap(),
                        spec,
                        &items[..selected],
                        &active.snapshot.params,
                    )
                    .await?
                }
                "core.stop" => {
                    return Err(ApiError::localized(
                        StatusCode::BAD_REQUEST,
                        Message::new("workflow.nodeStopped"),
                    ));
                }
                "core.merge" => items.into_iter().take(selected).collect(),
                "host.preview" => {
                    match super::preview::execute(s, &run, &planned.id, &active.snapshot.params[0])
                        .await?
                    {
                        super::preview::Step::Wait(deadline) => {
                            journal
                                .wait(run.version, node_id, active.node.version, deadline)
                                .await?;
                            run = current(s, run.id).await?;
                            waiting = true;
                            continue;
                        }
                        super::preview::Step::Done(output) => {
                            conditional = Some(output);
                            vec![]
                        }
                    }
                }
                _ if super::transform::supports(spec) => {
                    super::transform::execute(spec, &items[..selected], &active.snapshot.params)?
                }
                "core.wait" => {
                    let params = &active.snapshot.params[0];
                    let deadline = if params["mode"] == "duration" {
                        active
                            .snapshot
                            .frozen_at_ms
                            .checked_add(
                                graph::port_index(&params["duration_ms"])
                                    .and_then(|value| i64::try_from(value).ok())
                                    .ok_or_else(|| error("workflow.graphSchema", &planned.path))?,
                            )
                            .ok_or_else(ApiError::missing)?
                    } else {
                        chrono::DateTime::parse_from_rfc3339(
                            params["until"].as_str().ok_or_else(ApiError::missing)?,
                        )
                        .map_err(|_| error("workflow.graphSchema", &planned.path))?
                        .timestamp_millis()
                    };
                    if deadline > db::now() {
                        journal
                            .wait(run.version, node_id, active.node.version, deadline)
                            .await?;
                        run = current(s, run.id).await?;
                        waiting = true;
                        continue;
                    }
                    items.into_iter().take(selected).collect()
                }
                _ => return Err(error("workflow.runImplementation", &planned.path)),
            }
        };
        if *stop.borrow() {
            return Ok(());
        }
        let output = if let Some(mut output) = conditional {
            for port in &mut output.ports {
                port.items = successful_output(spec, port.skipped, std::mem::take(&mut port.items));
            }
            output
        } else if skipped && matches!(spec["type"].as_str(), Some("core.if" | "core.switch")) {
            Output {
                version: 1,
                ports: super::condition::ports(spec)?,
            }
        } else if skipped && spec["type"] == "host.preview" {
            super::preview::skipped(spec)
        } else if spec["type"] == "core.stop" {
            Output {
                version: 1,
                ports: vec![],
            }
        } else {
            Output {
                version: 1,
                ports: vec![Port {
                    name: "main".into(),
                    index: 0,
                    skipped,
                    items: if pin.is_some() {
                        result
                    } else {
                        successful_output(spec, skipped, result)
                    },
                }],
            }
        };
        let done = journal
            .complete(run.version, node_id, active.node.version, output)
            .await?;
        run = current(s, run.id).await?;
        completed.insert(
            planned.id.clone(),
            (
                done.node.clone(),
                journal.output(&s.db, &run, &done.node).await?,
            ),
        );
    }
    if *stop.borrow() {
        return Ok(());
    }
    if completed.len() == start.plan.nodes.len() {
        finish(&s.db, lease, "succeeded", "").await?;
    } else if waiting {
        journal.park(run.version).await?;
    } else {
        return Err(error("workflow.runMissingInput", "/plan"));
    }
    Ok(())
}
fn successful_output(spec: &Value, skipped: bool, mut items: Vec<Item>) -> Vec<Item> {
    if !skipped
        && spec
            .pointer("/settings/mode")
            .and_then(Value::as_str)
            .unwrap_or("active")
            == "active"
        && spec
            .pointer("/settings/always_output")
            .and_then(Value::as_bool)
            == Some(true)
        && items.is_empty()
    {
        items.push(Item {
            json: Map::new(),
            binary: Default::default(),
            paired: vec![],
        });
    }
    items
}

fn set(items: &[Item], params: &[Value]) -> Result<Vec<Item>> {
    if items.len() != params.len() {
        return Err(error("workflow.executionDataUnavailable", "/params"));
    }
    items
        .iter()
        .zip(params)
        .map(|(item, params)| {
            let mut result = item.clone();
            if params["keep_other_fields"] != true {
                result.json.clear();
            }
            let fields = params["fields"].as_array().ok_or_else(ApiError::missing)?;
            let mut paths = Vec::<Vec<&str>>::new();
            for field in fields {
                let path = field["path"].as_str().ok_or_else(ApiError::missing)?;
                let parts = super::transform::path(path)?;
                if paths
                    .iter()
                    .any(|prior| prior.starts_with(&parts) || parts.starts_with(prior))
                {
                    return Err(error("workflow.graphSchema", "/params/fields"));
                }
                paths.push(parts.clone());
                super::transform::write(&mut result.json, &parts, field["value"].clone())?;
            }
            Ok(result)
        })
        .collect()
}

pub(crate) async fn scheduler(
    s: AppState,
    mut stop: tokio::sync::watch::Receiver<bool>,
    mut drain: tokio::sync::watch::Receiver<bool>,
) {
    let boot = format!("{}:{}", s.instance_id, uuid::Uuid::new_v4());
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        while tasks.try_join_next().is_some() {}
        if *stop.borrow() || *drain.borrow() {
            break;
        }
        if tasks.len() < 16 {
            match claim(&s, &boot).await {
                Ok(Some(lease)) => {
                    let s = s.clone();
                    let stop = stop.clone();
                    tasks.spawn(async move {
                        let database = s.db.clone();
                        let token = lease.clone();
                        if let Err(error) = executions::supervise(lease, &database, stop, |stop| async move {
                            let backend=ArtifactStore::for_state(&s).await?;
                            if let Err(error) = dispatch(&s, &backend, &token, &stop).await {
                                tracing::warn!(execution=token.execution().id,%error,"workflow execution stopped");
                                if !*stop.borrow() {
                                    finish(&s.db, &token, "failed", failure_reason(&error)).await?;
                                }
                            }
                            Ok(())
                        }).await {
                            tracing::warn!(%error, "workflow supervision stopped");
                        }
                    });
                    continue;
                }
                Ok(None) => {}
                Err(error) => tracing::error!(%error,"workflow claim failed"),
            }
        }
        tokio::select! {_ = tokio::time::sleep(Duration::from_millis(100))=>{},_ = stop.changed()=>{},_ = drain.changed()=>{}}
    }
    while tasks.join_next().await.is_some() {}
}
async fn claim(s: &AppState, boot: &str) -> Result<Option<Lease>> {
    let mut candidates = runs::Entity::find()
        .filter(runs::Column::State.is_in(["queued", "running", "waiting", "cancelling"]))
        .order_by_desc(runs::Column::Priority)
        .order_by_asc(runs::Column::CreatedAt)
        .order_by_asc(runs::Column::Id)
        .paginate(&s.db, 500);
    // A page full of leased or future waiting executions must not hide runnable
    // executions behind it. Lease::claim_in remains the authoritative CAS.
    while let Some(page) = candidates.fetch_and_next().await? {
        for candidate in page {
            let tx = crate::variables::transaction(s, candidate.workspace_id.into()).await?;
            if let Some(lease) = Lease::claim_in(
                &tx,
                Execution {
                    kind: Kind::Workflow,
                    workspace: candidate.workspace_id,
                    id: candidate.id,
                },
                candidate.lease_epoch,
                boot,
                db::now(),
            )
            .await?
            {
                tx.commit().await?;
                return Ok(Some(lease));
            }
            tx.rollback().await?;
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests;
