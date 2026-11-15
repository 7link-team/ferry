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
