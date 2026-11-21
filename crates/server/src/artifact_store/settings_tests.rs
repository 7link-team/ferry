//! Storage settings are proven when saved, and every failure names its reason.
use super::*;
use crate::agents::tests::setup;
use axum::http::StatusCode;

#[tokio::test]
async fn configured_store_replaces_stale_cache_and_rejects_missing_objects() {
    let store = ArtifactStore(Arc::new(object_store::memory::InMemory::new()));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cached");
    std::fs::write(&path, b"stale").unwrap();
    store
        .create("object", Bytes::from_static(b"configured"))
        .await
        .unwrap();
    store.download_file("object", &path).await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"configured");
    assert!(store.download_file("missing", &path).await.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"configured");
}

#[tokio::test]
async fn local_same_path_streaming_preserves_large_files() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArtifactStore::local(dir.path()).unwrap();
    let path = dir.path().join("object");
    let bytes: Vec<u8> = (0..12 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &bytes).unwrap();
    store.upload_file("object", &path).await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    store.download_file("object", &path).await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[tokio::test]
async fn business_reads_use_configured_s3_for_cold_and_existing_cache() {
    use axum::{body::Body, response::IntoResponse};
    let (_dir, state) = setup().await;
    let metadata = serde_json::json!({"id":"42","url":"/api/workspaces/1/value-files/42","name":"fixture.txt","size":7,"content_type":"text/plain","resource":"settings"});
    let objects = Arc::new(std::sync::RwLock::new(std::collections::HashMap::from([
        (
            "bucket/prefix/artifacts/9/10".to_owned(),
            Bytes::from_static(b"artifact"),
        ),
        (
            "bucket/prefix/value-files/1/42/metadata.json".to_owned(),
            Bytes::from(metadata.to_string()),
        ),
    ])));
    let served = objects.clone();
    // A protocol-level S3 GET fixture; this is not a real S3/MinIO service.
    let router = axum::Router::new().route(
        "/{*key}",
        axum::routing::get(
            move |axum::extract::Path(key): axum::extract::Path<String>| {
                let objects = served.clone();
                async move {
                    match objects.read().unwrap().get(&key).cloned() {
                        Some(bytes) => (
                            [
                                ("content-length", bytes.len().to_string()),
                                ("last-modified", "Wed, 21 Oct 2015 07:28:00 GMT".into()),
                                ("etag", "\"fixture\"".into()),
                            ],
                            Body::from(bytes),
                        )
                            .into_response(),
                        None => StatusCode::NOT_FOUND.into_response(),
                    }
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let config = Configuration {
        kind: "s3".into(),
        endpoint,
        bucket: "bucket".into(),
        prefix: "prefix".into(),
        region: "test".into(),
        access_key: "fixture".into(),
        secret_key: "fixture".into(),
    };
    crate::entities::platform_state::Entity::update_many()
        .col_expr(
            crate::entities::platform_state::Column::FileStorage,
            sea_orm::sea_query::Expr::value(sealed(&state, &config)),
        )
        .exec(&state.db)
        .await
        .unwrap();
    let row = crate::entities::pipeline_artifacts::Model {
        id: 10,
        workspace_id: 1,
        pipeline_id: 8,
        run_id: 9,
        path: "fixture.txt".into(),
        sha256: String::new(),
        size: 8,
        mode: 0,
        created_at: 0,
        expires_at: 0,
    };
    let artifact_path = crate::pipelines::artifacts::file_path(&state, &row);
    assert!(!artifact_path.exists());
    let fetched = crate::pipelines::artifacts::available_path(&state, &row)
        .await
        .unwrap();
    assert_eq!(std::fs::read(fetched).unwrap(), b"artifact");
    let value = serde_json::json!([metadata]);
    crate::maintenance::files::validate_references(
        &state,
        &state.db,
        1.into(),
        "settings",
        &value,
        &serde_json::json!({}),
    )
    .await
    .unwrap();
    assert!(
        state
            .data_dir
            .join("value-files/1/42/metadata.json")
            .exists()
    );
    // Cached local data cannot mask a missing configured object.
    objects.write().unwrap().clear();
    assert!(
        crate::pipelines::artifacts::available_path(&state, &row)
            .await
            .is_err()
    );
    assert!(
        crate::maintenance::files::validate_references(
            &state,
            &state.db,
            1.into(),
            "settings",
            &value,
            &serde_json::json!({})
        )
        .await
        .is_err()
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires NATS_SERVER pointing to a local nats-server executable"]
async fn nats_default_and_saved_local_storage_work_across_shared_directory_states() {
    struct Server(std::process::Child);
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let nats_dir = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(std::env::var("NATS_SERVER").unwrap());
    command
        .args(["-js", "-a", "127.0.0.1", "-p", &port.to_string(), "-sd"])
        .arg(nats_dir.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let _server = Server(command.spawn().unwrap());
    let (_dir, state) = crate::platform_tools::tests::setup().await;
    let mut state = Arc::try_unwrap(state).ok().expect("unshared fixture");
    let url = format!("nats://127.0.0.1:{port}");
    let cluster = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if let Ok(cluster) =
                crate::cluster::Cluster::connect(&url, "storage_test", 1, state.db.clone()).await
            {
                break cluster;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    state.cluster = Some(cluster.clone());
    assert_eq!(ArtifactStore::check(&state).await.unwrap()["ok"], true);
    let first = ArtifactStore::for_state(&state).await.unwrap();
    let previous = sealed(&state, &s3("old"));
    let saved = save(
        &state,
        Some(&previous),
        serde_json::json!({"storage_kind":"local"}),
    )
    .await
    .unwrap()
    .unwrap();
    let state = Arc::new(state);
    let graph = serde_json::json!({"engine":"ferry-graph","schema":1,"settings":{"timezone":"UTC"},"nodes":[
        {"id":"start","name":"Start","type":"core.trigger.manual","type_version":1,"params":{"input_schema":{"type":"object"}},"position":[0,0]}
    ],"connections":[]});
    let (_, crate::error::Json(workflow)) = crate::workflows::create(
        axum::extract::State(state.clone()), crate::agents::tests::admin(1),
        crate::error::Json(serde_json::from_value(serde_json::json!({"name":"Local storage regression","definition":graph.to_string()})).unwrap()),
    ).await.unwrap();
    let workflow_id = serde_json::from_value(workflow["id"].clone()).unwrap();
    let (_, crate::error::Json(execution)) = crate::workflows::run::create(
        axum::extract::State(state.clone()), crate::agents::tests::admin(1),
        crate::error::Path(("1".into(), workflow_id)),
        crate::error::Json(serde_json::from_value(serde_json::json!({"mode":"manual","revision":"draft","version":1,"entry_node_id":"start"})).unwrap()),
    ).await.unwrap();
    let execution_id: crate::snowflake::Id =
        serde_json::from_value(execution["id"].clone()).unwrap();
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let (_drain, draining) = tokio::sync::watch::channel(false);
    let scheduler = tokio::spawn(crate::workflows::run::scheduler(
        state.clone(),
        stopped,
        draining,
    ));
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let run =
                crate::entities::workflow_executions::Entity::find_by_id(i64::from(execution_id))
                    .one(&state.db)
                    .await
                    .unwrap()
                    .unwrap();
            if run.state == "succeeded" {
                break;
            }
            assert_ne!(run.state, "failed");
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    stop.send(true).unwrap();
    scheduler.await.unwrap();
    crate::entities::platform_state::Entity::update_many()
        .col_expr(
            crate::entities::platform_state::Column::FileStorage,
            sea_orm::sea_query::Expr::value(saved),
        )
        .exec(&state.db)
        .await
        .unwrap();
    first
        .create("shared/object", Bytes::from_static(b"shared"))
        .await
        .unwrap();
    let (_other_dir, other) = setup().await;
    let mut other = Arc::try_unwrap(other).ok().expect("unshared fixture");
    other.data_dir = state.data_dir.clone();
    other.cluster = Some(cluster);
    let second = ArtifactStore::for_state(&other).await.unwrap();
    assert_eq!(second.get("shared/object", 100).await.unwrap(), "shared");
    assert!(ArtifactStore::for_state(&state).await.is_ok());
}

fn key(error: &crate::error::ApiError) -> String {
    error.2.as_ref().map(|m| m.key.clone()).unwrap_or_default()
}
fn sealed(s: &crate::state::State, config: &Configuration) -> String {
    s.vault
        .seal(
            &serde_json::to_string(config).unwrap(),
            "platform:file-storage",
        )
        .unwrap()
}
fn s3(bucket: &str) -> Configuration {
    Configuration {
        kind: "s3".into(),
        endpoint: "https://s3.example.invalid".into(),
        bucket: bucket.into(),
        region: "r".into(),
        ..Default::default()
    }
}
async fn save(
    s: &crate::state::State,
    previous: Option<&str>,
    fields: serde_json::Value,
) -> crate::error::Result<Option<String>> {
    Configuration::update(s, &s.db, previous, fields.as_object().unwrap()).await
}

#[tokio::test]
async fn local_storage_is_written_read_and_deleted_before_it_is_saved() {
    let (_dir, s) = setup().await;
    let previous = sealed(&s, &s3("bucket"));
    // A file where the check directory belongs makes the local directory unusable.
    std::fs::write(s.data_dir.join("storage-check"), "blocks the directory").unwrap();
    let error = save(
        &s,
        Some(&previous),
        serde_json::json!({"storage_kind":"local"}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert_eq!(key(&error), "platform.storageLocalUnwritable");
    let shown = error.render(crate::i18n::Locale::ZhCn);
    assert!(shown.contains(&s.data_dir.display().to_string()), "{shown}");
    std::fs::remove_file(s.data_dir.join("storage-check")).unwrap();
    let saved = save(
        &s,
        Some(&previous),
        serde_json::json!({"storage_kind":"local"}),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(Configuration::read(&s, Some(&saved)).unwrap().kind, "local");
    // The check leaves nothing behind.
    assert!(
        std::fs::read_dir(s.data_dir.join("storage-check"))
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true)
    );
}

#[tokio::test]
async fn s3_settings_are_rejected_with_the_precise_reason() {
    let (_dir, s) = setup().await;
    let cases = [
        (
            serde_json::json!({"storage_kind":"s3","storage_endpoint":"ftp://files.example","storage_bucket":"b","storage_region":"r"}),
            "platform.storageEndpointInvalid",
        ),
        (
            serde_json::json!({"storage_kind":"s3","storage_endpoint":"https://user:pw@s3.example","storage_bucket":"b","storage_region":"r"}),
            "platform.storageEndpointInvalid",
        ),
        (
            serde_json::json!({"storage_kind":"s3","storage_endpoint":"https://s3.example","storage_region":"r"}),
            "platform.storageBucketRequired",
        ),
        (
            serde_json::json!({"storage_kind":"s3","storage_endpoint":"https://s3.example","storage_bucket":"b"}),
            "platform.storageBucketRequired",
        ),
        (
            serde_json::json!({"storage_kind":"s3","storage_endpoint":"https://s3.example","storage_bucket":7,"storage_region":"r"}),
            "platform.storageFieldInvalid",
        ),
    ];
    for (fields, expected) in cases {
        let error = save(&s, None, fields.clone()).await.unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_REQUEST, "{fields}");
        assert_eq!(key(&error), expected, "{fields}");
    }
}

#[tokio::test]
async fn an_unreachable_s3_endpoint_is_named_without_waiting_for_retries() {
    let (_dir, s) = setup().await;
    let started = std::time::Instant::now();
    // Nothing listens on the discard port: connection refused, then retries.
    let error = save(
        &s,
        None,
        serde_json::json!({"storage_kind":"s3","storage_endpoint":"http://127.0.0.1:9","storage_bucket":"b","storage_region":"r"}),
    )
    .await
    .unwrap_err();
    assert_eq!(key(&error), "platform.storageUnreachable");
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}

#[test]
fn object_store_failures_are_classified() {
    let classify_text = |text: &str| classify("write", &anyhow::anyhow!(text.to_owned()), None);
    let problem = |text: &str| classify_text(text).message.key;
    assert_eq!(
        problem("The specified bucket does not exist (NoSuchBucket)"),
        "platform.storageBucketMissing"
    );
    assert_eq!(
        problem("AccessDenied: Access Denied"),
        "platform.storageAccessDenied"
    );
    assert_eq!(
        problem("SignatureDoesNotMatch"),
        "platform.storageAccessDenied"
    );
    assert_eq!(
        problem("error sending request: dns error"),
        "platform.storageUnreachable"
    );
    assert_eq!(
        problem("something else entirely"),
        "platform.storageCheckFailed"
    );
    // Causes decide over wording: a refused connection whose text says nothing useful.
    let refused = anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::ConnectionRefused))
        .context("Generic S3 error: request failed");
    assert_eq!(
        classify("write", &refused, None).message.key,
        "platform.storageUnreachable"
    );
    // A status-like number inside a URL's object key is not a status.
    assert_eq!(
        problem("Generic S3 error for http://127.0.0.1:9/b/storage-check/4031-401a"),
        "platform.storageCheckFailed"
    );
    let denied = anyhow::Error::from(object_store::Error::PermissionDenied {
        path: "x".into(),
        source: "no".into(),
    });
    assert_eq!(
        classify("write", &denied, None).message.key,
        "platform.storageAccessDenied"
    );
    let missing = anyhow::Error::from(object_store::Error::NotFound {
        path: "x".into(),
        source: "no".into(),
    });
    assert_eq!(
        classify("write", &missing, None).message.key,
        "platform.storageBucketMissing"
    );
    // Local storage always names its directory.
    let local = classify("write", &missing, Some(FsPath::new("/data/ferry")));
    assert_eq!(local.message.key, "platform.storageLocalUnwritable");
    assert_eq!(
        local.message.params["path"],
        FsPath::new("/data/ferry").display().to_string()
    );
}

#[tokio::test]
async fn administrators_see_the_live_storage_status_with_its_reason() {
    let (_dir, s) = setup().await;
    assert_eq!(
        ArtifactStore::check(&s).await.unwrap(),
        serde_json::json!({"ok": true})
    );
    // Settings sealed under another key cannot be decrypted any more.
    let other = s
        .vault
        .seal(
            &serde_json::to_string(&s3("bucket")).unwrap(),
            "another-purpose",
        )
        .unwrap();
    crate::entities::platform_state::Entity::update_many()
        .col_expr(
            crate::entities::platform_state::Column::FileStorage,
            sea_orm::sea_query::Expr::value(Some(other.clone())),
        )
        .exec(&s.db)
        .await
        .unwrap();
    let status = ArtifactStore::check(&s).await.unwrap();
    assert_eq!(status["ok"], false);
    assert_eq!(status["reason_key"], "platform.storageConfigUnreadable");
    assert!(!status["reason"].as_str().unwrap().is_empty());
    // Runtime users get the storage message, never internal wording.
    let error = ArtifactStore::for_state(&s).await.err().unwrap();
    assert_eq!(error.0, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(key(&error), "storage.unavailable");
    // The unreadable settings can be replaced from the settings page.
    let saved = save(
        &s,
        Some(&other),
        serde_json::json!({"storage_kind":"local"}),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(Configuration::read(&s, Some(&saved)).unwrap().kind, "local");
}
