use super::*;
use crate::{
    deployments::{project, quote},
    entities::{deployment_revisions, hosts},
};
use std::collections::BTreeMap;

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Instance {
    pub service: String,
    pub id: String,
    pub node: String,
    pub state: String,
    pub runtime_state: String,
    pub evidence: String,
}
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Service {
    pub name: String,
    pub expected: u32,
    pub healthy: u32,
    pub observed: u32,
    pub unknown: u32,
}
pub(super) async fn validate_host(
    conn: &impl ConnectionTrait,
    workspace: Id,
    app_id: i64,
    host_id: i64,
) -> Result<()> {
    let app = deployments::Entity::find_by_id(app_id)
        .filter(deployments::Column::WorkspaceId.eq(workspace))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)?;
    let host = hosts::Entity::find_by_id(host_id)
        .filter(hosts::Column::WorkspaceId.eq(workspace))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)?;
    if (app.target_kind == "host" && app.target_ref == host_id)
        || (app.target_kind == "host-group" && app.target_ref == host.group_id)
        || (app.target_kind == "blueprint" && blueprint_hosts(conn, &app).await?.contains(&host_id))
    {
        Ok(())
    } else {
        Err(bad("monitoring.bindingInvalid"))
    }
}
/// Each target host's compiled application in a blueprint revision's stored `rendered`.
fn rendered_targets(rendered: &str) -> Vec<(i64, String)> {
    let doc: Value = serde_json::from_str(rendered).unwrap_or_default();
    doc["targets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|target| {
            let host = Id::reference(target["host_id"].as_str()?).ok()?;
            Some((host.into(), target["manifest"].as_str()?.to_owned()))
        })
        .collect()
}
/// A blueprint deployment's target hosts and their compiled applications, as its current
/// revision stored them (never recompiled); none before the first revision.
pub(super) async fn blueprint_targets(
    conn: &impl ConnectionTrait,
    app: &deployments::Model,
) -> Result<Vec<(i64, crate::deployments::manifest::Manifest)>> {
    let Some(revision) = deployment_revisions::Entity::find_by_id(app.current_revision)
        .filter(deployment_revisions::Column::WorkspaceId.eq(app.workspace_id))
        .filter(deployment_revisions::Column::DeploymentId.eq(app.id))
        .one(conn)
        .await?
    else {
        return Ok(Vec::new());
    };
    rendered_targets(&revision.rendered)
        .into_iter()
        .map(|(host, manifest)| {
            Ok((
                host,
                crate::deployments::manifest::parse_for(&manifest, &app.mode)?,
            ))
        })
        .collect()
}
/// Where each blueprint instance (service) runs: a stack's stable instances on the hosts its
/// binding pins them to, a stateless role on the manager; elsewhere on the bound host.
pub(super) fn blueprint_instance_hosts(app: &deployments::Model) -> HashMap<String, (String, i64)> {
    let Some(binding) = app
        .blueprint_binding
        .as_deref()
        .and_then(|text| crate::blueprint::binding::parse(text).ok())
    else {
        return HashMap::new();
    };
    let mut hosts: HashMap<String, (String, i64)> = binding
        .roles
        .iter()
        .flat_map(|(role, list)| {
            list.iter().enumerate().map(move |(index, instance)| {
                (
                    format!("{role}-{index}"),
                    (role.clone(), i64::from(instance.host_id)),
                )
            })
        })
        .collect();
    // A stack's stateless role is one service named after the role, replicated by Swarm
    // from its manager: it has no fixed host.
    if let Some(manager) = binding.manager {
        for role in binding.roles.keys() {
            hosts
                .entry(role.clone())
                .or_insert_with(|| (role.clone(), manager.into()));
        }
    }
    hosts
}
/// Hosts a runtime check may observe a blueprint deployment from: its current revision's
/// targets only. A stack's single target is its manager; worker hosts have no deployment
/// target state, so a check there could never see a running revision.
async fn blueprint_hosts(
    conn: &impl ConnectionTrait,
    app: &deployments::Model,
) -> Result<Vec<i64>> {
    Ok(blueprint_targets(conn, app)
        .await?
        .into_iter()
        .map(|(host, _)| host)
        .collect())
}
pub(super) async fn running_revision(
    conn: &impl ConnectionTrait,
    app: &deployments::Model,
    host: i64,
) -> Result<i64> {
    if app.target_kind == "host" {
        return Ok(app.current_revision);
    }
    use crate::entities::deployment_target_states as targets;
    Ok(targets::Entity::find()
        .filter(targets::Column::WorkspaceId.eq(app.workspace_id))
        .filter(targets::Column::DeploymentId.eq(app.id))
        .filter(targets::Column::HostId.eq(host))
        .one(conn)
        .await?
        .map(|target| target.current_revision)
        .unwrap_or(0))
}
fn unknown(reason: &str) -> Observation {
    Observation {
        outcome: "unknown".into(),
        reason: reason.into(),
        ..Default::default()
    }
}

pub(super) async fn probe(
    s: &AppState,
    scope: &Scope,
    row: &checks::Model,
    settings: &Settings,
) -> Result<Observation> {
    validate_host(&s.db, scope.workspace_id, row.deployment_id, row.host_id).await?;
    let app = deployments::Entity::find_by_id(row.deployment_id)
        .filter(deployments::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let revision_id = running_revision(&s.db, &app, row.host_id).await?;
    if revision_id == 0 {
        return Ok(unknown("monitoring.noRevision"));
    }
    let revision = deployment_revisions::Entity::find_by_id(revision_id)
        .filter(deployment_revisions::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(deployment_revisions::Column::DeploymentId.eq(app.id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let manifest = if app.target_kind == "blueprint" {
        // A blueprint revision stores each target host's compiled application.
        let Some((_, manifest)) = rendered_targets(&revision.rendered)
            .into_iter()
            .find(|(host, _)| *host == row.host_id)
        else {
            return Ok(unknown("monitoring.noRevision"));
        };
        manifest
    } else {
        revision.manifest.clone()
    };
    let manifest = crate::deployments::manifest::parse_for(&manifest, &app.mode)?;
    let channel = crate::hosts::remote::open(s, &scope.workspace_id, &row.host_id.into()).await?;
    let mut result = Observation {
        outcome: "up".into(),
        revision_id: Id::from(revision.id).to_string(),
        ..Default::default()
    };
    let prefix = project(&app);
    let started = std::time::Instant::now();
    // One bounded command per service, no environment, secrets, health logs or arbitrary scripts.
    let work = async {
        if app.mode == "stack" {
            let output = channel
                .exec(
                    "docker info --format '{{.Swarm.Cluster.ID}}'",
                    Duration::from_secs(settings.timeout_seconds.into()),
                )
                .await?;
            let cluster = output.stdout.trim();
            if output.code != 0
                || cluster.is_empty()
                || cluster.len() > 128
                || !cluster.bytes().all(|byte| byte.is_ascii_alphanumeric())
            {
                return Ok(unknown("monitoring.observerUnavailable"));
            }
            result.runtime_scope = format!("swarm:{cluster}");
        }
        for service in &manifest.services {
            let name = &service.name;
            let command = match app.mode.as_str() {
                "native" => format!(
                    "systemctl show {} --no-pager --property=LoadState,ActiveState,SubState,MainPID",
                    quote(&format!("{prefix}-{name}.service"))
                ),
                "docker" => format!(
                    "docker container ls -a --no-trunc --filter {} --format '{{{{json .}}}}'",
                    quote(&format!("name=^/{prefix}-{name}$"))
                ),
                "compose" => format!(
                    "docker container ls -a --no-trunc --filter {} --filter {} --format '{{{{json .}}}}'",
                    quote(&format!("label=com.docker.compose.project={prefix}")),
                    quote(&format!("label=com.docker.compose.service={name}"))
                ),
                "stack" => format!(
                    "docker service ps --no-trunc --filter desired-state=running --format '{{{{json .}}}}' {}",
                    quote(&format!("{prefix}_{name}"))
                ),
                _ => return Ok(unknown("monitoring.runtimeUnsupported")),
            };
            let output = channel
                .exec(
                    &command,
                    Duration::from_secs(settings.timeout_seconds.into()),
                )
                .await?;
            if output.stdout.len() > 1024 * 1024 || output.code != 0 {
                return Ok(unknown("monitoring.observerUnavailable"));
            }
            let instances = parse(&app.mode, name, &row.host_id.to_string(), &output.stdout)?;
            let expected = if app.mode == "stack" {
                service.replicas
            } else {
                1
            };
            let healthy = instances.iter().filter(|i| i.state == "up").count() as u32;
            let unknown = instances.iter().filter(|i| i.state == "unknown").count() as u32;
            if healthy + unknown < expected {
                result.outcome = "down".into();
            } else if unknown > 0 && result.outcome != "down" {
                result.outcome = "unknown".into();
            }
            result.services.push(Service {
                name: name.clone(),
                expected,
                healthy,
                observed: instances.len() as u32,
                unknown,
            });
            result.instances.extend(instances);
        }
        if result.services.is_empty() {
            return Ok(unknown("monitoring.noRevision"));
        }
        result.latency_ms = started.elapsed().as_millis() as u64;
        Ok(result)
    };
    tokio::time::timeout(Duration::from_secs(settings.timeout_seconds.into()), work)
        .await
        .unwrap_or_else(|_| Ok(unknown("monitoring.observerUnavailable")))
}
pub(super) fn parse(mode: &str, service: &str, node: &str, output: &str) -> Result<Vec<Instance>> {
    if mode == "native" {
        let fields: HashMap<_, _> = output
            .lines()
            .filter_map(|line| line.split_once('='))
            .collect();
        let load = fields
            .get("LoadState")
            .copied()
            .ok_or_else(|| bad("monitoring.runtimeInvalid"))?;
        let active = fields
            .get("ActiveState")
            .copied()
            .ok_or_else(|| bad("monitoring.runtimeInvalid"))?;
        let pid = fields
            .get("MainPID")
            .copied()
            .ok_or_else(|| bad("monitoring.runtimeInvalid"))?;
        let state = if load == "not-found" {
            "down"
        } else if load != "loaded" {
            "unknown"
        } else if active == "active" && pid != "0" {
            "up"
        } else {
            "down"
        };
        return Ok(vec![Instance {
            service: service.into(),
            id: format!("{service}:{pid}"),
            node: node.into(),
            state: state.into(),
            runtime_state: active.into(),
            evidence: "process".into(),
        }]);
    }
    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let value: Value =
                serde_json::from_str(line).map_err(|_| bad("monitoring.runtimeInvalid"))?;
            let id = value["ID"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| bad("monitoring.runtimeInvalid"))?;
            let (runtime_state, state, evidence, node) = if mode == "stack" {
                let current = value["CurrentState"]
                    .as_str()
                    .ok_or_else(|| bad("monitoring.runtimeInvalid"))?;
                let state = if current.starts_with("Running ") || current == "Running" {
                    "up"
                } else if current.starts_with("Failed ")
                    || current.starts_with("Shutdown ")
                    || current.starts_with("Rejected ")
                {
                    "down"
                } else {
                    "unknown"
                };
                (
                    current,
                    state,
                    "task",
                    value["Node"].as_str().unwrap_or(node),
                )
            } else {
                let current = value["State"]
                    .as_str()
                    .ok_or_else(|| bad("monitoring.runtimeInvalid"))?;
                let status = value["Status"].as_str().unwrap_or("");
                let state = if current != "running" || status.contains("(unhealthy)") {
                    "down"
                } else if status.contains("health: starting") {
                    "unknown"
                } else {
                    "up"
                };
                (
                    current,
                    state,
                    if status.contains("(healthy)") || status.contains("(unhealthy)") {
                        "container-health"
                    } else {
                        "process"
                    },
                    node,
                )
            };
            Ok(Instance {
                service: service.into(),
                id: id.into(),
                node: node.into(),
                state: state.into(),
                runtime_state: runtime_state.into(),
                evidence: evidence.into(),
            })
        })
        .collect()
}

#[derive(Default)]
pub(super) struct Summary {
    pub state: String,
    pub entry_state: String,
    pub healthy: u32,
    pub expected: u32,
    pub unknown: u32,
    pub services: Vec<Service>,
}

/// Swarm managers observe the same cluster-wide demand, not separate replicas.
pub(super) fn cluster_services(samples: Vec<Service>) -> Vec<Service> {
    let mut services: BTreeMap<String, Service> = BTreeMap::new();
    for sample in samples {
        services
            .entry(sample.name.clone())
            .and_modify(|service| {
                service.expected = service.expected.max(sample.expected);
                service.healthy = service.healthy.min(sample.healthy);
                service.observed = service.observed.min(sample.observed);
                service.unknown = service.unknown.max(sample.unknown);
            })
            .or_insert(sample);
    }
    services.into_values().collect()
}

pub(super) fn aggregate(rows: &[&checks::Model], now: i64) -> Summary {
    let entries: Vec<_> = rows
        .iter()
        .filter(|r| r.role == "entry" && r.enabled == 1)
        .collect();
    let entry_down = entries.iter().any(|r| state(r, now) == "down");
    let entry_up = !entries.is_empty() && entries.iter().all(|r| state(r, now) == "up");
    let mut summary = Summary {
        entry_state: if entry_down {
            "down"
        } else if entry_up {
            "up"
        } else {
            "unknown"
        }
        .into(),
        ..Default::default()
    };
    let mut services: BTreeMap<(String, String), Service> = BTreeMap::new();
    let mut missing = false;
    let mut confirmed_bad = false;
    let runtime: Vec<_> = rows
        .iter()
        .filter(|r| r.kind == "runtime" && r.enabled == 1)
        .collect();
    for row in &runtime {
        let status = state(row, now);
        let result: CheckResult = serde_json::from_str(&row.result).unwrap_or_default();
        // Two checks on one target must not multiply expected/observed replicas.
        let key = if !result.observation.runtime_scope.is_empty() {
            result.observation.runtime_scope.clone()
        } else {
            row.host_id.to_string()
        };
        if matches!(status.as_str(), "unknown" | "stale" | "pending") {
            missing = true;
        }
        if status == "down" {
            confirmed_bad = true;
        }
        for service in result.observation.services {
            let mut service = service;
            if matches!(status.as_str(), "unknown" | "stale" | "pending") {
                service.unknown = service.expected;
                service.healthy = 0;
            }
            services
                .entry((key.clone(), service.name.clone()))
                .and_modify(|old| {
                    // Conservative intersection: conflicting observers never hide a failing replica.
                    old.expected = old.expected.max(service.expected);
                    old.healthy = old.healthy.min(service.healthy);
                    old.unknown = old.unknown.max(service.unknown);
                })
                .or_insert(service);
        }
    }
    for row in rows
        .iter()
        .filter(|r| r.role == "service" && r.kind != "runtime" && r.enabled == 1)
    {
        match state(row, now).as_str() {
            "down" => confirmed_bad = true,
            "up" => {}
            _ => missing = true,
        }
    }
    summary.services = services.into_values().collect();
    summary.expected = summary.services.iter().map(|s| s.expected).sum();
    summary.healthy = summary
        .services
        .iter()
        .map(|s| s.healthy.min(s.expected))
        .sum();
    summary.unknown = summary
        .services
        .iter()
        .map(|s| s.unknown.min(s.expected))
        .sum();
    summary.state = if entry_down {
        "down"
    } else if entry_up && confirmed_bad {
        "degraded"
    } else if !entry_up || missing || runtime.is_empty() || summary.expected == 0 {
        "unknown"
    } else if summary.healthy < summary.expected {
        "unknown"
    } else {
        "up"
    }
    .into();
    summary
}
