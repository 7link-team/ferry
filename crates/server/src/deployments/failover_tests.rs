//! Failover events API and the cleanup of events and fences.
use super::blueprint_tests::{body, create_app, fixture, host};
use super::events::{self, Kind, NewEvent};
use super::*;
use crate::agents::tests::admin;
use crate::entities::{deployment_events, deployment_fences};
use crate::i18n::{Locale, with_locale};
use sea_orm::{PaginatorTrait, Set};

fn suspected(deployment: Id, seq: usize) -> NewEvent {
    NewEvent {
        workspace: Id::from(1),
        deployment,
        kind: Kind::HostSuspected,
        instance: "nats-0".into(),
        host: Id::from(11),
        revision: Id::from(0),
        message: Message::new("deployment.event.hostSuspected").with("host", "Host 11"),
        data: json!({"seq": seq}),
    }
}

async fn raw_event(s: &AppState, id: i64, workspace: i64, deployment: i64) {
    deployment_events::Entity::insert(deployment_events::ActiveModel {
        id: Set(id),
        workspace_id: Set(workspace),
        deployment_id: Set(deployment),
        kind: Set(Kind::Fenced.as_str().into()),
        instance: Set("nats-1".into()),
        host_id: Set(12),
        detail: Set("{}".into()),
        revision_id: Set(0),
        created_at: Set(1),
    })
    .exec(&s.db)
    .await
    .unwrap();
}

async fn fence(s: &AppState, id: i64, deployment: i64, host: i64) {
    deployment_fences::Entity::insert(deployment_fences::ActiveModel {
        id: Set(id),
        workspace_id: Set(1),
        deployment_id: Set(deployment),
        host_id: Set(host),
        instance: Set("nats-0".into()),
        created_at: Set(1),
        cleared_at: Set(0),
    })
    .exec(&s.db)
    .await
    .unwrap();
}

async fn page(s: &AppState, scope: Scope, workspace: &str, id: &str, n: u64) -> Result<Value> {
    let raw = HashMap::from([
        ("page".to_owned(), n.to_string()),
        ("page_size".to_owned(), "10".to_owned()),
    ]);
    let Json(out) = events::list(
        State(s.clone()),
        scope,
        Path((workspace.into(), id.into())),
        Query(raw),
    )
    .await?;
    Ok(out)
}

#[tokio::test]
async fn events_api_paginates_and_is_workspace_scoped() {
    let (_dir, s) = fixture(&[11, 12]).await;
    let id = create_app(&s, body(&[11, 12])).await.unwrap();
    let app: Id = id.parse().unwrap();
    for seq in 0..25 {
        events::record(&s, &s.db, suspected(app, seq))
            .await
            .unwrap();
    }
    // Another deployment, and another workspace reusing this deployment's ID.
    raw_event(&s, 900_001, 1, 424_242).await;
    raw_event(&s, 900_002, 2, i64::from(app)).await;

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::ACCEPT_LANGUAGE,
        "ja-JP".parse().unwrap(),
    );
    let japanese = Locale::negotiate(&headers);
    let expected = crate::i18n::render(
        japanese,
        &Message::new("deployment.event.hostSuspected").with("host", "Host 11"),
    );
    assert_ne!(
        expected,
        crate::i18n::render(
            Locale::EnUs,
            &Message::new("deployment.event.hostSuspected").with("host", "Host 11")
        )
    );

    let mut seqs = Vec::new();
    for n in 1..=3 {
        let out = with_locale(japanese, page(&s, admin(1), "1", &id, n))
            .await
            .unwrap();
        assert_eq!(out["total"], 25);
        assert_eq!(
            (out["page"].as_u64(), out["page_size"].as_u64()),
            (Some(n), Some(10))
        );
        let items = out["items"].as_array().unwrap();
        assert_eq!(items.len(), if n == 3 { 5 } else { 10 });
        for item in items {
            assert_eq!(item["kind"], "hostSuspected");
            assert_eq!(item["instance"], "nats-0");
            assert_eq!(item["host_id"], "11");
            assert_eq!(item["host_name"], "Host 11");
            assert_eq!(item["revision_id"], "0");
            assert!(item["created_at"].as_i64().unwrap() > 0);
            assert!(item["id"].is_string());
            assert_eq!(item["message"], expected.as_str());
            assert!(item["detail"].get("message").is_none(), "{item}");
            seqs.push(item["detail"]["seq"].as_u64().unwrap());
        }
    }
    assert_eq!(seqs, (0..25).rev().collect::<Vec<u64>>());

    let err = page(&s, admin(2), "2", &id, 1).await.unwrap_err();
    assert_eq!(err.0, StatusCode::NOT_FOUND);
    let mut reader = admin(1);
    reader.permissions.remove("deployments.read");
    let err = page(&s, reader, "1", &id, 1).await.unwrap_err();
    assert_eq!(err.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn deleting_deployment_removes_events_and_fences() {
    let (_dir, s) = fixture(&[11, 12]).await;
    let id = create_app(&s, body(&[11, 12])).await.unwrap();
    let app: Id = id.parse().unwrap();
    events::record(&s, &s.db, suspected(app, 0)).await.unwrap();
    fence(&s, 800_001, i64::from(app), 11).await;
    raw_event(&s, 900_001, 1, 424_242).await;
    fence(&s, 800_002, 424_242, 11).await;

    remove(State(s.clone()), admin(1), Path(("1".into(), id.clone())))
        .await
        .unwrap();

    let events_left = deployment_events::Entity::find().all(&s.db).await.unwrap();
    assert_eq!(
        events_left.iter().map(|e| e.id).collect::<Vec<_>>(),
        [900_001]
    );
    let fences_left = deployment_fences::Entity::find().all(&s.db).await.unwrap();
    assert_eq!(
        fences_left.iter().map(|f| f.id).collect::<Vec<_>>(),
        [800_002]
    );
}

#[tokio::test]
async fn deleting_host_removes_its_fences() {
    let (_dir, s) = fixture(&[11]).await;
    host(&s, 13, 0).await;
    host(&s, 14, 0).await;
    fence(&s, 800_001, 424_242, 13).await;
    fence(&s, 800_002, 424_242, 14).await;
    fence(&s, 800_003, 424_242, 11).await;

    crate::hosts::delete_host(State(s.clone()), admin(1), Path(("1".into(), "13".into())))
        .await
        .unwrap();
    crate::hosts::bulk_delete_hosts(
        State(s.clone()),
        admin(1),
        Json(serde_json::from_value(json!({"ids": ["14"]})).unwrap()),
    )
    .await
    .unwrap();

    let left = deployment_fences::Entity::find().all(&s.db).await.unwrap();
    assert_eq!(left.iter().map(|f| f.host_id).collect::<Vec<_>>(), [11]);
    assert_eq!(
        deployment_events::Entity::find()
            .count(&s.db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn events_history_reads_one_instance_of_one_workspace() {
    let (_dir, s) = fixture(&[11]).await;
    let app = Id::from(424_242);
    let event = |kind, instance: &str| NewEvent {
        kind,
        instance: instance.into(),
        ..suspected(app, 0)
    };
    for kind in [
        Kind::ReplaceStarted,
        Kind::ReplaceFailed,
        Kind::NeedsAttention,
    ] {
        events::record(&s, &s.db, event(kind, "nats-0"))
            .await
            .unwrap();
    }
    // Another instance and another workspace never count.
    events::record(&s, &s.db, event(Kind::ReplaceFailed, "nats-1"))
        .await
        .unwrap();
    events::record(
        &s,
        &s.db,
        NewEvent {
            workspace: Id::from(2),
            ..event(Kind::ReplaceFailed, "nats-0")
        },
    )
    .await
    .unwrap();
    let h = events::history(&s.db, Id::from(1), app, "nats-0")
        .await
        .unwrap();
    assert_eq!((h.failures, h.attention, h.open), (1, true, false));
    assert!(h.last_started.is_some());
    let none = events::history(&s.db, Id::from(1), app, "nats-2")
        .await
        .unwrap();
    assert_eq!(none, events::InstanceHistory::default());
}

// ---- Replacing one instance (manual API) ----

use super::blueprint_tests::{
    bound_hosts, key, preview_of, revision, row, run_body, run_first, spec_body, tag_host,
    target_rows, update_spec,
};
use crate::entities::deployment_target_states;

const REPLACE_TEMPLATE: i64 = 9201;

/// NATS whose members replicate their data, so a member may be rebuilt on another host.
fn replicated() -> String {
    json!({
        "apiVersion": "ferry/blueprint-v1",
        "kind": "Blueprint",
        "metadata": {"name": "nats"},
        "roles": [{
            "name": "nats",
            "instances": {"min": 1, "max": 5, "default": 3},
            "placement": {"distinctHosts": true},
            "identity": "stable",
            "failover": {"replace": "supported"},
            "ports": [{"name": "client", "container": 4222, "published": 4222},
                      {"name": "cluster", "container": 6222, "published": 6222}],
            "service": {"image": "nats:{{ .Values.tag }}",
                        "files": [{"path": "/etc/nats.conf", "fromTemplate": "nats-conf"}]}
        }],
        "drivers": {"docker": {"supported": true}}
    })
    .to_string()
}

async fn replicated_template(s: &AppState) {
    use crate::entities::app_templates as t;
    t::Entity::insert(t::ActiveModel {
        id: Set(REPLACE_TEMPLATE),
        workspace_id: Set(1),
        name: Set("nats-replicated".into()),
        description: Set(String::new()),
        content: Set(replicated()),
        parameters: Set(r#"[{"name":"tag","type":"string","default":"2"}]"#.into()),
        version: Set(1),
        created_at: Set(1),
        updated_at: Set(1),
        ..Default::default()
    })
    .exec(&s.db)
    .await
    .unwrap();
}

fn replicated_spec(tag: &str) -> Value {
    json!({
        "apiVersion": "ferry/deploy-v1",
        "template": {"id": REPLACE_TEMPLATE.to_string(), "version": 1},
        "driver": "docker",
        "values": {"tag": tag},
        "roles": {"nats": {"instances": 3, "hosts": {"tags": ["mq"]}}}
    })
}

/// The fake runs no shell: put each host's staged artifact where the executed `cp` would.
async fn land(s: &AppState, id: &str) {
    let r = row(s, id).await;
    let dir = directory(&r);
    let rev = r.current_revision;
    for t in target_rows(s, &Id::from(rev).to_string()).await {
        let fake = crate::hosts::fake::get(t.host_id);
        let mut files = fake.files.lock().unwrap();
        let staged = files
            .get(&format!("{dir}/revision-{rev}.json"))
            .cloned()
            .expect("staged artifact");
        files.insert(format!("{dir}/compose.json"), staged);
    }
}

/// Four `mq` hosts; the spec's three members run on the first three, the last is spare.
async fn deployed(hosts: [i64; 4], spec: Value) -> (tempfile::TempDir, AppState, String) {
    let (dir, s) = fixture(&hosts).await;
    for h in hosts {
        tag_host(&s, "mq", h).await;
    }
    replicated_template(&s).await;
    let id = create_app(&s, spec_body(&spec)).await.unwrap();
    assert!(run_first(&s, &id, &[]).await);
    land(&s, &id).await;
    (dir, s, id)
}

async fn replace_api(
    s: &AppState,
    scope: Scope,
    id: &str,
    role: &str,
    index: u32,
    version: i64,
) -> Result<String> {
    let (status, Json(out)) = replace::replace(
        State(s.clone()),
        scope,
        Path(("1".into(), id.into(), role.into(), index)),
        Json(serde_json::from_value(json!({"version": version})).unwrap()),
    )
    .await?;
    assert_eq!(status, StatusCode::ACCEPTED);
    Ok(out["id"].as_str().unwrap().to_owned())
}

async fn replace_now(s: &AppState, id: &str, index: u32) -> String {
    let version = row(s, id).await.version;
    replace_api(s, admin(1), id, "nats", index, version)
        .await
        .unwrap()
}

/// Runs a queued revision to its end, as the scheduler would.
async fn run(s: &AppState, revision_id: &str) -> bool {
    let rev = revision(s, revision_id).await;
    let (ok, output) = execute(s, &rev).await.unwrap();
    finish(s, &rev, if ok { "succeeded" } else { "failed" }, &output)
        .await
        .unwrap();
    ok
}

fn app_id(id: &str) -> i64 {
    i64::from(id.parse::<Id>().unwrap())
}

async fn events_of(s: &AppState, id: &str) -> Vec<deployment_events::Model> {
    deployment_events::Entity::find()
        .filter(deployment_events::Column::DeploymentId.eq(app_id(id)))
        .order_by_asc(deployment_events::Column::Id)
        .all(&s.db)
        .await
        .unwrap()
}

async fn fences_of(s: &AppState, id: &str) -> Vec<deployment_fences::Model> {
    deployment_fences::Entity::find()
        .filter(deployment_fences::Column::DeploymentId.eq(app_id(id)))
        .order_by_asc(deployment_fences::Column::Id)
        .all(&s.db)
        .await
        .unwrap()
}

async fn state_hosts(s: &AppState, id: &str) -> Vec<i64> {
    deployment_target_states::Entity::find()
        .filter(deployment_target_states::Column::DeploymentId.eq(app_id(id)))
        .order_by_asc(deployment_target_states::Column::HostId)
        .all(&s.db)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.host_id)
        .collect()
}

fn nats_entries(binding: &str) -> Vec<Value> {
    let b: Value = serde_json::from_str(binding).unwrap();
    b["roles"]["nats"].as_array().unwrap().clone()
}

fn detail(event: &deployment_events::Model) -> Value {
    serde_json::from_str(&event.detail).unwrap()
}

fn docker_runs(host: i64) -> usize {
    crate::hosts::fake::get(host)
        .commands
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c.contains("docker run"))
        .count()
}

#[tokio::test]
async fn manual_replace_creates_manual_replace_revision() {
    let (_dir, s, id) = deployed([2011, 2012, 2013, 2014], replicated_spec("2")).await;
    let before = row(&s, &id).await;
    let rev = replace_now(&s, &id, 1).await;
    let r = revision(&s, &rev).await;
    assert_eq!(
        (r.action.as_str(), r.state.as_str()),
        ("manual-replace", "queued")
    );
    let after = row(&s, &id).await;
    let binding = after.blueprint_binding.clone().unwrap();
    assert_eq!(bound_hosts(&binding), ["2011", "2014", "2013"]);
    let (old, new) = (
        nats_entries(before.blueprint_binding.as_deref().unwrap()),
        nats_entries(&binding),
    );
    assert_eq!((&old[0], &old[2]), (&new[0], &new[2]));
    // The binding write and the queued revision each bump the version.
    assert_eq!(after.version, before.version + 2);
    let events = events_of(&s, &id).await;
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, ["replaceStarted", "fenced"]);
    for e in &events {
        assert_eq!((e.instance.as_str(), e.host_id), ("nats-1", 2012));
    }
    // The trigger is stored as a message key, rendered in each reader's language.
    let started = detail(&events[0]);
    assert!(
        started["message"]
            .as_str()
            .unwrap()
            .contains("deployment.replaceTrigger.manual"),
        "{started}"
    );
}

#[tokio::test]
async fn replace_targets_new_host_first_and_skips_unchanged() {
    let (_dir, s, id) = deployed([2021, 2022, 2023, 2024], replicated_spec("2")).await;
    let rev = replace_now(&s, &id, 1).await;
    let rows = target_rows(&s, &rev).await;
    assert_eq!(
        rows.iter().map(|t| t.host_id).collect::<Vec<_>>(),
        [2024, 2021, 2023]
    );
    for host in [2021, 2023, 2024] {
        crate::hosts::fake::get(host)
            .commands
            .lock()
            .unwrap()
            .clear();
    }
    assert!(run(&s, &rev).await);
    assert!(docker_runs(2024) > 0);
    assert_eq!((docker_runs(2021), docker_runs(2023)), (0, 0));
    for t in target_rows(&s, &rev).await {
        assert_eq!(t.state, "succeeded", "{}", t.host_id);
        assert_eq!(
            t.output.contains("deployment.output.hostUnchanged"),
            t.host_id != 2024,
            "{}",
            t.output
        );
    }
    // Unchanged hosts still point at the new revision.
    let current = groups::current(&s.db, Id::from(1), app_id(&id))
        .await
        .unwrap();
    for entry in current.as_array().unwrap() {
        assert_eq!(entry["current_revision"], rev.as_str());
    }
    // An ordinary apply never skips a host.
    let preview = preview_of(&s, &id, None).await.unwrap();
    let (_, Json(out)) = apply(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id.clone())),
        Json(run_body(&preview, "")),
    )
    .await
    .unwrap();
    let again = out["id"].as_str().unwrap().to_owned();
    assert!(run(&s, &again).await);
    assert!(docker_runs(2021) > 0 && docker_runs(2023) > 0);
}

#[tokio::test]
async fn manual_replace_stops_instance_on_reachable_old_host() {
    let (_dir, s, id) = deployed([2031, 2032, 2033, 2034], replicated_spec("2")).await;
    let project = project(&row(&s, &id).await);
    let rev = replace_now(&s, &id, 1).await;
    let stop = format!("docker rm -f '{project}-nats-1'");
    assert!(
        crate::hosts::fake::get(2032)
            .commands
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.contains(&stop)),
        "{stop}"
    );
    let fences = fences_of(&s, &id).await;
    assert_eq!(fences.len(), 1);
    assert_eq!(
        (fences[0].host_id, fences[0].instance.as_str()),
        (2032, "nats-1")
    );
    assert!(fences[0].cleared_at > 0);
    let fenced = events_of(&s, &id).await.pop().unwrap();
    assert_eq!(fenced.kind, "fenced");
    assert_eq!(detail(&fenced)["stopped"], true);
    assert!(run(&s, &rev).await);

    // An unreachable old host cannot be stopped: its fence stays until it returns.
    crate::hosts::fake::get(2031)
        .unreachable
        .store(true, std::sync::atomic::Ordering::SeqCst);
    replace_now(&s, &id, 0).await;
    let fences = fences_of(&s, &id).await;
    assert_eq!(fences.len(), 2);
    assert_eq!((fences[1].host_id, fences[1].cleared_at), (2031, 0));
    let fenced = events_of(&s, &id).await.pop().unwrap();
    assert_eq!((fenced.kind.as_str(), fenced.host_id), ("fenced", 2031));
    assert_eq!(detail(&fenced)["stopped"], false);
    // The cleared fence let 2032 take the instance.
    assert_eq!(
        bound_hosts(row(&s, &id).await.blueprint_binding.as_deref().unwrap()),
        ["2032", "2034", "2033"]
    );
}

#[tokio::test]
async fn manual_replace_rejects_stack_compose_and_manual_class() {
    let (_dir, s, id) = deployed([2041, 2042, 2043, 2044], replicated_spec("2")).await;
    let version = row(&s, &id).await.version;
    for mode in ["stack", "compose"] {
        apps::Entity::update_many()
            .col_expr(apps::Column::Mode, Expr::value(mode))
            .filter(apps::Column::Id.eq(app_id(&id)))
            .exec(&s.db)
            .await
            .unwrap();
        let error = replace_api(&s, admin(1), &id, "nats", 1, version)
            .await
            .unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        assert_eq!(key(error), "deployment.replaceUnsupportedDriver", "{mode}");
    }
    apps::Entity::update_many()
        .col_expr(apps::Column::Mode, Expr::value("docker"))
        .filter(apps::Column::Id.eq(app_id(&id)))
        .exec(&s.db)
        .await
        .unwrap();
    let error = replace_api(&s, admin(1), &id, "nats", 1, version - 1)
        .await
        .unwrap_err();
    assert_eq!(error.0, StatusCode::CONFLICT);
    assert_eq!(key(error), "deployment.error.030");
    let error = replace_api(&s, admin(1), &id, "nats", 3, version)
        .await
        .unwrap_err();
    assert_eq!(key(error), "blueprint.replaceInstanceUnknown");
    let error = replace_api(&s, admin(1), &id, "ghost", 0, version)
        .await
        .unwrap_err();
    assert_eq!(key(error), "blueprint.bindingRoleUnknown");
    // A stable role without replicated data needs a backup first (P4).
    let legacy = create_app(&s, body(&[2041, 2042])).await.unwrap();
    assert!(run_first(&s, &legacy, &[]).await);
    let v = row(&s, &legacy).await.version;
    let error = replace_api(&s, admin(1), &legacy, "nats", 1, v)
        .await
        .unwrap_err();
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert_eq!(key(error), "deployment.replaceNeedsBackup");
    // Nothing deployed yet: nothing to replace.
    let fresh = create_app(&s, spec_body(&replicated_spec("2")))
        .await
        .unwrap();
    let v = row(&s, &fresh).await.version;
    let error = replace_api(&s, admin(1), &fresh, "nats", 1, v)
        .await
        .unwrap_err();
    assert_eq!(key(error), "deployment.replaceNotDeployed");
    // Refusals record nothing and move nothing.
    for d in [&id, &legacy, &fresh] {
        assert!(events_of(&s, d).await.is_empty());
        assert!(fences_of(&s, d).await.is_empty());
    }
    let r = row(&s, &id).await;
    assert_eq!(
        bound_hosts(r.blueprint_binding.as_deref().unwrap()),
        ["2041", "2042", "2043"]
    );
    assert_eq!(r.version, version);
}

#[tokio::test]
async fn manual_replace_requires_apply_permission() {
    let (_dir, s, id) = deployed([2051, 2052, 2053, 2054], replicated_spec("2")).await;
    let version = row(&s, &id).await.version;
    let mut scope = admin(1);
    scope.permissions.remove("deployments.apply");
    let error = replace_api(&s, scope, &id, "nats", 1, version)
        .await
        .unwrap_err();
    assert_eq!(error.0, StatusCode::FORBIDDEN);
    // Another workspace cannot reach it either.
    let error = replace::replace(
        State(s.clone()),
        admin(2),
        Path(("2".into(), id.clone(), "nats".into(), 1)),
        Json(serde_json::from_value(json!({"version": version})).unwrap()),
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::NOT_FOUND);
    assert!(events_of(&s, &id).await.is_empty());
    assert_eq!(row(&s, &id).await.version, version);
}

#[tokio::test]
async fn spec_placement_mirrors_replaced_binding() {
    let manual = |hosts: [i64; 3], tag: &str| {
        let mut spec = replicated_spec(tag);
        spec["placement"] = json!({"nats": hosts.iter()
            .map(|h| json!({"hostId": h.to_string(), "address": "10.0.0.1"}))
            .collect::<Vec<_>>()});
        spec
    };
    let (_dir, s, id) = deployed([2061, 2062, 2063, 2064], manual([2061, 2062, 2063], "2")).await;
    hosts::Entity::update_many()
        .col_expr(hosts::Column::Address, Expr::value("198.51.100.64"))
        .filter(hosts::Column::Id.eq(2064))
        .exec(&s.db)
        .await
        .unwrap();
    let rev = replace_now(&s, &id, 1).await;
    let r = row(&s, &id).await;
    let expected = json!({"hostId": "2064", "address": "198.51.100.64"});
    assert_eq!(
        nats_entries(r.blueprint_binding.as_deref().unwrap())[1],
        expected
    );
    let stored = crate::blueprint::spec::parse(r.deploy_spec.as_deref().unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(&stored.placement.unwrap()["nats"][1]).unwrap(),
        expected
    );
    assert!(run(&s, &rev).await);
    // The user's spec now names the new host: a values change is an ordinary upgrade.
    update_spec(&s, &id, &manual([2061, 2064, 2063], "2.1"))
        .await
        .unwrap();
    let r = row(&s, &id).await;
    assert!(r.manifest.contains("nats:2.1"));
    assert_eq!(
        nats_entries(r.blueprint_binding.as_deref().unwrap())[1],
        expected
    );
    // Naming the old host again is a topology change.
    let error = update_spec(&s, &id, &manual([2061, 2062, 2063], "2.2"))
        .await
        .unwrap_err();
    assert_eq!(key(error), "blueprint.topologyChangeRequiresPlan");
}

#[tokio::test]
async fn replace_success_drops_old_host_target_state() {
    let (_dir, s, id) = deployed([2071, 2072, 2073, 2074], replicated_spec("2")).await;
    assert_eq!(state_hosts(&s, &id).await, [2071, 2072, 2073]);
    let rev = replace_now(&s, &id, 1).await;
    // Not before the revision succeeds.
    assert_eq!(state_hosts(&s, &id).await, [2071, 2072, 2073]);
    assert!(run(&s, &rev).await);
    assert_eq!(state_hosts(&s, &id).await, [2071, 2073, 2074]);
}

#[tokio::test]
async fn rollback_across_replace_rejected() {
    let (_dir, s, id) = deployed([2081, 2082, 2083, 2084], replicated_spec("2")).await;
    let first = Id::from(row(&s, &id).await.current_revision).to_string();
    let rev = replace_now(&s, &id, 1).await;
    assert!(run(&s, &rev).await);
    let error = preview_of(&s, &id, Some(first.clone())).await.unwrap_err();
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert_eq!(key(error), "deployment.rollbackAcrossReplace");
    let body = RunBody {
        version: row(&s, &id).await.version,
        expected_hash: "0".repeat(64),
        preview_token: String::new(),
        target_hashes: BTreeMap::new(),
        revision_id: first,
    };
    let error = rollback(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id.clone())),
        Json(body),
    )
    .await
    .unwrap_err();
    assert_eq!(key(error), "deployment.rollbackAcrossReplace");
    // The replacement revision itself may be rolled back to.
    let preview = preview_of(&s, &id, Some(rev.clone())).await.unwrap();
    rollback(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id.clone())),
        Json(run_body(&preview, &rev)),
    )
    .await
    .unwrap();
}
