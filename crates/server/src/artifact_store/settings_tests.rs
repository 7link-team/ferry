//! Storage settings are proven when saved, and every failure names its reason.
use super::*;
use crate::agents::tests::setup;
use axum::http::StatusCode;

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
