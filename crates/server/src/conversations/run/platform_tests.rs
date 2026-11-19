use super::*;
use crate::{
    agents::tests::admin,
    conversations::tests::{seeded, sent},
    entities::{definition_revisions, host_groups, workspace_roles},
};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel};

#[tokio::test]
async fn questions_survive_turn_completion_and_custom_replies_continue_exactly_once() {
    let (_dir, s, row, owner) = enabled().await;
    append(
        &s,
        &row,
        &owner,
        Event::Session {
            native_session_id: "question-session".into(),
        },
    )
    .await
    .unwrap();
    let result=platform_call(s.clone(),row.clone(),owner.clone(),"question-1".into(),"ferry_ask_user".into(),
        json!({"title":"Missing connection","question":"How should I continue?","reason":"The configured repository is unavailable.",
            "options":[{"id":"repair","label":"Repair connection","description":"Use corrected connection details."}],"recommended_option_id":"repair"}),None).await.1;
    assert_eq!(result["data"]["state"], "awaiting_user", "{result}");
    let blocked = platform_call(
        s.clone(),
        row.clone(),
        owner.clone(),
        "after-question".into(),
        "ferry_request".into(),
        json!({"method":"GET","path":"/workflows"}),
        None,
    )
    .await
    .1;
    assert_eq!(blocked["status"], 409);
    finish(&s, &row, &owner, Some(Outcome::Completed), "", true)
        .await
        .unwrap();
    let waiting = turns::Entity::find_by_id(row.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(super::super::pending_in(&waiting), 1);
    let answer = || crate::platform_tools::Call {
        method: "POST".into(),
        path: format!(
            "/conversations/{}/turns/{}/question-answers",
            row.conversation_id, row.id
        ),
        body: json!({"request_id":"question-1","action":"continue","answer":"Use a different repository; none of those choices fit."}),
        ..Default::default()
    };
    let conversation = conversations::Entity::find_by_id(row.conversation_id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let self_answer = crate::platform_tools::dispatch_as_agent(
        &s,
        admin(1),
        answer(),
        conversation.agent_id.into(),
    )
    .await
    .unwrap();
    assert_eq!(self_answer["status"], 403);
    let mut revoked = admin(1);
    revoked.permissions.remove("conversations.run");
    let denied = crate::platform_tools::dispatch(&s, revoked, answer())
        .await
        .unwrap();
    assert_eq!(denied["status"], 403);
    let (first, again) = tokio::join!(
        crate::platform_tools::dispatch(&s, admin(1), answer()),
        crate::platform_tools::dispatch(&s, admin(1), answer()),
    );
    let first = first.unwrap();
    let again = again.unwrap();
    assert!(first["ok"].as_bool().unwrap(), "{first}");
    assert!(again["ok"].as_bool().unwrap(), "{again}");
    assert_eq!(first["data"]["id"], again["data"]["id"]);
    let rows = turns::Entity::find()
        .filter(turns::Column::ConversationId.eq(row.conversation_id))
        .all(&s.db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .any(|turn| turn.prompt.contains("none of those choices fit"))
    );
    let answered = turns::Entity::find_by_id(row.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(super::super::pending_in(&answered), 0);
    let denied = crate::platform_tools::dispatch(&s, admin(2), answer())
        .await
        .unwrap();
    assert_eq!(denied["status"], 404);
}

#[tokio::test]
async fn assisted_and_yolo_approval_never_bypass_roles_or_auto_approve_sensitive_assisted_work() {
    use sea_orm::{ColumnTrait, QueryFilter, sea_query::Expr};
    for mode in ["assisted", "yolo"] {
        let (_dir, s, row, owner) = enabled().await;
        let c = conversations::Entity::find_by_id(row.conversation_id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let mut config: Value = serde_json::from_str(&c.config).unwrap();
        config["approval_mode"] = json!(mode);
        conversations::Entity::update_many()
            .col_expr(
                conversations::Column::Config,
                Expr::value(config.to_string()),
            )
            .filter(conversations::Column::Id.eq(c.id))
            .exec(&s.db)
            .await
            .unwrap();
        let created=platform_call(s.clone(),row.clone(),owner.clone(),"auto-group".into(),"ferry_request".into(),
            json!({"method":"POST","path":"/node-groups","body":{"name":"Automatic draft organization","parent_id":"0","position":0}}),None).await.1;
        assert!(created["ok"].as_bool().unwrap(), "{created}");
        let saved = turns::Entity::find_by_id(row.id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let decisions: Value = serde_json::from_str(&saved.decisions).unwrap();
        assert_eq!(decisions["auto-group"]["automatic"], true);
        assert_eq!(decisions["auto-group"]["sent"], true);
        if mode == "assisted" {
            let sensitive=platform_call(s.clone(),row.clone(),owner.clone(),"sensitive".into(),"ferry_request".into(),
                json!({"method":"POST","path":"/nodes/1/commands","body":{"command":"echo should-wait","timeout_seconds":10}}),None).await.1;
            assert!(sensitive.is_null(), "{sensitive}");
        }
        let mut role = workspace_roles::Entity::find_by_id(1)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let mut grants: Vec<String> = serde_json::from_str(&role.permissions).unwrap();
        grants.retain(|grant| grant != "host-groups.create");
        role.permissions = json!(grants).to_string();
        role.into_active_model()
            .reset_all()
            .update(&s.db)
            .await
            .unwrap();
        let denied=platform_call(s.clone(),row.clone(),owner.clone(),"denied".into(),"ferry_request".into(),
            json!({"method":"POST","path":"/node-groups","body":{"name":"Must not exist","parent_id":"0","position":0}}),None).await.1;
        assert_eq!(denied["status"], 403, "{denied}");
        assert_eq!(host_groups::Entity::find().count(&s.db).await.unwrap(), 1);
    }
}

#[tokio::test]
async fn paused_questions_create_no_turn_and_can_resume_with_a_custom_answer() {
    let (_dir, s, row, owner) = enabled().await;
    append(
        &s,
        &row,
        &owner,
        Event::Session {
            native_session_id: "pause-session".into(),
        },
    )
    .await
    .unwrap();
    platform_call(
        s.clone(),
        row.clone(),
        owner.clone(),
        "q".into(),
        "ferry_ask_user".into(),
        json!({"title":"Choose next step","question":"How should I proceed?"}),
        None,
    )
    .await;
    finish(&s, &row, &owner, Some(Outcome::Completed), "", true)
        .await
        .unwrap();
    let call = |action: &str, answer: &str| crate::platform_tools::Call {
        method: "POST".into(),
        path: format!(
            "/conversations/{}/turns/{}/question-answers",
            row.conversation_id, row.id
        ),
        body: json!({"request_id":"q","action":action,"answer":answer}),
        ..Default::default()
    };
    let paused = crate::platform_tools::dispatch(&s, admin(1), call("pause", ""))
        .await
        .unwrap();
    assert_eq!(paused["data"]["state"], "paused", "{paused}");
    assert_eq!(turns::Entity::find().count(&s.db).await.unwrap(), 1);
    let resumed = crate::platform_tools::dispatch(
        &s,
        admin(1),
        call("continue", "Now use my alternative approach"),
    )
    .await
    .unwrap();
    assert!(resumed["ok"].as_bool().unwrap(), "{resumed}");
    assert_eq!(turns::Entity::find().count(&s.db).await.unwrap(), 2);
}

#[tokio::test]
async fn failed_question_continuation_keeps_one_answer_and_retries_without_duplicate_turns() {
    use sea_orm::{ColumnTrait, QueryFilter, sea_query::Expr};
    let (_dir, s, row, owner) = enabled().await;
    append(
        &s,
        &row,
        &owner,
        Event::Session {
            native_session_id: "retry-question-session".into(),
        },
    )
    .await
    .unwrap();
    platform_call(
        s.clone(),
        row.clone(),
        owner.clone(),
        "q".into(),
        "ferry_ask_user".into(),
        json!({"title":"Choose next step","question":"How should I proceed?"}),
        None,
    )
    .await;
    finish(&s, &row, &owner, Some(Outcome::Completed), "", true)
        .await
        .unwrap();
    let call = |action: &str, answer: &str| crate::platform_tools::Call {
        method: "POST".into(),
        path: format!(
            "/conversations/{}/turns/{}/question-answers",
            row.conversation_id, row.id
        ),
        body: json!({"request_id":"q","action":action,"answer":answer}),
        ..Default::default()
    };
    let paused = crate::platform_tools::dispatch(&s, admin(1), call("pause", ""))
        .await
        .unwrap();
    assert_eq!(paused["status"], 200);
    // Fail normal turn admission after the answer transaction commits, without
    // test hooks in production: an archived room cannot admit a new turn.
    conversations::Entity::update_many()
        .col_expr(conversations::Column::State, Expr::value("archived"))
        .filter(conversations::Column::Id.eq(row.conversation_id))
        .exec(&s.db)
        .await
        .unwrap();
    let failed = crate::platform_tools::dispatch(&s, admin(1), call("continue", "First answer"))
        .await
        .unwrap();
    assert_eq!(failed["status"], 409, "{failed}");
    conversations::Entity::update_many()
        .col_expr(conversations::Column::State, Expr::value("active"))
        .filter(conversations::Column::Id.eq(row.conversation_id))
        .exec(&s.db)
        .await
        .unwrap();
    let replaced =
        crate::platform_tools::dispatch(&s, admin(1), call("continue", "Conflicting answer"))
            .await
            .unwrap();
    assert_eq!(
        replaced["status"], 409,
        "a committed continuation must not be replaced: {replaced}"
    );
    let (first, again) = tokio::join!(
        crate::platform_tools::dispatch(&s, admin(1), call("continue", "First answer")),
        crate::platform_tools::dispatch(&s, admin(1), call("continue", "First answer")),
    );
    let (first, again) = (first.unwrap(), again.unwrap());
    assert_eq!(first["status"], 200, "{first}");
    assert_eq!(first["data"]["id"], again["data"]["id"], "{again}");
    assert_eq!(turns::Entity::find().count(&s.db).await.unwrap(), 2);
}

#[tokio::test]
async fn abandoning_a_question_stops_the_turn_without_locking_the_conversation() {
    let (_dir, s, row, owner) = enabled().await;
    append(
        &s,
        &row,
        &owner,
        Event::Session {
            native_session_id: "abandon-question-session".into(),
        },
    )
    .await
    .unwrap();
    platform_call(
        s.clone(),
        row.clone(),
        owner.clone(),
        "q".into(),
        "ferry_ask_user".into(),
        json!({"title":"Choose next step","question":"How should I proceed?"}),
        None,
    )
    .await;
    let call = crate::platform_tools::Call {
        method: "POST".into(),
        path: format!(
            "/conversations/{}/turns/{}/question-answers",
            row.conversation_id, row.id
        ),
        body: json!({"request_id":"q","action":"abandon"}),
        ..Default::default()
    };
    let response = crate::platform_tools::dispatch(&s, admin(1), call)
        .await
        .unwrap();
    assert_eq!(response["data"]["state"], "abandoned", "{response}");
    let stopped = turns::Entity::find_by_id(row.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stopped.state, "stopping");
    finish(&s, &row, &owner, Some(Outcome::Cancelled), "", true)
        .await
        .unwrap();
    sent(&s, row.conversation_id.into(), "new-task-after-abandon").await;
    assert_eq!(turns::Entity::find().count(&s.db).await.unwrap(), 2);
}

#[tokio::test]
async fn settling_outstanding_platform_work_keeps_its_lease_alive() {
    let (_dir, s, row, owner) = enabled().await;
    let original_expiry = row.expires_at;
    renewing(&s, &row, &owner, async {
        tokio::time::sleep(Duration::from_millis((LEASE / 3 + 1500) as u64)).await;
        let current = turns::Entity::find_by_id(row.id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert!(
            current.expires_at > original_expiry,
            "lease renewed during outstanding work"
        );
        Ok(())
    })
    .await
    .unwrap();
    finish(&s, &row, &owner, Some(Outcome::Cancelled), "", true)
        .await
        .unwrap();
}

#[tokio::test]
async fn parallel_approvals_survive_every_terminal_outcome_and_lease_loss() {
    for outcome in [
        Some(Outcome::Completed),
        Some(Outcome::Failed),
        Some(Outcome::Cancelled),
        Some(Outcome::TimedOut),
        None,
    ] {
        let (_dir, s, row, owner) = enabled().await;
        append(
            &s,
            &row,
            &owner,
            Event::Session {
                native_session_id: "approval-session".into(),
            },
        )
        .await
        .unwrap();
        let mut tasks = JoinSet::new();
        for id in ["parallel-a", "parallel-b"] {
            tasks.spawn(platform_call(s.clone(), row.clone(), owner.clone(), id.into(), "ferry_request".into(), json!({"method":"POST","path":"/node-groups","body":{"name":id,"parent_id":"0","position":0}}), None));
        }
        while let Some(result) = tasks.join_next().await {
            assert!(result.unwrap().1.is_null());
        }
        finish(&s, &row, &owner, outcome, "", true).await.unwrap();
        let ended = turns::Entity::find_by_id(row.id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let decisions: Value = serde_json::from_str(&ended.decisions).unwrap();
        for id in ["parallel-a", "parallel-b"] {
            assert_eq!(decisions[id]["ended_undecided"], true);
            assert!(decisions[id]["decision"].is_null());
        }
        assert_eq!(super::super::pending_in(&ended), 2);
        assert_eq!(host_groups::Entity::find().count(&s.db).await.unwrap(), 0);
        assert!(
            super::super::decide(
                State(s.clone()),
                admin(1),
                Path(("1".into(), row.conversation_id.into(), row.id.into())),
                Json(super::super::Decision {
                    request_id: "parallel-a".into(),
                    allow: true
                })
            )
            .await
            .is_err()
        );
    }
    let (_dir, s, row, owner) = enabled().await;
    platform_call(
        s.clone(),
        row.clone(),
        owner,
        "lease-call".into(),
        "ferry_request".into(),
        json!({"method":"POST","path":"/node-groups","body":{"name":"Never executed"}}),
        None,
    )
    .await;
    let mut current = turns::Entity::find_by_id(row.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    current.expires_at = db::now() - 1;
    current
        .into_active_model()
        .reset_all()
        .update(&s.db)
        .await
        .unwrap();
    assert!(claim(&s, row.id.into()).await.unwrap().is_none());
    let current = turns::Entity::find_by_id(row.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.state, "interrupted");
    assert_eq!(
        serde_json::from_str::<Value>(&current.decisions).unwrap()["lease-call"]["ended_undecided"],
        true
    );
    assert_eq!(host_groups::Entity::find().count(&s.db).await.unwrap(), 0);
    // Historical terminal turns predate the explicit marker. They can still
    // be continued without rewriting old records or requiring a migration.
    let mut historical = current;
    let mut decisions: Value = serde_json::from_str(&historical.decisions).unwrap();
    decisions["lease-call"]
        .as_object_mut()
        .unwrap()
        .remove("ended_undecided");
    historical.decisions = decisions.to_string();
    historical
        .into_active_model()
        .reset_all()
        .update(&s.db)
        .await
        .unwrap();
    super::super::resolve_decision(
        State(s.clone()),
        admin(1),
        Path(("1".into(), row.conversation_id.into(), row.id.into())),
        Json(super::super::Resolution {
            request_id: "lease-call".into(),
            action: "resumed".into(),
        }),
    )
    .await
    .unwrap();
    let resolved = turns::Entity::find_by_id(row.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(super::super::pending_in(&resolved), 0);
}

#[tokio::test]
async fn failed_platform_read_returns_specific_error_without_creating_a_pending_approval() {
    let (_dir, s, row, owner) = enabled().await;
    let result = platform_call(
        s.clone(),
        row.clone(),
        owner,
        "missing-read".into(),
        "ferry_request".into(),
        json!({"method":"GET","path":"/node-groups/999999"}),
        None,
    )
    .await
    .1;
    assert_eq!(result["ok"], false);
    assert_eq!(result["status"], 404);
    assert!(
        !result["data"]["error"].as_str().unwrap_or("").is_empty()
            || !result["error"].as_str().unwrap_or("").is_empty()
    );
    let listed = super::super::list(State(s), admin(1), Query(HashMap::new()))
        .await
        .unwrap()
        .0;
    assert_eq!(listed["items"][0]["pending"], 0);
}

async fn enabled() -> (tempfile::TempDir, AppState, turns::Model, String) {
    let (dir, s, id) = seeded().await;
    let c = conversations::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let mut revision = definition_revisions::Entity::find_by_id(c.revision_id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let mut definition: Value = serde_json::from_str(&revision.definition).unwrap();
    definition["platform_role_id"] = json!("1");
    revision.definition = definition.to_string();
    revision
        .into_active_model()
        .reset_all()
        .update(&s.db)
        .await
        .unwrap();
    let turn = sent(&s, id, "platform-test").await;
    let (row, _, owner) = claim(&s, turn).await.unwrap().unwrap();
    (dir, s, row, owner)
}

async fn decision(s: &AppState, row: &turns::Model, owner: &str, id: &str, allow: bool) -> Value {
    super::super::decide(
        State(s.clone()),
        admin(1),
        Path(("1".into(), row.conversation_id.into(), row.id.into())),
        Json(serde_json::from_value(json!({"request_id":id,"allow":allow})).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(heartbeat(s, row, owner).await.unwrap().1.len(), 1);
    let arguments = approved_platform_call(s, row, owner, id)
        .await
        .unwrap()
        .unwrap();
    platform_call(
        s.clone(),
        row.clone(),
        owner.into(),
        id.into(),
        "ferry_request".into(),
        arguments,
        Some(allow),
    )
    .await
    .1
}

#[tokio::test]
async fn platform_writes_wait_for_confirmation_hide_secrets_and_decline_closes_receipt() {
    let (_dir, s, row, owner) = enabled().await;
    let arguments = json!({"method":"POST","path":"/node-groups","body":{"name":"Confirmed group","parent_id":"0","position":0,"password":"never-public"}});
    let result = platform_call(
        s.clone(),
        row.clone(),
        owner.clone(),
        "write-1".into(),
        "ferry_request".into(),
        arguments.clone(),
        None,
    )
    .await
    .1;
    assert!(result.is_null());
    assert_eq!(host_groups::Entity::find().count(&s.db).await.unwrap(), 0);
    let current = turns::Entity::find_by_id(row.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let public = super::super::view_turn(&current).to_string();
    assert!(!public.contains("never-public"));
    assert!(!public.contains("_sealed"));
    let events = super::super::history(
        State(s.clone()),
        admin(1),
        Path(("1".into(), row.conversation_id.into())),
        Query(super::super::Cursor { after: 0 }),
    )
    .await
    .unwrap()
    .0
    .to_string();
    assert!(!events.contains("never-public"));
    assert!(!events.contains("_sealed"));
    let result = decision(&s, &row, &owner, "write-1", false).await;
    assert_eq!(result["ok"], false);
    assert_eq!(host_groups::Entity::find().count(&s.db).await.unwrap(), 0);
    assert!(
        approved_platform_call(&s, &row, &owner, "write-1")
            .await
            .is_err()
    );
    let duplicate = platform_call(
        s.clone(),
        row.clone(),
        owner.clone(),
        "write-1".into(),
        "ferry_request".into(),
        arguments,
        None,
    )
    .await
    .1;
    assert_eq!(duplicate["ok"], false);
}

#[tokio::test]
async fn platform_confirmation_rechecks_revoked_permissions_and_success_is_once() {
    let (_dir, s, row, owner) = enabled().await;
    let args = json!({"method":"POST","path":"/node-groups","body":{"name":"Agent group","parent_id":"0","position":0}});
    platform_call(
        s.clone(),
        row.clone(),
        owner.clone(),
        "allowed".into(),
        "ferry_request".into(),
        args.clone(),
        None,
    )
    .await;
    assert_eq!(
        decision(&s, &row, &owner, "allowed", true).await["status"],
        201
    );
    assert_eq!(host_groups::Entity::find().count(&s.db).await.unwrap(), 1);
    assert!(
        approved_platform_call(&s, &row, &owner, "allowed")
            .await
            .is_err()
    );
    platform_call(
        s.clone(),
        row.clone(),
        owner.clone(),
        "revoked".into(),
        "ferry_request".into(),
        args,
        None,
    )
    .await;
    let mut role = workspace_roles::Entity::find_by_id(1)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let mut grants: Vec<String> = serde_json::from_str(&role.permissions).unwrap();
    grants.retain(|p| p != "host-groups.create");
    role.permissions = json!(grants).to_string();
    role.into_active_model()
        .reset_all()
        .update(&s.db)
        .await
        .unwrap();
    let result = decision(&s, &row, &owner, "revoked", true).await;
    assert_eq!(result["status"], 403);
    assert_eq!(host_groups::Entity::find().count(&s.db).await.unwrap(), 1);
    assert!(
        approved_platform_call(&s, &row, &owner, "revoked")
            .await
            .is_err()
    );
}
