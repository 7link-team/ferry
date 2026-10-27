//! Read external repository indexes; no Git hosting or local Git executable.
use crate::error::{ApiError, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const MAX_INDEX_BYTES: usize = 2 * 1024 * 1024;

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
    fn url(&self) -> Result<reqwest::Url> {
        let mut url = reqwest::Url::parse(&self.api_base)
            .map_err(|_| ApiError::bad("仓库API接入点无效。"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ApiError::bad(
                "API接入点需为不带用户信息、查询或片段的HTTP(S)地址。",
            ));
        }
        for (value, limit) in [
            (&self.project, 500),
            (&self.branch, 255),
            (&self.index_path, 500),
        ] {
            if value.trim().is_empty() || value.len() > limit || value.chars().any(char::is_control)
            {
                return Err(ApiError::bad(
                    "仓库、分支与索引路径不能为空或包含控制字符。",
                ));
            }
        }
        if self.index_path.starts_with('/')
            || self
                .index_path
                .split('/')
                .any(|p| matches!(p, "" | "." | ".."))
        {
            return Err(ApiError::bad("索引路径需为仓库内相对文件路径。"));
        }
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| ApiError::bad("API接入点无效。"))?;
            path.pop_if_empty();
            match self.provider.as_str() {
                "github" | "gitee" => {
                    let parts: Vec<_> = self.project.split('/').collect();
                    if parts.len() != 2
                        || parts
                            .iter()
                            .any(|s| s.is_empty() || matches!(*s, "." | ".."))
                    {
                        return Err(ApiError::bad("GitHub/Gitee项目格式为 owner/repository。"));
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
                _ => return Err(ApiError::bad("请选择GitHub、GitLab、Gitee或Codeup。")),
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
    let repo = reqwest::Url::parse(&address).map_err(|_| ApiError::bad("仓库URL无效。"))?;
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
        return Err(ApiError::bad(
            "API接入点必须属于所选仓库主机或该平台官方API域名，不能向其他地址发送仓库凭据。",
        ));
    }
    if repo.scheme() == "https" && api.scheme() != "https" {
        return Err(ApiError::bad("HTTPS仓库不可降级到HTTP API。"));
    }
    Ok(())
}

async fn fetch(
    source: &Source,
    token: Option<&str>,
    oauth: bool,
    timeout: Duration,
) -> Result<String> {
    let mut url = source.url()?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout.min(Duration::from_secs(20)))
        .user_agent("workspace-template-index/1")
        .build()
        .map_err(|_| ApiError::bad("无法初始化仓库连接。"))?;
    if source.provider == "gitee" {
        if let Some(token) = token {
            url.query_pairs_mut().append_pair("access_token", token);
        }
    }
    let mut request = client.get(url).header("Accept", "application/json");
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
        .map_err(|_| ApiError::bad("仓库请求失败或超时，请检查接入点和凭据。"))?;
    if !response.status().is_success() {
        return Err(ApiError::bad(format!(
            "仓库返回HTTP {}；请检查文件路径、分支和凭据。",
            response.status().as_u16()
        )));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ApiError::bad("读取仓库文件失败。"))?
    {
        if bytes.len() + chunk.len() > MAX_INDEX_BYTES * 2 {
            return Err(ApiError::bad("仓库文件响应过大。"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let document: Value = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::bad("仓库未返回JSON文件内容，请检查API接入点。"))?;
    let content = document
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad("目标不是可读取的文件，或提供方响应缺少content。"))?;
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
                .map_err(|_| ApiError::bad("仓库文件Base64编码无效。"))?;
            String::from_utf8(bytes).map_err(|_| ApiError::bad("索引必须使用UTF-8编码。"))?
        }
        "text" => content.to_owned(),
        _ => return Err(ApiError::bad("仓库文件编码不受支持。")),
    };
    if text.len() > MAX_INDEX_BYTES {
        return Err(ApiError::bad("索引文件不能超过2MiB。"));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
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

use crate::{
    db,
    entities::{marketplace_entries as entries, marketplace_sources as sources, repositories},
    resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
    workspace::Scope,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set, sea_query::Expr};
use serde_json::json;
use std::collections::{HashMap, HashSet};
const SOURCES: Spec = Spec {
    search: &["name"],
    filters: &["state", "repository_id"],
    sortable: &["name", "updated_at"],
    default_sort: "name",
};
const ENTRIES: Spec = Spec {
    search: &["name", "description"],
    filters: &["source_id"],
    sortable: &["name", "updated_at"],
    default_sort: "name",
};
#[derive(Deserialize)]
pub struct Body {
    name: String,
    repository_id: String,
    #[serde(flatten)]
    source: Source,
    #[serde(default)]
    version: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    version: u32,
    templates: Vec<IndexEntry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexEntry {
    key: String,
    name: String,
    #[serde(default)]
    description: String,
    content: Value,
    #[serde(default = "empty_parameters")]
    parameters: Value,
}
fn empty_parameters() -> Value {
    json!([])
}
fn parse_index(text: &str) -> Result<Index> {
    let index: Index = serde_json::from_str(text)
        .map_err(|_| ApiError::bad("索引需为含version和templates的JSON对象。"))?;
    if index.version != 1 || index.templates.len() > 500 {
        return Err(ApiError::bad("索引版本需为1，模板不能超过500项。"));
    }
    let mut keys = HashSet::new();
    for entry in &index.templates {
        if entry.key.is_empty()
            || entry.key.len() > 100
            || !entry
                .key
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
            || !keys.insert(&entry.key)
            || entry.name.trim().is_empty()
            || entry.name.chars().count() > 60
        {
            return Err(ApiError::bad("每个模板需唯一有效key及1–60字名称。"));
        }
        crate::templates::validate_app(&entry.content.to_string(), &entry.parameters.to_string())?;
    }
    Ok(index)
}
fn source_json(row: sources::Model) -> Result<Value> {
    let mut value: Value =
        serde_json::from_str(&row.config).map_err(|_| ApiError::bad("市场来源配置损坏。"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| ApiError::bad("来源配置损坏。"))?;
    for(k,v) in json!({"id":Id::from(row.id),"name":row.name,"repository_id":Id::from(row.repository_id),"state":row.state,"last_error":row.last_error,"synced_at":row.synced_at,"version":row.version,"updated_at":row.updated_at}).as_object().unwrap(){object.insert(k.clone(),v.clone());}
    Ok(value)
}
fn entry_json(row: entries::Model) -> Value {
    json!({"id":Id::from(row.id),"source_id":Id::from(row.source_id),"entry_key":row.entry_key,"name":row.name,"description":row.description,"content":row.content,"parameters":row.parameters,"version":row.version,"updated_at":row.updated_at})
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
        return Err(ApiError::bad("来源名称需1–60字。"));
    }
    let repo = repositories::Entity::find_by_id(i64::from(Id::reference(&b.repository_id)?))
        .filter(repositories::Column::WorkspaceId.eq(scope.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(|| ApiError::bad("所选外部仓库不存在于当前工作空间。"))?;
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
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let result = create_source_in(&s, &tx, &scope, b).await?;
    tx.commit().await?;
    Ok(result)
}
pub(crate) async fn create_source_in(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    scope: &Scope,
    b: Body,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("marketplace-sources.create")?;
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
    Json(b): Json<Body>,
) -> Result<Json<Value>> {
    scope.require("marketplace-sources.update")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    validate(&tx, &scope, &b).await?;
    scoped(&tx, &scope, id).await?;
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
        return Err(ApiError::conflict("来源已变化或正在同步，请刷新后重试。"));
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
        return Err(ApiError::conflict("来源正在同步，请等待完成。"));
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
pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("marketplace.read")?;
    let q = ListQuery::parse(&ENTRIES, &raw)?;
    let (rows, total) = resource::list::<entries::Entity, entries::Model>(
        &s.db,
        &ENTRIES,
        entries::Entity::find(),
        &scope.workspace_id,
        &q,
    )
    .await?;
    Ok(Json(
        json!({"items":rows.into_iter().map(entry_json).collect::<Vec<_>>(),"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
pub async fn get(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("marketplace.read")?;
    let row = entries::Entity::find_by_id(i64::from(id))
        .filter(entries::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok(Json(entry_json(row)))
}
#[derive(Deserialize)]
pub struct Install {
    version: i64,
}
pub async fn install(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(b): Json<Install>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("marketplace.install")?;
    let row = entries::Entity::find_by_id(i64::from(id))
        .filter(entries::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    if row.version != b.version {
        return Err(ApiError::conflict("市场目录已变化，请重新查看模板后导入。"));
    }
    let body=serde_json::from_value(json!({"name":row.name,"description":row.description,"content":row.content,"parameters":row.parameters})).map_err(|_|ApiError::bad("模板记录损坏。"))?;
    crate::templates::apps::create(State(s), scope, Json(body)).await
}
pub async fn synchronize(s: &AppState, scope: Scope, id: Id, timeout: Duration) -> Result<usize> {
    let deadline = std::time::Instant::now() + timeout;
    scope.require("marketplace-sources.sync")?;
    scope.require("repositories.read")?;
    let row = {
        let tx = crate::variables::transaction(s, scope.workspace_id).await?;
        let row = scoped(&tx, &scope, id).await?;
        if row.state == "syncing" {
            return Err(ApiError::conflict("来源正在同步。"));
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
    let result = async {
        let config: Source =
            serde_json::from_str(&row.config).map_err(|_| ApiError::bad("来源配置损坏。"))?;
        let repository = repositories::Entity::find_by_id(row.repository_id)
            .filter(repositories::Column::WorkspaceId.eq(scope.workspace_id))
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        validate_origin(&config, &repository.url)?;
        let token = if repository.credential_id != 0 {
            crate::repository_auth::validate_target(
                s,
                &s.db,
                scope.workspace_id,
                repository.credential_id.into(),
                &config.api_base,
                true,
            )
            .await?;
            let (kind, _, secret) = crate::credentials::unseal(
                s,
                &scope.workspace_id,
                &repository.credential_id.into(),
            )
            .await?;
            if kind != "token" {
                return Err(ApiError::bad(
                    "外部文件API需要令牌凭据；SSH密钥和密码仅用于Git构建。",
                ));
            }
            Some(secret)
        } else {
            None
        };
        let oauth = crate::entities::repository_connections::Entity::find()
            .filter(crate::entities::repository_connections::Column::TokenConfig.is_null())
            .filter(
                crate::entities::repository_connections::Column::WorkspaceId.eq(scope.workspace_id),
            )
            .filter(
                crate::entities::repository_connections::Column::CredentialId
                    .eq(repository.credential_id),
            )
            .one(&s.db)
            .await?
            .is_some();
        let index = parse_index(&fetch(&config, token.as_deref(), oauth, timeout).await?)?;
        let fresh = Scope::for_actor(s, scope.workspace_id, scope.user.id).await?;
        fresh.require("marketplace-sources.sync")?;
        fresh.require("repositories.read")?;
        let tx = crate::variables::transaction(s, scope.workspace_id).await?;
        let now = db::now();
        entries::Entity::delete_many()
            .filter(entries::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(entries::Column::SourceId.eq(id))
            .exec(&tx)
            .await?;
        let total = index.templates.len();
        for entry in index.templates {
            if std::time::Instant::now() >= deadline {
                return Err(ApiError::bad("同步超时，上次成功目录保留。"));
            }
            entries::Entity::insert(entries::ActiveModel {
                id: Set(s.next_id()?.into()),
                workspace_id: Set(scope.workspace_id.into()),
                source_id: Set(id.into()),
                entry_key: Set(entry.key),
                name: Set(entry.name),
                description: Set(entry.description),
                content: Set(serde_json::to_string_pretty(&entry.content).unwrap()),
                parameters: Set(serde_json::to_string(&entry.parameters).unwrap()),
                version: Set(1),
                created_at: Set(now),
                updated_at: Set(now),
            })
            .exec(&tx)
            .await?;
        }
        if std::time::Instant::now() >= deadline {
            return Err(ApiError::bad("同步超时，上次成功目录保留。"));
        }
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
            .col_expr(sources::Column::LastError, Expr::value(error.1.clone()))
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
            Expr::value("服务中断，同步未完成；上次成功目录保留，请重新同步。"),
        )
        .filter(sources::Column::State.eq("syncing"))
        .exec(&s.db)
        .await?;
    Ok(())
}
