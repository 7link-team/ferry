use super::*;
use crate::{
    entities::{pipeline_artifacts, pipeline_runs, repository_events},
    repositories::auth::MonitorAccess,
};
use sha2::{Digest, Sha256};
use std::{io::Read, time::Duration};
use tokio::sync::watch;

pub(super) async fn sources(
    s: &AppState,
    scope: &Scope,
    c: &Config,
) -> Result<Vec<(i64, String, String)>> {
    authorize_config(s, scope, c).await?;
    let mut result = BTreeMap::new();
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
        if connection.provider == "git" {
            result.insert(
                (connection.id, selection.repository.clone()),
                selection.repository.clone(),
            );
            continue;
        }
        let access = MonitorAccess::load(s, &connection).await?;
        for page in 1..=10000 {
            let value = access.repositories(page).await?;
            for row in value["items"]
                .as_array()
                .ok_or_else(|| bad("repositoryListing"))?
            {
                let path = row["full_name"]
                    .as_str()
                    .ok_or_else(|| bad("repositoryListing"))?;
                if matches_selection(selection, path) {
                    result.insert(
                        (connection.id, path.to_string()),
                        row["url"]
                            .as_str()
                            .ok_or_else(|| bad("repositoryListing"))?
                            .to_owned(),
                    );
                }
            }
            if value["has_more"] == false {
                break;
            }
            if page == 10000 {
                return Err(bad("repositoryListingLimit"));
            }
        }
    }
    Ok(result
        .into_iter()
        .map(|((conn, path), url)| (conn, path, url))
        .collect())
}

pub(super) async fn resolve_shas(
    s: &AppState,
    scope: &Scope,
    row: &defs::Model,
    c: &Config,
    connection: i64,
    path: &str,
    base: Option<&str>,
    head: Option<&str>,
    full_scan: bool,
) -> Result<(String, String)> {
    let mut base = base.unwrap_or("").to_owned();
    let mut head = head.unwrap_or("").to_owned();
    if full_scan && base.is_empty() && valid_sha(&head) {
        base = head.clone();
    }
    if base.is_empty() || head.is_empty() {
        let conn =
            crate::repositories::auth::owned_connection(&s.db, scope, connection.into()).await?;
        if conn.provider != "git" {
            let commits = MonitorAccess::load(s, &conn)
                .await?
                .commits(path, &c.branch)
                .await?;
            let commit = commits.first().ok_or_else(|| bad("shaRequired"))?;
            let text = |p: &str| {
                commit
                    .pointer(p)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned()
            };
            if head.is_empty() {
                head = ["/sha", "/id", "/commitId"]
                    .iter()
                    .map(|p| text(p))
                    .find(|s| valid_sha(s))
                    .unwrap_or_default();
            }
            if base.is_empty() {
                base = ["/parents/0/sha", "/parents/0/id", "/parentIds/0"]
                    .iter()
                    .map(|p| text(p))
                    .find(|s| valid_sha(s))
                    .unwrap_or_default();
            }
        } else {
            let refs = observe_git(s, scope, c, &conn, path).await?;
            if head.is_empty() {
                head = refs.get(&c.branch).cloned().unwrap_or_default();
            }
            if base.is_empty() {
                let baselines: BTreeMap<String, String> = decode(&row.baselines)?;
                base = baselines
                    .get(&format!("{}:{}", connection, path))
                    .cloned()
                    .unwrap_or_default();
            }
            let events = repository_events::Entity::find()
                .filter(repository_events::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(repository_events::Column::ConnectionId.eq(connection))
                .filter(repository_events::Column::Repository.eq(path))
                .filter(repository_events::Column::Event.eq("push"))
                .order_by_desc(repository_events::Column::CreatedAt)
                .limit(100)
                .all(&s.db)
                .await?;
            for e in events {
                if e.reference.trim_start_matches("refs/heads/") != c.branch {
                    continue;
                }
                let d: Value = e
                    .detail
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(json!({}));
                if head.is_empty() {
                    head = d["after"].as_str().unwrap_or("").to_owned();
                }
                if base.is_empty() {
                    base = d["before"].as_str().unwrap_or("").to_owned();
                }
                break;
            }
        }
        if base.is_empty() {
            let previous = jobs::Entity::find()
                .filter(jobs::Column::ReviewId.eq(row.id))
                .filter(jobs::Column::State.eq("completed"))
                .order_by_desc(jobs::Column::CreatedAt)
                .limit(100)
                .all(&s.db)
                .await?;
            for job in previous {
                let snap: Snapshot = decode(&job.snapshot)?;
                if snap.connection_id == connection && snap.repository == path {
                    base = snap.head_sha;
                    break;
                }
            }
        }
    }
    if full_scan && base.is_empty() && valid_sha(&head) {
        base = head.clone();
    }
    if !valid_sha(&base) || !valid_sha(&head) || base.len() != head.len() {
        return Err(bad("shaRequired"));
    }
    Ok((base, head))
}

pub(super) async fn authorize_snapshot(s: &AppState, scope: &Scope, snap: &Snapshot) -> Result<()> {
    scope.require("repositories.read")?;
    scope.require("credentials.read")?;
    if i64::from(scope.user.id) != snap.user_id {
        return Err(ApiError::forbidden());
    }
    crate::repositories::auth::resolve_connection(
        s,
        &s.db,
        scope,
        snap.connection_id.into(),
        &snap.url,
    )
    .await?;
    Ok(())
}
pub(super) async fn enqueue(
    s: &AppState,
    row: &defs::Model,
    mut snap: Snapshot,
) -> Result<jobs::Model> {
    let scope = Scope::for_actor(s, row.workspace_id.into(), snap.user_id.into()).await?;
    scope.require("code-reviews.run")?;
    scope.require("pipelines.run")?;
    authorize_snapshot(s, &scope, &snap).await?;
    let tx = crate::variables::transaction(s, row.workspace_id.into()).await?;
    let actor = Scope::for_actor_in(&tx, row.workspace_id.into(), snap.user_id.into()).await?;
    actor.require("code-reviews.run")?;
    actor.require("pipelines.run")?;
    actor.require("repositories.read")?;
    actor.require("credentials.read")?;
    crate::repositories::auth::resolve_connection(
        s,
        &tx,
        &actor,
        snap.connection_id.into(),
        &snap.url,
    )
    .await?;
    let current = defs::Entity::find_by_id(row.id)
        .lock_exclusive()
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    if current.version != row.version || !decode::<Config>(&current.config)?.enabled {
        return Err(conflict());
    }
    let pipeline = crate::entities::pipelines::Entity::find_by_id(snap.pipeline_id)
        .filter(crate::entities::pipelines::Column::WorkspaceId.eq(row.workspace_id))
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    crate::pipelines::validate_definition(&pipeline.definition)?;
    snap.definition = pipeline.definition;
    let key = hex::encode(Sha256::digest(
        format!(
            "{}:{}:{}:{}:{}:{}:{}:{}",
            row.id,
            row.version,
            snap.connection_id,
            snap.repository,
            snap.base_sha,
            snap.head_sha,
            snap.full_scan,
            format!("{}:{}", snap.config.tests_enabled, snap.config.ai_enabled)
        )
        .as_bytes(),
    ));
    if let Some(existing) = jobs::Entity::find()
        .filter(jobs::Column::Dedupe.eq(&key))
        .lock_exclusive()
        .one(&tx)
        .await?
    {
        tx.commit().await?;
        return Ok(existing);
    }
    let id = s.next_id()?;
    let model = jobs::Model {
        id: id.into(),
        workspace_id: row.workspace_id,
        review_id: row.id,
        dedupe: key.clone(),
        state: "queued".into(),
        snapshot: encode(&snap)?,
        result: encode(
            &json!({"test_state":if snap.config.tests_enabled{"pending"}else{"disabled"},"ai_state":if snap.config.ai_enabled{"pending"}else{"disabled"},"coverage":{"scanned":[],"skipped":[]},"error":""}),
        )?,
        pipeline_run_id: 0,
        lease_until: 0,
        lease_owner: String::new(),
        created_at: db::now(),
        finished_at: 0,
    };
    jobs::Entity::insert(model.into_active_model())
        .on_conflict_do_nothing()
        .exec_without_returning(&tx)
        .await?;
    let result = jobs::Entity::find()
        .filter(jobs::Column::Dedupe.eq(key))
        .lock_exclusive()
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    tx.commit().await?;
    Ok(result)
}

/// Public scheduler hook: durable rows are the queue; locks use the existing SQL lease.
pub(crate) async fn scheduler(s: AppState, mut shutdown: watch::Receiver<bool>) {
    let event_state = s.clone();
    let mut event_stop = shutdown.clone();
    let events = async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            tokio::select! {
                _=event_stop.changed()=>{if *event_stop.borrow(){break;}},
                _=interval.tick()=>{if let Err(e)=poll_events(&event_state).await{tracing::warn!(error=%e,"code review event matching failed");}}
            }
        }
    };
    let jobs = async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3));
        let mut active = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _=shutdown.changed()=>{if *shutdown.borrow(){active.abort_all();while active.join_next().await.is_some(){}break;}},
                result=active.join_next(),if !active.is_empty()=>{if let Some(Err(e))=result{tracing::warn!(error=%e,"code review worker terminated");}},
                _=interval.tick()=>{
                    match pending(&s).await {
                        Ok(rows)=>for row in rows.into_iter().take(16usize.saturating_sub(active.len())) {let s=s.clone();active.spawn(async move {if let Err(e)=process(&s,row).await {tracing::warn!(error=%e,"code review worker failed");}});},
                        Err(e)=>tracing::warn!(error=%e,"code review scheduler query failed"),
                    }
                }
            }
        }
    };
    tokio::join!(events, jobs);
}
async fn pending(s: &AppState) -> Result<Vec<jobs::Model>> {
    Ok(jobs::Entity::find()
        .filter(jobs::Column::State.is_in([
            "queued",
            "launching",
            "waiting",
            "running",
            "cancelling",
        ]))
        .filter(jobs::Column::LeaseUntil.lte(db::now()))
        .order_by_asc(jobs::Column::CreatedAt)
        .limit(50)
        .all(&s.db)
        .await?)
}
#[cfg(test)]
pub(super) async fn tick(s: &AppState) -> Result<()> {
    for row in pending(s).await? {
        process(s, row).await?;
    }
    Ok(())
}

/// Each job owns a renewable database fence. Dropping the future leaves the
/// durable state recoverable only after expiry; stale owners cannot publish.
pub(super) async fn process(s: &AppState, mut row: jobs::Model) -> Result<()> {
    let name = format!("code-review-job:{}", row.id);
    let owner = uuid::Uuid::new_v4().to_string();
    if !crate::lock::acquire(&s.db, &name, &owner, db::now(), 60_000).await? {
        return Ok(());
    }
    let claim = crate::variables::transaction(s, row.workspace_id.into()).await?;
    let lock = crate::entities::locks::Entity::find_by_id(name.clone())
        .lock_exclusive()
        .one(&claim)
        .await?
        .ok_or_else(|| bad("jobLeaseExpired"))?;
    if lock.owner != owner || lock.expires_at <= db::now() {
        return Err(bad("jobLeaseExpired"));
    }
    let current = jobs::Entity::find_by_id(row.id)
        .lock_exclusive()
        .one(&claim)
        .await?
        .ok_or_else(ApiError::missing)?;
    if !["queued", "launching", "waiting", "running", "cancelling"]
        .contains(&current.state.as_str())
    {
        claim.commit().await?;
        crate::lock::release(&s.db, &name, &owner).await?;
        return Ok(());
    }
    row = current;
    row.lease_owner = owner.clone();
    row.lease_until = db::now() + 60_000;
    jobs::Entity::update_many()
        .col_expr(jobs::Column::LeaseOwner, Expr::value(&owner))
        .col_expr(jobs::Column::LeaseUntil, Expr::value(row.lease_until))
        .filter(jobs::Column::Id.eq(row.id))
        .exec(&claim)
        .await?;
    claim.commit().await?;
    let result = {
        let renewal = async {
            let mut timer = tokio::time::interval(Duration::from_secs(10));
            timer.tick().await;
            loop {
                timer.tick().await;
                if crate::lock::renew(&s.db, &name, &owner, db::now(), 60_000).await? != 1 {
                    return Err::<(), ApiError>(bad("jobLeaseExpired"));
                }
                if jobs::Entity::update_many()
                    .col_expr(jobs::Column::LeaseUntil, Expr::value(db::now() + 60_000))
                    .filter(jobs::Column::Id.eq(row.id))
                    .filter(jobs::Column::LeaseOwner.eq(&owner))
                    .exec(&s.db)
                    .await?
                    .rows_affected
                    != 1
                {
                    return Err(bad("jobLeaseExpired"));
                }
            }
        };
        tokio::select! {result=process_owned(s,row.clone())=>result,result=renewal=>{result?;Err(bad("jobLeaseExpired"))}}
    };
    if let Err(error) = result {
        let latest = jobs::Entity::find_by_id(row.id)
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        let mut value: Value = decode(&latest.result)?;
        value["error"] = json!(error.stored());
        if value["ai_state"] == "pending" {
            value["ai_state"] = json!("failed");
        }
        if value["test_state"] == "pending" {
            value["test_state"] = json!("unavailable");
        }
        finish(s, &row, "failed", &value).await?;
    }
    jobs::Entity::update_many()
        .col_expr(jobs::Column::LeaseUntil, Expr::value(0i64))
        .filter(jobs::Column::Id.eq(row.id))
        .filter(jobs::Column::LeaseOwner.eq(&owner))
        .exec(&s.db)
        .await?;
    crate::lock::release(&s.db, &name, &owner).await?;
    Ok(())
}
async fn fenced(s: &AppState, row: &jobs::Model) -> Result<sea_orm::DatabaseTransaction> {
    let tx = crate::variables::transaction(s, row.workspace_id.into()).await?;
    let lock = crate::entities::locks::Entity::find_by_id(format!("code-review-job:{}", row.id))
        .lock_exclusive()
        .one(&tx)
        .await?
        .ok_or_else(|| bad("jobLeaseExpired"))?;
    if lock.owner != row.lease_owner || lock.expires_at <= db::now() {
        return Err(bad("jobLeaseExpired"));
    }
    let current = jobs::Entity::find_by_id(row.id)
        .lock_exclusive()
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    if current.lease_owner != row.lease_owner || current.lease_until <= db::now() {
        return Err(bad("jobLeaseExpired"));
    }
    Ok(tx)
}
async fn process_owned(s: &AppState, mut row: jobs::Model) -> Result<()> {
    let snap: Snapshot = decode(&row.snapshot)?;
    if row.state == "cancelling" {
        let run = pipeline_runs::Entity::find_by_id(row.pipeline_run_id)
            .filter(pipeline_runs::Column::WorkspaceId.eq(row.workspace_id))
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        if run.pipeline_id != snap.pipeline_id
            || run.triggered_by != snap.user_id
            || run.commit != snap.head_sha
        {
            return Err(bad("runContract"));
        }
        // Recover a crash after atomically marking an unclaimed run cancelling.
        if run.state == "cancelling" && run.started_at == 0 {
            crate::pipelines::runs::finish(s, run.id, "cancelled").await?;
        } else if !["succeeded", "failed", "cancelled", "skipped"].contains(&run.state.as_str()) {
            return Ok(());
        }
        let mut result: Value = decode(&row.result)?;
        cancel_result(&mut result);
        return finish(s, &row, "cancelled", &result).await;
    }
    let scope = Scope::for_actor(s, row.workspace_id.into(), snap.user_id.into()).await?;
    scope.require("code-reviews.run")?;
    scope.require("pipelines.run")?;
    authorize_snapshot(s, &scope, &snap).await?;
    let current = owned(s, &scope, row.review_id.into()).await?;
    if !decode::<Config>(&current.config)?.enabled {
        return Err(bad("disabled"));
    }
    if row.pipeline_run_id == 0 {
        if row.state == "launching" {
            // Recover a launch committed by the pipeline engine before this worker crashed.
            let candidates = pipeline_runs::Entity::find()
                .filter(pipeline_runs::Column::WorkspaceId.eq(row.workspace_id))
                .filter(pipeline_runs::Column::PipelineId.eq(snap.pipeline_id))
                .filter(pipeline_runs::Column::TriggeredBy.eq(snap.user_id))
                .filter(pipeline_runs::Column::CreatedAt.gte(row.created_at))
                .all(&s.db)
                .await?;
            for candidate in candidates {
                let inputs: Value = candidate
                    .inputs
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(json!({}));
                if inputs["review_job_id"] == row.id.to_string() {
                    row.pipeline_run_id = candidate.id;
                    break;
                }
            }
            if row.pipeline_run_id == 0 {
                return Err(bad("launchUncertain"));
            }
        } else {
            jobs::Entity::update_many()
                .col_expr(jobs::Column::State, Expr::value("launching"))
                .filter(jobs::Column::Id.eq(row.id))
                .filter(jobs::Column::LeaseOwner.eq(&row.lease_owner))
                .exec(&s.db)
                .await?;
            fenced(s, &row).await?.commit().await?;
            let body = json!({"branch":snap.config.branch,"commit":snap.head_sha,"review_lease_owner":row.lease_owner,"review_definition":snap.definition,"inputs":{"review_base":snap.base_sha,"review_head":snap.head_sha,"review_full":snap.full_scan.to_string(),"review_job_id":row.id.to_string()}});
            let launch = crate::pipelines::runs::trigger_with_analysis_source(
                s.clone(),
                scope,
                snap.pipeline_id.to_string(),
                bytes::Bytes::from(encode(&body)?),
                "code-review",
                Some((snap.connection_id, snap.url.clone())),
            )
            .await;
            let (_, Json(value)) = match launch {
                Ok(value) => value,
                Err(error)
                    if error
                        .2
                        .as_ref()
                        .is_some_and(|m| m.key == "actions.noEligibleRunner") =>
                {
                    let mut result: Value = decode(&row.result)?;
                    result["waiting_reason"] = json!(crate::i18n::store_message(
                        &crate::i18n::Message::new("reviews.waiting")
                    ));
                    jobs::Entity::update_many()
                        .col_expr(jobs::Column::State, Expr::value("waiting"))
                        .col_expr(jobs::Column::Result, Expr::value(encode(&result)?))
                        .filter(jobs::Column::Id.eq(row.id))
                        .filter(jobs::Column::LeaseOwner.eq(&row.lease_owner))
                        .exec(&s.db)
                        .await?;
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            row.pipeline_run_id = i64::from(
                value["id"]
                    .as_str()
                    .ok_or_else(|| bad("runId"))?
                    .parse::<Id>()
                    .map_err(|_| bad("runId"))?,
            );
        }
        jobs::Entity::update_many()
            .col_expr(
                jobs::Column::PipelineRunId,
                Expr::value(row.pipeline_run_id),
            )
            .col_expr(jobs::Column::State, Expr::value("waiting"))
            .filter(jobs::Column::Id.eq(row.id))
            .filter(jobs::Column::LeaseOwner.eq(&row.lease_owner))
            .exec(&s.db)
            .await?;
    }
    let run = pipeline_runs::Entity::find_by_id(row.pipeline_run_id)
        .filter(pipeline_runs::Column::WorkspaceId.eq(row.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    if run.pipeline_id != snap.pipeline_id
        || run.triggered_by != snap.user_id
        || run.commit != snap.head_sha
    {
        return Err(bad("runContract"));
    }
    let mut result: Value = decode(&row.result)?;
    result["pipeline_state"] = json!(run.state);
    if !["succeeded", "failed", "cancelled", "skipped"].contains(&run.state.as_str()) {
        let state = if run.state == "running" {
            "running"
        } else {
            "waiting"
        };
        result["waiting_reason"] = if state == "waiting" {
            json!(crate::i18n::store_message(&crate::i18n::Message::new(
                "reviews.waiting"
            )))
        } else {
            json!("")
        };
        jobs::Entity::update_many()
            .col_expr(jobs::Column::State, Expr::value(state))
            .col_expr(jobs::Column::Result, Expr::value(encode(&result)?))
            .filter(jobs::Column::Id.eq(row.id))
            .filter(jobs::Column::LeaseOwner.eq(&row.lease_owner))
            .exec(&s.db)
            .await?;
        return Ok(());
    }
    if run.state == "cancelled" || run.state == "skipped" {
        result["error"] = json!(crate::i18n::store_message(&crate::i18n::Message::new(
            "reviews.cancelled"
        )));
        return finish(s, &row, "cancelled", &result).await;
    }
    let artifacts = read_artifacts(s, &row).await?;
    // Tests and AI are independent: an absent/bad JUnit must not suppress AI.
    if snap.config.tests_enabled {
        if snap.config.junit_path.is_empty() {
            let execution = command_test_result(s, &run).await?;
            result["test_state"] = execution["state"].clone();
            result["test_exit_code"] = execution["exit_code"].clone();
            result["test_summary"] = Value::Null;
            result["report_source"] = json!("pipeline");
        } else {
            result["report_source"] = json!("junit");
            match artifacts
                .get(&snap.config.junit_path)
                .ok_or_else(|| bad("junitMissing"))
                .and_then(|b| audit::junit(b))
            {
                Ok(summary) => {
                    result["test_state"] = json!(if summary["failed"].as_u64().unwrap_or(0) > 0 {
                        "failed"
                    } else {
                        "passed"
                    });
                    result["test_summary"] = summary;
                }
                Err(e) => {
                    result["test_state"] = json!("unavailable");
                    result["test_error"] = json!(e.stored());
                }
            }
        }
    }
    if snap.config.ai_enabled {
        let provider = hex::encode(Sha256::digest(snap.config.ai_endpoint.as_bytes()));
        let owner = uuid::Uuid::new_v4().to_string();
        let mut slot = None;
        for index in 0..snap.config.ai_concurrency {
            let name = format!(
                "code-review-ai-slot:{}:{}:{}",
                row.workspace_id, provider, index
            );
            if crate::lock::acquire(
                &s.db,
                &name,
                &owner,
                db::now(),
                (snap.config.ai_timeout_seconds as i64 + 60) * 1000,
            )
            .await?
            {
                slot = Some(name);
                break;
            }
        }
        let Some(slot) = slot else {
            result["waiting_reason"] = json!(crate::i18n::store_message(
                &crate::i18n::Message::new("reviews.waitingAi")
            ));
            jobs::Entity::update_many()
                .col_expr(jobs::Column::State, Expr::value("waiting"))
                .col_expr(jobs::Column::Result, Expr::value(encode(&result)?))
                .filter(jobs::Column::Id.eq(row.id))
                .filter(jobs::Column::LeaseOwner.eq(&row.lease_owner))
                .exec(&s.db)
                .await?;
            return Ok(());
        };
        let audit_result=async {
            let input:audit::Input=serde_json::from_slice(artifacts.get("review-input.json").ok_or_else(||bad("inputMissing"))?).map_err(|_|bad("inputContract"))?;
            audit::validate_input(&input,&snap.config,&snap.base_sha,&snap.head_sha)?;
            result["coverage"]=json!({"scanned":input.files.iter().map(|f|f.file.clone()).collect::<Vec<_>>(),"skipped":input.skipped});
            // Recheck after artifact I/O and immediately before unsealing provider credentials.
            let scope=Scope::for_actor(s,row.workspace_id.into(),snap.user_id.into()).await?;
            scope.require("code-reviews.run")?;authorize_snapshot(s,&scope,&snap).await?;
            let(_,_,secret)=crate::credentials::unseal(s,&scope.workspace_id,&snap.config.ai_credential_id.parse().map_err(|_|bad("modelCredential"))?).await?;
            let mut found=audit::request(&snap.config,&secret,&input).await?;
            for f in &mut found { f.fingerprint=hex::encode(Sha256::digest(format!("{}\0{}\0{}",snap.connection_id,snap.repository,f.fingerprint).as_bytes())); }
            // Revocation while the HTTP request was running also fences persistence.
            let scope=Scope::for_actor(s,row.workspace_id.into(),snap.user_id.into()).await?;
            scope.require("code-reviews.run")?;authorize_snapshot(s,&scope,&snap).await?;
            persist_findings(s,&row,&snap,&input,&found).await?;
            result["findings"]=serde_json::to_value(&found).map_err(|_|bad("modelContract"))?;
            Ok::<_,ApiError>(found.len())
        }.await;
        crate::lock::release(&s.db, &slot, &owner).await?;
        match audit_result {
            Ok(n) => {
                result["ai_state"] = json!(if n == 0 {
                    "completed_clear"
                } else {
                    "completed_findings"
                });
                result["finding_count"] = json!(n);
            }
            Err(e) => {
                result["ai_state"] = json!("failed");
                result["ai_error"] = json!(e.stored());
            }
        }
    }
    let state = if run.state == "failed"
        || result["ai_state"] == "failed"
        || result["test_state"] == "unavailable"
    {
        "failed"
    } else {
        "completed"
    };
    finish(s, &row, state, &result).await
}
pub(super) async fn finish(
    s: &AppState,
    row: &jobs::Model,
    state: &str,
    result: &Value,
) -> Result<()> {
    let tx = fenced(s, row).await?;
    jobs::Entity::update_many()
        .col_expr(jobs::Column::State, Expr::value(state))
        .col_expr(jobs::Column::Result, Expr::value(encode(result)?))
        .col_expr(jobs::Column::FinishedAt, Expr::value(db::now()))
        .filter(jobs::Column::Id.eq(row.id))
        .filter(jobs::Column::LeaseOwner.eq(&row.lease_owner))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
async fn command_test_result(s: &AppState, run: &pipeline_runs::Model) -> Result<Value> {
    let snapshot =
        crate::pipelines::actions::snapshot(&run.definition).ok_or_else(|| bad("runContract"))?;
    let index = snapshot
        .jobs
        .iter()
        .find(|j| j.key == "review")
        .and_then(|j| j.steps.iter().find(|step| step.key == "repository_tests"))
        .map(|s| s.index);
    let Some(index) = index else {
        return Ok(json!({"state":"unavailable","exit_code":null}));
    };
    use crate::entities::pipeline_run_steps as steps;
    let step = steps::Entity::find()
        .filter(steps::Column::RunId.eq(run.id))
        .filter(steps::Column::StepIndex.eq(index))
        .one(&s.db)
        .await?;
    let Some(step) = step else {
        return Ok(json!({"state":"unavailable","exit_code":null}));
    };
    let state = if step.state == "failed" || step.exit_code.is_some_and(|c| c != 0) {
        "failed"
    } else if step.state == "succeeded" {
        "passed"
    } else {
        "unavailable"
    };
    Ok(json!({"state":state,"exit_code":step.exit_code}))
}
async fn read_artifacts(s: &AppState, row: &jobs::Model) -> Result<BTreeMap<String, Vec<u8>>> {
    let snap: Snapshot = decode(&row.snapshot)?;
    let rows = pipeline_artifacts::Entity::find()
        .filter(pipeline_artifacts::Column::WorkspaceId.eq(row.workspace_id))
        .filter(pipeline_artifacts::Column::RunId.eq(row.pipeline_run_id))
        .all(&s.db)
        .await?;
    let mut out = BTreeMap::new();
    let mut total = 0usize;
    for artifact in rows {
        let archive_kind = artifact.path.rsplit('/').nth(1).unwrap_or("");
        if artifact.path != "review-input.json"
            && artifact.path != snap.config.junit_path
            && !["review-input", "review-junit"].contains(&archive_kind)
        {
            continue;
        }
        if artifact.pipeline_id != snap.pipeline_id {
            return Err(bad("artifactContract"));
        }
        if artifact.size < 0 || artifact.size > 32 * 1024 * 1024 {
            return Err(bad("artifactLimit"));
        }
        let bytes = tokio::fs::read(crate::pipelines::artifacts::file_path(s, &artifact)).await?;
        if bytes.len() as i64 != artifact.size
            || hex::encode(Sha256::digest(&bytes)) != artifact.sha256
        {
            return Err(bad("artifactIntegrity"));
        }
        if artifact.path.ends_with(".zip") {
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
                .map_err(|_| bad("artifactArchive"))?;
            if zip.len() > 1000 {
                return Err(bad("artifactLimit"));
            }
            for n in 0..zip.len() {
                let mut file = zip.by_index(n).map_err(|_| bad("artifactArchive"))?;
                if file.is_dir() {
                    continue;
                }
                let mut name = file
                    .name()
                    .map_err(|_| bad("artifactArchive"))?
                    .into_owned();
                crate::pipelines::artifacts::validate_paths(&[name.clone()])?;
                // Native upload-artifact strips the common parent of a single
                // selected file. Restore the configured JUnit logical path.
                if archive_kind == "review-junit"
                    && name == snap.config.junit_path.rsplit('/').next().unwrap_or("")
                {
                    name = snap.config.junit_path.clone();
                }
                if file.size() > 16 * 1024 * 1024 {
                    return Err(bad("artifactLimit"));
                }
                let mut b = Vec::new();
                std::io::Read::by_ref(&mut file)
                    .take(16 * 1024 * 1024 + 1)
                    .read_to_end(&mut b)?;
                total += b.len();
                if total > 32 * 1024 * 1024 || out.insert(name, b).is_some() {
                    return Err(bad("artifactContract"));
                }
            }
        } else {
            total += bytes.len();
            if total > 32 * 1024 * 1024 || out.insert(artifact.path, bytes).is_some() {
                return Err(bad("artifactContract"));
            }
        }
    }
    Ok(out)
}

pub(super) async fn persist_findings(
    s: &AppState,
    row: &jobs::Model,
    snap: &Snapshot,
    input: &audit::Input,
    found: &[audit::Finding],
) -> Result<()> {
    let tx = fenced(s, row).await?;
    let now = db::now();
    // A slow older run retains its own report but cannot overwrite lifecycle
    // decisions already established by a newer completed run of this source.
    let newer = jobs::Entity::find()
        .filter(jobs::Column::ReviewId.eq(row.review_id))
        .filter(jobs::Column::Id.gt(row.id))
        .all(&tx)
        .await?;
    for newer in newer {
        let source: Snapshot = decode(&newer.snapshot)?;
        let outcome: Value = decode(&newer.result)?;
        if source.connection_id == snap.connection_id
            && source.repository == snap.repository
            && (outcome["audit_persisted"] == true
                || matches!(
                    outcome["ai_state"].as_str(),
                    Some("completed_clear" | "completed_findings")
                ))
        {
            tx.commit().await?;
            return Ok(());
        }
    }
    let old = findings::Entity::find()
        .filter(findings::Column::WorkspaceId.eq(row.workspace_id))
        .filter(findings::Column::ReviewId.eq(row.review_id))
        .filter(findings::Column::ConnectionId.eq(snap.connection_id))
        .filter(findings::Column::Repository.eq(&snap.repository))
        .all(&tx)
        .await?;
    let present: BTreeSet<_> = found.iter().map(|f| f.fingerprint.as_str()).collect();
    for f in found {
        let previous = old.iter().find(|r| r.fingerprint == f.fingerprint);
        if let Some(previous) = previous {
            let mut updated = previous.clone();
            updated.detail = encode(f)?;
            updated.last_seen = now;
            updated.version += 1;
            let mut seen: Vec<Value> = decode(&updated.history)?;
            seen.push(json!({"event":"seen","at":now,"run_id":Id::from(row.id)}));
            updated.history = encode(&seen)?;
            if updated.status == "fixed" {
                updated.status = "open".into();
                let mut h: Vec<Value> = decode(&updated.history)?;
                h.push(json!({"from":"fixed","to":"open","at":now,"run_id":Id::from(row.id)}));
                updated.history = encode(&h)?;
            }
            let n = findings::Entity::update_many()
                .col_expr(findings::Column::Detail, Expr::value(updated.detail))
                .col_expr(findings::Column::LastSeen, Expr::value(updated.last_seen))
                .col_expr(findings::Column::Version, Expr::value(updated.version))
                .col_expr(findings::Column::Status, Expr::value(updated.status))
                .col_expr(findings::Column::History, Expr::value(updated.history))
                .filter(findings::Column::Id.eq(previous.id))
                .filter(findings::Column::Version.eq(previous.version))
                .exec(&tx)
                .await?
                .rows_affected;
            if n != 1 {
                return Err(conflict());
            }
        } else {
            findings::Entity::insert(
                findings::Model {
                    id: s.next_id()?.into(),
                    workspace_id: row.workspace_id,
                    review_id: row.review_id,
                    connection_id: snap.connection_id,
                    repository: snap.repository.clone(),
                    repository_url: snap.url.clone(),
                    fingerprint: f.fingerprint.clone(),
                    file: f.file.clone(),
                    detail: encode(f)?,
                    status: "open".into(),
                    history: encode(&vec![
                        json!({"to":"open","at":now,"run_id":Id::from(row.id)}),
                    ])?,
                    first_seen: now,
                    last_seen: now,
                    version: 1,
                }
                .into_active_model(),
            )
            .exec(&tx)
            .await?;
        }
    }
    for previous in old {
        // Never infer a fix for excluded, binary, over-budget or otherwise unscanned files.
        if previous.status != "open"
            || present.contains(previous.fingerprint.as_str())
            || !input.files.iter().any(|f| f.file == previous.file)
        {
            continue;
        }
        let mut h: Vec<Value> = decode(&previous.history)?;
        h.push(json!({"from":"open","to":"fixed","at":now,"run_id":Id::from(row.id)}));
        let n = findings::Entity::update_many()
            .col_expr(findings::Column::Status, Expr::value("fixed"))
            .col_expr(findings::Column::History, Expr::value(encode(&h)?))
            .col_expr(findings::Column::Version, Expr::value(previous.version + 1))
            .filter(findings::Column::Id.eq(previous.id))
            .filter(findings::Column::Version.eq(previous.version))
            .exec(&tx)
            .await?
            .rows_affected;
        if n != 1 {
            return Err(conflict());
        }
    }
    let mut outcome: Value = decode(&row.result)?;
    outcome["audit_persisted"] = json!(true);
    jobs::Entity::update_many()
        .col_expr(jobs::Column::Result, Expr::value(encode(&outcome)?))
        .filter(jobs::Column::Id.eq(row.id))
        .filter(jobs::Column::LeaseOwner.eq(&row.lease_owner))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn poll_events(s: &AppState) -> Result<()> {
    for row in defs::Entity::find().all(&s.db).await? {
        let c: Config = decode(&row.config)?;
        if !c.enabled || c.trigger != "push" {
            continue;
        }
        let name = format!("code-review-events:{}", row.id);
        let owner = format!("{}:{}", s.instance_id, uuid::Uuid::new_v4());
        if !crate::lock::acquire(&s.db, &name, &owner, db::now(), 60_000).await? {
            continue;
        }
        let result = {
            let renewal = async {
                let mut interval = tokio::time::interval(Duration::from_secs(15));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    if crate::lock::renew(&s.db, &name, &owner, db::now(), 60000).await? != 1 {
                        return Err::<(), ApiError>(bad("eventLeaseExpired"));
                    }
                }
            };
            tokio::select! {result=poll_definition(s,&row,&c,&owner)=>result,result=renewal=>{result?;Err(bad("eventLeaseExpired"))}}
        };
        if let Err(e) = result {
            let tx = crate::variables::transaction(s, row.workspace_id.into()).await?;
            let current_lock = crate::entities::locks::Entity::find_by_id(name.clone())
                .lock_exclusive()
                .one(&tx)
                .await?;
            if !current_lock.is_some_and(|l| l.owner == owner && l.expires_at > db::now()) {
                continue;
            }
            let mut baseline: BTreeMap<String, String> = decode(&row.baselines)?;
            baseline.insert("_error".into(), e.stored());
            defs::Entity::update_many()
                .col_expr(defs::Column::Baselines, Expr::value(encode(&baseline)?))
                .filter(defs::Column::Id.eq(row.id))
                .filter(defs::Column::Version.eq(row.version))
                .exec(&tx)
                .await?;
            tx.commit().await?;
            tracing::warn!(review_id=row.id,error=%e,"code review trigger failed");
        }
        crate::lock::release(&s.db, &name, &owner).await?;
    }
    Ok(())
}
pub(super) async fn poll_definition(
    s: &AppState,
    row: &defs::Model,
    c: &Config,
    owner: &str,
) -> Result<()> {
    let scope = Scope::for_actor(s, row.workspace_id.into(), row.user_id.into()).await?;
    scope.require("code-reviews.run")?;
    scope.require("pipelines.run")?;
    authorize_config(s, &scope, c).await?;
    let mut baselines: BTreeMap<String, String> = decode(&row.baselines)?;
    let resolved = sources(s, &scope, c).await?;
    let mut platform_refs = BTreeMap::new();
    for (connection, path, url) in &resolved {
        let conn = crate::repositories::auth::owned_connection(&s.db, &scope, (*connection).into())
            .await?;
        let refs = if conn.provider == "git" {
            observe_git(s, &scope, c, &conn, path).await?
        } else {
            if !platform_refs.contains_key(connection) {
                let paths = resolved
                    .iter()
                    .filter(|(id, _, _)| id == connection)
                    .map(|(_, path, _)| path.clone())
                    .collect::<Vec<_>>();
                platform_refs.insert(
                    *connection,
                    crate::repositories::monitor::review_refs(s, &conn, "", &paths).await?,
                );
            }
            platform_refs
                .get(connection)
                .and_then(|repos| repos.get(path))
                .cloned()
                .ok_or_else(|| bad("branchMissing"))?
        };
        let head = refs
            .get(&c.branch)
            .filter(|h| valid_sha(h))
            .ok_or_else(|| bad("branchMissing"))?;
        apply_head(s, row, c, *connection, path, url, head, &mut baselines).await?;
    }
    let events = repository_events::Entity::find()
        .filter(repository_events::Column::WorkspaceId.eq(row.workspace_id))
        .filter(repository_events::Column::Event.eq("push"))
        .filter(repository_events::Column::CreatedAt.gte(row.cursor))
        .order_by_asc(repository_events::Column::CreatedAt)
        .limit(500)
        .all(&s.db)
        .await?;
    let mut cursor = row.cursor;
    let mut selected = Some(resolved);
    for event in events {
        cursor = std::cmp::max(cursor, event.created_at);
        if event.reference.trim_start_matches("refs/heads/") != c.branch
            || !c.repositories.iter().any(|r| {
                r.connection_id == event.connection_id.to_string()
                    && matches_selection(r, &event.repository)
            })
        {
            continue;
        }
        let detail: Value = event
            .detail
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(json!({}));
        let (base, head) = (
            detail["before"].as_str().unwrap_or(""),
            detail["after"].as_str().unwrap_or(""),
        );
        // Baseline establishment and deleted refs are observations, never historical replay.
        if !valid_sha(base) || !valid_sha(head) || base == head {
            continue;
        }
        if selected.is_none() {
            selected = Some(sources(s, &scope, c).await?);
        }
        let url = selected
            .as_ref()
            .unwrap()
            .iter()
            .find(|(conn, path, _)| *conn == event.connection_id && path == &event.repository)
            .map(|(_, _, url)| url.clone())
            .ok_or_else(|| bad("selectionRevoked"))?;
        enqueue(
            s,
            row,
            Snapshot {
                config: c.clone(),
                config_version: row.version,
                connection_id: event.connection_id,
                repository: event.repository,
                url,
                base_sha: base.into(),
                head_sha: head.into(),
                full_scan: false,
                pipeline_id: row.pipeline_id,
                user_id: row.user_id,
                definition: String::new(),
            },
        )
        .await?;
    }
    baselines.remove("_error");
    let tx = crate::variables::transaction(s, row.workspace_id.into()).await?;
    let fence =
        crate::entities::locks::Entity::find_by_id(format!("code-review-events:{}", row.id))
            .lock_exclusive()
            .one(&tx)
            .await?
            .ok_or_else(|| bad("eventLeaseExpired"))?;
    if fence.owner != owner || fence.expires_at <= db::now() {
        return Err(bad("eventLeaseExpired"));
    }
    defs::Entity::update_many()
        .col_expr(defs::Column::Cursor, Expr::value(cursor))
        .col_expr(defs::Column::Baselines, Expr::value(encode(&baselines)?))
        .filter(defs::Column::Id.eq(row.id))
        .filter(defs::Column::Version.eq(row.version))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn apply_head(
    s: &AppState,
    row: &defs::Model,
    c: &Config,
    connection: i64,
    path: &str,
    url: &str,
    head: &str,
    baselines: &mut BTreeMap<String, String>,
) -> Result<()> {
    if !valid_sha(head) {
        return Err(bad("shaRequired"));
    }
    let key = format!("{connection}:{path}");
    if let Some(base) = baselines.get(&key).filter(|b| valid_sha(b) && *b != head) {
        enqueue(
            s,
            row,
            Snapshot {
                config: c.clone(),
                config_version: row.version,
                connection_id: connection,
                repository: path.into(),
                url: url.into(),
                base_sha: base.clone(),
                head_sha: head.into(),
                full_scan: false,
                pipeline_id: row.pipeline_id,
                user_id: row.user_id,
                definition: String::new(),
            },
        )
        .await?;
    }
    // The first successful observation establishes a baseline without replay.
    baselines.insert(key, head.into());
    Ok(())
}

async fn observe_git(
    s: &AppState,
    scope: &Scope,
    c: &Config,
    connection: &crate::entities::repository_connections::Model,
    path: &str,
) -> Result<BTreeMap<String, String>> {
    let reference = Id::reference(&c.runner_ref)?;
    let candidates = crate::hosts::execution::candidates(
        &s.db,
        i64::from(scope.workspace_id),
        if c.runner_kind == "host" {
            reference.into()
        } else {
            0
        },
        if c.runner_kind == "host-group" {
            reference.into()
        } else {
            0
        },
        c.runner_kind == "pool",
        &c.runner_labels,
    )
    .await?;
    for (host, policy, _) in candidates {
        if host.connection != "runner" || host.state != "online" {
            continue;
        }
        let tx = crate::variables::transaction(s, scope.workspace_id).await?;
        let lease = crate::hosts::execution::try_acquire(
            &tx,
            host.id,
            policy.capacity,
            0,
            "code-review-ref-observation",
        )
        .await?;
        tx.commit().await?;
        let Some(lease) = lease else {
            continue;
        };
        let result = {
            let runner_id = host.id.to_string();
            let paths = [path.to_owned()];
            let observe =
                crate::repositories::monitor::review_refs(s, connection, &runner_id, &paths);
            let renewal = async {
                let mut interval = tokio::time::interval(Duration::from_secs(10));
                loop {
                    interval.tick().await;
                    if !lease.renew(&s.db).await? {
                        return Err::<(), ApiError>(bad("runnerLeaseExpired"));
                    }
                }
            };
            tokio::pin!(observe);
            tokio::pin!(renewal);
            tokio::select! {result=&mut observe=>result,result=&mut renewal=>{result?;Err(bad("runnerLeaseExpired"))}}
        };
        lease.release(&s.db).await?;
        return Ok(result?.remove(path).unwrap_or_default());
    }
    Err(ApiError::localized(
        StatusCode::CONFLICT,
        crate::i18n::Message::new("reviews.waiting"),
    ))
}
