//! Linear execution through HostChannel only. Cancellation stops waiting, not
//! necessarily the remote process; cleanup is awaited even after cancellation.
use crate::{
    channel::{HostChannel, Output},
    credentials, db,
    entities::{
        host_groups, hosts, pipeline_run_logs as logs, pipeline_run_steps as steps,
        pipeline_runs as runs, pipelines, repositories, workspaces,
    },
    error::{ApiError, Result},
    pipelines::Definition,
    resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
    workspace::Scope,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
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
struct VariableSnapshot {
    values: BTreeMap<String, String>,
    sources: Vec<Value>,
}
fn snapshot_aad(workspace: i64, run: i64) -> String {
    crate::vault::aad(
        &workspace.to_string(),
        "pipeline-run-variables",
        &run.to_string(),
    )
}
fn open_variables(s: &AppState, run: &runs::Model) -> Result<VariableSnapshot> {
    match &run.variable_snapshot {
        Some(sealed) => serde_json::from_str(
            &s.vault
                .open_secret(sealed, &snapshot_aad(run.workspace_id, run.id))?,
        )
        .map_err(|_| ApiError::bad("流水线变量快照无效。")),
        None => Ok(VariableSnapshot::default()),
    }
}

const SPEC: Spec = Spec {
    search: &[],
    filters: &["pipeline_id", "state"],
    sortable: &["number", "created_at", "state"],
    default_sort: "-number",
};
fn run_json(r: &runs::Model) -> Value {
    json!({"id":Id::from(r.id),"pipeline_id":Id::from(r.pipeline_id),"host_id":Id::from(r.host_id),
        "number":r.number,"state":r.state,"trigger_kind":r.trigger_kind,
        "triggered_by":Id::from(r.triggered_by),"branch":r.branch,"commit":r.commit,
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
    Ok(Json(
        json!({"items":rows.iter().map(run_json).collect::<Vec<_>>(),"total":total,
        "page":q.page,"page_size":q.page_size}),
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
    if r.variable_snapshot.is_some() {
        v["variables"] = json!(open_variables(&s, &r)?.sources);
    }
    v["steps"] = json!(
        rows.iter()
            .map(
                |v| json!({"index":v.step_index,"name":v.name,"state":v.state,
        "exit_code":v.exit_code,"started_at":v.started_at,"finished_at":v.finished_at})
            )
            .collect::<Vec<_>>()
    );
    v["artifacts"] = crate::artifacts::for_run(s.as_ref(), r.workspace_id, r.id).await?;
    Ok(Json(v))
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
        .map_err(|_| ApiError::bad("after 必须是非负整数。"))?
        .unwrap_or(0);
    if after < 0 {
        return Err(ApiError::bad("after 必须是非负整数。"));
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
        "stream":v.stream,"chunk":v.chunk})).collect::<Vec<_>>(),
        "next_seq":rows.last().map(|v|v.seq).unwrap_or(after)}),
    ))
}

#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerBody {
    branch: Option<String>,
    commit: Option<String>,
}

pub async fn trigger(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("pipelines.run")?;
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    let p = pipelines::Entity::find_by_id(i64::from(id))
        .filter(pipelines::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let definition = crate::pipelines::validate_definition(&p.definition)?;
    // Scope references again at execution time, not only when a definition is saved.
    let candidates =
        hosts::Entity::find().filter(hosts::Column::WorkspaceId.eq(scope.workspace_id));
    let host = match p.target_kind.as_str() {
        "host" => {
            candidates
                .filter(hosts::Column::Id.eq(p.target_ref))
                .one(&s.db)
                .await?
        }
        "host-group" => {
            host_groups::Entity::find_by_id(p.target_ref)
                .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
                .one(&s.db)
                .await?
                .ok_or_else(|| ApiError::bad("主机组不存在于当前工作空间。"))?;
            // One run has one concrete host. Resolve direct online members once,
            // deterministically, and retain that host even if group membership changes.
            candidates
                .filter(hosts::Column::GroupId.eq(p.target_ref))
                .filter(hosts::Column::State.eq("online"))
                .order_by_asc(hosts::Column::Id)
                .one(&s.db)
                .await?
        }
        _ => return Err(ApiError::bad("未知的执行目标类型。")),
    }
    .ok_or_else(|| ApiError::bad("当前工作空间中没有可用的执行主机（主机组需要在线成员）。"))?;
    let repo = if p.repository_id != 0 {
        Some(
            repositories::Entity::find_by_id(p.repository_id)
                .filter(repositories::Column::WorkspaceId.eq(scope.workspace_id))
                .one(&s.db)
                .await?
                .ok_or_else(|| ApiError::bad("仓库不存在于当前工作空间。"))?,
        )
    } else {
        None
    };
    let (repo_url, branch, credential_id) = repo
        .map(|r| (r.url, r.default_branch, r.credential_id))
        .unwrap_or_default();
    let body: TriggerBody = if body.is_empty() {
        TriggerBody::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|_| ApiError::bad("执行参数必须是包含 branch/commit 的 JSON 对象。"))?
    };
    let branch = body.branch.unwrap_or(branch);
    let commit = body.commit.unwrap_or_default();
    for value in [&branch, &commit] {
        if value.chars().count() > 255 || value.contains('\0') {
            return Err(ApiError::bad(
                "branch 和 commit 不能超过 255 字或包含空字符。",
            ));
        }
    }
    let (run_id, now) = (i64::from(s.next_id()?), db::now());
    let tx = s.db.begin().await?;
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
    let variable_rows = crate::variables::resolve(
        &tx,
        scope.workspace_id,
        "build",
        id,
        &crate::pipelines::variable_names(&definition),
    )
    .await?;
    let mut snapshot = VariableSnapshot::default();
    for (name, row) in variable_rows {
        let value = s.vault.open_secret(
            &row.value,
            &crate::vault::aad(
                &row.workspace_id.to_string(),
                "variables",
                &row.id.to_string(),
            ),
        )?;
        for environment in &definition.env {
            if environment.value_from.as_deref() == Some(&format!("variable/{name}")) {
                snapshot.sources.push(json!({"environment":environment.name,"name":name,"scope":row.scope,"scope_ref":Id::from(row.scope_ref),"version":row.version,"secret":row.kind=="secret"}));
            }
        }
        snapshot.values.insert(name, value);
    }
    let variable_snapshot = if snapshot.values.is_empty() {
        None
    } else {
        Some(
            s.vault.seal(
                &serde_json::to_string(&snapshot)
                    .map_err(|_| ApiError::bad("无法保存流水线变量快照。"))?,
                &crate::vault::aad(
                    &scope.workspace_id.to_string(),
                    "pipeline-run-variables",
                    &run_id.to_string(),
                ),
            )?,
        )
    };
    let run = runs::Model {
        id: run_id,
        workspace_id: i64::from(scope.workspace_id),
        pipeline_id: p.id,
        host_id: host.id,
        number: number,
        state: "queued".into(),
        trigger_kind: "manual".into(),
        triggered_by: i64::from(scope.user.id),
        definition: p.definition,
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
            name: Set(expand(&step.name, &run)),
            state: Set("queued".into()),
            exit_code: Set(None),
            started_at: Set(0),
            finished_at: Set(0),
        })
        .exec(&tx)
        .await?;
    }
    tx.commit().await?;
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
    finish(&s, r.id, "cancelled").await?;
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
async fn finish(s: &AppState, id: i64, state: &str) -> Result<()> {
    let tx = s.db.begin().await?;
    let changed = runs::Entity::update_many()
        .col_expr(runs::Column::State, Expr::value(state))
        .col_expr(runs::Column::FinishedAt, Expr::value(db::now()))
        .filter(runs::Column::Id.eq(id))
        .filter(runs::Column::State.is_in(["queued", "running"]))
        .exec(&tx)
        .await?
        .rows_affected;
    if changed > 0 {
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
async fn active(s: &AppState, id: i64) -> Result<bool> {
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
        .filter(runs::Column::State.eq("running"))
        .all(&s.db)
        .await
    {
        for r in rows {
            let _ = finish(&s, r.id, "failed").await;
        }
    }
    let mut next_cleanup = std::time::Instant::now();
    loop {
        if *stop.borrow() || *drain.borrow() {
            break;
        }
        if std::time::Instant::now() >= next_cleanup {
            if let Err(error) = crate::maintenance::prune_history(&s, false).await {
                tracing::error!(%error,"历史清理失败");
            }
            next_cleanup = std::time::Instant::now() + Duration::from_secs(60);
        }
        let result = dispatch(&s, &mut stop).await;
        if result.is_err() {
            tracing::error!("流水线调度失败；稍后检查队列。");
        }
        tokio::select! { _=drain.changed()=>break, _=stop.changed()=>break, _=tokio::time::sleep(Duration::from_millis(100))=>{} }
    }
}
async fn dispatch(s: &AppState, stop: &mut tokio::sync::watch::Receiver<bool>) -> Result<()> {
    let Some(r) = runs::Entity::find()
        .filter(runs::Column::State.eq("queued"))
        .order_by_asc(runs::Column::CreatedAt)
        .one(&s.db)
        .await?
    else {
        return Ok(());
    };
    let claimed = runs::Entity::update_many()
        .col_expr(runs::Column::State, Expr::value("running"))
        .col_expr(runs::Column::StartedAt, Expr::value(db::now()))
        .filter(runs::Column::Id.eq(r.id))
        .filter(runs::Column::State.eq("queued"))
        .exec(&s.db)
        .await?
        .rows_affected;
    if claimed == 0 {
        return Ok(());
    }
    if execute(s, &r, stop).await.is_err() {
        // Never log arbitrary remote errors here: they may contain credentials.
        append(
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
            "执行或凭据清理失败，请检查主机连接与配置。\n",
        )
        .await?;
        finish(s, r.id, "failed").await?;
    }
    Ok(())
}
fn quote(v: &str) -> String {
    format!("'{}'", v.replace('\'', "'\\''"))
}
fn expand(v: &str, r: &runs::Model) -> String {
    // Single pass: replacement values are never interpreted as more templates.
    let mut out = String::new();
    let mut rest = v;
    while let Some(pos) = rest.find("{{") {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos + 2..];
        let Some(end) = tail.find("}}") else {
            out.push_str(&rest[pos..]);
            return out;
        };
        out.push_str(&match &tail[..end] {
            "run" => r.number.to_string(),
            "branch" => r.branch.clone(),
            "repo_url" => r.repo_url.clone(),
            "commit" => r.commit.clone(),
            _ => format!("{{{{{}}}}}", &tail[..end]),
        });
        rest = &tail[end + 2..];
    }
    out.push_str(rest);
    out
}
async fn upload(ch: &dyn HostChannel, path: &str, value: String, mode: u32) -> Result<()> {
    ch.write_file(
        path,
        mode,
        Box::pin(futures::stream::once(async move { Ok(Bytes::from(value)) })),
    )
    .await
}
async fn append(
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
        *seq += 1;
        logs::Entity::insert(logs::ActiveModel {
            id: Set(i64::from(s.next_id()?)),
            run_id: Set(run),
            seq: Set(*seq),
            step_index: Set(index),
            stream: Set(stream.into()),
            chunk: Set(rest[..end].into()),
        })
        .exec(&s.db)
        .await?;
        rest = &rest[end..];
    }
    Ok(())
}
async fn execute(
    s: &AppState,
    r: &runs::Model,
    stop: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let host = hosts::Entity::find_by_id(r.host_id)
        .filter(hosts::Column::WorkspaceId.eq(r.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let host = crate::remote::Host {
        id: Id::from(host.id),
        workspace_id: Id::from(host.workspace_id),
        credential_id: Id::from(host.credential_id),
        name: host.name,
        address: host.address,
        port: host.port as u16,
        username: host.username,
        connection: host.connection,
        host_key: host.host_key,
    };
    let ch = crate::channel_open(s, &host).await?;
    let def = crate::pipelines::validate_definition(&r.definition)?;
    let variables = open_variables(s, r)?.values;
    let credential = if r.credential_id != 0 {
        crate::repository_auth::validate_target(
            s,
            &s.db,
            r.workspace_id.into(),
            r.credential_id.into(),
            &r.repo_url,
            false,
        )
        .await?;
        Some(credentials::unseal(s, &Id::from(r.workspace_id), &Id::from(r.credential_id)).await?)
    } else {
        None
    };
    let private = format!("/tmp/pipeline-credentials-{}", uuid::Uuid::new_v4());
    // mkdir is exclusive and 0700 before any secret is transferred. Secret
    // bytes only travel via write_file, NEVER via exec, env assignments or argv.
    let made = ch
        .exec(
            &format!("umask 077; mkdir {}", quote(&private)),
            Duration::from_secs(15),
        )
        .await?;
    if made.code != 0 {
        return Err(ApiError::bad("无法建立临时凭据目录。"));
    }
    let result = execute_private(
        s,
        r,
        &def,
        ch.as_ref(),
        &private,
        credential.as_ref(),
        &variables,
        stop,
    )
    .await;
    // This is outside the cancellable future. HostChannel cannot promise the
    // remote command stopped; deleting credentials still revokes future reads.
    let cleaned = ch
        .exec(
            &format!("rm -rf -- {}", quote(&private)),
            Duration::from_secs(15),
        )
        .await?;
    if cleaned.code != 0 {
        return Err(ApiError::bad("临时凭据清理失败。"));
    }
    let state = result?;
    finish(s, r.id, state).await
}
async fn execute_private(
    s: &AppState,
    r: &runs::Model,
    def: &Definition,
    ch: &dyn HostChannel,
    private: &str,
    credential: Option<&(String, String, String)>,
    variables: &BTreeMap<String, String>,
    stop: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<&'static str> {
    let mut env = String::new();
    for e in &def.env {
        let value = match (&e.value, &e.value_from) {
            (Some(value), None) => expand(value, r),
            (None, Some(reference)) => variables
                .get(reference.strip_prefix("variable/").unwrap_or(""))
                .cloned()
                .ok_or_else(|| ApiError::bad("流水线变量快照缺失。"))?,
            _ => return Err(ApiError::bad("流水线环境变量配置无效。")),
        };
        env.push_str(&format!("export {}={}\n", e.name, quote(&value)));
    }
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
                    "#!/bin/sh\ncat >/dev/null\n[ \"$1\" = get ] || exit 0\nprintf 'username='\ncat {private}/username\nprintf '\\npassword='\ncat {private}/password\nprintf '\\n\\n'\n"
                ),
                0o700,
            ).await?;
            env.push_str(&format!(
                "unset GIT_ASKPASS SSH_ASKPASS\nexport GIT_CONFIG_COUNT=3\nexport GIT_CONFIG_KEY_0=credential.helper\nexport GIT_CONFIG_VALUE_0=''\nexport GIT_CONFIG_KEY_1=credential.helper\nexport GIT_CONFIG_VALUE_1={}\nexport GIT_CONFIG_KEY_2=core.askPass\nexport GIT_CONFIG_VALUE_2=''\n",
                quote(&format!("{private}/credential-helper"))
            ));
        }
    }
    let mut seq = 0;
    for (index, step) in def.steps.iter().enumerate() {
        if !active(s, r.id).await? || *stop.borrow() {
            return Ok("cancelled");
        }
        let index = index as i64;
        let claimed = steps::Entity::update_many()
            .col_expr(steps::Column::State, Expr::value("running"))
            .col_expr(steps::Column::StartedAt, Expr::value(db::now()))
            .filter(steps::Column::RunId.eq(r.id))
            .filter(steps::Column::StepIndex.eq(index))
            .filter(steps::Column::State.eq("queued"))
            .exec(&s.db)
            .await?
            .rows_affected;
        if claimed == 0 {
            return Ok("cancelled");
        }
        // Upload the entire script too: env values never need to appear in ps.
        let wd = quote(&expand(&def.workdir, r));
        let script = format!(
            "{env}mkdir -p -- {wd} && cd -- {wd} || exit 1\n{}\n",
            expand(&step.run, r)
        );
        upload(ch, &format!("{private}/step"), script, 0o600).await?;
        let command = format!("sh {}", quote(&format!("{private}/step")));
        let seconds = step.timeout_seconds.unwrap_or(1800);
        let execution = ch.exec(&command, Duration::from_secs(seconds + 5));
        tokio::pin!(execution);
        let deadline = tokio::time::sleep(Duration::from_secs(seconds));
        tokio::pin!(deadline);
        let (state, output) = loop {
            tokio::select! {
                out=&mut execution=>break match out {
                    Ok(o)=>(if o.code==0 {"succeeded"}else{"failed"},Some(o)),
                    Err(e) if e.0==StatusCode::GATEWAY_TIMEOUT=>("timed_out",None),
                    Err(_)=>("failed",None),
                },
                _=&mut deadline=>break ("timed_out",None),
                _=stop.changed()=>break ("cancelled",None),
                _=tokio::time::sleep(Duration::from_millis(100))=>{
                    if !active(s,r.id).await? {break ("cancelled",None)}
                }
            }
        };
        let code = output.as_ref().map(|o| o.code);
        let output = output.unwrap_or(Output {
            code: -1,
            stdout: String::new(),
            stderr: format!("{state}\n"),
        });
        for (stream, text) in [("stdout", output.stdout), ("stderr", output.stderr)] {
            let text = redact(text, credential, variables);
            append(s, r.id, index, &mut seq, stream, &text).await?;
        }
        steps::Entity::update_many()
            .col_expr(steps::Column::State, Expr::value(state))
            .col_expr(steps::Column::ExitCode, Expr::value(code))
            .col_expr(steps::Column::FinishedAt, Expr::value(db::now()))
            .filter(steps::Column::RunId.eq(r.id))
            .filter(steps::Column::StepIndex.eq(index))
            .filter(steps::Column::State.eq("running"))
            .exec(&s.db)
            .await?;
        if state != "succeeded" {
            return Ok(if state == "cancelled" {
                "cancelled"
            } else {
                "failed"
            });
        }
    }
    if !active(s, r.id).await? || *stop.borrow() {
        return Ok("cancelled");
    }
    crate::artifacts::collect(s, r, ch, &expand(&def.workdir, r), &def.artifacts, stop).await?;
    Ok("succeeded")
}
fn redact(
    mut text: String,
    credential: Option<&(String, String, String)>,
    variables: &BTreeMap<String, String>,
) -> String {
    let mut values: Vec<&String> = variables.values().collect();
    if let Some((_, user, secret)) = credential {
        values.extend([secret, user]);
    }
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

    #[test]
    fn shell_quote_preserves_single_quotes_without_evaluation() {
        assert_eq!(quote("a'b$(id)"), "'a'\\''b$(id)'");
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
            expand("{{run}} {{branch}} {{repo_url}} {{commit}}", &r),
            "12 main https://example.invalid/{{run}} abc123"
        );
    }
}
