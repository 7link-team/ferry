use super::*;
use crate::{
    agents::tests::admin,
    entities::workspace_roles,
    error::{Json, Path},
    hosts::fake::{self, AgentStep, FakeHost},
    platform_tools::tests::{agent as create_agent, role, setup},
};
use axum::extract::State;
use ferry_protocol::agent::Usage;
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, sea_query::Expr,
};
use std::sync::atomic::Ordering;

fn schema() -> Value {
    json!({"type":"object","properties":{"summary":{"type":"string"},"count":{"type":"integer"}},
        "required":["summary"],"additionalProperties":false})
}
fn params(agent: i64) -> Value {
    json!({"agent_id":agent.to_string(),"prompt":"Summarize the workflows of this workspace.",
        "output_schema":schema(),"timeout_s":600,"max_tool_calls":64})
}
fn graph(params: Value) -> Value {
    json!({"engine":"ferry-graph","schema":1,"settings":{"timezone":"UTC"},"nodes":[
        {"id":"start","name":"Start","type":"core.trigger.manual","type_version":1,
            "params":{"input_schema":{"type":"object"}},"position":[0,0]},
        {"id":"agent","name":"Agent","type":"ai.agent.run","type_version":1,
            "params":params,"position":[300,0]}
    ],"connections":[{"from":{"node":"start","port":"main","index":0},
        "to":{"node":"agent","port":"main","index":0}}]})
}
fn key(error: &ApiError) -> &str {
    &error.2.as_ref().unwrap().key
}
fn text(id: &str, text: &str, replace: bool) -> AgentStep {
    AgentStep::Event(Event::Text {
        item_id: id.into(),
        text: text.into(),
        replace,
    })
}
fn finished() -> AgentStep {
    AgentStep::Event(Event::Finished {
        outcome: Outcome::Completed,
        usage: Some(Usage {
            input_tokens: 1,
            output_tokens: 1,
        }),
    })
}
fn call(id: &str, name: &str, arguments: Value) -> AgentStep {
    AgentStep::Event(Event::PlatformCall {
        request_id: id.into(),
        name: name.into(),
        arguments,
    })
}
fn answer(json: &str) -> Vec<AgentStep> {
    vec![text("final", json, true), finished()]
}

struct Fixture {
    _dir: tempfile::TempDir,
    s: AppState,
    run: workflow_executions::Model,
    agent: ai_agents::Model,
    role: Id,
    host: i64,
    fake: Arc<FakeHost>,
}

async fn create_workflow(s: &AppState, definition: Value) -> Result<Id> {
    let (_, Json(workflow)) = super::super::create(
        State(s.clone()),
        admin(1),
        Json(
            serde_json::from_value(json!({"name":"AI agent","definition":definition.to_string()}))
                .unwrap(),
        ),
    )
    .await?;
    Ok(serde_json::from_value(workflow["id"].clone()).unwrap())
}

impl Fixture {
    async fn new() -> Self {
        let (dir, s) = setup().await;
        let role = role(&s, 1, vec!["workflows.read"]).await;
        let agent = create_agent(&s, role).await;
        let host = i64::from(rand::random::<u32>()) + 1000;
        s.db.execute_unprepared(&format!("INSERT INTO hosts (id,workspace_id,name,address,port,username,connection,state,group_id,description,host_key,runner_key_hash,enroll_token,enroll_token_hash,jump_host_id,seen_at,created_at,updated_at,version,os,arch,agent_version,metrics,credential_id) VALUES ({host},1,'Build runner','127.0.0.1',22,'fixture','fake','online',0,'','','','','',0,0,0,0,1,'linux','x86_64','','',0)")).await.unwrap();
        ai_agents::Entity::update_many()
            .col_expr(ai_agents::Column::HostId, Expr::value(host))
            .col_expr(ai_agents::Column::Cwd, Expr::value(""))
            .filter(ai_agents::Column::Id.eq(agent.id))
            .exec(&s.db)
            .await
            .unwrap();
        let fake = fake::register(host);
        let id = create_workflow(&s, graph(params(agent.id))).await.unwrap();
        let (_, Json(created)) = super::super::run::create(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id)),
            Json(
                serde_json::from_value(
                    json!({"mode":"manual","revision":"draft","version":1,"entry_node_id":"start"}),
                )
                .unwrap(),
            ),
        )
        .await
        .unwrap();
        let execution: Id = serde_json::from_value(created["id"].clone()).unwrap();
        workflow_executions::Entity::update_many()
            .col_expr(workflow_executions::Column::State, Expr::value("running"))
            .filter(workflow_executions::Column::Id.eq(i64::from(execution)))
            .exec(&s.db)
            .await
            .unwrap();
        let run = workflow_executions::Entity::find_by_id(i64::from(execution))
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let agent = ai_agents::Entity::find_by_id(agent.id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        Self {
            _dir: dir,
            s,
            run,
            agent,
            role,
            host,
            fake,
        }
    }
    fn script(&self, steps: Vec<AgentStep>) {
        self.fake.agent_scripts.lock().unwrap().push_back(steps);
    }
    async fn execute_params(&self, params: Value, timeout: Duration) -> Result<Vec<Item>> {
        let input = Item {
            json: json!({"from":"trigger","since":"2026-11-01"})
                .as_object()
                .unwrap()
                .clone(),
            binary: Default::default(),
            paired: vec![Paired {
                node: "start".into(),
                iteration_hash: String::new(),
                index: 0,
            }],
        };
        let node = json!({"id":"agent","type":"ai.agent.run","params":params});
        execute_with(
            &self.s,
            &self.run,
            0,
            &node,
            &[input],
            &[params],
            Limits { timeout },
        )
        .await
    }
    async fn execute(&self) -> Result<Vec<Item>> {
        self.execute_params(params(self.agent.id), Duration::from_secs(30))
            .await
    }
    fn requests(&self) -> Vec<Request> {
        self.fake.agent_requests.lock().unwrap().clone()
    }
    fn results(&self) -> Vec<Value> {
        self.fake
            .agent_inputs
            .lock()
            .unwrap()
            .iter()
            .map(|input| match input {
                Input::PlatformResult { result, .. } => result.clone(),
                other => panic!("unexpected input {other:?}"),
            })
            .collect()
    }
    async fn grant_agent(&self, permissions: Vec<&str>) {
        workspace_roles::Entity::update_many()
            .col_expr(
                workspace_roles::Column::Permissions,
                Expr::value(json!(permissions).to_string()),
            )
            .filter(workspace_roles::Column::Id.eq(i64::from(self.role)))
            .exec(&self.s.db)
            .await
            .unwrap();
    }
    async fn grant_user(&self, permissions: Vec<&str>) {
        workspace_roles::Entity::update_many()
            .col_expr(
                workspace_roles::Column::Permissions,
                Expr::value(json!(permissions).to_string()),
            )
            .filter(workspace_roles::Column::Id.eq(1))
            .exec(&self.s.db)
            .await
            .unwrap();
    }
    async fn workflow_count(&self) -> usize {
        crate::entities::workflows::Entity::find()
            .filter(crate::entities::workflows::Column::WorkspaceId.eq(1))
            .all(&self.s.db)
            .await
            .unwrap()
            .len()
    }
}

#[tokio::test]
async fn a_valid_final_answer_becomes_one_item_from_a_read_only_runner_turn() {
    let fixture = Fixture::new().await;
    fixture.script(answer(r#"{"summary":"two workflows","count":2}"#));
    let items = fixture.execute().await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        Value::Object(items[0].json.clone()),
        json!({"summary":"two workflows","count":2})
    );
    // The output keeps the lineage of every input item.
    assert_eq!(items[0].paired.len(), 1);
    assert_eq!(items[0].paired[0].node, "start");
    let requests = fixture.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.permission, Permission::ReadOnly);
    assert!(request.platform_tools);
    assert_eq!(request.runtime, Runtime::Codex);
    assert!(request.native_session_id.is_none());
    // The managed workspace is keyed by workflow and Agent.
    assert_eq!(
        request.cwd,
        format!(
            "/fake/workspaces/{}-{}",
            fixture.run.workflow_id, fixture.agent.id
        )
    );
    assert_eq!(
        request.binding.as_ref().unwrap().canonical_directory,
        request.cwd
    );
    assert!(
        request
            .prompt
            .contains("Summarize the workflows of this workspace.")
    );
    assert!(request.prompt.contains(r#""since":"2026-11-01""#));
    assert!(request.prompt.contains(r#""additionalProperties":false"#));
    assert!(request.timeout_seconds <= 30);
    assert!(!fixture.fake.agent_cancelled.load(Ordering::SeqCst));
    // Nothing is held on the host afterwards.
    assert!(
        crate::entities::host_execution_slots::Entity::find()
            .filter(crate::entities::host_execution_slots::Column::HostId.eq(fixture.host))
            .one(&fixture.s.db)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn fenced_json_in_streamed_deltas_is_accepted_from_the_last_message() {
    let fixture = Fixture::new().await;
    fixture.script(vec![
        text("plan", "```json\n{\"summary\":\"draft\"}\n```", true),
        text("final", "Here is the result:\n```json\n{\"summ", false),
        text("final", "ary\":\"done\"}\n```\nThanks.", false),
        finished(),
    ]);
    let items = fixture.execute().await.unwrap();
    assert_eq!(
        Value::Object(items[0].json.clone()),
        json!({"summary":"done"})
    );
}

#[test]
fn json_is_taken_from_the_whole_text_or_the_last_json_fence() {
    assert_eq!(
        extract(" {\"a\":1} ").map(Value::Object),
        Some(json!({"a":1}))
    );
    assert_eq!(
        extract("x\n```json\n{\"a\":1}\n```\n```JSON\n{\"a\":2}\n```").map(Value::Object),
        Some(json!({"a":2}))
    );
    assert_eq!(
        extract("```\n{\"a\":3}\n```").map(Value::Object),
        Some(json!({"a":3}))
    );
    // Other languages, arrays and unterminated fences are not answers.
    assert!(extract("```js\n{\"a\":1}\n```").is_none());
    assert!(extract("[1,2]").is_none());
    assert!(extract("```json\n{\"a\":1}").is_none());
    assert!(extract("no json").is_none());
}

#[tokio::test]
async fn schema_invalid_output_gets_one_correction_in_the_same_session_then_fails() {
    let fixture = Fixture::new().await;
    fixture.script(vec![
        AgentStep::Event(Event::Session {
            native_session_id: "native-1".into(),
        }),
        text("final", r#"{"summary":3}"#, true),
        finished(),
    ]);
    fixture.script(answer(r#"{"summary":"fixed"}"#));
    let items = fixture.execute().await.unwrap();
    assert_eq!(
        Value::Object(items[0].json.clone()),
        json!({"summary":"fixed"})
    );
    let requests = fixture.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].native_session_id.as_deref(), Some("native-1"));
    assert_eq!(requests[1].permission, Permission::ReadOnly);
    assert!(requests[1].prompt.contains("rejected"));
    assert!(requests[1].prompt.contains("/summary"));

    // A second invalid answer fails with the schema reason and the original text.
    let fixture = Fixture::new().await;
    fixture.script(vec![
        AgentStep::Event(Event::Session {
            native_session_id: "native-2".into(),
        }),
        text("final", r#"{"summary":"x","extra":true}"#, true),
        finished(),
    ]);
    fixture.script(answer(r#"{"count":1}"#));
    let error = fixture.execute().await.unwrap_err();
    assert_eq!(error.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(key(&error), "workflow.agentInvalidOutput");
    let message = error.2.as_ref().unwrap();
    assert_eq!(message.params["text"], r#"{"count":1}"#);
    assert!(message.params["reason"].contains("summary"));
    assert_eq!(super::super::run::failure_reason(&error), "agent_output");
    assert_eq!(fixture.requests().len(), 2);
    assert!(crate::i18n::render(crate::i18n::Locale::ZhCn, message).contains("不符合输出 Schema"));

    // Without a native session there is nothing to continue: no retry.
    let fixture = Fixture::new().await;
    fixture.script(answer("I could not find any data."));
    let error = fixture.execute().await.unwrap_err();
    assert_eq!(key(&error), "workflow.agentNoJson");
    assert_eq!(
        error.2.as_ref().unwrap().params["text"],
        "I could not find any data."
    );
    assert_eq!(fixture.requests().len(), 1);
}

#[tokio::test]
async fn platform_reads_return_data_and_writes_are_refused_without_executing() {
    let fixture = Fixture::new().await;
    let before = fixture.workflow_count().await;
    fixture.script(vec![
        call("discover", "ferry_discover", json!({"search":"workflows"})),
        AgentStep::Input,
        call("read", "ferry_request", json!({"method":"GET","path":"/workflows?page_size=5"})),
        AgentStep::Input,
        call("write", "ferry_request",
            json!({"method":"POST","path":"/workflows","body":{"name":"Injected","definition":"{}"}})),
        AgentStep::Input,
        call("delete", "ferry_request", json!({"method":"DELETE","path":format!("/workflows/{}", Id::from(fixture.run.workflow_id))})),
        AgentStep::Input,
        call("unlisted", "ferry_request", json!({"method":"GET","path":"/workflows/1/nonexistent/route"})),
        AgentStep::Input,
        call("body", "ferry_request", json!({"method":"GET","path":"/workflows","body":{"a":1}})),
        AgentStep::Input,
        call("unknown", "shell", json!({"command":"rm -rf /"})),
        AgentStep::Input,
        text("final", r#"{"summary":"one workflow"}"#, true),
        finished(),
    ]);
    fixture.execute().await.unwrap();
    let results = fixture.results();
    assert_eq!(results.len(), 7);
    // Discovery lists only GET operations.
    let operations = results[0]["operations"].as_array().unwrap();
    assert!(!operations.is_empty());
    assert!(operations.iter().all(|row| row["method"] == "GET"));
    assert!(
        results[0]["instructions"]
            .as_str()
            .unwrap()
            .contains("read-only")
    );
    // A GET within (execution identity ∩ Agent role) returns the handler's data.
    assert_eq!(results[1]["status"], 200);
    assert_eq!(results[1]["data"]["items"][0]["name"], "AI agent");
    for result in &results[2..] {
        assert_eq!(result["status"], 403, "{result}");
        assert_eq!(result["ok"], false);
    }
    assert!(results[2]["error"].as_str().unwrap().contains("read-only"));
    assert_eq!(fixture.workflow_count().await, before);
    assert!(
        crate::entities::workflows::Entity::find_by_id(fixture.run.workflow_id)
            .one(&fixture.s.db)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn revoking_the_agent_role_mid_run_denies_the_next_call() {
    let fixture = Fixture::new().await;
    let (reached, resume) = (
        Arc::new(tokio::sync::Barrier::new(2)),
        Arc::new(tokio::sync::Barrier::new(2)),
    );
    fixture.script(vec![
        call(
            "first",
            "ferry_request",
            json!({"method":"GET","path":"/workflows"}),
        ),
        AgentStep::Input,
        AgentStep::Barrier(reached.clone()),
        AgentStep::Barrier(resume.clone()),
        call(
            "second",
            "ferry_request",
            json!({"method":"GET","path":"/workflows"}),
        ),
        AgentStep::Input,
        text("final", r#"{"summary":"partial"}"#, true),
        finished(),
    ]);
    let running = {
        let fixture = &fixture;
        async move { fixture.execute().await }
    };
    let control = async {
        reached.wait().await;
        fixture.grant_agent(vec![]).await;
        resume.wait().await;
    };
    let (result, ()) = tokio::join!(running, control);
    result.unwrap();
    let results = fixture.results();
    assert_eq!(results[0]["status"], 200);
    assert_eq!(results[1]["status"], 403);
    assert_eq!(results[1]["ok"], false);
}

#[tokio::test]
async fn revoking_the_execution_identity_mid_run_cancels_the_cli_and_fails() {
    let fixture = Fixture::new().await;
    let reached = Arc::new(tokio::sync::Barrier::new(2));
    fixture.script(vec![AgentStep::Barrier(reached.clone()), AgentStep::Hang]);
    let control = async {
        reached.wait().await;
        let mut permissions = admin(1).permissions;
        permissions.retain(|permission| permission != "workflows.run");
        fixture
            .grant_user(permissions.iter().map(String::as_str).collect())
            .await;
    };
    let (result, ()) = tokio::join!(fixture.execute(), control);
    let error = result.unwrap_err();
    assert_eq!(error.0, StatusCode::FORBIDDEN);
    assert_eq!(
        super::super::run::failure_reason(&error),
        "permission_revoked"
    );
    assert!(fixture.fake.agent_cancelled.load(Ordering::SeqCst));
}

#[tokio::test]
async fn cancelling_the_execution_or_reaching_the_timeout_cancels_the_cli() {
    let fixture = Fixture::new().await;
    let reached = Arc::new(tokio::sync::Barrier::new(2));
    fixture.script(vec![AgentStep::Barrier(reached.clone()), AgentStep::Hang]);
    let control = async {
        reached.wait().await;
        workflow_executions::Entity::update_many()
            .col_expr(
                workflow_executions::Column::State,
                Expr::value("cancelling"),
            )
            .filter(workflow_executions::Column::Id.eq(fixture.run.id))
            .exec(&fixture.s.db)
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(fixture.execute(), control);
    assert_eq!(result.unwrap_err().0, StatusCode::FORBIDDEN);
    assert!(fixture.fake.agent_cancelled.load(Ordering::SeqCst));

    let fixture = Fixture::new().await;
    fixture.script(vec![AgentStep::Hang]);
    let error = fixture
        .execute_params(params(fixture.agent.id), Duration::from_millis(300))
        .await
        .unwrap_err();
    assert_eq!(key(&error), "workflow.agentTimeout");
    assert_eq!(super::super::run::failure_reason(&error), "agent_timeout");
    assert!(fixture.fake.agent_cancelled.load(Ordering::SeqCst));
}

#[tokio::test]
async fn exceeding_the_tool_call_limit_cancels_and_fails() {
    let fixture = Fixture::new().await;
    let mut limited = params(fixture.agent.id);
    limited["max_tool_calls"] = json!(2);
    fixture.script(vec![
        call(
            "1",
            "ferry_request",
            json!({"method":"GET","path":"/workflows"}),
        ),
        AgentStep::Input,
        call("2", "ferry_discover", json!({})),
        AgentStep::Input,
        call(
            "3",
            "ferry_request",
            json!({"method":"GET","path":"/workflows"}),
        ),
        AgentStep::Hang,
    ]);
    let error = fixture
        .execute_params(limited, Duration::from_secs(30))
        .await
        .unwrap_err();
    assert_eq!(key(&error), "workflow.agentToolLimit");
    assert_eq!(error.2.as_ref().unwrap().params["max"], "2");
    assert_eq!(super::super::run::failure_reason(&error), "agent_limit");
    assert!(fixture.fake.agent_cancelled.load(Ordering::SeqCst));
    assert_eq!(fixture.results().len(), 2);
}

#[tokio::test]
async fn an_approval_request_or_an_unfinished_run_fails_the_node() {
    let fixture = Fixture::new().await;
    fixture.script(vec![
        AgentStep::Event(Event::Approval {
            request_id: "a".into(),
            tool: "shell".into(),
            input: json!({}),
        }),
        AgentStep::Hang,
    ]);
    let error = fixture.execute().await.unwrap_err();
    assert_eq!(key(&error), "workflow.agentApproval");
    assert!(fixture.fake.agent_cancelled.load(Ordering::SeqCst));

    let fixture = Fixture::new().await;
    fixture.script(vec![
        text("final", r#"{"summary":"late"}"#, true),
        AgentStep::Event(Event::Finished {
            outcome: Outcome::Failed,
            usage: None,
        }),
    ]);
    let error = fixture.execute().await.unwrap_err();
    assert_eq!(key(&error), "workflow.agentFailed");
    assert_eq!(super::super::run::failure_reason(&error), "agent_failed");
}

#[tokio::test]
async fn runner_problems_name_the_host_and_the_cli() {
    let fixture = Fixture::new().await;
    *fixture.fake.agent_probe.lock().unwrap() = Some(ferry_protocol::agent::Probe {
        platform_tools: true,
        runtime: Runtime::Codex,
        execution_user: "fake".into(),
        execution_identity: "id".into(),
        executable: None,
        version: None,
        models: vec![],
        authentication: "logged_in".into(),
        directory_exists: true,
        error: Some("missing".into()),
        canonical_directory: Some("/w".into()),
        write_workdir: true,
        file_tools: vec![],
    });
    let error = fixture.execute().await.unwrap_err();
    assert_eq!(key(&error), "workflow.agentCliMissing");
    let message = error.2.as_ref().unwrap();
    assert_eq!(message.params["host"], "Build runner");
    assert_eq!(message.params["cli"], "codex");
    assert!(fixture.requests().is_empty());

    let fixture = Fixture::new().await;
    *fixture.fake.agent_probe.lock().unwrap() = Some(ferry_protocol::agent::Probe {
        platform_tools: false,
        runtime: Runtime::Codex,
        execution_user: "fake".into(),
        execution_identity: "id".into(),
        executable: Some("/bin/codex".into()),
        version: None,
        models: vec![],
        authentication: "logged_in".into(),
        directory_exists: true,
        error: None,
        canonical_directory: Some("/w".into()),
        write_workdir: true,
        file_tools: vec![],
    });
    assert_eq!(
        key(&fixture.execute().await.unwrap_err()),
        "workflow.agentRunnerUpgrade"
    );
}

#[tokio::test]
async fn read_only_incapable_clis_and_unusable_agents_are_rejected_at_save_and_execution() {
    let fixture = Fixture::new().await;
    for runtime in ["kimi", "grok"] {
        // The CLI comes from the Agent's model.
        ai_agents::Entity::update_many()
            .col_expr(ai_agents::Column::Runtime, Expr::value(runtime))
            .col_expr(
                ai_agents::Column::ModelProfileId,
                Expr::value(crate::agents::tests::test_cli_model(1, runtime)),
            )
            .filter(ai_agents::Column::Id.eq(fixture.agent.id))
            .exec(&fixture.s.db)
            .await
            .unwrap();
        let error = fixture.execute().await.unwrap_err();
        assert_eq!(key(&error), "workflow.agentReadOnly", "{runtime}");
        assert_eq!(error.2.as_ref().unwrap().params["cli"], runtime);
        let error = create_workflow(&fixture.s, graph(params(fixture.agent.id)))
            .await
            .unwrap_err();
        assert_eq!(key(&error), "workflow.agentReadOnly", "{runtime}");
        assert_eq!(
            error.2.as_ref().unwrap().params["path"],
            "/nodes/1/params/agent_id"
        );
    }
    assert!(fixture.requests().is_empty());
    // Unknown and disabled Agents cannot be saved or run.
    let error = create_workflow(&fixture.s, graph(params(fixture.agent.id + 999)))
        .await
        .unwrap_err();
    assert_eq!(key(&error), "workflow.agentUnavailable");
    // An Agent without a model is never run, not even by the host's own CLI.
    ai_agents::Entity::update_many()
        .col_expr(ai_agents::Column::ModelProfileId, Expr::value(0_i64))
        .filter(ai_agents::Column::Id.eq(fixture.agent.id))
        .exec(&fixture.s.db)
        .await
        .unwrap();
    let error = fixture.execute().await.unwrap_err();
    assert_eq!(key(&error), "agents.modelRequired");
    assert_eq!(super::super::run::failure_reason(&error), "agent_invalid");
    assert!(fixture.requests().is_empty());
    ai_agents::Entity::update_many()
        .col_expr(ai_agents::Column::Runtime, Expr::value("codex"))
        .col_expr(
            ai_agents::Column::ModelProfileId,
            Expr::value(crate::agents::tests::test_cli_model(1, "codex")),
        )
        .col_expr(ai_agents::Column::State, Expr::value("disabled"))
        .filter(ai_agents::Column::Id.eq(fixture.agent.id))
        .exec(&fixture.s.db)
        .await
        .unwrap();
    let error = create_workflow(&fixture.s, graph(params(fixture.agent.id)))
        .await
        .unwrap_err();
    assert_eq!(key(&error), "workflow.agentUnavailable");
    let error = fixture.execute().await.unwrap_err();
    assert_eq!(key(&error), "workflow.agentUnavailable");
    assert_eq!(super::super::run::failure_reason(&error), "agent_invalid");
}

#[tokio::test]
async fn save_time_validation_bounds_the_prompt_and_requires_an_object_schema() {
    let fixture = Fixture::new().await;
    let agent = fixture.agent.id;
    let save = |params: Value| create_workflow(&fixture.s, graph(params));
    save(params(agent)).await.unwrap();
    // Expressions in the prompt are accepted and checked again once resolved.
    let mut expression = params(agent);
    expression["prompt"] = json!("=Analyze {{ $json.since }}");
    // (Saving compiles expressions in the JavaScript sandbox, absent in unit tests.)
    super::super::graph::validate(&graph(expression).to_string()).unwrap();
    let cases = [
        (
            "output_schema",
            json!({"type":"array"}),
            "workflow.graphSchema",
        ),
        (
            "output_schema",
            json!({"properties":{}}),
            "workflow.graphSchema",
        ),
        (
            "output_schema",
            json!({"type":"object","properties":{"a":{"type":"nope"}}}),
            "workflow.agentSchema",
        ),
        (
            "output_schema",
            json!({"type":"object","description":"x".repeat(64 * 1024)}),
            "workflow.agentSchema",
        ),
        (
            "output_schema",
            json!({"type":"object","properties":{"a":{"$ref":"https://example.invalid/s.json"}}}),
            "workflow.agentSchema",
        ),
        ("prompt", json!(""), "workflow.graphSchema"),
        ("prompt", json!("   "), "workflow.agentPrompt"),
        (
            "prompt",
            json!("x".repeat(32 * 1024 + 1)),
            "workflow.graphSchema",
        ),
        // Within the character limit but over 32 KiB of UTF-8.
        ("prompt", json!("数".repeat(12_000)), "workflow.agentPrompt"),
        ("timeout_s", json!(29), "workflow.graphSchema"),
        ("timeout_s", json!(1801), "workflow.graphSchema"),
        ("max_tool_calls", json!(0), "workflow.graphSchema"),
        ("max_tool_calls", json!(129), "workflow.graphSchema"),
        ("agent_id", json!("0"), "workflow.graphSchema"),
        ("agent_id", json!("=1"), "workflow.graphSchema"),
        ("host_id", json!("1"), "workflow.graphSchema"),
    ];
    for (field, value, expected) in cases {
        let mut invalid = params(agent);
        invalid[field] = value;
        let error = save(invalid).await.unwrap_err();
        assert_eq!(key(&error), expected, "{field}");
        assert_eq!(error.0, StatusCode::BAD_REQUEST, "{field}");
    }
    // Without file output the schema is required, and the message says so.
    let mut missing = params(agent);
    missing.as_object_mut().unwrap().remove("output_schema");
    assert_eq!(
        key(&save(missing).await.unwrap_err()),
        "workflow.agentSchema"
    );
}

#[tokio::test]
async fn no_host_and_oversized_prompts_fail_before_the_runner_is_used() {
    let fixture = Fixture::new().await;
    // The node's own target wins over the Agent host; a missing host fails clearly.
    let input = Item {
        json: Map::from_iter([("blob".into(), json!("x".repeat(70 * 1024)))]),
        binary: Default::default(),
        paired: vec![],
    };
    let node = json!({"id":"agent","type":"ai.agent.run"});
    let error = execute_with(
        &fixture.s,
        &fixture.run,
        0,
        &node,
        &[input],
        &[params(fixture.agent.id)],
        Limits {
            timeout: Duration::from_secs(30),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(key(&error), "workflow.agentInputTooLarge");
    ai_agents::Entity::update_many()
        .col_expr(ai_agents::Column::HostId, Expr::value(0))
        .filter(ai_agents::Column::Id.eq(fixture.agent.id))
        .exec(&fixture.s.db)
        .await
        .unwrap();
    // Without a pinned host the model's hosts (here: any online Runner) are used.
    let set_state = |state: &'static str| {
        crate::entities::hosts::Entity::update_many()
            .col_expr(crate::entities::hosts::Column::State, Expr::value(state))
            .filter(crate::entities::hosts::Column::Id.eq(fixture.host))
            .exec(&fixture.s.db)
    };
    set_state("offline").await.unwrap();
    assert_eq!(
        key(&fixture.execute().await.unwrap_err()),
        "models.noRunner"
    );
    set_state("online").await.unwrap();
    fixture.script(answer(r#"{"summary":"automatic"}"#));
    assert_eq!(
        fixture.execute().await.unwrap()[0].json["summary"],
        "automatic"
    );
    // An explicit target is used instead.
    fixture.script(answer(r#"{"summary":"targeted"}"#));
    let node = json!({"id":"agent","type":"ai.agent.run",
        "settings":{"runtime":"host","target_kind":"host","target_ref":fixture.host.to_string()}});
    let items = execute_with(
        &fixture.s,
        &fixture.run,
        0,
        &node,
        &[Item {
            json: Map::new(),
            binary: Default::default(),
            paired: vec![],
        }],
        &[params(fixture.agent.id)],
        Limits {
            timeout: Duration::from_secs(30),
        },
    )
    .await
    .unwrap();
    assert_eq!(items[0].json["summary"], "targeted");
    // No input items: nothing to analyse, no run.
    let none = execute_with(
        &fixture.s,
        &fixture.run,
        0,
        &node,
        &[],
        &[params(fixture.agent.id)],
        Limits {
            timeout: Duration::from_secs(30),
        },
    )
    .await
    .unwrap();
    assert!(none.is_empty());
    assert_eq!(fixture.requests().len(), 2);
}

#[tokio::test]
async fn model_hosts_are_tried_in_order_and_every_failure_is_named() {
    let fixture = Fixture::new().await;
    // A second Runner listed first on the model; the Agent is no longer pinned.
    let first = i64::from(rand::random::<u32>()) + 1_000_000;
    crate::entities::hosts::Entity::insert(
        fake::host_row(1, first, "Preferred runner", "online").into_active_model(),
    )
    .exec(&fixture.s.db)
    .await
    .unwrap();
    let preferred = fake::register(first);
    ai_agents::Entity::update_many()
        .col_expr(ai_agents::Column::HostId, Expr::value(0))
        .filter(ai_agents::Column::Id.eq(fixture.agent.id))
        .exec(&fixture.s.db)
        .await
        .unwrap();
    crate::entities::ai_model_profiles::Entity::update_many()
        .col_expr(
            crate::entities::ai_model_profiles::Column::Hosts,
            Expr::value(json!([first.to_string(), fixture.host.to_string()]).to_string()),
        )
        .filter(crate::entities::ai_model_profiles::Column::Id.eq(fixture.agent.model_profile_id))
        .exec(&fixture.s.db)
        .await
        .unwrap();
    // The preferred host cannot run the CLI, so the next one does.
    let mut missing = fake::probe_answer(Runtime::Codex, "/w");
    missing.executable = None;
    *preferred.agent_probe.lock().unwrap() = Some(missing.clone());
    fixture.script(answer(r#"{"summary":"second"}"#));
    assert_eq!(
        fixture.execute().await.unwrap()[0].json["summary"],
        "second"
    );
    assert!(preferred.agent_requests.lock().unwrap().is_empty());
    // Both failing: one error names each host and its reason, in order.
    *fixture.fake.agent_probe.lock().unwrap() = Some(missing);
    let error = fixture.execute().await.unwrap_err();
    assert_eq!(key(&error), "models.noHost");
    assert_eq!(super::super::run::failure_reason(&error), "agent_failed");
    let text = crate::i18n::render(crate::i18n::Locale::EnUs, error.2.as_ref().unwrap());
    let (a, b) = (
        text.find("Preferred runner").unwrap(),
        text.find("Build runner").unwrap(),
    );
    assert!(a < b, "{text}");
    assert!(text.contains("codex"), "{text}");
}

fn file_params(agent: i64, files: Value, schema: Option<Value>) -> Value {
    let mut params = json!({"agent_id":agent.to_string(),"prompt":"Make the weekly deck.",
        "timeout_s":600,"max_tool_calls":64,"files":files});
    if let Some(schema) = schema {
        params["output_schema"] = schema;
    }
    params
}

#[tokio::test]
async fn files_from_the_output_folder_become_workflow_files_and_the_workdir_is_removed() {
    let fixture = Fixture::new().await;
    fixture.fake.browsable.store(true, Ordering::SeqCst);
    let dir = format!("/fake/workspaces/{}-{}", fixture.run.id, fixture.run.id);
    fixture.fake.agent_probe.lock().unwrap().replace({
        let mut probe = fake::probe_answer(Runtime::Codex, &dir);
        probe.file_tools = vec!["python 3.12.1 (python-pptx, matplotlib)".into()];
        probe
    });
    fixture.script(vec![
        AgentStep::Write("output/周报.pdf".into(), "%PDF-1.7 weekly".into()),
        AgentStep::Write(
            "output/charts/commits.csv".into(),
            "名称,提交\nA,3\n".into(),
        ),
        AgentStep::Write("scratch/build.py".into(), "print('helper')".into()),
        text(
            "final",
            "Wrote 周报.pdf and charts/commits.csv with python-pptx.",
            true,
        ),
        finished(),
    ]);
    let params = file_params(fixture.agent.id, json!({"enabled":true}), None);
    let items = fixture
        .execute_params(params, Duration::from_secs(30))
        .await
        .unwrap();
    let request = &fixture.requests()[0];
    // A fresh per-execution directory, writable only there, never asking.
    assert_eq!(request.cwd, dir);
    assert!(request.write_workdir);
    assert_eq!(request.permission, Permission::ReadOnly);
    assert!(request.prompt.contains("./output/"));
    assert!(request.prompt.contains("python-pptx"));
    assert!(!request.prompt.contains("Do not modify files"));
    let item = &items[0];
    assert!(item.json["summary"].as_str().unwrap().contains("周报.pdf"));
    let files = item.json["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    let names: Vec<_> = files.iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["commits.csv", "周报.pdf"]);
    assert_eq!(files[1]["mime"], "application/pdf");
    assert_eq!(files[0]["meta"]["path"], "charts/commits.csv");
    let sha = files[1]["sha256"].as_str().unwrap();
    // Stored through the shared file-artifact helper: same shape as report.render,
    // with a signed download link only when a public address is configured.
    assert_eq!(files[1]["artifact_id"], format!("sha256:{sha}"));
    let url = files[1]["download_url"].as_str().unwrap();
    assert!(url.is_empty() || url.contains("/api/files/"), "{url}");
    assert!(!url.contains("/assets/"), "no session-only asset URL");
    assert_eq!(item.binary.len(), 2);
    assert_eq!(item.binary["file_2"].asset, format!("sha256:{sha}"));
    assert_eq!(item.binary["file_2"].name, "周报.pdf");
    // The stored bytes are the produced file.
    let asset = crate::entities::content_assets::Entity::find()
        .filter(crate::entities::content_assets::Column::Sha256.eq(sha))
        .one(&fixture.s.db)
        .await
        .unwrap()
        .unwrap();
    let backend = crate::artifact_store::ArtifactStore::for_state(&fixture.s)
        .await
        .unwrap();
    let store = crate::content_assets::Store {
        db: &fixture.s.db,
        backend: &backend,
        vault: &fixture.s.vault,
        ids: &fixture.s.ids,
        quota: crate::content_assets::DEFAULT_QUOTA,
    };
    assert_eq!(
        store
            .read_bytes(&fixture.s.db, asset.workspace_id, asset.id)
            .await
            .unwrap(),
        b"%PDF-1.7 weekly"
    );
    // Nothing the CLI wrote is left on the host.
    assert!(
        !fixture
            .fake
            .files
            .lock()
            .unwrap()
            .keys()
            .any(|path| path.starts_with(&dir)),
        "{:?}",
        fixture
            .fake
            .files
            .lock()
            .unwrap()
            .keys()
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn file_output_limits_types_links_and_old_runners_fail_clearly() {
    let fixture = Fixture::new().await;
    fixture.fake.browsable.store(true, Ordering::SeqCst);
    let run = |steps: Vec<AgentStep>, files: Value| {
        fixture.script(steps);
        fixture.execute_params(
            file_params(fixture.agent.id, files, None),
            Duration::from_secs(30),
        )
    };
    let done = || vec![text("final", "done", true), finished()];
    let with = |writes: Vec<(&str, &str)>| {
        let mut steps: Vec<AgentStep> = writes
            .into_iter()
            .map(|(path, content)| AgentStep::Write(path.into(), content.into()))
            .collect();
        steps.extend(done());
        steps
    };
    // Nothing produced.
    let error = run(done(), json!({"enabled":true})).await.unwrap_err();
    assert_eq!(key(&error), "workflow.agentNoFiles");
    // A disallowed format.
    let error = run(
        with(vec![("output/a.md", "# hi")]),
        json!({"enabled":true,"formats":["pdf"]}),
    )
    .await
    .unwrap_err();
    assert_eq!(key(&error), "workflow.agentFileFormat");
    // A fake binary: text pretending to be a deck.
    let error = run(
        with(vec![("output/deck.pptx", "slides as text")]),
        json!({"enabled":true}),
    )
    .await
    .unwrap_err();
    assert_eq!(key(&error), "workflow.agentFileMismatch");
    // Too many files.
    let error = run(
        with(vec![("output/1.md", "a"), ("output/2.md", "b")]),
        json!({"enabled":true,"max_files":1}),
    )
    .await
    .unwrap_err();
    assert_eq!(key(&error), "workflow.agentFilesTooMany");
    // A link is never followed: only the regular file is delivered.
    let dir = format!("/fake/workspaces/{}-{}", fixture.run.id, fixture.run.id);
    fixture
        .fake
        .symlinks
        .lock()
        .unwrap()
        .insert(format!("{dir}/output/secret.txt"));
    let items = run(
        with(vec![
            ("output/secret.txt", "host secret"),
            ("output/ok.md", "# ok"),
        ]),
        json!({"enabled":true}),
    )
    .await
    .unwrap();
    let names: Vec<_> = items[0].json["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["ok.md"]);
    // An older Runner cannot confine writes: refused before the CLI runs.
    let before = fixture.requests().len();
    fixture.fake.agent_probe.lock().unwrap().replace({
        let mut probe = fake::probe_answer(Runtime::Codex, "/");
        probe.write_workdir = false;
        probe
    });
    let error = run(done(), json!({"enabled":true})).await.unwrap_err();
    assert_eq!(key(&error), "workflow.agentFilesRunnerUpgrade");
    assert_eq!(fixture.requests().len(), before);
}

#[tokio::test]
async fn file_output_requires_a_confinable_cli_and_keeps_the_schema_optional_only_then() {
    // Without files the schema is still required; with files it is optional.
    let base = json!({"agent_id":"1","prompt":"p","timeout_s":600,"max_tool_calls":64});
    assert_eq!(
        key(&validate(&base, "/nodes/1").unwrap_err()),
        "workflow.agentSchema"
    );
    let mut files = base.clone();
    files["files"] = json!({"enabled":true});
    validate(&files, "/nodes/1").unwrap();
    files["files"]["max_files"] = json!(99);
    assert_eq!(
        key(&validate(&files, "/nodes/1").unwrap_err()),
        "workflow.agentFilesInvalid"
    );
    // A CLI that cannot confine its writes is refused when saving and running.
    let fixture = Fixture::new().await;
    ai_agents::Entity::update_many()
        .col_expr(ai_agents::Column::Runtime, Expr::value("gemini"))
        .col_expr(
            ai_agents::Column::ModelProfileId,
            Expr::value(crate::agents::tests::test_cli_model(1, "gemini")),
        )
        .filter(ai_agents::Column::Id.eq(fixture.agent.id))
        .exec(&fixture.s.db)
        .await
        .unwrap();
    let error = fixture
        .execute_params(
            file_params(fixture.agent.id, json!({"enabled":true}), None),
            Duration::from_secs(30),
        )
        .await
        .unwrap_err();
    assert_eq!(key(&error), "workflow.agentFilesRuntime");
    let error = create_workflow(
        &fixture.s,
        graph(file_params(fixture.agent.id, json!({"enabled":true}), None)),
    )
    .await
    .unwrap_err();
    assert_eq!(key(&error), "workflow.agentFilesRuntime");
    assert_eq!(error.2.as_ref().unwrap().params["cli"], "gemini");
    // Gemini can still be saved in a plain read-only node.
    create_workflow(&fixture.s, graph(params(fixture.agent.id)))
        .await
        .unwrap();
    assert!(fixture.requests().is_empty());
}

#[tokio::test]
async fn mcp_node_catalog_is_bounded_searchable_and_complete() {
    let fixture = Fixture::new().await;
    let mut offset = 0;
    let mut types = std::collections::BTreeSet::new();
    loop {
        let reply = crate::platform_tools::dispatch(
            &fixture.s,
            admin(1),
            crate::platform_tools::Call {
                method: "GET".into(),
                path: format!("/workflow-node-catalog?offset={offset}"),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(reply["status"], 200, "{reply}");
        assert!(serde_json::to_vec(&reply).unwrap().len() < 256 * 1024);
        let data = &reply["data"];
        for key in ["nodes", "builtin_nodes"] {
            for node in data[key].as_array().unwrap() {
                assert!(node["params_schema"].is_object());
                assert!(types.insert(node["type"].as_str().unwrap().to_owned()));
            }
        }
        if let Some(next) = data["next_offset"].as_u64() {
            assert!(next > offset);
            offset = next;
        } else {
            assert_eq!(types.len() as u64, data["total"].as_u64().unwrap());
            break;
        }
    }
    assert!(types.contains("core.trigger.manual"));
    assert!(types.contains("ferry.request"));
    assert!(types.len() > 50);
    let reply = crate::platform_tools::dispatch(
        &fixture.s,
        admin(1),
        crate::platform_tools::Call {
            method: "GET".into(),
            path: "/workflow-node-catalog?type=core.trigger.manual".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(reply["status"], 200, "{reply}");
    assert_eq!(reply["data"]["total"], 1);
    assert_eq!(
        reply["data"]["builtin_nodes"][0]["type"],
        "core.trigger.manual"
    );
}

#[tokio::test]
async fn the_published_definition_format_matches_the_validated_schema() {
    let fixture = Fixture::new().await;
    let format = crate::platform_tools::dispatch(
        &fixture.s,
        admin(1),
        crate::platform_tools::Call {
            method: "GET".into(),
            path: "/workflow-definition-format".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(format["status"], 200, "{format}");
    assert_eq!(
        format["data"]["schema"],
        super::super::graph::graph_schema()
    );
    super::super::graph::validate(&format["data"]["example"].to_string()).unwrap();
}

#[tokio::test]
#[ignore = "requires a built real ferry-sandbox via absolute FERRY_SANDBOX_PATH"]
async fn the_published_definition_example_validates_and_saves_with_real_ids() {
    assert!(std::path::Path::new(&std::env::var_os("FERRY_SANDBOX_PATH").unwrap()).is_file());
    let fixture = Fixture::new().await;
    let format = crate::platform_tools::dispatch(
        &fixture.s,
        admin(1),
        crate::platform_tools::Call {
            method: "GET".into(),
            path: "/workflow-definition-format".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(format["status"], 200, "{format}");
    assert_eq!(
        format["data"]["schema"],
        super::super::graph::graph_schema()
    );
    let mut example = format["data"]["example"].clone();
    super::super::graph::validate(&example.to_string()).unwrap();
    // Keep the published expressions intact: POST must syntax-check the real
    // example, not a literal-only substitute. This requires ferry-sandbox.
    for node in example["nodes"].as_array_mut().unwrap() {
        if node["type"] == "ai.agent.run" {
            node["params"]["agent_id"] = json!(fixture.agent.id.to_string());
        }
    }
    let saved = crate::platform_tools::dispatch(
        &fixture.s,
        admin(1),
        crate::platform_tools::Call {
            method: "POST".into(),
            path: "/workflows".into(),
            body: json!({"name":"Contributors report","definition":example.to_string()}),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(saved["status"], 201, "{saved}");
    let id: Id = serde_json::from_value(saved["data"]["id"].clone()).unwrap();
    let row = crate::entities::workflows::Entity::find_by_id(i64::from(id))
        .one(&fixture.s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&row.definition).unwrap(),
        example
    );
    let count = fixture.workflow_count().await;
    let mut missing = example.clone();
    missing.as_object_mut().unwrap().remove("engine");
    let mut invalid_params = example;
    invalid_params["nodes"][2]["params"]
        .as_object_mut()
        .unwrap()
        .remove("agent_id");
    for (definition, path) in [
        (missing, "/engine"),
        (invalid_params, "/nodes/2/params/agent_id"),
    ] {
        let rejected = crate::platform_tools::dispatch(
            &fixture.s,
            admin(1),
            crate::platform_tools::Call {
                method: "POST".into(),
                path: "/workflows".into(),
                body: json!({"name":"Invalid report","definition":definition.to_string()}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(rejected["status"], 400, "{rejected}");
        let detail = rejected["data"]["error"].as_str().unwrap();
        assert!(detail.contains(&format!("{path} (")), "{detail}");
        assert!(detail.contains("required"), "{detail}");
    }
    assert_eq!(fixture.workflow_count().await, count);
}
