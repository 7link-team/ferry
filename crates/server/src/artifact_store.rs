//! Shared byte storage; callers choose path or content addressing.
use bytes::Bytes;
use object_store::{
    ObjectStore, PutMode, PutOptions, aws::AmazonS3Builder, local::LocalFileSystem, path::Path,
};
use sea_orm::{EntityTrait, PaginatorTrait};
use serde::{Deserialize, Serialize};
use std::{path::Path as FsPath, sync::Arc};

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Configuration {
    pub kind: String,
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
}
impl Configuration {
    pub(crate) fn read(s: &crate::state::State, sealed: Option<&str>) -> anyhow::Result<Self> {
        let mut config: Self = match sealed {
            Some(value) => {
                serde_json::from_str(&s.vault.open_secret(value, "platform:file-storage")?)?
            }
            None => Self::default(),
        };
        if config.kind.is_empty() {
            config.kind = "local".into();
        }
        Ok(config)
    }
    pub(crate) fn view(&self) -> serde_json::Value {
        let env = std::env::var_os("FERRY_ARTIFACT_STORE").is_some();
        let field = |name: &str, value: &str| {
            if env {
                std::env::var(name).unwrap_or_default()
            } else {
                value.to_owned()
            }
        };
        serde_json::json!({"storage_kind":field("FERRY_ARTIFACT_STORE",&self.kind),"storage_endpoint":field("FERRY_ARTIFACT_S3_ENDPOINT",&self.endpoint),"storage_bucket":field("FERRY_ARTIFACT_S3_BUCKET",&self.bucket),"storage_region":field("FERRY_ARTIFACT_S3_REGION",&self.region),"storage_access_key":field("AWS_ACCESS_KEY_ID",&self.access_key),"storage_has_secret_key":if env{std::env::var_os("AWS_SECRET_ACCESS_KEY").is_some()}else{!self.secret_key.is_empty()},"storage_source":if env{"environment"}else{"platform"}})
    }
    fn backend(&self, root: &FsPath) -> anyhow::Result<ArtifactStore> {
        match self.kind.as_str() {
            "local" => ArtifactStore::local(root),
            "s3" => {
                let url = reqwest::Url::parse(&self.endpoint)?;
                anyhow::ensure!(
                    matches!(url.scheme(), "http" | "https")
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.query().is_none()
                        && url.fragment().is_none(),
                    "invalid S3 endpoint"
                );
                anyhow::ensure!(
                    !self.bucket.is_empty() && !self.region.is_empty(),
                    "S3 bucket and region required"
                );
                let mut builder = AmazonS3Builder::new()
                    .with_endpoint(&self.endpoint)
                    .with_bucket_name(&self.bucket)
                    .with_region(&self.region)
                    .with_allow_http(url.scheme() == "http")
                    .with_virtual_hosted_style_request(false);
                if !self.access_key.is_empty() {
                    builder = builder
                        .with_access_key_id(&self.access_key)
                        .with_secret_access_key(&self.secret_key);
                }
                Ok(ArtifactStore(Arc::new(builder.build()?)))
            }
            _ => anyhow::bail!("invalid storage kind"),
        }
    }
    pub(crate) async fn update(
        s: &crate::state::State,
        conn: &impl sea_orm::ConnectionTrait,
        sealed: Option<&str>,
        fields: &serde_json::Map<String, serde_json::Value>,
    ) -> crate::error::Result<Option<String>> {
        if !fields.keys().any(|key| key.starts_with("storage_")) {
            return Ok(sealed.map(str::to_owned));
        }
        let previous = Self::read(s, sealed).map_err(|_| crate::content_assets::unavailable())?;
        if std::env::var_os("FERRY_ARTIFACT_STORE").is_some() {
            let view = previous.view();
            let unchanged = fields
                .iter()
                .filter(|(key, _)| key.starts_with("storage_"))
                .all(|(key, value)| {
                    if key == "storage_secret_key" {
                        value.as_str().is_none_or(str::is_empty)
                    } else {
                        view.get(key).is_none_or(|old| old == value)
                    }
                });
            if unchanged {
                return Ok(sealed.map(str::to_owned));
            }
            return Err(crate::error::ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("platform.storageChangeRequiresMigration"),
            ));
        }
        let mut config = previous.clone();
        for (key, target) in [
            ("storage_kind", &mut config.kind),
            ("storage_endpoint", &mut config.endpoint),
            ("storage_bucket", &mut config.bucket),
            ("storage_region", &mut config.region),
            ("storage_access_key", &mut config.access_key),
            ("storage_secret_key", &mut config.secret_key),
        ] {
            if let Some(value) = fields.get(key) {
                let value = value
                    .as_str()
                    .ok_or_else(crate::content_assets::unavailable)?;
                if key != "storage_secret_key" || !value.is_empty() {
                    *target = value.trim().to_owned();
                }
            }
        }
        if config == previous {
            return Ok(sealed.map(str::to_owned));
        }
        let reject = || {
            crate::error::ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("platform.storageChangeRequiresMigration"),
            )
        };
        if config.kind == "local" && s.cluster.is_some() {
            return Err(reject());
        }
        // Switching a populated store without a migration would strand existing files.
        if config.kind != previous.kind
            || config.endpoint != previous.endpoint
            || config.bucket != previous.bucket
        {
            if crate::entities::content_assets::Entity::find()
                .count(conn)
                .await?
                > 0
                || crate::entities::pipeline_artifacts::Entity::find()
                    .count(conn)
                    .await?
                    > 0
            {
                return Err(reject());
            }
            if s.data_dir
                .join("value-files")
                .read_dir()
                .is_ok_and(|mut entries| entries.next().is_some())
            {
                return Err(reject());
            }
        }
        let backend = config
            .backend(&s.data_dir)
            .map_err(|_| crate::content_assets::unavailable())?;
        if config.kind == "s3" {
            let key = format!("storage-check/{}", uuid::Uuid::new_v4());
            let bytes = Bytes::from_static(b"storage-check");
            backend
                .create(&key, bytes.clone())
                .await
                .map_err(|_| crate::content_assets::unavailable())?;
            let read = backend.get(&key, 64).await;
            let deleted = backend.delete(&key).await;
            if read.ok().as_ref() != Some(&bytes) || deleted.is_err() {
                return Err(crate::content_assets::unavailable());
            }
        }
        Ok(Some(s.vault.seal(
            &serde_json::to_string(&config).map_err(|_| crate::content_assets::unavailable())?,
            "platform:file-storage",
        )?))
    }
}

#[derive(Clone)]
pub(crate) struct ArtifactStore(Arc<dyn ObjectStore>);

impl ArtifactStore {
    pub(crate) async fn is_remote(s: &crate::state::State) -> crate::error::Result<bool> {
        if let Ok(kind) = std::env::var("FERRY_ARTIFACT_STORE") {
            return Ok(kind == "s3");
        }
        let row = crate::entities::platform_state::Entity::find_by_id(1)
            .one(&s.db)
            .await?
            .ok_or_else(crate::content_assets::unavailable)?;
        Ok(Configuration::read(s, row.file_storage.as_deref())
            .map_err(|_| crate::content_assets::unavailable())?
            .kind
            == "s3")
    }
    pub(crate) async fn upload_file(&self, key: &str, file: &FsPath) -> anyhow::Result<()> {
        use tokio::io::AsyncWriteExt;
        let mut input = tokio::fs::File::open(file).await?;
        let mut output = object_store::buffered::BufWriter::new(self.0.clone(), Self::path(key)?);
        tokio::io::copy(&mut input, &mut output).await?;
        output.shutdown().await?;
        Ok(())
    }
    pub(crate) async fn download_file(&self, key: &str, file: &FsPath) -> anyhow::Result<()> {
        use futures::TryStreamExt;
        use tokio::io::AsyncWriteExt;
        tokio::fs::create_dir_all(
            file.parent()
                .ok_or_else(|| anyhow::anyhow!("missing parent"))?,
        )
        .await?;
        let temp = tempfile::NamedTempFile::new_in(file.parent().unwrap())?;
        let mut output = tokio::fs::File::from_std(temp.reopen()?);
        let mut stream = self.0.get(&Self::path(key)?).await?.into_stream();
        while let Some(bytes) = stream.try_next().await? {
            output.write_all(&bytes).await?;
        }
        output.sync_all().await?;
        drop(output);
        match temp.persist_noclobber(file) {
            Ok(_) => Ok(()),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(error.error.into()),
        }
    }
    pub(crate) async fn for_state(s: &crate::state::State) -> crate::error::Result<Self> {
        Self::for_connection(s, &s.db).await
    }
    /// Callers holding a transaction must reuse it, including on a one-connection pool.
    pub(crate) async fn for_connection(
        s: &crate::state::State,
        conn: &impl sea_orm::ConnectionTrait,
    ) -> crate::error::Result<Self> {
        let backend = if std::env::var_os("FERRY_ARTIFACT_STORE").is_some() {
            Self::configured(&s.data_dir)
        } else {
            let row = crate::entities::platform_state::Entity::find_by_id(1)
                .one(conn)
                .await?
                .ok_or_else(crate::content_assets::unavailable)?;
            Configuration::read(s, row.file_storage.as_deref()).and_then(|c| {
                anyhow::ensure!(s.cluster.is_none() || c.kind == "s3", "cluster requires S3");
                c.backend(&s.data_dir)
            })
        };
        if s.cluster.is_some() && std::env::var("FERRY_ARTIFACT_STORE").is_ok_and(|v| v != "s3") {
            return Err(crate::content_assets::unavailable());
        }
        backend.map_err(|_| crate::content_assets::unavailable())
    }
    pub(crate) fn local(root: &FsPath) -> anyhow::Result<Self> {
        std::fs::create_dir_all(root)?;
        Ok(Self(Arc::new(LocalFileSystem::new_with_prefix(root)?)))
    }
    pub(crate) fn configured(root: &FsPath) -> anyhow::Result<Self> {
        match std::env::var("FERRY_ARTIFACT_STORE")
            .as_deref()
            .unwrap_or("local")
        {
            "local" => Self::local(root),
            "s3" => {
                let setting =
                    |name| std::env::var(name).map_err(|_| anyhow::anyhow!("missing {name}"));
                let endpoint = setting("FERRY_ARTIFACT_S3_ENDPOINT")?;
                let store = AmazonS3Builder::from_env()
                    .with_endpoint(&endpoint)
                    .with_bucket_name(setting("FERRY_ARTIFACT_S3_BUCKET")?)
                    .with_region(setting("FERRY_ARTIFACT_S3_REGION")?)
                    .with_allow_http(endpoint.starts_with("http://"))
                    .with_virtual_hosted_style_request(false)
                    .build()?;
                Ok(Self(Arc::new(store)))
            }
            _ => anyhow::bail!("invalid FERRY_ARTIFACT_STORE"),
        }
    }
    fn path(key: &str) -> anyhow::Result<Path> {
        anyhow::ensure!(
            !key.is_empty()
                && !key.starts_with('/')
                && !key.contains('\\')
                && !key
                    .split('/')
                    .any(|s| s.is_empty() || s == "." || s == ".."),
            "invalid storage key"
        );
        let path = Path::parse(key)?;
        anyhow::ensure!(path.as_ref() == key, "non-canonical storage key");
        Ok(path)
    }
    /// Atomic create: a concurrent writer must never replace the first ciphertext.
    pub(crate) async fn create(&self, key: &str, bytes: Bytes) -> anyhow::Result<bool> {
        match self
            .0
            .put_opts(
                &Self::path(key)?,
                bytes.into(),
                PutOptions {
                    mode: PutMode::Create,
                    ..Default::default()
                },
            )
            .await
        {
            Ok(_) => Ok(true),
            Err(object_store::Error::AlreadyExists { .. }) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
    pub(crate) async fn get(&self, key: &str, limit: usize) -> anyhow::Result<Bytes> {
        let result = self.0.get(&Self::path(key)?).await?;
        anyhow::ensure!(
            result.meta.size <= limit as u64,
            "object exceeds read limit"
        );
        use futures::TryStreamExt;
        let mut stream = result.into_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.try_next().await? {
            anyhow::ensure!(
                chunk.len() <= limit.saturating_sub(bytes.len()),
                "object exceeds read limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes.into())
    }
    pub(crate) async fn delete(&self, key: &str) -> anyhow::Result<()> {
        match self.0.delete(&Self::path(key)?).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}
