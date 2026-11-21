//! Read external repository indexes through a hosting API, or directly over Git.
pub mod catalog;
#[cfg(test)]
mod catalog_tests;
pub(crate) mod git;
pub(crate) mod index;
pub mod platform;
pub(crate) mod store;
use crate::error::LogCause;
use crate::error::{ApiError, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

#[derive(Clone, Deserialize, Serialize)]
pub struct Source {
    pub provider: String,
    pub api_base: String,
    pub project: String,
    #[serde(default)]
    pub organization: String,
    pub branch: String,
    pub index_path: String,
}
impl Source {
    /// Same index location; the API address is compared as a URL, so a
    /// trailing slash or letter case in the host is not a change.
    fn same_location(&self, other: &Source) -> bool {
        let base = |s: &Source| reqwest::Url::parse(s.api_base.trim()).ok();
        self.provider == other.provider
            && self.project.trim() == other.project.trim()
            && self.organization.trim() == other.organization.trim()
            && self.branch.trim() == other.branch.trim()
            && self.index_path.trim() == other.index_path.trim()
            && match (base(self), base(other)) {
                (Some(a), Some(b)) => a == b,
                _ => self.api_base.trim() == other.api_base.trim(),
            }
    }
    fn url(&self) -> Result<reqwest::Url> {
        let mut url = reqwest::Url::parse(&self.api_base).map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("marketplace.repositoryEndpointInvalid"),
            )
        })?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("marketplace.endpointFormatInvalid"),
            ));
        }
        // A Git source is addressed by its clone URL alone.
        let project = if self.provider == "git" {
            "-".to_owned()
        } else {
            self.project.clone()
        };
        for (value, limit) in [
            (&project, 500),
            (&self.branch, 255),
            (&self.index_path, 500),
        ] {
            if value.trim().is_empty() || value.len() > limit || value.chars().any(char::is_control)
            {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("marketplace.sourceFieldsInvalid"),
                ));
            }
        }
        if self.index_path.starts_with('/')
            || self
                .index_path
                .split('/')
                .any(|p| matches!(p, "" | "." | ".."))
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("marketplace.indexPathInvalid"),
            ));
        }
        {
            let mut path = url.path_segments_mut().map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("marketplace.endpointInvalid"),
                )
            })?;
            path.pop_if_empty();
            match self.provider.as_str() {
                "git" => {
                    if !git::valid_branch(&self.branch) {
                        return Err(ApiError::localized(
                            axum::http::StatusCode::BAD_REQUEST,
                            crate::i18n::Message::new("marketplace.sourceFieldsInvalid"),
                        ));
                    }
                    drop(path);
                    return Ok(url);
                }
                "github" | "gitee" => {
                    let parts: Vec<_> = self.project.split('/').collect();
                    if parts.len() != 2
                        || parts
                            .iter()
                            .any(|s| s.is_empty() || matches!(*s, "." | ".."))
                    {
                        return Err(ApiError::localized(
                            axum::http::StatusCode::BAD_REQUEST,
                            crate::i18n::Message::new("marketplace.projectInvalid"),
                        ));
                    }
                    path.push("repos")
                        .extend(parts)
                        .push("contents")
                        .extend(self.index_path.split('/'));
                }
                "gitlab" => {
                    path.push("projects")
                        .push(&self.project)
                        .push("repository")
                        .push("files")
                        .push(&self.index_path);
                }
                "codeup" => {
                    path.push("oapi").push("v1").push("codeup");
                    if !self.organization.is_empty() {
                        path.push("organizations").push(&self.organization);
                    }
                    path.push("repositories")
                        .push(&self.project)
                        .push("files")
                        .extend(self.index_path.split('/'));
                }
                _ => {
                    return Err(ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("marketplace.providerRequired"),
                    ));
                }
            }
        }
        url.query_pairs_mut().append_pair("ref", &self.branch);
        Ok(url)
    }
}

fn validate_origin(source: &Source, repository_url: &str) -> Result<()> {
    let address = if let Some((host, path)) = repository_url
        .strip_prefix("git@")
        .and_then(|v| v.split_once(':'))
    {
        format!("ssh://git@{host}/{path}")
    } else {
        repository_url.to_owned()
    };
    let repo = reqwest::Url::parse(&address).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("marketplace.repositoryUrlInvalid"),
        )
    })?;
    let api = source.url()?;
    let same_host = repo.host_str() == api.host_str()
        && (repo.scheme() == "ssh" || repo.port_or_known_default() == api.port_or_known_default());
    let official = api.scheme() == "https"
        && api.port_or_known_default() == Some(443)
        && match (source.provider.as_str(), repo.host_str(), api.host_str()) {
            ("github", Some("github.com"), Some("api.github.com")) => true,
            ("codeup", Some("codeup.aliyun.com"), Some("openapi-rdc.aliyuncs.com")) => true,
            _ => false,
        };
    if !same_host && !official {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("marketplace.endpointOriginInvalid"),
        ));
    }
    if repo.scheme() == "https" && api.scheme() != "https" {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("marketplace.httpsDowngradeForbidden"),
        ));
    }
    Ok(())
}

async fn fetch(
    source: &Source,
    token: Option<&str>,
    oauth: bool,
    timeout: Duration,
) -> Result<String> {
    let work = async {
        let mut url = source.url()?;
        if source.provider == "git" {
            return git::read_index_with_timeout(
                url.as_str(),
                &source.branch,
                &source.index_path,
                token,
                usize::MAX,
                timeout,
            )
            .await;
        }
        let mut client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("workspace-template-index/1");
        if !timeout.is_zero() {
            client = client.timeout(timeout);
        }
        let client = client.build().map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("marketplace.clientInitFailed"),
            )
        })?;
        if source.provider == "gitee" {
            if let Some(token) = token {
                url.query_pairs_mut().append_pair("access_token", token);
            }
        }
        // GitHub's JSON wrapper carries no content above 1 MB; the raw media type serves files up to 100 MB.
        let accept = if source.provider == "github" {
            "application/vnd.github.raw"
        } else {
            "application/json"
        };
        let mut request = client.get(url).header("Accept", accept);
        if let Some(token) = token {
            request = match source.provider.as_str() {
                "github" => request.bearer_auth(token),
                "gitlab" if oauth => request.bearer_auth(token),
                "gitlab" => request.header("PRIVATE-TOKEN", token),
                "codeup" => request.header("x-yunxiao-token", token),
                _ => request,
            };
        }
        // Never expose reqwest errors: they may include Gitee's query credential.
        let mut response = request
            .send()
            .await
            .log_cause("marketplace index request")
            .map_err(|error| {
                let key = if error.is_timeout() {
                    "marketplace.requestTimeout"
                } else if error.is_connect() {
                    "marketplace.requestUnreachable"
                } else {
                    "marketplace.requestFailed"
                };
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new(key),
                )
            })?;
        if !response.status().is_success() {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("marketplace.httpStatus")
                    .with("status", response.status().as_u16().to_string()),
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .log_cause("marketplace index read")
            .map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("marketplace.fileReadFailed"),
                )
            })?
        {
            bytes.extend_from_slice(&chunk);
        }
        let document: Value = serde_json::from_slice(&bytes).map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("marketplace.responseJsonRequired"),
            )
        })?;
        // A raw index body is itself the document: neither index format has a top-level "content".
        if source.provider == "github" && document.get("content").is_none() {
            let text = String::from_utf8(bytes).map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("marketplace.utf8Required"),
                )
            })?;
            return Ok(text);
        }
        let content = document
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("marketplace.contentMissing"),
                )
            })?;
        let text = match document
            .get("encoding")
            .and_then(Value::as_str)
            .unwrap_or("base64")
        {
            "base64" => {
                let compact: String = content
                    .chars()
                    .filter(|c| !c.is_ascii_whitespace())
                    .collect();
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(compact)
                    .map_err(|_| {
                        ApiError::localized(
                            axum::http::StatusCode::BAD_REQUEST,
                            crate::i18n::Message::new("marketplace.base64Invalid"),
                        )
                    })?;
                String::from_utf8(bytes).map_err(|_| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("marketplace.utf8Required"),
                    )
                })?
            }
            "text" => content.to_owned(),
            _ => {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("marketplace.encodingUnsupported"),
                ));
            }
        };
        Ok(text)
    };
    if timeout.is_zero() {
        work.await
    } else {
        tokio::time::timeout(timeout, work).await.map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("marketplace.requestTimeout"),
            )
        })?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn marketplace_fetch_uses_the_configured_task_deadline() {
        delayed_index(Duration::from_secs(1), 2, false).await;
    }

    #[tokio::test]
    async fn marketplace_fetch_without_a_deadline_allows_slow_responses() {
        delayed_index(Duration::ZERO, 21, true).await;
    }

    async fn delayed_index(timeout: Duration, delay: u64, succeeds: bool) {
        let router = axum::Router::new().route(
            "/{*path}",
            axum::routing::get(move || async move {
                tokio::time::sleep(Duration::from_secs(delay)).await;
                axum::Json(serde_json::json!({"schemaVersion":2,"packages":[]}))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = source("github");
        config.api_base = format!("http://{}", listener.local_addr().unwrap());
        struct Server(tokio::task::JoinHandle<()>);
        impl Drop for Server {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        }));
        let result = fetch(&config, None, false, timeout).await;
        if succeeds {
            assert_eq!(
                serde_json::from_str::<Value>(&result.unwrap()).unwrap(),
                serde_json::json!({"schemaVersion":2,"packages":[]})
            );
        } else {
            assert!(result.is_err());
        }
    }

    #[test]
    fn an_unchanged_source_location_is_recognised_despite_formatting() {
        let stored = Source {
            provider: "github".into(),
            api_base: "http://127.0.0.1:9000".into(),
            project: "owner/repository".into(),
            organization: String::new(),
            branch: "main".into(),
            index_path: "marketplace.json".into(),
        };
        let same = Source {
            api_base: "http://127.0.0.1:9000/".into(),
            project: " owner/repository ".into(),
            ..stored.clone()
        };
        assert!(
            stored.same_location(&same),
            "trailing slash and spaces are not a change"
        );
        for moved in [
            Source {
                branch: "release".into(),
                ..stored.clone()
            },
            Source {
                index_path: "other.json".into(),
                ..stored.clone()
            },
            Source {
                api_base: "http://127.0.0.1:9001".into(),
                ..stored.clone()
            },
            Source {
                provider: "gitlab".into(),
                ..stored.clone()
            },
        ] {
            assert!(!stored.same_location(&moved));
        }
    }

    #[tokio::test]
    async fn persisted_source_errors_follow_the_reader_while_source_fields_stay_literal() {
        use crate::i18n::{Locale, Message, with_locale};
        let error = ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            Message::new("marketplace.httpStatus").with("status", "403"),
        );
        let user_text = r#"User {name} {"$system_message":{"key":"error.forbidden"}}"#;
        let row = sources::Model {
            id: 1,
            workspace_id: 2,
            name: user_text.into(),
            repository_id: 3,
            config: serde_json::to_string(&source("github")).unwrap(),
            state: "failed".into(),
            last_error: error.stored(),
            synced_at: 0,
            run_as: 1,
            version: 4,
            created_at: 0,
            updated_at: 0,
            trust: "workspace".into(),
            enabled: 1,
            sync_interval: 0,
            next_sync_at: 0,
            sync_owner: String::new(),
            sync_lease_until: 0,
            created_by: 0,
            updated_by: 0,
            synced_by: 0,
        };
        for locale in Locale::ALL {
            let view = with_locale(locale, async { source_json(row.clone()) })
                .await
                .unwrap();
            assert_eq!(view["last_error"], error.render(locale));
            assert_eq!(view["name"], user_text);
            assert_eq!(view["project"], "group/repository");
            assert_eq!(view["branch"], "feature/a b");
            assert_eq!(view["state"], "failed");
            let raw = sources::Model {
                last_error: crate::i18n::store_raw(user_text),
                ..row.clone()
            };
            assert_eq!(
                with_locale(locale, async { source_json(raw) })
                    .await
                    .unwrap()["last_error"],
                user_text
            );
        }
    }
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    fn source(provider: &str) -> Source {
        Source {
            provider: provider.into(),
            api_base: "https://api.example.invalid/api/v4".into(),
            project: "group/repository".into(),
            organization: "org".into(),
            branch: "feature/a b".into(),
            index_path: "templates/index.json".into(),
        }
    }
    #[test]
    fn platform_paths_escape_repository_and_file_identifiers() {
        assert_eq!(
            source("gitlab").url().unwrap().path(),
            "/api/v4/projects/group%2Frepository/repository/files/templates%2Findex.json"
        );
        assert!(
            source("github")
                .url()
                .unwrap()
                .path()
                .ends_with("/repos/group/repository/contents/templates/index.json")
        );
        assert!(source("codeup").url().unwrap().path().ends_with("/oapi/v1/codeup/organizations/org/repositories/group%2Frepository/files/templates/index.json"));
        assert_eq!(
            source("gitee")
                .url()
                .unwrap()
                .query_pairs()
                .find(|(k, _)| k == "ref")
                .unwrap()
                .1,
            "feature/a b"
        );
        let mut invalid = source("github");
        invalid.index_path = "../secret".into();
        assert!(invalid.url().is_err());
        invalid = source("github");
        invalid.api_base = "https://user:password@example.invalid".into();
        assert!(invalid.url().is_err());
    }
    #[test]
    fn repository_credentials_cannot_be_forwarded_to_another_origin() {
        let mut config = source("github");
        config.api_base = "https://api.github.com".into();
        assert!(validate_origin(&config, "https://github.com/owner/repository.git").is_ok());
        assert!(
            validate_origin(
                &config,
                "https://private.example.invalid/owner/repository.git"
            )
            .is_err()
        );
        config.api_base = "http://github.com".into();
        assert!(validate_origin(&config, "https://github.com/owner/repository.git").is_err());
        config = source("codeup");
        config.api_base = "https://openapi-rdc.aliyuncs.com".into();
        assert!(
            validate_origin(&config, "git@codeup.aliyun.com:organization/repository.git").is_ok()
        );
    }
    #[tokio::test]
    async fn github_requests_raw_file_and_accepts_unwrapped_body() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = source("github");
        config.api_base = format!("http://{}", listener.local_addr().unwrap());
        let handler = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                let mut part = [0u8; 1024];
                let n = stream.read(&mut part).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&part[..n]);
            }
            let lower = String::from_utf8(bytes).unwrap().to_ascii_lowercase();
            assert!(
                lower.contains("accept: application/vnd.github.raw"),
                "{lower}"
            );
            let body = r#"{"schemaVersion":2,"packages":[]}"#;
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        assert_eq!(
            fetch(&config, None, false, Duration::from_secs(20))
                .await
                .unwrap(),
            r#"{"schemaVersion":2,"packages":[]}"#
        );
        handler.await.unwrap();
    }

    #[tokio::test]
    async fn real_http_transport_uses_provider_auth_and_decodes_content() {
        for provider in ["github", "gitlab", "gitee", "codeup"] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut config = source(provider);
            config.api_base = format!("http://{}", listener.local_addr().unwrap());
            let handler = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut part = [0u8; 1024];
                    let n = stream.read(&mut part).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&part[..n]);
                    if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8(bytes).unwrap();
                let lower = request.to_ascii_lowercase();
                match provider {
                    "github" => assert!(lower.contains("authorization: bearer fixture-token")),
                    "gitlab" => assert!(lower.contains("private-token: fixture-token")),
                    "gitee" => assert!(request.contains("access_token=fixture-token")),
                    "codeup" => assert!(lower.contains("x-yunxiao-token: fixture-token")),
                    _ => unreachable!(),
                }
                let body = r#"{"content":"eyJ2ZXJzaW9uIjoxfQ==","encoding":"base64"}"#;
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            });
            assert_eq!(
                fetch(
                    &config,
                    Some("fixture-token"),
                    false,
                    Duration::from_secs(20)
                )
                .await
                .unwrap(),
                r#"{"version":1}"#
            );
            handler.await.unwrap();
        }
    }
}

use crate::error::{Json, Path, Query};
use crate::{
    accounts::workspace::Scope,
    db,
    entities::{marketplace_entries as entries, marketplace_sources as sources, repositories},
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{extract::State, http::StatusCode};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set, sea_query::Expr};
use serde_json::json;
use std::collections::HashMap;
const SOURCES: Spec = Spec {
    search: &["name"],
    filters: &["state", "repository_id"],
    sortable: &["name", "updated_at"],
    default_sort: "name",
};
#[derive(Deserialize)]
pub struct Body {
    name: String,
    #[serde(default)]
    repository_id: String,
    #[serde(default)]
    repository_selection: Option<RepositorySelection>,
    #[serde(flatten)]
    source: Source,
    #[serde(default)]
    version: i64,
    /// Save without reading the index (e.g. it is not published yet). Never stored.
    #[serde(default)]
    skip_check: bool,
}
#[derive(Deserialize)]
struct RepositorySelection {
    connection_id: Id,
    name: String,
    url: String,
    #[serde(default)]
    default_branch: String,
}

async fn bind_source_repository(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    scope: &Scope,
    body: &mut Body,
) -> Result<()> {
    if let Some(selection) = body.repository_selection.take() {
        let (provider, credential) = crate::repositories::auth::resolve_connection(
            s,
            tx,
            scope,
            selection.connection_id,
            &selection.url,
        )
        .await?;
        let repository = serde_json::from_value::<crate::repositories::Body>(json!({
            "name":selection.name,"provider":provider,"url":selection.url,
            "default_branch":selection.default_branch,"credential_id":credential
        }))
        .map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("marketplace.repositoryInvalid"),
            )
        })?;
        body.repository_id = crate::repositories::bind_selection(s, tx, scope, repository).await?;
    }
    Ok(())
}

/// Release version of this build, used to judge package compatibility.
pub(crate) fn ferry_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("crate version is semver")
}
fn source_json(row: sources::Model) -> Result<Value> {
    let mut value: Value = serde_json::from_str(&row.config).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("marketplace.sourceConfigCorrupt"),
        )
    })?;
    let object = value.as_object_mut().ok_or_else(|| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("marketplace.configCorrupt"),
        )
    })?;
    for(k,v) in json!({"id":Id::from(row.id),"name":row.name,"repository_id":Id::from(row.repository_id),"state":row.state,"last_error":crate::i18n::display_stored(&row.last_error),"synced_at":row.synced_at,"version":row.version,"updated_at":row.updated_at}).as_object().unwrap(){object.insert(k.clone(),v.clone());}
    Ok(value)
}
async fn scoped(conn: &impl ConnectionTrait, scope: &Scope, id: Id) -> Result<sources::Model> {
    sources::Entity::find_by_id(i64::from(id))
        .filter(sources::Column::WorkspaceId.eq(scope.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
async fn validate(conn: &impl ConnectionTrait, scope: &Scope, b: &Body) -> Result<()> {
    scope.require("repositories.read")?;
    b.source.url()?;
    if b.name.trim().is_empty() || b.name.chars().count() > 60 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("marketplace.nameInvalid"),
        ));
    }
    let repo = repositories::Entity::find_by_id(i64::from(Id::reference(&b.repository_id)?))
        .filter(repositories::Column::WorkspaceId.eq(scope.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("marketplace.repositoryMissing"),
            )
        })?;
    validate_origin(&b.source, &repo.url)?;
    Ok(())
}
pub async fn list_sources(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("marketplace-sources.read")?;
    let q = ListQuery::parse(&SOURCES, &raw)?;
    let (rows, total) = resource::list::<sources::Entity, sources::Model>(
        &s.db,
        &SOURCES,
        sources::Entity::find(),
        &scope.workspace_id,
        &q,
    )
    .await?;
    Ok(Json(
        json!({"items":rows.into_iter().map(source_json).collect::<Result<Vec<_>>>()?,"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
pub async fn get_source(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("marketplace-sources.read")?;
    Ok(Json(source_json(scoped(&s.db, &scope, id).await?)?))
}
pub async fn create_source(
    State(s): State<AppState>,
    scope: Scope,
    Json(b): Json<Body>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("marketplace-sources.create")?;
    if !b.skip_check {
        check_index(&s, &scope, &b).await?;
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let result = create_source_in(&s, &tx, &scope, b).await?;
    tx.commit().await?;
    Ok(result)
}
pub(crate) async fn create_source_in(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    scope: &Scope,
    mut b: Body,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("marketplace-sources.create")?;
    bind_source_repository(s, tx, scope, &mut b).await?;
    validate(tx, &scope, &b).await?;
    let id = s.next_id()?;
    let now = db::now();
    sources::Entity::insert(sources::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(scope.workspace_id.into()),
        name: Set(b.name.trim().into()),
        repository_id: Set(Id::reference(&b.repository_id)?.into()),
        config: Set(serde_json::to_string(&b.source).unwrap()),
        state: Set("idle".into()),
        last_error: Set(String::new()),
        synced_at: Set(0),
        run_as: Set(scope.user.id.into()),
        version: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    })
    .exec(tx)
    .await?;
    crate::hosts::activity_in(
        &s,
        tx,
        &scope,
        "create",
        "marketplace-sources",
        &id,
        json!({"name":b.name}),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id,"version":1}))))
}
pub async fn update_source(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(mut b): Json<Body>,
) -> Result<Json<Value>> {
    scope.require("marketplace-sources.update")?;
    // Re-read the index only when where it comes from changed: renaming a
    // source, or saving it unchanged, must not depend on the index being reachable.
    let existing = scoped(&s.db, &scope, id).await?;
    let moved = b.repository_selection.is_some()
        || Id::reference(&b.repository_id)? != Id::from(existing.repository_id)
        || serde_json::from_str::<Source>(&existing.config)
            .map_or(true, |stored| !stored.same_location(&b.source));
    if moved && !b.skip_check {
        check_index(&s, &scope, &b).await?;
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    scoped(&tx, &scope, id).await?;
    bind_source_repository(&s, &tx, &scope, &mut b).await?;
    validate(&tx, &scope, &b).await?;
    let n = sources::Entity::update_many()
        .col_expr(sources::Column::Name, Expr::value(b.name.trim()))
        .col_expr(
            sources::Column::RepositoryId,
            Expr::value(Id::reference(&b.repository_id)?),
        )
        .col_expr(
            sources::Column::Config,
            Expr::value(serde_json::to_string(&b.source).unwrap()),
        )
        .col_expr(sources::Column::RunAs, Expr::value(scope.user.id))
        .col_expr(sources::Column::UpdatedAt, Expr::value(db::now()))
        .col_expr(sources::Column::Version, Expr::value(b.version + 1))
        .filter(sources::Column::Id.eq(id))
        .filter(sources::Column::Version.eq(b.version))
        .filter(sources::Column::State.ne("syncing"))
        .exec(&tx)
        .await?
        .rows_affected;
    if n == 0 {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("marketplace.sourceChanged"),
        ));
    }
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "marketplace-sources",
        &id,
        json!({"name":b.name}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"version":b.version+1})))
}
pub async fn delete_source(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<StatusCode> {
    scope.require("marketplace-sources.delete")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let row = scoped(&tx, &scope, id).await?;
    if row.state == "syncing" {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("marketplace.waitForSync"),
        ));
    }
    crate::cron_jobs::ensure_unreferenced(&tx, scope.workspace_id, "marketplace-source", &[id])
        .await?;
    entries::Entity::delete_many()
        .filter(entries::Column::SourceId.eq(id))
        .filter(entries::Column::WorkspaceId.eq(scope.workspace_id))
        .exec(&tx)
        .await?;
    sources::Entity::delete_by_id(i64::from(id))
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "delete",
        "marketplace-sources",
        &id,
        json!({"imported_templates_preserved":true}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
pub use catalog::{get, install, list};
pub async fn synchronize(s: &AppState, scope: Scope, id: Id, timeout: Duration) -> Result<usize> {
    scope.require("marketplace-sources.sync")?;
    scope.require("repositories.read")?;
    let row = {
        let tx = crate::variables::transaction(s, scope.workspace_id).await?;
        let row = scoped(&tx, &scope, id).await?;
        if row.state == "syncing" {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("marketplace.syncing"),
            ));
        }
        sources::Entity::update_many()
            .col_expr(sources::Column::State, Expr::value("syncing"))
            .col_expr(sources::Column::LastError, Expr::value(""))
            .filter(sources::Column::Id.eq(id))
            .exec(&tx)
            .await?;
        tx.commit().await?;
        row
    };
    let result: Result<usize> = async {
        let config: Source = serde_json::from_str(&row.config).map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("marketplace.configCorrupt"),
            )
        })?;
        let repository = repositories::Entity::find_by_id(row.repository_id)
            .filter(repositories::Column::WorkspaceId.eq(scope.workspace_id))
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        let packages = load_index(
            s,
            scope.workspace_id,
            &repository.url,
            repository.credential_id,
            &config,
        )
        .await?;
        // The write budget starts after the fetch: a Git fetch may take minutes.
        let deadline = if timeout.is_zero() {
            None
        } else {
            std::time::Instant::now().checked_add(timeout)
        };
        let fresh = Scope::for_actor(s, scope.workspace_id, scope.user.id).await?;
        fresh.require("marketplace-sources.sync")?;
        fresh.require("repositories.read")?;
        let tx = crate::variables::transaction(s, scope.workspace_id).await?;
        let now = db::now();
        let total = store::replace_entries(
            s,
            &tx,
            scope.workspace_id.into(),
            id.into(),
            scope.user.id.into(),
            packages,
            deadline,
        )
        .await?;
        sources::Entity::update_many()
            .col_expr(sources::Column::State, Expr::value("idle"))
            .col_expr(sources::Column::SyncedAt, Expr::value(now))
            .col_expr(sources::Column::UpdatedAt, Expr::value(now))
            .filter(sources::Column::Id.eq(id))
            .exec(&tx)
            .await?;
        crate::hosts::activity_in(
            s,
            &tx,
            &fresh,
            "sync",
            "marketplace-sources",
            &id,
            json!({"templates":total}),
        )
        .await?;
        tx.commit().await?;
        Ok(total)
    }
    .await;
    if let Err(error) = &result {
        sources::Entity::update_many()
            .col_expr(sources::Column::State, Expr::value("failed"))
            .col_expr(sources::Column::LastError, Expr::value(error.stored()))
            .filter(sources::Column::Id.eq(id))
            .exec(&s.db)
            .await?;
    }
    result
}
pub async fn sync(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    Ok(Json(
        json!({"templates":synchronize(&s,scope,id,Duration::from_secs(20)).await?}),
    ))
}
pub async fn recover(s: &AppState) -> Result<()> {
    sources::Entity::update_many()
        .col_expr(sources::Column::State, Expr::value("failed"))
        .col_expr(
            sources::Column::LastError,
            Expr::value(crate::i18n::store_message(&crate::i18n::Message::new(
                "marketplace.interrupted",
            ))),
        )
        .filter(sources::Column::State.eq("syncing"))
        .filter(sources::Column::WorkspaceId.ne(platform::PLATFORM))
        .exec(&s.db)
        .await?;
    platform::release_expired(s).await
}

/// Reads and parses a source's index exactly as a sync does, without storing
/// anything. Saving a source runs it first so a source that cannot be read is
/// refused with the reason instead of failing at its first sync. It never runs
/// inside a transaction: unsealing the credential uses its own connection.
async fn load_index(
    s: &AppState,
    workspace: Id,
    repository_url: &str,
    credential_id: i64,
    config: &Source,
) -> Result<Vec<index::PackageVersion>> {
    validate_origin(config, repository_url)?;
    let token = if credential_id != 0 {
        crate::repositories::auth::validate_target(
            s,
            &s.db,
            workspace,
            credential_id.into(),
            &config.api_base,
            true,
        )
        .await?;
        let (kind, _, secret) =
            crate::credentials::unseal(s, &workspace, &credential_id.into()).await?;
        if kind != "token" {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("marketplace.tokenRequired"),
            ));
        }
        Some(secret)
    } else {
        None
    };
    let oauth = crate::entities::repository_connections::Entity::find()
        .filter(crate::entities::repository_connections::Column::TokenConfig.is_null())
        .filter(crate::entities::repository_connections::Column::WorkspaceId.eq(workspace))
        .filter(crate::entities::repository_connections::Column::CredentialId.eq(credential_id))
        .one(&s.db)
        .await?
        .is_some();
    let policy = crate::accounts::resource_policy::load(&s.db).await?;
    let timeout = policy
        .task_timeout_seconds
        .map(Duration::from_secs)
        .unwrap_or(Duration::ZERO);
    let text = fetch(config, token.as_deref(), oauth, timeout).await?;
    if !ferry_protocol::resources::allows(policy.tool_response_bytes, text.len()) {
        return Err(ApiError::localized(
            axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            crate::i18n::Message::new("request.resourceLimit"),
        ));
    }
    let rows = index::parse_bundle(&text, &ferry_version(), "en-US")?;
    for row in &rows {
        if !ferry_protocol::resources::allows(policy.file_count, row.files.len())
            || row.files.iter().any(|file| {
                !ferry_protocol::resources::allows(policy.file_upload_bytes, file.content.len())
            })
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::PAYLOAD_TOO_LARGE,
                crate::i18n::Message::new("request.resourceLimit"),
            ));
        }
    }
    Ok(rows)
}
/// The repository a source body points at: a new selection through a saved
/// connection, or an existing repository of the workspace.
async fn source_repository(s: &AppState, scope: &Scope, b: &Body) -> Result<(String, i64)> {
    if let Some(selection) = &b.repository_selection {
        let (_, credential) = crate::repositories::auth::resolve_connection(
            s,
            &s.db,
            scope,
            selection.connection_id,
            &selection.url,
        )
        .await?;
        return Ok((selection.url.clone(), credential.into()));
    }
    let repository = repositories::Entity::find_by_id(i64::from(Id::reference(&b.repository_id)?))
        .filter(repositories::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("marketplace.repositoryMissing"),
            )
        })?;
    Ok((repository.url, repository.credential_id))
}
/// A source is proven when saved: its index must be readable and valid.
async fn check_index(s: &AppState, scope: &Scope, b: &Body) -> Result<()> {
    scope.require("repositories.read")?;
    b.source.url()?;
    let (url, credential) = source_repository(s, scope, b).await?;
    load_index(s, scope.workspace_id, &url, credential, &b.source).await?;
    Ok(())
}
