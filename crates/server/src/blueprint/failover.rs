//! Failover decisions: workload class, response per mode/driver, and the host-down verdict.
use super::model::{Identity, Replace, Role};
use super::spec::{DeploySpec, FailoverMode, FailoverSpec, HOST_DOWN_AFTER_DEFAULT};
use crate::snowflake::Id;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FailoverClass {
    Stateless,
    Replicated,
    Manual,
}

/// A = stateless, B = stable with replicated data; any other stable role needs a backup (P4).
pub fn class_of(role: &Role) -> FailoverClass {
    match (role.identity, role.failover.replace) {
        (Identity::Stateless, _) => FailoverClass::Stateless,
        (Identity::Stable, Replace::Supported) => FailoverClass::Replicated,
        (Identity::Stable, Replace::Unsupported) => FailoverClass::Manual,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum AlertReason {
    ModeAlert,
    NeedsBackup,
    DriverUnsupported,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Response {
    Ignore,
    AlertOnly(AlertReason),
    Replace(FailoverClass),
}

/// What failover does for one role when its host goes down. Only Ferry-scheduled drivers
/// (`native`/`docker`) are ever replaced; Swarm owns `stack` stateless roles.
pub fn respond(mode: FailoverMode, driver: &str, role: &Role) -> Response {
    if mode == FailoverMode::Off {
        return Response::Ignore;
    }
    let class = class_of(role);
    match driver {
        "native" | "docker" => {}
        "stack" if class == FailoverClass::Stateless => return Response::Ignore,
        _ => return Response::AlertOnly(AlertReason::DriverUnsupported),
    }
    match (mode, class) {
        (FailoverMode::Alert, _) => Response::AlertOnly(AlertReason::ModeAlert),
        (_, FailoverClass::Manual) => Response::AlertOnly(AlertReason::NeedsBackup),
        (_, class) => Response::Replace(class),
    }
}

/// The spec's failover settings. Deployments without a spec (P2a blueprints) only alert.
pub fn settings(spec: Option<&DeploySpec>) -> FailoverSpec {
    let default = |mode| FailoverSpec {
        mode,
        host_down_after: HOST_DOWN_AFTER_DEFAULT,
    };
    match spec {
        None => default(FailoverMode::Alert),
        Some(spec) => spec
            .failover
            .clone()
            .unwrap_or_else(|| default(FailoverMode::default())),
    }
}

/// `seen_at` (ms) is written by runner heartbeats or successful SSH monitor probes.
pub struct HostFacts {
    pub runner: bool,
    pub seen_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// TCP check of the host's SSH address through a healthy peer of the same deployment.
pub enum PeerCheck {
    Reachable(Id),
    Unreachable(Id),
    Inconclusive,
    NoPeer,
}

/// Second-opinion probes, gathered only once the host is stale.
pub struct Evidence {
    pub server_ok: bool,
    pub peer: Option<PeerCheck>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Healthy,
    NeedsProbe,
    Suspected,
    Down { single_source: bool },
    Vetoed(Id),
}

pub fn is_stale(facts: &HostFacts, now_ms: i64, after_secs: u32) -> bool {
    silent_for(facts, now_ms) > window_ms(after_secs)
}

fn silent_for(facts: &HostFacts, now_ms: i64) -> i64 {
    now_ms.saturating_sub(facts.seen_at)
}

fn window_ms(after_secs: u32) -> i64 {
    i64::from(after_secs) * 1000
}

/// Host-down verdict. A stale host needs the server probe to fail; SSH hosts also need a
/// peer to fail to reach it. Without a usable peer (runners have no inbound port) the host
/// must stay silent for twice the window, recorded as a single-source confirmation.
pub fn judge(
    facts: &HostFacts,
    now_ms: i64,
    after_secs: u32,
    evidence: Option<&Evidence>,
) -> Verdict {
    if !is_stale(facts, now_ms, after_secs) {
        return Verdict::Healthy;
    }
    let Some(evidence) = evidence else {
        return Verdict::NeedsProbe;
    };
    if evidence.server_ok {
        return Verdict::Healthy;
    }
    if !facts.runner {
        match evidence.peer {
            None => return Verdict::Suspected,
            Some(PeerCheck::Reachable(peer)) => return Verdict::Vetoed(peer),
            Some(PeerCheck::Unreachable(_)) => {
                return Verdict::Down {
                    single_source: false,
                };
            }
            Some(PeerCheck::NoPeer | PeerCheck::Inconclusive) => {}
        }
    }
    if silent_for(facts, now_ms) > 2 * window_ms(after_secs) {
        Verdict::Down {
            single_source: true,
        }
    } else {
        Verdict::Suspected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn role(identity: &str, replace: Option<&str>) -> Role {
        let mut v = json!({
            "name": "r",
            "instances": {"min": 1, "max": 3, "default": 1},
            "identity": identity,
            "service": {},
        });
        if let Some(replace) = replace {
            v["failover"] = json!({ "replace": replace });
        }
        serde_json::from_value(v).unwrap()
    }

    fn spec(failover: Option<Value>) -> DeploySpec {
        let mut v = json!({
            "template": {"id": "1", "version": 1},
            "driver": "native",
            "roles": {},
        });
        if let Some(f) = failover {
            v["failover"] = f;
        }
        serde_json::from_value(v).unwrap()
    }

    const AFTER: u32 = 90;
    const NOW: i64 = 10_000_000;
    const WINDOW: i64 = AFTER as i64 * 1000;

    fn ssh(stale_ms: i64) -> HostFacts {
        HostFacts {
            runner: false,
            seen_at: NOW - stale_ms,
        }
    }

    fn runner(stale_ms: i64) -> HostFacts {
        HostFacts {
            runner: true,
            seen_at: NOW - stale_ms,
        }
    }

    fn ev(server_ok: bool, peer: Option<PeerCheck>) -> Evidence {
        Evidence { server_ok, peer }
    }

    #[test]
    fn class_of_derives_a_b_manual() {
        assert_eq!(class_of(&role("stateless", None)), FailoverClass::Stateless);
        assert_eq!(
            class_of(&role("stable", Some("supported"))),
            FailoverClass::Replicated
        );
        assert_eq!(
            class_of(&role("stable", Some("unsupported"))),
            FailoverClass::Manual
        );
        assert_eq!(class_of(&role("stable", None)), FailoverClass::Manual);
        assert_eq!(
            class_of(&role("stateless", Some("supported"))),
            FailoverClass::Stateless
        );
        assert_eq!(
            serde_json::to_value(FailoverClass::Replicated).unwrap(),
            json!("replicated")
        );
    }

    #[test]
    fn respond_by_mode_and_driver() {
        use AlertReason::*;
        use FailoverMode::*;
        let a = role("stateless", None);
        let b = role("stable", Some("supported"));
        let m = role("stable", None);
        let cases: Vec<(FailoverMode, &str, &Role, Response)> = vec![
            // off ignores everything
            (Off, "native", &a, Response::Ignore),
            (Off, "docker", &b, Response::Ignore),
            (Off, "stack", &b, Response::Ignore),
            (Off, "compose", &m, Response::Ignore),
            // stack stateless belongs to Swarm in any mode
            (Auto, "stack", &a, Response::Ignore),
            (Alert, "stack", &a, Response::Ignore),
            // stack stateful and compose only alert
            (Auto, "stack", &b, Response::AlertOnly(DriverUnsupported)),
            (Alert, "stack", &m, Response::AlertOnly(DriverUnsupported)),
            (Auto, "compose", &a, Response::AlertOnly(DriverUnsupported)),
            (Alert, "compose", &b, Response::AlertOnly(DriverUnsupported)),
            // alert mode on ferry-scheduled drivers
            (Alert, "native", &a, Response::AlertOnly(ModeAlert)),
            (Alert, "docker", &b, Response::AlertOnly(ModeAlert)),
            (Alert, "docker", &m, Response::AlertOnly(ModeAlert)),
            // auto replaces A/B, manual needs a backup
            (
                Auto,
                "native",
                &a,
                Response::Replace(FailoverClass::Stateless),
            ),
            (
                Auto,
                "docker",
                &a,
                Response::Replace(FailoverClass::Stateless),
            ),
            (
                Auto,
                "native",
                &b,
                Response::Replace(FailoverClass::Replicated),
            ),
            (
                Auto,
                "docker",
                &b,
                Response::Replace(FailoverClass::Replicated),
            ),
            (Auto, "native", &m, Response::AlertOnly(NeedsBackup)),
            (Auto, "docker", &m, Response::AlertOnly(NeedsBackup)),
        ];
        for (mode, driver, role, want) in cases {
            assert_eq!(
                respond(mode, driver, role),
                want,
                "{mode:?} {driver} {:?}",
                class_of(role)
            );
        }
    }

    #[test]
    fn settings_default_without_spec_is_alert() {
        let legacy = settings(None);
        assert_eq!(
            legacy,
            FailoverSpec {
                mode: FailoverMode::Alert,
                host_down_after: 90
            }
        );
        // A spec without `failover` takes the field defaults.
        assert_eq!(
            settings(Some(&spec(None))),
            FailoverSpec {
                mode: FailoverMode::Auto,
                host_down_after: 90
            }
        );
        assert_eq!(
            settings(Some(&spec(Some(
                json!({"mode": "off", "hostDownAfter": 300})
            )))),
            FailoverSpec {
                mode: FailoverMode::Off,
                host_down_after: 300
            }
        );
    }

    #[test]
    fn fresh_host_is_healthy_without_probe() {
        for facts in [ssh(WINDOW), runner(WINDOW), ssh(0)] {
            assert!(!is_stale(&facts, NOW, AFTER));
            assert_eq!(judge(&facts, NOW, AFTER, None), Verdict::Healthy);
        }
        let stale = ssh(WINDOW + 1);
        assert!(is_stale(&stale, NOW, AFTER));
        assert_eq!(judge(&stale, NOW, AFTER, None), Verdict::NeedsProbe);
        assert_eq!(
            judge(&runner(WINDOW + 1), NOW, AFTER, None),
            Verdict::NeedsProbe
        );
    }

    #[test]
    fn server_probe_success_is_healthy() {
        let peer = Some(PeerCheck::Unreachable(Id::from(3)));
        assert_eq!(
            judge(&ssh(10 * WINDOW), NOW, AFTER, Some(&ev(true, peer))),
            Verdict::Healthy
        );
        assert_eq!(
            judge(&runner(10 * WINDOW), NOW, AFTER, Some(&ev(true, None))),
            Verdict::Healthy
        );
    }

    #[test]
    fn peer_reachable_vetoes() {
        let e = ev(false, Some(PeerCheck::Reachable(Id::from(7))));
        assert_eq!(
            judge(&ssh(10 * WINDOW), NOW, AFTER, Some(&e)),
            Verdict::Vetoed(Id::from(7))
        );
    }

    #[test]
    fn peer_unreachable_confirms_ssh() {
        let e = ev(false, Some(PeerCheck::Unreachable(Id::from(7))));
        // Two sources agree, so the single window is enough.
        assert_eq!(
            judge(&ssh(WINDOW + 1), NOW, AFTER, Some(&e)),
            Verdict::Down {
                single_source: false
            }
        );
        // Server failed but the peer check has not run yet: wait for it.
        assert_eq!(
            judge(&ssh(10 * WINDOW), NOW, AFTER, Some(&ev(false, None))),
            Verdict::Suspected
        );
    }

    #[test]
    fn runner_needs_twice_the_window() {
        let e = ev(false, None);
        assert_eq!(
            judge(&runner(WINDOW * 3 / 2), NOW, AFTER, Some(&e)),
            Verdict::Suspected
        );
        assert_eq!(
            judge(&runner(2 * WINDOW), NOW, AFTER, Some(&e)),
            Verdict::Suspected
        );
        assert_eq!(
            judge(&runner(2 * WINDOW + 1), NOW, AFTER, Some(&e)),
            Verdict::Down {
                single_source: true
            }
        );
    }

    #[test]
    fn ssh_without_peer_uses_single_source_rule() {
        for peer in [PeerCheck::NoPeer, PeerCheck::Inconclusive] {
            let e = ev(false, Some(peer));
            assert_eq!(
                judge(&ssh(WINDOW * 3 / 2), NOW, AFTER, Some(&e)),
                Verdict::Suspected,
                "{peer:?}"
            );
            assert_eq!(
                judge(&ssh(2 * WINDOW + 1), NOW, AFTER, Some(&e)),
                Verdict::Down {
                    single_source: true
                },
                "{peer:?}"
            );
        }
    }
}
