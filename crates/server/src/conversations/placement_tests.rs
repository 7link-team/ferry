use super::*;
use crate::{
    agents::tests::admin,
    conversations::tests::{seeded, sent},
    hosts::fake::{self, AgentStep, FakeHost},
    i18n::{Locale, render_stored},
};
use ferry_protocol::agent::{Event, Outcome};
use std::sync::{Arc, atomic::Ordering};

/// An online Runner served by a fake channel. Fakes are registered process-wide,
/// so ids are random rather than from this test's own id generator.
async fn runner(s: &AppState, name: &str) -> (i64, Arc<FakeHost>) {
    let id = i64::from(rand::random::<u32>()) + 10_000_000;
    hosts::Entity::insert(fake::host_row(1, id, name, "online").into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    (id, fake::register(id))
}

/// A shared Agent on the Claude test model, pinned to no host.
async fn floating_agent(s: &AppState) -> ai_agents::Model {
    let data = json!({"name":"Floating","runtime":"claude",
        "model_profile_id":crate::agents::tests::test_cli_model(1,"claude").to_string(),
        "owner_id":"1","system_prompt":"","visibility":"workspace","state":"enabled"});
    let (_, Json(created)) = crate::agents::create(
        State(s.clone()),
        admin(1),
        Json(serde_json::from_value(data).unwrap()),
    )
    .await
    .unwrap();
    ai_agents::Entity::find_by_id(created["id"].as_str().unwrap().parse::<i64>().unwrap())
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
}

async fn conversation(s: &AppState, agent: &ai_agents::Model, extra: Value) -> Id {
    let mut body = json!({"agent_id":Id::from(agent.id)});
    for (key, value) in extra.as_object().cloned().unwrap_or_default() {
        body[key] = value;
    }
    let (status, Json(value)) = super::super::create(
        State(s.clone()),
        admin(1),
        Json(serde_json::from_value(body).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::CREATED);
    serde_json::from_value(value["id"].clone()).unwrap()
}

fn reply(session: &str, text: &str) -> Vec<AgentStep> {
    vec![
        AgentStep::Event(Event::Session {
            native_session_id: session.into(),
        }),
        AgentStep::Event(Event::Text {
            item_id: "answer".into(),
            text: text.into(),
            replace: true,
        }),
        AgentStep::Event(Event::Finished {
            outcome: Outcome::Completed,
            usage: None,
        }),
    ]
}

/// Sends a message and runs its turn to the end, as the scheduler would.
async fn turn(s: &AppState, id: Id, client: &str) -> turns::Model {
    let turn = sent(s, id, client).await;
    let (_stop, stop) = tokio::sync::watch::channel(false);
    super::super::run::execute(s.clone(), turn, stop)
        .await
        .unwrap();
    turns::Entity::find_by_id(turn)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
}

async fn stored(s: &AppState, id: Id) -> (Configuration, String) {
    let c = conversations::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    (serde_json::from_str(&c.config).unwrap(), c.binding)
}

async fn notices(s: &AppState, id: Id) -> Vec<String> {
    chat::Entity::find()
        .filter(chat::Column::ConversationId.eq(id))
        .filter(chat::Column::Role.eq("notice"))
        .order_by_asc(chat::Column::Id)
        .all(&s.db)
        .await
        .unwrap()
        .iter()
        .map(|m| render_stored(Locale::EnUs, &m.prompt))
        .collect()
}

fn requests(fake: &FakeHost) -> Vec<ferry_protocol::agent::Request> {
    fake.agent_requests.lock().unwrap().clone()
}

#[tokio::test]
async fn creating_needs_no_runner_and_the_first_message_names_why_none_runs() {
    let (_dir, s, _) = seeded().await;
    let agent = floating_agent(&s).await;
    let id = conversation(&s, &agent, json!({})).await;
    let (config, binding) = stored(&s, id).await;
    assert_eq!(binding, UNBOUND);
    assert_eq!(i64::from(config.host_id), 0);
    let Json(detail) = super::super::detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    assert!(detail["host"].is_null());
    let failed = turn(&s, id, "no-runner").await;
    assert_eq!(failed.state, "failed");
    assert!(failed.error.contains("models.noRunner"), "{}", failed.error);
    assert_eq!(stored(&s, id).await.1, UNBOUND);
}

#[tokio::test]
async fn the_first_message_binds_a_host_that_later_messages_keep() {
    let (_dir, s, _) = seeded().await;
    let agent = floating_agent(&s).await;
    let (alpha, alpha_fake) = runner(&s, "alpha").await;
    let id = conversation(&s, &agent, json!({})).await;
    alpha_fake
        .agent_scripts
        .lock()
        .unwrap()
        .push_back(reply("s1", "first answer"));
    assert_eq!(turn(&s, id, "one").await.state, "completed");
    let (config, binding) = stored(&s, id).await;
    assert_eq!(i64::from(config.host_id), alpha);
    assert_eq!(config.cwd, format!("/fake/workspaces/{id}-{}", agent.id));
    assert_ne!(binding, UNBOUND);
    let Json(detail) = super::super::detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    assert_eq!(detail["host"]["name"], "alpha");
    // Another Runner appearing does not move a working conversation.
    let (_, beta_fake) = runner(&s, "beta").await;
    alpha_fake
        .agent_scripts
        .lock()
        .unwrap()
        .push_back(reply("s1", "second answer"));
    assert_eq!(turn(&s, id, "two").await.state, "completed");
    let sent = requests(&alpha_fake);
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].native_session_id.as_deref(), Some("s1"));
    assert!(requests(&beta_fake).is_empty());
    assert!(notices(&s, id).await.is_empty());
}

#[tokio::test]
async fn concurrent_first_turns_bind_one_host_once() {
    let (_dir, s, _) = seeded().await;
    let agent = floating_agent(&s).await;
    let (alpha, _alpha_fake) = runner(&s, "alpha").await;
    let id = conversation(&s, &agent, json!({})).await;
    let turn = sent(&s, id, "race").await;
    let row = turns::Entity::find_by_id(turn)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let c = conversations::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let scope = admin(1);
    let usable = |_: &str| true;
    let (first, second) = tokio::join!(
        place(&s, &scope, &row, &c, &agent, usable),
        place(&s, &scope, &row, &c, &agent, usable)
    );
    let (first, second) = (first.unwrap(), second.unwrap());
    assert_eq!(first.host.id, alpha);
    assert_eq!(second.host.id, alpha);
    // Exactly one of them bound it; the other used that binding.
    assert!(first.fresh != second.fresh);
    let (_, binding) = stored(&s, id).await;
    assert_eq!(
        serde_json::from_str::<Binding>(&binding)
            .unwrap()
            .canonical_directory,
        first.binding.canonical_directory
    );
    assert!(notices(&s, id).await.is_empty());
}

#[tokio::test]
async fn an_unusable_host_is_replaced_with_history_and_a_notice() {
    let (_dir, s, _) = seeded().await;
    let agent = floating_agent(&s).await;
    let (_, alpha_fake) = runner(&s, "alpha").await;
    let id = conversation(&s, &agent, json!({})).await;
    alpha_fake
        .agent_scripts
        .lock()
        .unwrap()
        .push_back(reply("s1", "first answer"));
    assert_eq!(turn(&s, id, "one").await.state, "completed");
    let (beta, beta_fake) = runner(&s, "beta").await;
    alpha_fake.unreachable.store(true, Ordering::SeqCst);
    beta_fake
        .agent_scripts
        .lock()
        .unwrap()
        .push_back(reply("s2", "moved answer"));
    assert_eq!(turn(&s, id, "two").await.state, "completed");
    let (config, _) = stored(&s, id).await;
    assert_eq!(i64::from(config.host_id), beta);
    let moved = requests(&beta_fake);
    assert_eq!(moved.len(), 1);
    // The old native session lives on alpha; Ferry's history goes along instead.
    assert_eq!(moved[0].native_session_id, None);
    assert!(
        moved[0].prompt.contains("first answer"),
        "{}",
        moved[0].prompt
    );
    let notices = notices(&s, id).await;
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0].contains("alpha") && notices[0].contains("beta"),
        "{}",
        notices[0]
    );
    // The notice is part of the conversation as read by its members.
    let Json(detail) = super::super::detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    let shown = detail["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "notice")
        .unwrap();
    assert!(
        !shown["prompt"]
            .as_str()
            .unwrap()
            .contains("$system_message")
    );
    assert_eq!(detail["host"]["name"], "beta");
}

#[tokio::test]
async fn a_bound_host_that_is_only_reconnecting_keeps_its_conversation() {
    let (_dir, s, _) = seeded().await;
    let agent = floating_agent(&s).await;
    let (alpha, alpha_fake) = runner(&s, "alpha").await;
    let id = conversation(&s, &agent, json!({})).await;
    alpha_fake
        .agent_scripts
        .lock()
        .unwrap()
        .push_back(reply("s1", "first answer"));
    assert_eq!(turn(&s, id, "one").await.state, "completed");
    // Another host is available, but alpha is only gone for a moment: it was
    // seen just now (a real Runner's heartbeats keep this current).
    hosts::Entity::update_many()
        .col_expr(
            hosts::Column::SeenAt,
            sea_orm::sea_query::Expr::value(crate::db::now()),
        )
        .filter(hosts::Column::Id.eq(alpha))
        .exec(&s.db)
        .await
        .unwrap();
    let (_, beta_fake) = runner(&s, "beta").await;
    alpha_fake.unreachable.store(true, Ordering::SeqCst);
    let back = alpha_fake.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        back.unreachable.store(false, Ordering::SeqCst);
    });
    alpha_fake
        .agent_scripts
        .lock()
        .unwrap()
        .push_back(reply("s1", "same host answer"));
    assert_eq!(turn(&s, id, "two").await.state, "completed");
    let (config, _) = stored(&s, id).await;
    assert_eq!(i64::from(config.host_id), alpha, "stayed on its host");
    assert!(requests(&beta_fake).is_empty(), "never moved to beta");
    // Its CLI session continues; no "switched host" notice.
    assert_eq!(
        requests(&alpha_fake)
            .last()
            .unwrap()
            .native_session_id
            .as_deref(),
        Some("s1")
    );
    assert!(notices(&s, id).await.is_empty());
}

#[tokio::test]
async fn a_chosen_directory_never_moves_by_itself_and_can_be_released() {
    let (dir, s, _) = seeded().await;
    let agent = floating_agent(&s).await;
    let (alpha, alpha_fake) = runner(&s, "alpha").await;
    let cwd = dir.path().to_string_lossy().replace('\\', "/");
    let id = conversation(&s, &agent, json!({"cwd":cwd})).await;
    alpha_fake
        .agent_scripts
        .lock()
        .unwrap()
        .push_back(reply("s1", "first answer"));
    assert_eq!(turn(&s, id, "one").await.state, "completed");
    assert_eq!(i64::from(stored(&s, id).await.0.host_id), alpha);
    let (beta, beta_fake) = runner(&s, "beta").await;
    alpha_fake.unreachable.store(true, Ordering::SeqCst);
    let failed = turn(&s, id, "two").await;
    assert_eq!(failed.state, "failed");
    assert!(requests(&beta_fake).is_empty());
    assert_eq!(i64::from(stored(&s, id).await.0.host_id), alpha);
    assert_eq!(
        super::super::view_turn(&failed)["error_key"],
        "conversations.hostUnavailable"
    );
    let text = render_stored(Locale::EnUs, &failed.error);
    assert!(text.contains("alpha"), "{text}");
    // The user releases the host; the next message runs elsewhere, in a
    // Runner-managed directory, even once the old host is back.
    rebind(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id)),
        Json(Rebind { agent_id: None }),
    )
    .await
    .unwrap();
    let (released, binding) = stored(&s, id).await;
    assert_eq!(binding, UNBOUND);
    assert_eq!(released.pinned_directory, Some(false));
    assert!(
        rebind(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id)),
            Json(Rebind { agent_id: None }),
        )
        .await
        .is_err()
    );
    alpha_fake.unreachable.store(false, Ordering::SeqCst);
    beta_fake
        .agent_scripts
        .lock()
        .unwrap()
        .push_back(reply("s2", "elsewhere"));
    assert_eq!(turn(&s, id, "three").await.state, "completed");
    let (config, _) = stored(&s, id).await;
    assert_eq!(i64::from(config.host_id), beta);
    assert_eq!(config.cwd, format!("/fake/workspaces/{id}-{}", agent.id));
    assert_eq!(requests(&alpha_fake).len(), 1);
    let notices = notices(&s, id).await;
    assert_eq!(notices.len(), 2, "{notices:?}");
    assert!(notices[0].contains("alpha"), "{}", notices[0]);
    assert!(
        notices[1].contains("alpha") && notices[1].contains("beta"),
        "{}",
        notices[1]
    );
}

#[tokio::test]
async fn candidates_follow_request_pin_and_model_order() {
    let (_dir, s, _) = seeded().await;
    let mut agent = floating_agent(&s).await;
    let (a, _) = runner(&s, "a").await;
    let (b, _) = runner(&s, "b").await;
    let (c, _) = runner(&s, "c").await;
    let (offline, _) = runner(&s, "offline").await;
    hosts::Entity::update_many()
        .col_expr(
            hosts::Column::State,
            sea_orm::sea_query::Expr::value("offline"),
        )
        .filter(hosts::Column::Id.eq(offline))
        .exec(&s.db)
        .await
        .unwrap();
    let ids = |rows: Vec<hosts::Model>| rows.iter().map(|h| h.id).collect::<Vec<_>>();
    let ws = Id::from(1);
    // No model hosts: every online Runner by id.
    let mut online = vec![a, b, c];
    online.sort();
    assert_eq!(
        ids(agent_hosts::candidates(&s.db, ws, None, &agent, &[])
            .await
            .unwrap()),
        online
    );
    // Model hosts, in order and named even when offline, and nothing else.
    crate::entities::ai_model_profiles::Entity::update_many()
        .col_expr(
            crate::entities::ai_model_profiles::Column::Hosts,
            sea_orm::sea_query::Expr::value(
                json!([offline.to_string(), c.to_string(), a.to_string()]).to_string(),
            ),
        )
        .filter(crate::entities::ai_model_profiles::Column::Id.eq(agent.model_profile_id))
        .exec(&s.db)
        .await
        .unwrap();
    assert_eq!(
        ids(agent_hosts::candidates(&s.db, ws, None, &agent, &[])
            .await
            .unwrap()),
        vec![offline, c, a]
    );
    // A legacy pin comes first, a requested host before it; exclusions apply.
    agent.host_id = b;
    assert_eq!(
        ids(agent_hosts::candidates(&s.db, ws, Some(a), &agent, &[c])
            .await
            .unwrap()),
        vec![a, b, offline]
    );
}

fn producing(session: &str, writes: &[(&str, &str)], text: &str) -> Vec<AgentStep> {
    let mut steps = vec![AgentStep::Event(Event::Session {
        native_session_id: session.into(),
    })];
    for (path, content) in writes {
        steps.push(AgentStep::Write((*path).into(), (*content).into()));
    }
    steps.push(AgentStep::Event(Event::Text {
        item_id: "answer".into(),
        text: text.into(),
        replace: true,
    }));
    steps.push(AgentStep::Event(Event::Finished {
        outcome: Outcome::Completed,
        usage: None,
    }));
    steps
}

async fn reply_attachments(s: &AppState, id: Id) -> Vec<Value> {
    let reply = chat::Entity::find()
        .filter(chat::Column::ConversationId.eq(id))
        .filter(chat::Column::Role.eq("assistant"))
        .order_by_desc(chat::Column::Id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    serde_json::from_str(&reply.attachments).unwrap()
}

#[tokio::test]
async fn files_an_agent_writes_to_output_are_attached_to_its_reply_and_removed_from_the_host() {
    let (_dir, s, _) = seeded().await;
    let agent = floating_agent(&s).await;
    let (_, alpha_fake) = runner(&s, "alpha").await;
    alpha_fake.browsable.store(true, Ordering::SeqCst);
    let id = conversation(&s, &agent, json!({})).await;
    alpha_fake
        .agent_scripts
        .lock()
        .unwrap()
        .push_back(producing(
            "s1",
            &[
                ("output/计划.md", "# 本周计划"),
                ("output/data.csv", "名称,值\nA,1\n"),
                ("notes/draft.txt", "not delivered"),
            ],
            "Wrote 计划.md and data.csv.",
        ));
    assert_eq!(turn(&s, id, "make-files").await.state, "completed");
    // A read-only conversation writes only inside its managed directory.
    let request = &requests(&alpha_fake)[0];
    assert!(request.write_workdir);
    assert!(request.prompt.contains("./output/"));
    assert!(
        request
            .prompt
            .contains("You may write only inside your working directory")
    );
    // The files are downloadable conversation files on the Agent's reply.
    let attachments = reply_attachments(&s, id).await;
    let names: Vec<_> = attachments
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["data.csv", "计划.md"]);
    assert_eq!(attachments[0]["mime"], "text/csv");
    assert_eq!(attachments[1]["size"], "# 本周计划".len());
    let file: Id = serde_json::from_value(attachments[1]["file_id"].clone()).unwrap();
    let stored_file = super::super::files::resolve(&s.db, 1.into(), id, file)
        .await
        .unwrap();
    assert_eq!(stored_file.name, "计划.md");
    // Delivered files leave the host; other working files stay for later turns.
    let dir = format!("/fake/workspaces/{id}-{}", agent.id);
    let left: Vec<String> = alpha_fake.files.lock().unwrap().keys().cloned().collect();
    assert_eq!(left, [format!("{dir}/notes/draft.txt")]);
    // A turn that produces nothing adds nothing.
    alpha_fake
        .agent_scripts
        .lock()
        .unwrap()
        .push_back(reply("s1", "just text"));
    assert_eq!(turn(&s, id, "text-only").await.state, "completed");
    assert!(reply_attachments(&s, id).await.is_empty());
}

#[tokio::test]
async fn file_limits_become_a_notice_and_unconfinable_or_chosen_directories_never_collect() {
    let (dir, s, _) = seeded().await;
    let agent = floating_agent(&s).await;
    let (_, alpha_fake) = runner(&s, "alpha").await;
    alpha_fake.browsable.store(true, Ordering::SeqCst);
    let id = conversation(&s, &agent, json!({})).await;
    let many: Vec<(String, String)> = (0..11)
        .map(|i| (format!("output/{i}.md"), format!("# {i}")))
        .collect();
    let writes: Vec<(&str, &str)> = many.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    alpha_fake
        .agent_scripts
        .lock()
        .unwrap()
        .push_back(producing("s1", &writes, "made eleven"));
    assert_eq!(turn(&s, id, "too-many").await.state, "completed");
    assert!(reply_attachments(&s, id).await.is_empty());
    let notes = notices(&s, id).await;
    assert!(
        notes.iter().any(|n| n.contains("more files than allowed")),
        "{notes:?}"
    );
    // Content that does not match its extension is refused the same way.
    alpha_fake
        .agent_scripts
        .lock()
        .unwrap()
        .push_back(producing(
            "s1",
            &[("output/deck.pptx", "slides as text")],
            "made a deck",
        ));
    alpha_fake.files.lock().unwrap().clear();
    assert_eq!(turn(&s, id, "fake-deck").await.state, "completed");
    assert!(
        notices(&s, id)
            .await
            .iter()
            .any(|n| n.contains("does not match its file type"))
    );
    // A chosen directory is never written to or collected from by this mode.
    let cwd = dir.path().to_string_lossy().replace('\\', "/");
    let chosen = conversation(&s, &agent, json!({"cwd":cwd})).await;
    alpha_fake
        .agent_scripts
        .lock()
        .unwrap()
        .push_back(producing("s2", &[("output/x.md", "# x")], "wrote x"));
    assert_eq!(turn(&s, chosen, "chosen").await.state, "completed");
    let last = requests(&alpha_fake).last().cloned().unwrap();
    assert!(!last.write_workdir);
    assert!(last.prompt.contains("cannot deliver files"));
    assert!(reply_attachments(&s, chosen).await.is_empty());
    // A Runner that cannot confine writes gets the same explanation.
    let other = conversation(&s, &agent, json!({})).await;
    alpha_fake.agent_probe.lock().unwrap().replace({
        let mut probe = fake::probe_answer(
            ferry_protocol::agent::Runtime::Claude,
            &format!("/fake/workspaces/{other}-{}", agent.id),
        );
        probe.write_workdir = false;
        probe
    });
    alpha_fake
        .agent_scripts
        .lock()
        .unwrap()
        .push_back(reply("s3", "cannot"));
    assert_eq!(turn(&s, other, "old-runner").await.state, "completed");
    let last = requests(&alpha_fake).last().cloned().unwrap();
    assert!(!last.write_workdir);
    assert!(last.prompt.contains("too old to deliver files"));
}
