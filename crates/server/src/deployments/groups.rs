//! Ordered multi-host snapshots (host groups and cluster blueprints) and per-host outcomes;
//! mode execution is shared.
use super::*;
use crate::blueprint::{
    binding,
    compile::{HostPlan, RoleTags},
    swarm,
    template::Context,
};
use crate::entities::{
    deployment_revision_targets as targets, deployment_target_states as current, host_groups,
};
use sea_orm::ConnectionTrait;
use std::collections::BTreeSet;

pub(super) async fn members(
    conn: &impl ConnectionTrait,
    workspace: Id,
    group: i64,
) -> Result<Vec<Id>> {
    host_groups::Entity::find_by_id(group)
        .filter(host_groups::Column::WorkspaceId.eq(workspace))
        .one(conn)
        .await?
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.051"),
            )
        })?;
    Ok(hosts::Entity::find()
        .filter(hosts::Column::WorkspaceId.eq(workspace))
        .filter(hosts::Column::GroupId.eq(group))
        .order_by_asc(hosts::Column::Id)
        .all(conn)
        .await?
        .into_iter()
        .map(|h| h.id.into())
        .collect())
}
/// Deployments whose revisions are pinned per host in `deployment_revision_targets`.
pub(super) fn is_multi(kind: &str) -> bool {
    matches!(kind, "host-group" | "blueprint")
}
/// One host's share of a multi-host deployment.
pub(super) struct HostTarget {
    pub host_id: Id,
    /// The deployment as this host runs it: its own manifest, `target_kind` "host" and
    /// `target_ref` the host; directory, project and variable scope stay the deployment's.
    pub app: apps::Model,
    /// Instance contexts for config templates; blueprints only.
    pub contexts: Option<BTreeMap<String, Context>>,
}
fn host_app(app: &apps::Model, host: Id, manifest: String) -> apps::Model {
    let mut out = app.clone();
    out.manifest = manifest;
    out.target_kind = "host".into();
    out.target_ref = host.into();
    out
}
/// Compiles a blueprint deployment without consulting hosts: the binding was validated when
/// it was stored, so readers that only need the compiled services never fail on a host change.
/// A stack is compiled unresolved (stand-in Swarm nodes), which is enough for readers. A
/// stored `deploy_spec` is parsed for its selector tags (label constraints); it was validated
/// when written, so this does not fail on a host change either.
pub(crate) fn compiled(app: &apps::Model) -> Result<Vec<HostPlan>> {
    let binding = binding::parse(app.blueprint_binding.as_deref().unwrap_or_default())?;
    let tags = role_tags(app.workspace_id.into(), app.deploy_spec.as_deref())?;
    crate::blueprint::compile::compile_unresolved_tagged(&app.manifest, &binding, &tags)
}
/// The host tags each role of `workspace`'s deployment spec selects by; none without a spec.
pub(super) fn role_tags(workspace: Id, spec: Option<&str>) -> Result<RoleTags> {
    let roles = match spec {
        Some(text) => crate::blueprint::spec::parse(text)?
            .roles
            .into_iter()
            .map(|(role, r)| (role, r.hosts.tags))
            .filter(|(_, tags)| !tags.is_empty())
            .collect(),
        None => BTreeMap::new(),
    };
    Ok(RoleTags { workspace, roles })
}
/// Validates a binding against this workspace's current hosts and returns them by id.
async fn known_hosts(
    conn: &impl ConnectionTrait,
    workspace: Id,
    bp: &crate::blueprint::model::Blueprint,
    b: &binding::Binding,
) -> Result<BTreeMap<Id, hosts::Model>> {
    let rows: BTreeMap<Id, hosts::Model> = hosts::Entity::find()
        .filter(hosts::Column::WorkspaceId.eq(workspace))
        .all(conn)
        .await?
        .into_iter()
        .map(|h| (Id::from(h.id), h))
        .collect();
    let known = rows
        .iter()
        .map(|(id, h)| {
            (
                *id,
                binding::HostInfo {
                    id: *id,
                    os: h.os.clone(),
                },
            )
        })
        .collect();
    binding::validate(bp, b, &known)?;
    Ok(rows)
}
/// Validates a binding against this workspace's current hosts, then compiles every host's
/// plan without reaching any host: a stack is compiled unresolved, its Swarm node checks
/// deferred to [`targets`] (preview and apply), since hosts may be offline when a deployment
/// is created or edited.
pub(super) async fn compile_offline(
    conn: &impl ConnectionTrait,
    workspace: Id,
    manifest: &str,
    b: &binding::Binding,
    tags: &RoleTags,
) -> Result<Vec<HostPlan>> {
    let bp = crate::blueprint::model::parse(manifest)?;
    known_hosts(conn, workspace, &bp, b).await?;
    crate::blueprint::compile::compile_unresolved_tagged(manifest, b, tags)
}
/// A stack blueprint's Swarm node per instance, looked up through its hosts for one
/// manifest and binding. Looked up before any transaction opens: reaching hosts can take
/// long, and opening a channel reads the database.
pub(super) struct SwarmNodes {
    manifest: String,
    binding: Option<String>,
    nodes: BTreeMap<String, String>,
    /// The manager host, and each bound host's own Swarm node ID.
    manager: Id,
    manager_name: String,
    hosts: BTreeMap<Id, String>,
}
/// Where a preview records the Swarm nodes its stack artifact is pinned to.
const PREVIEW_NODES: &str = "x-swarm-nodes";
impl SwarmNodes {
    /// Whether `preview` (a stored preview artifact) was pinned to these same nodes; a node
    /// that left and rejoined, or a changed assignment, means the preview is stale.
    pub(super) fn match_preview(&self, preview: &str) -> bool {
        serde_json::from_str::<Value>(preview)
            .is_ok_and(|doc| doc.get(PREVIEW_NODES) == Some(&json!(self.nodes)))
    }
}
/// Looks up a stack blueprint deployment's Swarm nodes: the manager lists the Swarm's nodes
/// and each distinct bound host reports its own node ID, each host asked once. `None` for
/// any other deployment.
pub(super) async fn swarm_nodes(
    s: &AppState,
    workspace: Id,
    app: &apps::Model,
) -> Result<Option<SwarmNodes>> {
    if app.target_kind != "blueprint" || app.mode != "stack" {
        return Ok(None);
    }
    let b = binding::parse(app.blueprint_binding.as_deref().unwrap_or_default())?;
    let bp = crate::blueprint::model::parse(&app.manifest)?;
    let rows = known_hosts(&s.db, workspace, &bp, &b).await?;
    let names: BTreeMap<Id, String> = rows.iter().map(|(id, h)| (*id, h.name.clone())).collect();
    // `validate` checked that a stack names its manager and every host exists here.
    let Some(manager) = b.manager.filter(|id| names.contains_key(id)) else {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("blueprint.bindingInvalid"),
        ));
    };
    let nodes = probe(s, workspace, manager, &names[&manager], true)
        .await?
        .nodes;
    let mut host_nodes = BTreeMap::new();
    for host in b.roles.values().flatten().map(|inst| inst.host_id) {
        if let std::collections::btree_map::Entry::Vacant(entry) = host_nodes.entry(host) {
            entry.insert(probe(s, workspace, host, &names[&host], false).await?.node);
        }
    }
    Ok(Some(SwarmNodes {
        manifest: app.manifest.clone(),
        binding: app.blueprint_binding.clone(),
        nodes: swarm::assign(&bp, &b, &nodes, &host_nodes, &names)?,
        manager,
        manager_name: names[&manager].clone(),
        hosts: host_nodes,
    }))
}
/// Before a stack deployment from a spec is applied, labels the Swarm nodes of its bound
/// hosts (the [`swarm_nodes`] just looked up) with the tags its role selectors use, so the
/// compiled `node.labels.ferry.w<ws>.tag.*` constraints match the hosts' current tags: a node gets
/// a tag's label when its host has the tag and loses it otherwise. Other labels, other tags'
/// labels (another workspace's included: labels are workspace-scoped) and nodes of hosts
/// this deployment does not bind are never touched. Changes the
/// cluster, so only apply calls it, outside any transaction (it reaches the manager).
pub(super) async fn sync_labels(
    s: &AppState,
    workspace: Id,
    app: &apps::Model,
    nodes: &SwarmNodes,
) -> Result<()> {
    let used: BTreeSet<String> = role_tags(workspace, app.deploy_spec.as_deref())?
        .roles
        .into_values()
        .flatten()
        .collect();
    if used.is_empty() {
        return Ok(());
    }
    let host_tags: BTreeMap<Id, Vec<String>> = super::candidates::load(&s.db, workspace)
        .await?
        .into_iter()
        .map(|c| (c.id, c.tags))
        .collect();
    let mut desired: BTreeMap<String, BTreeMap<String, bool>> = BTreeMap::new();
    for (host, node) in &nodes.hosts {
        let own = host_tags.get(host).map(Vec::as_slice).unwrap_or_default();
        let labels = desired.entry(node.clone()).or_default();
        for tag in &used {
            // Two Ferry hosts on one node: the node has the tag if either host does.
            *labels.entry(swarm::tag_label(workspace, tag)).or_default() |= own.contains(tag);
        }
    }
    let result = async {
        let channel = remote::open(s, &workspace, &nodes.manager).await?;
        swarm::sync_labels(channel.as_ref(), &desired).await
    }
    .await;
    result.map_err(|e| swarm::on_host(e, &nodes.manager_name))
}
/// What one Swarm probe found: the manager's node list, or one host's own node ID.
#[derive(Default)]
struct Probe {
    nodes: Vec<swarm::Node>,
    node: String,
}
/// Asks `host` for the Swarm's node list (`manager`) or its own node ID. The probe's own
/// Swarm findings (not a manager, not joined, unreadable node list) name the host; errors
/// reaching it (unreachable, credentials, offline runner, missing host) pass through
/// unchanged with their own messages.
async fn probe(s: &AppState, workspace: Id, host: Id, name: &str, manager: bool) -> Result<Probe> {
    let result: Result<Probe> = async {
        let channel = remote::open(s, &workspace, &host).await?;
        Ok(if manager {
            Probe {
                nodes: swarm::manager_nodes(channel.as_ref()).await?,
                ..Probe::default()
            }
        } else {
            Probe {
                node: swarm::node_id(channel.as_ref()).await?,
                ..Probe::default()
            }
        })
    }
    .await;
    result.map_err(|e| match e.2.as_ref() {
        Some(m) if m.key.starts_with("blueprint.") => {
            ApiError::localized(e.0, m.clone().with("host", name))
        }
        _ => e,
    })
}
fn blueprint_targets(app: &apps::Model, plans: Vec<HostPlan>) -> Vec<HostTarget> {
    plans
        .into_iter()
        .map(|plan| HostTarget {
            host_id: plan.host_id,
            app: host_app(app, plan.host_id, plan.manifest),
            contexts: Some(plan.contexts),
        })
        .collect()
}
/// Every host a new revision of `app` deploys to, in target order: host-group members share
/// the deployment manifest; a blueprint compiles one application per bound host, a stack one
/// on its manager with stable instances pinned to the `nodes` [`swarm_nodes`] looked up for
/// this very manifest and binding (otherwise the record changed since: preview again).
pub(super) async fn targets(
    conn: &impl ConnectionTrait,
    workspace: Id,
    app: &apps::Model,
    nodes: Option<&SwarmNodes>,
) -> Result<Vec<HostTarget>> {
    if app.target_kind == "blueprint" {
        let b = binding::parse(app.blueprint_binding.as_deref().unwrap_or_default())?;
        let bp = crate::blueprint::model::parse(&app.manifest)?;
        known_hosts(conn, workspace, &bp, &b).await?;
        let nodes = match nodes {
            Some(n) if n.manifest == app.manifest && n.binding == app.blueprint_binding => {
                Some(&n.nodes)
            }
            Some(_) => {
                return Err(ApiError::localized(
                    axum::http::StatusCode::CONFLICT,
                    crate::i18n::Message::new("deployment.error.030"),
                ));
            }
            None => None,
        };
        let tags = role_tags(workspace, app.deploy_spec.as_deref())?;
        let plans = crate::blueprint::compile::compile_tagged(&app.manifest, &b, nodes, &tags)?;
        return Ok(blueprint_targets(app, plans));
    }
    member_targets(conn, workspace, app).await
}
/// [`targets`] without reaching any host, for checks made while a deployment is stored: a
/// stack's single target is compiled unresolved (see [`compile_offline`]).
pub(super) async fn offline_targets(
    conn: &impl ConnectionTrait,
    workspace: Id,
    app: &apps::Model,
) -> Result<Vec<HostTarget>> {
    if app.target_kind == "blueprint" {
        let b = binding::parse(app.blueprint_binding.as_deref().unwrap_or_default())?;
        let tags = role_tags(workspace, app.deploy_spec.as_deref())?;
        let plans = compile_offline(conn, workspace, &app.manifest, &b, &tags).await?;
        return Ok(blueprint_targets(app, plans));
    }
    member_targets(conn, workspace, app).await
}
async fn member_targets(
    conn: &impl ConnectionTrait,
    workspace: Id,
    app: &apps::Model,
) -> Result<Vec<HostTarget>> {
    Ok(members(conn, workspace, app.target_ref)
        .await?
        .into_iter()
        .map(|id| HostTarget {
            host_id: id,
            app: host_app(app, id, app.manifest.clone()),
            contexts: None,
        })
        .collect())
}
/// Every host's public bindings in host order; an entry an earlier host already lists is
/// not repeated, so identical host-group members collapse to one host's list.
pub(super) fn merge<'a>(lists: impl IntoIterator<Item = &'a Value>) -> Value {
    let mut out: Vec<Value> = Vec::new();
    for list in lists {
        let before = out.len();
        for item in list.as_array().into_iter().flatten() {
            if !out[..before].contains(item) {
                out.push(item.clone());
            }
        }
    }
    json!(out)
}
/// Each blueprint host's compiled application and the snapshot file names its own artifact
/// references, as stored in a revision's `rendered`.
pub(super) fn rendered(targets: &[HostTarget], files: &[Vec<String>]) -> Result<String> {
    serde_json::to_string_pretty(&json!({"targets": targets
        .iter()
        .zip(files)
        .map(|(t, files)| json!({"host_id": t.host_id, "manifest": t.app.manifest, "files": files}))
        .collect::<Vec<_>>()}))
    .map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.006"),
        )
    })
}
fn plans(artifact: &str) -> Result<Vec<(Id, String)>> {
    let doc: Value = serde_json::from_str(artifact).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.052"),
        )
    })?;
    let rows = doc["targets"]
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.053"),
            )
        })?;
    let mut seen = BTreeSet::new();
    rows.iter()
        .map(|row| {
            let id = Id::reference(row["host_id"].as_str().unwrap_or(""))?;
            if !seen.insert(id.to_string()) {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("deployment.error.054"),
                ));
            }
            let artifact = row["artifact"]
                .as_str()
                .ok_or_else(|| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("deployment.error.055"),
                    )
                })?
                .to_owned();
            Ok((id, artifact))
        })
        .collect()
}
pub(super) async fn preview(
    s: &AppState,
    scope: &Scope,
    app: &apps::Model,
    b: PreviewBody,
) -> Result<Json<Value>> {
    let (desired, source) = if let Some(id) = b.revision_id {
        let old = old_revision(&s.db, scope, app.id, &id).await?;
        (old.artifact, old.id)
    } else {
        let nodes = swarm_nodes(s, scope.workspace_id, app).await?;
        let hosts = targets(&s.db, scope.workspace_id, app, nodes.as_ref()).await?;
        if hosts.is_empty() {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.056"),
            ));
        }
        let mut rows = Vec::new();
        let mut encoded = Vec::new();
        let mut metadata = Vec::new();
        for target in &hosts {
            let (bound, _, _) = bindings(
                &s.db,
                &target.app,
                &target.app.manifest,
                target.contexts.as_ref(),
            )
            .await?;
            let channel = remote::open(s, &scope.workspace_id, &target.host_id).await?;
            let artifact =
                prepare_artifact(s, scope, channel.as_ref(), &target.app, &bound).await?;
            let doc: Value = serde_json::from_str(&artifact).map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("deployment.error.057"),
                )
            })?;
            encoded.push(doc.get("x-variable-bindings").cloned().unwrap_or(json!([])));
            rows.push(json!({"host_id":target.host_id,"artifact":artifact}));
            metadata.push(bound);
        }
        let mut doc = json!({"targets":rows,"x-variable-bindings":merge(&encoded)});
        if let Some(nodes) = &nodes {
            doc[PREVIEW_NODES] = json!(nodes.nodes);
        }
        let desired = serde_json::to_string_pretty(&doc).map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.052"),
            )
        })?;
        let tx = crate::variables::transaction(s, scope.workspace_id).await?;
        let again = targets(&tx, scope.workspace_id, app, nodes.as_ref()).await?;
        let mut changed = again.len() != hosts.len();
        for (target, (before, bound)) in again.iter().zip(hosts.iter().zip(&metadata)) {
            changed |= target.host_id != before.host_id
                || target.app.manifest != before.app.manifest
                || &bindings(
                    &tx,
                    &target.app,
                    &target.app.manifest,
                    target.contexts.as_ref(),
                )
                .await?
                .0 != bound;
        }
        if changed {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("deployment.error.058"),
            ));
        }
        save_preview(&tx, scope, app, &desired).await?;
        tx.commit().await?;
        (desired, 0)
    };
    let mut hashes = BTreeMap::new();
    let mut existing = Vec::new();
    let mut changed = false;
    for (id, artifact) in plans(&desired)? {
        let channel = remote::open(s, &scope.workspace_id, &id).await?;
        let value = remote_artifact(channel.as_ref(), app).await?;
        changed |= value != artifact;
        hashes.insert(id.to_string(), hash(&value));
        existing.push(json!({"host_id":id,"artifact":value}));
    }
    let expected = hash(&serde_json::to_string(&hashes).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.035"),
        )
    })?);
    crate::hosts::activity(
        s,
        scope,
        "preview",
        "deployments",
        &app.id.into(),
        json!({"targets":hashes.len()}),
    )
    .await?;
    Ok(Json(
        json!({"desired":desired,"current":serde_json::to_string_pretty(&json!({"targets":existing})).unwrap(),"expected_hash":expected,"target_hashes":hashes,"preview_token":preview_token(&desired,&expected,app.version,source),"version":app.version,"changed":changed,"variables":artifact_bindings(&desired)?,"notice":crate::i18n::message("deployment.notice.group")}),
    ))
}
#[allow(clippy::too_many_arguments)]
pub(super) async fn enqueue(
    s: &AppState,
    conn: &impl ConnectionTrait,
    app: &apps::Model,
    revision: Id,
    artifact: &str,
    body: &RunBody,
    source: i64,
    nodes: Option<&SwarmNodes>,
) -> Result<()> {
    let plans = plans(artifact)?;
    if source == 0
        && targets(conn, app.workspace_id.into(), app, nodes)
            .await?
            .iter()
            .map(|t| t.host_id)
            .collect::<Vec<_>>()
            != plans.iter().map(|(id, _)| *id).collect::<Vec<_>>()
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("deployment.error.059"),
        ));
    }
    if body.target_hashes.len() != plans.len()
        || hash(&serde_json::to_string(&body.target_hashes).map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.035"),
            )
        })?) != body.expected_hash
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.060"),
        ));
    }
    for (position, (host, artifact)) in plans.into_iter().enumerate() {
        hosts::Entity::find_by_id(i64::from(host))
            .filter(hosts::Column::WorkspaceId.eq(app.workspace_id))
            .one(conn)
            .await?
            .ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("deployment.error.061"),
                )
            })?;
        let expected = body
            .target_hashes
            .get(&host.to_string())
            .filter(|h| h.len() == 64 && h.bytes().all(|c| c.is_ascii_hexdigit()))
            .ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("deployment.error.062"),
                )
            })?;
        targets::Entity::insert(
            targets::Model {
                id: s.next_id()?.into(),
                workspace_id: app.workspace_id,
                revision_id: revision.into(),
                host_id: host.into(),
                position: position as i64,
                artifact,
                expected_hash: expected.clone(),
                state: "pending".into(),
                output: String::new(),
                started_at: 0,
                finished_at: 0,
            }
            .into_active_model(),
        )
        .exec(conn)
        .await?;
    }
    Ok(())
}
pub(super) async fn details(
    conn: &impl ConnectionTrait,
    workspace: Id,
    revision: i64,
) -> Result<Value> {
    let rows = targets::Entity::find()
        .filter(targets::Column::WorkspaceId.eq(workspace))
        .filter(targets::Column::RevisionId.eq(revision))
        .order_by_asc(targets::Column::Position)
        .all(conn)
        .await?;
    Ok(json!(rows.into_iter().map(|r|json!({"id":Id::from(r.id),"host_id":Id::from(r.host_id),"position":r.position,"state":r.state,"output":crate::i18n::display_text_stored(&r.output),"started_at":r.started_at,"finished_at":r.finished_at,"artifact":r.artifact})).collect::<Vec<_>>()))
}
pub(super) async fn current(conn: &impl ConnectionTrait, workspace: Id, app: i64) -> Result<Value> {
    let rows = current::Entity::find()
        .filter(current::Column::WorkspaceId.eq(workspace))
        .filter(current::Column::DeploymentId.eq(app))
        .order_by_asc(current::Column::HostId)
        .all(conn)
        .await?;
    Ok(json!(rows.into_iter().map(|r|json!({"host_id":Id::from(r.host_id),"current_revision":Id::from(r.current_revision)})).collect::<Vec<_>>()))
}
/// Whether some host runs a revision of this deployment: a host's target state is recorded
/// only when its own step succeeded, so a first revision that failed on every host leaves the
/// topology undeployed. Uncertain targets keep the deployment in `needs-review`, which blocks
/// edits until an operator acknowledges it.
pub(super) async fn deployed(conn: &impl ConnectionTrait, app: &apps::Model) -> Result<bool> {
    if app.current_revision != 0 {
        return Ok(true);
    }
    let any = current::Entity::find()
        .filter(current::Column::WorkspaceId.eq(app.workspace_id))
        .filter(current::Column::DeploymentId.eq(app.id))
        .one(conn)
        .await?;
    Ok(any.is_some())
}
pub(super) async fn settle(conn: &impl ConnectionTrait, revision: i64) -> Result<()> {
    targets::Entity::update_many()
        .col_expr(targets::Column::State, Expr::value("needs-review"))
        .col_expr(targets::Column::FinishedAt, Expr::value(db::now()))
        .col_expr(
            targets::Column::Output,
            Expr::value(
                StoredText::from_message(Message::new("deployment.output.targetUncertain"))
                    .stored(),
            ),
        )
        .filter(targets::Column::RevisionId.eq(revision))
        .filter(targets::Column::State.eq("running"))
        .exec(conn)
        .await?;
    targets::Entity::update_many()
        .col_expr(targets::Column::State, Expr::value("skipped"))
        .col_expr(targets::Column::FinishedAt, Expr::value(db::now()))
        .col_expr(
            targets::Column::Output,
            Expr::value(
                StoredText::from_message(Message::new("deployment.output.skipped")).stored(),
            ),
        )
        .filter(targets::Column::RevisionId.eq(revision))
        .filter(targets::Column::State.eq("pending"))
        .exec(conn)
        .await?;
    Ok(())
}
pub(super) async fn execute(
    s: &AppState,
    app: &apps::Model,
    rev: &revisions::Model,
    protected: bool,
) -> Result<(bool, String)> {
    let rows = targets::Entity::find()
        .filter(targets::Column::WorkspaceId.eq(rev.workspace_id))
        .filter(targets::Column::RevisionId.eq(rev.id))
        .order_by_asc(targets::Column::Position)
        .all(&s.db)
        .await?;
    if rows.is_empty() {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.error.063"),
        ));
    }
    // A blueprint host runs its own compiled application with only its own snapshot files.
    // Everything is checked before any target starts, so a damaged revision fails cleanly.
    let mut per_host = BTreeMap::new();
    if app.target_kind == "blueprint" {
        let all = open_snapshot(&s.vault, rev)?;
        let Some(plans) = stored_plans(&rev.rendered, &all).filter(|plans| {
            rows.iter()
                .all(|row| plans.contains_key(&row.host_id.to_string()))
        }) else {
            let mut output = StoredText::new();
            output.push_message(Message::new("deployment.error.063"));
            return Ok((false, output.stored()));
        };
        for (host, (manifest, files)) in plans {
            let own: BTreeMap<String, String> = files
                .iter()
                .map(|name| (name.clone(), all[name].clone()))
                .collect();
            let sealed = seal_snapshot(&s.vault, rev.workspace_id, rev.id, &own)?;
            per_host.insert(host, (manifest, sealed));
        }
    }
    let mut output = StoredText::new();
    for row in rows {
        let scope =
            match Scope::for_actor(s, rev.workspace_id.into(), rev.triggered_by.into()).await {
                Ok(scope) => scope,
                Err(error) if error.0 == StatusCode::FORBIDDEN => {
                    output.push_message(Message::new("deployment.output.identityRevoked"));
                    return Ok((false, output.stored()));
                }
                Err(error) => return Err(error),
            };
        if scope
            .require(if rev.action == "rollback" {
                "deployments.rollback"
            } else {
                "deployments.apply"
            })
            .is_err()
        {
            output.push_message(Message::new("deployment.output.permissionRevoked"));
            return Ok((false, output.stored()));
        }
        targets::Entity::update_many()
            .col_expr(targets::Column::State, Expr::value("running"))
            .col_expr(targets::Column::StartedAt, Expr::value(db::now()))
            .filter(targets::Column::Id.eq(row.id))
            .exec(&s.db)
            .await?;
        let mut host_app = host_app(app, row.host_id.into(), app.manifest.clone());
        let mut host_revision = rev.clone();
        if let Some((manifest, sealed)) = per_host.get(&row.host_id.to_string()) {
            host_app.manifest = manifest.clone();
            host_revision.manifest = manifest.clone();
            host_revision.variable_snapshot = Some(sealed.clone());
        }
        host_revision.artifact = row.artifact;
        host_revision.expected_hash = row.expected_hash;
        let result = execute_host(s, &host_revision, protected, &host_app).await;
        let (success, text) = match result {
            Ok(value) => value,
            Err(error) => {
                return Err(if protected {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("deployment.error.064"),
                    )
                } else {
                    error
                });
            }
        };
        let tx = crate::variables::transaction(s, rev.workspace_id.into()).await?;
        targets::Entity::update_many()
            .col_expr(
                targets::Column::State,
                Expr::value(if success { "succeeded" } else { "failed" }),
            )
            .col_expr(targets::Column::Output, Expr::value(&text))
            .col_expr(targets::Column::FinishedAt, Expr::value(db::now()))
            .filter(targets::Column::Id.eq(row.id))
            .exec(&tx)
            .await?;
        if success {
            let existing = current::Entity::find()
                .filter(current::Column::WorkspaceId.eq(rev.workspace_id))
                .filter(current::Column::DeploymentId.eq(app.id))
                .filter(current::Column::HostId.eq(row.host_id))
                .one(&tx)
                .await?;
            if let Some(existing) = existing {
                current::Entity::update_many()
                    .col_expr(current::Column::CurrentRevision, Expr::value(rev.id))
                    .filter(current::Column::Id.eq(existing.id))
                    .exec(&tx)
                    .await?;
            } else {
                current::Entity::insert(
                    current::Model {
                        id: s.next_id()?.into(),
                        workspace_id: rev.workspace_id,
                        deployment_id: app.id,
                        host_id: row.host_id,
                        current_revision: rev.id,
                    }
                    .into_active_model(),
                )
                .exec(&tx)
                .await?;
            }
        }
        tx.commit().await?;
        output.push_message(
            Message::new(if success {
                "deployment.output.hostSuccess"
            } else {
                "deployment.output.hostFailure"
            })
            .with("host", row.host_id.to_string()),
        );
        if !success {
            return Ok((false, output.stored()));
        }
    }
    Ok((true, output.stored()))
}

/// Host id → (compiled application, own snapshot files) from a blueprint revision's
/// `rendered`; `None` when any entry is missing, malformed, or names a file the sealed
/// snapshot does not hold.
fn stored_plans(
    rendered: &str,
    snapshot: &BTreeMap<String, String>,
) -> Option<BTreeMap<String, (String, Vec<String>)>> {
    let doc: Value = serde_json::from_str(rendered).ok()?;
    let mut plans = BTreeMap::new();
    for target in doc["targets"].as_array()? {
        let host = target["host_id"].as_str()?;
        let manifest = target["manifest"].as_str()?;
        manifest::parse(manifest).ok()?;
        let files = target["files"]
            .as_array()?
            .iter()
            .map(|name| {
                name.as_str()
                    .filter(|name| snapshot.contains_key(*name))
                    .map(str::to_owned)
            })
            .collect::<Option<Vec<_>>>()?;
        if plans
            .insert(host.to_owned(), (manifest.to_owned(), files))
            .is_some()
        {
            return None;
        }
    }
    Some(plans)
}
pub(super) async fn remove(conn: &impl ConnectionTrait, workspace: Id, app: i64) -> Result<()> {
    targets::Entity::delete_many()
        .filter(targets::Column::WorkspaceId.eq(workspace))
        .filter(
            targets::Column::RevisionId.in_subquery(
                revisions::Entity::find()
                    .select_only()
                    .column(revisions::Column::Id)
                    .filter(revisions::Column::WorkspaceId.eq(workspace))
                    .filter(revisions::Column::DeploymentId.eq(app))
                    .into_query(),
            ),
        )
        .exec(conn)
        .await?;
    current::Entity::delete_many()
        .filter(current::Column::WorkspaceId.eq(workspace))
        .filter(current::Column::DeploymentId.eq(app))
        .exec(conn)
        .await?;
    Ok(())
}
