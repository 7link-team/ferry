//! Workflow operations use the same SSH/Agent channel, reservations and slots as Actions.
use crate::{
    accounts::workspace::Scope,
    entities::workflow_executions,
    error::{ApiError, Result},
    hosts::{channel::HostChannel, execution},
    i18n::Message,
    snowflake::Id,
    state::AppState,
};
use axum::http::StatusCode;
use bytes::Bytes;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, sea_query::Expr};
use serde_json::{Value, json};
use std::time::Duration;

const NODE_LOG_LIMIT: usize = 256 * 1024;
#[derive(Default)]
struct NodeLog {
    text: String,
    truncated: bool,
    dirty: bool,
}
impl NodeLog {
    fn push(&mut self, stream: &str, bytes: &[u8]) {
        if bytes.is_empty() || self.truncated { return; }
        let entry = format!("[{}] [{}] {}\n", chrono::Utc::now().to_rfc3339(), stream, String::from_utf8_lossy(&bytes[..bytes.len().min(NODE_LOG_LIMIT)]));
        let mut end = entry.len().min(NODE_LOG_LIMIT.saturating_sub(self.text.len()));
        while !entry.is_char_boundary(end) { end -= 1; }
        self.text.push_str(&entry[..end]);
        self.truncated = end < entry.len() || bytes.len() > NODE_LOG_LIMIT;
        self.dirty = true;
    }
    async fn flush(&mut self, s: &AppState, node: Option<i64>) -> Result<()> {
        if !self.dirty { return Ok(()); }
        if let Some(id) = node {
            use crate::entities::workflow_node_runs as nodes;
            nodes::Entity::update_many()
                .col_expr(nodes::Column::Logs, Expr::value(self.text.clone()))
                .col_expr(nodes::Column::LogsTruncated, Expr::value(self.truncated))
                .col_expr(nodes::Column::LogsUpdatedAt, Expr::value(crate::db::now()))
                .filter(nodes::Column::Id.eq(id))
                .filter(nodes::Column::State.eq("running"))
                .exec(&s.db).await?;
        }
        self.dirty = false;
        Ok(())
    }
}

pub(super) fn invalid() -> ApiError {
    ApiError::localized(
        StatusCode::BAD_REQUEST,
        Message::new("workflow.hostExecutionFailed"),
    )
}
pub(super) fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Apply a graph default only to operations which explicitly support hosts.
pub(super) fn effective(graph: &Value, node: &Value) -> Value {
    let mut node = node.clone();
    if node.pointer("/settings/runtime").is_none_or(Value::is_null)
        && super::graph::manifest(&node).is_some_and(|m| {
            m["runtime"]["allowed"]
                .as_array()
                .is_some_and(|a| a.contains(&json!("host")))
        })
    {
        if let Some(default) = graph.pointer("/settings/execution") {
            if !node["settings"].is_object() {
                node["settings"] = json!({});
            }
            for key in ["runtime", "target_kind", "target_ref"] {
                if let Some(value) = default.get(key) {
                    if key != "runtime"
                        || super::graph::manifest(&node).is_some_and(|m| {
                            m["runtime"]["allowed"]
                                .as_array()
                                .is_some_and(|a| a.contains(value))
                        })
                    {
                        node["settings"][key] = value.clone();
                    }
                }
            }
        }
    }
    node
}
pub(super) fn requested(node: &Value) -> bool {
    node.pointer("/settings/runtime")
        .and_then(Value::as_str)
        .or_else(|| super::graph::manifest(node).and_then(|m| m["runtime"]["default"].as_str()))
        == Some("host")
}

async fn active(s: &AppState, run: &workflow_executions::Model) -> Result<()> {
    if crate::executions::stop_requested() {
        return Err(crate::executions::ownership_lost());
    }
    let scope = Scope::for_actor(s, run.workspace_id.into(), run.triggered_by.into()).await?;
    scope.require("workflows.run")?;
    scope.require("hosts.terminal")?;
    let workflow = super::scoped(&s.db, scope.workspace_id, run.workflow_id.into()).await?;
    crate::definitions::access::require(
        &s.db,
        &scope,
        &workflow,
        crate::definitions::access::Operation::Run,
    )
    .await?;
    if workflow.lifecycle != "active"
        || workflow_executions::Entity::find_by_id(run.id)
            .one(&s.db)
            .await?
            .is_none_or(|r| r.state != "running")
    {
        return Err(ApiError::forbidden());
    }
    Ok(())
}

/// The script is transferred privately: credentials never appear in command arguments.
pub(super) async fn execute(
    s: &AppState,
    run: &workflow_executions::Model,
    node: &Value,
    script: &str,
    log_output: bool,
) -> Result<Value> {
    let log_node = if log_output {
        use crate::entities::workflow_node_runs as nodes;
        nodes::Entity::find().filter(nodes::Column::ExecutionId.eq(run.id))
            .filter(nodes::Column::NodeId.eq(node["id"].as_str().unwrap_or("")))
            .filter(nodes::Column::State.eq("running"))
            .order_by_desc(nodes::Column::Id).one(&s.db).await?.map(|row|row.id)
    } else { None };
    let target = node
        .pointer("/settings/target_ref")
        .and_then(Value::as_str)
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .ok_or_else(invalid)?;
    let group = node
        .pointer("/settings/target_kind")
        .and_then(Value::as_str)
        == Some("host_group");
    let lease = loop {
        active(s, run).await?;
        let tx = crate::variables::transaction(s, run.workspace_id.into()).await?;
        let candidates = execution::candidates(
            &tx,
            run.workspace_id,
            if group { 0 } else { target },
            if group { target } else { 0 },
            false,
            &[],
        )
        .await?;
        if candidates.is_empty() {
            return Err(invalid());
        }
        let mut chosen = None;
        for (host, policy, load) in candidates {
            if load < policy.capacity as usize {
                if let Some(lease) = execution::try_acquire(
                    &tx,
                    host.id,
                    policy.capacity,
                    run.id,
                    &format!("workflow:{}", node["id"].as_str().unwrap_or_default()),
                )
                .await?
                {
                    chosen = Some(lease);
                    break;
                }
            }
        }
        tx.commit().await?;
        if let Some(lease) = chosen {
            break lease;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    let result = async {
        let (_idle, idle) = tokio::sync::watch::channel(false);
        let mut stop = crate::executions::stop_signal().unwrap_or(idle);
        let lease_ref = &lease;
        let mut renewed = std::time::Instant::now();
        let guards = crate::hosts::runner::reservations::wait_for_reservation(
            crate::hosts::runner::reservations::reserve_with(s.cluster.as_ref(), vec![lease.host.to_string()]),
            &mut stop,
            || {
                let renew = renewed.elapsed() >= Duration::from_secs(10);
                if renew { renewed = std::time::Instant::now(); }
                async move {
                    active(s, run).await?;
                    if renew && !lease_ref.renew(&s.db).await? {
                        return Err(crate::executions::ownership_lost());
                    }
                    Ok(true)
                }
            },
        ).await?.ok_or_else(crate::executions::ownership_lost)?;
        guards.run(async {
            active(s, run).await?;
            if !lease.renew(&s.db).await? { return Err(crate::executions::ownership_lost()); }
            let row = crate::entities::hosts::Entity::find_by_id(lease.host).one(&s.db).await?.ok_or_else(invalid)?;
            let ch = crate::hosts::remote::open(s, &Id::from(run.workspace_id), &Id::from(lease.host)).await?;
            let (ch, private): (Box<dyn HostChannel>, String) = if row.os == "windows" {
                let shell = crate::hosts::windows_shell::GitShell::open(ch).await?;
                let private = shell.private_dir("ferry-workflow").await?;
                (Box::new(shell), private)
            } else {
                let private = format!("/tmp/workflow-{}", uuid::Uuid::new_v4());
                let output = ch.exec(&format!("umask 077; mkdir {}", quote(&private)), Duration::from_secs(15)).await?;
                if output.code != 0 { return Err(invalid()); }
                (ch, private)
            };
            let result = async {
                let path = format!("{private}/operation.sh");
                let bytes = Bytes::from(script.to_owned());
                ch.write_file(&path, 0o700, Box::pin(futures::stream::once(async move {Ok(bytes)}))).await?;
                let (cancel, receiver) = tokio::sync::watch::channel(false);
                let (sender, mut output) = tokio::sync::mpsc::channel(16);
                let command = format!("sh {}", quote(&path));
                let task = ch.exec_stream(&command, Duration::from_secs(120), sender, receiver);
                tokio::pin!(task);
                let mut stdout = Vec::new();
                let mut node_log = NodeLog::default();
                let mut log_tick = tokio::time::interval(Duration::from_millis(250));
                let mut stderr_len = 0usize;
                let mut failure = None;
                let mut output_open = true;
                let mut check = tokio::time::interval(Duration::from_millis(500));
                let mut renewal = tokio::time::interval(Duration::from_secs(10));
                let mut code = loop {
                    tokio::select! {
                        result = &mut task => break result?,
                        chunk = output.recv(), if output_open => if let Some((kind, bytes)) = chunk {
                            if log_node.is_some() { node_log.push(&kind, &bytes); }
                            if kind == "stdout" {
                                if stdout.len() + bytes.len() > 1024*1024 { failure.get_or_insert_with(invalid); let _ = cancel.send(true); }
                                else { stdout.extend_from_slice(&bytes); }
                            } else { stderr_len += bytes.len(); if stderr_len > 1024*1024 { failure.get_or_insert_with(invalid); let _=cancel.send(true); } }
                        } else { output_open = false; },
                        _ = log_tick.tick(), if log_node.is_some() && node_log.dirty => if let Err(error) = node_log.flush(s, log_node).await { failure.get_or_insert(error); let _ = cancel.send(true); },
                        _ = check.tick() => if let Err(error) = active(s, run).await { failure.get_or_insert(error); let _ = cancel.send(true); },
                        _ = renewal.tick() => if !lease.renew(&s.db).await.unwrap_or(false) { failure.get_or_insert_with(crate::executions::ownership_lost); let _=cancel.send(true); },
                    }
                };
                // Drain chunks queued immediately before process completion.
                while let Ok((kind, bytes)) = output.try_recv() {
                    if log_node.is_some() { node_log.push(&kind, &bytes); }
                    if kind == "stdout" { if stdout.len()+bytes.len()>1024*1024 {code=-1;} else {stdout.extend_from_slice(&bytes);} }
                    else { stderr_len += bytes.len(); if stderr_len > 1024*1024 { code = -1; } }
                }
                node_log.flush(s, log_node).await?;
                if let Some(error) = failure { return Err(error); }
                active(s, run).await?;
                if code != 0 { return Err(invalid()); }
                Ok(json!({"stdout":String::from_utf8(stdout).map_err(|_| invalid())?,"execution_host":{"id":Id::from(row.id),"name":row.name,"connection":row.connection}}))
            }.await;
            // Only the freshly generated private directory is removed.
            let cleaned = ch.exec(&format!("rm -rf -- {}", quote(&private)), Duration::from_secs(30)).await?;
            if cleaned.code != 0 { return Err(invalid()); }
            result
        }).await
    }.await;
    lease.release(&s.db).await?;
    result
}

pub(super) fn git_script(operation: &str, params: &Value) -> Result<String> {
    let git =
        "git -c core.hooksPath=/dev/null -c protocol.file.allow=never -c protocol.ext.allow=never";
    let url = || -> Result<String> {
        let raw = params["url"].as_str().ok_or_else(invalid)?;
        let url = reqwest::Url::parse(raw).map_err(|_| invalid())?;
        if !matches!(url.scheme(), "https" | "ssh")
            || url.host_str().is_none()
            || url.password().is_some()
            || (url.scheme() == "https" && !url.username().is_empty())
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid());
        }
        Ok(quote(raw))
    };
    let directory = || -> Result<String> {
        let directory = params["directory"].as_str().ok_or_else(invalid)?;
        crate::hosts::channel::check_path(directory)?;
        Ok(quote(directory))
    };
    let command = match operation {
        "remote_refs" => format!("{git} ls-remote --heads --tags -- {}", url()?),
        "clone" => format!(
            "{git} clone --depth {} -- {} {}",
            params["depth"].as_u64().unwrap_or(1).clamp(1, 10000),
            url()?,
            directory()?
        ),
        "status" => format!("{git} -C {} status --porcelain=v1 --branch", directory()?),
        "log" => format!(
            "{git} -C {} log -n {} --format='%H%x09%an%x09%ae%x09%aI%x09%s' --",
            directory()?,
            params["limit"].as_u64().unwrap_or(20).clamp(1, 100)
        ),
        "branches" => format!("{git} -C {} branch --all --no-color", directory()?),
        "tags" => format!("{git} -C {} tag --list", directory()?),
        "fetch" => format!(
            "{git} -C {} fetch --no-recurse-submodules origin",
            directory()?
        ),
        _ => return Err(invalid()),
    };
    Ok(format!(
        "#!/bin/sh\nset -eu\nexport GIT_TERMINAL_PROMPT=0 GIT_OPTIONAL_LOCKS=0\n{command}\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ColumnTrait, ConnectionTrait, IntoActiveModel, QueryFilter};

    #[test]
    fn node_logs_bound_unicode_and_stop_growing_after_truncation() {
        let mut log = NodeLog::default();
        log.push("stderr", "failure 中文".repeat(NODE_LOG_LIMIT).as_bytes());
        assert!(log.truncated);
        assert!(log.text.len() <= NODE_LOG_LIMIT);
        assert!(log.text.contains("[stderr] failure 中文"));
        let frozen = log.text.clone();
        log.dirty = false;
        log.push("stdout", b"must not keep writing");
        assert_eq!(log.text, frozen);
        assert!(!log.dirty);
    }

    #[test]
    fn node_logs_keep_both_streams_and_ignore_empty_chunks() {
        let mut log = NodeLog::default();
        log.push("stdout", b"");
        assert!(!log.dirty);
        log.push("stdout", b"first");
        log.push("stderr", b"second");
        assert!(log.text.contains("[stdout] first"));
        assert!(log.text.contains("[stderr] second"));
        assert!(!log.truncated);
    }

    async fn waiting_fixture() -> (
        tempfile::TempDir,
        super::super::journal::tests::Fixture,
        AppState,
        i64,
    ) {
        use crate::entities::{users, workspaces};
        use sea_orm::Set;
        let fixture = super::super::journal::tests::Fixture::new().await;
        users::Entity::insert(users::ActiveModel {
            id: Set(3),
            email: Set("host-wait@example.invalid".into()),
            name: Set("Host wait".into()),
            password_hash: Set(String::new()),
            is_superadmin: Set(1),
            active: Set(1),
            version: Set(1),
            created_at: Set(0),
            updated_at: Set(0),
            last_login_at: Set(None),
            last_login_ip: Set(None),
        })
        .exec(&fixture.db)
        .await
        .unwrap();
        workspaces::Entity::insert(workspaces::ActiveModel {
            id: Set(11),
            slug: Set("host-wait".into()),
            name: Set("Host wait".into()),
            owner_id: Set(3),
            join_code: Set("host-wait".into()),
            pipeline_run_number: Set(0),
            workflow_execution_number: Set(0),
            created_at: Set(0),
            updated_at: Set(0),
        })
        .exec(&fixture.db)
        .await
        .unwrap();
        let host = i64::from(rand::random::<u32>()) + 100;
        fixture.db.execute_unprepared(&format!("INSERT INTO hosts (id,workspace_id,name,address,port,username,connection,state,group_id,description,host_key,runner_key_hash,enroll_token,enroll_token_hash,jump_host_id,seen_at,created_at,updated_at,version,os,arch,agent_version,metrics,credential_id) VALUES ({host},11,'host wait','127.0.0.1',22,'fixture','runner','online',0,'','','','','',0,0,0,0,1,'linux','x86_64','','',0)")).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let s = std::sync::Arc::new(crate::state::State {
            ids: std::sync::Arc::new(
                crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap(),
            ),
            db: fixture.db.clone(),
            data_dir: dir.path().into(),
            instance_id: "host-wait".into(),
            server_mode: false,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), false).unwrap(),
            cluster: None,
            host_monitor: Default::default(),
            monitor: Default::default(),
        });
        (dir, fixture, s, host)
    }

    async fn slot(s: &AppState, host: i64) -> Option<crate::entities::host_execution_slots::Model> {
        use crate::entities::host_execution_slots as slots;
        slots::Entity::find()
            .filter(slots::Column::HostId.eq(host))
            .one(&s.db)
            .await
            .unwrap()
    }

    async fn blocked(
        s: &AppState,
        run: workflow_executions::Model,
        host: i64,
    ) -> tokio::task::JoinHandle<Result<Value>> {
        let state = s.clone();
        let node =
            json!({"id":"git","settings":{"target_ref":host.to_string(),"target_kind":"host"}});
        let task = tokio::spawn(async move { execute(&state, &run, &node, "exit 0", false).await });
        for _ in 0..100 {
            if slot(s, host).await.is_some() {
                return task;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        task.abort();
        panic!("workflow never acquired its execution slot");
    }

    #[tokio::test]
    async fn cancelling_while_waiting_for_exclusive_reservation_releases_the_slot() {
        let (_dir, fixture, s, host) = waiting_fixture().await;
        let _upgrade = crate::hosts::runner::reservations::exclusive(&host.to_string()).await;
        let mut task = blocked(&s, fixture.run().await, host).await;
        s.db.execute_unprepared("UPDATE workflow_executions SET state='cancelling'")
            .await
            .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), &mut task).await;
        if result.is_err() {
            task.abort();
            let _ = task.await;
        }
        let result = result
            .expect("cancellation must not wait for the upgrade to finish")
            .unwrap();
        assert_eq!(result.unwrap_err().0, StatusCode::FORBIDDEN);
        assert!(slot(&s, host).await.is_none());
    }

    #[tokio::test]
    async fn revoked_actor_while_waiting_for_upgrade_releases_the_slot() {
        let (_dir, fixture, s, host) = waiting_fixture().await;
        let _upgrade = crate::hosts::runner::reservations::exclusive(&host.to_string()).await;
        let mut task = blocked(&s, fixture.run().await, host).await;
        s.db.execute_unprepared("UPDATE users SET active=0 WHERE id=3")
            .await
            .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), &mut task).await;
        if result.is_err() {
            task.abort();
            let _ = task.await;
        }
        assert!(
            result
                .expect("revocation must not wait for the upgrade")
                .unwrap()
                .is_err()
        );
        assert!(slot(&s, host).await.is_none());
    }

    #[tokio::test]
    async fn exclusive_reservation_wait_renews_the_execution_slot() {
        let (_dir, fixture, s, host) = waiting_fixture().await;
        let _upgrade = crate::hosts::runner::reservations::exclusive(&host.to_string()).await;
        let mut task = blocked(&s, fixture.run().await, host).await;
        let before = slot(&s, host).await.unwrap().expires_at;
        tokio::time::sleep(Duration::from_secs(11)).await;
        let renewed = slot(&s, host).await.unwrap().expires_at;
        s.db.execute_unprepared("UPDATE workflow_executions SET state='cancelling'")
            .await
            .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), &mut task).await;
        if result.is_err() {
            task.abort();
            let _ = task.await;
        }
        assert!(
            renewed > before,
            "waiting for an upgrade must retain capacity ownership"
        );
        assert!(result.unwrap().unwrap().is_err());
        assert!(slot(&s, host).await.is_none());
    }

    #[tokio::test]
    async fn stolen_slot_cannot_open_a_channel_after_the_upgrade_finishes() {
        let (_dir, fixture, s, host) = waiting_fixture().await;
        let upgrade = crate::hosts::runner::reservations::exclusive(&host.to_string()).await;
        let mut task = blocked(&s, fixture.run().await, host).await;
        let mut successor = slot(&s, host).await.unwrap().into_active_model();
        successor.owner = sea_orm::Set("successor".into());
        crate::entities::host_execution_slots::Entity::update(successor)
            .exec(&s.db)
            .await
            .unwrap();
        assert_eq!(slot(&s, host).await.unwrap().owner, "successor");
        drop(upgrade);
        let result = tokio::time::timeout(Duration::from_secs(2), &mut task).await;
        if result.is_err() {
            task.abort();
            let _ = task.await;
        }
        let error = result.unwrap().unwrap().unwrap_err();
        assert_eq!(error.2.unwrap().key, "execution.leaseLost");
        assert_eq!(slot(&s, host).await.unwrap().owner, "successor");
    }
    #[test]
    fn git_rejects_local_transports_and_embedded_secrets_and_quotes_host_paths() {
        for url in [
            "file:///etc/passwd",
            "ext::sh -c evil",
            "https://token@example.org/repo",
            "https://user:secret@example.org/repo",
        ] {
            assert!(git_script("remote_refs", &json!({"url":url})).is_err());
        }
        assert!(git_script("log", &json!({"directory":"../outside"})).is_err());
        let script = git_script(
            "log",
            &json!({"directory":"/srv/repo'; touch /tmp/escape #","limit":20}),
        )
        .unwrap();
        assert!(script.contains("'/srv/repo'\\''; touch /tmp/escape #'"));
        assert!(script.contains("protocol.file.allow=never"));
    }
    #[test]
    fn workflow_defaults_apply_only_to_supported_nodes_and_node_overrides_win() {
        let graph = json!({"settings":{"execution":{"runtime":"host","target_ref":"7","target_kind":"host"}}});
        let http = json!({"type":"integration.github.repository.get","type_version":1});
        assert_eq!(effective(&graph, &http)["settings"]["target_ref"], "7");
        let node = json!({"type":"integration.github.repository.get","type_version":1,"settings":{"runtime":"server"}});
        assert_eq!(effective(&graph, &node)["settings"]["runtime"], "server");
        assert!(
            effective(&graph, &json!({"type":"data.set","type_version":1}))["settings"].is_null()
        );
    }
}
