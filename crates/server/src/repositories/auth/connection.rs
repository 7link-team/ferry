use super::*;

fn validate_anonymous_git_address(address: &str) -> Result<()> {
    if address.starts_with("git://") {
        let url = reqwest::Url::parse(address).map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.gitAddressInvalid"),
            )
        })?;
        if url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || address.chars().any(char::is_control)
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.gitAddressCredentials"),
            ));
        }
        Ok(())
    } else {
        parse_endpoint(address).map(|_| ())
    }
}
pub(super) async fn save_connection(
    s: &AppState,
    scope: &Scope,
    kind: &str,
    config: &Provider,
    account: &str,
    access: &str,
    expires: i64,
    refresh: Option<&str>,
    token_config: Option<String>,
    named: Option<(&str, &str, Option<Id>)>,
) -> Result<(Id, Id)> {
    let fresh = Scope::for_actor(&s, scope.workspace_id, scope.user.id).await?;
    fresh.require("credentials.create")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let now = db::now();
    let existing = match named {
        Some((_, _, Some(id))) => {
            fresh.require("repositories.update")?;
            let row = owned_connection(&tx, &fresh, id).await?;
            if row.provider != kind || row.token_config.is_some() {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.auth.authorizationTypeChanged"),
                ));
            }
            Some(row)
        }
        Some(_) => {
            fresh.require("repositories.create")?;
            None
        }
        None => {
            connections::Entity::find()
                .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(connections::Column::UserId.eq(scope.user.id))
                .filter(connections::Column::Provider.eq(kind))
                .filter(connections::Column::Name.eq(""))
                .one(&tx)
                .await?
        }
    };
    let id = existing
        .as_ref()
        .map(|r| Id::from(r.id))
        .unwrap_or(s.next_id()?);
    let credential = existing
        .as_ref()
        .map(|r| Id::from(r.credential_id))
        .unwrap_or(s.next_id()?);
    let sealed = s.vault.seal(
        access,
        &vault::aad(
            &scope.workspace_id.to_string(),
            "credentials",
            &credential.to_string(),
        ),
    )?;
    if existing.is_some() {
        credentials::Entity::update_many()
            .col_expr(credentials::Column::Secret, Expr::value(sealed))
            .col_expr(credentials::Column::Username, Expr::value(account))
            .col_expr(credentials::Column::UpdatedAt, Expr::value(now))
            .col_expr(
                credentials::Column::Version,
                Expr::col(credentials::Column::Version).add(1),
            )
            .filter(credentials::Column::Id.eq(credential))
            .filter(credentials::Column::WorkspaceId.eq(scope.workspace_id))
            .exec(&tx)
            .await?;
        connections::Entity::delete_by_id(i64::from(id))
            .exec(&tx)
            .await?;
    } else {
        credentials::Entity::insert(credentials::ActiveModel {
            id: Set(credential.into()),
            workspace_id: Set(scope.workspace_id.into()),
            name: Set(format!("{} · {}", kind, account).chars().take(60).collect()),
            kind: Set("token".into()),
            scope: Set("build".into()),
            username: Set(account.into()),
            secret: Set(sealed),
            description: Set(crate::i18n::render(
                crate::i18n::current_locale(),
                &crate::i18n::Message::new("repository.auth.managedCredentialDescription"),
            )),
            version: Set(1),
            created_at: Set(now),
            updated_at: Set(now),
        })
        .exec(&tx)
        .await?;
    }
    connections::Entity::insert(connections::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(scope.workspace_id.into()),
        user_id: Set(scope.user.id.into()),
        provider: Set(kind.to_owned()),
        name: Set(named.map(|v| v.0.trim().to_owned()).unwrap_or_default()),
        description: Set(named.map(|v| v.1.trim().to_owned()).unwrap_or_default()),
        credential_id: Set(credential.into()),
        account: Set(account.into()),
        refresh_secret: Set(match refresh {
            Some(v) => s.vault.seal(v, &connection_aad(scope.workspace_id, id))?,
            None => String::new(),
        }),
        expires_at: Set(expires),
        state: Set("connected".into()),
        last_error: Set(String::new()),
        config_fingerprint: Set(config_fingerprint(config)),
        client_id: Set(config.client_id.clone()),
        token_config: Set(token_config),
        version: Set(existing.as_ref().map(|r| r.version + 1).unwrap_or(1)),
        created_at: Set(existing.map(|r| r.created_at).unwrap_or(now)),
        updated_at: Set(now),
    })
    .exec(&tx)
    .await?;
    crate::repositories::analysis::connection_changed(
        &s,
        &tx,
        scope.workspace_id.into(),
        scope.user.id.into(),
        id.into(),
    )
    .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &fresh,
        "authorize",
        "credentials",
        &credential,
        json!({"connection_id":id,"account":account}),
    )
    .await?;
    tx.commit().await?;
    Ok((id, credential))
}

#[derive(Deserialize)]
pub struct RepositoryPage {
    #[serde(default = "first_page")]
    page: u32,
}
fn first_page() -> u32 {
    1
}
pub async fn browse_repositories(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, kind)): Path<(String, String)>,
    Query(page): Query<RepositoryPage>,
) -> Result<Json<Value>> {
    scope.require("credentials.read")?;
    scope.require("repositories.create")?;
    if page.page == 0 || page.page > 10000 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.pageInvalid"),
        ));
    }
    let row = connections::Entity::find()
        .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(connections::Column::UserId.eq(scope.user.id))
        .filter(connections::Column::Provider.eq(&kind))
        .filter(connections::Column::Name.eq(""))
        .one(&s.db)
        .await?
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.connectFirst"),
            )
        })?;
    let config = connection_provider(&s, &s.db, &row).await?;
    ensure_fresh(&s, scope.workspace_id, row.credential_id.into()).await?;
    let (_, _, secret) =
        crate::credentials::unseal(&s, &scope.workspace_id, &row.credential_id.into()).await?;
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
    let mut result = remote_repositories(
        &kind,
        &config,
        &secret,
        token.is_some(),
        token
            .as_ref()
            .map(|c| c.organization.as_str())
            .unwrap_or(""),
        page.page,
    )
    .await?;
    result["credential_id"] = json!(Id::from(row.credential_id));
    result["account"] = json!(row.account);
    Ok(Json(result))
}

/// A web-authorized credential is usable only with its configured provider.
pub async fn validate_target(
    s: &AppState,
    conn: &impl sea_orm::ConnectionTrait,
    workspace: Id,
    credential: Id,
    address: &str,
    api: bool,
) -> Result<()> {
    let Some(row) = connections::Entity::find()
        .filter(connections::Column::WorkspaceId.eq(workspace))
        .filter(connections::Column::CredentialId.eq(credential))
        .filter(connections::Column::Provider.ne("git"))
        .filter(
            sea_orm::Condition::any()
                .add(connections::Column::Name.eq(""))
                .add(connections::Column::TokenConfig.is_null()),
        )
        .one(conn)
        .await?
    else {
        return Ok(());
    };
    let config = connection_provider(s, conn, &row).await?;
    let expected = if api { config.api } else { config.base };
    let address =
        if let Some((host, path)) = address.strip_prefix("git@").and_then(|v| v.split_once(':')) {
            format!("ssh://git@{host}/{path}")
        } else {
            address.to_owned()
        };
    let actual = reqwest::Url::parse(&address).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.repositoryUrlInvalid"),
        )
    })?;
    if actual.host_str() != expected.host_str()
        || (actual.scheme() != "ssh"
            && actual.port_or_known_default() != expected.port_or_known_default())
        || (expected.scheme() == "https" && actual.scheme() == "http")
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.oauthScopeMismatch"),
        ));
    }
    Ok(())
}

const CONNECTION_SPEC: crate::resources::resource::Spec = crate::resources::resource::Spec {
    search: &["name", "account"],
    filters: &["provider", "state"],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "name",
};

fn connection_json(row: &connections::Model) -> Value {
    let config: Value = row
        .token_config
        .as_deref()
        .and_then(|v| serde_json::from_str(v).ok())
        .unwrap_or(json!({}));
    let mut value = json!({"id":Id::from(row.id),"name":row.name,"provider":row.provider,
        "description":row.description,"credential_id":Id::from(row.credential_id),
        "account":row.account,"state":row.state,"expires_at":row.expires_at,
        "last_error":crate::i18n::display_stored(&row.last_error),"authorization":if row.token_config.is_none(){"oauth"}else{"credential"},
        "base_url":config["base_url"].as_str().unwrap_or(""),
        "api_url":config["api_url"].as_str().unwrap_or(""),
        "organization":config["organization"].as_str().unwrap_or(""),
        "version":row.version,"created_at":row.created_at,"updated_at":row.updated_at});
    if row.provider == "git" {
        value["url"] = json!(config["url"].as_str().unwrap_or(""));
    }
    value
}

pub(crate) async fn owned_connection(
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    id: Id,
) -> Result<connections::Model> {
    connections::Entity::find_by_id(i64::from(id))
        .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(connections::Column::UserId.eq(scope.user.id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}

pub(super) fn validate_connection_name(name: &str, description: &str) -> Result<()> {
    if name.trim().is_empty()
        || name.chars().count() > 60
        || name.chars().any(char::is_control)
        || description.chars().count() > 255
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.connectionNameInvalid"),
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct ConnectionBody {
    name: String,
    provider: String,
    #[serde(default)]
    credential_id: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    api_url: String,
    #[serde(default)]
    organization: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    version: i64,
}

pub async fn list_connections(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("repositories.read")?;
    let query = crate::resources::resource::ListQuery::parse(&CONNECTION_SPEC, &raw)?;
    let (rows, total) =
        crate::resources::resource::list::<connections::Entity, connections::Model>(
            &s.db,
            &CONNECTION_SPEC,
            connections::Entity::find()
                .filter(connections::Column::UserId.eq(scope.user.id))
                .filter(connections::Column::Name.ne("")),
            &scope.workspace_id,
            &query,
        )
        .await?;
    let recent = crate::repositories::monitor::latest_commit_events(
        &s,
        scope.workspace_id,
        rows.iter().map(|row| row.id).collect(),
    )
    .await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|row| {
            let mut item = connection_json(row);
            item["recent_commit_event"] = recent.get(&row.id).cloned().unwrap_or(Value::Null);
            item
        })
        .collect();
    Ok(Json(
        json!({"items":items,"total":total,"page":query.page,"page_size":query.page_size}),
    ))
}

pub async fn get_connection(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    scope.require("repositories.read")?;
    Ok(Json(connection_json(
        &owned_connection(&s.db, &scope, id.parse().map_err(|_| ApiError::missing())?).await?,
    )))
}

// Validate before opening a write transaction: provider requests never hold the workspace lock.
async fn check_connection(
    s: &AppState,
    scope: &Scope,
    body: &ConnectionBody,
) -> Result<(Id, String, String, String)> {
    validate_connection_name(&body.name, &body.description)?;
    scope.require("credentials.read")?;
    let credential = Id::reference(&body.credential_id)?;
    if body.provider == "git" && credential == Id::from(0) {
        if !body.url.is_empty() {
            parse_endpoint(&body.url)?;
        }
        return Ok((
            credential,
            String::new(),
            json!({"url":body.url}).to_string(),
            String::new(),
        ));
    }
    let (kind, username, secret) =
        crate::credentials::unseal(s, &scope.workspace_id, &credential).await?;
    // An OAuth credential belongs to its authorizing user and must retain its refresh owner.
    let managed = connections::Entity::find()
        .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(connections::Column::CredentialId.eq(credential))
        .filter(connections::Column::TokenConfig.is_null())
        .one(&s.db)
        .await?;
    if managed.is_some() {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.oauthCredentialFlow"),
        ));
    }
    if body.provider == "git" {
        if !crate::credentials::KINDS.contains(&kind.as_str()) {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.gitCredentialRequired"),
            ));
        }
        if !body.url.is_empty() {
            validate_git_credential(&body.url, &kind)?;
        }
        return Ok((
            credential,
            username,
            json!({"url":body.url}).to_string(),
            String::new(),
        ));
    }
    if kind != "token" {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.apiCredentialRequired"),
        ));
    }
    let settings = token_config(
        &body.provider,
        &TokenConnect {
            token: secret.clone(),
            username: username.clone(),
            base_url: body.base_url.clone(),
            api_url: body.api_url.clone(),
            organization: body.organization.clone(),
        },
    )?;
    let provider = settings.provider()?;
    let account = if body.provider == "codeup" {
        username
    } else {
        let value = response_json(
            authorize_request(
                http()?.get(endpoint(&provider.api, "user")?),
                &body.provider,
                &secret,
                true,
            )
            .send()
            .await
            .map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.auth.tokenVerify"),
                )
            })?,
        )
        .await?;
        value
            .get("login")
            .or_else(|| value.get("username"))
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty() && v.len() < 255)
            .ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.auth.platformAccountMissing"),
                )
            })?
            .to_owned()
    };
    remote_repositories(
        &body.provider,
        &provider,
        &secret,
        true,
        &settings.organization,
        1,
    )
    .await?;
    Ok((
        credential,
        account,
        serde_json::to_string(&settings).map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.connectionConfigEncode"),
            )
        })?,
        config_fingerprint(&provider),
    ))
}

pub async fn create_connection(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<ConnectionBody>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("repositories.create")?;
    let (credential, account, config, fingerprint) = check_connection(&s, &scope, &body).await?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    // The credential could have been deleted while the remote API was responding.
    require_credential_in(&tx, &scope, credential).await?;
    let id = s.next_id()?;
    let now = db::now();
    connections::Entity::insert(connections::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(scope.workspace_id.into()),
        user_id: Set(scope.user.id.into()),
        name: Set(body.name.trim().into()),
        description: Set(body.description.trim().into()),
        provider: Set(body.provider),
        credential_id: Set(credential.into()),
        account: Set(account),
        refresh_secret: Set(String::new()),
        expires_at: Set(0),
        state: Set("connected".into()),
        last_error: Set(String::new()),
        client_id: Set(String::new()),
        config_fingerprint: Set(fingerprint),
        token_config: Set(Some(config)),
        version: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(&tx)
    .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "create",
        "repositories",
        &id,
        json!({"connection":true,"name":body.name}),
    )
    .await?;
    let row = owned_connection(&tx, &scope, id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(connection_json(&row))))
}

async fn require_credential_in(
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    credential: Id,
) -> Result<()> {
    if credential == Id::from(0) {
        return Ok(());
    }
    credentials::Entity::find_by_id(i64::from(credential))
        .filter(credentials::Column::WorkspaceId.eq(scope.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.credentialMissing"),
            )
        })?;
    Ok(())
}

pub async fn update_connection(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Json(body): Json<ConnectionBody>,
) -> Result<Json<Value>> {
    scope.require("repositories.update")?;
    let id: Id = id.parse().map_err(|_| ApiError::missing())?;
    let existing = owned_connection(&s.db, &scope, id).await?;
    validate_connection_name(&body.name, &body.description)?;
    let replacement = if existing.token_config.is_none() {
        if body.provider != existing.provider
            || Id::reference(&body.credential_id)? != Id::from(existing.credential_id)
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.oauthAccountChange"),
            ));
        }
        None
    } else {
        Some(check_connection(&s, &scope, &body).await?)
    };
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut update = connections::Entity::update_many()
        .col_expr(connections::Column::Name, Expr::value(body.name.trim()))
        .col_expr(
            connections::Column::Description,
            Expr::value(body.description.trim()),
        )
        .col_expr(connections::Column::UpdatedAt, Expr::value(db::now()))
        .col_expr(
            connections::Column::Version,
            Expr::col(connections::Column::Version).add(1),
        )
        .filter(connections::Column::Id.eq(id))
        .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(connections::Column::UserId.eq(scope.user.id))
        .filter(connections::Column::Version.eq(body.version));
    if let Some((credential, account, config, fingerprint)) = replacement {
        require_credential_in(&tx, &scope, credential).await?;
        update = update
            .col_expr(connections::Column::Provider, Expr::value(body.provider))
            .col_expr(
                connections::Column::CredentialId,
                Expr::value(i64::from(credential)),
            )
            .col_expr(connections::Column::Account, Expr::value(account))
            .col_expr(connections::Column::TokenConfig, Expr::value(config))
            .col_expr(
                connections::Column::ConfigFingerprint,
                Expr::value(fingerprint),
            );
    }
    if update.exec(&tx).await?.rows_affected == 0 {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("repository.auth.connectionChanged"),
        ));
    }
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "repositories",
        &id,
        json!({"connection":true}),
    )
    .await?;
    let row = owned_connection(&tx, &scope, id).await?;
    tx.commit().await?;
    Ok(Json(connection_json(&row)))
}

pub async fn delete_connection(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<StatusCode> {
    scope.require("repositories.delete")?;
    let id: Id = id.parse().map_err(|_| ApiError::missing())?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let row = owned_connection(&tx, &scope, id).await?;
    let pipelines = crate::entities::pipelines::Entity::find()
        .filter(crate::entities::pipelines::Column::WorkspaceId.eq(scope.workspace_id))
        .all(&tx)
        .await?;
    for pipeline in pipelines {
        if crate::pipelines::actions::document(&pipeline.definition).is_some_and(|document| {
            document
                .checkout_bindings
                .iter()
                .any(|binding| Id::reference(&binding.connection_id).is_ok_and(|value| value == id))
        }) {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("repository.auth.sourceReference"),
            ));
        }
    }
    for run in crate::entities::pipeline_runs::Entity::find()
        .filter(crate::entities::pipeline_runs::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(crate::entities::pipeline_runs::Column::State.is_in([
            "queued",
            "running",
            "cancelling",
        ]))
        .all(&tx)
        .await?
    {
        if crate::pipelines::actions::snapshot(&run.definition).is_some_and(|snapshot| {
            snapshot.checkouts.iter().any(|checkout| {
                Id::reference(&checkout.binding.connection_id).is_ok_and(|value| value == id)
            })
        }) {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("repository.auth.pipelineRunning"),
            ));
        }
    }
    // OAuth refresh ownership is retained while an existing source binding uses it.
    if row.token_config.is_none() {
        use sea_orm::PaginatorTrait;
        let used = crate::entities::repositories::Entity::find()
            .filter(crate::entities::repositories::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(crate::entities::repositories::Column::CredentialId.eq(row.credential_id))
            .count(&tx)
            .await?;
        if used > 0 {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("repository.auth.repositoryReference"),
            ));
        }
    }
    crate::entities::repository_monitors::Entity::delete_by_id(row.id)
        .exec(&tx)
        .await?;
    crate::entities::repository_poll_states::Entity::delete_by_id(row.id)
        .exec(&tx)
        .await?;
    crate::entities::repository_events::Entity::delete_many()
        .filter(crate::entities::repository_events::Column::ConnectionId.eq(row.id))
        .exec(&tx)
        .await?;
    connections::Entity::delete_by_id(row.id).exec(&tx).await?;
    crate::repositories::analysis::connection_changed(
        &s,
        &tx,
        row.workspace_id,
        row.user_id,
        row.id,
    )
    .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "delete",
        "repositories",
        &id,
        json!({"connection":true}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn connection_repositories(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Query(page): Query<RepositoryPage>,
) -> Result<Json<Value>> {
    scope.require("repositories.read")?;
    scope.require("credentials.read")?;
    if page.page == 0 || page.page > 10000 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.pageInvalid"),
        ));
    }
    let row = owned_connection(&s.db, &scope, id.parse().map_err(|_| ApiError::missing())?).await?;
    if row.provider == "git" {
        return Ok(Json(json!({"items":[],"page":page.page,"has_more":false})));
    }
    let config = connection_provider(&s, &s.db, &row).await?;
    ensure_fresh(&s, scope.workspace_id, row.credential_id.into()).await?;
    let (_, _, secret) =
        crate::credentials::unseal(&s, &scope.workspace_id, &row.credential_id.into()).await?;
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
    let mut result = remote_repositories(
        &row.provider,
        &config,
        &secret,
        token.is_some(),
        token
            .as_ref()
            .map(|v| v.organization.as_str())
            .unwrap_or(""),
        page.page,
    )
    .await?;
    result["connection_id"] = json!(Id::from(row.id));
    Ok(Json(result))
}

fn validate_git_credential(address: &str, kind: &str) -> Result<()> {
    let ssh = address.starts_with("git@") || address.starts_with("ssh://");
    if (kind == "ssh-key") != ssh {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.gitCredentialScheme"),
        ));
    }
    if ssh {
        let normalized = address
            .strip_prefix("git@")
            .and_then(|v| v.split_once(':'))
            .map(|(host, path)| format!("ssh://git@{host}/{path}"))
            .unwrap_or_else(|| address.to_owned());
        let parsed = reqwest::Url::parse(&normalized).map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.gitAddressInvalid"),
            )
        })?;
        if parsed.host_str().is_none()
            || parsed.password().is_some()
            || address.chars().any(char::is_control)
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.gitAddressInvalid"),
            ));
        }
    } else {
        parse_endpoint(address)?;
    }
    Ok(())
}

/// Resolve a selected connection without trusting client-supplied credential IDs.
pub async fn resolve_connection(
    s: &AppState,
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    connection_id: Id,
    address: &str,
) -> Result<(String, Id)> {
    let row = owned_connection(conn, scope, connection_id).await?;
    resolve_connection_row(s, conn, scope, row, address).await
}

/// Execution of a saved binding follows workspace permissions; editing still requires ownership.
pub async fn resolve_saved_connection(
    s: &AppState,
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    connection_id: Id,
    address: &str,
) -> Result<(String, Id)> {
    let row = connections::Entity::find_by_id(i64::from(connection_id))
        .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)?;
    resolve_connection_row(s, conn, scope, row, address).await
}

async fn resolve_connection_row(
    s: &AppState,
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    row: connections::Model,
    address: &str,
) -> Result<(String, Id)> {
    if row.state != "connected" {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.connectionAuthorizationInvalid"),
        ));
    }
    let credential = Id::from(row.credential_id);
    if credential != Id::from(0) {
        let auth = credentials::Entity::find_by_id(row.credential_id)
            .filter(credentials::Column::WorkspaceId.eq(scope.workspace_id))
            .one(conn)
            .await?
            .ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.auth.credentialMissing"),
                )
            })?;
        validate_git_credential(address, &auth.kind)?;
    } else if row.provider == "git" {
        validate_anonymous_git_address(address)?;
    } else {
        parse_endpoint(address)?;
    }
    if row.provider != "git" {
        let config = connection_provider(s, conn, &row).await?;
        validate_origin(&config.base, address)?;
    }
    Ok((row.provider, credential))
}

fn validate_origin(expected: &reqwest::Url, address: &str) -> Result<()> {
    let actual = parse_endpoint(address)?;
    if actual.host_str() != expected.host_str()
        || actual.port_or_known_default() != expected.port_or_known_default()
        || (expected.scheme() == "https" && actual.scheme() != "https")
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.selectedProviderMismatch"),
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct BranchQuery {
    url: String,
    #[serde(default = "first_page")]
    page: u32,
}

pub async fn connection_branches(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Query(query): Query<BranchQuery>,
) -> Result<Json<Value>> {
    scope.require("repositories.read")?;
    scope.require("credentials.read")?;
    if query.page == 0 || query.page > 10000 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.pageInvalid"),
        ));
    }
    let id: Id = id.parse().map_err(|_| ApiError::missing())?;
    resolve_connection(&s, &s.db, &scope, id, &query.url).await?;
    let row = owned_connection(&s.db, &scope, id).await?;
    if row.provider == "git" {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.gitRefRequired"),
        ));
    }
    let config = connection_provider(&s, &s.db, &row).await?;
    let source = parse_endpoint(&query.url)?;
    let path = source
        .path()
        .strip_prefix(config.base.path().trim_end_matches('/'))
        .unwrap_or(source.path())
        .trim_start_matches('/')
        .trim_end_matches(".git");
    if path.is_empty() {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.repositoryRequired"),
        ));
    }
    // Url::path is already escaped. Decode before pushing API path segments to
    // avoid turning %E4 into %25E4 for Unicode, spaces and literal percent signs.
    let path = percent_encoding::percent_decode_str(path)
        .decode_utf8()
        .map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.repositoryPathInvalid"),
            )
        })?;
    let path = path.as_ref();
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
    let mut url = config.api.clone();
    {
        let mut segments = url.path_segments_mut().map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.apiUrlInvalid"),
            )
        })?;
        segments.pop_if_empty();
        match row.provider.as_str() {
            "gitlab" => {
                segments
                    .push("projects")
                    .push(path)
                    .push("repository")
                    .push("branches");
            }
            "codeup" => {
                segments.extend(["oapi", "v1", "codeup"]);
                let organization = token
                    .as_ref()
                    .map(|v| v.organization.as_str())
                    .unwrap_or("");
                if !organization.is_empty() {
                    segments.push("organizations").push(organization);
                }
                segments.push("repositories").push(path).push("branches");
            }
            _ => {
                segments
                    .push("repos")
                    .extend(path.split('/'))
                    .push("branches");
            }
        }
    }
    url.query_pairs_mut()
        .append_pair("page", &query.page.to_string())
        .append_pair(
            if row.provider == "codeup" {
                "perPage"
            } else if row.provider == "gitea" {
                "limit"
            } else {
                "per_page"
            },
            "50",
        );
    ensure_fresh(&s, scope.workspace_id, row.credential_id.into()).await?;
    let (_, _, secret) =
        crate::credentials::unseal(&s, &scope.workspace_id, &row.credential_id.into()).await?;
    let response = authorize_request(http()?.get(url), &row.provider, &secret, token.is_some())
        .send()
        .await
        .map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.branchesRead"),
            )
        })?;
    let has_more = (row.provider == "codeup")
        .then(|| codeup_has_more(&response, query.page))
        .flatten();
    let value = repository_response(response, &row.provider).await?;
    let rows = value.as_array().ok_or_else(|| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.branchesInvalid"),
        )
    })?;
    let items = rows
        .iter()
        .filter_map(|row| {
            row.get("name")
                .or_else(|| row.get("branchName"))
                .and_then(Value::as_str)
        })
        .map(|name| json!({"name":name}))
        .collect::<Vec<_>>();
    Ok(Json(
        json!({"items":items,"page":query.page,"has_more":has_more.unwrap_or(rows.len()==50)}),
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn anonymous_git_transport_keeps_platform_endpoints_and_credentials_separate() {
        assert!(validate_anonymous_git_address("git://git.example.test:9418/project.git").is_ok());
        assert!(validate_anonymous_git_address("https://git.example.test/project.git").is_ok());
        for value in [
            "git://reader@git.example.test/project.git",
            "git://git.example.test/project.git?token=secret",
            "git://git.example.test/project.git#ref",
            "file:///tmp/project.git",
            "git://git.example.test/proj\nect.git",
        ] {
            assert!(validate_anonymous_git_address(value).is_err(), "{value}");
        }
        assert!(parse_endpoint("git://git.example.test/project.git").is_err());
    }
}
