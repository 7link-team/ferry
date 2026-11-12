//! Blueprint deployments reuse the host-group per-host target machinery end to end.
use super::*;
use crate::agents::tests::admin;
use crate::entities::deployment_revision_targets as targets;
use crate::platform_tools::tests::setup;
use sea_orm::{PaginatorTrait, Set};

const CONF: &str = "server_name={{ .Instance.Name }}\nroutes={{ range .Role \"nats\" }}{{ .Host }}:{{ .Port \"cluster\" }},{{ end }}\n";

fn blueprint(image: &str) -> String {
    json!({
        "apiVersion": "ferry/blueprint-v1",
        "kind": "Blueprint",
        "metadata": {"name": "nats"},
        "roles": [{
            "name": "nats",
            "instances": {"min": 1, "max": 5, "default": 2},
            "placement": {"distinctHosts": true},
            "identity": "stable",
            "ports": [{"name": "client", "container": 4222, "published": 4222},
                      {"name": "cluster", "container": 6222, "published": 6222}],
            "service": {"image": image,
                        "files": [{"path": "/etc/nats.conf", "fromTemplate": "nats-conf"}]}
        }],
        "drivers": {"docker": {"supported": true}}
    })
    .to_string()
}

fn binding(hosts: &[i64]) -> Value {
    let list: Vec<Value> = hosts
        .iter()
        .enumerate()
        .map(|(i, h)| json!({"hostId": h.to_string(), "address": format!("10.0.0.{}", i + 1)}))
        .collect();
    json!({"driver": "docker", "roles": {"nats": list}})
}

fn body(hosts: &[i64]) -> Value {
    json!({"name": "mq", "manifest": blueprint("nats:2"), "mode": "docker",
           "target_kind": "blueprint", "target_ref": "0", "workdir": "/srv",
           "blueprint_binding": binding(hosts)})
}

fn key(e: ApiError) -> String {
    e.2.map(|m| m.key).unwrap_or_else(|| e.1)
}

async fn host(s: &AppState, id: i64, group: i64) {
    crate::hosts::fake::register(id);
    hosts::Entity::insert(hosts::ActiveModel {
        id: Set(id),
        workspace_id: Set(1),
        name: Set(format!("Host {id}")),
        address: Set("192.0.2.1".into()),
        port: Set(22),
        username: Set("test".into()),
        connection: Set("fake".into()),
        state: Set("online".into()),
        group_id: Set(group),
        description: Set(String::new()),
        host_key: Set(String::new()),
        runner_key_hash: Set(String::new()),
        enroll_token: Set(String::new()),
        enroll_token_hash: Set(String::new()),
        seen_at: Set(0),
        created_at: Set(0),
        updated_at: Set(0),
        version: Set(1),
        os: Set("linux".into()),
        arch: Set(String::new()),
        agent_version: Set(String::new()),
        metrics: Set(String::new()),
        credential_id: Set(0),
        ..Default::default()
    })
    .exec(&s.db)
    .await
    .unwrap();
}

async fn template(s: &AppState, content: &str, version: i64) {
    use crate::entities::config_templates as t;
    t::Entity::delete_by_id(601).exec(&s.db).await.unwrap();
    t::Entity::insert(t::ActiveModel {
        id: Set(601),
        workspace_id: Set(1),
        name: Set("nats-conf".into()),
        description: Set(String::new()),
        content: Set(content.into()),
        parameters: Set("[]".into()),
        version: Set(version),
        created_at: Set(1),
        updated_at: Set(1),
        ..Default::default()
    })
    .exec(&s.db)
    .await
    .unwrap();
}

/// Workspace 1 with fake hosts and the instance-aware config template.
async fn fixture(hosts: &[i64]) -> (tempfile::TempDir, AppState) {
    let (dir, s) = setup().await;
    for id in hosts {
        host(&s, *id, 0).await;
    }
    template(&s, CONF, 1).await;
    (dir, s)
}

async fn create_app(s: &AppState, b: Value) -> Result<String> {
    let (_, Json(out)) = create(
        State(s.clone()),
        admin(1),
        Json(serde_json::from_value(b).unwrap()),
    )
    .await?;
    Ok(out["id"].as_str().unwrap().to_owned())
}

async fn row(s: &AppState, id: &str) -> apps::Model {
    scoped(&s.db, &admin(1), id).await.unwrap()
}

async fn preview_of(s: &AppState, id: &str, revision: Option<String>) -> Result<Value> {
    let Json(out) = preview(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id.into())),
        Json(PreviewBody {
            revision_id: revision,
        }),
    )
    .await?;
    Ok(out)
}

fn run_body(preview: &Value, revision: &str) -> RunBody {
    RunBody {
        version: preview["version"].as_i64().unwrap(),
        expected_hash: preview["expected_hash"].as_str().unwrap().into(),
        preview_token: preview["preview_token"].as_str().unwrap().into(),
        target_hashes: serde_json::from_value(preview["target_hashes"].clone()).unwrap(),
        revision_id: revision.into(),
    }
}

async fn apply_preview(s: &AppState, id: &str, preview: &Value) -> Result<String> {
    let (_, Json(out)) = apply(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id.into())),
        Json(run_body(preview, "")),
    )
    .await?;
    Ok(out["id"].as_str().unwrap().to_owned())
}

async fn revision(s: &AppState, id: &str) -> revisions::Model {
    revisions::Entity::find_by_id(i64::from(id.parse::<Id>().unwrap()))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
}

async fn target_rows(s: &AppState, revision: &str) -> Vec<targets::Model> {
    targets::Entity::find()
        .filter(targets::Column::RevisionId.eq(i64::from(revision.parse::<Id>().unwrap())))
        .order_by_asc(targets::Column::Position)
        .all(&s.db)
        .await
        .unwrap()
}

/// Marks a queued revision as having succeeded so it can be rolled back to.
async fn succeed(s: &AppState, app: &str, revision: &str) {
    let rev = i64::from(revision.parse::<Id>().unwrap());
    revisions::Entity::update_many()
        .col_expr(revisions::Column::State, Expr::value("succeeded"))
        .filter(revisions::Column::Id.eq(rev))
        .exec(&s.db)
        .await
        .unwrap();
    apps::Entity::update_many()
        .col_expr(apps::Column::State, Expr::value("succeeded"))
        .col_expr(apps::Column::CurrentRevision, Expr::value(rev))
        .filter(apps::Column::Id.eq(i64::from(app.parse::<Id>().unwrap())))
        .exec(&s.db)
        .await
        .unwrap();
}

fn services(artifact: &str) -> Vec<String> {
    let doc: Value = serde_json::from_str(artifact).unwrap();
    doc["services"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect()
}

#[tokio::test]
async fn create_blueprint_deployment_validates_binding() {
    let (_dir, s) = fixture(&[711, 712]).await;
    let id = create_app(&s, body(&[711, 712])).await.unwrap();
    let r = row(&s, &id).await;
    assert_eq!((r.target_kind.as_str(), r.target_ref), ("blueprint", 0));
    let stored: Value = serde_json::from_str(r.blueprint_binding.as_deref().unwrap()).unwrap();
    assert_eq!(stored["roles"]["nats"][1]["hostId"], "712");
    assert_eq!(app_json(&r)["blueprint_binding"]["driver"], "docker");

    let cases: Vec<(Value, &str)> = vec![
        (body(&[711, 999]), "blueprint.bindingHostUnknown"),
        (body(&[711, 711]), "blueprint.bindingDistinctHosts"),
        (
            {
                let mut b = body(&[711, 712]);
                b["mode"] = json!("compose");
                b
            },
            "blueprint.deploymentInvalid",
        ),
        (
            {
                let mut b = body(&[711, 712]);
                b["target_ref"] = json!("711");
                b
            },
            "blueprint.deploymentInvalid",
        ),
        (
            {
                let mut b = body(&[711, 712]);
                b.as_object_mut().unwrap().remove("blueprint_binding");
                b
            },
            "blueprint.deploymentInvalid",
        ),
        (
            {
                let mut b = body(&[711, 712]);
                b["blueprint_binding"] = json!({"driver": "docker"});
                b
            },
            "blueprint.bindingInvalid",
        ),
        (
            {
                let mut b = body(&[711, 712]);
                b["manifest"] = json!(json!({"apiVersion":"ferry/v2","kind":"Application","metadata":{"name":"x"},"services":[{"name":"web","image":"nginx:1"}]}).to_string());
                b
            },
            "blueprint.deploymentInvalid",
        ),
        (
            {
                let mut b = body(&[711, 712]);
                b["target_kind"] = json!("host");
                b["target_ref"] = json!("711");
                b["manifest"] = json!(json!({"apiVersion":"ferry/v2","kind":"Application","metadata":{"name":"x"},"services":[{"name":"web","image":"nginx:1"}]}).to_string());
                b
            },
            "blueprint.deploymentInvalid",
        ),
    ];
    for (b, expected) in cases {
        let error = create_app(&s, b.clone()).await.expect_err(expected);
        assert_eq!(key(error), expected, "{b}");
    }
}

#[tokio::test]
async fn blueprint_preview_produces_one_target_per_host() {
    let (_dir, s) = fixture(&[721, 722]).await;
    let id = create_app(&s, body(&[721, 722])).await.unwrap();
    let preview = preview_of(&s, &id, None).await.unwrap();
    let desired: Value = serde_json::from_str(preview["desired"].as_str().unwrap()).unwrap();
    let rows = desired["targets"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["host_id"], "721");
    assert_eq!(rows[1]["host_id"], "722");
    assert_eq!(services(rows[0]["artifact"].as_str().unwrap()), ["nats-0"]);
    assert_eq!(services(rows[1]["artifact"].as_str().unwrap()), ["nats-1"]);
    assert!(
        rows[0]["artifact"]
            .as_str()
            .unwrap()
            .contains(crate::hosts::fake::DIGEST)
    );
    // The union of every host's bindings: one rendered config per instance.
    let files: Vec<&Value> = preview["variables"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["kind"] == "file")
        .collect();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["service"], "nats-0");
    assert!(
        files[0]["content"]
            .as_str()
            .unwrap()
            .contains("server_name=nats-0")
    );
    assert!(
        files[1]["content"]
            .as_str()
            .unwrap()
            .contains("server_name=nats-1")
    );
    assert!(
        files[1]["content"]
            .as_str()
            .unwrap()
            .contains("routes=10.0.0.1:6222,10.0.0.2:6222,")
    );
    assert_eq!(preview["target_hashes"].as_object().unwrap().len(), 2);

    let rev = apply_preview(&s, &id, &preview).await.unwrap();
    let stored = target_rows(&s, &rev).await;
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].host_id, 721);
    assert_eq!(stored[0].artifact, rows[0]["artifact"].as_str().unwrap());
    assert_eq!(stored[1].artifact, rows[1]["artifact"].as_str().unwrap());
    // Each host's compiled application travels with the revision for execution and replay.
    let rendered: Value = serde_json::from_str(&revision(&s, &rev).await.rendered).unwrap();
    let manifest: Value =
        serde_json::from_str(rendered["targets"][1]["manifest"].as_str().unwrap()).unwrap();
    assert_eq!(rendered["targets"][1]["host_id"], "722");
    assert_eq!(manifest["services"][0]["name"], "nats-1");
}

#[tokio::test]
async fn blueprint_apply_rejects_stale_preview() {
    let (_dir, s) = fixture(&[731, 732]).await;
    let id = create_app(&s, body(&[731, 732])).await.unwrap();
    let preview = preview_of(&s, &id, None).await.unwrap();
    // A config template edit after preview changes one host's bindings.
    template(&s, &format!("{CONF}# edited\n"), 2).await;
    let error = apply_preview(&s, &id, &preview).await.unwrap_err();
    assert_eq!(key(error), "deployment.error.040");
    // A fresh preview is accepted.
    let preview = preview_of(&s, &id, None).await.unwrap();
    apply_preview(&s, &id, &preview).await.unwrap();
}

#[tokio::test]
async fn blueprint_preview_rejects_missing_host() {
    let (_dir, s) = fixture(&[741, 742]).await;
    let id = create_app(&s, body(&[741, 742])).await.unwrap();
    hosts::Entity::delete_by_id(742).exec(&s.db).await.unwrap();
    let error = preview_of(&s, &id, None).await.unwrap_err();
    assert_eq!(key(error), "blueprint.bindingHostUnknown");
}

#[tokio::test]
async fn binding_frozen_after_first_revision() {
    let (_dir, s) = fixture(&[751, 752, 753]).await;
    let id = create_app(&s, body(&[751, 752])).await.unwrap();
    let update_with = |b: Value| {
        let s = s.clone();
        let id = id.clone();
        async move {
            update(
                State(s.clone()),
                admin(1),
                Path(("1".into(), id.clone())),
                Json(serde_json::from_value(b).unwrap()),
            )
            .await
        }
    };
    // Before the first revision the topology may still change.
    let mut b = body(&[751, 753]);
    b["version"] = json!(row(&s, &id).await.version);
    update_with(b).await.unwrap();
    assert!(
        row(&s, &id)
            .await
            .blueprint_binding
            .unwrap()
            .contains("753")
    );

    let preview = preview_of(&s, &id, None).await.unwrap();
    let rev = apply_preview(&s, &id, &preview).await.unwrap();
    succeed(&s, &id, &rev).await;
    let mut b = body(&[751, 752]);
    b["version"] = json!(row(&s, &id).await.version);
    assert_eq!(
        key(update_with(b).await.unwrap_err()),
        "blueprint.topologyChangeRequiresPlan"
    );
    let mut b = body(&[751, 753]);
    b["version"] = json!(row(&s, &id).await.version);
    b["target_kind"] = json!("host-group");
    assert!(update_with(b).await.is_err());
    // Same topology, other fields: accepted.
    let mut b = body(&[751, 753]);
    b["version"] = json!(row(&s, &id).await.version);
    b["description"] = json!("same topology");
    update_with(b).await.unwrap();
    assert_eq!(row(&s, &id).await.description, "same topology");
}

async fn update_body(s: &AppState, id: &str, mut b: Value) -> Result<()> {
    b["version"] = json!(row(s, id).await.version);
    update(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id.into())),
        Json(serde_json::from_value(b).unwrap()),
    )
    .await
    .map(|_| ())
}

/// Applies an outdated preview: first as captured (stale record version), then re-sent with
/// the current version, which must still fail because the edit discarded the preview.
async fn assert_stale_preview_rejected(s: &AppState, id: &str, preview: &Value) {
    let error = apply_preview(s, id, preview).await.unwrap_err();
    assert_eq!(key(error), "deployment.error.030");
    let mut body = run_body(preview, "");
    body.version = row(s, id).await.version;
    let error = apply(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id.into())),
        Json(body),
    )
    .await
    .unwrap_err();
    assert_eq!(key(error), "deployment.error.040");
    assert_eq!(revision_count(s, id).await, 0);
}

async fn revision_count(s: &AppState, id: &str) -> u64 {
    revisions::Entity::find()
        .filter(revisions::Column::DeploymentId.eq(i64::from(id.parse::<Id>().unwrap())))
        .count(&s.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn blueprint_apply_rejects_preview_after_manifest_edit() {
    let (_dir, s) = fixture(&[831, 832]).await;
    let id = create_app(&s, body(&[831, 832])).await.unwrap();
    let preview = preview_of(&s, &id, None).await.unwrap();
    let mut b = body(&[831, 832]);
    b["manifest"] = json!(blueprint("nats:3"));
    update_body(&s, &id, b).await.unwrap();
    assert_stale_preview_rejected(&s, &id, &preview).await;
}

#[tokio::test]
async fn blueprint_apply_rejects_preview_after_binding_edit() {
    let (_dir, s) = fixture(&[833, 834, 835]).await;
    let id = create_app(&s, body(&[833, 834])).await.unwrap();
    let preview = preview_of(&s, &id, None).await.unwrap();
    update_body(&s, &id, body(&[833, 835])).await.unwrap();
    assert_stale_preview_rejected(&s, &id, &preview).await;
}

#[tokio::test]
async fn blueprint_preview_rejects_host_moved_to_other_workspace() {
    let (_dir, s) = fixture(&[836, 837]).await;
    let id = create_app(&s, body(&[836, 837])).await.unwrap();
    hosts::Entity::update_many()
        .col_expr(hosts::Column::WorkspaceId, Expr::value(2))
        .filter(hosts::Column::Id.eq(837))
        .exec(&s.db)
        .await
        .unwrap();
    let error = preview_of(&s, &id, None).await.unwrap_err();
    assert_eq!(key(error), "blueprint.bindingHostUnknown");
    assert_eq!(revision_count(&s, &id).await, 0);
    assert_eq!(row(&s, &id).await.preview_artifact, "");
}

#[tokio::test]
async fn blueprint_rejects_non_string_pipeline_reference() {
    let (_dir, s) = fixture(&[838]).await;
    // A numeric id would skip the name-to-id lookup (and its pipelines.read check) and only
    // become text later, during compilation.
    let mut manifest: Value = serde_json::from_str(&blueprint("nats:2")).unwrap();
    manifest["drivers"] = json!({"native": {"supported": true}});
    let service = &mut manifest["roles"][0]["service"];
    service.as_object_mut().unwrap().remove("image");
    service["artifact"] = json!({"source": "pipeline", "pipeline": 424242, "path": "bin/nats"});
    let mut b = body(&[838]);
    b["manifest"] = json!(manifest.to_string());
    b["mode"] = json!("native");
    b["blueprint_binding"]["driver"] = json!("native");
    let error = create_app(&s, b).await.unwrap_err();
    assert_eq!(key(error), "blueprint.serviceInvalid");
}

/// Applies and runs a first revision; `failing` hosts exit non-zero.
async fn run_first(s: &AppState, id: &str, failing: &[i64]) -> bool {
    for host in failing {
        crate::hosts::fake::get(*host)
            .fail
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
    let preview = preview_of(s, id, None).await.unwrap();
    let rev = revision(s, &apply_preview(s, id, &preview).await.unwrap()).await;
    let (ok, output) = execute(s, &rev).await.unwrap();
    finish(s, &rev, if ok { "succeeded" } else { "failed" }, &output)
        .await
        .unwrap();
    ok
}

#[tokio::test]
async fn binding_editable_after_first_revision_failed_everywhere() {
    let (_dir, s) = fixture(&[841, 842, 843]).await;
    let id = create_app(&s, body(&[841, 842])).await.unwrap();
    assert!(!run_first(&s, &id, &[841, 842]).await);
    assert_eq!(row(&s, &id).await.revision_number, 1);
    update_body(&s, &id, body(&[841, 843])).await.unwrap();
    assert!(
        row(&s, &id)
            .await
            .blueprint_binding
            .unwrap()
            .contains("843")
    );
}

#[tokio::test]
async fn binding_frozen_once_any_host_deployed() {
    // The first host succeeds and the second fails: the revision failed, but one host now
    // runs this topology.
    let (_dir, s) = fixture(&[844, 845, 846]).await;
    let id = create_app(&s, body(&[844, 845])).await.unwrap();
    assert!(!run_first(&s, &id, &[845]).await);
    assert_eq!(row(&s, &id).await.current_revision, 0);
    assert_eq!(
        key(update_body(&s, &id, body(&[844, 846])).await.unwrap_err()),
        "blueprint.topologyChangeRequiresPlan"
    );
    // A fully successful first revision freezes the binding as well.
    let (_dir2, s2) = fixture(&[847, 848, 849]).await;
    let id2 = create_app(&s2, body(&[847, 848])).await.unwrap();
    assert!(run_first(&s2, &id2, &[]).await);
    assert_eq!(
        key(update_body(&s2, &id2, body(&[847, 849])).await.unwrap_err()),
        "blueprint.topologyChangeRequiresPlan"
    );
}

#[tokio::test]
async fn free_text_name_and_rename_keep_compiled_hosts() {
    let (_dir, s) = fixture(&[771, 772]).await;
    let mut b = body(&[771, 772]);
    b["name"] = json!("NATS cluster");
    let id = create_app(&s, b).await.unwrap();
    let before = preview_of(&s, &id, None).await.unwrap();
    let desired: Value = serde_json::from_str(before["desired"].as_str().unwrap()).unwrap();
    let plans = groups::compiled(&row(&s, &id).await).unwrap();
    let manifest: Value = serde_json::from_str(&plans[0].manifest).unwrap();
    // Compiled applications are named by the blueprint, never by the deployment.
    assert_eq!(manifest["metadata"]["name"], "nats");
    assert_eq!(desired["targets"].as_array().unwrap().len(), 2);

    let mut b = body(&[771, 772]);
    b["name"] = json!("Messaging (renamed)");
    b["version"] = json!(row(&s, &id).await.version);
    update(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id.clone())),
        Json(serde_json::from_value(b).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(row(&s, &id).await.name, "Messaging (renamed)");
    let after = preview_of(&s, &id, None).await.unwrap();
    assert_eq!(after["desired"], before["desired"]);
    assert_eq!(after["target_hashes"], before["target_hashes"]);
    assert_eq!(after["expected_hash"], before["expected_hash"]);
}

#[tokio::test]
async fn blueprint_rollback_replays_stored_targets() {
    let (_dir, s) = fixture(&[761, 762]).await;
    let id = create_app(&s, body(&[761, 762])).await.unwrap();
    let preview = preview_of(&s, &id, None).await.unwrap();
    let first = apply_preview(&s, &id, &preview).await.unwrap();
    succeed(&s, &id, &first).await;
    let old = revision(&s, &first).await;
    let old_targets = target_rows(&s, &first).await;

    // The blueprint changes afterwards; rollback must not recompile it.
    let mut b = body(&[761, 762]);
    b["manifest"] = json!(blueprint("nats:3"));
    b["version"] = json!(row(&s, &id).await.version);
    update(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id.clone())),
        Json(serde_json::from_value(b).unwrap()),
    )
    .await
    .unwrap();

    let preview = preview_of(&s, &id, Some(first.clone())).await.unwrap();
    assert_eq!(preview["desired"].as_str().unwrap(), old.artifact);
    let (_, Json(out)) = rollback(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id.clone())),
        Json(run_body(&preview, &first)),
    )
    .await
    .unwrap();
    let back = out["id"].as_str().unwrap().to_owned();
    let replay = revision(&s, &back).await;
    assert_eq!(replay.artifact, old.artifact);
    assert_eq!(replay.rendered, old.rendered);
    assert_eq!(replay.manifest, old.manifest);
    let replayed = target_rows(&s, &back).await;
    assert_eq!(
        replayed
            .iter()
            .map(|t| (t.host_id, t.artifact.clone()))
            .collect::<Vec<_>>(),
        old_targets
            .iter()
            .map(|t| (t.host_id, t.artifact.clone()))
            .collect::<Vec<_>>()
    );
    assert!(replayed[0].artifact.contains("nats@sha256"));

    // Execution runs each host with its own stored application.
    let (ok, output) = execute(&s, &replay).await.unwrap();
    assert!(ok, "{output}");
    for (host, artifact) in [
        (761, &old_targets[0].artifact),
        (762, &old_targets[1].artifact),
    ] {
        let fake = crate::hosts::fake::get(host);
        let files = fake.files.lock().unwrap();
        let stage = files
            .iter()
            .find(|(path, _)| path.ends_with(&format!("revision-{}.json", replay.id)))
            .map(|(_, text)| text.clone())
            .expect("stage upload");
        assert_eq!(&stage, artifact);
    }
}

#[tokio::test]
async fn host_group_behaviour_unchanged() {
    let (_dir, s) = setup().await;
    crate::entities::host_groups::Entity::insert(crate::entities::host_groups::ActiveModel {
        id: Set(77),
        workspace_id: Set(1),
        name: Set("edge".into()),
        description: Set(String::new()),
        parent_id: Set(0),
        position: Set(0),
        created_at: Set(0),
        updated_at: Set(0),
        version: Set(1),
    })
    .exec(&s.db)
    .await
    .unwrap();
    host(&s, 771, 77).await;
    host(&s, 772, 77).await;
    let manifest = json!({"apiVersion":"ferry/v2","kind":"Application","metadata":{"name":"web"},
                          "services":[{"name":"web","image":"nginx:1"}]})
    .to_string();
    let id = create_app(
        &s,
        json!({"name":"web","manifest":manifest,"mode":"docker","target_kind":"host-group",
               "target_ref":"77","workdir":"/srv"}),
    )
    .await
    .unwrap();
    assert_eq!(row(&s, &id).await.blueprint_binding, None);
    let preview = preview_of(&s, &id, None).await.unwrap();
    let desired: Value = serde_json::from_str(preview["desired"].as_str().unwrap()).unwrap();
    let rows = desired["targets"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["artifact"], rows[1]["artifact"]);
    assert_eq!(desired["x-variable-bindings"], json!([]));
    let rev = apply_preview(&s, &id, &preview).await.unwrap();
    assert_eq!(target_rows(&s, &rev).await.len(), 2);
    let r = revision(&s, &rev).await;
    assert_eq!(r.manifest, row(&s, &id).await.manifest);
    let (ok, output) = execute(&s, &r).await.unwrap();
    assert!(ok, "{output}");
}

#[tokio::test]
async fn blueprint_role_files_keep_config_templates_referenced() {
    let (_dir, s) = fixture(&[781, 782]).await;
    create_app(&s, body(&[781, 782])).await.unwrap();
    let error = crate::templates::configs::delete(
        State(s.clone()),
        admin(1),
        Path(("1".into(), Id::from(601))),
    )
    .await
    .unwrap_err();
    assert_eq!(key(error), "templates.applicationReferenced");
}

#[tokio::test]
async fn blueprint_variables_are_found_in_compiled_applications() {
    let (_dir, s) = fixture(&[791, 792]).await;
    let id = create_app(&s, body(&[791, 792])).await.unwrap();
    let r = row(&s, &id).await;
    let unrelated = references_variable(&s.db, &r, 424242).await.unwrap();
    assert!(!unrelated);
    // Readers that only need compiled services do not depend on hosts still existing.
    hosts::Entity::delete_by_id(792).exec(&s.db).await.unwrap();
    assert_eq!(applications(&r).unwrap().len(), 2);
}

async fn config(s: &AppState, id: i64, name: &str, content: &str, parameters: &str) {
    use crate::entities::config_templates as t;
    t::Entity::insert(t::ActiveModel {
        id: Set(id),
        workspace_id: Set(1),
        name: Set(name.into()),
        description: Set(String::new()),
        content: Set(content.into()),
        parameters: Set(parameters.into()),
        version: Set(1),
        created_at: Set(1),
        updated_at: Set(1),
        ..Default::default()
    })
    .exec(&s.db)
    .await
    .unwrap();
}

async fn secret(s: &AppState, id: i64, name: &str, value: &str) {
    crate::entities::variables::Entity::insert(crate::entities::variables::ActiveModel {
        id: Set(id),
        workspace_id: Set(1),
        name: Set(name.into()),
        scope: Set("global".into()),
        scope_ref: Set(0),
        kind: Set("secret".into()),
        description: Set(String::new()),
        value: Set(s
            .vault
            .seal(value, &crate::vault::aad("1", "variables", &id.to_string()))
            .unwrap()),
        version: Set(1),
        created_at: Set(1),
        updated_at: Set(1),
    })
    .exec(&s.db)
    .await
    .unwrap();
}

/// Two roles on two hosts; each role's env and config use a secret of its own.
async fn two_roles(s: &AppState, hosts: [i64; 2]) -> String {
    const PASSWORD: &str = r#"[{"name":"password","type":"string","secret":true}]"#;
    config(
        s,
        611,
        "alpha-conf",
        "role=alpha\npassword={{ .Values.password }}\n",
        PASSWORD,
    )
    .await;
    config(
        s,
        612,
        "bravo-conf",
        "role=bravo\npassword={{ .Values.password }}\n",
        PASSWORD,
    )
    .await;
    secret(s, 621, "alpha-env", "ALPHA-ENV-SECRET").await;
    secret(s, 622, "alpha-file", "ALPHA-FILE-SECRET").await;
    secret(s, 623, "bravo-env", "BRAVO-ENV-SECRET").await;
    secret(s, 624, "bravo-file", "BRAVO-FILE-SECRET").await;
    let role = |name: &str| {
        json!({
            "name": name,
            "instances": {"min": 1, "max": 1, "default": 1},
            "identity": "stateless",
            "service": {
                "image": format!("{name}:1"),
                "env": [{"name": "TOKEN", "valueFrom": format!("variable/{name}-env")}],
                "files": [{"path": "/etc/app.conf", "fromTemplate": format!("{name}-conf"),
                           "values": {"password": format!("{name}-file")}}]
            }
        })
    };
    let manifest = json!({
        "apiVersion": "ferry/blueprint-v1",
        "kind": "Blueprint",
        "metadata": {"name": "pair"},
        "roles": [role("alpha"), role("bravo")],
        "drivers": {"docker": {"supported": true}}
    })
    .to_string();
    let binding = json!({"driver": "docker", "roles": {
        "alpha": [{"hostId": hosts[0].to_string(), "address": "10.0.0.1"}],
        "bravo": [{"hostId": hosts[1].to_string(), "address": "10.0.0.2"}]
    }});
    create_app(
        s,
        json!({"name": "pair", "manifest": manifest, "mode": "docker",
               "target_kind": "blueprint", "target_ref": "0", "workdir": "/srv",
               "blueprint_binding": binding}),
    )
    .await
    .unwrap()
}

fn uploaded(host: i64) -> String {
    let fake = crate::hosts::fake::get(host);
    let files = fake.files.lock().unwrap();
    files.values().cloned().collect::<Vec<_>>().join("\n")
}

#[tokio::test]
async fn blueprint_hosts_receive_only_their_own_files() {
    let (_dir, s) = fixture(&[801, 802]).await;
    let id = two_roles(&s, [801, 802]).await;
    let preview = preview_of(&s, &id, None).await.unwrap();
    let rev = revision(&s, &apply_preview(&s, &id, &preview).await.unwrap()).await;
    // The sealed revision snapshot still holds every host's files.
    let all = open_snapshot(&s.vault, &rev).unwrap();
    assert_eq!(all.len(), 4);
    let (ok, output) = execute(&s, &rev).await.unwrap();
    assert!(ok, "{output}");

    let alpha = uploaded(801);
    assert!(alpha.contains("ALPHA-ENV-SECRET") && alpha.contains("ALPHA-FILE-SECRET"));
    assert!(!alpha.contains("BRAVO"), "{alpha}");
    let bravo = uploaded(802);
    assert!(bravo.contains("BRAVO-ENV-SECRET") && bravo.contains("BRAVO-FILE-SECRET"));
    assert!(!bravo.contains("ALPHA"), "{bravo}");
    // Each host's file set is recorded with its application for replay.
    let rendered: Value = serde_json::from_str(&rev.rendered).unwrap();
    for target in rendered["targets"].as_array().unwrap() {
        assert_eq!(target["files"].as_array().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn blueprint_execution_rejects_damaged_targets_before_running() {
    let (_dir, s) = fixture(&[811, 812]).await;
    let id = create_app(&s, body(&[811, 812])).await.unwrap();
    let preview = preview_of(&s, &id, None).await.unwrap();
    let applied = apply_preview(&s, &id, &preview).await.unwrap();
    let rev = revision(&s, &applied).await;
    let mut rendered: Value = serde_json::from_str(&rev.rendered).unwrap();
    let missing = {
        let mut doc = rendered.clone();
        doc["targets"].as_array_mut().unwrap().pop();
        doc.to_string()
    };
    rendered["targets"][0]["files"] = json!([".config-unknown.txt"]);
    for damaged in [missing, rendered.to_string(), "not json".to_owned()] {
        let mut broken = rev.clone();
        broken.rendered = damaged;
        let (ok, output) = execute(&s, &broken).await.unwrap();
        assert!(!ok);
        assert!(output.contains("deployment.error.063"), "{output}");
        let states: Vec<String> = target_rows(&s, &applied)
            .await
            .into_iter()
            .map(|t| t.state)
            .collect();
        assert_eq!(states, ["pending", "pending"]);
    }
    // Finishing the failed revision settles the untouched targets as skipped.
    finish(&s, &rev, "failed", "").await.unwrap();
    assert!(
        target_rows(&s, &applied)
            .await
            .iter()
            .all(|t| t.state == "skipped")
    );
    assert!(
        crate::hosts::fake::get(811)
            .files
            .lock()
            .unwrap()
            .is_empty()
    );
}

// ---------------------------------------------------------------- stack (Docker Swarm)

/// Offsets of a Swarm's manager (`base`) and its three workers from a test's `base` host id;
/// each host's Swarm node ID is `node-<host>`.
const SWARM: [i64; 4] = [0, 1, 2, 3];

fn stack_blueprint() -> String {
    json!({
        "apiVersion": "ferry/blueprint-v1",
        "kind": "Blueprint",
        "metadata": {"name": "nats"},
        "roles": [{
            "name": "nats",
            "instances": {"min": 1, "max": 5, "default": 3},
            "placement": {"distinctHosts": true},
            "identity": "stable",
            "ports": [{"name": "client", "container": 4222, "published": 4222},
                      {"name": "cluster", "container": 6222, "published": 6222}],
            "service": {"image": "nats:2",
                        "files": [{"path": "/etc/nats.conf", "fromTemplate": "nats-conf"}]}
        }],
        "drivers": {"stack": {"supported": true}}
    })
    .to_string()
}

/// NATS on the three workers, managed by `base`.
fn stack_body(base: i64) -> Value {
    let mut binding = binding(&[base + 1, base + 2, base + 3]);
    binding["driver"] = json!("stack");
    binding["manager"] = json!(base.to_string());
    json!({"name": "mq", "manifest": stack_blueprint(), "mode": "stack",
           "target_kind": "blueprint", "target_ref": "0", "workdir": "/srv",
           "blueprint_binding": binding})
}

fn node_line(host: i64) -> String {
    json!({"ID": format!("node-{host}"), "Hostname": format!("h{host}"),
           "Status": "Ready", "Availability": "Active"})
    .to_string()
}

/// Canned `docker` answers: the manager lists every node; each host reports its own ID.
fn join_swarm(base: i64) {
    let list: Vec<String> = SWARM.iter().map(|i| node_line(base + i)).collect();
    let manager = crate::hosts::fake::get(base);
    manager.respond("Swarm.ControlAvailable", "true\n");
    manager.respond("docker node ls", &(list.join("\n") + "\n"));
    for i in SWARM {
        crate::hosts::fake::get(base + i).respond("Swarm.NodeID", &format!("node-{}\n", base + i));
    }
}

async fn stack_fixture(base: i64) -> (tempfile::TempDir, AppState) {
    let hosts: Vec<i64> = SWARM.iter().map(|i| base + i).collect();
    fixture(&hosts).await
}

fn commands(host: i64, pattern: &str) -> usize {
    crate::hosts::fake::get(host)
        .commands
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c.contains(pattern))
        .count()
}

#[tokio::test]
async fn stack_blueprint_preview_single_manager_target() {
    let base = 910;
    let (_dir, s) = stack_fixture(base).await;
    join_swarm(base);
    let id = create_app(&s, stack_body(base)).await.unwrap();
    let preview = preview_of(&s, &id, None).await.unwrap();
    let desired: Value = serde_json::from_str(preview["desired"].as_str().unwrap()).unwrap();
    let rows = desired["targets"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["host_id"], base.to_string());
    let artifact = rows[0]["artifact"].as_str().unwrap();
    assert_eq!(services(artifact), ["nats-0", "nats-1", "nats-2"]);
    // Each stable instance is pinned to its bound host's Swarm node.
    for host in [base + 1, base + 2, base + 3] {
        assert!(
            artifact.contains(&format!("node.id==node-{host}")),
            "{artifact}"
        );
    }
    assert_eq!(preview["target_hashes"].as_object().unwrap().len(), 1);
    // A preview asks each host once; its in-transaction recheck reuses the answers.
    assert_eq!(commands(base, "docker node ls"), 1);
    for host in [base + 1, base + 2, base + 3] {
        assert_eq!(commands(host, "Swarm.NodeID"), 1);
    }

    let rev = apply_preview(&s, &id, &preview).await.unwrap();
    // Applying looks the nodes up once more, outside its transaction.
    assert_eq!(commands(base, "docker node ls"), 2);
    let stored = target_rows(&s, &rev).await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].host_id, base);
    assert_eq!(stored[0].artifact, artifact);
    let (ok, output) = execute(&s, &revision(&s, &rev).await).await.unwrap();
    assert!(ok, "{output}");
    // The stack is deployed through the manager only.
    assert_eq!(commands(base, "docker stack deploy"), 1);
    for host in [base + 1, base + 2, base + 3] {
        assert_eq!(commands(host, "docker stack deploy"), 0);
    }
}

#[tokio::test]
async fn stack_blueprint_rejects_host_outside_swarm_at_preview() {
    let base = 920;
    let (_dir, s) = stack_fixture(base).await;
    join_swarm(base);
    // The third worker reports a node the manager does not list.
    let outsider = crate::hosts::fake::register(base + 3);
    outsider.respond("Swarm.NodeID", "node-elsewhere\n");
    let id = create_app(&s, stack_body(base)).await.unwrap();
    let message = preview_of(&s, &id, None).await.unwrap_err().2.unwrap();
    assert_eq!(message.key, "blueprint.swarmNodeUnavailable");
    assert_eq!(message.params["host"], format!("Host {}", base + 3));
    assert_eq!(row(&s, &id).await.preview_artifact, "");
    // An unreachable host keeps its own connection error rather than a Swarm one.
    crate::hosts::fake::unregister(base + 3);
    let e = preview_of(&s, &id, None).await.unwrap_err();
    assert_eq!(e.0, axum::http::StatusCode::BAD_GATEWAY);
    assert_eq!(e.2.unwrap().key, "ssh.unreachable");
    // A manager host that is not a Swarm manager, named in every locale's message.
    crate::hosts::fake::register(base);
    let message = preview_of(&s, &id, None).await.unwrap_err().2.unwrap();
    assert_eq!(message.key, "blueprint.swarmManagerRequired");
    assert_eq!(message.params["host"], format!("Host {base}"));
    for locale in crate::i18n::Locale::ALL {
        let text = crate::i18n::render(locale, &message);
        assert!(text.contains(&format!("Host {base}")), "{locale:?}: {text}");
    }
}

#[tokio::test]
async fn stack_blueprint_create_offline_then_preview_validates_nodes() {
    let base = 930;
    let (_dir, s) = stack_fixture(base).await;
    // Every host is offline: creation checks only what needs no host.
    for i in SWARM {
        crate::hosts::fake::unregister(base + i);
    }
    let id = create_app(&s, stack_body(base)).await.unwrap();
    let r = row(&s, &id).await;
    assert_eq!(
        (r.target_kind.as_str(), r.mode.as_str()),
        ("blueprint", "stack")
    );
    // Offline readers see the manager's single application.
    assert_eq!(groups::compiled(&r).unwrap().len(), 1);
    assert_eq!(applications(&r).unwrap()[0].services.len(), 3);
    // Faults that need no host are still rejected when stored.
    let mut b = stack_body(base);
    b["blueprint_binding"]
        .as_object_mut()
        .unwrap()
        .remove("manager");
    assert_eq!(
        key(create_app(&s, b).await.unwrap_err()),
        "blueprint.bindingInvalid"
    );
    let mut b = stack_body(base);
    b["blueprint_binding"]["manager"] = json!("999");
    assert_eq!(
        key(create_app(&s, b).await.unwrap_err()),
        "blueprint.bindingHostUnknown"
    );
    // Edits stay offline too.
    update_body(&s, &id, stack_body(base)).await.unwrap();

    // Preview needs the hosts: an offline manager reports why it cannot be reached.
    let message = preview_of(&s, &id, None).await.unwrap_err().2.unwrap();
    assert_eq!(message.key, "ssh.unreachable");
    // A reachable manager outside any Swarm is a Swarm error naming the host.
    crate::hosts::fake::register(base);
    let message = preview_of(&s, &id, None).await.unwrap_err().2.unwrap();
    assert_eq!(message.key, "blueprint.swarmManagerRequired");
    assert_eq!(message.params["host"], format!("Host {base}"));
    for i in SWARM {
        crate::hosts::fake::register(base + i);
    }
    join_swarm(base);
    let preview = preview_of(&s, &id, None).await.unwrap();
    let desired: Value = serde_json::from_str(preview["desired"].as_str().unwrap()).unwrap();
    assert_eq!(desired["targets"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn stack_blueprint_rollback_replays_without_node_lookup() {
    let base = 940;
    let (_dir, s) = stack_fixture(base).await;
    join_swarm(base);
    let id = create_app(&s, stack_body(base)).await.unwrap();
    let preview = preview_of(&s, &id, None).await.unwrap();
    let first = apply_preview(&s, &id, &preview).await.unwrap();
    succeed(&s, &id, &first).await;
    let old = revision(&s, &first).await;
    let old_targets = target_rows(&s, &first).await;

    // The workers leave the Swarm: a replay must not need node lookups.
    for i in SWARM {
        let fake = crate::hosts::fake::get(base + i);
        fake.commands.lock().unwrap().clear();
        fake.responses
            .lock()
            .unwrap()
            .retain(|(pattern, _)| pattern != "Swarm.NodeID");
    }
    let preview = preview_of(&s, &id, Some(first.clone())).await.unwrap();
    assert_eq!(preview["desired"].as_str().unwrap(), old.artifact);
    let (_, Json(out)) = rollback(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id.clone())),
        Json(run_body(&preview, &first)),
    )
    .await
    .unwrap();
    let back = out["id"].as_str().unwrap().to_owned();
    let replay = revision(&s, &back).await;
    assert_eq!(replay.rendered, old.rendered);
    let replayed = target_rows(&s, &back).await;
    assert_eq!(replayed.len(), 1);
    assert_eq!(replayed[0].host_id, base);
    assert_eq!(replayed[0].artifact, old_targets[0].artifact);
    let (ok, output) = execute(&s, &replay).await.unwrap();
    assert!(ok, "{output}");
    assert_eq!(commands(base, "docker stack deploy"), 1);
    for i in SWARM {
        assert_eq!(commands(base + i, "docker node ls"), 0);
        assert_eq!(commands(base + i, "Swarm.NodeID"), 0);
    }
}

#[tokio::test]
async fn stack_apply_rejects_changed_swarm_node() {
    let base = 950;
    let (_dir, s) = stack_fixture(base).await;
    join_swarm(base);
    let id = create_app(&s, stack_body(base)).await.unwrap();
    let preview = preview_of(&s, &id, None).await.unwrap();
    // The second worker leaves and rejoins the Swarm under a new node ID.
    let rejoined = |host: i64| {
        if host == base + 2 {
            format!("node-{host}-new")
        } else {
            format!("node-{host}")
        }
    };
    let list: Vec<String> = SWARM
        .iter()
        .map(|i| {
            json!({"ID": rejoined(base + i), "Hostname": format!("h{}", base + i),
                   "Status": "Ready", "Availability": "Active"})
            .to_string()
        })
        .collect();
    for i in SWARM {
        crate::hosts::fake::get(base + i)
            .responses
            .lock()
            .unwrap()
            .clear();
    }
    let manager = crate::hosts::fake::get(base);
    manager.respond("Swarm.ControlAvailable", "true\n");
    manager.respond("docker node ls", &(list.join("\n") + "\n"));
    for i in SWARM {
        crate::hosts::fake::get(base + i)
            .respond("Swarm.NodeID", &format!("{}\n", rejoined(base + i)));
    }
    let error = apply_preview(&s, &id, &preview).await.unwrap_err();
    assert_eq!(error.0, StatusCode::CONFLICT);
    assert_eq!(key(error), "deployment.error.040");
    assert_eq!(revision_count(&s, &id).await, 0);
    // A fresh preview pins the new node and applies.
    let preview = preview_of(&s, &id, None).await.unwrap();
    assert!(
        preview["desired"]
            .as_str()
            .unwrap()
            .contains(&format!("node.id==node-{}-new", base + 2))
    );
    let rev = apply_preview(&s, &id, &preview).await.unwrap();
    assert_eq!(target_rows(&s, &rev).await.len(), 1);
}

#[tokio::test]
async fn stack_apply_checks_version_before_probing() {
    let base = 960;
    let (_dir, s) = stack_fixture(base).await;
    join_swarm(base);
    let id = create_app(&s, stack_body(base)).await.unwrap();
    let preview = preview_of(&s, &id, None).await.unwrap();
    for i in SWARM {
        crate::hosts::fake::get(base + i)
            .commands
            .lock()
            .unwrap()
            .clear();
    }
    let mut body = run_body(&preview, "");
    body.version += 1;
    let error = apply(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id.clone())),
        Json(body),
    )
    .await
    .unwrap_err();
    assert_eq!(key(error), "deployment.error.030");
    for i in SWARM {
        assert_eq!(commands(base + i, "docker node ls"), 0);
        assert_eq!(commands(base + i, "Swarm.NodeID"), 0);
    }
}

/// A stack deployment of [`stack_body`] whose stored spec selects `nats` hosts tagged `mq`.
async fn stack_with_spec(s: &AppState, base: i64) -> String {
    let id = create_app(s, stack_body(base)).await.unwrap();
    let spec = json!({
        "apiVersion": "ferry/deploy-v1",
        "template": {"id": "9101", "version": 1},
        "driver": "stack",
        "manager": base.to_string(),
        "roles": {"nats": {"instances": 3, "hosts": {"tags": ["mq"]}}}
    });
    apps::Entity::update_many()
        .col_expr(apps::Column::DeploySpec, Expr::value(spec.to_string()))
        .filter(apps::Column::Id.eq(id.parse::<i64>().unwrap()))
        .exec(&s.db)
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn stack_apply_syncs_tag_labels_before_deploy() {
    let base = 970;
    let (_dir, s) = stack_fixture(base).await;
    join_swarm(base);
    // The first two workers carry `mq`; the third lost it after placement.
    tag_host(&s, "mq", base + 1).await;
    tag_host(&s, "mq", base + 2).await;
    let label = crate::blueprint::swarm::tag_label("mq");
    let (n1, n2, n3) = (base + 1, base + 2, base + 3);
    crate::hosts::fake::get(base).respond(
        "docker node inspect",
        &format!(
            "node-{n1} null\nnode-{n2} {{\"{label}\":\"true\"}}\nnode-{n3} {{\"{label}\":\"true\",\"zone\":\"x\"}}\n"
        ),
    );
    let id = stack_with_spec(&s, base).await;
    // Preview stays read-only.
    let preview = preview_of(&s, &id, None).await.unwrap();
    assert_eq!(commands(base, "docker node inspect"), 0);
    assert_eq!(commands(base, "docker node update"), 0);
    let rev = apply_preview(&s, &id, &preview).await.unwrap();
    let manager = crate::hosts::fake::get(base);
    let updates: Vec<String> = manager
        .commands
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c.starts_with("docker node "))
        .filter(|c| !c.starts_with("docker node ls"))
        .cloned()
        .collect();
    assert_eq!(
        updates,
        [
            format!(
                "docker node inspect --format '{{{{.ID}}}} {{{{json .Spec.Labels}}}}' 'node-{n1}' 'node-{n2}' 'node-{n3}'"
            ),
            format!("docker node update --label-add '{label}=true' 'node-{n1}'"),
            format!("docker node update --label-rm '{label}' 'node-{n3}'"),
        ]
    );
    // Workers are never asked to change labels; the manager deploys afterwards.
    for host in [n1, n2, n3] {
        assert_eq!(commands(host, "docker node update"), 0);
    }
    let (ok, output) = execute(&s, &revision(&s, &rev).await).await.unwrap();
    assert!(ok, "{output}");
    assert_eq!(commands(base, "docker stack deploy"), 1);
}

#[tokio::test]
async fn stack_apply_stops_when_label_sync_fails() {
    let base = 980;
    let (_dir, s) = stack_fixture(base).await;
    join_swarm(base);
    tag_host(&s, "mq", base + 1).await;
    let manager = crate::hosts::fake::get(base);
    manager.fail_on(
        "docker node update",
        "Error response from daemon: rpc error",
    );
    let id = stack_with_spec(&s, base).await;
    let preview = preview_of(&s, &id, None).await.unwrap();
    let error = apply_preview(&s, &id, &preview).await.unwrap_err();
    let message = error.2.unwrap();
    assert_eq!(message.key, "blueprint.swarmLabelSyncFailed");
    assert_eq!(message.params["host"], format!("Host {base}"));
    assert_eq!(message.params["node"], format!("node-{}", base + 1));
    assert_eq!(revision_count(&s, &id).await, 0);
    assert_eq!(commands(base, "docker stack deploy"), 0);
    // A deployment without a spec has no tags to sync.
    let plain = create_app(&s, stack_body(base)).await.unwrap();
    let preview = preview_of(&s, &plain, None).await.unwrap();
    apply_preview(&s, &plain, &preview).await.unwrap();
    assert_eq!(commands(base, "docker node inspect"), 1);
}

// ---- Deployment specs (`ferry/deploy-v1`) ----

const SPEC_TEMPLATE: i64 = 9101;
const SPEC_PARAMETERS: &str = r#"[{"name":"tag","type":"string","default":"2"}]"#;

/// App template `SPEC_TEMPLATE` in `workspace`, holding the NATS blueprint with a `tag` value.
async fn app_template(s: &AppState, workspace: i64, version: i64) {
    use crate::entities::app_templates as t;
    t::Entity::delete_by_id(SPEC_TEMPLATE)
        .exec(&s.db)
        .await
        .unwrap();
    t::Entity::insert(t::ActiveModel {
        id: Set(SPEC_TEMPLATE),
        workspace_id: Set(workspace),
        name: Set("nats".into()),
        description: Set(String::new()),
        content: Set(blueprint("nats:{{ .Values.tag }}")),
        parameters: Set(SPEC_PARAMETERS.into()),
        version: Set(version),
        created_at: Set(1),
        updated_at: Set(1),
        ..Default::default()
    })
    .exec(&s.db)
    .await
    .unwrap();
}

/// Attaches tag `name` (workspace 1, created on first use) to `host`.
async fn tag_host(s: &AppState, name: &str, host: i64) {
    use crate::entities::{host_tags, tags};
    let found = tags::Entity::find()
        .filter(tags::Column::WorkspaceId.eq(1))
        .filter(tags::Column::Name.eq(name))
        .one(&s.db)
        .await
        .unwrap();
    let id = match found {
        Some(t) => t.id,
        None => {
            let id = i64::from(s.next_id().unwrap());
            tags::Entity::insert(tags::ActiveModel {
                id: Set(id),
                workspace_id: Set(1),
                name: Set(name.into()),
                color: Set(String::new()),
                created_at: Set(0),
                updated_at: Set(0),
                version: Set(1),
                ..Default::default()
            })
            .exec(&s.db)
            .await
            .unwrap();
            id
        }
    };
    host_tags::Entity::insert(host_tags::ActiveModel {
        host_id: Set(host),
        tag_id: Set(id),
    })
    .exec(&s.db)
    .await
    .unwrap();
}

/// Hosts tagged `mq` plus the spec's app template (version 1).
async fn spec_fixture(hosts: &[i64]) -> (tempfile::TempDir, AppState) {
    let (dir, s) = fixture(hosts).await;
    for h in hosts {
        tag_host(&s, "mq", *h).await;
    }
    app_template(&s, 1, 1).await;
    (dir, s)
}

fn spec_doc(version: i64, tag: &str, instances: u32) -> Value {
    json!({
        "apiVersion": "ferry/deploy-v1",
        "template": {"id": SPEC_TEMPLATE.to_string(), "version": version},
        "driver": "docker",
        "values": {"tag": tag},
        "roles": {"nats": {"instances": instances, "hosts": {"tags": ["mq"]}}}
    })
}

fn spec_body(spec: &Value) -> Value {
    json!({"name": "mq", "workdir": "/srv", "deploy_spec": spec.to_string()})
}

async fn place_spec(s: &AppState, scope: Scope, spec: &Value) -> Result<Value> {
    let body = serde_json::from_value(json!({"spec": spec.to_string()})).unwrap();
    let Json(out) = placement(State(s.clone()), scope, Json(body)).await?;
    Ok(out)
}

fn bound_hosts(binding: &str) -> Vec<String> {
    let b: Value = serde_json::from_str(binding).unwrap();
    b["roles"]["nats"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["hostId"].as_str().unwrap().to_owned())
        .collect()
}

async fn deployment_count(s: &AppState) -> u64 {
    apps::Entity::find().count(&s.db).await.unwrap()
}

#[tokio::test]
async fn placement_endpoint_renders_and_places() {
    let (_dir, s) = spec_fixture(&[1511, 1512, 1513]).await;
    tag_host(&s, "zone:a", 1511).await;
    tag_host(&s, "zone:a", 1512).await;
    tag_host(&s, "zone:b", 1513).await;
    let mut spec = spec_doc(1, "2.10", 2);
    spec["roles"]["nats"]["hosts"]["spread"] = json!({"prefix": "zone:"});
    let before = deployment_count(&s).await;
    let out = place_spec(&s, admin(1), &spec).await.unwrap();
    let manifest = out["manifest"].as_str().unwrap();
    assert!(manifest.contains("nats:2.10"), "{manifest}");
    assert!(crate::blueprint::model::is_blueprint(manifest));
    // Spread puts the second instance in the other zone.
    assert_eq!(bound_hosts(&out["binding"].to_string()), ["1511", "1513"]);
    assert_eq!(out["binding"]["driver"], "docker");
    assert_eq!(out["warnings"], json!([]));
    assert_eq!(deployment_count(&s).await, before);

    // Only zone a matches: both instances share it and the plan carries a warning.
    let mut spec = spec_doc(1, "2.10", 2);
    spec["roles"]["nats"]["hosts"]["tags"] = json!(["zone:a"]);
    spec["roles"]["nats"]["hosts"]["spread"] = json!({"prefix": "zone:"});
    let out = place_spec(&s, admin(1), &spec).await.unwrap();
    assert_eq!(out["warnings"][0]["key"], "blueprint.placementSpreadUnmet");
    assert!(!out["warnings"][0]["message"].as_str().unwrap().is_empty());

    for permission in ["deployments.create", "app-templates.read", "hosts.read"] {
        let mut scope = admin(1);
        scope.permissions.remove(permission);
        let error = place_spec(&s, scope, &spec_doc(1, "2", 2))
            .await
            .unwrap_err();
        assert_eq!(error.0, StatusCode::FORBIDDEN, "{permission}");
    }
    // Not enough matching hosts: explained, nothing placed.
    let error = place_spec(&s, admin(1), &spec_doc(1, "2", 4))
        .await
        .unwrap_err();
    assert_eq!(key(error), "blueprint.placementInsufficientHosts");
}

#[tokio::test]
async fn create_from_spec_stores_spec_manifest_and_binding() {
    let (_dir, s) = spec_fixture(&[1521, 1522, 1523]).await;
    let mut b = spec_body(&spec_doc(1, "2.10", 2));
    // Client-sent derived fields are ignored.
    b["manifest"] = json!("not a blueprint");
    b["mode"] = json!("native");
    b["target_kind"] = json!("host");
    b["target_ref"] = json!("1521");
    b["blueprint_binding"] = binding(&[1523]);
    let id = create_app(&s, b).await.unwrap();
    let r = row(&s, &id).await;
    assert_eq!(
        (r.target_kind.as_str(), r.target_ref, r.mode.as_str()),
        ("blueprint", 0, "docker")
    );
    assert!(r.manifest.contains("nats:2.10"));
    assert_eq!(
        bound_hosts(r.blueprint_binding.as_deref().unwrap()),
        ["1521", "1522"]
    );
    let stored = r.deploy_spec.clone().unwrap();
    let parsed = crate::blueprint::spec::parse(&stored).unwrap();
    assert_eq!(parsed.values["tag"], "2.10");
    assert_eq!(app_json(&r)["deploy_spec"], stored);
    // The ordinary blueprint machinery takes over: one target per bound host.
    let preview = preview_of(&s, &id, None).await.unwrap();
    let desired: Value = serde_json::from_str(preview["desired"].as_str().unwrap()).unwrap();
    assert_eq!(desired["targets"].as_array().unwrap().len(), 2);

    // Without a spec, only edits that keep manifest and binding are accepted.
    let stored_binding: Value =
        serde_json::from_str(r.blueprint_binding.as_deref().unwrap()).unwrap();
    let unchanged = json!({"name": "renamed", "manifest": r.manifest, "mode": "docker",
        "target_kind": "blueprint", "target_ref": "0", "workdir": "/srv",
        "blueprint_binding": stored_binding});
    let mut edited = unchanged.clone();
    edited["manifest"] = json!(blueprint("nats:9"));
    let error = update_body(&s, &id, edited).await.unwrap_err();
    assert_eq!(error.0, StatusCode::CONFLICT);
    assert_eq!(key(error), "deployments.specManaged");
    let mut moved = unchanged.clone();
    moved["blueprint_binding"] = binding(&[1521, 1523]);
    let error = update_body(&s, &id, moved).await.unwrap_err();
    assert_eq!(key(error), "deployments.specManaged");
    update_body(&s, &id, unchanged).await.unwrap();
    let r = row(&s, &id).await;
    assert_eq!(r.name, "renamed");
    assert_eq!(r.deploy_spec.as_deref(), Some(stored.as_str()));

    // Legacy blueprint deployments (no spec) keep working as before.
    let legacy = create_app(&s, body(&[1521, 1522])).await.unwrap();
    assert_eq!(row(&s, &legacy).await.deploy_spec, None);
}

#[tokio::test]
async fn manual_placement_must_satisfy_selector() {
    let (_dir, s) = spec_fixture(&[1531, 1532, 1533, 1534, 1535, 1536]).await;
    // 1532 offline, 1533 untagged, 1534 not Linux, 1535 in another workspace.
    hosts::Entity::update_many()
        .col_expr(hosts::Column::State, Expr::value("offline"))
        .filter(hosts::Column::Id.eq(1532))
        .exec(&s.db)
        .await
        .unwrap();
    crate::entities::host_tags::Entity::delete_many()
        .filter(crate::entities::host_tags::Column::HostId.eq(1533))
        .exec(&s.db)
        .await
        .unwrap();
    hosts::Entity::update_many()
        .col_expr(hosts::Column::Os, Expr::value("windows"))
        .filter(hosts::Column::Id.eq(1534))
        .exec(&s.db)
        .await
        .unwrap();
    hosts::Entity::update_many()
        .col_expr(hosts::Column::WorkspaceId, Expr::value(2))
        .filter(hosts::Column::Id.eq(1535))
        .exec(&s.db)
        .await
        .unwrap();
    let manual = |bad: i64| {
        let mut spec = spec_doc(1, "2", 2);
        spec["placement"] = json!({"nats": [
            {"hostId": "1531", "address": "10.0.0.1"},
            {"hostId": bad.to_string(), "address": "10.0.0.2"}]});
        spec
    };
    let before = deployment_count(&s).await;
    for (bad, host) in [
        (1532, "Host 1532"),
        (1533, "Host 1533"),
        (1534, "Host 1534"),
        (1535, "1535"),
        (1599, "1599"),
    ] {
        for error in [
            place_spec(&s, admin(1), &manual(bad)).await.unwrap_err(),
            create_app(&s, spec_body(&manual(bad))).await.unwrap_err(),
        ] {
            let message = error.2.expect("localized");
            assert_eq!(message.key, "blueprint.placementManualInvalid", "{bad}");
            assert_eq!(message.params["role"], "nats");
            assert_eq!(message.params["instance"], "nats-1");
            assert_eq!(message.params["host"], host);
        }
    }
    assert_eq!(deployment_count(&s).await, before);
    // Entries must match the blueprint's roles and the spec's instance count.
    let mut ghost = manual(1536);
    ghost["placement"]["ghost"] = json!([{"hostId": "1531", "address": "10.0.0.1"}]);
    let mut extra = manual(1536);
    extra["placement"]["nats"]
        .as_array_mut()
        .unwrap()
        .push(json!({"hostId": "1533", "address": "10.0.0.3"}));
    for (spec, expected) in [
        (ghost, "blueprint.bindingRoleUnknown"),
        (extra, "blueprint.placementManualCount"),
    ] {
        for error in [
            place_spec(&s, admin(1), &spec).await.unwrap_err(),
            create_app(&s, spec_body(&spec)).await.unwrap_err(),
        ] {
            let message = error.2.expect("localized");
            assert_eq!(message.key, expected);
            if expected == "blueprint.bindingRoleUnknown" {
                assert_eq!(message.params["role"], "ghost");
            } else {
                assert_eq!(message.params["role"], "nats");
                assert_eq!(message.params["instances"], "2");
                assert_eq!(message.params["count"], "3");
            }
        }
    }
    assert_eq!(deployment_count(&s).await, before);
    // A valid manual placement keeps its hosts, never rescheduled; addresses come from them.
    hosts::Entity::update_many()
        .col_expr(hosts::Column::Address, Expr::value("198.51.100.36"))
        .filter(hosts::Column::Id.eq(1536))
        .exec(&s.db)
        .await
        .unwrap();
    let expected = json!([{"hostId": "1531", "address": "192.0.2.1"},
                          {"hostId": "1536", "address": "198.51.100.36"}]);
    let planned = place_spec(&s, admin(1), &manual(1536)).await.unwrap();
    assert_eq!(planned["binding"]["roles"]["nats"], expected);
    let id = create_app(&s, spec_body(&manual(1536))).await.unwrap();
    let r = row(&s, &id).await;
    let stored: Value = serde_json::from_str(r.blueprint_binding.as_deref().unwrap()).unwrap();
    assert_eq!(stored["roles"]["nats"], expected);
    let spec = crate::blueprint::spec::parse(r.deploy_spec.as_deref().unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(&spec.placement.unwrap()["nats"]).unwrap(),
        expected
    );
}

#[tokio::test]
async fn template_version_mismatch_conflicts() {
    let (_dir, s) = spec_fixture(&[1541, 1542]).await;
    let stale = spec_doc(2, "2", 2);
    let error = place_spec(&s, admin(1), &stale).await.unwrap_err();
    assert_eq!(error.0, StatusCode::CONFLICT);
    assert_eq!(key(error), "templates.changed");
    let error = create_app(&s, spec_body(&stale)).await.unwrap_err();
    assert_eq!(error.0, StatusCode::CONFLICT);
    assert_eq!(key(error), "templates.changed");
    // Another workspace's template is never read.
    app_template(&s, 2, 1).await;
    let error = create_app(&s, spec_body(&spec_doc(1, "2", 2)))
        .await
        .unwrap_err();
    assert_eq!(key(error), "templates.changed");
}

async fn update_spec(s: &AppState, id: &str, spec: &Value) -> Result<()> {
    update_body(s, id, spec_body(spec)).await
}

#[tokio::test]
async fn values_change_after_deploy_is_upgrade() {
    let (_dir, s) = spec_fixture(&[1551, 1552, 1553]).await;
    let id = create_app(&s, spec_body(&spec_doc(1, "2.10", 2)))
        .await
        .unwrap();
    assert!(run_first(&s, &id, &[]).await);
    let binding_before = row(&s, &id).await.blueprint_binding;
    update_spec(&s, &id, &spec_doc(1, "2.11", 2)).await.unwrap();
    let r = row(&s, &id).await;
    assert!(r.manifest.contains("nats:2.11"));
    assert_eq!(r.blueprint_binding, binding_before);
    // A template version bump with new values re-renders as well.
    app_template(&s, 1, 2).await;
    let error = update_spec(&s, &id, &spec_doc(1, "2.12", 2))
        .await
        .unwrap_err();
    assert_eq!(key(error), "templates.changed");
    update_spec(&s, &id, &spec_doc(2, "2.12", 2)).await.unwrap();
    let r = row(&s, &id).await;
    assert!(r.manifest.contains("nats:2.12"));
    assert_eq!(r.blueprint_binding, binding_before);
    let stored = crate::blueprint::spec::parse(r.deploy_spec.as_deref().unwrap()).unwrap();
    assert_eq!(stored.template.version, 2);
    // The upgrade previews like any manifest change.
    let preview = preview_of(&s, &id, None).await.unwrap();
    let desired: Value = serde_json::from_str(preview["desired"].as_str().unwrap()).unwrap();
    assert_eq!(desired["targets"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn topology_change_after_deploy_rejected() {
    let (_dir, s) = spec_fixture(&[1561, 1562, 1563, 1564]).await;
    let id = create_app(&s, spec_body(&spec_doc(1, "2", 2)))
        .await
        .unwrap();
    // Before anything is deployed the spec may resize; it is scheduled afresh, exactly as
    // the placement endpoint previews it.
    update_spec(&s, &id, &spec_doc(1, "2", 3)).await.unwrap();
    let stored = row(&s, &id).await.blueprint_binding.unwrap();
    let planned = place_spec(&s, admin(1), &spec_doc(1, "2", 3))
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&stored).unwrap(),
        planned["binding"]
    );
    assert_eq!(bound_hosts(&stored), ["1561", "1562", "1563"]);
    assert!(run_first(&s, &id, &[]).await);
    let binding_before = row(&s, &id).await.blueprint_binding;
    let mut names = spec_doc(1, "2", 3);
    names["roles"]["nats"]["hosts"]["names"] = json!(["Host 1561"]);
    let mut manual = spec_doc(1, "2", 3);
    manual["placement"] = json!({"nats": [{"hostId": "1561", "address": "10.0.0.1"},
        {"hostId": "1562", "address": "10.0.0.2"}, {"hostId": "1564", "address": "10.0.0.4"}]});
    let mut driver = spec_doc(1, "2", 3);
    driver["driver"] = json!("compose");
    for changed in [spec_doc(1, "2", 2), names, manual, driver] {
        let error = update_spec(&s, &id, &changed).await.unwrap_err();
        assert_eq!(
            key(error),
            "blueprint.topologyChangeRequiresPlan",
            "{changed}"
        );
    }
    assert_eq!(row(&s, &id).await.blueprint_binding, binding_before);
    // A deployed legacy blueprint deployment cannot adopt a spec in place either.
    let legacy = create_app(&s, body(&[1561, 1562])).await.unwrap();
    assert!(run_first(&s, &legacy, &[]).await);
    let error = update_spec(&s, &legacy, &spec_doc(1, "2", 2))
        .await
        .unwrap_err();
    assert_eq!(key(error), "blueprint.topologyChangeRequiresPlan");
}

#[tokio::test]
async fn frozen_placement_ignores_tag_changes() {
    let (_dir, s) = spec_fixture(&[1571, 1572, 1573]).await;
    let id = create_app(&s, spec_body(&spec_doc(1, "2", 2)))
        .await
        .unwrap();
    assert!(run_first(&s, &id, &[]).await);
    let binding_before = row(&s, &id).await.blueprint_binding;
    assert_eq!(
        bound_hosts(binding_before.as_deref().unwrap()),
        ["1571", "1572"]
    );
    // The bound hosts lose the tag and the tag is renamed: nothing matches `mq` any more.
    use crate::entities::{host_tags, tags};
    host_tags::Entity::delete_many()
        .filter(host_tags::Column::HostId.is_in([1571, 1572]))
        .exec(&s.db)
        .await
        .unwrap();
    tags::Entity::update_many()
        .col_expr(tags::Column::Name, Expr::value("queue"))
        .filter(tags::Column::Name.eq("mq"))
        .exec(&s.db)
        .await
        .unwrap();
    let error = place_spec(&s, admin(1), &spec_doc(1, "2", 2))
        .await
        .unwrap_err();
    assert_eq!(key(error), "blueprint.placementSelectorEmpty");
    // The deployed placement is reused, not rescheduled.
    update_spec(&s, &id, &spec_doc(1, "2.1", 2)).await.unwrap();
    assert_eq!(row(&s, &id).await.blueprint_binding, binding_before);
    let preview = preview_of(&s, &id, None).await.unwrap();
    let desired: Value = serde_json::from_str(preview["desired"].as_str().unwrap()).unwrap();
    let hosts: Vec<&str> = desired["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["host_id"].as_str().unwrap())
        .collect();
    assert_eq!(hosts, ["1571", "1572"]);
}
