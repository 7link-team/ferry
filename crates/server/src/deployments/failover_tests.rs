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
