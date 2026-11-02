use super::*;

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
fn flow_aad(workspace: Id, id: Id) -> String {
    format!("repository-oauth:{workspace}:{id}")
}
pub async fn list(State(s): State<AppState>, scope: Scope) -> Result<Json<Value>> {
    scope.require("credentials.read")?;
    let mut rows = Vec::new();
    let settings = OAuthSettings::load(&s, &s.db).await?;
    for kind in ["github", "gitlab", "gitee", "gitea", "codeup"] {
        let row = connections::Entity::find()
            .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(connections::Column::UserId.eq(scope.user.id))
            .filter(connections::Column::Provider.eq(kind))
            .filter(connections::Column::Name.eq(""))
            .one(&s.db)
            .await?;
        let configured = settings.provider(kind).is_ok() && settings.callback(&s).is_ok();
        let state = row
            .as_ref()
            .map(|r| {
                if r.state != "connected" {
                    r.state.as_str()
                } else if r.expires_at > 0 && r.expires_at <= db::now() {
                    if r.refresh_secret.is_empty() {
                        "expired"
                    } else {
                        "refreshable"
                    }
                } else {
                    r.state.as_str()
                }
            })
            .unwrap_or("not-connected");
        let setup = if configured {
            String::new()
        } else {
            crate::i18n::render(
                crate::i18n::current_locale(),
                &crate::i18n::Message::new(if kind == "codeup" {
                    "repository.auth.setupCodeup"
                } else {
                    "repository.auth.setupOAuth"
                }),
            )
        };
        rows.push(json!({"id":kind,"provider":kind,"web_available":configured,"state":state,"credential_id":row.as_ref().map(|r|Id::from(r.credential_id)),"account":row.as_ref().map(|r|r.account.as_str()).unwrap_or(""),"expires_at":row.as_ref().map(|r|r.expires_at).unwrap_or(0),"last_error":crate::i18n::display_stored(row.as_ref().map(|r|r.last_error.as_str()).unwrap_or("")),"setup":setup}));
    }
    Ok(Json(
        json!({"items":rows,"total":5,"page":1,"page_size":20}),
    ))
}
#[derive(Default, Deserialize)]
pub struct StartOptions {
    #[serde(default)]
    popup: bool,
    #[serde(default)]
    channel: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    connection_id: String,
}
pub async fn start(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, kind)): Path<(String, String)>,
    Query(options): Query<StartOptions>,
) -> Result<Json<Value>> {
    scope.require("credentials.create")?;
    if options.channel.len() > 64
        || !options
            .channel
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.popupInvalid"),
        ));
    }
    if !options.name.is_empty() {
        validate_connection_name(&options.name, &options.description)?;
        scope.require("repositories.create")?;
    }
    let target = Id::reference(&options.connection_id)?;
    if target != Id::from(0) {
        scope.require("repositories.update")?;
        let row = owned_connection(&s.db, &scope, target).await?;
        if row.provider != kind
            || row.name.is_empty()
            || options.name.is_empty()
            || row.token_config.is_some()
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.reauthorizeSameProvider"),
            ));
        }
    }
    let settings = OAuthSettings::load(&s, &s.db).await?;
    let config = settings.provider(&kind)?;
    let redirect = settings.callback(&s)?;
    let state = random_token();
    let verifier = random_token();
    let id = s.next_id()?;
    let now = db::now();
    let proof = json!({"verifier":verifier,"redirect":redirect,"client_id":config.client_id,"base":config.base.as_str(),"config":config_fingerprint(&config),"popup":options.popup,"channel":options.channel,"name":options.name,"description":options.description,"connection_id":options.connection_id,"locale":crate::i18n::current_locale().tag()});
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    flows::Entity::delete_many()
        .filter(flows::Column::ExpiresAt.lt(now))
        .exec(&tx)
        .await?;
    flows::Entity::insert(flows::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(scope.workspace_id.into()),
        user_id: Set(scope.user.id.into()),
        provider: Set(kind.clone()),
        state_hash: Set(hash(&state)),
        proof: Set(s
            .vault
            .seal(&proof.to_string(), &flow_aad(scope.workspace_id, id))?),
        expires_at: Set(now + 600_000),
        version: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(&tx)
    .await?;
    tx.commit().await?;
    let mut url = endpoint(
        &config.base,
        if kind == "github" {
            "login/oauth/authorize"
        } else if kind == "gitea" {
            "login/oauth/authorize"
        } else {
            "oauth/authorize"
        },
    )?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("client_id", &config.client_id)
            .append_pair("redirect_uri", &redirect)
            .append_pair("response_type", "code")
            .append_pair("state", &state)
            .append_pair(
                "scope",
                match kind.as_str() {
                    "github" => "repo read:user",
                    "gitlab" => "read_user read_api read_repository",
                    "gitea" => "read:user read:repository",
                    _ => "user_info projects",
                },
            );
        if kind != "gitee" {
            query
                .append_pair("code_challenge_method", "S256")
                .append_pair(
                    "code_challenge",
                    &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
                );
        }
    }
    Ok(Json(json!({"url":url.as_str()})))
}
fn oauth_return_path(workspace: Id, provider: &str, proof: &Value) -> String {
    // The callback's browser/provider headers do not identify the initiating
    // UI language. Restore it only from the authenticated encrypted proof.
    let locale = proof["locale"]
        .as_str()
        .and_then(crate::i18n::Locale::from_tag)
        .unwrap_or_default();
    if proof["popup"].as_bool() == Some(true) {
        format!(
            "/{}/workspaces/{workspace}/code-connections/new?oauth_popup=1&provider={provider}&oauth_channel={}",
            locale.tag(),
            proof["channel"].as_str().unwrap_or("")
        )
    } else {
        format!(
            "/{}/workspaces/{workspace}/code-connections/new?provider={provider}",
            locale.tag()
        )
    }
}

#[derive(Deserialize)]
pub struct Callback {
    #[serde(default)]
    state: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    error: String,
}
pub async fn complete(
    State(s): State<AppState>,
    account: crate::accounts::auth::Account,
    Query(b): Query<Callback>,
) -> Result<Redirect> {
    let pending = flows::Entity::find()
        .filter(flows::Column::StateHash.eq(hash(&b.state)))
        .filter(flows::Column::UserId.eq(account.user.id))
        .filter(flows::Column::ExpiresAt.gt(db::now()))
        .one(&s.db)
        .await?
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.stateExpired"),
            )
        })?;
    let scope = Scope::for_actor(&s, pending.workspace_id.into(), account.user.id).await?;
    scope.require("credentials.create")?;
    let workspace = scope.workspace_id;
    let flow = {
        let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
        let row = flows::Entity::find()
            .filter(flows::Column::StateHash.eq(hash(&b.state)))
            .filter(flows::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(flows::Column::UserId.eq(scope.user.id))
            .filter(flows::Column::ExpiresAt.gt(db::now()))
            .one(&tx)
            .await?
            .ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.auth.stateExpired"),
                )
            })?;
        flows::Entity::delete_by_id(row.id).exec(&tx).await?;
        tx.commit().await?;
        row
    };
    let proof: Value = serde_json::from_str(
        &s.vault
            .open_secret(&flow.proof, &flow_aad(scope.workspace_id, flow.id.into()))?,
    )
    .map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.stateCorrupt"),
        )
    })?;
    let return_path = oauth_return_path(workspace, &flow.provider, &proof);
    if !b.error.is_empty() || b.code.is_empty() {
        return Ok(Redirect::to(&format!(
            "{return_path}&authorization=cancelled"
        )));
    }
    let settings = OAuthSettings::load(&s, &s.db).await?;
    let config = settings.provider(&flow.provider)?;
    if proof["redirect"].as_str() != Some(settings.callback(&s)?.as_str())
        || proof["client_id"].as_str() != Some(&config.client_id)
        || proof["base"].as_str() != Some(config.base.as_str())
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.oauthConfigChanged"),
        ));
    }
    require_same_config(&config, proof["config"].as_str().unwrap_or(""))?;
    let mut form = vec![
        ("client_id", config.client_id.as_str()),
        ("client_secret", config.client_secret.as_str()),
        ("code", b.code.as_str()),
        ("redirect_uri", proof["redirect"].as_str().unwrap_or("")),
        ("grant_type", "authorization_code"),
    ];
    if flow.provider != "gitee" {
        form.push(("code_verifier", proof["verifier"].as_str().unwrap_or("")));
    }
    let client = http()?;
    let tokens = response_json(
        client
            .post(endpoint(
                &config.base,
                if matches!(flow.provider.as_str(), "github" | "gitea") {
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
                    crate::i18n::Message::new("repository.auth.exchangeFailed"),
                )
            })?,
    )
    .await?;
    let (access, expires, refresh) = token_data(&tokens)?;
    let mut request = client.get(endpoint(&config.api, "user")?);
    if flow.provider == "gitee" {
        request = request.query(&[("access_token", access)]);
    } else {
        request = request.bearer_auth(access);
    }
    let account = response_json(request.send().await.map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.accountRead"),
        )
    })?)
    .await?;
    let account = account
        .get("login")
        .or_else(|| account.get("username"))
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() < 255)
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.auth.accountMissing"),
            )
        })?;
    let connection_name = proof["name"].as_str().unwrap_or("");
    let target = Id::reference(proof["connection_id"].as_str().unwrap_or(""))?;
    let (connection_id, _) = save_connection(
        &s,
        &scope,
        &flow.provider,
        &config,
        account,
        access,
        expires,
        refresh,
        None,
        if connection_name.is_empty() {
            None
        } else {
            Some((
                connection_name,
                proof["description"].as_str().unwrap_or(""),
                if target == Id::from(0) {
                    None
                } else {
                    Some(target)
                },
            ))
        },
    )
    .await?;
    Ok(Redirect::to(&format!(
        "{return_path}&authorization=connected&connection_id={connection_id}"
    )))
}

pub async fn connect_token(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, kind)): Path<(String, String)>,
    Json(body): Json<TokenConnect>,
) -> Result<Json<Value>> {
    scope.require("credentials.create")?;
    scope.require("repositories.create")?;
    let settings = token_config(&kind, &body)?;
    let config = settings.provider()?;
    let account = if kind == "codeup" {
        body.username.trim().to_owned()
    } else {
        let value = response_json(
            authorize_request(
                http()?.get(endpoint(&config.api, "user")?),
                &kind,
                &body.token,
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
    // Do not save a credential or mark connected unless repository access also succeeds.
    let mut result =
        remote_repositories(&kind, &config, &body.token, true, &settings.organization, 1).await?;
    let stored = serde_json::to_string(&settings).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.auth.connectionConfigEncode"),
        )
    })?;
    let (_, id) = save_connection(
        &s,
        &scope,
        &kind,
        &config,
        &account,
        &body.token,
        0,
        None,
        Some(stored),
        None,
    )
    .await?;
    result["credential_id"] = json!(id);
    result["account"] = json!(account);
    Ok(Json(result))
}
pub async fn recover(s: &AppState) -> Result<()> {
    let interrupted = connections::Entity::find()
        .filter(connections::Column::State.eq("refreshing"))
        .all(&s.db)
        .await?;
    connections::Entity::update_many()
        .col_expr(connections::Column::State, Expr::value("reauthorize"))
        .col_expr(
            connections::Column::LastError,
            Expr::value(crate::i18n::store_message(&crate::i18n::Message::new(
                "repository.authorization.interrupted",
            ))),
        )
        .filter(connections::Column::State.eq("refreshing"))
        .exec(&s.db)
        .await?;
    for connection in interrupted {
        let tx = crate::variables::transaction(s, connection.workspace_id.into()).await?;
        crate::repositories::analysis::connection_changed(
            s,
            &tx,
            connection.workspace_id,
            connection.user_id,
            connection.id,
        )
        .await?;
        tx.commit().await?;
    }
    flows::Entity::delete_many()
        .filter(flows::Column::ExpiresAt.lt(db::now()))
        .exec(&s.db)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oauth_return_restores_initiating_locale_without_localizing_callback_routes() {
        let workspace = Id::from(7);
        assert_eq!(
            oauth_return_path(workspace, "github", &json!({"locale":"ja-JP"})),
            "/ja-JP/workspaces/7/code-connections/new?provider=github"
        );
        assert_eq!(
            oauth_return_path(
                workspace,
                "gitlab",
                &json!({"locale":"zh-Hant","popup":true,"channel":"test-123"})
            ),
            "/zh-TW/workspaces/7/code-connections/new?oauth_popup=1&provider=gitlab&oauth_channel=test-123"
        );
        for proof in [json!({}), json!({"locale":"//invalid/path"})] {
            assert_eq!(
                oauth_return_path(workspace, "gitee", &proof),
                "/en-US/workspaces/7/code-connections/new?provider=gitee"
            );
        }
    }
}
