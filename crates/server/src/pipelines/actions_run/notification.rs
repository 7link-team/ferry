//! The built-in notification action uses the existing private worker response channel.
use super::events::Preview;
use super::*;
use crate::{
    accounts::workspace::Scope,
    definitions::access::{self, Operation},
};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    id: String,
    response: String,
    channel_id: Id,
    message: crate::notifications::center::SendBody,
}

pub(super) fn request(line: &[u8], bridge: &Preview<'_>) -> Option<Request> {
    let value: Value = serde_json::from_slice(line).ok()?;
    let text = value["raw"].as_str().or_else(|| value["msg"].as_str())?;
    let prefix = format!("::ferry-notify::{}::", bridge.nonce);
    let text = text.strip_prefix(&prefix)?.trim_end();
    if text.len() > 64 * 1024 {
        return None;
    }
    let request: Request = serde_json::from_str(text).ok()?;
    (events::preview_path(bridge.dir, &request.id)? == request.response).then_some(request)
}

async fn send(
    s: &AppState,
    run: &runs::Model,
    job: &Job,
    bridge: &Preview<'_>,
    mut request: Request,
) -> Result<Value> {
    let scope = Scope::for_actor(s, run.workspace_id.into(), run.triggered_by.into()).await?;
    let origin = history::open_variables(s, run)?.origin_agent_id;
    let future = async {
        let scope = Scope::for_actor(s, scope.workspace_id, scope.user.id).await?;
        scope.require("pipelines.run")?;
        access::require_pipeline(&s.db, &scope, run.pipeline_id.into(), Operation::Run).await?;
        let current = runs::Entity::find_by_id(run.id)
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        if current.state != "running" {
            return Err(ApiError::forbidden());
        }
        // Nonce is invocation-specific; repeated log chunks return the same durable delivery.
        use sha2::{Digest, Sha256};
        request.message.client_id = format!(
            "pipeline:{}:{:x}",
            run.id,
            Sha256::digest(format!("{}:{}:{}", job.key, bridge.nonce, request.id))
        );
        crate::notifications::center::send_message(
            s,
            &scope,
            request.channel_id,
            request.message,
            "pipeline",
            &run.id.to_string(),
        )
        .await
    };
    match origin {
        Some(agent) => {
            crate::platform_tools::delegated_future(s, scope.clone(), agent, future).await
        }
        None => future.await,
    }
}

pub(super) async fn serve(
    s: &AppState,
    run: &runs::Model,
    job: &Job,
    ch: &dyn HostChannel,
    bridge: &Preview<'_>,
    request: Request,
) {
    let response = request.response.clone();
    let result = match send(s, run, job, bridge, request).await {
        Ok(value) if value["state"] == "succeeded" => {
            json!({"delivery_id":value["id"],"state":"succeeded"})
        }
        Ok(value) => {
            json!({"error":crate::i18n::message(if value["state"] == "unknown" {"notification.unknown"} else {"notification.sendFailed"})})
        }
        Err(error) => json!({"error":error.render(crate::i18n::current_locale())}),
    };
    events::answer_preview(ch, &response, result).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agents::tests::admin,
        error::Json,
        platform_tools::tests::{agent, role, setup},
    };
    use axum::extract::State;
    use sea_orm::{IntoActiveModel, sea_query::Expr};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn payload(channel: Id) -> Value {
        json!({"id":"notify-abc","response":"/private/notify-abc.json","channel_id":channel,
            "message":{"client_id":"ignored","body":"Build report"}})
    }

    #[test]
    fn marker_requires_this_invocations_nonce_and_private_response_path() {
        let bridge = Preview {
            nonce: "nonce",
            dir: "/private",
            host_id: 1,
        };
        let body = payload(12.into());
        let line = |nonce: &str, body: &Value| {
            serde_json::to_vec(&json!({"raw":format!("::ferry-notify::{nonce}::{body}\n")}))
                .unwrap()
        };
        assert!(request(&line("nonce", &body), &bridge).is_some());
        assert!(request(&line("forged", &body), &bridge).is_none());
        let mut body = body;
        body["response"] = json!("/private/../escape.json");
        assert!(request(&line("nonce", &body), &bridge).is_none());
    }

    #[tokio::test]
    async fn pipeline_notifications_are_idempotent_scoped_and_agent_revocation_safe() {
        let (_dir, s) = setup().await;
        let count = Arc::new(AtomicUsize::new(0));
        let observed = count.clone();
        let app = axum::Router::new().route(
            "/",
            axum::routing::post(move || {
                let count = observed.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    "{}"
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let receiver = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let role_id = role(
            &s,
            1,
            vec![
                "pipelines.read",
                "pipelines.run",
                "notifications.read",
                "notifications.send",
            ],
        )
        .await;
        let agent = agent(&s, role_id).await;
        let (_, Json(channel)) = crate::notifications::center::create(
            State(s.clone()),
            admin(1),
            Json(
                serde_json::from_value(
                    json!({"name":"Pipeline fixture","provider":"Webhook","webhook":url}),
                )
                .unwrap(),
            ),
        )
        .await
        .unwrap();
        let channel_id: Id = serde_json::from_value(channel["id"].clone()).unwrap();
        crate::entities::pipelines::Entity::insert(
            crate::entities::pipelines::Model {
                id: 1,
                workspace_id: 1,
                owner_id: Some(1),
                name: "Fixture".into(),
                group_id: 0,
                application_id: 0,
                repository_id: 0,
                definition: String::new(),
                target_kind: "host".into(),
                target_ref: 1,
                description: String::new(),
                created_at: 0,
                updated_at: 0,
                version: 1,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
        let mut run = history::lease_tests::run(7);
        run.workspace_id = 1;
        run.state = "running".into();
        let mut snapshot = history::VariableSnapshot::default();
        snapshot.origin_agent_id = Some(agent.id.into());
        run.variable_snapshot = Some(
            s.vault
                .seal(
                    &serde_json::to_string(&snapshot).unwrap(),
                    &crate::vault::aad("1", "pipeline-run-variables", "7"),
                )
                .unwrap(),
        );
        runs::Entity::insert(run.clone().into_active_model())
            .exec(&s.db)
            .await
            .unwrap();
        let job = super::super::tests::test_job("build", &[]);
        let bridge = Preview {
            nonce: "nonce",
            dir: "/private",
            host_id: 1,
        };
        let request = || serde_json::from_value(payload(channel_id)).unwrap();
        let result = send(&s, &run, &job, &bridge, request()).await.unwrap();
        assert_eq!(result["state"], "succeeded");
        assert_eq!(result["agent_id"], json!(Id::from(agent.id)));
        let again = send(&s, &run, &job, &bridge, request()).await.unwrap();
        assert_eq!(result["id"], again["id"]);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        crate::entities::workspace_roles::Entity::update_many()
            .col_expr(
                crate::entities::workspace_roles::Column::Permissions,
                Expr::value(
                    json!(["pipelines.read", "pipelines.run", "notifications.read"]).to_string(),
                ),
            )
            .filter(crate::entities::workspace_roles::Column::Id.eq(role_id))
            .exec(&s.db)
            .await
            .unwrap();
        assert_eq!(
            send(&s, &run, &job, &bridge, request())
                .await
                .unwrap_err()
                .0,
            StatusCode::FORBIDDEN
        );
        run.workspace_id = 2;
        assert!(send(&s, &run, &job, &bridge, request()).await.is_err());
        assert_eq!(count.load(Ordering::SeqCst), 1);
        receiver.abort();
    }
}
