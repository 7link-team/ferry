//! Deployment specs (`ferry/deploy-v1`): the server renders the template and schedules the
//! instances, then the result flows through the ordinary blueprint deployment checks.
use super::{Body, apps, candidates, groups};
use crate::{
    accounts::workspace::Scope,
    blueprint::{
        binding::{self, Binding, HostInfo},
        model::{self, Blueprint},
        placement,
        spec::{self, DeploySpec},
    },
    entities::app_templates,
    error::{ApiError, Json, Result},
    i18n::Message,
    snowflake::Id,
    state::AppState,
};
use axum::{extract::State, http::StatusCode};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The spec's template (id + version) rendered with its values into a blueprint manifest.
async fn render(
    conn: &impl ConnectionTrait,
    scope: &Scope,
    spec: &DeploySpec,
) -> Result<(String, Blueprint)> {
    scope.require("app-templates.read")?;
    let row = app_templates::Entity::find_by_id(i64::from(spec.template.id))
        .filter(app_templates::Column::WorkspaceId.eq(scope.workspace_id))
        .one(conn)
        .await?
        .filter(|row| row.version == spec.template.version)
        .ok_or_else(|| {
            ApiError::localized(StatusCode::CONFLICT, Message::new("templates.changed"))
        })?;
    let manifest = crate::templates::render_app(&row.content, &row.parameters, &spec.values)?;
    if !model::is_blueprint(&manifest) {
        return Err(super::blueprint_invalid());
    }
    let bp = model::parse(&manifest)?;
    Ok((manifest, bp))
}

/// Manual placements are checked (their addresses normalized in `spec`), the remaining roles
/// scheduled, and the whole binding validated against this workspace's hosts.
///
/// Nothing of this deployment runs yet whenever it is scheduled, so there is no existing
/// load: the placement endpoint and the stored binding always agree for the same spec.
async fn schedule(
    conn: &impl ConnectionTrait,
    scope: &Scope,
    bp: &Blueprint,
    spec: &mut DeploySpec,
) -> Result<placement::Placed> {
    scope.require("hosts.read")?;
    let candidates = candidates::load(conn, scope.workspace_id).await?;
    placement::check_manual(bp, spec, &candidates)?;
    let placed = placement::place(bp, spec, &candidates, &BTreeMap::new())?;
    let hosts: BTreeMap<Id, HostInfo> = candidates
        .iter()
        .map(|c| {
            let info = HostInfo {
                id: c.id,
                os: c.os.clone(),
            };
            (c.id, info)
        })
        .collect();
    binding::validate(bp, &placed.binding, &hosts)?;
    Ok(placed)
}

/// Everything but the values and the template version: what decides where instances run.
/// `failover` is not topology either (it only decides how a down host is handled), so it
/// may change after deploying.
fn same_topology(a: &DeploySpec, b: &DeploySpec) -> bool {
    a.template.id == b.template.id
        && a.driver == b.driver
        && a.manager == b.manager
        && a.roles == b.roles
        && hosts_of(a) == hosts_of(b)
}

/// Manual placement as chosen hosts; addresses are always derived from the hosts.
fn hosts_of(s: &DeploySpec) -> Option<BTreeMap<&String, Vec<Id>>> {
    s.placement.as_ref().map(|p| {
        p.iter()
            .map(|(role, list)| (role, list.iter().map(|i| i.host_id).collect()))
            .collect()
    })
}

fn topology_change() -> ApiError {
    ApiError::localized(
        StatusCode::BAD_REQUEST,
        Message::new("blueprint.topologyChangeRequiresPlan"),
    )
}

/// When `b` carries a deployment spec, derives its manifest, binding, target and mode from
/// it (client-sent values are ignored) and returns the canonical spec text to store.
///
/// `previous` is the deployment being updated. Once a host runs it, the stored binding is
/// kept and only a values/template-version change is accepted; before that, the spec is
/// scheduled afresh, exactly as the placement endpoint would.
pub(super) async fn resolve(
    conn: &impl ConnectionTrait,
    scope: &Scope,
    b: &mut Body,
    previous: Option<&apps::Model>,
) -> Result<Option<String>> {
    let Some(text) = &b.deploy_spec else {
        return Ok(None);
    };
    let mut spec = spec::parse(text)?;
    if previous.is_some_and(|p| p.target_kind != "blueprint") {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("deployment.error.007"),
        ));
    }
    let (manifest, bp) = render(conn, scope, &spec).await?;
    let frozen = match previous {
        Some(p) => groups::deployed(conn, p).await?,
        None => false,
    };
    let binding: Binding = if let (true, Some(p)) = (frozen, previous) {
        let stored = p.deploy_spec.as_deref().map(spec::parse).transpose();
        // A deployed topology never moves: not even a legacy deployment adopting a spec.
        match stored {
            // Keep the stored (address-normalized) manual placement.
            Ok(Some(stored)) if same_topology(&stored, &spec) => spec.placement = stored.placement,
            _ => return Err(topology_change()),
        }
        binding::parse(p.blueprint_binding.as_deref().unwrap_or_default())?
    } else {
        schedule(conn, scope, &bp, &mut spec).await?.binding
    };
    b.manifest = manifest;
    b.blueprint_binding =
        Some(serde_json::to_value(&binding).map_err(|_| super::blueprint_invalid())?);
    b.target_kind = "blueprint".into();
    b.target_ref = "0".into();
    b.mode = spec.driver.clone();
    Ok(Some(spec::to_yaml(&spec)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementBody {
    spec: String,
}

/// Renders and schedules a spec without storing anything: for a new deployment, or for
/// editing an undeployed one's spec (so creators and updaters may both preview).
pub async fn placement(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<PlacementBody>,
) -> Result<Json<Value>> {
    if !scope.permissions.contains("deployments.create")
        && !scope.permissions.contains("deployments.update")
    {
        return Err(ApiError::forbidden());
    }
    let mut spec = spec::parse(&body.spec)?;
    let (manifest, bp) = render(&s.db, &scope, &spec).await?;
    let placed = schedule(&s.db, &scope, &bp, &mut spec).await?;
    let locale = crate::i18n::current_locale();
    let warnings: Vec<Value> = placed
        .warnings
        .iter()
        .map(|m| json!({"key":m.key,"params":m.params,"message":crate::i18n::render(locale, m)}))
        .collect();
    Ok(Json(
        json!({"manifest":manifest,"binding":placed.binding,"warnings":warnings}),
    ))
}
