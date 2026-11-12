//! Swarm node lookup for `stack` bindings: every bound instance host must be a Ready/Active
//! node of the manager's Swarm, and `distinctHosts` is judged by Swarm node ID.
use super::binding::Binding;
use super::model::Blueprint;
use crate::deployments::quote;
use crate::error::{ApiError, Result};
use crate::hosts::channel::HostChannel;
use crate::i18n::Message;
use crate::snowflake::Id;
use axum::http::StatusCode;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub id: String,
    pub hostname: String,
    pub ready: bool,
    pub active: bool,
}

const TIMEOUT: Duration = Duration::from_secs(30);

fn fail<T>(message: Message) -> Result<T> {
    Err(ApiError::localized(StatusCode::BAD_REQUEST, message))
}

/// `swarmNodeUnavailable` for `host`; an empty name until [`on_host`] fills it in.
fn unavailable<T>(host: &str) -> Result<T> {
    fail(Message::new("blueprint.swarmNodeUnavailable").with("host", host))
}

/// Names the Ferry host in a `swarmNodeUnavailable` error from [`manager_nodes`] or
/// [`node_id`], or a `swarmLabelSyncFailed` from [`sync_labels`] (the manager), which only
/// see the channel; other errors pass through unchanged.
pub fn on_host(error: ApiError, host: &str) -> ApiError {
    match &error.2 {
        Some(m)
            if m.key == "blueprint.swarmNodeUnavailable"
                || m.key == "blueprint.swarmLabelSyncFailed" =>
        {
            ApiError::localized(error.0, m.clone().with("host", host))
        }
        _ => error,
    }
}

/// The manager's Swarm nodes; the host must be a Swarm manager (`ControlAvailable`).
pub async fn manager_nodes(ch: &dyn HostChannel) -> Result<Vec<Node>> {
    let control = ch
        .exec(
            "docker info --format '{{.Swarm.ControlAvailable}}'",
            TIMEOUT,
        )
        .await?;
    if control.code != 0 || control.stdout.trim() != "true" {
        return fail(Message::new("blueprint.swarmManagerRequired"));
    }
    let list = ch
        .exec("docker node ls --format '{{json .}}'", TIMEOUT)
        .await?;
    if list.code != 0 {
        return fail(Message::new("blueprint.swarmManagerRequired"));
    }
    parse_nodes(&list.stdout)
}

/// The Swarm node ID of the host behind `ch`; empty (not joined) is unavailable.
pub async fn node_id(ch: &dyn HostChannel) -> Result<String> {
    let info = ch
        .exec("docker info --format '{{.Swarm.NodeID}}'", TIMEOUT)
        .await?;
    let id = info.stdout.trim();
    if info.code != 0 || id.is_empty() || id.contains(char::is_whitespace) {
        return unavailable("");
    }
    Ok(id.to_string())
}

/// The Swarm node label standing for host tag `tag` of `workspace`:
/// `ferry.w<workspace>.tag.` + the first 12 hex characters of the SHA-256 of its name, so any
/// tag name makes a valid label. Scoped by workspace: two workspaces sharing a Swarm never
/// schedule by, or rewrite, each other's labels. A node carries it (value `true`) when its
/// Ferry host has the tag.
pub fn tag_label(workspace: Id, tag: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = hex::encode(Sha256::digest(tag.as_bytes()));
    format!("ferry.w{workspace}.tag.{}", &digest[..12])
}

/// `swarmLabelSyncFailed` for `node` (one or several node IDs); an empty host until
/// [`on_host`] names the manager.
fn label_sync_failed<T>(node: &str, detail: &str) -> Result<T> {
    fail(
        Message::new("blueprint.swarmLabelSyncFailed")
            .with("host", "")
            .with("node", node)
            .with("detail", detail.trim()),
    )
}

/// Brings the tag labels of Swarm nodes to `desired` (node ID → label → whether the node
/// carries it) through the manager behind `ch`. Only the listed labels of the listed nodes
/// are touched, and a node already in the desired state is left alone: the current labels
/// are read first (`--label-rm` of an absent label fails).
pub async fn sync_labels(
    ch: &dyn HostChannel,
    desired: &BTreeMap<String, BTreeMap<String, bool>>,
) -> Result<()> {
    if desired.is_empty() {
        return Ok(());
    }
    let ids: Vec<String> = desired.keys().map(|id| quote(id)).collect();
    let inspect = ch
        .exec(
            &format!(
                "docker node inspect --format '{{{{.ID}}}} {{{{json .Spec.Labels}}}}' {}",
                ids.join(" ")
            ),
            TIMEOUT,
        )
        .await?;
    if inspect.code != 0 {
        let all: Vec<&str> = desired.keys().map(String::as_str).collect();
        return label_sync_failed(&all.join(", "), &inspect.stderr);
    }
    let mut current: BTreeMap<&str, BTreeMap<String, String>> = BTreeMap::new();
    for line in inspect
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
    {
        let (id, labels) = line.split_once(' ').unwrap_or((line, "null"));
        let Ok(labels) = serde_json::from_str::<Option<BTreeMap<String, String>>>(labels) else {
            return label_sync_failed(id, line);
        };
        current.insert(id, labels.unwrap_or_default());
    }
    let none = BTreeMap::new();
    for (node, labels) in desired {
        let have = current.get(node.as_str()).unwrap_or(&none);
        let mut args = Vec::new();
        for (label, want) in labels {
            match (*want, have.get(label).map(String::as_str)) {
                (true, Some("true")) | (false, None) => {}
                (true, _) => args.push(format!("--label-add {}", quote(&format!("{label}=true")))),
                (false, Some(_)) => args.push(format!("--label-rm {}", quote(label))),
            }
        }
        if args.is_empty() {
            continue;
        }
        let out = ch
            .exec(
                &format!("docker node update {} {}", args.join(" "), quote(node)),
                TIMEOUT,
            )
            .await?;
        if out.code != 0 {
            return label_sync_failed(node, &out.stderr);
        }
    }
    Ok(())
}

/// One line of `docker node ls --format '{{json .}}'`.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct NodeLine {
    #[serde(rename = "ID")]
    id: String,
    hostname: String,
    status: String,
    availability: String,
}

fn parse_nodes(text: &str) -> Result<Vec<Node>> {
    let mut nodes = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Ok(n) = serde_json::from_str::<NodeLine>(line) else {
            return unavailable("");
        };
        if n.id.is_empty() {
            return unavailable("");
        }
        nodes.push(Node {
            id: n.id,
            hostname: n.hostname,
            ready: n.status.eq_ignore_ascii_case("ready"),
            active: n.availability.eq_ignore_ascii_case("active"),
        });
    }
    Ok(nodes)
}

/// Returns instance name → node ID; checks every instance host's node is in `nodes` and
/// Ready/Active, then re-checks `distinctHosts` by node ID (two Ferry hosts may be one node).
/// `host_names` is only used to name hosts in errors.
pub fn assign(
    bp: &Blueprint,
    b: &Binding,
    nodes: &[Node],
    host_nodes: &BTreeMap<Id, String>,
    host_names: &BTreeMap<Id, String>,
) -> Result<BTreeMap<String, String>> {
    let usable: BTreeSet<&str> = nodes
        .iter()
        .filter(|n| n.ready && n.active)
        .map(|n| n.id.as_str())
        .collect();
    let mut assigned = BTreeMap::new();
    for role in &bp.roles {
        let list = b
            .roles
            .get(&role.name)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut seen = BTreeSet::new();
        for (index, inst) in list.iter().enumerate() {
            let node = host_nodes
                .get(&inst.host_id)
                .filter(|id| usable.contains(id.as_str()));
            let Some(node) = node else {
                let name = host_names
                    .get(&inst.host_id)
                    .cloned()
                    .unwrap_or_else(|| inst.host_id.to_string());
                return unavailable(&name);
            };
            if role.placement.distinct_hosts && !seen.insert(node.as_str()) {
                return fail(
                    Message::new("blueprint.bindingDistinctHosts").with("role", &role.name),
                );
            }
            assigned.insert(format!("{}-{index}", role.name), node.clone());
        }
    }
    Ok(assigned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosts::fake;
    use crate::hosts::remote::Host;
    use serde_json::json;

    /// Real `docker node ls --format '{{json .}}'` output (Docker 27) from a 3-node Swarm.
    const NODE_LS: &str = concat!(
        r#"{"Availability":"Active","EngineVersion":"27.3.1","Hostname":"swarm-0","ID":"p3ft6t7g9h3x2hd8k1lqz0abc","ManagerStatus":"Leader","Self":true,"Status":"Ready","TLSStatus":"Ready"}"#,
        "\n",
        r#"{"Availability":"Drain","EngineVersion":"27.3.1","Hostname":"swarm-1","ID":"w1k9d0c6s2m4q8r7t5y3u1iop","ManagerStatus":"","Self":false,"Status":"Ready","TLSStatus":"Ready"}"#,
        "\n",
        r#"{"Availability":"Active","EngineVersion":"27.3.1","Hostname":"swarm-2","ID":"z8x7c6v5b4n3m2l1k0j9h8gfd","ManagerStatus":"","Self":false,"Status":"Down","TLSStatus":"Ready"}"#,
        "\n",
    );

    fn channel(id: i64) -> (std::sync::Arc<fake::FakeHost>, Box<dyn HostChannel>) {
        let host = fake::register(id);
        let ch = fake::open(&Host {
            id: Id::from(id),
            workspace_id: Id::from(1),
            credential_id: Id::from(0),
            name: format!("Host {id}"),
            address: "192.0.2.1".into(),
            port: 22,
            username: "test".into(),
            connection: "fake".into(),
            host_key: String::new(),
        })
        .expect("fake channel");
        (host, ch)
    }

    fn message(e: ApiError) -> Message {
        e.2.expect("localized error")
    }

    fn node(id: &str, ready: bool, active: bool) -> Node {
        Node {
            id: id.into(),
            hostname: format!("{id}.swarm"),
            ready,
            active,
        }
    }

    fn blueprint(distinct: bool) -> Blueprint {
        let text = json!({
            "apiVersion": "ferry/blueprint-v1",
            "kind": "Blueprint",
            "metadata": {"name": "demo"},
            "roles": [
                {"name": "nats", "instances": {"min": 1, "max": 5, "default": 3},
                 "placement": {"distinctHosts": distinct}, "identity": "stable",
                 "service": {"image": "nats"}},
                {"name": "web", "instances": {"min": 1, "max": 5, "default": 1},
                 "identity": "stateless", "service": {"image": "nginx"}},
            ],
            "drivers": {"stack": {"supported": true}},
        })
        .to_string();
        super::super::model::parse(&text).expect("blueprint")
    }

    fn binding(nats: &[i64], web: &[i64]) -> Binding {
        let list = |hosts: &[i64]| {
            hosts
                .iter()
                .map(|h| json!({"hostId": h.to_string(), "address": format!("10.0.0.{h}")}))
                .collect::<Vec<_>>()
        };
        super::super::binding::parse(
            &json!({"driver": "stack", "manager": "1",
                    "roles": {"nats": list(nats), "web": list(web)}})
            .to_string(),
        )
        .expect("binding")
    }

    fn ids(pairs: &[(i64, &str)]) -> BTreeMap<Id, String> {
        pairs
            .iter()
            .map(|(id, text)| (Id::from(*id), text.to_string()))
            .collect()
    }

    fn names() -> BTreeMap<Id, String> {
        ids(&[(1, "alpha"), (2, "bravo"), (3, "charlie"), (4, "delta")])
    }

    fn healthy() -> Vec<Node> {
        vec![
            node("n1", true, true),
            node("n2", true, true),
            node("n3", true, true),
        ]
    }

    #[test]
    fn node_ls_json_parsed() {
        let nodes = parse_nodes(NODE_LS).expect("parsed");
        assert_eq!(
            nodes,
            vec![
                Node {
                    id: "p3ft6t7g9h3x2hd8k1lqz0abc".into(),
                    hostname: "swarm-0".into(),
                    ready: true,
                    active: true,
                },
                Node {
                    id: "w1k9d0c6s2m4q8r7t5y3u1iop".into(),
                    hostname: "swarm-1".into(),
                    ready: true,
                    active: false,
                },
                Node {
                    id: "z8x7c6v5b4n3m2l1k0j9h8gfd".into(),
                    hostname: "swarm-2".into(),
                    ready: false,
                    active: true,
                },
            ]
        );
        assert_eq!(
            message(parse_nodes("not json\n").expect_err("garbage")).key,
            "blueprint.swarmNodeUnavailable"
        );
    }

    #[tokio::test]
    async fn manager_must_control_swarm() {
        // Worker: ControlAvailable is false.
        let (host, ch) = channel(931_001);
        host.respond("{{.Swarm.ControlAvailable}}", "false\n");
        let e = manager_nodes(ch.as_ref()).await.expect_err("worker");
        assert_eq!(message(e).key, "blueprint.swarmManagerRequired");
        // Not in a Swarm / no docker: empty output.
        let (_, ch) = channel(931_002);
        let e = manager_nodes(ch.as_ref()).await.expect_err("no swarm");
        assert_eq!(message(e).key, "blueprint.swarmManagerRequired");
        // Manager: the node list is read and parsed.
        let (host, ch) = channel(931_003);
        host.respond("{{.Swarm.ControlAvailable}}", "true\n");
        host.respond("docker node ls --format '{{json .}}'", NODE_LS);
        let nodes = manager_nodes(ch.as_ref()).await.expect("manager");
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0].hostname, "swarm-0");
    }

    #[tokio::test]
    async fn node_id_read_from_docker_info() {
        let (host, ch) = channel(931_004);
        host.respond("{{.Swarm.NodeID}}", "p3ft6t7g9h3x2hd8k1lqz0abc\n");
        assert_eq!(
            node_id(ch.as_ref()).await.expect("joined"),
            "p3ft6t7g9h3x2hd8k1lqz0abc"
        );
        let commands = host.commands.lock().unwrap().clone();
        assert_eq!(commands, vec!["docker info --format '{{.Swarm.NodeID}}'"]);
        let (host, ch) = channel(931_005);
        host.respond("{{.Swarm.NodeID}}", "\n");
        let e = node_id(ch.as_ref()).await.expect_err("not joined");
        let m = message(on_host(e, "alpha"));
        assert_eq!(m.key, "blueprint.swarmNodeUnavailable");
        assert_eq!(m.params["host"], "alpha");
    }

    #[test]
    fn bound_host_outside_swarm_rejected() {
        let host_nodes = ids(&[(1, "n1"), (2, "n2"), (3, "elsewhere")]);
        let m = message(
            assign(
                &blueprint(true),
                &binding(&[1, 2, 3], &[1]),
                &healthy(),
                &host_nodes,
                &names(),
            )
            .expect_err("outside"),
        );
        assert_eq!(m.key, "blueprint.swarmNodeUnavailable");
        assert_eq!(m.params["host"], "charlie");
        // A host whose node ID was never resolved is rejected the same way.
        let host_nodes = ids(&[(1, "n1"), (2, "n2")]);
        let m = message(
            assign(
                &blueprint(true),
                &binding(&[1, 2, 3], &[1]),
                &healthy(),
                &host_nodes,
                &names(),
            )
            .expect_err("unresolved"),
        );
        assert_eq!(m.key, "blueprint.swarmNodeUnavailable");
        assert_eq!(m.params["host"], "charlie");
    }

    #[test]
    fn drained_or_down_node_rejected() {
        let host_nodes = ids(&[(1, "n1"), (2, "n2"), (3, "n3")]);
        for nodes in [
            vec![
                node("n1", true, true),
                node("n2", true, false),
                node("n3", true, true),
            ],
            vec![
                node("n1", true, true),
                node("n2", false, true),
                node("n3", true, true),
            ],
        ] {
            let m = message(
                assign(
                    &blueprint(true),
                    &binding(&[1, 2, 3], &[1]),
                    &nodes,
                    &host_nodes,
                    &names(),
                )
                .expect_err("unavailable"),
            );
            assert_eq!(m.key, "blueprint.swarmNodeUnavailable");
            assert_eq!(m.params["host"], "bravo");
        }
    }

    #[test]
    fn same_node_twice_violates_distinct_hosts() {
        // Hosts 2 and 4 are two Ferry entries for the same Swarm node.
        let host_nodes = ids(&[(1, "n1"), (2, "n2"), (4, "n2")]);
        let m = message(
            assign(
                &blueprint(true),
                &binding(&[1, 2, 4], &[1]),
                &healthy(),
                &host_nodes,
                &names(),
            )
            .expect_err("same node"),
        );
        assert_eq!(m.key, "blueprint.bindingDistinctHosts");
        assert_eq!(m.params["role"], "nats");
        // Without distinctHosts the same node may run several instances.
        assign(
            &blueprint(false),
            &binding(&[1, 2, 4], &[1]),
            &healthy(),
            &host_nodes,
            &names(),
        )
        .expect("shared node allowed");
    }

    #[test]
    fn assign_maps_instances_to_node_ids() {
        let host_nodes = ids(&[(1, "n1"), (2, "n2"), (3, "n3")]);
        let map = assign(
            &blueprint(true),
            &binding(&[3, 1, 2], &[2, 2]),
            &healthy(),
            &host_nodes,
            &names(),
        )
        .expect("assigned");
        let expected: BTreeMap<String, String> = [
            ("nats-0", "n3"),
            ("nats-1", "n1"),
            ("nats-2", "n2"),
            ("web-0", "n2"),
            ("web-1", "n2"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        assert_eq!(map, expected);
    }

    /// The workspace of the label tests.
    fn ws() -> Id {
        Id::from(7)
    }

    #[test]
    fn label_name_is_stable_hash() {
        // `ferry.w<workspace>.tag.` + the first 12 lowercase hex characters of
        // sha256("cache") = 5e1ecee06a7fc06f...
        assert_eq!(tag_label(ws(), "cache"), "ferry.w7.tag.5e1ecee06a7f");
        // Any tag name is hashed over its UTF-8 bytes, so the label is always valid.
        assert_eq!(tag_label(ws(), "数据库"), "ferry.w7.tag.263200d7d529");
        assert_eq!(tag_label(ws(), "zone:a"), "ferry.w7.tag.349cd40b95a9");
        assert_ne!(tag_label(ws(), "cache"), tag_label(ws(), "Cache"));
        assert_eq!(
            tag_label(Id::from(42), "cache"),
            "ferry.w42.tag.5e1ecee06a7f"
        );
    }

    #[tokio::test]
    async fn labels_are_workspace_scoped() {
        let (w1, w2) = (Id::from(1), Id::from(2));
        let (mine, theirs) = (tag_label(w2, "mq"), tag_label(w1, "mq"));
        assert_ne!(mine, theirs);
        // Workspace 2 syncs `mq` on a node workspace 1 also labelled for its own `mq`: W1's
        // label is never added or removed, whether W2's host has the tag or not.
        for want in [false, true] {
            let (host, ch) = channel(931_020 + i64::from(want));
            host.respond(
                "docker node inspect",
                &format!(
                    "n1 {{\"{theirs}\":\"true\"}}
"
                ),
            );
            let desired =
                BTreeMap::from([("n1".to_string(), BTreeMap::from([(mine.clone(), want)]))]);
            sync_labels(ch.as_ref(), &desired).await.expect("synced");
            let commands = host.commands.lock().unwrap().clone();
            assert!(
                commands.iter().all(|c| !c.contains(&theirs)),
                "{commands:?}"
            );
            let updates: Vec<&String> = commands
                .iter()
                .filter(|c| c.starts_with("docker node update"))
                .collect();
            if want {
                assert_eq!(
                    updates,
                    [&format!(
                        "docker node update --label-add '{mine}=true' 'n1'"
                    )]
                );
            } else {
                assert!(updates.is_empty(), "{updates:?}");
            }
        }
    }

    fn desired(rows: &[(&str, &[(&str, bool)])]) -> BTreeMap<String, BTreeMap<String, bool>> {
        rows.iter()
            .map(|(node, labels)| {
                let labels = labels
                    .iter()
                    .map(|(tag, want)| (tag_label(ws(), tag), *want))
                    .collect();
                (node.to_string(), labels)
            })
            .collect()
    }

    #[tokio::test]
    async fn label_sync_adds_and_removes_on_manager() {
        let (cache, mq) = (tag_label(ws(), "cache"), tag_label(ws(), "mq"));
        let (host, ch) = channel(931_010);
        // n1 already matches; n2 has no labels; n3 carries a stale `mq` and an unrelated
        // label (and a `ferry.w<ws>.tag.` label of a tag this deployment does not use).
        let other = tag_label(ws(), "other");
        host.respond(
            "docker node inspect",
            &format!(
                "n1 {{\"{cache}\":\"true\",\"zone\":\"a\"}}\nn2 null\nn3 {{\"{mq}\":\"true\",\"zone\":\"b\",\"{other}\":\"true\"}}\n"
            ),
        );
        sync_labels(
            ch.as_ref(),
            &desired(&[
                ("n1", &[("cache", true), ("mq", false)]),
                ("n2", &[("cache", true), ("mq", true)]),
                ("n3", &[("cache", false), ("mq", false)]),
            ]),
        )
        .await
        .expect("synced");
        let commands = host.commands.lock().unwrap().clone();
        assert_eq!(
            commands,
            vec![
                "docker node inspect --format '{{.ID}} {{json .Spec.Labels}}' 'n1' 'n2' 'n3'"
                    .to_string(),
                format!(
                    "docker node update --label-add '{cache}=true' --label-add '{mq}=true' 'n2'"
                ),
                format!("docker node update --label-rm '{mq}' 'n3'"),
            ]
        );
        // Nothing to sync: no command at all.
        let (host, ch) = channel(931_011);
        sync_labels(ch.as_ref(), &BTreeMap::new())
            .await
            .expect("empty");
        assert!(host.commands.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn label_sync_failure_is_localized() {
        let (host, ch) = channel(931_012);
        host.respond("docker node inspect", "n1 null\n");
        host.fail_on("docker node update", "Error: node n1 not found");
        let e = sync_labels(ch.as_ref(), &desired(&[("n1", &[("cache", true)])]))
            .await
            .expect_err("update fails");
        let m = message(on_host(e, "alpha"));
        assert_eq!(m.key, "blueprint.swarmLabelSyncFailed");
        assert_eq!(m.params["node"], "n1");
        assert_eq!(m.params["host"], "alpha");
        assert_eq!(m.params["detail"], "Error: node n1 not found");
        for locale in crate::i18n::Locale::ALL {
            let text = crate::i18n::render(locale, &m);
            assert!(
                text.contains("alpha") && text.contains("n1"),
                "{locale:?}: {text}"
            );
        }
        // An unreadable label state stops before any update.
        let (host, ch) = channel(931_013);
        host.fail_on("docker node inspect", "permission denied");
        let e = sync_labels(
            ch.as_ref(),
            &desired(&[("n1", &[("cache", true)]), ("n2", &[("cache", false)])]),
        )
        .await
        .expect_err("inspect fails");
        let m = message(e);
        assert_eq!(m.key, "blueprint.swarmLabelSyncFailed");
        // Every requested node is named: the failing one is unknown.
        assert_eq!(m.params["node"], "n1, n2");
        assert_eq!(
            host.commands
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c.contains("docker node update"))
                .count(),
            0
        );
    }
}
