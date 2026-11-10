use super::*;
use crate::agents::tests::{admin, setup};
use sea_orm::PaginatorTrait;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test]
async fn sms_configuration_is_encrypted_scoped_and_preview_uses_approved_template() {
    let (_dir, s) = setup().await;
    for provider in ["Aliyun SMS", "Tencent SMS"] {
        let scope = admin(1);
        let body = || {
            serde_json::from_value::<ChannelBody>(json!({"name":"SMS fixture","provider":provider,"sms_access_key":"fixtureAccess","sms_secret_key":"fixtureSecret","sms_region":"ap-guangzhou","sms_app_id":"1400000000","sms_sign_name":"Fixture","sms_template_code":"123456","sms_parameter_names":["code"],"recipients":["+8613800000000"]})).unwrap()
        };
        let (_, Json(created)) = create(State(s.clone()), scope.clone(), Json(body()))
            .await
            .unwrap();
        let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
        let Json(view) = get(State(s.clone()), scope.clone(), Path(("1".into(), id)))
            .await
            .unwrap();
        assert_eq!(view["has_sms_secret"], true);
        assert!(!view.to_string().contains("fixtureSecret"));
        let row = channel(&s, scope.workspace_id, id).await.unwrap();
        assert!(!row.config.contains("fixtureSecret"));
        let mut changed = body();
        changed.version = 1;
        changed.config.sms.sms_secret_key.clear();
        update(
            State(s.clone()),
            scope.clone(),
            Path(("1".into(), id)),
            Json(changed),
        )
        .await
        .unwrap();
        assert_eq!(
            decode(&s, &channel(&s, scope.workspace_id, id).await.unwrap())
                .unwrap()
                .sms
                .sms_secret_key,
            "fixtureSecret"
        );
        let request = || {
            serde_json::from_value::<SendBody>(
                json!({"client_id":"sms-fixture","variables":{"code":"123456"}}),
            )
            .unwrap()
        };
        let Json(previewed) = preview(
            State(s.clone()),
            scope.clone(),
            Path(("1".into(), id)),
            Json(request()),
        )
        .await
        .unwrap();
        assert_eq!(previewed["parameters"]["code"], "123456");
        assert_eq!(previewed["sms_template_code"], "123456");
        assert_eq!(previewed["to"], json!(["+8613800000000"]));
        assert_eq!(
            send_message(&s, &admin(2), id, request(), "manual", "")
                .await
                .unwrap_err()
                .0,
            StatusCode::NOT_FOUND
        );
        let mut forbidden = scope.clone();
        forbidden.user.is_superadmin = false;
        forbidden.permissions.clear();
        assert_eq!(
            send_message(&s, &forbidden, id, request(), "manual", "")
                .await
                .unwrap_err()
                .0,
            StatusCode::FORBIDDEN
        );
        let mut invalid = request();
        invalid.to = vec!["not-a-phone".into()];
        assert_eq!(
            send_message(&s, &scope, id, invalid, "manual", "")
                .await
                .unwrap_err()
                .0,
            StatusCode::BAD_REQUEST
        );
        let mut missing = request();
        missing.variables.clear();
        assert_eq!(
            send_message(&s, &scope, id, missing, "manual", "")
                .await
                .unwrap_err()
                .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(deliveries::Entity::find().count(&s.db).await.unwrap(), 0);
    }
}

fn channel_body(url: &str) -> ChannelBody {
    serde_json::from_value(json!({"name":"Fixture", "provider":"Webhook", "webhook":url, "bearer_token":"private-fixture"})).unwrap()
}
fn request(key: &str) -> SendBody {
    serde_json::from_value(json!({"client_id":key,"subject":"Fixture", "body":"Body\nSecond line"}))
        .unwrap()
}

async fn receiver(status: StatusCode) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let app = axum::Router::new().route(
        "/notify",
        axum::routing::post(move |Json(body): Json<Value>| {
            let observed = observed.clone();
            async move {
                assert!(body["body"].as_str().is_some());
                observed.fetch_add(1, Ordering::SeqCst);
                (status, "{}")
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/notify", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, count, task)
}

#[tokio::test]
async fn channel_crud_is_scoped_versioned_and_secret_preserving() {
    let (_dir, s) = setup().await;
    let scope = admin(1);
    let (_, Json(created)) = create(
        State(s.clone()),
        scope.clone(),
        Json(channel_body("http://127.0.0.1:9/notify")),
    )
    .await
    .unwrap();
    let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
    let Json(view) = get(State(s.clone()), scope.clone(), Path(("1".into(), id)))
        .await
        .unwrap();
    assert!(view["has_webhook"].as_bool().unwrap());
    assert!(!view.to_string().contains("private-fixture"));
    assert!(view.get("webhook").is_none());
    assert_eq!(
        get(State(s.clone()), admin(2), Path(("2".into(), id)))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
    let mut changed = channel_body("");
    changed.name = "Changed".into();
    changed.version = 1;
    let Json(updated) = update(
        State(s.clone()),
        scope.clone(),
        Path(("1".into(), id)),
        Json(changed),
    )
    .await
    .unwrap();
    assert_eq!(updated["name"], "Changed");
    assert_eq!(updated["version"], 2);
    let stored = channel(&s, 1.into(), id).await.unwrap();
    assert!(!stored.config.contains("private-fixture"));
    assert_eq!(
        decode(&s, &stored).unwrap().options.bearer_token,
        "private-fixture"
    );
    let mut stale = channel_body("http://127.0.0.1:9/notify");
    stale.version = 1;
    assert_eq!(
        update(
            State(s.clone()),
            scope.clone(),
            Path(("1".into(), id)),
            Json(stale)
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::CONFLICT
    );
    let mut readonly = scope.clone();
    readonly.permissions.retain(|p| p == "notifications.read");
    assert_eq!(
        create(
            State(s.clone()),
            readonly,
            Json(channel_body("http://127.0.0.1:9/notify"))
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::FORBIDDEN
    );
    delete(State(s.clone()), scope, Path(("1".into(), id)))
        .await
        .unwrap();
    assert!(channel(&s, 1.into(), id).await.is_err());
}

#[tokio::test]
async fn delivery_requires_send_permission_and_deduplicates_concurrent_retries() {
    let (_dir, s) = setup().await;
    let scope = admin(1);
    let (url, count, task) = receiver(StatusCode::OK).await;
    let (_, Json(created)) = create(State(s.clone()), scope.clone(), Json(channel_body(&url)))
        .await
        .unwrap();
    let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
    let mut readonly = scope.clone();
    readonly.permissions.retain(|p| p == "notifications.read");
    assert_eq!(
        send_message(&s, &readonly, id, request("unauthorized"), "manual", "")
            .await
            .unwrap_err()
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send_message(&s, &admin(2), id, request("cross-space"), "manual", "")
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    let (a, b) = tokio::join!(
        send_message(&s, &scope, id, request("same"), "manual", ""),
        send_message(&s, &scope, id, request("same"), "manual", "")
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(a["id"], b["id"]);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let replay = send_message(&s, &scope, id, request("same"), "manual", "")
        .await
        .unwrap();
    assert_eq!(replay["state"], "succeeded");
    let mut different = request("same");
    different.body = "Different".into();
    assert_eq!(
        send_message(&s, &scope, id, different, "manual", "")
            .await
            .unwrap_err()
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(deliveries::Entity::find().count(&s.db).await.unwrap(), 1);
    let Json(page) = list_deliveries(State(s.clone()), scope.clone(), Query(HashMap::new()))
        .await
        .unwrap();
    assert!(page["items"][0].get("body").is_none());
    let record_id: Id = serde_json::from_value(a["id"].clone()).unwrap();
    assert_eq!(
        get_delivery(State(s.clone()), admin(2), Path(("2".into(), record_id)))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
    task.abort();
}

#[tokio::test]
async fn templates_escape_variables_and_old_retries_survive_template_deletion() {
    let (_dir, s) = setup().await;
    let scope = admin(1);
    let body = TemplateBody {
        name: "HTML".into(),
        language: "zh-CN".into(),
        subject: "Hello {{recipient_email}}".into(),
        body: "<p>{{content}}</p>".into(),
        format: "html".into(),
        version: 0,
    };
    let (_, Json(t)) = create_template(State(s.clone()), scope.clone(), Json(body))
        .await
        .unwrap();
    let template_id: Id = serde_json::from_value(t["id"].clone()).unwrap();
    let (url, count, task) = receiver(StatusCode::OK).await;
    let (_, Json(c)) = create(State(s.clone()), scope.clone(), Json(channel_body(&url)))
        .await
        .unwrap();
    let id: Id = serde_json::from_value(c["id"].clone()).unwrap();
    let mut body = request("template");
    body.template_id = Some(template_id);
    body.variables
        .insert("recipient_email".into(), "recipient@example.invalid".into());
    body.variables
        .insert("content".into(), "<script>bad</script>".into());
    let result = send_message(&s, &scope, id, body.clone(), "manual", "")
        .await
        .unwrap();
    assert_eq!(result["body"], "<p>&lt;script&gt;bad&lt;/script&gt;</p>");
    delete_template(
        State(s.clone()),
        scope.clone(),
        Path(("1".into(), template_id)),
    )
    .await
    .unwrap();
    let again = send_message(&s, &scope, id, body, "manual", "")
        .await
        .unwrap();
    assert_eq!(result["id"], again["id"]);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test]
async fn restart_does_not_mark_another_nodes_live_send_unknown() {
    let (_dir, s) = setup().await;
    let scope = admin(1);
    let (url, count, task) = receiver(StatusCode::OK).await;
    let (_, Json(c)) = create(State(s.clone()), scope.clone(), Json(channel_body(&url)))
        .await
        .unwrap();
    let id: Id = serde_json::from_value(c["id"].clone()).unwrap();
    let result = send_message(&s, &scope, id, request("once"), "manual", "")
        .await
        .unwrap();
    let delivery_id: Id = serde_json::from_value(result["id"].clone()).unwrap();
    deliveries::Entity::update_many()
        .col_expr(deliveries::Column::State, Expr::value("sending"))
        .exec(&s.db)
        .await
        .unwrap();
    prepare(&s).await.unwrap();
    assert_eq!(
        deliveries::Entity::find_by_id(delivery_id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .state,
        "sending"
    );
    deliveries::Entity::update_many()
        .col_expr(
            deliveries::Column::CreatedAt,
            Expr::value(db::now() - 61_000),
        )
        .exec(&s.db)
        .await
        .unwrap();
    prepare(&s).await.unwrap();
    let again = send_message(&s, &scope, id, request("once"), "manual", "")
        .await
        .unwrap();
    assert_eq!(again["state"], "unknown");
    assert_eq!(count.load(Ordering::SeqCst), 1);
    task.abort();
}

#[test]
fn generic_message_preview_keeps_body_without_an_empty_label() {
    let body = "Line one\nLine two";
    let notice = super::super::webhook::Notice {
        title: "Subject",
        accent: None,
        field_links: vec![],
        field_accents: vec![],
        fields: vec![(String::new(), body.into())],
        generic: json!({}),
    };
    let result = super::super::webhook::preview("WeCom", &notice).unwrap();
    let markdown = result["payload"]["markdown"]["content"].as_str().unwrap();
    assert!(markdown.contains(body));
    assert!(!markdown.contains("："));
}

#[tokio::test]
async fn smtp_delivers_html_and_plain_text_through_the_shared_service() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (_dir, s) = setup().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (captured, message) = tokio::sync::oneshot::channel();
    let receiver = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (read, mut write) = socket.into_split();
        let mut read = BufReader::new(read);
        write
            .write_all(b"220 localhost SMTP fixture\r\n")
            .await
            .unwrap();
        let mut data = String::new();
        let mut in_data = false;
        loop {
            let mut line = String::new();
            if read.read_line(&mut line).await.unwrap() == 0 {
                break;
            }
            if in_data {
                if line == ".\r\n" {
                    in_data = false;
                    write.write_all(b"250 accepted\r\n").await.unwrap();
                } else {
                    data.push_str(&line);
                }
            } else if line.starts_with("EHLO") {
                write
                    .write_all(b"250-localhost\r\n250 8BITMIME\r\n")
                    .await
                    .unwrap();
            } else if line.starts_with("DATA") {
                in_data = true;
                write.write_all(b"354 send message\r\n").await.unwrap();
            } else if line.starts_with("QUIT") {
                write.write_all(b"221 bye\r\n").await.unwrap();
                break;
            } else {
                write.write_all(b"250 ok\r\n").await.unwrap();
            }
        }
        let _ = captured.send(data);
    });
    let (_, Json(created)) = create(State(s.clone()), admin(1), Json(serde_json::from_value(json!({
        "name":"SMTP fixture","provider":"SMTP","host":"127.0.0.1","port":port,"security":"none",
        "from":"sender@example.invalid","recipients":["recipient@example.invalid"]
    })).unwrap())).await.unwrap();
    let id = serde_json::from_value(created["id"].clone()).unwrap();
    let mut body = request("smtp");
    body.format = "html".into();
    body.body = "<h1>Acceptance</h1><p>Message body</p>".into();
    let sent = send_message(&s, &admin(1), id, body, "manual", "")
        .await
        .unwrap();
    assert_eq!(sent["state"], "succeeded", "{sent}");
    let data = tokio::time::timeout(Duration::from_secs(3), message)
        .await
        .unwrap()
        .unwrap();
    assert!(data.contains("multipart/alternative"), "{data}");
    assert!(data.contains("text/html"));
    assert!(data.contains("text/plain"));
    assert!(data.contains("recipient@example.invalid"));
    assert!(data.contains("Acceptance"));
    receiver.await.unwrap();
}

#[tokio::test]
async fn legacy_channel_import_is_idempotent_and_does_not_change_poll_state() {
    use crate::entities::{repository_monitors as monitors, repository_poll_states as polls};
    let (_dir, s) = setup().await;
    let original = json!({"enabled":true,"provider":"github","mode":"poll","all_repositories":true,"repositories":[],"events":["push"],"secret":"incoming-secret","webhook":"http://127.0.0.1:9/legacy","notification_language":"ja-JP"});
    let aad = "repository-monitor:1:80";
    monitors::Entity::insert(
        monitors::Model {
            id: 80,
            workspace_id: 1,
            config: s.vault.seal(&original.to_string(), aad).unwrap(),
            version: 7,
            created_at: 123,
            updated_at: 456,
        }
        .into_active_model(),
    )
    .exec(&s.db)
    .await
    .unwrap();
    let poll = polls::Model {
        id: 80,
        workspace_id: 1,
        config_version: 7,
        snapshot: "encrypted-baseline".into(),
        last_polled_at: 111,
        next_poll_at: 222,
        last_error: "old error".into(),
    };
    polls::Entity::insert(poll.clone().into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    crate::repositories::monitor::prepare_channels(&s)
        .await
        .unwrap();
    crate::repositories::monitor::prepare_channels(&s)
        .await
        .unwrap();
    assert_eq!(channels::Entity::find().count(&s.db).await.unwrap(), 1);
    assert_eq!(
        polls::Entity::find_by_id(80)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        poll
    );
    let updated = monitors::Entity::find_by_id(80)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (updated.version, updated.created_at, updated.updated_at),
        (7, 123, 456)
    );
    let config: Value =
        serde_json::from_str(&s.vault.open_secret(&updated.config, aad).unwrap()).unwrap();
    for key in [
        "events",
        "secret",
        "webhook",
        "notification_language",
        "repositories",
        "all_repositories",
    ] {
        assert_eq!(config[key], original[key], "{key}");
    }
    let id: Id = serde_json::from_value(config["notification_channel_ids"][0].clone()).unwrap();
    let row = channel(&s, 1.into(), id).await.unwrap();
    assert_eq!(decode(&s, &row).unwrap().language, "ja-JP");
    assert_eq!(
        delete(State(s.clone()), admin(1), Path(("1".into(), id)))
            .await
            .unwrap_err()
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        deliveries::Entity::find().count(&s.db).await.unwrap(),
        0,
        "migration does not send or replay notifications"
    );
}

#[tokio::test]
async fn disabled_channels_and_explicit_provider_failure_are_not_replayed() {
    let (_dir, s) = setup().await;
    let scope = admin(1);
    let (url, count, task) = receiver(StatusCode::BAD_REQUEST).await;
    let (_, Json(c)) = create(State(s.clone()), scope.clone(), Json(channel_body(&url)))
        .await
        .unwrap();
    let id: Id = serde_json::from_value(c["id"].clone()).unwrap();
    let result = send_message(&s, &scope, id, request("rejected"), "manual", "")
        .await
        .unwrap();
    assert_eq!(result["state"], "failed");
    let again = send_message(&s, &scope, id, request("rejected"), "manual", "")
        .await
        .unwrap();
    assert_eq!(again["id"], result["id"]);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    channels::Entity::update_many()
        .col_expr(channels::Column::Enabled, Expr::value(0_i64))
        .exec(&s.db)
        .await
        .unwrap();
    assert_eq!(
        send_message(&s, &scope, id, request("disabled"), "manual", "")
            .await
            .unwrap_err()
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    task.abort();
}
