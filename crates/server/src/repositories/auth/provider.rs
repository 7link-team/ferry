use super::*;

pub(super) struct Provider {
    pub(super) client_id: String,
    pub(super) client_secret: String,
    pub(super) base: reqwest::Url,
    pub(super) api: reqwest::Url,
}
pub(super) fn endpoint(base: &reqwest::Url, path: &str) -> Result<reqwest::Url> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.endpointInvalid"),
            )
        })?
        .pop_if_empty()
        .extend(path.split('/'));
    Ok(url)
}
const SETTINGS_AAD: &str = "platform-repository-oauth:1";
const PROVIDERS: [&str; 4] = ["github", "gitlab", "gitee", "gitea"];

#[derive(Default, Serialize, Deserialize)]
struct ProviderSettings {
    enabled: bool,
    client_id: String,
    client_secret: String,
    base_url: String,
    api_url: String,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct OAuthSettings {
    public_url: String,
    providers: std::collections::BTreeMap<String, ProviderSettings>,
}
fn default_base(kind: &str) -> &str {
    match kind {
        "github" => "https://github.com",
        "gitlab" => "https://gitlab.com",
        "gitea" => "",
        _ => "https://gitee.com",
    }
}
pub(super) fn parse_endpoint(value: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(value).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.endpointInvalid"),
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
            crate::i18n::Message::new("repository.auth.endpointFormat"),
        ));
    }
    Ok(url)
}

impl OAuthSettings {
    pub(crate) fn read(s: &AppState, stored: Option<&str>) -> Result<Self> {
        if let Some(stored) = stored {
            let mut config: Self = serde_json::from_str(
                &s.vault.open_secret(stored, SETTINGS_AAD)?,
            )
            .map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.auth.oauthConfigCorrupt"),
                )
            })?;
            for kind in PROVIDERS {
                config.providers.entry(kind.into()).or_default();
            }
            return Ok(config);
        }
        let mut providers = std::collections::BTreeMap::new();
        for kind in PROVIDERS {
            let prefix = format!("REPOSITORY_OAUTH_{}", kind.to_ascii_uppercase());
            let value = |key: &str| std::env::var(format!("{prefix}_{key}")).unwrap_or_default();
            let client_id = value("CLIENT_ID");
            let client_secret = value("CLIENT_SECRET");
            providers.insert(
                kind.into(),
                ProviderSettings {
                    enabled: !client_id.is_empty() && !client_secret.is_empty(),
                    client_id,
                    client_secret,
                    base_url: value("BASE_URL"),
                    api_url: value("API_URL"),
                },
            );
        }
        Ok(Self {
            public_url: std::env::var("REPOSITORY_OAUTH_PUBLIC_URL").unwrap_or_default(),
            providers,
        })
    }
    pub(crate) async fn load(s: &AppState, conn: &impl sea_orm::ConnectionTrait) -> Result<Self> {
        let row = platform_state::Entity::find_by_id(1)
            .one(conn)
            .await?
            .ok_or_else(ApiError::missing)?;
        Self::read(s, row.repository_oauth.as_deref())
    }
    pub(crate) fn view(&self, s: &AppState, saved: bool) -> Value {
        let mut view = json!({"oauth_public_url":self.public_url,
            "oauth_callback_url":self.callback(s).unwrap_or_default(),
            "oauth_source":if saved { crate::i18n::render(crate::i18n::current_locale(), &crate::i18n::Message::new("repository.monitor.sourceSaved")) } else { crate::i18n::render(crate::i18n::current_locale(), &crate::i18n::Message::new("repository.monitor.sourceEnvironment")) }});
        for (kind, value) in &self.providers {
            for (key, field) in [
                ("enabled", json!(value.enabled)),
                ("client_id", json!(value.client_id)),
                ("has_client_secret", json!(!value.client_secret.is_empty())),
                ("base_url", json!(value.base_url)),
                ("api_url", json!(value.api_url)),
            ] {
                view[format!("oauth_{kind}_{key}")] = field;
            }
        }
        view
    }
    pub(crate) fn update(
        &mut self,
        s: &AppState,
        patch: &serde_json::Map<String, Value>,
    ) -> Result<()> {
        fn text(
            patch: &serde_json::Map<String, Value>,
            key: &str,
            target: &mut String,
            secret: bool,
        ) -> Result<()> {
            if let Some(value) = patch.get(key) {
                let value = value.as_str().ok_or_else(|| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("repository.auth.oauthFieldType"),
                    )
                })?;
                if value.len() > 8192 || value.chars().any(char::is_control) {
                    return Err(ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("repository.auth.oauthFieldLength"),
                    ));
                }
                if !secret || !value.is_empty() {
                    *target = if secret {
                        value.into()
                    } else {
                        value.trim().into()
                    };
                }
            }
            Ok(())
        }
        text(patch, "oauth_public_url", &mut self.public_url, false)?;
        if !self.public_url.is_empty() {
            self.callback(s)?;
        }
        for kind in PROVIDERS {
            let value = self.providers.get_mut(kind).ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.auth.oauthConfigCorrupt"),
                )
            })?;
            let prefix = format!("oauth_{kind}_");
            if let Some(enabled) = patch.get(&(prefix.clone() + "enabled")) {
                value.enabled = enabled.as_bool().ok_or_else(|| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("repository.auth.oauthEnabledInvalid"),
                    )
                })?;
            }
            text(
                patch,
                &(prefix.clone() + "client_id"),
                &mut value.client_id,
                false,
            )?;
            text(
                patch,
                &(prefix.clone() + "client_secret"),
                &mut value.client_secret,
                true,
            )?;
            text(
                patch,
                &(prefix.clone() + "base_url"),
                &mut value.base_url,
                false,
            )?;
            text(patch, &(prefix + "api_url"), &mut value.api_url, false)?;
            for url in [&value.base_url, &value.api_url] {
                if !url.is_empty() {
                    parse_endpoint(url)?;
                }
            }
            if value.enabled {
                let missing = if value.client_id.is_empty() {
                    Some("Client ID")
                } else if value.client_secret.is_empty() {
                    Some("Client Secret")
                } else if kind == "gitea" && value.base_url.is_empty() {
                    Some("Base URL")
                } else {
                    None
                };
                if let Some(field) = missing {
                    let field = match (kind, field) {
                        ("gitlab", "Client ID") => "Application ID",
                        ("gitlab", "Client Secret") => "Secret",
                        _ => field,
                    };
                    return Err(ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("repository.auth.oauthRequiredField")
                            .with(
                                "provider",
                                match kind {
                                    "github" => "GitHub",
                                    "gitlab" => "GitLab",
                                    "gitea" => "Gitea",
                                    _ => "Gitee",
                                },
                            )
                            .with("field", field),
                    ));
                }
                self.callback(s)?;
                self.provider(kind)?;
            }
        }
        Ok(())
    }
    pub(crate) fn seal(&self, s: &AppState) -> Result<String> {
        s.vault.seal(
            &serde_json::to_string(self).map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.auth.oauthConfigEncode"),
                )
            })?,
            SETTINGS_AAD,
        )
    }
    pub(super) fn provider(&self, kind: &str) -> Result<Provider> {
        let value = self.providers.get(kind).ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.apiTokenRequired"),
            )
        })?;
        if !value.enabled || value.client_id.is_empty() || value.client_secret.is_empty() {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.oauthNotConfigured"),
            ));
        }
        let base = parse_endpoint(if value.base_url.is_empty() {
            default_base(kind)
        } else {
            &value.base_url
        })?;
        let api_default = match kind {
            "github" if base.host_str() == Some("github.com") => {
                "https://api.github.com".to_owned()
            }
            "github" => endpoint(&base, "api/v3")?.to_string(),
            "gitlab" => endpoint(&base, "api/v4")?.to_string(),
            "gitea" => endpoint(&base, "api/v1")?.to_string(),
            _ => endpoint(&base, "api/v5")?.to_string(),
        };
        let api = parse_endpoint(if value.api_url.is_empty() {
            &api_default
        } else {
            &value.api_url
        })?;
        Ok(Provider {
            client_id: value.client_id.clone(),
            client_secret: value.client_secret.clone(),
            base,
            api,
        })
    }
    pub(super) fn callback(&self, s: &AppState) -> Result<String> {
        let base = if self.public_url.is_empty() {
            format!("http://{}", s.bind)
        } else {
            self.public_url.clone()
        };
        let url = parse_endpoint(&base)?;
        if url
            .host_str()
            .is_none_or(|h| matches!(h, "0.0.0.0" | "[::]"))
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.publicUrlRequired"),
            ));
        }
        Ok(endpoint(&url, "api/repository-providers/callback")?.to_string())
    }
}
pub(super) fn config_fingerprint(config: &Provider) -> String {
    hash(&json!([config.client_id, config.base.as_str(), config.api.as_str()]).to_string())
}
pub(super) fn require_same_config(config: &Provider, fingerprint: &str) -> Result<()> {
    if config_fingerprint(config) != fingerprint {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.oauthEndpointChanged"),
        ));
    }
    Ok(())
}
pub(super) async fn response_json(mut response: reqwest::Response) -> Result<Value> {
    if !response.status().is_success() {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.providerHttp")
                .with("status", response.status().as_u16().to_string()),
        ));
    }
    let mut bytes = Vec::new();
    while let Some(part) = response.chunk().await.map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.authorizationRead"),
        )
    })? {
        if bytes.len() + part.len() > 1024 * 1024 {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.authorizationTooLarge"),
            ));
        }
        bytes.extend_from_slice(&part);
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.authorizationInvalid"),
        )
    })
}
pub(super) fn http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .user_agent("workspace-repository-authorization/1")
        .build()
        .map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.authorizationClient"),
            )
        })
}

// Codeup may return fewer items than requested; the documented response headers
// determine whether another page exists. Keep the row-count fallback for proxies
// that omit them, and for the other providers' existing contracts.
pub(super) fn codeup_has_more(response: &reqwest::Response, page: u32) -> Option<bool> {
    if let Some(next) = response
        .headers()
        .get("x-next-page")
        .and_then(|v| v.to_str().ok())
    {
        if next.trim().is_empty() {
            return Some(false);
        }
        if let Ok(next) = next.trim().parse::<u32>() {
            return Some(next > page);
        }
    }
    response
        .headers()
        .get("x-total-pages")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u32>().ok())
        .map(|total| page < total)
}

pub(super) async fn repository_response(response: reqwest::Response, kind: &str) -> Result<Value> {
    if kind == "codeup" && !response.status().is_success() {
        let message = match response.status().as_u16() {
            401 => "repository.auth.codeupUnauthorized",
            403 => "repository.auth.codeupForbidden",
            404 => "repository.auth.codeupMissing",
            429 => "repository.auth.codeupRateLimit",
            500..=599 => "repository.auth.codeupUnavailable",
            300..=399 => "repository.auth.codeupRedirect",
            _ => "repository.auth.codeupFailed",
        };
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new(message),
        ));
    }
    response_json(response).await
}
pub(super) fn token_data(value: &Value) -> Result<(&str, i64, Option<&str>)> {
    let valid = |v: &str| {
        !v.is_empty() && v.len() < 65536 && !v.chars().any(|c| c.is_control() || c.is_whitespace())
    };
    let access = value
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|v| valid(v))
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.accessTokenMissing"),
            )
        })?;
    let expires = match value.get("expires_in") {
        None => 0,
        Some(v) => {
            let seconds = v
                .as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                .ok_or_else(|| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("repository.auth.expiryInvalid"),
                    )
                })?;
            i64::try_from(seconds)
                .ok()
                .and_then(|v| v.checked_mul(1000))
                .and_then(|v| db::now().checked_add(v))
                .filter(|v| *v <= 8_640_000_000_000_000)
                .ok_or_else(|| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("repository.auth.expiryInvalid"),
                    )
                })?
        }
    };
    let refresh = match value.get("refresh_token") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().filter(|v| valid(v)).ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.refreshTokenInvalid"),
            )
        })?),
    };
    Ok((access, expires, refresh))
}

#[derive(Serialize, Deserialize)]
pub(super) struct TokenConfig {
    base_url: String,
    api_url: String,
    #[serde(default)]
    pub(super) organization: String,
}
impl TokenConfig {
    pub(super) fn provider(&self) -> Result<Provider> {
        Ok(Provider {
            client_id: String::new(),
            client_secret: String::new(),
            base: parse_endpoint(&self.base_url)?,
            api: parse_endpoint(&self.api_url)?,
        })
    }
}
pub(super) async fn connection_provider(
    s: &AppState,
    conn: &impl sea_orm::ConnectionTrait,
    row: &connections::Model,
) -> Result<Provider> {
    let config = if let Some(value) = &row.token_config {
        serde_json::from_str::<TokenConfig>(value)
            .map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.auth.connectionConfigCorrupt"),
                )
            })?
            .provider()?
    } else {
        OAuthSettings::load(s, conn)
            .await?
            .provider(&row.provider)?
    };
    require_same_config(&config, &row.config_fingerprint)?;
    Ok(config)
}
pub(super) fn token_config(kind: &str, body: &TokenConnect) -> Result<TokenConfig> {
    if !PROVIDERS.contains(&kind) && kind != "codeup" {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.providerInvalid"),
        ));
    }
    if body.token.is_empty()
        || body.token.len() > 65536
        || body
            .token
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.tokenRequired"),
        ));
    }
    if kind == "codeup"
        && (body.username.trim().is_empty()
            || body.username.len() > 255
            || body.username.chars().any(char::is_control))
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.codeupUsername"),
        ));
    }
    let base = parse_endpoint(if body.base_url.trim().is_empty() {
        if kind == "codeup" {
            "https://codeup.aliyun.com"
        } else {
            default_base(kind)
        }
    } else {
        body.base_url.trim()
    })?;
    let default_api = match kind {
        "github" if base.host_str() == Some("github.com") => "https://api.github.com".into(),
        "github" => endpoint(&base, "api/v3")?.to_string(),
        "gitlab" => endpoint(&base, "api/v4")?.to_string(),
        "gitea" => endpoint(&base, "api/v1")?.to_string(),
        "codeup" => "https://openapi-rdc.aliyuncs.com".into(),
        _ => endpoint(&base, "api/v5")?.to_string(),
    };
    let api = parse_endpoint(if body.api_url.trim().is_empty() {
        &default_api
    } else {
        body.api_url.trim()
    })?;
    if body.organization.len() > 255
        || body
            .organization
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '?' | '#'))
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.codeupOrganizationInvalid"),
        ));
    }
    if kind == "codeup"
        && api.host_str() == Some("openapi-rdc.aliyuncs.com")
        && body.organization.trim().is_empty()
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.codeupOrganizationRequired"),
        ));
    }
    Ok(TokenConfig {
        base_url: base.to_string(),
        api_url: api.to_string(),
        organization: body.organization.trim().into(),
    })
}
pub(super) fn authorize_request(
    request: reqwest::RequestBuilder,
    kind: &str,
    token: &str,
    personal: bool,
) -> reqwest::RequestBuilder {
    match kind {
        "gitee" => request.query(&[("access_token", token)]),
        "codeup" => request.header("x-yunxiao-token", token),
        "gitlab" if personal => request.header("PRIVATE-TOKEN", token),
        "gitea" if personal => request.header("Authorization", format!("token {token}")),
        _ => request.bearer_auth(token),
    }
}
pub(super) async fn remote_repositories(
    kind: &str,
    config: &Provider,
    access: &str,
    personal: bool,
    organization: &str,
    page: u32,
) -> Result<Value> {
    let mut url = match kind {
        "gitlab" => endpoint(&config.api, "projects")?,
        "codeup" => {
            let mut url = endpoint(&config.api, "oapi/v1/codeup")?;
            {
                let mut path = url.path_segments_mut().map_err(|_| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("repository.auth.apiUrlInvalid"),
                    )
                })?;
                if !organization.is_empty() {
                    path.push("organizations").push(organization);
                }
                path.push("repositories");
            }
            url
        }
        _ => endpoint(&config.api, "user/repos")?,
    };
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("page", &page.to_string()).append_pair(
            if kind == "codeup" {
                "perPage"
            } else if kind == "gitea" {
                "limit"
            } else {
                "per_page"
            },
            "50",
        );
        if kind == "gitlab" {
            query.append_pair("membership", "true");
        }
    }
    let response = authorize_request(http()?.get(url), kind, access, personal)
        .send()
        .await
        .map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.repositoriesRead"),
            )
        })?;
    let has_more = (kind == "codeup")
        .then(|| codeup_has_more(&response, page))
        .flatten();
    let value = repository_response(response, kind).await?;
    let rows = value.as_array().ok_or_else(|| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.repositoriesInvalid"),
        )
    })?;
    let mut items = Vec::new();
    for row in rows {
        let text = |keys: &[&str]| {
            keys.iter()
                .find_map(|k| row.get(k).and_then(Value::as_str))
                .unwrap_or("")
        };
        let clone_url = text(&[
            "clone_url",
            "http_url_to_repo",
            "httpCloneUrl",
            "httpUrlToRepo",
        ]);
        let url = if clone_url.is_empty() && kind == "codeup" {
            let web = text(&["webUrl"]);
            if web.is_empty() {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.auth.codeupRepositoryMissing"),
                ));
            }
            format!("{}.git", web.trim_end_matches('/').trim_end_matches(".git"))
        } else {
            clone_url.to_owned()
        };
        let address = parse_endpoint(&url)?;
        if address.host_str() != config.base.host_str()
            || address.port_or_known_default() != config.base.port_or_known_default()
            || (config.base.scheme() == "https" && address.scheme() != "https")
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.repositoryHostMismatch"),
            ));
        }
        items.push(json!({"name":text(&["name"]),"full_name":text(&["full_name","path_with_namespace","pathWithNamespace","name"]),
            "url":url,"default_branch":text(&["default_branch","defaultBranch"]),
            "provider_instance":config.api.as_str(),
            "remote_id":row.get("id").or_else(||row.get("repositoryId"))
                .map(|v|v.as_str().map(str::to_owned).unwrap_or_else(||v.to_string())).unwrap_or_default()}));
    }
    Ok(json!({"items":items,"page":page,"has_more":has_more.unwrap_or(rows.len()==50)}))
}
/// Called only when an execution needs a credential, not during metadata reads.
pub async fn ensure_fresh(s: &AppState, workspace: Id, credential: Id) -> Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    let (row, version) = loop {
        let tx = crate::variables::transaction(s, workspace).await?;
        let Some(row) = connections::Entity::find()
            .filter(connections::Column::WorkspaceId.eq(workspace))
            .filter(connections::Column::CredentialId.eq(credential))
            .filter(connections::Column::TokenConfig.is_null())
            .one(&tx)
            .await?
        else {
            return Ok(());
        };
        if row.state == "reauthorize" {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.oauthInvalid"),
            ));
        }
        if row.state == "refreshing" {
            tx.commit().await?;
            if std::time::Instant::now() >= deadline {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.auth.refreshPending"),
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }
        if row.expires_at == 0 || row.expires_at > db::now() + 30_000 {
            return Ok(());
        }
        if row.refresh_secret.is_empty() {
            if row.expires_at > db::now() {
                return Ok(());
            }
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.oauthExpired"),
            ));
        }
        let version = row.version + 1;
        connections::Entity::update_many()
            .col_expr(connections::Column::State, Expr::value("refreshing"))
            .col_expr(connections::Column::Version, Expr::value(version))
            .filter(connections::Column::Id.eq(row.id))
            .exec(&tx)
            .await?;
        tx.commit().await?;
        break (row, version);
    };
    let result: Result<()> = async {
        let config = OAuthSettings::load(s, &s.db)
            .await?
            .provider(&row.provider)?;
        require_same_config(&config, &row.config_fingerprint)?;
        let refresh = s.vault.open_secret(
            &row.refresh_secret,
            &connection_aad(workspace, row.id.into()),
        )?;
        let form = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
        ];
        let tokens = response_json(
            http()?
                .post(endpoint(
                    &config.base,
                    if matches!(row.provider.as_str(), "github" | "gitea") {
                        "login/oauth/access_token"
                    } else {
                        "oauth/token"
                    },
                )?)
                .header("Accept", "application/json")
                .form(&form)
                .send()
                .await
                .map_err(|_| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("repository.auth.refreshFailed"),
                    )
                })?,
        )
        .await?;
        let (access, expires, new_refresh) = token_data(&tokens)?;
        let tx = crate::variables::transaction(s, workspace).await?;
        if connections::Entity::find_by_id(row.id)
            .filter(connections::Column::Version.eq(version))
            .one(&tx)
            .await?
            .is_none()
        {
            return Ok(());
        }
        credentials::Entity::update_many()
            .col_expr(
                credentials::Column::Secret,
                Expr::value(s.vault.seal(
                    access,
                    &vault::aad(
                        &workspace.to_string(),
                        "credentials",
                        &credential.to_string(),
                    ),
                )?),
            )
            .col_expr(
                credentials::Column::Version,
                Expr::col(credentials::Column::Version).add(1),
            )
            .col_expr(credentials::Column::UpdatedAt, Expr::value(db::now()))
            .filter(credentials::Column::Id.eq(credential))
            .filter(credentials::Column::WorkspaceId.eq(workspace))
            .exec(&tx)
            .await?;
        connections::Entity::update_many()
            .col_expr(connections::Column::State, Expr::value("connected"))
            .col_expr(connections::Column::ExpiresAt, Expr::value(expires))
            .col_expr(
                connections::Column::RefreshSecret,
                Expr::value(match new_refresh {
                    Some(value) => s
                        .vault
                        .seal(value, &connection_aad(workspace, row.id.into()))?,
                    None => row.refresh_secret.clone(),
                }),
            )
            .col_expr(connections::Column::LastError, Expr::value(""))
            .col_expr(connections::Column::Version, Expr::value(version + 1))
            .col_expr(connections::Column::UpdatedAt, Expr::value(db::now()))
            .filter(connections::Column::Id.eq(row.id))
            .exec(&tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    .await;
    if let Err(error) = &result {
        let tx = crate::variables::transaction(s, workspace).await?;
        let changed = connections::Entity::update_many()
            .col_expr(connections::Column::State, Expr::value("reauthorize"))
            .col_expr(connections::Column::LastError, Expr::value(error.stored()))
            .filter(connections::Column::Id.eq(row.id))
            .filter(connections::Column::Version.eq(version))
            .exec(&tx)
            .await?;
        if changed.rows_affected == 1 {
            crate::repositories::analysis::connection_changed(
                s,
                &tx,
                row.workspace_id,
                row.user_id,
                row.id,
            )
            .await?;
        }
        tx.commit().await?;
    }
    result
}
/// Reuse browsing authorization/refresh and construct pages locally, never
/// forwarding credentials to an untrusted provider pagination link.
fn comparison_stats(
    provider: &str,
    before: &str,
    value: &Value,
    more: Option<bool>,
) -> Option<(i64, i64)> {
    if more == Some(true)
        || ["compare_timeout", "truncated", "overflow"]
            .iter()
            .any(|k| value[*k] == true)
    {
        return None;
    }
    // GitHub comparisons use a merge base. A rewritten history must never be
    // reported as the direct before/after diff when that base differs.
    if provider == "github"
        && value
            .pointer("/merge_base_commit/sha")
            .and_then(Value::as_str)
            != Some(before)
    {
        return None;
    }
    let files = value["files"]
        .as_array()
        .or_else(|| value["diffs"].as_array())?;
    if (provider == "github" && files.len() >= 300) || files.len() > 1000 {
        return None;
    }
    crate::repositories::analysis::notification_diff_totals(files).ok()
}

pub(crate) struct MonitorAccess {
    provider: String,
    config: Provider,
    access: String,
    personal: bool,
    organization: String,
}
/// Server-only connection material for authenticated API consumers. Never serialize.
pub(crate) struct RepositoryApiAccess {
    pub credential_id: Id,
    pub api: reqwest::Url,
    pub token: String,
    pub organization: String,
    pub personal: bool,
}
pub(crate) struct MonitorPage {
    pub rows: Vec<Value>,
    pub has_more: bool,
}
impl MonitorAccess {
    pub(super) fn into_api_access(self, credential_id: Id) -> RepositoryApiAccess {
        RepositoryApiAccess {
            credential_id,
            api: self.config.api,
            token: self.access,
            organization: self.organization,
            personal: self.personal,
        }
    }
    /// A bounded, best-effort comparison for one notification, using the same
    /// authenticated endpoint and diff parser as code insights.
    pub(crate) async fn change_stats(
        &self,
        path: &str,
        before: &str,
        after: &str,
    ) -> Option<(i64, i64)> {
        if !ferry_protocol::contribution::valid_sha(before)
            || !ferry_protocol::contribution::valid_sha(after)
            || before.chars().all(|c| c == '0')
            || after.chars().all(|c| c == '0')
        {
            return None;
        }
        let compare = format!("{before}...{after}");
        let request = async {
            let (value, more) = if self.provider == "gitlab" {
                self.analysis_request(
                    path,
                    &["compare"],
                    &[
                        ("from", before.into()),
                        ("to", after.into()),
                        ("straight", "true".into()),
                    ],
                )
                .await
                .ok()?
            } else {
                self.analysis_request(path, &["compare", &compare], &[])
                    .await
                    .ok()?
            };
            comparison_stats(&self.provider, before, &value, more)
        };
        tokio::time::timeout(Duration::from_secs(5), request)
            .await
            .ok()
            .flatten()
    }

    /// Analysis uses the same pinned endpoint, token refresh and redirect policy
    /// as browsing. Resource segments are encoded individually, never URLs.
    pub(crate) async fn analysis_request(
        &self,
        path: &str,
        resource: &[&str],
        query: &[(&str, String)],
    ) -> Result<(Value, Option<bool>)> {
        let mut url = self.config.api.clone();
        {
            let mut segments = url.path_segments_mut().map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.auth.apiUrlInvalid"),
                )
            })?;
            segments.pop_if_empty();
            match self.provider.as_str() {
                "codeup" => {
                    segments.extend(["oapi", "v1", "codeup"]);
                    if !self.organization.is_empty() {
                        segments.push("organizations").push(&self.organization);
                    }
                    segments.push("repositories").push(path);
                }
                "gitlab" => {
                    segments.push("projects").push(path).push("repository");
                }
                _ => {
                    segments.push("repos").extend(path.split('/'));
                }
            }
            segments.extend(resource.iter().copied());
        }
        url.query_pairs_mut()
            .extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        let response = authorize_request(
            http()?.get(url),
            &self.provider,
            &self.access,
            self.personal,
        )
        .send()
        .await
        .map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.analysisRequestFailed"),
            )
        })?;
        let page = query
            .iter()
            .find(|(k, _)| *k == "page")
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(1);
        let more = codeup_has_more(&response, page)
            .or_else(|| {
                response
                    .headers()
                    .get("x-next-page")
                    .and_then(|v| v.to_str().ok())
                    .map(|v| !v.is_empty())
            })
            .or_else(|| {
                response
                    .headers()
                    .get("link")
                    .and_then(|v| v.to_str().ok())
                    .map(|v| v.contains("rel=\"next\""))
            });
        Ok((repository_response(response, &self.provider).await?, more))
    }

    pub(crate) async fn load(s: &AppState, row: &connections::Model) -> Result<Self> {
        let config = connection_provider(s, &s.db, row).await?;
        ensure_fresh(s, row.workspace_id.into(), row.credential_id.into()).await?;
        let (_, _, access) =
            crate::credentials::unseal(s, &row.workspace_id.into(), &row.credential_id.into())
                .await?;
        let token = row
            .token_config
            .as_deref()
            .map(serde_json::from_str::<TokenConfig>)
            .transpose()
            .map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.auth.configCorrupt"),
                )
            })?;
        Ok(Self {
            provider: row.provider.clone(),
            config,
            access,
            personal: token.is_some(),
            organization: token.map(|v| v.organization).unwrap_or_default(),
        })
    }
    pub(crate) async fn repositories(&self, page: u32) -> Result<Value> {
        remote_repositories(
            &self.provider,
            &self.config,
            &self.access,
            self.personal,
            &self.organization,
            page,
        )
        .await
    }
    pub(crate) async fn resource(
        &self,
        path: &str,
        resource: &str,
        page: u32,
    ) -> Result<MonitorPage> {
        self.listing(path, resource, page, None).await
    }
    /// Newest commits of one branch (first page), for event detail.
    pub(crate) async fn commits(&self, path: &str, reference: &str) -> Result<Vec<Value>> {
        Ok(self
            .listing(path, "commits", 1, Some(reference))
            .await?
            .rows)
    }
    async fn listing(
        &self,
        path: &str,
        resource: &str,
        page: u32,
        reference: Option<&str>,
    ) -> Result<MonitorPage> {
        let mut url = self.config.api.clone();
        let requests = resource == "requests";
        {
            let mut segments = url.path_segments_mut().map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.auth.apiUrlInvalid"),
                )
            })?;
            segments.pop_if_empty();
            match self.provider.as_str() {
                "codeup" => {
                    segments.extend(["oapi", "v1", "codeup"]);
                    if !self.organization.is_empty() {
                        segments.push("organizations").push(&self.organization);
                    }
                    if requests {
                        segments.push("changeRequests");
                    } else {
                        segments.push("repositories").push(path).push(resource);
                    }
                }
                "gitlab" => {
                    segments.push("projects").push(path);
                    if requests {
                        segments.push("merge_requests");
                    } else {
                        segments.push("repository").push(resource);
                    }
                }
                _ => {
                    segments
                        .push("repos")
                        .extend(path.split('/'))
                        .push(if requests { "pulls" } else { resource });
                }
            }
        }
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("page", &page.to_string()).append_pair(
                match self.provider.as_str() {
                    "codeup" => "perPage",
                    "gitea" => "limit",
                    _ => "per_page",
                },
                "50",
            );
            if let Some(reference) = reference {
                query.append_pair(
                    match self.provider.as_str() {
                        "codeup" => "refName",
                        "gitlab" => "ref_name",
                        _ => "sha",
                    },
                    reference,
                );
            }
            if requests {
                if self.provider == "codeup" {
                    query.append_pair("projectIds", path);
                } else {
                    query.append_pair("state", "all");
                }
                if self.provider == "gitlab" {
                    query.append_pair("scope", "all");
                }
            }
        }
        let response = authorize_request(
            http()?.get(url),
            &self.provider,
            &self.access,
            self.personal,
        )
        .send()
        .await
        .map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.pollRequestFailed"),
            )
        })?;
        let more = codeup_has_more(&response, page).or_else(|| {
            response
                .headers()
                .get("link")
                .and_then(|v| v.to_str().ok())
                .map(|link| link.contains("rel=\"next\""))
        });
        let value = repository_response(response, &self.provider).await?;
        let rows = value
            .as_array()
            .ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.auth.pollListInvalid"),
                )
            })?
            .clone();
        let has_more = more.unwrap_or(rows.len() == 50);
        Ok(MonitorPage { rows, has_more })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn notification_comparison_requires_complete_direct_diff() {
        let before = "a".repeat(40);
        let value = serde_json::json!({"merge_base_commit":{"sha":before},"files":[{"additions":12,"deletions":3}]});
        assert_eq!(
            super::comparison_stats("github", &before, &value, Some(false)),
            Some((12, 3))
        );
        assert_eq!(
            super::comparison_stats("github", &before, &value, Some(true)),
            None
        );
        let mut bad = value.clone();
        bad["merge_base_commit"]["sha"] = serde_json::json!("b".repeat(40));
        assert_eq!(super::comparison_stats("github", &before, &bad, None), None);
        let gitlab = serde_json::json!({"diffs":[{"diff":"@@ -1 +1,2 @@\n-old\n+one\n+two\n"}]});
        assert_eq!(
            super::comparison_stats("gitlab", &before, &gitlab, None),
            Some((2, 1))
        );
        for flag in ["compare_timeout", "truncated", "overflow"] {
            let mut bad = gitlab.clone();
            bad[flag] = serde_json::json!(true);
            assert_eq!(super::comparison_stats("gitlab", &before, &bad, None), None);
        }
        let missing = serde_json::json!({"files":[{"status":"modified"}]});
        assert_eq!(
            super::comparison_stats("gitea", &before, &missing, None),
            None
        );
    }

    use super::*;
    #[tokio::test]
    async fn validation_errors_retain_identity_and_render_for_each_reader() {
        let error = parse_endpoint("file:///local/repository").unwrap_err();
        assert_eq!(error.0, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(
            error.2.as_ref().unwrap().key,
            "repository.auth.endpointFormat"
        );
        let stored = error.stored();
        for locale in crate::i18n::Locale::ALL {
            crate::i18n::with_locale(locale, async {
                assert_eq!(crate::i18n::display_stored(&stored), error.render(locale));
                assert!(!error.render(locale).contains("repository.auth."));
            })
            .await;
        }
        assert_ne!(
            error.render(crate::i18n::Locale::EnUs),
            error.render(crate::i18n::Locale::JaJp)
        );
    }

    #[test]
    fn malformed_tokens_cannot_be_persisted_as_permanent_credentials() {
        assert_eq!(
            token_data(&json!({"access_token":"opaque-token"}))
                .unwrap()
                .1,
            0
        );
        assert!(
            token_data(&json!({"access_token":"opaque-token","expires_in":"3600"}))
                .unwrap()
                .1
                > db::now()
        );
        for value in [
            json!({"access_token":"a\npassword=another"}),
            json!({"access_token":"a","expires_in":-1}),
            json!({"access_token":"a","expires_in":"invalid"}),
            json!({"access_token":"a","expires_in":u64::MAX}),
            json!({"access_token":"a","refresh_token":"b\r\n"}),
        ] {
            assert!(token_data(&value).is_err());
        }
    }
}
