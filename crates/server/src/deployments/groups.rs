//! Ordered multi-host snapshots (host groups and cluster blueprints) and per-host outcomes;
//! mode execution is shared.
use super::*;
use crate::blueprint::{binding, compile::HostPlan, template::Context};
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
pub(crate) fn compiled(app: &apps::Model) -> Result<Vec<HostPlan>> {
    let binding = binding::parse(app.blueprint_binding.as_deref().unwrap_or_default())?;
    crate::blueprint::compile::compile(&app.manifest, &binding, &app.name)
}
/// Validates a binding against this workspace's current hosts, then compiles every host's plan.
pub(super) async fn compile(
    conn: &impl ConnectionTrait,
    workspace: Id,
    manifest: &str,
    b: &binding::Binding,
    name: &str,
) -> Result<Vec<HostPlan>> {
    let bp = crate::blueprint::model::parse(manifest)?;
    let known = hosts::Entity::find()
        .filter(hosts::Column::WorkspaceId.eq(workspace))
        .all(conn)
        .await?
        .into_iter()
        .map(|h| {
            let id = Id::from(h.id);
            (id, binding::HostInfo { id, os: h.os })
        })
        .collect();
    binding::validate(&bp, b, &known)?;
    crate::blueprint::compile::compile(manifest, b, name)
}
/// Every host a new revision of `app` deploys to, in target order: host-group members share
/// the deployment manifest; a blueprint compiles one application per bound host.
pub(super) async fn targets(
    conn: &impl ConnectionTrait,
    workspace: Id,
    app: &apps::Model,
) -> Result<Vec<HostTarget>> {
    if app.target_kind == "blueprint" {
        let b = binding::parse(app.blueprint_binding.as_deref().unwrap_or_default())?;
        return Ok(compile(conn, workspace, &app.manifest, &b, &app.name)
            .await?
            .into_iter()
            .map(|plan| HostTarget {
                host_id: plan.host_id,
                app: host_app(app, plan.host_id, plan.manifest),
                contexts: Some(plan.contexts),
            })
            .collect());
    }
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
/// Each blueprint host's compiled application, as stored in a revision's `rendered`.
pub(super) fn rendered(targets: &[HostTarget]) -> Result<String> {
    serde_json::to_string_pretty(&json!({"targets": targets
        .iter()
        .map(|t| json!({"host_id": t.host_id, "manifest": t.app.manifest}))
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
        let hosts = targets(&s.db, scope.workspace_id, app).await?;
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
        let desired = serde_json::to_string_pretty(
            &json!({"targets":rows,"x-variable-bindings":merge(&encoded)}),
        )
        .map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("deployment.error.052"),
            )
        })?;
        let tx = crate::variables::transaction(s, scope.workspace_id).await?;
        let again = targets(&tx, scope.workspace_id, app).await?;
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
pub(super) async fn enqueue(
    s: &AppState,
    conn: &impl ConnectionTrait,
    app: &apps::Model,
    revision: Id,
    artifact: &str,
    body: &RunBody,
    source: i64,
) -> Result<()> {
    let plans = plans(artifact)?;
    if source == 0
        && targets(conn, app.workspace_id.into(), app)
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
    // A blueprint host runs its own compiled application, stored with the revision.
    let mut manifests = BTreeMap::new();
    if app.target_kind == "blueprint" {
        let doc: Value = serde_json::from_str(&rev.rendered).unwrap_or_default();
        for target in doc["targets"].as_array().into_iter().flatten() {
            if let (Some(host), Some(manifest)) =
                (target["host_id"].as_str(), target["manifest"].as_str())
            {
                manifests.insert(host.to_owned(), manifest.to_owned());
            }
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
        if app.target_kind == "blueprint" {
            let manifest = manifests
                .get(&row.host_id.to_string())
                .cloned()
                .ok_or_else(|| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("deployment.error.063"),
                    )
                })?;
            host_app.manifest = manifest.clone();
            host_revision.manifest = manifest;
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
