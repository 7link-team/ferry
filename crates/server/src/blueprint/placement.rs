//! Deterministic, tag-aware placement: deployment spec + candidate hosts -> instance binding.
use super::binding::{Binding, InstanceBinding};
use super::model::{Blueprint, Role};
use super::spec::{DeploySpec, HostSelector};
use crate::deployments::candidates::Candidate;
use crate::error::{ApiError, Result};
use crate::i18n::Message;
use crate::snowflake::Id;
use axum::http::StatusCode;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

/// At most this many host names are listed in a placement failure.
const MAX_LISTED_HOSTS: usize = 10;

#[derive(Debug)]
pub struct Placed {
    pub binding: Binding,
    pub warnings: Vec<Message>,
}

fn fail<T>(message: Message) -> Result<T> {
    Err(ApiError::localized(StatusCode::BAD_REQUEST, message))
}

/// Online, and Linux or an SSH host with unknown OS.
fn eligible(c: &Candidate) -> bool {
    c.online && (c.os.is_empty() || c.os.eq_ignore_ascii_case("linux"))
}

/// `tags` are AND; `names` and `group` intersect with them; an empty selector matches all.
fn matches(selector: &HostSelector, c: &Candidate) -> bool {
    selector.tags.iter().all(|t| c.tags.contains(t))
        && (selector.names.is_empty() || selector.names.contains(&c.name))
        && selector
            .group
            .as_ref()
            .is_none_or(|g| c.group.as_ref() == Some(g))
}

fn describe(selector: &HostSelector) -> String {
    let mut parts = Vec::new();
    if !selector.tags.is_empty() {
        parts.push(format!("tags=[{}]", selector.tags.join(", ")));
    }
    if !selector.names.is_empty() {
        parts.push(format!("names=[{}]", selector.names.join(", ")));
    }
    if let Some(group) = &selector.group {
        parts.push(format!("group={group}"));
    }
    if parts.is_empty() {
        "*".to_string()
    } else {
        parts.join(" ")
    }
}

fn insufficient(
    role: &str,
    selector: &HostSelector,
    needed: usize,
    hosts: &[&Candidate],
) -> Message {
    let mut names: Vec<&str> = hosts
        .iter()
        .take(MAX_LISTED_HOSTS)
        .map(|c| c.name.as_str())
        .collect();
    if hosts.len() > MAX_LISTED_HOSTS {
        names.push("…");
    }
    // An empty list still reads naturally after the message's colon.
    let names = if names.is_empty() {
        "-".to_string()
    } else {
        names.join(", ")
    };
    Message::new("blueprint.placementInsufficientHosts")
        .with("role", role)
        .with("selector", describe(selector))
        .with("needed", needed.to_string())
        .with("matched", hosts.len().to_string())
        .with("hosts", names)
}

/// Spread domains of `c`: its tags with `prefix` (e.g. `zone:a` for `zone:`). A host with no
/// such tag belongs to one shared pseudo-domain (`None`) with every other untagged host.
fn zones<'a>(c: &'a Candidate, prefix: &str) -> BTreeSet<Option<&'a str>> {
    let tagged: BTreeSet<Option<&str>> = c
        .tags
        .iter()
        .filter(|t| t.starts_with(prefix))
        .map(|t| Some(t.as_str()))
        .collect();
    if tagged.is_empty() {
        BTreeSet::from([None])
    } else {
        tagged
    }
}

/// Checks the manual placement (`spec.placement`) and normalizes it in place.
///
/// Each entry must name a blueprint role. When the spec gives that role an instance count,
/// the list must have exactly that many instances. Every instance must run on a candidate
/// host that is online, Linux (or unknown OS) and matches the role's selector. A violating
/// entry is refused, never rescheduled. Users choose hosts, not addresses: each instance's
/// `address` is overwritten with its host's address.
pub fn check_manual(bp: &Blueprint, spec: &mut DeploySpec, candidates: &[Candidate]) -> Result<()> {
    let Some(placement) = spec.placement.as_mut() else {
        return Ok(());
    };
    for (role, list) in placement.iter_mut() {
        if !bp.roles.iter().any(|r| &r.name == role) {
            return fail(Message::new("blueprint.bindingRoleUnknown").with("role", role));
        }
        let role_spec = spec.roles.get(role);
        if let Some(wanted) = role_spec.map(|r| r.instances)
            && list.len() != wanted as usize
        {
            return fail(
                Message::new("blueprint.placementManualCount")
                    .with("role", role)
                    .with("instances", wanted.to_string())
                    .with("count", list.len().to_string()),
            );
        }
        let selector = role_spec.map(|r| r.hosts.clone()).unwrap_or_default();
        for (index, inst) in list.iter_mut().enumerate() {
            let host = candidates.iter().find(|c| c.id == inst.host_id);
            match host {
                Some(c) if eligible(c) && matches(&selector, c) => {
                    inst.address = c.address.clone();
                }
                _ => {
                    return fail(
                        Message::new("blueprint.placementManualInvalid")
                            .with("role", role)
                            .with("instance", format!("{role}-{index}"))
                            .with(
                                "host",
                                host.map_or_else(|| inst.host_id.to_string(), |c| c.name.clone()),
                            ),
                    );
                }
            }
        }
    }
    Ok(())
}

/// Places every blueprint role without a manual `placement` entry onto the candidate hosts.
///
/// `existing` counts THIS deployment's instances already on each host, excluding any instance
/// listed in `spec.placement`; `place` adds the manual placements to the load itself.
/// Instances are placed one at a time on the host with, in order: fewest spread violations,
/// fewest instances of this deployment (existing + placed so far), most free memory
/// (unknown last), lowest id. The compose driver instead puts every instance on one host
/// chosen by load, then free memory, then id, ignoring spread.
pub fn place(
    bp: &Blueprint,
    spec: &DeploySpec,
    candidates: &[Candidate],
    existing: &BTreeMap<Id, u32>,
) -> Result<Placed> {
    let manual = spec.placement.clone().unwrap_or_default();
    for name in spec.roles.keys().chain(manual.keys()) {
        if !bp.roles.iter().any(|r| &r.name == name) {
            return fail(Message::new("blueprint.bindingRoleUnknown").with("role", name));
        }
    }
    let manager = resolve_manager(spec, candidates)?;
    let mut load = existing.clone();
    for inst in manual.values().flatten() {
        *load.entry(inst.host_id).or_default() += 1;
    }
    let mut scheduler = Scheduler {
        bp,
        spec,
        candidates,
        load,
        roles: manual,
        warnings: Vec::new(),
    };
    if spec.driver == "compose" {
        scheduler.place_single_host()?;
    } else {
        for role in &bp.roles {
            scheduler.place_role(role)?;
        }
    }
    Ok(Placed {
        binding: Binding {
            driver: spec.driver.clone(),
            manager,
            roles: scheduler.roles,
        },
        warnings: scheduler.warnings,
    })
}

/// The Swarm manager (host name or id) must be an online Linux candidate; other drivers have none.
fn resolve_manager(spec: &DeploySpec, candidates: &[Candidate]) -> Result<Option<Id>> {
    if spec.driver != "stack" {
        return Ok(None);
    }
    let wanted = spec.manager.as_deref().unwrap_or_default();
    let usable = || candidates.iter().filter(|c| eligible(c));
    let by_name: Vec<&Candidate> = usable().filter(|c| c.name == wanted).collect();
    // An ambiguous name is refused rather than resolved to an arbitrary host.
    let found = match by_name.as_slice() {
        [one] => Some(*one),
        [] => usable().find(|c| c.id.to_string() == wanted),
        _ => None,
    };
    match found {
        Some(c) if !wanted.is_empty() => Ok(Some(c.id)),
        _ => fail(Message::new("blueprint.placementManagerUnavailable").with("manager", wanted)),
    }
}

struct Scheduler<'a> {
    bp: &'a Blueprint,
    spec: &'a DeploySpec,
    candidates: &'a [Candidate],
    load: BTreeMap<Id, u32>,
    roles: BTreeMap<String, Vec<InstanceBinding>>,
    warnings: Vec<Message>,
}

impl<'a> Scheduler<'a> {
    fn selector(&self, role: &Role) -> HostSelector {
        self.spec
            .roles
            .get(&role.name)
            .map(|r| r.hosts.clone())
            .unwrap_or_default()
    }

    /// Compose runs on one host: every instance of every role goes to the best host that
    /// satisfies all scheduled roles' selectors (and holds any manual placement).
    fn place_single_host(&mut self) -> Result<()> {
        let bp = self.bp;
        if !bp.allow_single_host && bp.roles.iter().any(|r| r.placement.distinct_hosts) {
            return fail(Message::new("blueprint.bindingComposeNeedsSingleHost"));
        }
        let pinned: BTreeSet<Id> = self.roles.values().flatten().map(|i| i.host_id).collect();
        if pinned.len() > 1 {
            return fail(Message::new("blueprint.bindingComposeSingleHost"));
        }
        let scheduled: Vec<&Role> = bp
            .roles
            .iter()
            .filter(|r| !self.roles.contains_key(&r.name))
            .collect();
        let Some(first) = scheduled.first() else {
            return Ok(());
        };
        let mut counts = BTreeMap::new();
        for role in &scheduled {
            counts.insert(role.name.as_str(), self.count(role)?);
        }
        for role in &scheduled {
            if let Some(target) = &role.placement.colocate_with {
                let target_count = self
                    .roles
                    .get(target)
                    .map(Vec::len)
                    .or_else(|| counts.get(target.as_str()).copied());
                if target_count != Some(counts[role.name.as_str()]) {
                    return fail(
                        Message::new("blueprint.bindingColocateCount")
                            .with("role", &role.name)
                            .with("target", target),
                    );
                }
            }
        }
        let selectors: Vec<HostSelector> = scheduled.iter().map(|r| self.selector(r)).collect();
        let fitting: Vec<&'a Candidate> = self
            .candidates
            .iter()
            .filter(|c| eligible(c) && selectors.iter().all(|s| matches(s, c)))
            .collect();
        let best = fitting.iter().copied().min_by_key(|c| {
            (
                self.load.get(&c.id).copied().unwrap_or(0),
                Reverse(c.mem_free.unwrap_or(0)),
                c.id,
            )
        });
        let host = match (pinned.first(), best) {
            (Some(id), _) => match fitting.iter().copied().find(|c| c.id == *id) {
                Some(c) => c,
                None => return fail(Message::new("blueprint.bindingComposeSingleHost")),
            },
            (None, Some(c)) => c,
            (None, None) => {
                return fail(
                    Message::new("blueprint.placementSelectorEmpty")
                        .with("role", &first.name)
                        .with("selector", describe(&selectors[0])),
                );
            }
        };
        for role in scheduled {
            let n = counts[role.name.as_str()];
            let list = vec![
                InstanceBinding {
                    host_id: host.id,
                    address: host.address.clone(),
                };
                n
            ];
            *self.load.entry(host.id).or_default() += n as u32;
            self.roles.insert(role.name.clone(), list);
        }
        Ok(())
    }

    fn place_role(&mut self, role: &Role) -> Result<()> {
        if self.roles.contains_key(&role.name) {
            return Ok(());
        }
        let count = self.count(role)?;
        let selector = self.selector(role);
        let mut matched: Vec<&'a Candidate> = self
            .candidates
            .iter()
            .filter(|c| eligible(c) && matches(&selector, c))
            .collect();
        // Listed in failures, so keep it independent of candidate order.
        matched.sort_by_key(|c| c.id);
        if matched.is_empty() {
            return fail(
                Message::new("blueprint.placementSelectorEmpty")
                    .with("role", &role.name)
                    .with("selector", describe(&selector)),
            );
        }
        let list = match &role.placement.colocate_with {
            Some(target) => self.colocate(role, target, count, &selector, &matched)?,
            None => self.spread(role, count, &selector, &matched)?,
        };
        for inst in &list {
            *self.load.entry(inst.host_id).or_default() += 1;
        }
        self.roles.insert(role.name.clone(), list);
        Ok(())
    }

    /// Spec instance count (or the blueprint default) within the role's min/max/odd rules.
    fn count(&self, role: &Role) -> Result<usize> {
        let count = self
            .spec
            .roles
            .get(&role.name)
            .map_or(role.instances.default, |r| r.instances);
        if !(role.instances.min..=role.instances.max).contains(&count) {
            return fail(
                Message::new("blueprint.bindingCount")
                    .with("role", &role.name)
                    .with("min", role.instances.min.to_string())
                    .with("max", role.instances.max.to_string()),
            );
        }
        if role.instances.odd && count.is_multiple_of(2) {
            return fail(Message::new("blueprint.bindingOdd").with("role", &role.name));
        }
        Ok(count as usize)
    }

    /// Instance i runs on the host of target instance i, which must also match this selector.
    fn colocate(
        &mut self,
        role: &Role,
        target: &str,
        count: usize,
        selector: &HostSelector,
        matched: &[&'a Candidate],
    ) -> Result<Vec<InstanceBinding>> {
        if let Some(t) = self.bp.roles.iter().find(|r| r.name == target) {
            self.place_role(t)?;
        }
        let targets = self.roles.get(target).cloned().unwrap_or_default();
        if targets.len() != count {
            return fail(
                Message::new("blueprint.bindingColocateCount")
                    .with("role", &role.name)
                    .with("target", target),
            );
        }
        if role.placement.distinct_hosts {
            let distinct: BTreeSet<Id> = targets.iter().map(|t| t.host_id).collect();
            if distinct.len() != targets.len() {
                return fail(
                    Message::new("blueprint.bindingDistinctHosts").with("role", &role.name),
                );
            }
        }
        // Spread does not apply: the hosts are the target's.
        let hosts: Vec<Option<&'a Candidate>> = targets
            .iter()
            .map(|t| matched.iter().copied().find(|c| c.id == t.host_id))
            .collect();
        if hosts.iter().any(Option::is_none) {
            let mut usable: Vec<&Candidate> = hosts.iter().flatten().copied().collect();
            usable.sort_by_key(|c| c.id);
            usable.dedup_by_key(|c| c.id);
            return fail(insufficient(&role.name, selector, count, &usable));
        }
        Ok(hosts
            .into_iter()
            .flatten()
            .map(|c| InstanceBinding {
                host_id: c.id,
                address: c.address.clone(),
            })
            .collect())
    }

    fn spread(
        &mut self,
        role: &Role,
        count: usize,
        selector: &HostSelector,
        matched: &[&'a Candidate],
    ) -> Result<Vec<InstanceBinding>> {
        let distinct = role.placement.distinct_hosts;
        let needed = if distinct { count } else { 1 };
        if matched.len() < needed {
            return fail(insufficient(&role.name, selector, needed, matched));
        }
        let prefix = selector.spread.as_ref().map(|s| s.prefix.as_str());
        let mut placed: Vec<&'a Candidate> = Vec::new();
        let mut load = self.load.clone();
        let mut unmet = false;
        for _ in 0..count {
            let violations = |c: &Candidate| -> usize {
                let Some(prefix) = prefix else { return 0 };
                let own = zones(c, prefix);
                placed
                    .iter()
                    .filter(|p| !zones(p, prefix).is_disjoint(&own))
                    .count()
            };
            let best = matched
                .iter()
                .copied()
                .filter(|c| !distinct || placed.iter().all(|p| p.id != c.id))
                .min_by_key(|c| {
                    (
                        violations(c),
                        load.get(&c.id).copied().unwrap_or(0),
                        Reverse(c.mem_free.unwrap_or(0)),
                        c.id,
                    )
                })
                .expect("enough matched hosts were checked above");
            unmet |= violations(best) > 0;
            *load.entry(best.id).or_default() += 1;
            placed.push(best);
        }
        if unmet && let Some(prefix) = prefix {
            self.warnings.push(
                Message::new("blueprint.placementSpreadUnmet")
                    .with("role", &role.name)
                    .with("prefix", prefix),
            );
        }
        Ok(placed
            .into_iter()
            .map(|c| InstanceBinding {
                host_id: c.id,
                address: c.address.clone(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blueprint::spec::{RoleSpec, Spread, TemplateRef};
    use serde_json::{Value, json};

    fn role(name: &str, min: u32, default: u32, max: u32, odd: bool, placement: Value) -> Value {
        json!({
            "name": name,
            "instances": {"min": min, "max": max, "default": default, "odd": odd},
            "placement": placement,
            "identity": "stable",
            "service": {"image": "x"}
        })
    }

    fn blueprint(roles: Vec<Value>) -> Blueprint {
        blueprint_with(roles, false)
    }

    fn blueprint_with(roles: Vec<Value>, single: bool) -> Blueprint {
        let text = json!({
            "apiVersion": "ferry/blueprint-v1",
            "kind": "Blueprint",
            "metadata": {"name": "demo"},
            "roles": roles,
            "drivers": {"native": {"supported": true}, "stack": {"supported": true},
                        "compose": {"supported": true}},
            "allowSingleHost": single,
        })
        .to_string();
        super::super::model::parse(&text).expect("blueprint")
    }

    fn nats() -> Blueprint {
        blueprint(vec![
            role("nats", 1, 3, 5, true, json!({"distinctHosts": true})),
            role("exporter", 1, 3, 5, false, json!({"colocateWith": "nats"})),
        ])
    }

    fn host(id: i64, tags: &[&str]) -> Candidate {
        Candidate {
            id: Id::from(id),
            name: format!("host-{id}"),
            address: format!("10.0.0.{id}"),
            os: "linux".into(),
            online: true,
            group: None,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            mem_free: None,
            disk_free: None,
        }
    }

    fn hosts(n: i64) -> Vec<Candidate> {
        (1..=n).map(|id| host(id, &[])).collect()
    }

    fn spec(driver: &str, roles: &[(&str, u32, HostSelector)]) -> DeploySpec {
        DeploySpec {
            template: TemplateRef {
                id: Id::from(1),
                version: 1,
            },
            driver: driver.into(),
            manager: None,
            values: BTreeMap::new(),
            roles: roles
                .iter()
                .map(|(name, instances, hosts)| {
                    (
                        name.to_string(),
                        RoleSpec {
                            instances: *instances,
                            hosts: hosts.clone(),
                        },
                    )
                })
                .collect(),
            placement: None,
        }
    }

    fn tags(t: &[&str]) -> HostSelector {
        HostSelector {
            tags: t.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn ids(b: &Binding, role: &str) -> Vec<i64> {
        b.roles[role].iter().map(|i| i64::from(i.host_id)).collect()
    }

    fn none() -> BTreeMap<Id, u32> {
        BTreeMap::new()
    }

    fn err(r: Result<Placed>) -> Message {
        r.expect_err("expected error").2.expect("message")
    }

    #[test]
    fn places_distinct_hosts_deterministically() {
        let s = spec("native", &[]);
        let a = place(&nats(), &s, &hosts(5), &none()).unwrap();
        assert_eq!(ids(&a.binding, "nats"), [1, 2, 3]);
        assert_eq!(a.binding.roles["nats"][1].address, "10.0.0.2");
        assert_eq!(a.binding.driver, "native");
        assert_eq!(a.binding.manager, None);
        assert!(a.warnings.is_empty());
        // Same input, same output, regardless of candidate order.
        let mut reversed = hosts(5);
        reversed.reverse();
        let b = place(&nats(), &s, &reversed, &none()).unwrap();
        assert_eq!(ids(&b.binding, "nats"), [1, 2, 3]);
        // Hosts already running this deployment are used last.
        let existing = BTreeMap::from([(Id::from(1), 2), (Id::from(2), 1)]);
        let c = place(&nats(), &s, &hosts(5), &existing).unwrap();
        assert_eq!(ids(&c.binding, "nats"), [3, 4, 5]);
        // The result is a valid binding.
        let info: BTreeMap<Id, super::super::binding::HostInfo> = hosts(5)
            .iter()
            .map(|h| {
                (
                    h.id,
                    super::super::binding::HostInfo {
                        id: h.id,
                        os: h.os.clone(),
                    },
                )
            })
            .collect();
        super::super::binding::validate(&nats(), &a.binding, &info).expect("valid binding");
        // Instance counts follow the spec and the blueprint's rules.
        let five = spec(
            "native",
            &[
                ("nats", 5, Default::default()),
                ("exporter", 5, Default::default()),
            ],
        );
        let d = place(&nats(), &five, &hosts(5), &none()).unwrap();
        assert_eq!(ids(&d.binding, "nats"), [1, 2, 3, 4, 5]);
        let even = spec("native", &[("nats", 4, Default::default())]);
        assert_eq!(
            err(place(&nats(), &even, &hosts(5), &none())).key,
            "blueprint.bindingOdd"
        );
        let many = spec("native", &[("nats", 7, Default::default())]);
        assert_eq!(
            err(place(&nats(), &many, &hosts(9), &none())).key,
            "blueprint.bindingCount"
        );
        let ghost = spec("native", &[("ghost", 1, Default::default())]);
        assert_eq!(
            err(place(&nats(), &ghost, &hosts(5), &none())).key,
            "blueprint.bindingRoleUnknown"
        );
    }

    #[test]
    fn colocate_follows_target_role() {
        // exporter is listed first in the blueprint but still follows nats.
        let bp = blueprint(vec![
            role("exporter", 1, 3, 5, false, json!({"colocateWith": "nats"})),
            role("nats", 1, 3, 5, true, json!({"distinctHosts": true})),
        ]);
        let mut cands = hosts(4);
        for c in &mut cands {
            c.mem_free = Some(100);
        }
        cands[0].mem_free = Some(1);
        let s = spec("native", &[]);
        let p = place(&bp, &s, &cands, &none()).unwrap();
        assert_eq!(ids(&p.binding, "nats"), [2, 3, 4]);
        assert_eq!(ids(&p.binding, "exporter"), [2, 3, 4]);
        assert_eq!(p.binding.roles["exporter"][0].address, "10.0.0.2");
        // Counts must agree.
        let s = spec("native", &[("exporter", 1, Default::default())]);
        assert_eq!(
            err(place(&bp, &s, &cands, &none())).key,
            "blueprint.bindingColocateCount"
        );
        // The colocated role's own selector must hold on the target's hosts.
        let mut tagged = hosts(4);
        tagged[1].tags = vec!["metrics".into()];
        let s = spec("native", &[("exporter", 3, tags(&["metrics"]))]);
        let m = err(place(&bp, &s, &tagged, &none()));
        assert_eq!(m.key, "blueprint.placementInsufficientHosts");
        assert_eq!(m.params["role"], "exporter");
        assert_eq!(m.params["needed"], "3");
        assert_eq!(m.params["matched"], "1");
        assert_eq!(m.params["hosts"], "host-2");
        // No target host passes: the list reads "-" rather than ending empty.
        let mut tagged = hosts(4);
        tagged[3].tags = vec!["metrics".into()];
        let m = err(place(&bp, &s, &tagged, &none()));
        assert_eq!(m.key, "blueprint.placementInsufficientHosts");
        assert_eq!(m.params["matched"], "0");
        assert_eq!(m.params["hosts"], "-");
        // Spread on a colocated role is ignored, without a warning.
        let s = spec("native", &[("exporter", 3, spread_sel("zone:"))]);
        let p = place(&bp, &s, &hosts(3), &none()).unwrap();
        assert_eq!(ids(&p.binding, "exporter"), [1, 2, 3]);
        assert!(p.warnings.is_empty());
        // A distinctHosts role cannot follow a target that shares hosts.
        let shared = blueprint(vec![
            role("web", 1, 3, 5, false, json!({})),
            role(
                "sidecar",
                1,
                3,
                5,
                false,
                json!({"colocateWith": "web", "distinctHosts": true}),
            ),
        ]);
        let m = err(place(&shared, &spec("native", &[]), &hosts(1), &none()));
        assert_eq!(m.key, "blueprint.bindingDistinctHosts");
        assert_eq!(m.params["role"], "sidecar");
        // Manual placement is kept verbatim and the colocated role follows it.
        let mut s = spec("native", &[]);
        s.placement = Some(BTreeMap::from([(
            "nats".to_string(),
            [4, 3, 1]
                .iter()
                .map(|id| InstanceBinding {
                    host_id: Id::from(*id),
                    address: format!("n{id}.internal"),
                })
                .collect(),
        )]));
        let p = place(&bp, &s, &cands, &none()).unwrap();
        assert_eq!(p.binding.roles["nats"][0].address, "n4.internal");
        assert_eq!(ids(&p.binding, "exporter"), [4, 3, 1]);
    }

    #[test]
    fn tags_are_and_semantics() {
        let bp = blueprint(vec![role(
            "web",
            1,
            1,
            10,
            false,
            json!({"distinctHosts": true}),
        )]);
        let cands = vec![
            host(1, &["cache"]),
            host(2, &["cache", "ssd"]),
            host(3, &["ssd"]),
            host(4, &["ssd", "cache", "gpu"]),
        ];
        let s = spec("native", &[("web", 2, tags(&["cache", "ssd"]))]);
        let p = place(&bp, &s, &cands, &none()).unwrap();
        assert_eq!(ids(&p.binding, "web"), [2, 4]);
        let s = spec("native", &[("web", 1, tags(&["cache", "nvme"]))]);
        let m = err(place(&bp, &s, &cands, &none()));
        assert_eq!(m.key, "blueprint.placementSelectorEmpty");
        assert_eq!(m.params["role"], "web");
        assert_eq!(m.params["selector"], "tags=[cache, nvme]");
    }

    #[test]
    fn names_and_group_intersect() {
        let bp = blueprint(vec![role(
            "web",
            1,
            1,
            10,
            false,
            json!({"distinctHosts": true}),
        )]);
        let mut cands = vec![
            host(1, &["edge"]),
            host(2, &["edge"]),
            host(3, &["edge"]),
            host(4, &[]),
        ];
        for c in &mut cands[1..] {
            c.group = Some("dc1".into());
        }
        let sel = HostSelector {
            tags: vec!["edge".into()],
            names: vec!["host-1".into(), "host-2".into(), "host-4".into()],
            group: Some("dc1".into()),
            spread: None,
        };
        let s = spec("native", &[("web", 1, sel.clone())]);
        let p = place(&bp, &s, &cands, &none()).unwrap();
        assert_eq!(ids(&p.binding, "web"), [2]);
        let s = spec("native", &[("web", 2, sel)]);
        let m = err(place(&bp, &s, &cands, &none()));
        assert_eq!(m.key, "blueprint.placementInsufficientHosts");
        assert_eq!(m.params["hosts"], "host-2");
        // Names alone, group alone.
        let names = HostSelector {
            names: vec!["host-4".into(), "host-1".into()],
            ..Default::default()
        };
        let s = spec("native", &[("web", 2, names)]);
        assert_eq!(
            ids(&place(&bp, &s, &cands, &none()).unwrap().binding, "web"),
            [1, 4]
        );
        let group = HostSelector {
            group: Some("dc1".into()),
            ..Default::default()
        };
        let s = spec("native", &[("web", 3, group)]);
        assert_eq!(
            ids(&place(&bp, &s, &cands, &none()).unwrap().binding, "web"),
            [2, 3, 4]
        );
    }

    #[test]
    fn offline_and_non_linux_excluded() {
        let bp = blueprint(vec![role(
            "web",
            1,
            1,
            10,
            false,
            json!({"distinctHosts": true}),
        )]);
        let mut cands = hosts(5);
        cands[0].online = false;
        cands[1].os = "windows".into();
        cands[2].os = String::new();
        cands[3].os = "Linux".into();
        let s = spec("native", &[("web", 3, Default::default())]);
        let p = place(&bp, &s, &cands, &none()).unwrap();
        assert_eq!(ids(&p.binding, "web"), [3, 4, 5]);
        let s = spec("native", &[("web", 4, Default::default())]);
        assert_eq!(
            err(place(&bp, &s, &cands, &none())).key,
            "blueprint.placementInsufficientHosts"
        );
        // Swarm manager: by name or id, and only an online Linux host.
        let mut s = spec("stack", &[("web", 1, Default::default())]);
        s.manager = Some("host-4".into());
        assert_eq!(
            place(&bp, &s, &cands, &none()).unwrap().binding.manager,
            Some(Id::from(4))
        );
        s.manager = Some("5".into());
        assert_eq!(
            place(&bp, &s, &cands, &none()).unwrap().binding.manager,
            Some(Id::from(5))
        );
        let mut dup = cands.clone();
        dup[4].name = "host-4".into();
        s.manager = Some("host-4".into());
        assert_eq!(
            err(place(&bp, &s, &dup, &none())).key,
            "blueprint.placementManagerUnavailable"
        );
        for bad in [Some("host-1"), Some("host-2"), Some("nope"), None] {
            s.manager = bad.map(String::from);
            let m = err(place(&bp, &s, &cands, &none()));
            assert_eq!(m.key, "blueprint.placementManagerUnavailable", "{bad:?}");
        }
    }

    #[test]
    fn insufficient_hosts_explained() {
        let bp = blueprint(vec![role(
            "nats",
            3,
            3,
            5,
            true,
            json!({"distinctHosts": true}),
        )]);
        let cands = vec![host(1, &["cache"]), host(2, &["cache"]), host(3, &[])];
        let s = spec("native", &[("nats", 3, tags(&["cache"]))]);
        let m = err(place(&bp, &s, &cands, &none()));
        assert_eq!(m.key, "blueprint.placementInsufficientHosts");
        assert_eq!(m.params["role"], "nats");
        assert_eq!(m.params["needed"], "3");
        assert_eq!(m.params["matched"], "2");
        assert_eq!(m.params["hosts"], "host-1, host-2");
        assert_eq!(m.params["selector"], "tags=[cache]");
        let reversed: Vec<Candidate> = cands.into_iter().rev().collect();
        let m = err(place(&bp, &s, &reversed, &none()));
        assert_eq!(m.params["hosts"], "host-1, host-2");
        // At most ten names are listed; the count stays exact.
        let bp = blueprint(vec![role(
            "web",
            1,
            20,
            30,
            false,
            json!({"distinctHosts": true}),
        )]);
        let s = spec("native", &[]);
        let m = err(place(&bp, &s, &hosts(12), &none()));
        assert_eq!(m.params["matched"], "12");
        assert_eq!(m.params["needed"], "20");
        let listed: Vec<&str> = m.params["hosts"].split(", ").collect();
        assert_eq!(listed.len(), 11);
        assert_eq!(listed[9], "host-10");
        assert_eq!(listed[10], "…");
        // Without distinctHosts a single host is enough.
        let bp = blueprint(vec![role("web", 1, 3, 5, false, json!({}))]);
        let p = place(&bp, &s, &hosts(1), &none()).unwrap();
        assert_eq!(ids(&p.binding, "web"), [1, 1, 1]);
        let p = place(&bp, &s, &hosts(2), &none()).unwrap();
        assert_eq!(ids(&p.binding, "web"), [1, 2, 1]);
    }

    fn spread_sel(prefix: &str) -> HostSelector {
        HostSelector {
            spread: Some(Spread {
                prefix: prefix.into(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn spread_prefers_different_zone_tags() {
        let bp = blueprint(vec![role(
            "nats",
            1,
            3,
            5,
            true,
            json!({"distinctHosts": true}),
        )]);
        let cands = vec![
            host(1, &["zone:a"]),
            host(2, &["zone:a"]),
            host(3, &["zone:b"]),
            host(4, &["zone:b"]),
            host(5, &["zone:c"]),
        ];
        let s = spec("native", &[("nats", 3, spread_sel("zone:"))]);
        let p = place(&bp, &s, &cands, &none()).unwrap();
        assert_eq!(ids(&p.binding, "nats"), [1, 3, 5]);
        assert!(p.warnings.is_empty());
        // Spread outranks load and memory.
        let existing = BTreeMap::from([(Id::from(5), 4)]);
        let p = place(&bp, &s, &cands, &existing).unwrap();
        assert_eq!(ids(&p.binding, "nats"), [1, 3, 5]);
    }

    #[test]
    fn spread_unmet_is_warning_not_error() {
        let bp = blueprint(vec![role(
            "nats",
            1,
            3,
            5,
            true,
            json!({"distinctHosts": true}),
        )]);
        let cands = vec![
            host(1, &["zone:a"]),
            host(2, &["zone:a"]),
            host(3, &["zone:b"]),
        ];
        let s = spec("native", &[("nats", 3, spread_sel("zone:"))]);
        let p = place(&bp, &s, &cands, &none()).unwrap();
        assert_eq!(ids(&p.binding, "nats"), [1, 3, 2]);
        assert_eq!(p.warnings.len(), 1);
        assert_eq!(p.warnings[0].key, "blueprint.placementSpreadUnmet");
        assert_eq!(p.warnings[0].params["role"], "nats");
        assert_eq!(p.warnings[0].params["prefix"], "zone:");
        // Hosts without any zone tag share one pseudo-zone.
        let s = spec("native", &[("nats", 3, spread_sel("rack:"))]);
        let p = place(&bp, &s, &cands, &none()).unwrap();
        assert_eq!(p.warnings[0].key, "blueprint.placementSpreadUnmet");
        let s = spec("native", &[("nats", 1, spread_sel("rack:"))]);
        assert!(place(&bp, &s, &cands, &none()).unwrap().warnings.is_empty());
    }

    #[test]
    fn prefers_more_free_memory_then_lower_id() {
        let bp = blueprint(vec![role(
            "web",
            1,
            1,
            10,
            false,
            json!({"distinctHosts": true}),
        )]);
        let mut cands = hosts(5);
        cands[0].mem_free = None;
        cands[1].mem_free = Some(100);
        cands[2].mem_free = Some(500);
        cands[3].mem_free = Some(100);
        cands[4].mem_free = None;
        let s = spec("native", &[("web", 5, Default::default())]);
        let p = place(&bp, &s, &cands, &none()).unwrap();
        assert_eq!(ids(&p.binding, "web"), [3, 2, 4, 1, 5]);
        // Load outranks memory.
        let existing = BTreeMap::from([(Id::from(3), 1)]);
        let s = spec("native", &[("web", 1, Default::default())]);
        let p = place(&bp, &s, &cands, &existing).unwrap();
        assert_eq!(ids(&p.binding, "web"), [2]);
    }

    #[test]
    fn spread_untagged_hosts_share_one_pseudo_zone() {
        let bp = blueprint(vec![role(
            "nats",
            1,
            3,
            5,
            true,
            json!({"distinctHosts": true}),
        )]);
        let cands = vec![
            host(1, &["zone:a"]),
            host(2, &["zone:a"]),
            host(3, &["zone:a"]),
            host(4, &[]),
            host(5, &[]),
        ];
        // Pick 1: all free, lowest id -> 1. Pick 2: zone:a hosts have 1 violation, untagged 0
        // -> 4. Pick 3: host 2 (zone:a holds 1) ties host 5 (untagged holds 1) -> lower id 2.
        let s = spec("native", &[("nats", 3, spread_sel("zone:"))]);
        let p = place(&bp, &s, &cands, &none()).unwrap();
        assert_eq!(ids(&p.binding, "nats"), [1, 4, 2]);
        assert_eq!(p.warnings.len(), 1);
        assert_eq!(p.warnings[0].key, "blueprint.placementSpreadUnmet");
    }

    #[test]
    fn compose_places_all_instances_on_one_host() {
        let bp = blueprint_with(
            vec![
                role("nats", 1, 3, 5, true, json!({"distinctHosts": true})),
                role("exporter", 1, 3, 5, false, json!({"colocateWith": "nats"})),
            ],
            true,
        );
        let mut cands = vec![
            host(1, &["metrics"]),
            host(2, &[]),
            host(3, &["metrics"]),
            host(4, &["metrics"]),
        ];
        for (c, mem) in cands.iter_mut().zip([100, 900, 500, 500]) {
            c.mem_free = Some(mem);
        }
        let s = spec("compose", &[("exporter", 3, tags(&["metrics"]))]);
        let existing = BTreeMap::from([(Id::from(3), 1)]);
        // Host 2 fails the exporter selector; host 3 carries load; host 4 beats host 1 on memory.
        let p = place(&bp, &s, &cands, &existing).unwrap();
        assert_eq!(ids(&p.binding, "nats"), [4, 4, 4]);
        assert_eq!(ids(&p.binding, "exporter"), [4, 4, 4]);
        assert_eq!(p.binding.roles["nats"][0].address, "10.0.0.4");
        assert_eq!(p.binding.manager, None);
        let info: BTreeMap<Id, super::super::binding::HostInfo> = cands
            .iter()
            .map(|h| {
                (
                    h.id,
                    super::super::binding::HostInfo {
                        id: h.id,
                        os: h.os.clone(),
                    },
                )
            })
            .collect();
        super::super::binding::validate(&bp, &p.binding, &info).expect("valid compose binding");
        // A manual placement fixes the host.
        let manual = |hosts: &[i64]| {
            let mut s = s.clone();
            s.placement = Some(BTreeMap::from([(
                "nats".to_string(),
                hosts
                    .iter()
                    .map(|id| InstanceBinding {
                        host_id: Id::from(*id),
                        address: format!("10.0.0.{id}"),
                    })
                    .collect(),
            )]));
            s
        };
        let p = place(&bp, &manual(&[1, 1, 1]), &cands, &none()).unwrap();
        assert_eq!(ids(&p.binding, "exporter"), [1, 1, 1]);
        for conflict in [&[2, 2, 2][..], &[1, 3, 1][..]] {
            assert_eq!(
                err(place(&bp, &manual(conflict), &cands, &none())).key,
                "blueprint.bindingComposeSingleHost"
            );
        }
        // No host satisfies every selector.
        let s = spec(
            "compose",
            &[
                ("nats", 3, tags(&["cache"])),
                ("exporter", 3, tags(&["metrics"])),
            ],
        );
        let m = err(place(&bp, &s, &cands, &none()));
        assert_eq!(m.key, "blueprint.placementSelectorEmpty");
        assert_eq!(m.params["role"], "nats");
    }

    #[test]
    fn compose_rejects_blueprint_without_single_host_support() {
        assert_eq!(
            err(place(&nats(), &spec("compose", &[]), &hosts(5), &none())).key,
            "blueprint.bindingComposeNeedsSingleHost"
        );
        // Without distinctHosts roles a single host is always acceptable.
        let plain = blueprint(vec![role("web", 1, 2, 3, false, json!({}))]);
        let p = place(&plain, &spec("compose", &[]), &hosts(3), &none()).unwrap();
        assert_eq!(ids(&p.binding, "web"), [1, 1]);
    }
}
