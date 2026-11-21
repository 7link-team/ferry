//! Shared byte storage; callers choose path or content addressing.
use bytes::Bytes;
use object_store::{
    ObjectStore, PutMode, PutOptions, aws::AmazonS3Builder, local::LocalFileSystem, path::Path,
};
use sea_orm::{EntityTrait, PaginatorTrait};
use serde::{Deserialize, Serialize};
use std::{path::Path as FsPath, sync::Arc};

/// Preserve SDK proxy/security/environment options, overriding only I/O timers.
#[derive(Debug)]
struct TransferConnector;
impl object_store::client::HttpConnector for TransferConnector {
    fn connect(
        &self,
        options: &object_store::ClientOptions,
    ) -> object_store::Result<object_store::client::HttpClient> {
        object_store::client::ReqwestConnector::default().connect(
            &options
                .clone()
                .with_timeout_disabled()
                .with_connect_timeout_disabled(),
        )
    }
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Configuration {
    pub kind: String,
    pub endpoint: String,
    pub bucket: String,
    pub prefix: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
}
/// Why file storage cannot be used, as a localized, actionable message. The
/// `detail` (which may name configuration) goes to the server log only.
pub(crate) struct Problem {
    pub message: crate::i18n::Message,
    pub detail: String,
}
impl Problem {
    fn new(key: &'static str, detail: impl Into<String>) -> Self {
        Self {
            message: crate::i18n::Message::new(key),
            detail: detail.into(),
        }
    }
    fn check(step: &'static str, detail: impl Into<String>) -> Self {
        let mut problem = Self::new("platform.storageCheckFailed", detail);
        problem.message = problem.message.with("step", step);
        problem
    }
    fn local(root: &FsPath, detail: impl Into<String>) -> Self {
        let mut problem = Self::new("platform.storageLocalUnwritable", detail);
        problem.message = problem.message.with("path", root.display().to_string());
        problem
    }
    /// A settings form error: the save is refused with this reason.
    fn rejected(self) -> crate::error::ApiError {
        tracing::warn!(detail = %self.detail, "file storage settings rejected");
        crate::error::ApiError::localized(axum::http::StatusCode::BAD_REQUEST, self.message)
    }
}

/// How long one storage check may take before the store counts as unreachable.
const PROBE_TIMEOUT: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_secs(3)
} else {
    std::time::Duration::from_secs(10)
};

/// Classifies an object store failure; `local` names the directory for local storage.
fn classify(step: &'static str, error: &anyhow::Error, local: Option<&FsPath>) -> Problem {
    let detail = format!("{step}: {error:#}");
    if let Some(root) = local {
        return Problem::local(root, detail);
    }
    if let Some(kind) = error.downcast_ref::<object_store::Error>() {
        match kind {
            object_store::Error::PermissionDenied { .. }
            | object_store::Error::Unauthenticated { .. } => {
                return Problem::new("platform.storageAccessDenied", detail);
            }
            object_store::Error::NotFound { .. } if step == "write" => {
                return Problem::new("platform.storageBucketMissing", detail);
            }
            _ => {}
        }
    }
    // Network failures are recognised from the error's causes, not its wording,
    // which differs between platforms and HTTP stacks.
    for cause in error.chain() {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            use std::io::ErrorKind::*;
            if matches!(
                io.kind(),
                ConnectionRefused
                    | ConnectionReset
                    | ConnectionAborted
                    | NotConnected
                    | TimedOut
                    | HostUnreachable
                    | NetworkUnreachable
                    | AddrNotAvailable
            ) {
                return Problem::new("platform.storageUnreachable", detail);
            }
        }
    }
    // URLs carry random object keys; "401"/"403" inside one must not read as a status.
    let text = detail
        .split_whitespace()
        .filter(|word| !word.contains("://"))
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let any = |needles: &[&str]| needles.iter().any(|needle| text.contains(needle));
    if any(&["nosuchbucket", "bucket does not exist"]) {
        Problem::new("platform.storageBucketMissing", detail)
    } else if any(&[
        "accessdenied",
        "access denied",
        "invalidaccesskeyid",
        "signaturedoesnotmatch",
        "forbidden",
        "403",
        "401",
    ]) {
        Problem::new("platform.storageAccessDenied", detail)
    } else if any(&[
        "dns",
        "connect",
        "connection",
        "tls",
        "certificate",
        "timed out",
        "timeout",
        "error sending request",
        "unreachable",
    ]) {
        Problem::new("platform.storageUnreachable", detail)
    } else {
        Problem::check(step, detail)
    }
}

/// Whether the S3 endpoint accepts a TCP connection at all (DNS and connect),
/// so an unreachable endpoint is named the same way on every platform.
pub(crate) async fn reachable(endpoint: &str) -> Result<(), Problem> {
    let unreachable = |detail: String| Problem::new("platform.storageUnreachable", detail);
    let url =
        reqwest::Url::parse(endpoint).map_err(|error| unreachable(format!("endpoint: {error}")))?;
    let host = url.host_str().unwrap_or_default().to_owned();
    let port = url.port_or_known_default().unwrap_or(443);
    match tokio::time::timeout(
        PROBE_TIMEOUT,
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await
    {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(error)) => Err(unreachable(format!("connect {host}:{port}: {error}"))),
        Err(_) => Err(unreachable(format!("connect {host}:{port}: timed out"))),
    }
}

/// Writes, reads back and deletes one small object, each step bounded in time.
pub(crate) async fn probe(store: &ArtifactStore, local: Option<&FsPath>) -> Result<(), Problem> {
    let key = format!("storage-check/{}", uuid::Uuid::new_v4());
    let bytes = Bytes::from_static(b"storage-check");
    let timeout = |step: &'static str| {
        if let Some(root) = local {
            Problem::local(root, format!("{step}: timed out"))
        } else {
            Problem::new("platform.storageUnreachable", format!("{step}: timed out"))
        }
    };
    tokio::time::timeout(PROBE_TIMEOUT, store.create(&key, bytes.clone()))
        .await
        .map_err(|_| timeout("write"))?
        .map_err(|error| classify("write", &error, local))?;
    let read = tokio::time::timeout(PROBE_TIMEOUT, store.get(&key, 64))
        .await
        .map_err(|_| timeout("read"))?;
    let deleted = tokio::time::timeout(PROBE_TIMEOUT, store.delete(&key))
        .await
        .map_err(|_| timeout("delete"))?;
    match read {
        Ok(read) if read == bytes => {}
        Ok(_) => return Err(Problem::check("read", "read back different bytes")),
        Err(error) => return Err(classify("read", &error, local)),
    }
    deleted.map_err(|error| classify("delete", &error, local))
}

impl Configuration {
    /// Field checks that need no network, each with its own message.
    fn check_fields(&self) -> Result<(), Problem> {
        match self.kind.as_str() {
            "local" => Ok(()),
            "s3" => {
                let valid = reqwest::Url::parse(&self.endpoint).is_ok_and(|url| {
                    matches!(url.scheme(), "http" | "https")
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.query().is_none()
                        && url.fragment().is_none()
                });
                if !valid {
                    return Err(Problem::new("platform.storageEndpointInvalid", "endpoint"));
                }
                if self.bucket.is_empty() || self.region.is_empty() {
                    return Err(Problem::new(
                        "platform.storageBucketRequired",
                        "bucket/region",
                    ));
                }
                Ok(())
            }
            other => Err(Problem::check("configure", format!("unknown kind {other}"))),
        }
    }
    /// Opens the configured store, with a precise reason when it cannot be.
    fn open(&self, root: &FsPath) -> Result<ArtifactStore, Problem> {
        self.check_fields()?;
        self.backend(root).map_err(|error| {
            if self.kind == "local" {
                Problem::local(root, format!("{error:#}"))
            } else {
                Problem::check("configure", format!("{error:#}"))
            }
        })
    }
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
        serde_json::json!({"storage_kind":field("FERRY_ARTIFACT_STORE",&self.kind),"storage_endpoint":field("FERRY_ARTIFACT_S3_ENDPOINT",&self.endpoint),"storage_bucket":field("FERRY_ARTIFACT_S3_BUCKET",&self.bucket),"storage_prefix":field("FERRY_ARTIFACT_S3_PREFIX",&self.prefix),"storage_region":field("FERRY_ARTIFACT_S3_REGION",&self.region),"storage_access_key":field("AWS_ACCESS_KEY_ID",&self.access_key),"storage_has_secret_key":if env{std::env::var_os("AWS_SECRET_ACCESS_KEY").is_some()}else{!self.secret_key.is_empty()},"storage_source":if env{"environment"}else{"platform"}})
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
                    .with_http_connector(TransferConnector)
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
                ArtifactStore::prefixed(Arc::new(builder.build()?), &self.prefix)
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
        // Settings that can no longer be decrypted are re-entered rather than blocking the fix.
        let (previous, unreadable) = match Self::read(s, sealed) {
            Ok(previous) => (previous, false),
            Err(error) => {
                tracing::error!(error = %format!("{error:#}"), "saved file storage settings unreadable");
                (
                    Self {
                        kind: "local".into(),
                        ..Self::default()
                    },
                    true,
                )
            }
        };
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
            ("storage_prefix", &mut config.prefix),
            ("storage_region", &mut config.region),
            ("storage_access_key", &mut config.access_key),
            ("storage_secret_key", &mut config.secret_key),
        ] {
            if let Some(value) = fields.get(key) {
                let value = value.as_str().ok_or_else(|| {
                    crate::error::ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("platform.storageFieldInvalid")
                            .with("field", key),
                    )
                })?;
                if key != "storage_secret_key" || !value.is_empty() {
                    *target = value.trim().to_owned();
                }
            }
        }
        config.prefix = ArtifactStore::normalize_prefix(&config.prefix).map_err(|_| {
            crate::error::ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("platform.storagePrefixInvalid"),
            )
        })?;
        if config == previous && !unreadable {
            return Ok(sealed.map(str::to_owned));
        }
        let reject = || {
            crate::error::ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("platform.storageChangeRequiresMigration"),
            )
        };
        // Switching a populated store without a migration would strand existing files.
        if config.kind != previous.kind
            || config.endpoint != previous.endpoint
            || config.bucket != previous.bucket
            || config.prefix != previous.prefix
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
        // Settings that must work are proven when saved: write, read back, delete.
        let backend = config.open(&s.data_dir).map_err(Problem::rejected)?;
        let local = (config.kind == "local").then(|| s.data_dir.as_path());
        if config.kind == "s3" {
            reachable(&config.endpoint)
                .await
                .map_err(Problem::rejected)?;
        }
        probe(&backend, local).await.map_err(Problem::rejected)?;
        let sealed = s.vault.seal(
            &serde_json::to_string(&config).map_err(|error| crate::error::internal_fault(error))?,
            "platform:file-storage",
        )?;
        forget_status();
        Ok(Some(sealed))
    }
}

#[derive(Clone)]
pub(crate) struct ArtifactStore(Arc<dyn ObjectStore>, Option<std::time::Duration>);

impl ArtifactStore {
    async fn transfer<T>(
        &self,
        work: impl std::future::Future<Output = anyhow::Result<T>>,
    ) -> anyhow::Result<T> {
        match self.1 {
            Some(timeout) => tokio::time::timeout(timeout, work).await?,
            None => work.await,
        }
    }

    async fn upload_file_with_deadline(&self, key: &str, file: &FsPath) -> anyhow::Result<()> {
        use futures::{StreamExt, TryStreamExt};
        use tokio::io::AsyncReadExt;
        const PART: usize = 10 * 1024 * 1024;
        let mut upload: Option<Box<dyn object_store::MultipartUpload>> = None;
        let result = self
            .transfer(async {
                let path = Self::path(key)?;
                let mut input = tokio::fs::File::open(file).await?;
                let mut bytes = Vec::with_capacity(PART);
                (&mut input)
                    .take(PART as u64)
                    .read_to_end(&mut bytes)
                    .await?;
                if bytes.len() < PART {
                    drop(input);
                    self.0.put(&path, Bytes::from(bytes).into()).await?;
                    return Ok(());
                }
                // Retain the known upload outside the cancellable future, including
                // during complete(). BufWriter::abort panics after shutdown begins.
                // Initiation before a handle is returned, caller cancellation, and
                // process crashes retain the SDK's existing multipart cleanup gap.
                upload = Some(self.0.put_multipart(&path).await?);
                let handle = upload.as_mut().unwrap();
                let mut parts = futures::stream::FuturesUnordered::new();
                loop {
                    parts.push(handle.put_part(Bytes::from(bytes).into()));
                    if parts.len() >= 8 {
                        parts.try_next().await?;
                    }
                    bytes = Vec::with_capacity(PART);
                    (&mut input)
                        .take(PART as u64)
                        .read_to_end(&mut bytes)
                        .await?;
                    if bytes.is_empty() {
                        break;
                    }
                }
                drop(input);
                while let Some(result) = parts.next().await {
                    result?;
                }
                handle.complete().await?;
                Ok(())
            })
            .await;
        if result.is_err() {
            if let Some(upload) = upload.as_mut() {
                // Cleanup is finite independently of the actual transfer budget.
                if !matches!(
                    tokio::time::timeout(std::time::Duration::from_secs(5), upload.abort()).await,
                    Ok(Ok(()))
                ) {
                    tracing::warn!("failed to abort timed or failed multipart upload");
                }
            }
        }
        result
    }
    #[cfg(test)]
    pub(crate) fn from_backend_for_test(backend: Arc<dyn ObjectStore>) -> Self {
        Self(backend, None)
    }
    fn normalize_prefix(prefix: &str) -> anyhow::Result<String> {
        let prefix = prefix.trim();
        anyhow::ensure!(!prefix.starts_with('/'), "S3 prefix must be relative");
        let prefix = prefix.strip_suffix('/').unwrap_or(prefix);
        anyhow::ensure!(prefix.len() <= 512, "S3 prefix too long");
        if !prefix.is_empty() {
            Self::path(prefix)?;
        }
        Ok(prefix.to_owned())
    }
    fn prefixed(store: Arc<dyn ObjectStore>, prefix: &str) -> anyhow::Result<Self> {
        let prefix = Self::normalize_prefix(prefix)?;
        if prefix.is_empty() {
            return Ok(Self(store, None));
        }
        Ok(Self(
            Arc::new(object_store::prefix::PrefixStore::new(
                store,
                Path::parse(prefix)?,
            )),
            None,
        ))
    }
    pub(crate) async fn upload_file(&self, key: &str, file: &FsPath) -> anyhow::Result<()> {
        if self.1.is_some() {
            return self.upload_file_with_deadline(key, file).await;
        }
        use tokio::io::AsyncWriteExt;
        let mut input = tokio::fs::File::open(file).await?;
        let mut output = object_store::buffered::BufWriter::new(self.0.clone(), Self::path(key)?);
        tokio::io::copy(&mut input, &mut output).await?;
        drop(input);
        output.shutdown().await?;
        Ok(())
    }
    pub(crate) async fn download_file(&self, key: &str, file: &FsPath) -> anyhow::Result<()> {
        self.transfer(async {
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
            drop(stream);
            drop(output);
            temp.persist(file).map_err(|error| error.error)?;
            Ok(())
        })
        .await
    }
    pub(crate) async fn for_state(s: &crate::state::State) -> crate::error::Result<Self> {
        Self::for_connection(s, &s.db).await
    }
    /// Callers holding a transaction must reuse it, including on a one-connection pool.
    pub(crate) async fn for_connection(
        s: &crate::state::State,
        conn: &impl sea_orm::ConnectionTrait,
    ) -> crate::error::Result<Self> {
        let policy = crate::accounts::resource_policy::load(conn).await?;
        match Self::resolve(s, conn).await? {
            Ok((mut store, _)) => {
                store.1 = policy
                    .task_timeout_seconds
                    .map(std::time::Duration::from_secs);
                Ok(store)
            }
            Err(problem) => {
                // The reason can name configuration; it goes to the server log, not the client.
                tracing::error!(detail = %problem.detail, "file storage unavailable");
                Err(storage_unavailable())
            }
        }
    }
    /// The configured store and, for local storage, its directory; or why not.
    async fn resolve(
        s: &crate::state::State,
        conn: &impl sea_orm::ConnectionTrait,
    ) -> crate::error::Result<Result<(Self, Option<std::path::PathBuf>), Problem>> {
        if let Ok(kind) = std::env::var("FERRY_ARTIFACT_STORE") {
            let local = (kind == "local").then(|| s.data_dir.clone());
            return Ok(Self::configured(&s.data_dir)
                .map(|store| (store, local))
                .map_err(|error| Problem::check("environment", format!("{error:#}"))));
        }
        let row = crate::entities::platform_state::Entity::find_by_id(1)
            .one(conn)
            .await?
            .ok_or_else(|| crate::error::internal_fault("platform state row missing"))?;
        let config = match Configuration::read(s, row.file_storage.as_deref()) {
            Ok(config) => config,
            Err(error) => {
                return Ok(Err(Problem::new(
                    "platform.storageConfigUnreadable",
                    format!("{error:#}"),
                )));
            }
        };
        let local = (config.kind == "local").then(|| s.data_dir.clone());
        Ok(config.open(&s.data_dir).map(|store| (store, local)))
    }
    /// Live storage health for administrators, probed at most every 30 s.
    pub(crate) async fn status(s: &crate::state::State) -> crate::error::Result<serde_json::Value> {
        if let Some((at, value)) = STATUS.lock().unwrap_or_else(|p| p.into_inner()).clone() {
            if at.elapsed() < std::time::Duration::from_secs(30) {
                return Ok(value);
            }
        }
        let value = Self::check(s).await?;
        *STATUS.lock().unwrap_or_else(|p| p.into_inner()) =
            Some((std::time::Instant::now(), value.clone()));
        Ok(value)
    }
    /// One uncached storage check: `{ok}` or `{ok:false, reason_key, reason}`.
    pub(crate) async fn check(s: &crate::state::State) -> crate::error::Result<serde_json::Value> {
        let result = match Self::resolve(s, &s.db).await? {
            Ok((store, local)) => probe(&store, local.as_deref()).await,
            Err(problem) => Err(problem),
        };
        Ok(match result {
            Ok(()) => serde_json::json!({"ok": true}),
            Err(problem) => {
                tracing::error!(detail = %problem.detail, "file storage check failed");
                serde_json::json!({
                    "ok": false,
                    "reason_key": problem.message.key.clone(),
                    "reason": crate::i18n::render(crate::i18n::current_locale(), &problem.message),
                })
            }
        })
    }
    pub(crate) fn local(root: &FsPath) -> anyhow::Result<Self> {
        std::fs::create_dir_all(root)?;
        Ok(Self(
            Arc::new(LocalFileSystem::new_with_prefix(root)?),
            None,
        ))
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
                    .with_http_connector(TransferConnector)
                    .with_endpoint(&endpoint)
                    .with_bucket_name(setting("FERRY_ARTIFACT_S3_BUCKET")?)
                    .with_region(setting("FERRY_ARTIFACT_S3_REGION")?)
                    .with_allow_http(endpoint.starts_with("http://"))
                    .with_virtual_hosted_style_request(false)
                    .build()?;
                Self::prefixed(
                    Arc::new(store),
                    &std::env::var("FERRY_ARTIFACT_S3_PREFIX").unwrap_or_default(),
                )
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
        self.transfer(async {
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
        })
        .await
    }
    pub(crate) async fn get(&self, key: &str, limit: usize) -> anyhow::Result<Bytes> {
        self.transfer(async {
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
        })
        .await
    }
    /// Inspect a format header without materializing the object body.
    pub(crate) async fn header(&self, key: &str, length: usize) -> anyhow::Result<Option<Bytes>> {
        self.transfer(async {
            let path = Self::path(key)?;
            let metadata = match self.0.head(&path).await {
                Ok(metadata) => metadata,
                Err(object_store::Error::NotFound { .. }) => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            let end = metadata.size.min(length as u64);
            if end == 0 {
                return Ok(Some(Bytes::new()));
            }
            Ok(Some(self.0.get_range(&path, 0..end).await?))
        })
        .await
    }
    pub(crate) async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        self.transfer(async {
            match self.0.head(&Self::path(key)?).await {
                Ok(_) => Ok(true),
                Err(object_store::Error::NotFound { .. }) => Ok(false),
                Err(error) => Err(error.into()),
            }
        })
        .await
    }
    pub(crate) async fn delete(&self, key: &str) -> anyhow::Result<()> {
        self.transfer(async {
            match self.0.delete(&Self::path(key)?).await {
                Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
                Err(error) => Err(error.into()),
            }
        })
        .await
    }
}

#[cfg(test)]
mod settings_tests;
#[cfg(test)]
mod prefix_tests {
    use super::*;
    #[test]
    fn prefix_is_relative_canonical_and_old_config_remains_readable() {
        assert_eq!(
            ArtifactStore::normalize_prefix("  team/production/ ").unwrap(),
            "team/production"
        );
        assert_eq!(ArtifactStore::normalize_prefix("").unwrap(), "");
        for invalid in [
            "/",
            "/team",
            "team//",
            "team//files",
            "team/../other",
            "./team",
            "team\\other",
        ] {
            assert!(
                ArtifactStore::normalize_prefix(invalid).is_err(),
                "{invalid}"
            );
        }
        assert!(ArtifactStore::normalize_prefix(&"x".repeat(513)).is_err());
        let old: Configuration = serde_json::from_str(r#"{"kind":"s3","bucket":"old"}"#).unwrap();
        assert_eq!(old.prefix, "");
    }
    #[tokio::test]
    async fn prefix_bounds_create_read_delete_and_preserves_other_businesses() {
        let raw: Arc<dyn ObjectStore> =
            if std::env::var("FERRY_TEST_ASSET_S3").as_deref() == Ok("1") {
                ArtifactStore::configured(FsPath::new("unused-s3-root"))
                    .unwrap()
                    .0
            } else {
                Arc::new(object_store::memory::InMemory::new())
            };
        let root = ArtifactStore::prefixed(raw.clone(), "").unwrap();
        let first = ArtifactStore::prefixed(raw.clone(), "business/first").unwrap();
        let second = ArtifactStore::prefixed(raw, "business/second/").unwrap();
        for (store, value) in [(&root, "original"), (&first, "first"), (&second, "second")] {
            assert!(store.create("same/key", Bytes::from(value)).await.unwrap());
            assert!(
                !store
                    .create("same/key", Bytes::from_static(b"overwrite"))
                    .await
                    .unwrap()
            );
            assert_eq!(store.get("same/key", 100).await.unwrap(), value);
        }
        assert_eq!(
            root.get("business/first/same/key", 100).await.unwrap(),
            "first"
        );
        assert!(first.get("same/key", 1).await.is_err());
        assert!(first.get("../second/same/key", 100).await.is_err());
        first.delete("same/key").await.unwrap();
        assert!(first.get("same/key", 100).await.is_err());
        assert_eq!(root.get("same/key", 100).await.unwrap(), "original");
        assert_eq!(second.get("same/key", 100).await.unwrap(), "second");
        root.delete("same/key").await.unwrap();
        second.delete("same/key").await.unwrap();
    }
    #[tokio::test]
    async fn prefix_change_on_populated_store_is_rejected_before_network_access() {
        use crate::agents::tests::setup;
        let (_dir, s) = setup().await;
        std::fs::create_dir_all(s.data_dir.join("value-files")).unwrap();
        std::fs::write(s.data_dir.join("value-files/existing"), "existing").unwrap();
        let sealed = s
            .vault
            .seal(
                &serde_json::to_string(&Configuration {
                    kind: "s3".into(),
                    bucket: "bucket".into(),
                    ..Default::default()
                })
                .unwrap(),
                "platform:file-storage",
            )
            .unwrap();
        let fields = serde_json::json!({"storage_prefix":"another-business"});
        let result =
            Configuration::update(&s, &s.db, Some(&sealed), fields.as_object().unwrap()).await;
        assert_eq!(result.unwrap_err().0, axum::http::StatusCode::CONFLICT);
    }
}

/// File storage could not be opened; administrators fix it in platform settings.
pub(crate) fn storage_unavailable() -> crate::error::ApiError {
    crate::error::ApiError::localized(
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        crate::i18n::Message::new("storage.unavailable"),
    )
}

static STATUS: std::sync::Mutex<Option<(std::time::Instant, serde_json::Value)>> =
    std::sync::Mutex::new(None);
/// Saved settings change the answer; the next status request probes again.
pub(crate) fn forget_status() {
    *STATUS.lock().unwrap_or_else(|p| p.into_inner()) = None;
}
