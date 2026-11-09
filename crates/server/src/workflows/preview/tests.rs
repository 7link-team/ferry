//! host.preview: contract, review routing, service lifetime and failures.
//! The terminal hooks (`finish`, immediate cancel) and the wake-up of a parked
//! review are tested next to them in `run/tests.rs`.
use super::*;
use crate::agents::tests::{admin, setup};
use crate::error::ApiError;
use sea_orm::{ActiveValue::Set, IntoActiveModel};
use std::sync::atomic::AtomicBool;

fn edge(from: &str, port: &str, to: &str) -> Value {
    json!({"from":{"node":from,"port":port,"index":0},"to":{"node":to,"port":"main","index":0}})
}

fn graph(params: Value, port: &str) -> String {
    let manual =
        super::super::graph::manifest(&json!({"type":"core.trigger.manual","type_version":1}))
            .unwrap();
    json!({"engine":"ferry-graph","schema":1,"nodes":[
        {"id":"start","name":"Start","type":"core.trigger.manual","type_version":1,
         "params":manual["example_params"],"position":[0,0]},
        {"id":"preview","name":"Preview","type":"host.preview","type_version":1,
         "params":params,"position":[1,0]},
        {"id":"next","name":"Next","type":"core.no_op","type_version":1,"params":{},"position":[2,0]}],
     "connections":[edge("start","main","preview"), edge("preview",port,"next")]})
    .to_string()
}

fn params(extra: Value) -> Value {
    let mut params = json!({"host_id":"11","port":5173});
    for (key, value) in extra.as_object().unwrap() {
        params[key] = value.clone();
    }
    params
}

fn key(error: ApiError) -> String {
    error.2.unwrap().key
}

#[test]
fn contract_accepts_both_review_modes_and_rejects_invalid_parameters() {
    let validate = |params: Value, port: &str| super::super::graph::validate(&graph(params, port));
    validate(params(json!({})), "main").unwrap();
    validate(
        params(
            json!({"command":"npm run dev","input_env":{"BRANCH":"={{ $json.branch }}"},
            "name":"Web","strip_prefix":false,"ready_timeout_seconds":600,
            "review":"none","review_timeout_minutes":1}),
        ),
        "main",
    )
    .unwrap();
    // The host may come from an upstream node, e.g. host.command's execution host.
    validate(
        params(json!({"host_id":"={{ $json.execution_host.id }}"})),
        "main",
    )
    .unwrap();
    validate(params(json!({"review":"human"})), "approved").unwrap();
    validate(params(json!({"review":"human"})), "rejected").unwrap();

    // Ports follow the review mode.
    for (review, port) in [
        ("human", "main"),
        ("none", "approved"),
        ("none", "rejected"),
    ] {
        assert_eq!(
            key(validate(params(json!({ "review": review })), port).unwrap_err()),
            "workflow.graphPort",
            "{review} {port}"
        );
    }
    assert_eq!(
        key(validate(params(json!({"command":"echo {{ $json.x }}"})), "main").unwrap_err()),
        "workflow.graphCommand"
    );
    for invalid in [
        json!({"port":0}),
        json!({"port":65536}),
        json!({"host_id":"0"}),
        json!({"host_id":"abc"}),
        json!({"command":"x".repeat(4001)}),
        json!({"name":"x".repeat(129)}),
        json!({"ready_timeout_seconds":0}),
        json!({"ready_timeout_seconds":601}),
        json!({"review":"ai"}),
        json!({"review_timeout_minutes":0}),
        json!({"review_timeout_minutes":1441}),
        json!({"input_env":{"1BAD":"x"}}),
        json!({"unknown":true}),
        // Review mode is static: the ports depend on it.
        json!({"review":"={{ $json.review }}"}),
    ] {
        assert_eq!(
            key(validate(params(invalid.clone()), "main").unwrap_err()),
            "workflow.graphSchema",
            "{invalid}"
        );
    }
    let mut missing = params(json!({}));
    missing.as_object_mut().unwrap().remove("port");
    assert!(validate(missing, "main").is_err());
}

#[test]
fn defaults_apply_and_skipped_nodes_skip_the_ports_of_their_mode() {
    let request = Request::parse(&json!({"host_id":"11","port":80,"command":"  "})).unwrap();
    assert!(request.command.is_none(), "a blank command starts nothing");
    assert!(request.strip_prefix);
    assert_eq!(request.ready_seconds, 60);
    assert!(!request.human);
    assert_eq!(request.review_minutes, 60);
    let names = |review: &str| {
        skipped(&json!({"params":{"review":review}}))
            .ports
            .into_iter()
            .map(|port| (port.name, port.skipped))
            .collect::<Vec<_>>()
    };
    assert_eq!(names("none"), vec![("main".into(), true)]);
    assert_eq!(
        names("human"),
        vec![("approved".into(), true), ("rejected".into(), true)]
    );
}

async fn add_runner(s: &AppState, id: i64) {
    hosts::Entity::insert(hosts::ActiveModel {
        id: Set(id),
        workspace_id: Set(1),
        name: Set(format!("Runner {id}")),
        address: Set(String::new()),
        port: Set(0),
        username: Set(String::new()),
        connection: Set("runner".into()),
        state: Set("offline".into()),
        group_id: Set(0),
        description: Set(String::new()),
        host_key: Set(String::new()),
        runner_key_hash: Set(String::new()),
        enroll_token: Set(String::new()),
        enroll_token_hash: Set(String::new()),
        seen_at: Set(0),
        created_at: Set(0),
        updated_at: Set(0),
        version: Set(1),
        os: Set(String::new()),
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

async fn opened(s: &AppState, execution: i64, review: Option<i64>) -> holes::Model {
    open_owned(
        s,
        &admin(1),
        OwnedRequest {
            host_id: Id::from(11),
            port: 5173,
            name: String::new(),
            strip_prefix: true,
            source_kind: "workflow",
            source_id: execution,
            source_ref: "preview".into(),
            review_minutes: review,
        },
    )
    .await
    .unwrap()
}

async fn set(s: &AppState, hole: i64, column: holes::Column, value: impl Into<sea_orm::Value>) {
    holes::Entity::update_many()
        .col_expr(column, Expr::value(value))
        .filter(holes::Column::Id.eq(hole))
        .exec(&s.db)
        .await
        .unwrap();
}

fn done(step: Step) -> Output {
    match step {
        Step::Done(output) => output,
        Step::Wait(_) => panic!("expected the node to complete"),
    }
}

/// (port, skipped, review_state of the item on the taken port)
fn routes(output: &Output) -> Vec<(String, bool, Option<String>)> {
    output
        .ports
        .iter()
        .map(|port| {
            (
                port.name.clone(),
                port.skipped,
                port.items
                    .first()
                    .map(|item| item.json["review_state"].as_str().unwrap().to_owned()),
            )
        })
        .collect()
}

#[tokio::test]
async fn review_routes_approved_and_everything_else_to_rejected() {
    let (_dir, s) = setup().await;
    add_runner(&s, 11).await;

    // review=none completes on main with the link and the host.
    let plain = opened(&s, 60, None).await;
    let output = done(settle(&s, plain.id, false).await.unwrap());
    assert_eq!(
        routes(&output),
        vec![("main".into(), false, Some("none".into()))]
    );
    let item = &output.ports[0].items[0].json;
    assert_eq!(item["url"], json!(crate::holes::link(&s, &plain).unwrap()));
    assert!(item["url"].as_str().unwrap().starts_with("/holes/"));
    assert_eq!(item["hole_id"], json!(plain.id.to_string()));
    assert_eq!(
        item["execution_host"],
        json!({"id":"11","name":"Runner 11"})
    );

    // review=human waits until the review deadline while the review is open.
    let human = opened(&s, 61, Some(30)).await;
    match settle(&s, human.id, true).await.unwrap() {
        Step::Wait(deadline) => assert_eq!(deadline, human.expires_at),
        Step::Done(_) => panic!("an open review must park the node"),
    }
    set(&s, human.id, holes::Column::ReviewState, "approved").await;
    assert_eq!(
        routes(&done(settle(&s, human.id, true).await.unwrap())),
        vec![
            ("approved".into(), false, Some("approved".into())),
            ("rejected".into(), true, None)
        ]
    );

    let rejected = opened(&s, 62, Some(30)).await;
    set(&s, rejected.id, holes::Column::ReviewState, "rejected").await;
    let timed_out = opened(&s, 63, Some(30)).await;
    set(&s, timed_out.id, holes::Column::ExpiresAt, 1_i64).await;
    let revoked = opened(&s, 64, Some(30)).await;
    set(&s, revoked.id, holes::Column::RevokedAt, 5_i64).await;
    for (hole, state) in [
        (rejected.id, "rejected"),
        (timed_out.id, "timed_out"),
        (revoked.id, "revoked"),
    ] {
        assert_eq!(
            routes(&done(settle(&s, hole, true).await.unwrap())),
            vec![
                ("approved".into(), true, None),
                ("rejected".into(), false, Some(state.into()))
            ],
            "{state}"
        );
    }
}

#[tokio::test]
async fn a_service_that_never_comes_up_fails_the_node_and_leaves_no_preview() {
    let (_dir, s) = setup().await;
    add_runner(&s, 11).await; // offline: the port never becomes reachable
    let request =
        Request::parse(&json!({"host_id":"11","port":5173,"ready_timeout_seconds":1})).unwrap();
    let error = start(&s, &admin(1), 70, "preview", &request)
        .await
        .err()
        .unwrap();
    assert_eq!(key(error), "holes.notReady");
    let row = latest(&s.db, 70, "preview").await.unwrap().unwrap();
    assert!(row.revoked_at > 0);

    // A command that cannot start (runner offline) also revokes the preview.
    let request =
        Request::parse(&json!({"host_id":"11","port":5173,"command":"npm run dev"})).unwrap();
    assert!(start(&s, &admin(1), 71, "preview", &request).await.is_err());
    assert!(
        latest(&s.db, 71, "preview")
            .await
            .unwrap()
            .unwrap()
            .revoked_at
            > 0
    );

    // Hosts that are not runner hosts of the workspace, or a missing right, are refused.
    let request = Request::parse(&json!({"host_id":"2","port":5173})).unwrap();
    assert_eq!(
        key(start(&s, &admin(1), 72, "preview", &request)
            .await
            .err()
            .unwrap()),
        "holes.invalid"
    );
    let mut denied = admin(1);
    denied.permissions.retain(|p| p != "holes.create");
    let request = Request::parse(&json!({"host_id":"11","port":5173})).unwrap();
    assert_eq!(
        start(&s, &denied, 73, "preview", &request)
            .await
            .err()
            .unwrap()
            .0,
        axum::http::StatusCode::FORBIDDEN
    );
    assert!(latest(&s.db, 73, "preview").await.unwrap().is_none());
}

/// A service that runs until it is cancelled, like a dev server.
#[derive(Default)]
struct Service {
    started: tokio::sync::Notify,
    cancelled: AtomicBool,
    commands: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl HostChannel for std::sync::Arc<Service> {
    async fn exec_stream(
        &self,
        command: &str,
        timeout: Duration,
        output: mpsc::Sender<(String, Vec<u8>)>,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<i32> {
        assert!(timeout > Duration::from_secs(24 * 3600), "no 120 s limit");
        self.commands.lock().unwrap().push(command.into());
        let _ = output.send(("stdout".into(), b"listening".to_vec())).await;
        self.started.notify_one();
        while !*cancel.borrow() {
            cancel.changed().await.unwrap();
        }
        self.cancelled.store(true, Ordering::Release);
        Ok(-130)
    }
    async fn exec(&self, command: &str, _: Duration) -> Result<crate::hosts::channel::Output> {
        self.commands.lock().unwrap().push(command.into());
        Ok(crate::hosts::channel::Output {
            code: 0,
            stdout: String::new(),
            stderr: String::new(),
        })
    }
    async fn monitor(&self) -> Result<ferry_protocol::HostMonitor> {
        panic!("unexpected monitor")
    }
    async fn open_pty(
        &self,
        _: crate::hosts::channel::PtySize,
    ) -> Result<Box<dyn crate::hosts::channel::Pty>> {
        panic!("unexpected pty")
    }
    async fn list_dir(&self, _: &str) -> Result<Vec<crate::hosts::channel::DirEntry>> {
        panic!("unexpected list")
    }
    async fn create_file(&self, _: &str) -> Result<()> {
        panic!("unexpected create file")
    }
    async fn create_dir(&self, _: &str) -> Result<()> {
        panic!("unexpected create directory")
    }
    async fn read_file(
        &self,
        _: &str,
    ) -> Result<(
        crate::hosts::channel::FileMeta,
        crate::hosts::channel::ByteStream,
    )> {
        panic!("unexpected read")
    }
    async fn write_file(
        &self,
        _: &str,
        _: u32,
        _: crate::hosts::channel::ByteStream,
    ) -> Result<()> {
        panic!("unexpected write")
    }
    async fn delete_file(&self, _: &str) -> Result<()> {
        panic!("unexpected delete")
    }
}

fn registered(execution: i64) -> usize {
    SERVICES.lock().unwrap().get(&execution).map_or(0, Vec::len)
}

async fn until(what: &str, check: impl Fn() -> bool) {
    for _ in 0..100 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn execution(s: &AppState, id: i64, state: &str) {
    let mut row = crate::migration::workflow_tests::execution(id);
    row.state = state.into();
    row.number = id;
    runs::Entity::insert(row.into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn services_outlive_the_node_until_the_execution_stops_them() {
    let (_dir, s) = setup().await;
    execution(&s, 81, "running").await;
    let service = std::sync::Arc::new(Service::default());
    spawn(
        s.clone(),
        81,
        None,
        Box::new(service.clone()),
        "/tmp/workflow-x".into(),
        "sh '/tmp/workflow-x/service.sh'".into(),
    );
    service.started.notified().await;
    assert_eq!(registered(81), 1);
    // Still running well after the node would have completed.
    tokio::time::sleep(WATCH_EVERY * 3).await;
    assert!(!service.cancelled.load(Ordering::Acquire));
    stop(81);
    until("the service to be cancelled and cleaned up", || {
        service
            .commands
            .lock()
            .unwrap()
            .contains(&"rm -rf -- '/tmp/workflow-x'".to_owned())
    })
    .await;
    assert!(service.cancelled.load(Ordering::Acquire));
    assert_eq!(registered(81), 0);

    // Backstop: an execution that ended elsewhere stops its service too.
    execution(&s, 82, "running").await;
    let other = std::sync::Arc::new(Service::default());
    spawn(
        s.clone(),
        82,
        None,
        Box::new(other.clone()),
        "/tmp/workflow-y".into(),
        "sh '/tmp/workflow-y/service.sh'".into(),
    );
    other.started.notified().await;
    runs::Entity::update_many()
        .col_expr(runs::Column::State, Expr::value("failed"))
        .filter(runs::Column::Id.eq(82))
        .exec(&s.db)
        .await
        .unwrap();
    until("the ended execution to stop its service", || {
        other.cancelled.load(Ordering::Acquire)
    })
    .await;
    until("the registry entry to go", || registered(82) == 0).await;
}

#[tokio::test]
async fn a_review_nudges_only_a_parked_execution_of_its_own() {
    let (_dir, s) = setup().await;
    add_runner(&s, 11).await;
    execution(&s, 90, "waiting").await;
    execution(&s, 91, "running").await;
    let far = Some(db::now() + 3_600_000);
    runs::Entity::update_many()
        .col_expr(runs::Column::NotBefore, Expr::value(far))
        .exec(&s.db)
        .await
        .unwrap();
    for id in [90, 91] {
        nudge(&s.db, &opened(&s, id, Some(30)).await).await.unwrap();
    }
    let not_before = |id| {
        let s = s.clone();
        async move {
            runs::Entity::find_by_id(id)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap()
                .not_before
        }
    };
    assert!(not_before(90).await.unwrap() <= db::now());
    assert_eq!(not_before(91).await, far);
}
