use super::*;
use crate::migration::Migrator;
use sea_orm::{ConnectOptions, Database};
use sea_orm_migration::MigratorTrait;
use std::time::Instant;

struct Fixture {
    dir: tempfile::TempDir,
    db: DatabaseConnection,
    backend: ArtifactStore,
    vault: Vault,
    ids: Generator,
}
impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut url = format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("assets.sqlite").display()
        );
        if let Ok(root) = std::env::var("FERRY_TEST_JOURNAL_MYSQL_ROOT_URL") {
            let admin = Database::connect(&root).await.unwrap();
            let name = format!("workflow_journal_{}", uuid::Uuid::new_v4().simple());
            admin
                .execute_unprepared(&format!(
                    "CREATE DATABASE {name} CHARACTER SET utf8mb4 COLLATE utf8mb4_bin"
                ))
                .await
                .unwrap();
            admin.close().await.unwrap();
            url = format!("{}/{name}", root.trim_end_matches('/'));
            eprintln!("isolated content fixture: {name}");
        }
        let mut options = ConnectOptions::new(url);
        options.max_connections(4).sqlx_logging(false);
        let db = Database::connect(options).await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let backend = if std::env::var_os("FERRY_TEST_ASSET_S3").is_some() {
            ArtifactStore::configured(&dir.path().join("objects")).unwrap()
        } else {
            ArtifactStore::local(&dir.path().join("objects")).unwrap()
        };
        let vault = Vault::open(dir.path(), false).unwrap();
        let ids = Generator::new(1, Instant::now()).unwrap();
        Self {
            dir,
            db,
            backend,
            vault,
            ids,
        }
    }
    fn store(&self) -> Store<'_> {
        Store {
            db: &self.db,
            backend: &self.backend,
            vault: &self.vault,
            ids: &self.ids,
            quota: DEFAULT_QUOTA,
        }
    }
}

#[tokio::test]
async fn binary_bytes_roundtrip_encrypted_and_keep_text_packets_compatible() {
    let f = Fixture::new().await;
    let store = f.store();
    let bytes = [0, 255, 128, 13, 10, 42];
    let asset = store
        .stage_bytes(11, &bytes, "audio/wav", 0, "upload", 7)
        .await
        .unwrap();
    assert_eq!(asset.kind, "audio");
    assert_eq!(asset.created_by, 7);
    assert_eq!(asset.size, bytes.len() as i64);
    assert_eq!(asset.sha256, hex::encode(Sha256::digest(bytes)));
    assert_eq!(store.read_bytes(&f.db, 11, asset.id).await.unwrap(), bytes);
    assert!(store.read(&f.db, 11, asset.id).await.is_err());
    assert!(store.read_bytes(&f.db, 12, asset.id).await.is_err());
    let tx = f.db.begin().await.unwrap();
    store
        .attach(&tx, 11, asset.id, "workflow", 99)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let tx = f.db.begin().await.unwrap();
    store.pin(&tx, 11, asset.id, true).await.unwrap();
    tx.commit().await.unwrap();
    let text = store.stage(11, "{\"old\":true}", 8, "old").await.unwrap();
    assert_eq!(
        store.read(&f.db, 11, text.id).await.unwrap(),
        "{\"old\":true}"
    );
}

#[tokio::test]
async fn binary_pin_references_validate_tenant_metadata_and_block_gc_transactionally() {
    let f = Fixture::new().await;
    let store = f.store();
    let asset = store
        .stage(11, "{\"pinned\":true}", 8, "node")
        .await
        .unwrap();
    let reference = ferry_protocol::sandbox::Asset {
        asset: format!("sha256:{}", asset.sha256),
        mime: asset.mime.clone(),
        size: asset.size as u64,
        name: "sample.json".into(),
    };
    let tx = f.db.begin().await.unwrap();
    store
        .attach_binary(&tx, 11, &reference, "pin", 15)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(refs::Entity::find().count(&f.db).await.unwrap(), 0);
    let tx = f.db.begin().await.unwrap();
    assert!(
        store
            .attach_binary(&tx, 12, &reference, "pin", 15)
            .await
            .is_err()
    );
    let mut wrong = reference.clone();
    wrong.size += 1;
    assert!(
        store
            .attach_binary(&tx, 11, &wrong, "pin", 15)
            .await
            .is_err()
    );
    wrong = reference.clone();
    wrong.mime = "application/other".into();
    assert!(
        store
            .attach_binary(&tx, 11, &wrong, "pin", 15)
            .await
            .is_err()
    );
    store
        .attach_binary(&tx, 11, &reference, "pin", 15)
        .await
        .unwrap();
    store
        .attach_binary(&tx, 11, &reference, "pin", 15)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(refs::Entity::find().count(&f.db).await.unwrap(), 1);
    assert_eq!(jobs::Entity::find().count(&f.db).await.unwrap(), 0);
}

#[tokio::test]
async fn content_deduplicates_isolates_workspace_and_survives_restart() {
    let f = Fixture::new().await;
    let store = f.store();
    let plain = format!("{{\"secret\":\"{}\"}}", "private".repeat(20000));
    let a = store.stage(11, &plain, 8, "node").await.unwrap();
    let again = store.stage(11, &plain, 9, "other").await.unwrap();
    assert_eq!(a.id, again.id);
    assert_eq!(assets::Entity::find().count(&f.db).await.unwrap(), 1);
    let bytes = f
        .backend
        .get(&key(&a), plain.len() * 2 + 128)
        .await
        .unwrap();
    assert!(!std::str::from_utf8(&bytes).unwrap().contains("private"));
    let b = store.stage(12, &plain, 10, "node").await.unwrap();
    assert_ne!(key(&a), key(&b));
    assert!(store.read(&f.db, 12, a.id).await.is_err());
    let reopened = ArtifactStore::local(&f.dir.path().join("objects")).unwrap();
    let store = Store {
        backend: &reopened,
        ..f.store()
    };
    assert_eq!(store.read(&f.db, 11, a.id).await.unwrap(), plain);
    // Copying the encrypted bytes across workspace/asset identity must fail.
    f.backend.delete(&key(&b)).await.unwrap();
    f.backend.create(&key(&b), bytes).await.unwrap();
    assert!(store.read(&f.db, 12, b.id).await.is_err());
}

#[tokio::test]
async fn content_reference_rollback_grace_and_gc_generation() {
    let f = Fixture::new().await;
    let store = f.store();
    let a = store.stage(11, "{\"value\":1}", 8, "node").await.unwrap();
    let tx = f.db.begin().await.unwrap();
    store.attach(&tx, 11, a.id, "node_run", 31).await.unwrap();
    tx.rollback().await.unwrap();
    assert!(job(&f.db, 11, a.id).await.unwrap().is_some());
    assert!(
        !store
            .collect(11, a.id, a.created_at + GRACE_MS - 1)
            .await
            .unwrap()
    );
    let tx = f.db.begin().await.unwrap();
    store.attach(&tx, 11, a.id, "node_run", 31).await.unwrap();
    store.attach(&tx, 11, a.id, "node_run", 31).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(refs::Entity::find().count(&f.db).await.unwrap(), 1);
    assert!(!store.collect(11, a.id, i64::MAX).await.unwrap());
    let tx = f.db.begin().await.unwrap();
    store.detach(&tx, 11, a.id, "node_run", 31).await.unwrap();
    tx.commit().await.unwrap();
    let gc = job(&f.db, 11, a.id).await.unwrap().unwrap();
    assert!(store.collect(11, a.id, gc.eligible_at).await.unwrap());
    assert!(
        assets::Entity::find_by_id(a.id)
            .one(&f.db)
            .await
            .unwrap()
            .is_none()
    );
    let b = store.stage(11, "{\"value\":1}", 8, "node").await.unwrap();
    assert_ne!(key(&a), key(&b));
    // A late deletion of the old physical generation cannot affect the replacement.
    f.backend.delete(&key(&a)).await.unwrap();
    assert_eq!(store.read(&f.db, 11, b.id).await.unwrap(), "{\"value\":1}");
}

#[tokio::test]
async fn content_quota_counts_candidates_and_dedup_does_not_charge_twice() {
    let f = Fixture::new().await;
    let store = Store {
        quota: 7,
        ..f.store()
    };
    let a = store.stage(11, "{\"x\":1}", 8, "n").await.unwrap();
    assert_eq!(store.stage(11, "{\"x\":1}", 8, "n").await.unwrap().id, a.id);
    assert!(store.stage(11, "{\"x\":2}", 8, "n").await.is_err());
    assert_eq!(assets::Entity::find().count(&f.db).await.unwrap(), 1);
}

#[tokio::test]
async fn content_deleting_rejects_references_and_missing_or_corrupt_data() {
    let f = Fixture::new().await;
    let store = f.store();
    let a = store.stage(11, "{\"x\":1}", 8, "n").await.unwrap();
    jobs::Entity::update_many()
        .col_expr(jobs::Column::State, Expr::value("deleting"))
        .filter(jobs::Column::AssetId.eq(a.id))
        .exec(&f.db)
        .await
        .unwrap();
    let tx = f.db.begin().await.unwrap();
    assert!(store.attach(&tx, 11, a.id, "execution", 8).await.is_err());
    tx.rollback().await.unwrap();
    assert!(store.stage(11, "{\"x\":1}", 8, "n").await.is_err());
    assert!(store.read(&f.db, 11, a.id).await.is_err());
    assert!(store.collect(11, a.id, 0).await.unwrap()); // durable deleting resumes immediately
    let b = store.stage(11, "{\"x\":1}", 8, "n").await.unwrap();
    f.backend.delete(&key(&b)).await.unwrap();
    assert!(store.read(&f.db, 11, b.id).await.is_err());
    f.backend
        .create(&key(&b), bytes::Bytes::from_static(b"corrupt"))
        .await
        .unwrap();
    assert!(store.read(&f.db, 11, b.id).await.is_err());
}

#[tokio::test]
async fn content_competing_uploads_reserve_quota_once() {
    let f = Fixture::new().await;
    let a = f.store();
    let b = f.store();
    let (x, y) = tokio::join!(
        a.stage(11, "{\"x\":1}", 8, "a"),
        b.stage(11, "{\"x\":1}", 8, "b")
    );
    assert_eq!(x.unwrap().id, y.unwrap().id);
    assert_eq!(assets::Entity::find().count(&f.db).await.unwrap(), 1);
}

#[tokio::test]
async fn content_backend_create_and_limits_are_real() {
    let f = Fixture::new().await;
    assert!(
        f.backend
            .create("path/run/object", bytes::Bytes::from_static(b"first"))
            .await
            .unwrap()
    );
    assert!(
        !f.backend
            .create("path/run/object", bytes::Bytes::from_static(b"second"))
            .await
            .unwrap()
    );
    assert_eq!(
        &f.backend.get("path/run/object", 5).await.unwrap()[..],
        b"first"
    );
    assert!(f.backend.get("path/run/object", 4).await.is_err());
    for key in ["../escape", "/absolute", "a/../b", "a//b", "a\\b", ""] {
        assert!(f.backend.create(key, bytes::Bytes::new()).await.is_err());
    }
}

#[tokio::test]
async fn content_pin_and_reference_race_with_gc_cannot_lose_live_data() {
    let f = Fixture::new().await;
    let store = f.store();
    let a = store.stage(11, "{\"x\":1}", 8, "n").await.unwrap();
    let tx = f.db.begin().await.unwrap();
    store.pin(&tx, 11, a.id, true).await.unwrap();
    tx.commit().await.unwrap();
    assert!(!store.collect(11, a.id, i64::MAX).await.unwrap());
    let tx = f.db.begin().await.unwrap();
    store.pin(&tx, 11, a.id, false).await.unwrap();
    tx.commit().await.unwrap();
    let attach = async {
        let tx = f.db.begin().await.unwrap();
        let result = store.attach(&tx, 11, a.id, "execution", 8).await;
        if result.is_ok() {
            tx.commit().await.unwrap();
        } else {
            tx.rollback().await.unwrap();
        }
        result
    };
    let (attached, collected) = tokio::join!(attach, store.collect(11, a.id, i64::MAX));
    if attached.is_ok() {
        assert!(!collected.unwrap());
        assert!(store.read(&f.db, 11, a.id).await.is_ok());
    } else {
        assert!(collected.unwrap());
        assert_eq!(refs::Entity::find().count(&f.db).await.unwrap(), 0);
    }
}

#[cfg(windows)]
#[tokio::test]
async fn content_failed_object_delete_retains_metadata_and_retries_after_restart() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new().await;
    let store = f.store();
    let a = store.stage(11, "{\"x\":1}", 8, "n").await.unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(f.dir.path().join("objects").join(key(&a)))
        .unwrap();
    assert!(store.collect(11, a.id, i64::MAX).await.is_err());
    let failed = job(&f.db, 11, a.id).await.unwrap().unwrap();
    assert_eq!(failed.state, "deleting");
    assert!(failed.error.is_some());
    assert!(
        assets::Entity::find_by_id(a.id)
            .one(&f.db)
            .await
            .unwrap()
            .is_some()
    );
    drop(held);
    let backend = ArtifactStore::local(&f.dir.path().join("objects")).unwrap();
    let resumed = Store {
        backend: &backend,
        ..f.store()
    };
    assert!(resumed.collect(11, a.id, 0).await.unwrap());
    assert!(
        assets::Entity::find_by_id(a.id)
            .one(&f.db)
            .await
            .unwrap()
            .is_none()
    );
}
