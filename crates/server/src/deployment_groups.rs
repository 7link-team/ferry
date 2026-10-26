//! Ordered host-group snapshots and per-host outcomes; mode execution is shared.
use super::*;
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
        .ok_or_else(|| ApiError::bad("目标主机组不存在。"))?;
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
fn plans(artifact: &str) -> Result<Vec<(Id, String)>> {
    let doc: Value =
        serde_json::from_str(artifact).map_err(|_| ApiError::bad("主机组产物无效。"))?;
    let rows = doc["targets"]
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| ApiError::bad("主机组没有固定目标，请重新预览。"))?;
    let mut seen = BTreeSet::new();
    rows.iter()
        .map(|row| {
            let id = Id::reference(row["host_id"].as_str().unwrap_or(""))?;
            if !seen.insert(id.to_string()) {
                return Err(ApiError::bad("主机组目标重复。"));
            }
            let artifact = row["artifact"]
                .as_str()
                .ok_or_else(|| ApiError::bad("目标产物缺失。"))?
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
        let ids = members(&s.db, scope.workspace_id, app.target_ref).await?;
        if ids.is_empty() {
            return Err(ApiError::bad("主机组没有直接成员。"));
        }
        let (metadata, _, _) = bindings(&s.db, app, &app.manifest).await?;
        let mut rows = Vec::new();
        let mut encoded_metadata = json!([]);
        for id in &ids {
            let channel = remote::open(s, &scope.workspace_id, id).await?;
            let artifact = prepare_artifact(s, scope, channel.as_ref(), app, &metadata).await?;
            let doc: Value =
                serde_json::from_str(&artifact).map_err(|_| ApiError::bad("目标产物无效。"))?;
            encoded_metadata = doc.get("x-variable-bindings").cloned().unwrap_or(json!([]));
            rows.push(json!({"host_id":id,"artifact":artifact}));
        }
        let desired = serde_json::to_string_pretty(
            &json!({"targets":rows,"x-variable-bindings":encoded_metadata}),
        )
        .map_err(|_| ApiError::bad("主机组产物无效。"))?;
        let tx = crate::variables::transaction(s, scope.workspace_id).await?;
        if members(&tx, scope.workspace_id, app.target_ref).await? != ids
            || bindings(&tx, app, &app.manifest).await?.0 != metadata
        {
            return Err(ApiError::conflict("主机组成员或变量已变化，请重新预览。"));
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
    let expected =
        hash(&serde_json::to_string(&hashes).map_err(|_| ApiError::bad("目标摘要无效。"))?);
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
        json!({"desired":desired,"current":serde_json::to_string_pretty(&json!({"targets":existing})).unwrap(),"expected_hash":expected,"target_hashes":hashes,"preview_token":preview_token(&desired,&expected,app.version,source),"version":app.version,"changed":changed,"variables":artifact_bindings(&desired)?,"notice":"目标按主机ID顺序逐台执行；遇到失败立即停止，已成功主机不自动回滚。回滚使用历史目标列表及加密保存的配置和变量，卷数据不回滚。"}),
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
        && members(conn, app.workspace_id.into(), app.target_ref).await?
            != plans.iter().map(|(id, _)| *id).collect::<Vec<_>>()
    {
        return Err(ApiError::conflict("主机组成员已变化，请重新预览。"));
    }
    if body.target_hashes.len() != plans.len()
        || hash(
            &serde_json::to_string(&body.target_hashes)
                .map_err(|_| ApiError::bad("目标摘要无效。"))?,
        ) != body.expected_hash
    {
        return Err(ApiError::bad("目标摘要与预览不一致。"));
    }
    for (position, (host, artifact)) in plans.into_iter().enumerate() {
        hosts::Entity::find_by_id(i64::from(host))
            .filter(hosts::Column::WorkspaceId.eq(app.workspace_id))
            .one(conn)
            .await?
            .ok_or_else(|| ApiError::bad("历史目标主机已不存在。"))?;
        let expected = body
            .target_hashes
            .get(&host.to_string())
            .filter(|h| h.len() == 64 && h.bytes().all(|c| c.is_ascii_hexdigit()))
            .ok_or_else(|| ApiError::bad("目标主机摘要缺失或无效。"))?;
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
    Ok(json!(rows.into_iter().map(|r|json!({"id":Id::from(r.id),"host_id":Id::from(r.host_id),"position":r.position,"state":r.state,"output":r.output,"started_at":r.started_at,"finished_at":r.finished_at,"artifact":r.artifact})).collect::<Vec<_>>()))
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
            Expr::value("执行结果未确认，请核实目标服务。"),
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
            Expr::value("前序目标失败或执行中断，未尝试此目标。"),
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
        return Err(ApiError::bad("主机组执行快照缺失。"));
    }
    let mut output = String::new();
    for row in rows {
        let scope =
            match Scope::for_actor(s, rev.workspace_id.into(), rev.triggered_by.into()).await {
                Ok(scope) => scope,
                Err(error) if error.0 == StatusCode::FORBIDDEN => {
                    return Ok((false, format!("{output}执行身份已失效，后续目标未执行。\n")));
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
            return Ok((false, format!("{output}执行权限已撤回，后续目标未执行。\n")));
        }
        targets::Entity::update_many()
            .col_expr(targets::Column::State, Expr::value("running"))
            .col_expr(targets::Column::StartedAt, Expr::value(db::now()))
            .filter(targets::Column::Id.eq(row.id))
            .exec(&s.db)
            .await?;
        let mut host_app = app.clone();
        host_app.target_ref = row.host_id;
        host_app.target_kind = "host".into();
        let mut host_revision = rev.clone();
        host_revision.artifact = row.artifact;
        host_revision.expected_hash = row.expected_hash;
        let result = execute_host(s, &host_revision, protected, &host_app).await;
        let (success, text) = match result {
            Ok(value) => value,
            Err(error) => {
                return Err(if protected {
                    ApiError::bad("主机组引用变量执行结果无法确认，请核实当前目标。")
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
        output.push_str(&format!(
            "主机 {}：{}\n",
            row.host_id,
            if success {
                "成功"
            } else {
                "失败；后续目标不再执行"
            }
        ));
        if !success {
            return Ok((false, output));
        }
    }
    Ok((true, output))
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
