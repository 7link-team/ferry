//! Versioned, bounded-memory file transport using the existing vault AEAD.
//! The immutable manifest is published first, so every child is discoverable by
//! the existing GC candidate even if a writer dies before uploading its body.
use super::*;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tempfile::NamedTempFile;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

const MAGIC: &[u8] = b"FERRY-ASSET-CHUNKS-2\n";
const DATA_MAGIC: &[u8] = b"FERRY-ASSET-DATA-2\n";
const CHUNK: usize = 64 * 1024;
const MAX_FRAME: usize = CHUNK * 2 + 128;
const MAX_MANIFEST: usize = 8192;
const SEGMENT_CHUNKS: u64 = 31;
const SEGMENT_LIMIT: usize = 4 * 1024 * 1024;
// Vault v1: v1. + 48 hex nonce bytes + . + 32 hex tag bytes.
const SEALED_OVERHEAD: u64 = 84;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u8,
    child: String,
    sha256: String,
    length: u64,
    chunks: u64,
    segment_chunks: u64,
    segments: u64,
    encoded_length: u64,
}
impl Manifest {
    fn new(asset: &assets::Model) -> Result<Self> {
        let length = u64::try_from(asset.size).map_err(|_| fault())?;
        let count = chunks(length);
        Ok(Self {
            version: 2,
            child: format!("{}.data-{}", key(asset), uuid::Uuid::new_v4()),
            sha256: asset.sha256.clone(),
            length,
            chunks: count,
            segment_chunks: SEGMENT_CHUNKS,
            segments: count.div_ceil(SEGMENT_CHUNKS).max(1),
            encoded_length: encoded_length(length, count)?,
        })
    }
}
fn encoded_length(length: u64, count: u64) -> Result<u64> {
    length
        .checked_mul(2)
        .and_then(|n| {
            count
                .checked_mul(5 + SEALED_OVERHEAD)
                .and_then(|frames| n.checked_add(frames))
        })
        .and_then(|n| n.checked_add(DATA_MAGIC.len() as u64 + 5 + SEALED_OVERHEAD + 32))
        .ok_or_else(fault)
}
fn segment_key(manifest: &Manifest, index: u64) -> String {
    format!("{}.part-{index:016x}", manifest.child)
}
fn segment_length(manifest: &Manifest, index: u64) -> Result<usize> {
    if index >= manifest.segments {
        return Err(fault());
    }
    let first = index.checked_mul(SEGMENT_CHUNKS).ok_or_else(fault)?;
    let count = (manifest.chunks - first).min(SEGMENT_CHUNKS);
    let start = first
        .checked_mul(CHUNK as u64)
        .ok_or_else(fault)?
        .min(manifest.length);
    let plain = (manifest.length - start).min(SEGMENT_CHUNKS * CHUNK as u64);
    let length = plain
        .checked_mul(2)
        .and_then(|n| n.checked_add(count * (5 + SEALED_OVERHEAD)))
        .and_then(|n| {
            n.checked_add(if index == 0 {
                DATA_MAGIC.len() as u64
            } else {
                0
            })
        })
        .and_then(|n| {
            n.checked_add(if index + 1 == manifest.segments {
                5 + SEALED_OVERHEAD + 32
            } else {
                0
            })
        })
        .ok_or_else(fault)?;
    let length = usize::try_from(length).map_err(|_| fault())?;
    if length > SEGMENT_LIMIT {
        return Err(fault());
    }
    Ok(length)
}
fn completion_record(manifest: &Manifest) -> Vec<u8> {
    [
        manifest.encoded_length.to_be_bytes(),
        manifest.segments.to_be_bytes(),
        manifest.segment_chunks.to_be_bytes(),
    ]
    .concat()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_assets::tests::Fixture;

    #[derive(Debug, Default)]
    struct AtomicOnly(
        object_store::memory::InMemory,
        std::sync::Mutex<Option<(object_store::path::Path, bytes::Bytes)>>,
        Option<std::sync::Arc<MetadataIoGate>>,
    );
    #[derive(Debug)]
    struct MetadataIoGate {
        // 1 = GET, 2 = PUT; consume the arm once, after fixture setup.
        armed: std::sync::atomic::AtomicU8,
        entered: tokio::sync::Notify,
        release: tokio::sync::Semaphore,
    }
    impl MetadataIoGate {
        async fn wait(&self, operation: u8) {
            if self
                .armed
                .compare_exchange(
                    operation,
                    0,
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                )
                .is_ok()
            {
                self.entered.notify_one();
                self.release.acquire().await.unwrap().forget();
            }
        }
    }
    impl std::fmt::Display for AtomicOnly {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "atomic-only fixture")
        }
    }
    #[async_trait::async_trait]
    impl object_store::ObjectStore for AtomicOnly {
        async fn put_opts(
            &self,
            p: &object_store::path::Path,
            b: object_store::PutPayload,
            o: object_store::PutOptions,
        ) -> object_store::Result<object_store::PutResult> {
            if let Some(gate) = &self.2 {
                gate.wait(2).await;
            }
            assert!(matches!(o.mode, object_store::PutMode::Create));
            assert!(b.content_length() <= SEGMENT_LIMIT);
            let winner = {
                let mut pending = self.1.lock().unwrap();
                if pending.as_ref().is_some_and(|(path, _)| path == p) {
                    pending.take()
                } else {
                    None
                }
            };
            if let Some((_, bytes)) = winner {
                self.0
                    .put_opts(
                        p,
                        bytes.into(),
                        object_store::PutOptions {
                            mode: object_store::PutMode::Create,
                            ..Default::default()
                        },
                    )
                    .await?;
            }
            self.0.put_opts(p, b, o).await
        }
        async fn put_multipart_opts(
            &self,
            _: &object_store::path::Path,
            _: object_store::PutMultipartOptions,
        ) -> object_store::Result<Box<dyn object_store::MultipartUpload>> {
            panic!("content upload attempted multipart")
        }
        async fn get_opts(
            &self,
            p: &object_store::path::Path,
            o: object_store::GetOptions,
        ) -> object_store::Result<object_store::GetResult> {
            if let Some(gate) = &self.2 {
                gate.wait(1).await;
            }
            self.0.get_opts(p, o).await
        }
        async fn delete(&self, p: &object_store::path::Path) -> object_store::Result<()> {
            self.0.delete(p).await
        }
        fn list(
            &self,
            _: Option<&object_store::path::Path>,
        ) -> futures::stream::BoxStream<'static, object_store::Result<object_store::ObjectMeta>>
        {
            panic!("content GC attempted LIST")
        }
        async fn list_with_delimiter(
            &self,
            _: Option<&object_store::path::Path>,
        ) -> object_store::Result<object_store::ListResult> {
            panic!("content GC attempted LIST")
        }
        async fn copy(
            &self,
            _: &object_store::path::Path,
            _: &object_store::path::Path,
        ) -> object_store::Result<()> {
            unreachable!()
        }
        async fn copy_if_not_exists(
            &self,
            _: &object_store::path::Path,
            _: &object_store::path::Path,
        ) -> object_store::Result<()> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn late_canonical_create_winner_uses_its_own_child_identity() {
        let fixture = Fixture::new().await;
        let atomic = std::sync::Arc::new(AtomicOnly::default());
        let backend = ArtifactStore::from_backend_for_test(atomic.clone());
        let store = Store {
            backend: &backend,
            ..fixture.store()
        };
        let bytes = vec![59; CHUNK + 19];
        let asset = store
            .reserve_asset(
                11,
                hex::encode(Sha256::digest(&bytes)),
                bytes.len() as i64,
                "application/octet-stream",
                0,
                "late-winner",
                7,
            )
            .await
            .unwrap();
        let winner = Manifest::new(&asset).unwrap();
        let mut main = MAGIC.to_vec();
        main.extend_from_slice(
            store
                .vault
                .seal_bytes(&serde_json::to_vec(&winner).unwrap(), &manifest_aad(&asset))
                .unwrap()
                .as_bytes(),
        );
        *atomic.1.lock().unwrap() = Some((
            object_store::path::Path::parse(key(&asset)).unwrap(),
            main.into(),
        ));
        let stored = store
            .stage_bytes(11, &bytes, "application/octet-stream", 0, "contender", 7)
            .await
            .unwrap();
        assert_eq!(stored.id, asset.id);
        assert_eq!(manifest(&store, &asset).await.child, winner.child);
        assert_eq!(
            store.read_bytes(store.db, 11, asset.id).await.unwrap(),
            bytes
        );
    }

    #[tokio::test]
    async fn bounded_immutable_segments_need_neither_multipart_nor_listing() {
        let fixture = Fixture::new().await;
        let backend =
            ArtifactStore::from_backend_for_test(std::sync::Arc::new(AtomicOnly::default()));
        let store = Store {
            backend: &backend,
            ..fixture.store()
        };
        for length in [0, CHUNK * 31, CHUNK * 31 + 17, CHUNK * 80 + 7] {
            let bytes = vec![87; length];
            let asset = store
                .stage_bytes(11, &bytes, "application/octet-stream", 0, "segments", 7)
                .await
                .unwrap();
            let m = manifest(&store, &asset).await;
            assert_eq!(
                m.encoded_length,
                (0..m.segments)
                    .map(|i| segment_length(&m, i).unwrap() as u64)
                    .sum::<u64>()
            );
            assert_eq!(
                store.read_bytes(store.db, 11, asset.id).await.unwrap(),
                bytes
            );
            let eligible = job(store.db, 11, asset.id)
                .await
                .unwrap()
                .unwrap()
                .eligible_at;
            assert!(store.collect(11, asset.id, eligible + 1).await.unwrap());
            for index in 0..m.segments {
                assert!(!backend.exists(&segment_key(&m, index)).await.unwrap());
            }
            assert!(!backend.exists(&m.child).await.unwrap());
        }
    }

    #[tokio::test]
    async fn partial_segments_resume_immutably_and_layout_corruption_fails_closed() {
        let fixture = Fixture::new().await;
        let store = fixture.store();
        let bytes = vec![83; CHUNK * 31 + 9];
        let asset = store
            .reserve_asset(
                11,
                hex::encode(Sha256::digest(&bytes)),
                bytes.len() as i64,
                "application/octet-stream",
                0,
                "partial",
                7,
            )
            .await
            .unwrap();
        let m = Manifest::new(&asset).unwrap();
        let plain = temporary().unwrap();
        tokio::fs::write(plain.path(), &bytes).await.unwrap();
        let encoded = store.encode(&asset, &m, plain.path()).await.unwrap();
        let mut main = MAGIC.to_vec();
        main.extend_from_slice(
            store
                .vault
                .seal_bytes(&serde_json::to_vec(&m).unwrap(), &manifest_aad(&asset))
                .unwrap()
                .as_bytes(),
        );
        store
            .backend
            .create(&key(&asset), main.into())
            .await
            .unwrap();
        let mut input = tokio::fs::File::open(encoded.path()).await.unwrap();
        let mut first = vec![0; segment_length(&m, 0).unwrap()];
        input.read_exact(&mut first).await.unwrap();
        store
            .backend
            .create(&segment_key(&m, 0), first.clone().into())
            .await
            .unwrap();
        assert!(!store.complete(&asset, &m).await.unwrap());
        store
            .stage_bytes(11, &bytes, "application/octet-stream", 0, "resume", 7)
            .await
            .unwrap();
        assert_eq!(
            store
                .backend
                .get(&segment_key(&m, 0), SEGMENT_LIMIT)
                .await
                .unwrap()
                .as_ref(),
            first
        );
        assert_eq!(
            store.read_bytes(store.db, 11, asset.id).await.unwrap(),
            bytes
        );
        let mut broken = m.clone();
        broken.segments = u64::MAX;
        let mut main = MAGIC.to_vec();
        main.extend_from_slice(
            store
                .vault
                .seal_bytes(&serde_json::to_vec(&broken).unwrap(), &manifest_aad(&asset))
                .unwrap()
                .as_bytes(),
        );
        replace(&store, &key(&asset), &main).await;
        assert!(store.read_file(store.db, 11, asset.id).await.is_err());
        let mut huge = asset.clone();
        huge.size = i64::MAX;
        assert!(Manifest::new(&huge).is_err());
        assert_eq!(
            store
                .vault
                .seal_bytes(&[], "format-overhead")
                .unwrap()
                .len() as u64,
            SEALED_OVERHEAD
        );
    }

    async fn replace(store: &Store<'_>, object: &str, bytes: &[u8]) {
        let temp = temporary().unwrap();
        tokio::fs::write(temp.path(), bytes).await.unwrap();
        store
            .backend
            .upload_file(object, temp.path())
            .await
            .unwrap();
    }
    async fn manifest(store: &Store<'_>, asset: &assets::Model) -> Manifest {
        match store.format(asset).await.unwrap() {
            Format::Chunked(m) => m,
            _ => panic!("chunked object"),
        }
    }

    #[tokio::test]
    async fn streaming_roundtrip_dedup_and_gc_preserve_the_immutable_manifest() {
        let fixture = Fixture::new().await;
        let store = fixture.store();
        let bytes = vec![91; CHUNK * 3 + 17];
        let asset = store
            .stage_bytes(11, &bytes, "application/octet-stream", 0, "upload", 7)
            .await
            .unwrap();
        let main = store.backend.get(&key(&asset), MAX_MANIFEST).await.unwrap();
        assert!(main.starts_with(MAGIC));
        let m = manifest(&store, &asset).await;
        let cipher = store
            .backend
            .get(&segment_key(&m, 0), MAX_FRAME * 5)
            .await
            .unwrap();
        assert!(!cipher.windows(32).any(|window| window == &bytes[..32]));
        let file = store.read_file(store.db, 11, asset.id).await.unwrap();
        assert_eq!(tokio::fs::read(file.path()).await.unwrap(), bytes);
        let duplicate = store
            .stage_bytes(11, &bytes, "application/octet-stream", 0, "again", 7)
            .await
            .unwrap();
        assert_eq!(duplicate.id, asset.id);
        assert_eq!(
            store.backend.get(&key(&asset), MAX_MANIFEST).await.unwrap(),
            main
        );
        assert_eq!(
            store
                .backend
                .get(&segment_key(&m, 0), MAX_FRAME * 5)
                .await
                .unwrap(),
            cipher
        );
        let eligible = job(store.db, 11, asset.id)
            .await
            .unwrap()
            .unwrap()
            .eligible_at;
        assert!(store.collect(11, asset.id, eligible + 1).await.unwrap());
        assert!(!store.backend.exists(&m.child).await.unwrap());
        assert!(!store.backend.exists(&key(&asset)).await.unwrap());
    }

    #[tokio::test]
    async fn legacy_single_aead_objects_remain_readable_and_are_never_rewritten() {
        let fixture = Fixture::new().await;
        let store = fixture.store();
        let bytes = b"legacy-content-fixture";
        let asset = store
            .reserve_asset(
                11,
                hex::encode(Sha256::digest(bytes)),
                bytes.len() as i64,
                "text/plain",
                0,
                "legacy",
                7,
            )
            .await
            .unwrap();
        let cipher = store.vault.seal_bytes(bytes, &aad(&asset)).unwrap();
        store
            .backend
            .create(&key(&asset), cipher.clone().into())
            .await
            .unwrap();
        assert_eq!(
            store.read_bytes(store.db, 11, asset.id).await.unwrap(),
            bytes
        );
        assert_eq!(
            store
                .stage_bytes(11, bytes, "text/plain", 0, "new", 7)
                .await
                .unwrap()
                .id,
            asset.id
        );
        assert_eq!(
            store
                .backend
                .get(&key(&asset), 4096)
                .await
                .unwrap()
                .as_ref(),
            cipher.as_bytes()
        );
    }

    #[tokio::test]
    async fn manifest_first_missing_child_recovers_only_an_unreferenced_staged_asset() {
        let fixture = Fixture::new().await;
        let store = fixture.store();
        let bytes = vec![42; CHUNK + 9];
        let asset = store
            .reserve_asset(
                11,
                hex::encode(Sha256::digest(&bytes)),
                bytes.len() as i64,
                "application/octet-stream",
                0,
                "crash",
                7,
            )
            .await
            .unwrap();
        let m = Manifest::new(&asset).unwrap();
        let mut main = MAGIC.to_vec();
        main.extend_from_slice(
            store
                .vault
                .seal_bytes(&serde_json::to_vec(&m).unwrap(), &manifest_aad(&asset))
                .unwrap()
                .as_bytes(),
        );
        store
            .backend
            .create(&key(&asset), main.clone().into())
            .await
            .unwrap();
        assert!(!store.backend.exists(&m.child).await.unwrap());
        assert_eq!(
            store
                .stage_bytes(11, &bytes, "application/octet-stream", 0, "retry", 7)
                .await
                .unwrap()
                .id,
            asset.id
        );
        assert_eq!(
            store
                .backend
                .get(&key(&asset), MAX_MANIFEST)
                .await
                .unwrap()
                .as_ref(),
            main
        );
        assert_eq!(
            store.read_bytes(store.db, 11, asset.id).await.unwrap(),
            bytes
        );
        let tx = store.db.begin().await.unwrap();
        store
            .attach(&tx, 11, asset.id, "workflow", 7)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        store.backend.delete(&m.child).await.unwrap();
        assert!(
            store
                .stage_bytes(
                    11,
                    &bytes,
                    "application/octet-stream",
                    0,
                    "published-missing",
                    7
                )
                .await
                .is_err()
        );
        assert!(!store.backend.exists(&m.child).await.unwrap());
    }

    #[tokio::test]
    async fn authentication_rejects_tampering_reordering_truncation_extra_bytes_and_wrong_final_counts()
     {
        let fixture = Fixture::new().await;
        let store = fixture.store();
        let bytes = vec![51; CHUNK * 2 + 7];
        let asset = store
            .stage_bytes(11, &bytes, "application/octet-stream", 0, "integrity", 7)
            .await
            .unwrap();
        let m = manifest(&store, &asset).await;
        let original = store
            .backend
            .get(&segment_key(&m, 0), MAX_FRAME * 5)
            .await
            .unwrap()
            .to_vec();
        let mut frames = Vec::new();
        let mut at = DATA_MAGIC.len();
        while at < original.len() {
            let length = u32::from_be_bytes(original[at + 1..at + 5].try_into().unwrap()) as usize;
            frames.push(original[at..at + 5 + length].to_vec());
            at += 5 + length;
        }
        let mut tampered = original.clone();
        tampered[DATA_MAGIC.len() + 12] ^= 1;
        let mut reordered = DATA_MAGIC.to_vec();
        for i in [1, 0, 2, 3] {
            reordered.extend_from_slice(&frames[i]);
        }
        let mut extra = original.clone();
        extra.push(0);
        let missing_final = original[..original.len() - frames.last().unwrap().len()].to_vec();
        let mut wrong_final = original[..original.len() - frames.last().unwrap().len()].to_vec();
        let mut counts = Vec::new();
        counts.extend_from_slice(&(m.chunks + 1).to_be_bytes());
        counts.extend_from_slice(&m.length.to_be_bytes());
        let sealed = store
            .vault
            .seal_bytes(&counts, &frame_aad(&asset, &m, m.chunks, "final"))
            .unwrap();
        wrong_final.push(1);
        wrong_final.extend_from_slice(&(sealed.len() as u32).to_be_bytes());
        wrong_final.extend_from_slice(sealed.as_bytes());
        for corrupted in [
            tampered,
            reordered,
            extra,
            missing_final,
            original[..original.len() - 1].to_vec(),
            wrong_final,
        ] {
            replace(&store, &segment_key(&m, 0), &corrupted).await;
            assert!(store.read_file(store.db, 11, asset.id).await.is_err());
            assert!(
                store
                    .stage_bytes(11, &bytes, "application/octet-stream", 0, "no-repair", 7)
                    .await
                    .is_err()
            );
            assert_eq!(
                store
                    .backend
                    .get(&segment_key(&m, 0), MAX_FRAME * 5)
                    .await
                    .unwrap()
                    .as_ref(),
                corrupted
            );
        }
        replace(&store, &segment_key(&m, 0), &original).await;
        assert_eq!(
            store.read_bytes(store.db, 11, asset.id).await.unwrap(),
            bytes
        );
    }

    #[tokio::test]
    async fn slow_object_io_does_not_hold_the_metadata_writer() {
        use std::time::Duration;
        let fixture = Fixture::new().await;
        let gate = std::sync::Arc::new(MetadataIoGate {
            armed: std::sync::atomic::AtomicU8::new(0),
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Semaphore::new(0),
        });
        let backend = ArtifactStore::from_backend_for_test(std::sync::Arc::new(AtomicOnly(
            Default::default(),
            Default::default(),
            Some(gate.clone()),
        )));
        let store = Store {
            backend: &backend,
            ..fixture.store()
        };
        let bytes = vec![77; CHUNK + 19];
        let mut staged = None;
        // Block a real upload, then a read while reusing that same stored asset.
        // I/O cannot finish until every metadata write has committed.
        for operation in [2, 1] {
            gate.armed
                .store(operation, std::sync::atomic::Ordering::SeqCst);
            let work = store.stage_bytes(11, &bytes, "application/octet-stream", 0, "slow", 7);
            let (asset, writes) = tokio::join!(work, async {
                tokio::time::timeout(Duration::from_secs(5), gate.entered.notified())
                    .await
                    .expect("staging must reach the object I/O gate");
                let writes = tokio::time::timeout(Duration::from_secs(5), async {
                    for index in 0..10 {
                        let tx = store.db.begin().await.unwrap();
                        assert!(
                            crate::lock::acquire(
                                &tx,
                                "unrelated-metadata-writer",
                                &index.to_string(),
                                crate::db::now(),
                                1000
                            )
                            .await
                            .unwrap()
                        );
                        crate::lock::release(&tx, "unrelated-metadata-writer", &index.to_string())
                            .await
                            .unwrap();
                        tx.commit().await.unwrap();
                    }
                    10
                })
                .await;
                gate.release.add_permits(1);
                writes
            });
            assert_eq!(writes.expect("metadata writer blocked by object I/O"), 10);
            staged = Some(asset.unwrap());
        }
        let asset = staged.unwrap();
        gate.armed.store(1, std::sync::atomic::Ordering::SeqCst);
        let preparing = store.prepare(11, asset.id);
        tokio::pin!(preparing);
        let (receipt, ()) = tokio::join!(&mut preparing, async {
            tokio::time::timeout(Duration::from_secs(5), gate.entered.notified())
                .await
                .expect("attachment verification must reach the object I/O gate");
            let write = tokio::time::timeout(Duration::from_secs(5), async {
                let tx = store.db.begin().await.unwrap();
                assert!(
                    crate::lock::acquire(&tx, "another-writer", "writer", crate::db::now(), 1000)
                        .await
                        .unwrap()
                );
                tx.commit().await.unwrap();
            })
            .await;
            gate.release.add_permits(1);
            write.expect("metadata writer blocked by attachment verification");
        });
        let receipt = receipt.unwrap();
        let tx = store.db.begin().await.unwrap();
        receipt.attach(&store, &tx, "workflow", 7).await.unwrap();
        tx.commit().await.unwrap();
    }

    #[tokio::test]
    async fn expired_receipt_cannot_renew_attach_or_release_a_successor() {
        let fixture = Fixture::new().await;
        let store = fixture.store();
        let asset = store
            .stage_bytes(11, b"fenced", "text/plain", 0, "lease", 7)
            .await
            .unwrap();
        let receipt = store.prepare(11, asset.id).await.unwrap();
        crate::entities::locks::Entity::update_many()
            .col_expr(
                crate::entities::locks::Column::ExpiresAt,
                Expr::value(crate::db::now() - 1),
            )
            .filter(crate::entities::locks::Column::Name.eq(&receipt.lease.name))
            .exec(store.db)
            .await
            .unwrap();
        assert!(receipt.lease.ensure(store.db).await.is_err());
        let tx = store.db.begin().await.unwrap();
        assert!(receipt.attach(&store, &tx, "workflow", 7).await.is_err());
        tx.rollback().await.unwrap();
        assert!(
            crate::lock::acquire(
                store.db,
                &receipt.lease.name,
                "successor",
                crate::db::now(),
                LEASE_TTL
            )
            .await
            .unwrap()
        );
        receipt.lease.release(store.db).await.unwrap();
        assert_eq!(
            crate::entities::locks::Entity::find_by_id(&receipt.lease.name)
                .one(store.db)
                .await
                .unwrap()
                .unwrap()
                .owner,
            "successor"
        );
        assert_eq!(refs::Entity::find().count(store.db).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn cancelled_input_leaves_no_asset_or_object_generation() {
        let fixture = Fixture::new().await;
        let store = fixture.store();
        let (mut sender, reader) = tokio::io::duplex(CHUNK * 2);
        sender.write_all(&vec![1; CHUNK]).await.unwrap();
        {
            let work = store.stage_reader(11, reader, "application/octet-stream", 0, "cancel", 7);
            tokio::pin!(work);
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(100), &mut work)
                    .await
                    .is_err()
            );
        }
        assert!(sender.write_all(&[1]).await.is_err());
        assert_eq!(assets::Entity::find().count(store.db).await.unwrap(), 0);
        assert_eq!(jobs::Entity::find().count(store.db).await.unwrap(), 0);
    }
}
enum Format {
    Missing,
    Legacy,
    Chunked(Manifest),
}
const LEASE_TTL: i64 = 60_000;

struct AssetLease {
    db: DatabaseConnection,
    name: String,
    owner: String,
    lost: tokio::sync::watch::Receiver<bool>,
    renewal: tokio::task::JoinHandle<()>,
}
impl AssetLease {
    async fn acquire(db: &DatabaseConnection, asset: &assets::Model) -> Result<Self> {
        let name = format!("content-asset:{}:{}", asset.workspace_id, asset.id);
        let owner = uuid::Uuid::new_v4().to_string();
        loop {
            let tx = db.begin().await?;
            if crate::lock::acquire_live(&tx, &name, &owner, LEASE_TTL).await? {
                tx.commit().await?;
                break;
            }
            tx.rollback().await?;
            // Match the previous transaction lock's serialized dedup behavior,
            // while leaving the SQL writer available during long object I/O.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let (signal, lost) = tokio::sync::watch::channel(false);
        let (renew_db, renew_name, renew_owner) = (db.clone(), name.clone(), owner.clone());
        let renewal = tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(10));
            tick.tick().await;
            loop {
                tick.tick().await;
                if !matches!(
                    crate::lock::renew_live(&renew_db, &renew_name, &renew_owner, LEASE_TTL).await,
                    Ok(1)
                ) {
                    signal.send_replace(true);
                    break;
                }
            }
        });
        Ok(Self {
            db: db.clone(),
            name,
            owner,
            lost,
            renewal,
        })
    }
    async fn ensure(&self, db: &impl ConnectionTrait) -> Result<()> {
        if *self.lost.borrow()
            || crate::lock::renew_live(db, &self.name, &self.owner, LEASE_TTL).await? != 1
        {
            return Err(busy());
        }
        Ok(())
    }
    fn run<'a, T: 'a>(
        &'a self,
        work: impl std::future::Future<Output = Result<T>> + 'a,
    ) -> impl std::future::Future<Output = Result<T>> + 'a {
        let work = Box::pin(work);
        async move {
            self.ensure(&self.db).await?;
            let mut lost = self.lost.clone();
            let result = tokio::select! { result=work => result, _=lost.changed()=>Err(busy()) };
            self.ensure(&self.db).await?;
            result
        }
    }
    async fn release(&self, db: &impl ConnectionTrait) -> Result<()> {
        crate::lock::release(db, &self.name, &self.owner).await?;
        Ok(())
    }
}
impl Drop for AssetLease {
    fn drop(&mut self) {
        self.renewal.abort();
        let (db, name, owner) = (self.db.clone(), self.name.clone(), self.owner.clone());
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = crate::lock::release(&db, &name, &owner).await;
            });
        }
    }
}

/// A verified object held against GC/internal writers until the caller's short
/// metadata transaction attaches it. Large data I/O happens before that TX.
pub(crate) struct PreparedAsset {
    asset: assets::Model,
    lease: AssetLease,
}
impl PreparedAsset {
    pub(crate) async fn attach(
        &self,
        store: &Store<'_>,
        tx: &DatabaseTransaction,
        owner_kind: &str,
        owner_id: i64,
    ) -> Result<()> {
        self.lease.ensure(tx).await?;
        store
            .attach_metadata(
                tx,
                self.asset.workspace_id,
                self.asset.id,
                owner_kind,
                owner_id,
            )
            .await?;
        // Commit publishes the ref and releases the fence atomically. On
        // rollback the receipt's Drop releases the restored logical lease.
        self.lease.release(tx).await
    }
}
fn chunks(length: u64) -> u64 {
    length / CHUNK as u64 + u64::from(length % CHUNK as u64 != 0)
}
fn manifest_aad(asset: &assets::Model) -> String {
    format!("{}:stream-v2:manifest", aad(asset))
}
fn frame_aad(asset: &assets::Model, manifest: &Manifest, index: u64, kind: &str) -> String {
    format!("{}:stream-v2:{}:{index}:{kind}", aad(asset), manifest.child)
}
fn temporary() -> Result<NamedTempFile> {
    tempfile::Builder::new()
        .prefix("content-asset-")
        .tempfile()
        .map_err(storage)
}
fn input_error(error: std::io::Error) -> ApiError {
    if error.get_ref().is_some_and(|inner| inner.is::<ApiError>()) {
        return *error.into_inner().unwrap().downcast::<ApiError>().unwrap();
    }
    storage(error)
}

impl Store<'_> {
    /// Spool an input while hashing and enforcing the optional upload ceiling.
    /// No plaintext, ciphertext, or hex representation grows with the file size.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn stage_reader(
        &self,
        workspace: i64,
        input: impl AsyncRead + Unpin,
        mime: &str,
        execution: i64,
        node: &str,
        actor: i64,
    ) -> Result<assets::Model> {
        let stream = futures::stream::try_unfold(input, |mut input| async move {
            let mut buffer = vec![0; CHUNK];
            let count = input.read(&mut buffer).await.map_err(input_error)?;
            if count == 0 {
                return Ok(None);
            }
            buffer.truncate(count);
            Ok(Some((bytes::Bytes::from(buffer), input)))
        });
        self.stage_stream(workspace, Box::pin(stream), mime, execution, node, actor)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn stage_stream(
        &self,
        workspace: i64,
        mut input: impl futures::Stream<Item = Result<bytes::Bytes>> + Unpin,
        mime: &str,
        execution: i64,
        node: &str,
        actor: i64,
    ) -> Result<assets::Model> {
        use futures::StreamExt;
        let policy = crate::accounts::resource_policy::load(self.db).await?;
        let plain = temporary()?;
        let mut output = tokio::fs::File::from_std(plain.reopen().map_err(storage)?);
        let mut hash = Sha256::new();
        let mut size = 0u64;
        while let Some(chunk) = input.next().await {
            let chunk = chunk?;
            size = size.checked_add(chunk.len() as u64).ok_or_else(fault)?;
            if policy.file_upload_bytes.is_some_and(|limit| size > limit) {
                return Err(ApiError::localized(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    Message::new("storage.tooLarge")
                        .with("limit", size_text(policy.file_upload_bytes.unwrap() as i64)),
                ));
            }
            hash.update(&chunk);
            output.write_all(&chunk).await.map_err(storage)?;
        }
        output.flush().await.map_err(storage)?;
        drop(output);
        self.stage_prepared(
            workspace,
            plain.path(),
            hex::encode(hash.finalize()),
            i64::try_from(size).map_err(|_| fault())?,
            mime,
            execution,
            node,
            actor,
        )
        .await
    }

    /// A caller-owned immutable spool avoids copying a collected file again.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn stage_file(
        &self,
        workspace: i64,
        path: &Path,
        mime: &str,
        execution: i64,
        node: &str,
        actor: i64,
    ) -> Result<assets::Model> {
        let mut input = tokio::fs::File::open(path).await.map_err(storage)?;
        let mut hash = Sha256::new();
        let mut size = 0u64;
        let mut buffer = vec![0; CHUNK];
        loop {
            let count = input.read(&mut buffer).await.map_err(storage)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
            size = size.checked_add(count as u64).ok_or_else(fault)?;
        }
        drop(input);
        self.stage_prepared(
            workspace,
            path,
            hex::encode(hash.finalize()),
            i64::try_from(size).map_err(|_| fault())?,
            mime,
            execution,
            node,
            actor,
        )
        .await
    }

    async fn staged(&self, tx: &impl ConnectionTrait, asset: &assets::Model) -> Result<bool> {
        Ok(asset.pinned == 0
            && job(tx, asset.workspace_id, asset.id)
                .await?
                .is_some_and(|j| j.state == "candidate")
            && refs::Entity::find()
                .filter(refs::Column::AssetId.eq(asset.id))
                .count(tx)
                .await?
                == 0)
    }

    async fn format(&self, asset: &assets::Model) -> Result<Format> {
        let Some(header) = self
            .backend
            .header(&key(asset), MAGIC.len())
            .await
            .map_err(storage)?
        else {
            return Ok(Format::Missing);
        };
        if header.as_ref() != MAGIC {
            return Ok(Format::Legacy);
        }
        let body = self
            .backend
            .get(&key(asset), MAX_MANIFEST)
            .await
            .map_err(storage)?;
        if !body.starts_with(MAGIC) {
            return Err(fault());
        }
        let sealed = std::str::from_utf8(&body[MAGIC.len()..]).map_err(|_| fault())?;
        let plain = self
            .vault
            .open_bytes(sealed, &manifest_aad(asset))
            .map_err(|_| fault())?;
        let manifest: Manifest = serde_json::from_slice(&plain).map_err(|_| fault())?;
        let prefix = format!("{}.data-", key(asset));
        let identity = manifest.child.strip_prefix(&prefix).ok_or_else(fault)?;
        if manifest.version != 2
            || asset.size < 0
            || manifest.length != asset.size as u64
            || manifest.sha256 != asset.sha256
            || manifest.chunks != chunks(manifest.length)
            || manifest.segment_chunks != SEGMENT_CHUNKS
            || manifest.segments != manifest.chunks.div_ceil(SEGMENT_CHUNKS).max(1)
            || manifest.encoded_length != encoded_length(manifest.length, manifest.chunks)?
            || uuid::Uuid::parse_str(identity).is_err()
        {
            return Err(fault());
        }
        segment_length(&manifest, 0)?;
        segment_length(&manifest, manifest.segments - 1)?;
        Ok(Format::Chunked(manifest))
    }

    #[allow(clippy::too_many_arguments)]
    async fn stage_prepared(
        &self,
        workspace: i64,
        path: &Path,
        hash: String,
        size: i64,
        mime: &str,
        execution: i64,
        node: &str,
        actor: i64,
    ) -> Result<assets::Model> {
        let asset = self
            .reserve_asset(workspace, hash, size, mime, execution, node, actor)
            .await?;
        let lease = AssetLease::acquire(self.db, &asset).await?;
        let outcome = lease
            .run(async {
                let asset = row(self.db, workspace, asset.id).await?;
                if job(self.db, workspace, asset.id)
                    .await?
                    .is_some_and(|j| j.state == "deleting")
                {
                    return Err(ApiError::missing());
                }
                let mut encoded_ready = None;
                let format = match self.format(&asset).await? {
                    Format::Missing => {
                        if !self.staged(self.db, &asset).await? {
                            return Err(fault());
                        }
                        let manifest = Manifest::new(&asset)?;
                        // Local encoding creates no remote multipart state. The manifest
                        // is published before the first immutable ciphertext segment.
                        encoded_ready = Some(self.encode(&asset, &manifest, path).await?);
                        let sealed = self.vault.seal_bytes(
                            &serde_json::to_vec(&manifest).map_err(|_| fault())?,
                            &manifest_aad(&asset),
                        )?;
                        let mut body = MAGIC.to_vec();
                        body.extend_from_slice(sealed.as_bytes());
                        if !self
                            .backend
                            .create(&key(&asset), body.into())
                            .await
                            .map_err(storage)?
                        {
                            // Another immutable manifest may bind a different child.
                            encoded_ready = None;
                        }
                        self.format(&asset).await?
                    }
                    other => other,
                };
                if let Format::Chunked(manifest) = format {
                    if !self.complete(&asset, &manifest).await? {
                        if !self.staged(self.db, &asset).await? {
                            return Err(fault());
                        }
                        let encoded = match encoded_ready {
                            Some(file) => file,
                            None => self.encode(&asset, &manifest, path).await?,
                        };
                        self.upload_segments(&asset, &manifest, encoded.path())
                            .await?;
                    }
                } else if !matches!(format, Format::Legacy) {
                    return Err(fault());
                }
                // Existing valid objects are reused; present corruption is never replaced.
                let _verified = self.read_file_row(&asset).await?;
                Ok(())
            })
            .await;
        if outcome.is_ok() {
            let tx = self.db.begin().await?;
            lease.ensure(&tx).await?;
            jobs::Entity::update_many()
                .col_expr(
                    jobs::Column::EligibleAt,
                    Expr::value(crate::db::now().checked_add(GRACE_MS).ok_or_else(fault)?),
                )
                .col_expr(jobs::Column::UpdatedAt, Expr::value(crate::db::now()))
                .col_expr(
                    jobs::Column::Version,
                    sea_orm::ExprTrait::add(Expr::col(jobs::Column::Version), 1),
                )
                .filter(jobs::Column::AssetId.eq(asset.id))
                .filter(jobs::Column::State.eq("candidate"))
                .exec(&tx)
                .await?;
            tx.commit().await?;
        }
        let released = lease.release(self.db).await;
        outcome?;
        released?;
        Ok(asset)
    }

    pub(crate) async fn prepare(&self, workspace: i64, id: i64) -> Result<PreparedAsset> {
        let asset = row(self.db, workspace, id).await?;
        let lease = AssetLease::acquire(self.db, &asset).await?;
        let result = lease
            .run(async {
                if job(self.db, workspace, id)
                    .await?
                    .is_some_and(|j| j.state == "deleting")
                {
                    return Err(ApiError::missing());
                }
                let current = row(self.db, workspace, id).await?;
                let _verified = self.read_file_row(&current).await?;
                Ok(())
            })
            .await;
        if let Err(error) = result {
            let _ = lease.release(self.db).await;
            return Err(error);
        }
        Ok(PreparedAsset { asset, lease })
    }

    pub(crate) async fn binary_id(
        &self,
        workspace: i64,
        reference: &ferry_protocol::sandbox::Asset,
    ) -> Result<i64> {
        let hash = reference.asset.strip_prefix("sha256:").ok_or_else(fault)?;
        let asset = assets::Entity::find()
            .filter(assets::Column::WorkspaceId.eq(workspace))
            .filter(assets::Column::Sha256.eq(hash))
            .one(self.db)
            .await?
            .ok_or_else(fault)?;
        if u64::try_from(asset.size).ok() != Some(reference.size) || asset.mime != reference.mime {
            return Err(fault());
        }
        Ok(asset.id)
    }

    pub(crate) async fn prepare_ids(
        &self,
        workspace: i64,
        ids: impl IntoIterator<Item = i64>,
    ) -> Result<Vec<PreparedAsset>> {
        let ids: std::collections::BTreeSet<_> = ids.into_iter().collect();
        let mut prepared = Vec::with_capacity(ids.len());
        for id in ids {
            prepared.push(self.prepare(workspace, id).await?);
        }
        Ok(prepared)
    }

    async fn complete(&self, asset: &assets::Model, manifest: &Manifest) -> Result<bool> {
        if !self
            .backend
            .exists(&manifest.child)
            .await
            .map_err(storage)?
        {
            return Ok(false);
        }
        let sealed = self
            .backend
            .get(&manifest.child, MAX_MANIFEST)
            .await
            .map_err(storage)?;
        let sealed = std::str::from_utf8(&sealed).map_err(|_| fault())?;
        let plain = self
            .vault
            .open_bytes(
                sealed,
                &frame_aad(asset, manifest, manifest.chunks, "complete"),
            )
            .map_err(|_| fault())?;
        if plain != completion_record(manifest) {
            return Err(fault());
        }
        Ok(true)
    }
    async fn download_segments(
        &self,
        manifest: &Manifest,
        output: &mut tokio::fs::File,
    ) -> Result<()> {
        for index in 0..manifest.segments {
            let length = segment_length(manifest, index)?;
            let bytes = self
                .backend
                .get(&segment_key(manifest, index), length)
                .await
                .map_err(storage)?;
            if bytes.len() != length {
                return Err(fault());
            }
            output.write_all(&bytes).await.map_err(storage)?;
        }
        output.flush().await.map_err(storage)
    }
    async fn upload_segments(
        &self,
        asset: &assets::Model,
        manifest: &Manifest,
        path: &Path,
    ) -> Result<()> {
        if tokio::fs::metadata(path).await.map_err(storage)?.len() != manifest.encoded_length {
            return Err(fault());
        }
        let mut input = tokio::fs::File::open(path).await.map_err(storage)?;
        let assembled = temporary()?;
        let mut output = tokio::fs::File::from_std(assembled.reopen().map_err(storage)?);
        for index in 0..manifest.segments {
            let length = segment_length(manifest, index)?;
            let mut buffer = vec![0; length];
            input.read_exact(&mut buffer).await.map_err(storage)?;
            let candidate = bytes::Bytes::from(buffer);
            let object = segment_key(manifest, index);
            // Atomic single PUT: bounded, immutable, no upload ID or paid MPU parts.
            let bytes = if self
                .backend
                .create(&object, candidate.clone())
                .await
                .map_err(storage)?
            {
                candidate
            } else {
                self.backend.get(&object, length).await.map_err(storage)?
            };
            if bytes.len() != length {
                return Err(fault());
            }
            output.write_all(&bytes).await.map_err(storage)?;
        }
        output.flush().await.map_err(storage)?;
        drop(output);
        drop(input);
        // Existing partial segments are authenticated together with the newly
        // filled ones before the immutable completion marker becomes visible.
        let _verified = self.decode(asset, manifest, assembled.path()).await?;
        let sealed = self.vault.seal_bytes(
            &completion_record(manifest),
            &frame_aad(asset, manifest, manifest.chunks, "complete"),
        )?;
        self.backend
            .create(&manifest.child, sealed.into())
            .await
            .map_err(storage)?;
        Ok(())
    }

    async fn write_frame(
        &self,
        output: &mut tokio::fs::File,
        asset: &assets::Model,
        manifest: &Manifest,
        index: u64,
        kind: u8,
        plain: &[u8],
    ) -> Result<()> {
        let sealed = self.vault.seal_bytes(
            plain,
            &frame_aad(
                asset,
                manifest,
                index,
                if kind == 0 { "data" } else { "final" },
            ),
        )?;
        output.write_u8(kind).await.map_err(storage)?;
        output
            .write_u32(u32::try_from(sealed.len()).map_err(|_| fault())?)
            .await
            .map_err(storage)?;
        output.write_all(sealed.as_bytes()).await.map_err(storage)?;
        Ok(())
    }

    async fn encode(
        &self,
        asset: &assets::Model,
        manifest: &Manifest,
        path: &Path,
    ) -> Result<NamedTempFile> {
        let encoded = temporary()?;
        let mut output = tokio::fs::File::from_std(encoded.reopen().map_err(storage)?);
        let mut input = tokio::fs::File::open(path).await.map_err(storage)?;
        output.write_all(DATA_MAGIC).await.map_err(storage)?;
        let mut buffer = vec![0; CHUNK];
        let mut hash = Sha256::new();
        let mut total = 0u64;
        for index in 0..manifest.chunks {
            let count = usize::try_from((manifest.length - total).min(CHUNK as u64))
                .map_err(|_| fault())?;
            input
                .read_exact(&mut buffer[..count])
                .await
                .map_err(storage)?;
            hash.update(&buffer[..count]);
            self.write_frame(&mut output, asset, manifest, index, 0, &buffer[..count])
                .await?;
            total += count as u64;
        }
        if input.read(&mut buffer[..1]).await.map_err(storage)? != 0
            || total != manifest.length
            || hex::encode(hash.finalize()) != manifest.sha256
        {
            return Err(fault());
        }
        let mut final_record = Vec::with_capacity(16);
        final_record.extend_from_slice(&manifest.chunks.to_be_bytes());
        final_record.extend_from_slice(&total.to_be_bytes());
        self.write_frame(
            &mut output,
            asset,
            manifest,
            manifest.chunks,
            1,
            &final_record,
        )
        .await?;
        output.flush().await.map_err(storage)?;
        drop(output);
        drop(input);
        Ok(encoded)
    }

    async fn decode(
        &self,
        asset: &assets::Model,
        manifest: &Manifest,
        encoded: &Path,
    ) -> Result<NamedTempFile> {
        let plain = temporary()?;
        let mut output = tokio::fs::File::from_std(plain.reopen().map_err(storage)?);
        let mut input = tokio::fs::File::open(encoded).await.map_err(storage)?;
        let mut magic = vec![0; DATA_MAGIC.len()];
        input.read_exact(&mut magic).await.map_err(|_| fault())?;
        if magic != DATA_MAGIC {
            return Err(fault());
        }
        let mut hash = Sha256::new();
        let mut total = 0u64;
        for index in 0..=manifest.chunks {
            let kind = input.read_u8().await.map_err(|_| fault())?;
            let length = input.read_u32().await.map_err(|_| fault())? as usize;
            let final_frame = index == manifest.chunks;
            if kind != u8::from(final_frame) || length == 0 || length > MAX_FRAME {
                return Err(fault());
            }
            let mut sealed = vec![0; length];
            input.read_exact(&mut sealed).await.map_err(|_| fault())?;
            let sealed = std::str::from_utf8(&sealed).map_err(|_| fault())?;
            let chunk = self
                .vault
                .open_bytes(
                    sealed,
                    &frame_aad(
                        asset,
                        manifest,
                        index,
                        if final_frame { "final" } else { "data" },
                    ),
                )
                .map_err(|_| fault())?;
            if final_frame {
                let mut expected = Vec::with_capacity(16);
                expected.extend_from_slice(&manifest.chunks.to_be_bytes());
                expected.extend_from_slice(&manifest.length.to_be_bytes());
                if chunk != expected {
                    return Err(fault());
                }
            } else {
                let expected = (manifest.length - total).min(CHUNK as u64) as usize;
                if chunk.len() != expected {
                    return Err(fault());
                }
                total += chunk.len() as u64;
                hash.update(&chunk);
                output.write_all(&chunk).await.map_err(storage)?;
            }
        }
        let mut extra = [0];
        if input.read(&mut extra).await.map_err(storage)? != 0
            || total != manifest.length
            || hex::encode(hash.finalize()) != manifest.sha256
        {
            return Err(fault());
        }
        output.flush().await.map_err(storage)?;
        drop(output);
        drop(input);
        Ok(plain)
    }

    pub(super) async fn read_file_row(&self, asset: &assets::Model) -> Result<NamedTempFile> {
        match self.format(asset).await? {
            Format::Missing => Err(storage("content asset object missing")),
            Format::Legacy => {
                // Historical v1 objects retain their original single-AEAD encoding.
                let limit = usize::try_from(asset.size).map_err(|_| fault())?;
                let cipher = self
                    .backend
                    .get(
                        &key(asset),
                        limit
                            .checked_mul(2)
                            .and_then(|n| n.checked_add(128))
                            .ok_or_else(fault)?,
                    )
                    .await
                    .map_err(storage)?;
                let cipher = std::str::from_utf8(&cipher).map_err(|_| fault())?;
                let bytes = self
                    .vault
                    .open_bytes(cipher, &aad(asset))
                    .map_err(|_| fault())?;
                if bytes.len() != limit || hex::encode(Sha256::digest(&bytes)) != asset.sha256 {
                    return Err(fault());
                }
                let plain = temporary()?;
                tokio::fs::write(plain.path(), bytes)
                    .await
                    .map_err(storage)?;
                Ok(plain)
            }
            Format::Chunked(manifest) => {
                if !self.complete(asset, &manifest).await? {
                    return Err(storage("content asset completion missing"));
                }
                let encoded = temporary()?;
                let mut output = tokio::fs::File::from_std(encoded.reopen().map_err(storage)?);
                self.download_segments(&manifest, &mut output).await?;
                drop(output);
                self.decode(asset, &manifest, encoded.path()).await
            }
        }
    }

    /// Verify the complete object before exposing a file or any HTTP body bytes.
    pub(crate) async fn read_file(
        &self,
        db: &impl ConnectionTrait,
        workspace: i64,
        id: i64,
    ) -> Result<NamedTempFile> {
        let asset = row(db, workspace, id).await?;
        if job(db, workspace, id)
            .await?
            .is_some_and(|j| j.state == "deleting")
        {
            return Err(ApiError::missing());
        }
        self.read_file_row(&asset).await
    }

    pub(super) async fn delete_payload(&self, asset: &assets::Model) -> Result<()> {
        if let Format::Chunked(manifest) = self.format(asset).await? {
            for index in 0..manifest.segments {
                self.backend
                    .delete(&segment_key(&manifest, index))
                    .await
                    .map_err(storage)?;
            }
            self.backend
                .delete(&manifest.child)
                .await
                .map_err(storage)?;
        }
        self.backend.delete(&key(asset)).await.map_err(storage)
    }
}
