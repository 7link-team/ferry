//! Replacing one blueprint instance on another host: the old host is fenced, the binding
//! moves the instance (and its colocated peers), and a new revision rebuilds the placement.
use super::events::{self, Kind, NewEvent};
use super::*;
use crate::blueprint::{
    binding::{self, Binding},
    failover::{self, FailoverClass},
    model::{self, Blueprint},
    placement,
    spec::{self, DeploySpec},
};
use crate::entities::deployment_fences as fences;
use sea_orm::ConnectionTrait;
use std::collections::BTreeSet;

/// Revision actions of a replacement; rollbacks never cross one, whatever its outcome.
pub(crate) const ACTIONS: [&str; 2] = ["auto-replace", "manual-replace"];
/// How long stopping one fenced instance may take.
const STOP_TIMEOUT: Duration = Duration::from_secs(60);

pub(crate) enum ReplaceTrigger {
    /// The reconciler confirmed `failed_host` down.
    #[allow(dead_code)] // built by the failover reconciler (P3 Task 6)
    Auto {
        failed_host: Id,
    },
    Manual,
}

impl ReplaceTrigger {
    pub fn action(&self) -> &'static str {
        match self {
            ReplaceTrigger::Auto { .. } => ACTIONS[0],
            ReplaceTrigger::Manual => ACTIONS[1],
        }
    }
    fn message(&self) -> Message {
        Message::new(match self {
            ReplaceTrigger::Auto { .. } => "deployment.replaceTrigger.auto",
            ReplaceTrigger::Manual => "deployment.replaceTrigger.manual",
        })
    }
}

fn refuse(status: StatusCode, key: &str) -> ApiError {
    ApiError::localized(status, Message::new(key))
}

/// A deployment instance that may be replaced, as read before anything changes.
pub(super) struct Checked {
    app: apps::Model,
    bp: Blueprint,
    spec: Option<DeploySpec>,
    binding: Binding,
    /// The host the instance runs on now.
    from: Id,
    /// The instance and its colocated peers (same index), all on `from`.
    instances: Vec<String>,
}

/// Blueprint, driver `native`/`docker`, a replaceable class for every moved role, a current
/// revision, and no revision queued, running or awaiting review.
pub(super) async fn check(
    conn: &impl ConnectionTrait,
    scope: &Scope,
    deployment: Id,
    role: &str,
    index: u32,
) -> Result<Checked> {
    scope.require("deployments.apply")?;
    let app = scoped(conn, scope, &deployment.to_string()).await?;
    if app.target_kind != "blueprint" {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            "deployment.replaceNotBlueprint",
        ));
    }
    if !matches!(app.mode.as_str(), "native" | "docker") {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            "deployment.replaceUnsupportedDriver",
        ));
    }
    let bp = model::parse(&app.manifest)?;
    let Some(wanted) = bp.roles.iter().find(|r| r.name == role) else {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("blueprint.bindingRoleUnknown").with("role", role),
        ));
    };
    let group = placement::colocated(&bp, wanted);
    if group
        .iter()
        .any(|r| failover::class_of(r) == FailoverClass::Manual)
    {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            "deployment.replaceNeedsBackup",
        ));
    }
    if app.current_revision == 0 {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            "deployment.replaceNotDeployed",
        ));
    }
    if matches!(app.state.as_str(), "queued" | "running" | "needs-review") {
        return Err(refuse(StatusCode::CONFLICT, "deployment.error.044"));
    }
    let binding = binding::parse(app.blueprint_binding.as_deref().unwrap_or_default())?;
    let Some(from) = binding
        .roles
        .get(role)
        .and_then(|list| list.get(index as usize))
        .map(|i| i.host_id)
    else {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("blueprint.replaceInstanceUnknown")
                .with("role", role)
                .with("instance", format!("{role}-{index}")),
        ));
    };
    let instances = group
        .iter()
        .map(|r| format!("{}-{index}", r.name))
        .collect();
    let spec = app.deploy_spec.as_deref().map(spec::parse).transpose()?;
    Ok(Checked {
        app,
        bp,
        spec,
        binding,
        from,
        instances,
    })
}

/// Why a replacement stopped before any revision was queued.
pub(super) struct Stopped {
    pub error: ApiError,
    /// The new binding was already stored (the old host may have been fenced).
    pub binding_moved: bool,
}

/// Validates (see [`check`]), places the instance anew with [`placement::replace`] (never on
/// the failed host or a host still fenced for this deployment), then claims it: the new
/// binding (mirrored into the spec's manual placement, if the spec has one for a moved role)
/// is stored under the record's version. Only a successful claim fences the old host, so a
/// lost race never stops anything; then the replacement revision is previewed and queued.
#[allow(dead_code)] // called by the replacement sequence and reconciler (P3 Tasks 5-6)
pub(crate) async fn replace_instance(
    s: &AppState,
    scope: &Scope,
    deployment: Id,
    role: &str,
    index: u32,
    trigger: ReplaceTrigger,
) -> Result<Id> {
    let checked = check(&s.db, scope, deployment, role, index).await?;
    replace_checked(s, scope, checked, role, index, trigger, None)
        .await
        .map_err(|stopped| stopped.error)
}

/// [`replace_instance`] after [`check`]; with `expected`, the record must still be at that
/// version.
pub(super) async fn replace_checked(
    s: &AppState,
    scope: &Scope,
    checked: Checked,
    role: &str,
    index: u32,
    trigger: ReplaceTrigger,
    expected: Option<i64>,
) -> std::result::Result<Id, Stopped> {
    let before = |error| Stopped {
        error,
        binding_moved: false,
    };
    let after = |error| Stopped {
        error,
        binding_moved: true,
    };
    let Checked {
        app,
        bp,
        mut spec,
        binding,
        from,
        instances,
    } = checked;
    let deployment = Id::from(app.id);
    if expected.is_some_and(|v| v != app.version) {
        return Err(before(refuse(StatusCode::CONFLICT, "deployment.error.030")));
    }
    let candidates = candidates::load(&s.db, scope.workspace_id)
        .await
        .map_err(before)?;
    let mut excluded = fenced_hosts(&s.db, scope.workspace_id, deployment)
        .await
        .map_err(before)?;
    if let ReplaceTrigger::Auto { failed_host } = trigger {
        excluded.insert(failed_host);
    }
    let moved = placement::replace(
        &bp,
        spec.as_ref(),
        &binding,
        role,
        index as usize,
        &candidates,
        &excluded,
    )
    .map_err(before)?;
    let mut spec_text = app.deploy_spec.clone();
    if let Some(spec) = spec.as_mut()
        && let Some(manual) = spec.placement.as_mut()
    {
        // Keeps the stored spec and the binding the same topology, so a later spec edit naming
        // the new host is not mistaken for a move.
        let mut mirrored = false;
        for (name, list) in manual.iter_mut() {
            let bound = moved.roles.get(name).and_then(|l| l.get(index as usize));
            if let (Some(slot), Some(bound)) = (list.get_mut(index as usize), bound)
                && slot != bound
            {
                *slot = bound.clone();
                mirrored = true;
            }
        }
        if mirrored {
            spec_text = Some(spec::to_yaml(spec));
        }
    }
    let text = serde_json::to_string(&moved).map_err(|_| before(super::blueprint_invalid()))?;
    claim(s, &app, text, spec_text, from, &instances)
        .await
        .map_err(before)?;
    fence(s, &app, from, &instances).await.map_err(after)?;
    scheduled_replace(s, scope.clone(), &deployment.to_string(), trigger.action())
        .await
        .map_err(after)
}

/// Stores the moved binding and spec, guarded by the version and state `app` was read at.
async fn claim(
    s: &AppState,
    app: &apps::Model,
    binding: String,
    spec: Option<String>,
    from: Id,
    instances: &[String],
) -> Result<()> {
    let tx = s.db.begin().await?;
    let n = apps::Entity::update_many()
        .col_expr(apps::Column::BlueprintBinding, Expr::value(binding))
        .col_expr(apps::Column::DeploySpec, Expr::value(spec))
        .col_expr(apps::Column::PreviewArtifact, Expr::value(""))
        .col_expr(
            apps::Column::Version,
            Expr::col(apps::Column::Version).add(1),
        )
        .col_expr(apps::Column::UpdatedAt, Expr::value(db::now()))
        .filter(apps::Column::Id.eq(app.id))
        .filter(apps::Column::WorkspaceId.eq(app.workspace_id))
        .filter(apps::Column::Version.eq(app.version))
        .filter(apps::Column::State.is_not_in(["queued", "running", "needs-review"]))
        .exec(&tx)
        .await?
        .rows_affected;
    if n == 0 {
        return Err(refuse(StatusCode::CONFLICT, "deployment.error.030"));
    }
    // A crash after this commit must leave the old instances fenced, even when no
    // replacement revision could be previewed. Binding and fences are one durable change.
    reserve_fences(s, &tx, app, from, instances).await?;
    let boundary = revisions::Entity::find()
        .filter(revisions::Column::WorkspaceId.eq(app.workspace_id))
        .filter(revisions::Column::DeploymentId.eq(app.id))
        .order_by_desc(revisions::Column::Number)
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    for instance in instances {
        events::record(
            s,
            &tx,
            NewEvent {
                workspace: app.workspace_id.into(),
                deployment: app.id.into(),
                kind: Kind::Fenced,
                instance: instance.clone(),
                host: from,
                // The last revision before the placement changed, not a wall-clock ordering.
                revision: boundary.id.into(),
                message: Message::new("deployment.event.fenced")
                    .with("host", host_name(&tx, app.workspace_id, from).await?)
                    .with("instance", instance),
                data: json!({"pending": true, "stopped": false}),
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn reserve_fences(
    s: &AppState,
    conn: &impl ConnectionTrait,
    app: &apps::Model,
    host: Id,
    instances: &[String],
) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    for instance in instances {
        let existing = fences::Entity::find()
            .filter(fences::Column::WorkspaceId.eq(app.workspace_id))
            .filter(fences::Column::DeploymentId.eq(app.id))
            .filter(fences::Column::HostId.eq(host))
            .filter(fences::Column::Instance.eq(instance))
            .filter(fences::Column::ClearedAt.eq(0))
            .one(conn)
            .await?;
        let id = if let Some(row) = existing {
            row.id
        } else {
            let id = i64::from(s.next_id()?);
            fences::Entity::insert(fences::ActiveModel {
                id: sea_orm::Set(id),
                workspace_id: sea_orm::Set(app.workspace_id),
                deployment_id: sea_orm::Set(app.id),
                host_id: sea_orm::Set(host.into()),
                instance: sea_orm::Set(instance.clone()),
                created_at: sea_orm::Set(db::now()),
                cleared_at: sea_orm::Set(0),
            })
            .exec(conn)
            .await?;
            id
        };
        ids.push(id);
    }
    Ok(ids)
}

/// The command that stops `instance` of `app` on its host, refusing anything this deployment
/// does not own. An instance that is already gone counts as stopped.
pub(crate) fn stop_command(app: &apps::Model, instance: &str) -> String {
    let prefix = project(app);
    let owner = format!("workspace.deployment={prefix}");
    if app.mode == "native" {
        let unit = quote(&format!("{prefix}-{instance}.service"));
        return format!(
            "found=$(systemctl show -p FragmentPath --value {unit} 2>/dev/null || :); if [ -n \"$found\" ] && [ -e \"$found\" ]; then [ \"$(head -n 1 \"$found\")\" = {marker} ] || exit 75; systemctl disable --now {unit}; fi",
            marker = quote(&format!("# {owner}")),
        );
    }
    let container = quote(&format!("{prefix}-{instance}"));
    // A daemon that does not answer must not read as "container already gone".
    format!(
        "docker info >/dev/null 2>&1 || exit 1; if docker container inspect {container} >/dev/null 2>&1; then owner=$(docker inspect --format '{{{{index .Config.Labels \"workspace.deployment\"}}}}' {container}); [ \"$owner\" = {prefix} ] || exit 75; docker rm -f {container}; fi",
        prefix = quote(&prefix),
    )
}

async fn host_name(conn: &impl ConnectionTrait, workspace: i64, host: Id) -> Result<String> {
    Ok(hosts::Entity::find_by_id(i64::from(host))
        .filter(hosts::Column::WorkspaceId.eq(workspace))
        .one(conn)
        .await?
        .map_or_else(|| host.to_string(), |h| h.name))
}

/// Fences `instances` of `app` on `host`. When the host can be reached its instances are
/// stopped; pending fences are stored before any remote command and cleared only after
/// successful stops. Each instance records a `fenced` event (`detail.stopped`).
pub(crate) async fn fence(
    s: &AppState,
    app: &apps::Model,
    host: Id,
    instances: &[String],
) -> Result<()> {
    let workspace = Id::from(app.workspace_id);
    let tx = s.db.begin().await?;
    let ids = reserve_fences(s, &tx, app, host, instances).await?;
    tx.commit().await?;
    // Command output is never kept: only whether every stop succeeded.
    let stopped = async {
        let channel = remote::open(s, &workspace, &host).await?;
        for instance in instances {
            let out = channel
                .exec(&stop_command(app, instance), STOP_TIMEOUT)
                .await?;
            if out.code != 0 {
                return Ok(false);
            }
        }
        Ok::<_, ApiError>(true)
    }
    .await
    .unwrap_or(false);
    let name = host_name(&s.db, app.workspace_id, host).await?;
    let now = db::now();
    let tx = s.db.begin().await?;
    if stopped {
        fences::Entity::update_many()
            .col_expr(fences::Column::ClearedAt, Expr::value(now))
            .filter(fences::Column::Id.is_in(ids))
            .filter(fences::Column::WorkspaceId.eq(app.workspace_id))
            .filter(fences::Column::DeploymentId.eq(app.id))
            .exec(&tx)
            .await?;
    }
    for instance in instances {
        events::record(
            s,
            &tx,
            NewEvent {
                workspace,
                deployment: app.id.into(),
                kind: Kind::Fenced,
                instance: instance.clone(),
                host,
                revision: Id::from(0),
                message: Message::new("deployment.event.fenced")
                    .with("host", &name)
                    .with("instance", instance),
                data: json!({"stopped": stopped}),
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Hosts with an uncleared fence for this deployment: never candidates for it.
pub(crate) async fn fenced_hosts(
    conn: &impl ConnectionTrait,
    workspace: Id,
    deployment: Id,
) -> Result<BTreeSet<Id>> {
    Ok(fences::Entity::find()
        .filter(fences::Column::WorkspaceId.eq(workspace))
        .filter(fences::Column::DeploymentId.eq(deployment))
        .filter(fences::Column::ClearedAt.eq(0))
        .all(conn)
        .await?
        .into_iter()
        .map(|f| f.host_id.into())
        .collect())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplaceBody {
    version: i64,
}

/// `POST /deployments/{id}/instances/{role}/{index}/replace`: moves one instance to another
/// host as a `manual-replace` revision. Answers 202 with the revision; the revision runs
/// like any other.
pub async fn replace(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, role, index)): Path<(String, String, String, u32)>,
    Json(b): Json<ReplaceBody>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("deployments.apply")?;
    let deployment = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    let checked = check(&s.db, &scope, deployment, &role, index).await?;
    if checked.app.version != b.version {
        return Err(refuse(StatusCode::CONFLICT, "deployment.error.030"));
    }
    let trigger = ReplaceTrigger::Manual;
    let instance = format!("{role}-{index}");
    let old_host = checked.from;
    let event = |kind, message: Message, data: Value| NewEvent {
        workspace: scope.workspace_id,
        deployment,
        kind,
        instance: instance.clone(),
        host: old_host,
        revision: Id::from(0),
        message,
        data,
    };
    let from = host_name(&s.db, checked.app.workspace_id, checked.from).await?;
    events::record(
        &s,
        &s.db,
        event(
            Kind::ReplaceStarted,
            Message::new("deployment.event.replaceStarted")
                .with("instance", &instance)
                .with("from", &from)
                .with_message("trigger", trigger.message()),
            json!({"trigger": trigger.action()}),
        ),
    )
    .await?;
    match replace_checked(&s, &scope, checked, &role, index, trigger, Some(b.version)).await {
        Ok(revision) => Ok((StatusCode::ACCEPTED, Json(json!({"id": revision})))),
        Err(Stopped {
            error,
            binding_moved,
        }) => {
            // No revision ran: the request lost a race, or it stopped part way.
            let reason = match error.2.as_ref() {
                Some(m) if m.key == "blueprint.replaceNoCandidate" => "noCandidate",
                _ if error.0 == StatusCode::CONFLICT => "conflict",
                _ => "interrupted",
            };
            let mut data = json!({"reason": reason});
            if binding_moved {
                data["bindingMoved"] = json!(true);
            }
            events::record(
                &s,
                &s.db,
                event(
                    Kind::ReplaceFailed,
                    Message::new("deployment.event.replaceFailed")
                        .with("instance", &instance)
                        .with_message(
                            "reason",
                            Message::new(format!("deployment.failoverReason.{reason}")),
                        ),
                    data,
                ),
            )
            .await?;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::tests::admin;
    use crate::deployments::blueprint_tests::{body, create_app, fixture, row, run_first};

    #[tokio::test]
    async fn claim_persists_fences_before_any_remote_work() {
        let (_dir, s) = fixture(&[2131, 2132, 2133]).await;
        let id = create_app(&s, body(&[2131, 2132])).await.unwrap();
        assert!(run_first(&s, &id, &[]).await);
        let before = row(&s, &id).await;
        let mut binding = binding::parse(before.blueprint_binding.as_deref().unwrap()).unwrap();
        binding.roles.get_mut("nats").unwrap()[0].host_id = Id::from(2133);
        let text = serde_json::to_string(&binding).unwrap();
        crate::hosts::fake::get(2131)
            .commands
            .lock()
            .unwrap()
            .clear();
        claim(
            &s,
            &before,
            text.clone(),
            before.deploy_spec.clone(),
            Id::from(2131),
            &["nats-0".into()],
        )
        .await
        .unwrap();
        // Stop here, as if the process exited before fencing or preview could run.
        assert!(
            crate::hosts::fake::get(2131)
                .commands
                .lock()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            row(&s, &id).await.blueprint_binding.as_deref(),
            Some(text.as_str())
        );
        let pending = fenced_hosts(&s.db, Id::from(1), Id::from(before.id))
            .await
            .unwrap();
        assert_eq!(pending, BTreeSet::from([Id::from(2131)]));
        // A different server's clock/worker ID may sort the decision before the old
        // revision; the reference to its revision number remains authoritative.
        use crate::entities::deployment_events as timeline;
        let decision = timeline::Entity::find()
            .filter(timeline::Column::DeploymentId.eq(before.id))
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(decision.revision_id, before.current_revision);
        timeline::Entity::update_many()
            .col_expr(timeline::Column::Id, Expr::value(1_i64))
            .col_expr(timeline::Column::CreatedAt, Expr::value(0_i64))
            .filter(timeline::Column::Id.eq(decision.id))
            .exec(&s.db)
            .await
            .unwrap();
        let old = Id::from(before.current_revision).to_string();
        let error = old_revision(&s.db, &admin(1), before.id, &old)
            .await
            .unwrap_err();
        assert_eq!(error.2.unwrap().key, "deployment.rollbackAcrossReplace");
        // Failing the claim cannot create a second fence or move the binding again.
        assert!(
            claim(
                &s,
                &before,
                text,
                before.deploy_spec.clone(),
                Id::from(2132),
                &["nats-1".into()]
            )
            .await
            .is_err()
        );
        assert_eq!(
            fenced_hosts(&s.db, Id::from(1), Id::from(before.id))
                .await
                .unwrap(),
            pending
        );
    }
}
