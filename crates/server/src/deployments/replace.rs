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

/// Revision actions of a replacement; rollbacks never cross a succeeded one.
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
struct Checked {
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
async fn check(
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

/// Validates (see [`check`]), places the instance anew with [`placement::replace`] (never on
/// the failed host or a host still fenced for this deployment), fences the old host, stores
/// the new binding (mirrored into the spec's manual placement, if the spec has one for a
/// moved role) under the record's version, and queues the replacement revision.
#[allow(dead_code)] // called by the replacement sequence and reconciler (P3 Tasks 5-6)
pub(crate) async fn replace_instance(
    s: &AppState,
    scope: &Scope,
    deployment: Id,
    role: &str,
    index: u32,
    trigger: ReplaceTrigger,
) -> Result<Id> {
    replace_at(s, scope, deployment, role, index, trigger, None).await
}

/// [`replace_instance`]; with `expected`, the record must still be at that version.
async fn replace_at(
    s: &AppState,
    scope: &Scope,
    deployment: Id,
    role: &str,
    index: u32,
    trigger: ReplaceTrigger,
    expected: Option<i64>,
) -> Result<Id> {
    let Checked {
        app,
        bp,
        mut spec,
        binding,
        from,
        instances,
    } = check(&s.db, scope, deployment, role, index).await?;
    if expected.is_some_and(|v| v != app.version) {
        return Err(refuse(StatusCode::CONFLICT, "deployment.error.030"));
    }
    let candidates = candidates::load(&s.db, scope.workspace_id).await?;
    let mut excluded = fenced_hosts(&s.db, scope.workspace_id, deployment).await?;
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
    )?;
    fence(s, &app, from, &instances).await?;
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
    let text = serde_json::to_string(&moved).map_err(|_| super::blueprint_invalid())?;
    let tx = s.db.begin().await?;
    let n = apps::Entity::update_many()
        .col_expr(apps::Column::BlueprintBinding, Expr::value(text))
        .col_expr(apps::Column::DeploySpec, Expr::value(spec_text))
        .col_expr(apps::Column::PreviewArtifact, Expr::value(""))
        .col_expr(
            apps::Column::Version,
            Expr::col(apps::Column::Version).add(1),
        )
        .col_expr(apps::Column::UpdatedAt, Expr::value(db::now()))
        .filter(apps::Column::Id.eq(app.id))
        .filter(apps::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(apps::Column::Version.eq(app.version))
        .filter(apps::Column::State.is_not_in(["queued", "running", "needs-review"]))
        .exec(&tx)
        .await?
        .rows_affected;
    if n == 0 {
        return Err(refuse(StatusCode::CONFLICT, "deployment.error.030"));
    }
    tx.commit().await?;
    scheduled_replace(s, scope.clone(), &deployment.to_string(), trigger.action()).await
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
    format!(
        "if docker container inspect {container} >/dev/null 2>&1; then owner=$(docker inspect --format '{{{{index .Config.Labels \"workspace.deployment\"}}}}' {container}); [ \"$owner\" = {prefix} ] || exit 75; docker rm -f {container}; fi",
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
/// stopped first; a fence row is always written, already cleared when they were stopped,
/// and each instance records a `fenced` event (`detail.stopped`).
pub(crate) async fn fence(
    s: &AppState,
    app: &apps::Model,
    host: Id,
    instances: &[String],
) -> Result<()> {
    let workspace = Id::from(app.workspace_id);
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
    for instance in instances {
        fences::Entity::insert(fences::ActiveModel {
            id: sea_orm::Set(s.next_id()?.into()),
            workspace_id: sea_orm::Set(app.workspace_id),
            deployment_id: sea_orm::Set(app.id),
            host_id: sea_orm::Set(host.into()),
            instance: sea_orm::Set(instance.clone()),
            created_at: sea_orm::Set(now),
            cleared_at: sea_orm::Set(if stopped { now } else { 0 }),
        })
        .exec(&tx)
        .await?;
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
    let event = |kind, message: Message, data: Value| NewEvent {
        workspace: scope.workspace_id,
        deployment,
        kind,
        instance: instance.clone(),
        host: checked.from,
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
    match replace_at(
        &s,
        &scope,
        deployment,
        &role,
        index,
        trigger,
        Some(b.version),
    )
    .await
    {
        Ok(revision) => Ok((StatusCode::ACCEPTED, Json(json!({"id": revision})))),
        Err(error) => {
            let reason = match error.2.as_ref() {
                Some(m) if m.key == "blueprint.replaceNoCandidate" => "noCandidate",
                _ => "revisionFailed",
            };
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
                    json!({"reason": reason}),
                ),
            )
            .await?;
            Err(error)
        }
    }
}
