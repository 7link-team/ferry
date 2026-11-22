//! Durable node attempts. Computation happens outside these transactions; every
//! write locks and fences the parent execution before touching a node attempt.
use crate::{
    entities::{definition_revisions, workflow_executions as runs, workflow_node_runs as nodes},
    error::{ApiError, Result},
    executions::{self, lease::Lease},
    i18n::Message,
    vault::Vault,
};
use axum::http::StatusCode;
use ferry_protocol::sandbox::{InputData, Item};
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, DatabaseTransaction, EntityTrait, IntoActiveModel,
    PaginatorTrait, QueryFilter, QuerySelect, TransactionTrait, sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const INLINE_BYTES: usize = 64 * 1024;
#[cfg(test)]
const MAX_NODE_EXECUTIONS: u64 = 5000;
#[cfg(test)]
pub(super) const MAX_ITEM_BYTES: usize = 1024 * 1024;

#[cfg(test)]
pub(super) mod tests;

fn conflict() -> ApiError {
    ApiError::localized(
        StatusCode::CONFLICT,
        Message::new("execution.versionConflict"),
    )
}
/// Stored execution data broke an invariant (decode, identity or digest); logged
/// with an incident id and the caller's location, never shown as internal text.
#[track_caller]
fn fault() -> ApiError {
    crate::error::internal_fault("workflow execution journal")
}
fn too_large(key: &'static str, limit: usize) -> ApiError {
    ApiError::localized(
        StatusCode::PAYLOAD_TOO_LARGE,
        Message::new(key).with("limit", crate::content_assets::size_text(limit as i64)),
    )
}
fn invalid() -> ApiError {
    ApiError::localized(
        StatusCode::BAD_REQUEST,
        Message::new("workflow.graphSchema").with("path", "/execution"),
    )
}
fn canonical<T: Serialize>(value: &T) -> Result<String> {
    crate::definitions::canonical(&serde_json::to_string(value).map_err(|_| invalid())?)
}
fn digest<T: Serialize>(value: &T) -> Result<String> {
    Ok(hex::encode(Sha256::digest(canonical(value)?.as_bytes())))
}
fn seal<T: Serialize>(vault: &Vault, value: &T, aad: &str) -> Result<String> {
    let plain = canonical(value)?;
    // Larger packets need the asset store, never an oversized inline fallback.
    if plain.len() > INLINE_BYTES {
        return Err(fault());
    }
    vault.seal(&plain, aad)
}
fn open<T: serde::de::DeserializeOwned>(vault: &Vault, value: &str, aad: &str) -> Result<T> {
    serde_json::from_str(&vault.open_secret(value, aad)?).map_err(|_| fault())
}
fn node_condition(node: &nodes::Model) -> Condition {
    Condition::all()
        .add(nodes::Column::Id.eq(node.id))
        .add(nodes::Column::ExecutionId.eq(node.execution_id))
        .add(nodes::Column::NodeId.eq(&node.node_id))
        .add(nodes::Column::IterationHash.eq(&node.iteration_hash))
        .add(nodes::Column::Attempt.eq(node.attempt))
        .add(nodes::Column::State.eq(&node.state))
        .add(nodes::Column::Version.eq(node.version))
}
fn aad(run: &runs::Model, node: Option<i64>, field: &str) -> String {
    format!(
        "workflow:{}:{}:{}:{field}",
        run.workspace_id,
        run.id,
        node.map_or_else(|| "execution".into(), |id| format!("node:{id}"))
    )
}

fn execution_context(vault: &Vault, run: &runs::Model) -> Result<Value> {
    let value = match &run.context {
        None => json!({}),
        Some(value) => open(vault, value, &aad(run, None, "context"))?,
    };
    if !value.is_object() {
        return Err(fault());
    }
    Ok(value)
}

/// Only expose the saved public diagnostic, never the rest of the encrypted context.
pub(crate) fn execution_failure(vault: &Vault, run: &runs::Model) -> Result<Option<String>> {
    Ok(execution_context(vault, run)?["failure"]
        .as_str()
        .map(crate::i18n::display_stored))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Identity {
    pub workspace: i64,
    pub execution: i64,
    pub revision: i64,
    pub node: String,
    pub iteration_hash: String,
    pub attempt: i64,
    pub redo_round: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Source {
    pub node_run: i64,
    pub attempt: i64,
    pub iteration_hash: String,
    pub port: String,
    pub port_index: usize,
    pub output_sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Snapshot {
    pub version: u8,
    pub identity: Identity,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    /// Frozen from declared outputs, never inferred from an empty result packet.
    #[serde(default)]
    pub outputless: bool,
    pub params: Vec<Value>,
    pub frozen_at_ms: i64,
    pub now: Value,
    pub today: Value,
    pub sources: Vec<Source>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Port {
    pub name: String,
    pub index: usize,
    pub skipped: bool,
    pub items: Vec<Item>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Output {
    pub version: u8,
    pub ports: Vec<Port>,
}

impl Output {
    pub(super) fn validate(&self, outputless: bool) -> Result<()> {
        self.validate_with_policy(
            outputless,
            &ferry_protocol::resources::ResourcePolicy::default(),
        )
    }
    pub(super) fn validate_with_policy(
        &self,
        outputless: bool,
        policy: &ferry_protocol::resources::ResourcePolicy,
    ) -> Result<()> {
        let mut ports = BTreeSet::new();
        if self.version != 1 || self.ports.is_empty() != outputless {
            return Err(fault());
        }
        let mut bytes = 0usize;
        let mut count = 0usize;
        for port in &self.ports {
            count = count.checked_add(port.items.len()).ok_or_else(invalid)?;
            if !ferry_protocol::resources::allows(policy.workflow_items, count) {
                return Err(invalid());
            }
            if port.name.is_empty()
                || !ports.insert((&port.name, port.index))
                || (port.skipped && !port.items.is_empty())
                || !(InputData {
                    items: port.items.clone(),
                    upstream: vec![],
                })
                .valid()
            {
                return Err(invalid());
            }
            for item in &port.items {
                let size = serde_json::to_vec(item).map_err(|_| invalid())?.len();
                if !ferry_protocol::resources::allows(policy.workflow_packet_bytes, size) {
                    return Err(too_large(
                        "workflow.itemTooLarge",
                        ferry_protocol::resources::byte_limit(policy.workflow_packet_bytes),
                    ));
                }
                bytes = bytes.checked_add(size).ok_or_else(|| fault())?;
            }
        }
        if !ferry_protocol::resources::allows(policy.workflow_packet_bytes, bytes) {
            return Err(too_large(
                "workflow.outputTooLarge",
                ferry_protocol::resources::byte_limit(policy.workflow_packet_bytes),
            ));
        }
        Ok(())
    }
}

pub(crate) struct Activation {
    pub id: i64,
    pub node_path: String,
    pub iteration: Value,
    pub attempt: i64,
    pub redo_round: i64,
    pub params: Vec<Value>,
    pub frozen_at_ms: i64,
    pub timezone: Option<String>,
    pub sources: Vec<Source>,
    pub seed: Option<i64>,
}
pub(crate) struct Checkpoint {
    pub execution_version: i64,
    pub node: nodes::Model,
    pub snapshot: Snapshot,
}
pub(crate) struct Journal<'a> {
    pub db: &'a DatabaseConnection,
    pub vault: &'a Vault,
    pub lease: &'a Lease,
    pub assets: Option<&'a crate::content_assets::Store<'a>>,
    pub authority: Option<&'a crate::state::AppState>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetPacket {
    asset_packet: u8,
    id: i64,
    sha256: String,
}
struct Packet {
    value: String,
    asset: Option<i64>,
    bytes: i64,
}

/// Authenticated packets are opened before taking the execution write lock.
/// Keep their SQL identities so that preloading never authorizes stale data.
#[derive(Default)]
struct ReadSet {
    nodes: Vec<nodes::Model>,
    assets: Vec<(
        crate::entities::content_assets::Model,
        crate::entities::asset_refs::Model,
    )>,
}
impl ReadSet {
    async fn node(
        &mut self,
        journal: &Journal<'_>,
        run: &runs::Model,
        node: &nodes::Model,
        output: bool,
    ) -> Result<()> {
        use crate::entities::{asset_refs, content_assets};
        self.nodes.push(node.clone());
        for (field, value) in [
            ("params", node.resolved_params.as_deref()),
            ("output", output.then_some(node.output.as_str())),
        ] {
            let Some(value) = value.filter(|value| !value.is_empty()) else {
                continue;
            };
            let plain = journal
                .vault
                .open_secret(value, &aad(run, Some(node.id), field))?;
            let value: Value = serde_json::from_str(&plain).map_err(|_| fault())?;
            if value.get("asset_packet").is_none() {
                continue;
            }
            let locator: AssetPacket = serde_json::from_value(value).map_err(|_| fault())?;
            let asset = content_assets::Entity::find_by_id(locator.id)
                .filter(content_assets::Column::WorkspaceId.eq(run.workspace_id))
                .one(journal.db)
                .await?
                .ok_or_else(fault)?;
            let reference =
                asset_refs::Entity::find_by_id((locator.id, "node_run".to_owned(), node.id))
                    .one(journal.db)
                    .await?
                    .ok_or_else(fault)?;
            self.assets.push((asset, reference));
        }
        Ok(())
    }

    async fn validate(
        &self,
        tx: &DatabaseTransaction,
        run: &runs::Model,
        preview: &runs::Model,
    ) -> Result<()> {
        use crate::entities::{asset_gc_jobs, asset_refs, content_assets};
        if run.id != preview.id
            || run.workspace_id != preview.workspace_id
            || run.workflow_id != preview.workflow_id
            || run.revision_id != preview.revision_id
            || run.mode != preview.mode
        {
            return Err(conflict());
        }
        for node in &self.nodes {
            let current = nodes::Entity::find()
                .filter(node_condition(node))
                .lock_exclusive()
                .one(tx)
                .await?
                .ok_or_else(conflict)?;
            // Logs have an independent write path; they do not change packets or versions.
            if current.resolved_params != node.resolved_params
                || current.output != node.output
                || current.output_asset != node.output_asset
                || current.iteration_path != node.iteration_path
                || current.redo_round != node.redo_round
                || current.error_policy != node.error_policy
            {
                return Err(conflict());
            }
        }
        for (asset, reference) in &self.assets {
            if content_assets::Entity::find_by_id(asset.id)
                .lock_exclusive()
                .one(tx)
                .await?
                .as_ref()
                != Some(asset)
                || asset_refs::Entity::find_by_id((
                    reference.asset_id,
                    reference.owner_kind.clone(),
                    reference.owner_id,
                ))
                .lock_exclusive()
                .one(tx)
                .await?
                .as_ref()
                    != Some(reference)
                || asset_gc_jobs::Entity::find()
                    .filter(asset_gc_jobs::Column::WorkspaceId.eq(asset.workspace_id))
                    .filter(asset_gc_jobs::Column::AssetId.eq(asset.id))
                    .lock_exclusive()
                    .one(tx)
                    .await?
                    .is_some_and(|job| job.state == "deleting")
            {
                return Err(conflict());
            }
        }
        Ok(())
    }
}

impl Journal<'_> {
    /// Resolution can fail before a node attempt exists. Keep its public error
    /// under the same encryption and lease fence as the rest of the execution.
    pub(crate) async fn record_execution_failure(&self, message: &str) -> Result<()> {
        let current = runs::Entity::find_by_id(self.lease.execution().id)
            .one(self.db)
            .await?
            .ok_or_else(fault)?;
        let tx = self.db.begin().await?;
        let run = self.guard(&tx, current.version).await?;
        let mut context = self.context(&run)?;
        context["failure"] = json!(message);
        runs::Entity::update_many()
            .col_expr(
                runs::Column::Context,
                Expr::value(seal(self.vault, &context, &aad(&run, None, "context"))?),
            )
            .filter(runs::Column::Id.eq(run.id))
            .exec(&tx)
            .await?;
        self.commit(tx, &run).await?;
        Ok(())
    }
    async fn packet<T: Serialize>(
        &self,
        run: &runs::Model,
        node: i64,
        field: &str,
        value: &T,
    ) -> Result<Packet> {
        let plain = canonical(value)?;
        let bytes = i64::try_from(plain.len()).map_err(|_| fault())?;
        let (value, asset) = if plain.len() <= INLINE_BYTES {
            (self.vault.seal(&plain, &aad(run, Some(node), field))?, None)
        } else {
            let store = self.assets.ok_or_else(|| fault())?;
            let asset = store
                .stage(run.workspace_id, &plain, run.id, &node.to_string())
                .await?;
            (
                self.vault.seal(
                    &canonical(&AssetPacket {
                        asset_packet: 1,
                        id: asset.id,
                        sha256: asset.sha256,
                    })?,
                    &aad(run, Some(node), field),
                )?,
                Some(asset.id),
            )
        };
        Ok(Packet {
            value,
            asset,
            bytes,
        })
    }
    async fn open_packet<T: serde::de::DeserializeOwned>(
        &self,
        db: &impl sea_orm::ConnectionTrait,
        run: &runs::Model,
        node: i64,
        field: &str,
        value: &str,
        expected: Option<i64>,
    ) -> Result<T> {
        read_packet(
            self.vault,
            self.assets,
            db,
            run,
            node,
            field,
            value,
            expected,
        )
        .await
    }
    async fn retain(
        &self,
        tx: &DatabaseTransaction,
        run: &runs::Model,
        packet: &Packet,
    ) -> Result<()> {
        runs::Entity::update_many()
            .col_expr(
                runs::Column::DataBytes,
                Expr::value(
                    run.data_bytes
                        .checked_add(packet.bytes)
                        .ok_or_else(|| fault())?,
                ),
            )
            .filter(runs::Column::Id.eq(run.id))
            .exec(tx)
            .await?;
        Ok(())
    }
    async fn guard(&self, tx: &DatabaseTransaction, version: i64) -> Result<runs::Model> {
        if let Some(_state) = self.authority {
            let workspace = self.lease.execution().workspace.into();
            crate::accounts::workspace_admin::lock_workspace(tx, workspace).await?;
            let run = runs::Entity::find_by_id(self.lease.execution().id)
                .filter(runs::Column::WorkspaceId.eq(workspace))
                .one(tx)
                .await?
                .ok_or_else(|| fault())?;
            let scope = crate::accounts::workspace::Scope::for_actor_in(
                tx,
                workspace,
                run.triggered_by.into(),
            )
            .await?;
            let workflow = super::scoped(tx, workspace, run.workflow_id.into()).await?;
            crate::definitions::access::require(
                tx,
                &scope,
                &workflow,
                crate::definitions::access::Operation::Run,
            )
            .await?;
        }
        let next = version.checked_add(1).ok_or_else(conflict)?;
        if runs::Entity::update_many()
            .col_expr(runs::Column::Version, Expr::value(next))
            .filter(executions::workflow_condition(self.lease, version))
            .exec(tx)
            .await?
            .rows_affected
            != 1
        {
            return Err(executions::ownership_lost());
        }
        runs::Entity::find_by_id(self.lease.execution().id)
            .one(tx)
            .await?
            .ok_or_else(|| fault())
    }
    async fn commit(&self, tx: DatabaseTransaction, run: &runs::Model) -> Result<i64> {
        // Re-evaluate expiry at the final SQL write, after all encryption/I/O.
        let final_run = self.guard(&tx, run.version).await?;
        tx.commit().await?;
        Ok(final_run.version)
    }
    async fn snapshot(
        &self,
        db: &impl sea_orm::ConnectionTrait,
        run: &runs::Model,
        node: &nodes::Model,
    ) -> Result<Snapshot> {
        read_snapshot(self.vault, self.assets, db, run, node).await
    }
    pub(crate) async fn output(
        &self,
        db: &impl sea_orm::ConnectionTrait,
        run: &runs::Model,
        node: &nodes::Model,
    ) -> Result<Output> {
        if node.execution_id != run.id {
            return Err(fault());
        }
        let output: Output = self
            .open_packet(db, run, node.id, "output", &node.output, node.output_asset)
            .await?;
        let outputless = output.ports.is_empty() && self.snapshot(db, run, node).await?.outputless;
        output.validate(outputless)?;
        Ok(output)
    }
    async fn sources(
        &self,
        reads: &mut ReadSet,
        run: &runs::Model,
        sources: &[Source],
    ) -> Result<i64> {
        let mut count = 0i64;
        for source in sources {
            let row = nodes::Entity::find_by_id(source.node_run)
                .filter(nodes::Column::ExecutionId.eq(run.id))
                .one(self.db)
                .await?
                .ok_or_else(|| fault())?;
            if row.attempt != source.attempt
                || row.iteration_hash != source.iteration_hash
                || !matches!(
                    row.state.as_str(),
                    "succeeded"
                        | "skipped"
                        | "bypassed"
                        | "cached"
                        | "pinned"
                        | "reused"
                        | "failed"
                )
                || (row.state == "failed" && row.error_policy == "stop")
            {
                return Err(fault());
            }
            reads.node(self, run, &row, true).await?;
            self.snapshot(self.db, run, &row).await?;
            let output = self.output(self.db, run, &row).await?;
            let port = output
                .ports
                .iter()
                .find(|port| port.name == source.port && port.index == source.port_index)
                .ok_or_else(|| fault())?;
            if digest(port)? != source.output_sha256 {
                return Err(fault());
            }
            count = count
                .checked_add(port.items.len().try_into().map_err(|_| fault())?)
                .ok_or_else(|| fault())?;
        }
        Ok(count)
    }

    /// Duplicate activation returns the original parameters and clock. A new
    /// owner must not replace them with newly evaluated variables or timestamps.
    pub(crate) async fn activate(
        &self,
        version: i64,
        activation: Activation,
    ) -> Result<Checkpoint> {
        let policy = crate::accounts::resource_policy::load(self.db).await?;
        let preview_run = runs::Entity::find_by_id(self.lease.execution().id)
            .filter(runs::Column::WorkspaceId.eq(self.lease.execution().workspace))
            .one(self.db)
            .await?
            .ok_or_else(|| fault())?;
        let run = preview_run;
        let revision = definition_revisions::Entity::find_by_id(run.revision_id)
            .filter(definition_revisions::Column::WorkspaceId.eq(run.workspace_id))
            .filter(definition_revisions::Column::SubjectKind.eq("workflow"))
            .filter(definition_revisions::Column::SubjectId.eq(run.workflow_id))
            .one(self.db)
            .await?
            .ok_or_else(|| fault())?;
        let definition: Value = serde_json::from_str(&revision.definition).map_err(|_| fault())?;
        let spec = super::graph::node_at(&definition, &activation.node_path).ok_or_else(invalid)?;
        let node_id = spec["id"].as_str().ok_or_else(invalid)?;
        if activation.id <= 0
            || activation.attempt < 1
            || activation.redo_round < 0
            || !activation.iteration.is_array()
            || activation.iteration.as_array().unwrap().len() > 3
        {
            return Err(invalid());
        }
        let iteration_path = canonical(&activation.iteration)?;
        let iteration_hash = digest(&activation.iteration)?;
        let mut reads = ReadSet::default();
        if let Some(node) = nodes::Entity::find()
            .filter(nodes::Column::ExecutionId.eq(run.id))
            .filter(nodes::Column::NodeId.eq(node_id))
            .filter(nodes::Column::IterationHash.eq(&iteration_hash))
            .filter(nodes::Column::Attempt.eq(activation.attempt))
            .one(self.db)
            .await?
        {
            if node.iteration_path != iteration_path || node.redo_round != activation.redo_round {
                return Err(fault());
            }
            reads.node(self, &run, &node, false).await?;
            let snapshot = self.snapshot(self.db, &run, &node).await?;
            self.sources(&mut reads, &run, &snapshot.sources).await?;
            let tx = self.db.begin().await?;
            let guarded = self.guard(&tx, version).await?;
            reads.validate(&tx, &guarded, &run).await?;
            self.guard(&tx, guarded.version).await?;
            tx.rollback().await?;
            return Ok(Checkpoint {
                execution_version: version,
                node,
                snapshot,
            });
        }
        for params in &activation.params {
            super::graph::validate_resolved(spec, &activation.node_path, params)?;
        }
        let (now, today) = super::expression::clock_with_timezone(
            &definition["settings"],
            activation.timezone.as_deref(),
            activation.frozen_at_ms,
        )?;
        let snapshot = Snapshot {
            outputless: super::graph::manifest(spec)
                .and_then(|manifest| manifest["outputs"].as_array())
                .is_some_and(Vec::is_empty),
            version: 1,
            pinned: super::graph::pin_data(&definition, spec, &run.mode).is_some(),
            mode: spec
                .pointer("/settings/mode")
                .and_then(Value::as_str)
                .map(str::to_owned),
            identity: Identity {
                workspace: run.workspace_id,
                execution: run.id,
                revision: run.revision_id,
                node: node_id.into(),
                iteration_hash: iteration_hash.clone(),
                attempt: activation.attempt,
                redo_round: activation.redo_round,
            },
            params: activation.params,
            frozen_at_ms: activation.frozen_at_ms,
            now,
            today,
            sources: activation.sources,
        };
        let error_policy = spec
            .pointer("/settings/on_error")
            .and_then(Value::as_str)
            .unwrap_or("stop")
            .to_owned();
        let items_in = self.sources(&mut reads, &run, &snapshot.sources).await?;
        let packet = self
            .packet(&run, activation.id, "params", &snapshot)
            .await?;
        let prepared = match packet.asset {
            Some(id) => Some(
                self.assets
                    .ok_or_else(fault)?
                    .prepare(run.workspace_id, id)
                    .await?,
            ),
            None => None,
        };
        let tx = self.db.begin().await?;
        let preview_run = run;
        let run = self.guard(&tx, version).await?;
        reads.validate(&tx, &run, &preview_run).await?;
        if run.workspace_id != snapshot.identity.workspace
            || run.revision_id != snapshot.identity.revision
        {
            return Err(fault());
        }
        if nodes::Entity::find()
            .filter(nodes::Column::ExecutionId.eq(run.id))
            .filter(nodes::Column::NodeId.eq(node_id))
            .filter(nodes::Column::IterationHash.eq(&iteration_hash))
            .filter(nodes::Column::Attempt.eq(activation.attempt))
            .one(&tx)
            .await?
            .is_some()
        {
            return Err(conflict());
        }
        // guard holds the parent write lock: concurrent attempts share one
        // budget, while replaying a persisted attempt consumes nothing extra.
        if let Some(limit) = policy.workflow_node_executions {
            if nodes::Entity::find()
                .filter(nodes::Column::ExecutionId.eq(run.id))
                .count(&tx)
                .await?
                >= limit
            {
                return Err(ApiError::localized(
                    StatusCode::TOO_MANY_REQUESTS,
                    Message::new("workflow.nodeExecutionLimit").with("limit", limit.to_string()),
                ));
            }
        }
        if let Some(prepared) = &prepared {
            prepared
                .attach(
                    self.assets.ok_or_else(fault)?,
                    &tx,
                    "node_run",
                    activation.id,
                )
                .await?;
        }
        self.retain(&tx, &run, &packet).await?;
        let node = nodes::Model {
            id: activation.id,
            execution_id: run.id,
            node_id: node_id.into(),
            iteration_path,
            iteration_hash,
            attempt: activation.attempt,
            state: "running".into(),
            items_in,
            items_out: 0,
            output: String::new(),
            output_asset: None,
            logs: None,
            logs_truncated: false,
            logs_updated_at: 0,
            error: String::new(),
            cache_key: String::new(),
            cache_hit_from: None,
            cost_micros: 0,
            started_at: crate::db::now(),
            started_by: run.triggered_by,
            finished_at: 0,
            finished_by: 0,
            resolved_params: Some(packet.value),
            seed: activation.seed,
            redo_round: activation.redo_round,
            error_policy,
            version: 1,
        };
        nodes::Entity::insert(node.clone().into_active_model())
            .exec(&tx)
            .await?;
        let execution_version = self.commit(tx, &run).await?;
        Ok(Checkpoint {
            execution_version,
            node,
            snapshot,
        })
    }

    /// Only terminal output is visible. Parent and child versions commit together.
    pub(crate) async fn complete(
        &self,
        version: i64,
        node_id: i64,
        node_version: i64,
        output: Output,
    ) -> Result<Checkpoint> {
        // The authenticated snapshot below decides whether empty ports are valid.
        let policy = crate::accounts::resource_policy::load(self.db).await?;
        output.validate_with_policy(output.ports.is_empty(), &policy)?;
        let preview_run = runs::Entity::find_by_id(self.lease.execution().id)
            .filter(runs::Column::WorkspaceId.eq(self.lease.execution().workspace))
            .one(self.db)
            .await?
            .ok_or_else(|| fault())?;
        let node = nodes::Entity::find_by_id(node_id)
            .filter(nodes::Column::ExecutionId.eq(preview_run.id))
            .one(self.db)
            .await?
            .ok_or_else(fault)?;
        let duplicate = matches!(
            node.state.as_str(),
            "succeeded" | "skipped" | "bypassed" | "pinned"
        );
        let mut reads = ReadSet::default();
        reads.node(self, &preview_run, &node, duplicate).await?;
        let snapshot = self.snapshot(self.db, &preview_run, &node).await?;
        output.validate_with_policy(snapshot.outputless, &policy)?;
        if duplicate {
            if digest(&self.output(self.db, &preview_run, &node).await?)? != digest(&output)? {
                return Err(conflict());
            }
        } else {
            if node.state != "running" || node.version != node_version {
                return Err(conflict());
            }
            // Missing frozen source data never becomes an empty input.
            self.sources(&mut reads, &preview_run, &snapshot.sources)
                .await?;
        }
        let packet = self
            .packet(&preview_run, node_id, "output", &output)
            .await?;
        let mut ids: Vec<_> = packet.asset.into_iter().collect();
        for reference in output
            .ports
            .iter()
            .flat_map(|port| &port.items)
            .flat_map(|item| item.binary.values())
        {
            ids.push(
                self.assets
                    .ok_or_else(fault)?
                    .binary_id(preview_run.workspace_id, reference)
                    .await?,
            );
        }
        let prepared = if ids.is_empty() {
            Vec::new()
        } else {
            self.assets
                .ok_or_else(fault)?
                .prepare_ids(preview_run.workspace_id, ids)
                .await?
        };
        let tx = self.db.begin().await?;
        let run = self.guard(&tx, version).await?;
        reads.validate(&tx, &run, &preview_run).await?;
        if duplicate {
            self.guard(&tx, run.version).await?;
            tx.rollback().await?;
            return Ok(Checkpoint {
                execution_version: version,
                node,
                snapshot,
            });
        }
        self.retain(&tx, &run, &packet).await?;
        for prepared in &prepared {
            prepared
                .attach(self.assets.ok_or_else(fault)?, &tx, "node_run", node_id)
                .await?;
        }
        let items_out: usize = output.ports.iter().map(|port| port.items.len()).sum();
        let state = if output.ports.iter().all(|port| port.skipped) {
            "skipped"
        } else if snapshot.pinned {
            "pinned"
        } else if snapshot.mode.as_deref() == Some("bypass") {
            "bypassed"
        } else {
            "succeeded"
        };
        if nodes::Entity::update_many()
            .col_expr(nodes::Column::State, Expr::value(state))
            .col_expr(
                nodes::Column::ItemsOut,
                Expr::value(i64::try_from(items_out).map_err(|_| fault())?),
            )
            .col_expr(nodes::Column::Output, Expr::value(packet.value))
            .col_expr(nodes::Column::OutputAsset, Expr::value(packet.asset))
            .col_expr(nodes::Column::FinishedAt, Expr::value(crate::db::now()))
            .col_expr(nodes::Column::FinishedBy, Expr::value(run.triggered_by))
            .col_expr(
                nodes::Column::Version,
                Expr::value(node_version.checked_add(1).ok_or_else(conflict)?),
            )
            .filter(node_condition(&node))
            .exec(&tx)
            .await?
            .rows_affected
            != 1
        {
            return Err(conflict());
        }
        let node = nodes::Entity::find_by_id(node_id)
            .one(&tx)
            .await?
            .ok_or_else(|| fault())?;
        let execution_version = self.commit(tx, &run).await?;
        Ok(Checkpoint {
            execution_version,
            node,
            snapshot,
        })
    }

    fn context(&self, run: &runs::Model) -> Result<Value> {
        execution_context(self.vault, run)
    }

    /// Mark an individual timed wait; other concurrent nodes may still complete.
    pub(crate) async fn wait(
        &self,
        version: i64,
        node_id: i64,
        node_version: i64,
        wake_at: i64,
    ) -> Result<i64> {
        let preview_run = runs::Entity::find_by_id(self.lease.execution().id)
            .filter(runs::Column::WorkspaceId.eq(self.lease.execution().workspace))
            .one(self.db)
            .await?
            .ok_or_else(fault)?;
        let node = nodes::Entity::find_by_id(node_id)
            .filter(nodes::Column::ExecutionId.eq(preview_run.id))
            .one(self.db)
            .await?
            .ok_or_else(fault)?;
        let mut reads = ReadSet::default();
        reads.node(self, &preview_run, &node, false).await?;
        self.snapshot(self.db, &preview_run, &node).await?;
        let tx = self.db.begin().await?;
        let run = self.guard(&tx, version).await?;
        reads.validate(&tx, &run, &preview_run).await?;
        let mut context = self.context(&run)?;
        let key = node_id.to_string();
        if node.state == "waiting"
            && context
                .pointer(&format!("/journal/waits/{key}"))
                .and_then(Value::as_i64)
                == Some(wake_at)
        {
            tx.rollback().await?;
            return Ok(version);
        }
        if node.state != "running" || node.version != node_version {
            return Err(conflict());
        }
        if context.get("journal").is_none() {
            context["journal"] = json!({"version":1,"waits":{}});
        }
        if context["journal"]["version"] != 1 || !context["journal"]["waits"].is_object() {
            return Err(fault());
        }
        context["journal"]["waits"][&key] = json!(wake_at);
        if nodes::Entity::update_many()
            .col_expr(nodes::Column::State, Expr::value("waiting"))
            .col_expr(
                nodes::Column::Version,
                Expr::value(node_version.checked_add(1).ok_or_else(conflict)?),
            )
            .filter(node_condition(&node))
            .exec(&tx)
            .await?
            .rows_affected
            != 1
        {
            return Err(conflict());
        }
        runs::Entity::update_many()
            .col_expr(
                runs::Column::Context,
                Expr::value(seal(self.vault, &context, &aad(&run, None, "context"))?),
            )
            .filter(runs::Column::Id.eq(run.id))
            .exec(&tx)
            .await?;
        self.commit(tx, &run).await
    }

    /// The scheduler calls this only after draining running work. Timed waits
    /// release capacity; a future claim uses the existing not_before lease gate.
    pub(crate) async fn park(&self, version: i64) -> Result<i64> {
        let tx = self.db.begin().await?;
        let run = self.guard(&tx, version).await?;
        if nodes::Entity::find()
            .filter(nodes::Column::ExecutionId.eq(run.id))
            .filter(nodes::Column::State.eq("running"))
            .one(&tx)
            .await?
            .is_some()
        {
            return Err(conflict());
        }
        let mut context = self.context(&run)?;
        let waits = context
            .pointer("/journal/waits")
            .and_then(Value::as_object)
            .ok_or_else(|| fault())?;
        let wake_at = waits
            .values()
            .map(|value| value.as_i64().ok_or_else(|| fault()))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .min()
            .ok_or_else(conflict)?;
        context["journal"]["parked_at"] = json!(crate::db::now());
        let final_version = run.version.checked_add(1).ok_or_else(conflict)?;
        if runs::Entity::update_many()
            .col_expr(runs::Column::State, Expr::value("waiting"))
            .col_expr(runs::Column::Version, Expr::value(final_version))
            .col_expr(runs::Column::NotBefore, Expr::value(wake_at))
            .col_expr(runs::Column::EndReason, Expr::value("node_wait"))
            .col_expr(
                runs::Column::Context,
                Expr::value(seal(self.vault, &context, &aad(&run, None, "context"))?),
            )
            .filter(executions::workflow_condition(self.lease, run.version))
            .exec(&tx)
            .await?
            .rows_affected
            != 1
        {
            return Err(executions::ownership_lost());
        }
        tx.commit().await?;
        Ok(final_version)
    }

    /// A new lease resumes the same attempt and its original frozen parameters.
    /// `due` names waiting attempts that an outside event (a preview review)
    /// released before their deadline.
    pub(crate) async fn wake(&self, version: i64, due: &[i64]) -> Result<i64> {
        let preview_run = runs::Entity::find_by_id(self.lease.execution().id)
            .filter(runs::Column::WorkspaceId.eq(self.lease.execution().workspace))
            .one(self.db)
            .await?
            .ok_or_else(fault)?;
        let run = &preview_run;
        let mut reads = ReadSet::default();
        let mut context = self.context(&run)?;
        let now = crate::db::now();
        let waits = context
            .pointer_mut("/journal/waits")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| fault())?;
        let ready: Vec<_> = waits
            .iter()
            .filter_map(|(id, wake)| {
                wake.as_i64()
                    .filter(|wake| *wake <= now || due.iter().any(|node| node.to_string() == *id))
                    .map(|_| id.clone())
            })
            .collect();
        let mut ready_nodes = Vec::new();
        for id in &ready {
            let node_id = id.parse::<i64>().map_err(|_| fault())?;
            let node = nodes::Entity::find_by_id(node_id)
                .filter(nodes::Column::ExecutionId.eq(run.id))
                .one(self.db)
                .await?
                .ok_or_else(fault)?;
            if node.state != "waiting" {
                return Err(conflict());
            }
            reads.node(self, run, &node, false).await?;
            self.snapshot(self.db, run, &node).await?;
            ready_nodes.push(node);
        }
        let tx = self.db.begin().await?;
        let run = self.guard(&tx, version).await?;
        reads.validate(&tx, &run, &preview_run).await?;
        if run.context != preview_run.context {
            return Err(conflict());
        }
        for (id, node) in ready.iter().zip(&ready_nodes) {
            if nodes::Entity::update_many()
                .col_expr(nodes::Column::State, Expr::value("running"))
                .col_expr(
                    nodes::Column::Version,
                    Expr::value(node.version.checked_add(1).ok_or_else(conflict)?),
                )
                .filter(node_condition(&node))
                .exec(&tx)
                .await?
                .rows_affected
                != 1
            {
                return Err(conflict());
            }
            waits.remove(id);
        }
        if ready.is_empty() {
            tx.rollback().await?;
            return Ok(version);
        }
        let parked = context["journal"]
            .as_object_mut()
            .unwrap()
            .remove("parked_at")
            .and_then(|value| value.as_i64())
            .ok_or_else(|| fault())?;
        let waiting_ms = run
            .waiting_ms
            .checked_add(std::cmp::max(now.saturating_sub(parked), 0))
            .ok_or_else(|| fault())?;
        runs::Entity::update_many()
            .col_expr(runs::Column::WaitingMs, Expr::value(waiting_ms))
            .col_expr(runs::Column::NotBefore, Expr::value(Option::<i64>::None))
            .col_expr(runs::Column::EndReason, Expr::value(""))
            .col_expr(
                runs::Column::Context,
                Expr::value(seal(self.vault, &context, &aad(&run, None, "context"))?),
            )
            .filter(runs::Column::Id.eq(run.id))
            .exec(&tx)
            .await?;
        self.commit(tx, &run).await
    }
}

/// Read the same authenticated snapshot for execution recovery and editor inspection.
pub(crate) async fn read_snapshot(
    vault: &Vault,
    assets: Option<&crate::content_assets::Store<'_>>,
    db: &impl sea_orm::ConnectionTrait,
    run: &runs::Model,
    node: &nodes::Model,
) -> Result<Snapshot> {
    if node.execution_id != run.id {
        return Err(fault());
    }
    let snapshot: Snapshot = read_packet(
        vault,
        assets,
        db,
        run,
        node.id,
        "params",
        node.resolved_params.as_deref().ok_or_else(|| fault())?,
        None,
    )
    .await?;
    let identity = Identity {
        workspace: run.workspace_id,
        execution: run.id,
        revision: run.revision_id,
        node: node.node_id.clone(),
        iteration_hash: node.iteration_hash.clone(),
        attempt: node.attempt,
        redo_round: node.redo_round,
    };
    if snapshot.version != 1
        || snapshot.identity != identity
        || digest(&serde_json::from_str::<Value>(&node.iteration_path).map_err(|_| fault())?)?
            != node.iteration_hash
    {
        return Err(fault());
    }
    Ok(snapshot)
}

pub(crate) async fn read_packet<T: serde::de::DeserializeOwned>(
    vault: &Vault,
    assets: Option<&crate::content_assets::Store<'_>>,
    db: &impl sea_orm::ConnectionTrait,
    run: &runs::Model,
    node: i64,
    field: &str,
    value: &str,
    expected: Option<i64>,
) -> Result<T> {
    let plain = vault.open_secret(value, &aad(run, Some(node), field))?;
    let value: Value = serde_json::from_str(&plain).map_err(|_| fault())?;
    let plain = if value.get("asset_packet").is_some() {
        let locator: AssetPacket = serde_json::from_value(value).map_err(|_| fault())?;
        if locator.asset_packet != 1
            || (field == "output" && expected.is_none())
            || expected.is_some_and(|id| id != locator.id)
        {
            return Err(fault());
        }
        if crate::entities::asset_refs::Entity::find_by_id((
            locator.id,
            "node_run".to_owned(),
            node,
        ))
        .one(db)
        .await?
        .is_none()
        {
            return Err(fault());
        }
        let plain = assets
            .ok_or_else(|| fault())?
            .read(db, run.workspace_id, locator.id)
            .await?;
        if hex::encode(Sha256::digest(plain.as_bytes())) != locator.sha256 {
            return Err(fault());
        }
        plain
    } else {
        if expected.is_some() {
            return Err(fault());
        }
        plain
    };
    serde_json::from_str(&plain).map_err(|_| fault())
}
