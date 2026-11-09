//! Runner credentials, enrollment tokens, and host connection commands.
use super::*;

/// Enrolment token lifetime, in minutes, and the bounds an operator may pick.
const TOKEN_MINUTES: i64 = 30;
const TOKEN_MINUTES_RANGE: std::ops::RangeInclusive<i64> = 5..=1440;

// ------------------------------------------------------------------ 凭据

/// A runner key and an enrolment token are both 256 bits of randomness, so the
/// server stores only a SHA-256 of them. Argon2 is for passwords a human
/// chose; stretching a random 256-bit secret buys nothing and would be paid on
/// every reconnect. There is no timing concern in comparing the hashes either:
/// matching one means finding a preimage, not guessing it byte by byte.
pub fn fingerprint(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

pub fn secret() -> String {
    use rand::Rng;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill(&mut bytes[..]);
    hex::encode(bytes)
}

// ------------------------------------------------------------ 注册令牌

pub(crate) async fn bootstrap_token(db: &sea_orm::DatabaseConnection, token: &str) -> Result<()> {
    if token.len() != 64 {
        return Err(ApiError::localized(
            StatusCode::UNAUTHORIZED,
            crate::i18n::Message::new("runner.invalidToken"),
        ));
    }
    let valid = runner_tokens::Entity::find()
        .filter(runner_tokens::Column::TokenHash.eq(fingerprint(token)))
        .filter(runner_tokens::Column::UsedAt.eq(0_i64))
        .filter(runner_tokens::Column::ExpiresAt.gt(db::now()))
        .one(db)
        .await?;
    let persistent = hosts::Entity::find()
        .filter(hosts::Column::EnrollTokenHash.eq(fingerprint(token)))
        .filter(hosts::Column::Connection.eq("runner"))
        .one(db)
        .await?;
    if valid.is_none() && persistent.is_none() {
        return Err(ApiError::localized(
            StatusCode::UNAUTHORIZED,
            crate::i18n::Message::new("runner.expiredToken"),
        ));
    }
    Ok(())
}

fn command_aad(workspace: Id, host: Id) -> String {
    crate::vault::aad(&workspace.to_string(), "host-connection", &host.to_string())
}

async fn command_host(s: &AppState, scope: &Scope, id: Id) -> Result<hosts::Model> {
    scope.require("hosts.enroll")?;
    hosts::Entity::find_by_id(i64::from(id))
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(hosts::Column::Connection.eq("runner"))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)
}

pub async fn connection_command(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    let mut host = command_host(&s, &scope, id).await?;
    let token = if host.enroll_token.is_empty() {
        let token = secret();
        let sealed = s.vault.seal(&token, &command_aad(scope.workspace_id, id))?;
        hosts::Entity::update_many()
            .col_expr(hosts::Column::EnrollToken, Expr::value(sealed))
            .col_expr(
                hosts::Column::EnrollTokenHash,
                Expr::value(fingerprint(&token)),
            )
            .filter(hosts::Column::Id.eq(id))
            .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(hosts::Column::EnrollToken.eq(""))
            .exec(&s.db)
            .await?;
        host = command_host(&s, &scope, id).await?;
        s.vault
            .open_secret(&host.enroll_token, &command_aad(scope.workspace_id, id))?
    } else {
        s.vault
            .open_secret(&host.enroll_token, &command_aad(scope.workspace_id, id))?
    };
    let authority = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    let scheme = if s.secure_cookie { "https" } else { "http" };
    let mut installation_scope = live().get(&id.to_string()).and_then(|live| live.installation_scope.clone());
    if installation_scope.is_none() {
        if let Some(cluster) = &s.cluster {
            installation_scope = cluster.owner(&id.to_string()).await?.and_then(|owner| owner.info).and_then(|info| info.installation_scope);
        }
    }
    Ok(Json(
        json!({ "token": token, "server": format!("{scheme}://{authority}"), "state": host.state, "os": host.os, "installation_scope": installation_scope }),
    ))
}

pub async fn reset_connection_command(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    let host = command_host(&s, &scope, id).await?;
    let previous_owner = match &s.cluster {
        Some(cluster) => cluster.owner(&id.to_string()).await?,
        None => None,
    };
    let token = secret();
    let sealed = s.vault.seal(&token, &command_aad(scope.workspace_id, id))?;
    hosts::Entity::update_many()
        .col_expr(hosts::Column::EnrollToken, Expr::value(sealed))
        .col_expr(
            hosts::Column::EnrollTokenHash,
            Expr::value(fingerprint(&token)),
        )
        .col_expr(hosts::Column::RunnerKeyHash, Expr::value(""))
        .col_expr(hosts::Column::State, Expr::value("offline"))
        .filter(hosts::Column::Id.eq(id))
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .exec(&s.db)
        .await?;
    runner_tokens::Entity::update_many()
        .col_expr(runner_tokens::Column::ExpiresAt, Expr::value(db::now() - 1))
        .filter(runner_tokens::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(runner_tokens::Column::HostId.eq(id))
        .filter(runner_tokens::Column::UsedAt.eq(0_i64))
        .exec(&s.db)
        .await?;
    if let Some(cluster) = &s.cluster {
        cluster
            .revoke(&s, &id.to_string(), previous_owner.as_ref())
            .await?;
    }
    if let Some(registration) = live().remove(&id.to_string()) {
        let _ = registration.disconnect.send(true);
    }
    crate::hosts::activity(
        &s,
        &scope,
        "reset-connection",
        "hosts",
        &id,
        json!({ "name": host.name }),
    )
    .await?;
    connection_command(
        State(s),
        scope,
        Path((String::new(), id.to_string())),
        headers,
    )
    .await
}

pub async fn token_status(
    State(s): State<AppState>,
    scope: Scope,
    Path((_workspace, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    scope.require("hosts.enroll")?;
    let token = runner_tokens::Entity::find_by_id(i64::from(Id::reference(&id)?))
        .filter(runner_tokens::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let host = if token.used_by != 0 {
        hosts::Entity::find_by_id(token.used_by)
            .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .one(&s.db)
            .await?
    } else {
        None
    };
    Ok(Json(json!({
        "id": Id::from(token.id), "expires_at": token.expires_at, "used_at": token.used_at,
        "host_id": if token.used_by != 0 { token.used_by.to_string() } else { String::new() },
        "host_state": host.map(|h| h.state).unwrap_or_default(),
        "state": if token.used_by != 0 { "connected" } else if token.expires_at <= db::now() { "expired" } else { "pending" },
    })))
}

#[derive(Deserialize)]
pub struct TokenBody {
    /// Bind the token to a host the operator already created. Empty means the
    /// runner's own hostname becomes a new host row when it enrols.
    #[serde(default)]
    host_id: String,
    #[serde(default)]
    ttl_minutes: i64,
}

/// `POST .../runner-tokens` — mints the token that goes in the one command the
/// operator runs on the host.
pub async fn create_token(
    State(s): State<AppState>,
    scope: Scope,
    headers: HeaderMap,
    Json(body): Json<TokenBody>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("hosts.enroll")?;
    let minutes = if body.ttl_minutes == 0 {
        TOKEN_MINUTES
    } else if TOKEN_MINUTES_RANGE.contains(&body.ttl_minutes) {
        body.ttl_minutes
    } else {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("runner.tokenDuration"),
        ));
    };
    let host_id = Id::reference(&body.host_id)?;
    if host_id != Id::from(0) {
        // Refuse now rather than burning a token on a host that cannot use it.
        hosts::Entity::find_by_id(i64::from(host_id))
            .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(hosts::Column::Connection.eq("runner"))
            .one(&s.db)
            .await?
            .ok_or_else(|| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("runner.hostMissing"),
                )
            })?;
    }

    let (id, now, token) = (s.next_id()?, db::now(), secret());
    let expires_at = now + minutes * 60 * 1000;
    runner_tokens::Entity::insert(runner_tokens::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(scope.workspace_id.into()),
        host_id: Set(host_id.into()),
        token_hash: Set(fingerprint(&token)),
        expires_at: Set(expires_at),
        used_at: Set(0),
        used_by: Set(0),
        created_by: Set(scope.user.id.into()),
        created_at: Set(now),
        updated_at: Set(now),
        version: Set(1),
    })
    .exec(&s.db)
    .await?;
    // The token itself is never stored, so it is audited by id only.
    crate::hosts::activity(
        &s,
        &scope,
        "enroll-token",
        "hosts",
        &host_id,
        json!({"token_id": id, "expires_at": expires_at}),
    )
    .await?;

    // Built from the Host header, which the request guard has already checked
    // against the allowed-hosts list, so it cannot be pointed somewhere else.
    let authority = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("localhost");
    let scheme = if s.secure_cookie { "https" } else { "http" };
    let server = format!("{scheme}://{authority}");
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "token": token,
            "expires_at": expires_at,
            "server": server,
            "command": format!("ferry-runner --server {server} --token {token}"),
        })),
    ))
}

/// Burns the token and reports which workspace and host it was for.
///
/// One conditional UPDATE does the whole check: unknown, expired and
/// already-used tokens all affect zero rows, and two runners racing with the
/// same token cannot both win because only one UPDATE sees `used_at = 0`.
async fn consume_token(db: &impl sea_orm::ConnectionTrait, token: &str) -> Result<(Id, Id, Id)> {
    let (hash, now) = (fingerprint(token), db::now());
    let claimed = runner_tokens::Entity::update_many()
        .col_expr(runner_tokens::Column::UsedAt, Expr::value(now))
        .col_expr(runner_tokens::Column::UpdatedAt, Expr::value(now))
        .filter(runner_tokens::Column::TokenHash.eq(hash.clone()))
        .filter(runner_tokens::Column::UsedAt.eq(0_i64))
        .filter(runner_tokens::Column::ExpiresAt.gt(now))
        .exec(db)
        .await?
        .rows_affected;
    if claimed == 0 {
        return Err(ApiError::localized(
            StatusCode::UNAUTHORIZED,
            crate::i18n::Message::new("runner.registrationToken"),
        ));
    }
    let row = runner_tokens::Entity::find()
        .filter(runner_tokens::Column::TokenHash.eq(hash))
        .one(db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok((
        Id::from(row.id),
        Id::from(row.workspace_id),
        Id::from(row.host_id),
    ))
}

/// A name for a host row the runner created itself.
fn reported_name(hostname: &str) -> String {
    let name: String = hostname.trim().chars().take(60).collect();
    if name.is_empty() {
        "未命名主机".into()
    } else {
        name
    }
}

pub(super) fn reported_address(address: &str) -> Option<String> {
    address
        .parse::<std::net::IpAddr>()
        .ok()
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        .map(|ip| ip.to_string())
}

/// Exchanges a token for the host row it owns and the long-lived key it keeps.
pub(super) async fn enrol(
    s: &AppState,
    hello: &wire::Hello,
    token: &str,
) -> Result<(String, String)> {
    let persistent = hosts::Entity::find()
        .filter(hosts::Column::EnrollTokenHash.eq(fingerprint(token)))
        .filter(hosts::Column::Connection.eq("runner"))
        .one(&s.db)
        .await?;
    let (token_id, workspace_id, bound_host) = if let Some(host) = persistent {
        (None, Id::from(host.workspace_id), Id::from(host.id))
    } else {
        let (token_id, workspace_id, bound_host) = consume_token(&s.db, token).await?;
        (Some(token_id), workspace_id, bound_host)
    };
    let (key, now) = (secret(), db::now());
    // The key rotates the host's credential, so the row's version moves with
    // it: an operator's open edit form is genuinely stale afterwards.
    let host_id = if bound_host == Id::from(0) {
        let id = s.next_id()?;
        hosts::Entity::insert(hosts::ActiveModel {
            id: Set(id.into()),
            workspace_id: Set(workspace_id.into()),
            name: Set(reported_name(&hello.hostname)),
            address: Set(reported_address(&hello.ip_address).unwrap_or_default()),
            port: Set(0),
            username: Set(String::new()),
            connection: Set("runner".into()),
            state: Set("online".into()),
            group_id: Set(0),
            description: Set(String::new()),
            host_key: Set(String::new()),
            runner_key_hash: Set(fingerprint(&key)),
            os: Set(hello.os.clone()),
            arch: Set(hello.arch.clone()),
            agent_version: Set(hello.agent_version.clone()),
            metrics: Set(String::new()),
            seen_at: Set(now),
            created_at: Set(now),
            updated_at: Set(now),
            version: Set(1),
            ..Default::default()
        })
        .exec(&s.db)
        .await?;
        id
    } else {
        let mut update = hosts::Entity::update_many()
            .col_expr(hosts::Column::RunnerKeyHash, Expr::value(fingerprint(&key)))
            .col_expr(hosts::Column::Os, Expr::value(hello.os.clone()))
            .col_expr(hosts::Column::Arch, Expr::value(hello.arch.clone()))
            .col_expr(
                hosts::Column::AgentVersion,
                Expr::value(hello.agent_version.clone()),
            )
            .col_expr(hosts::Column::State, Expr::value("online"))
            .col_expr(hosts::Column::SeenAt, Expr::value(now))
            .col_expr(hosts::Column::UpdatedAt, Expr::value(now))
            .col_expr(
                hosts::Column::Version,
                sea_orm::ExprTrait::add(Expr::col(hosts::Column::Version), 1_i64),
            );
        if let Some(address) = reported_address(&hello.ip_address) {
            update = update.col_expr(hosts::Column::Address, Expr::value(address));
        }
        let affected = update
            .filter(hosts::Column::WorkspaceId.eq(workspace_id))
            .filter(hosts::Column::Id.eq(bound_host))
            .filter(hosts::Column::Connection.eq("runner"))
            .filter(if token_id.is_none() {
                hosts::Column::EnrollTokenHash.eq(fingerprint(token))
            } else {
                hosts::Column::Id.eq(bound_host)
            })
            .exec(&s.db)
            .await?
            .rows_affected;
        if affected == 0 {
            // The token is spent either way: a token that survives a failed
            // binding would be a token that can be retried.
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("runner.registrationHost"),
            ));
        }
        bound_host
    };
    if let Some(token_id) = token_id {
        runner_tokens::Entity::update_many()
            .col_expr(runner_tokens::Column::UsedBy, Expr::value(host_id))
            .filter(runner_tokens::Column::Id.eq(token_id))
            .exec(&s.db)
            .await?;
    }
    tracing::info!(%host_id, %workspace_id, hostname = %hello.hostname, "runner 注册成功");
    Ok((host_id.to_string(), key))
}

/// Checks a stored key against the one host it could belong to.
pub(super) async fn verify_key(s: &AppState, host_id: &str, key: &str) -> Result<()> {
    hosts::Entity::find_by_id(i64::from(Id::reference(host_id)?))
        .filter(hosts::Column::Connection.eq("runner"))
        .filter(hosts::Column::RunnerKeyHash.ne(""))
        .filter(hosts::Column::RunnerKeyHash.eq(fingerprint(key)))
        .one(&s.db)
        .await?
        .ok_or_else(|| {
            ApiError::localized(
                StatusCode::UNAUTHORIZED,
                crate::i18n::Message::new("runner.credentials"),
            )
        })
        .map(|_| ())
}

// ------------------------------------------------------------ 控制连接

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosts::runner::test_support::scratch_db;
    use sea_orm::DatabaseConnection;

    #[test]
    fn a_secret_is_hashed_before_it_is_stored() {
        let (first, second) = (secret(), secret());
        assert_eq!(first.len(), 64, "256 bits of hex");
        assert_ne!(first, second, "secrets must not repeat");
        let hash = fingerprint(&first);
        assert_eq!(hash.len(), 64);
        assert_eq!(hash, fingerprint(&first), "same input, same hash");
        assert_ne!(hash, first, "the stored value is not the secret");
        assert_ne!(fingerprint("a"), fingerprint("b"));
    }

    async fn mint(db: &DatabaseConnection, token: &str, expires_at: i64) {
        runner_tokens::Entity::insert(runner_tokens::ActiveModel {
            id: Set(1),
            workspace_id: Set(1),
            host_id: Set(0),
            token_hash: Set(fingerprint(token)),
            expires_at: Set(expires_at),
            used_at: Set(0),
            used_by: Set(0),
            created_by: Set(1),
            created_at: Set(0),
            updated_at: Set(0),
            version: Set(1),
        })
        .exec(db)
        .await
        .expect("insert token");
    }

    #[tokio::test]
    async fn bootstrap_probe_does_not_consume_and_rejects_expired_or_used_tokens() {
        let scratch = scratch_db().await;
        let db = &*scratch;
        let token = secret();
        mint(db, &token, db::now() + 60_000).await;
        bootstrap_token(db, &token).await.unwrap();
        bootstrap_token(db, &token).await.unwrap();
        consume_token(db, &token).await.unwrap();
        assert_eq!(
            bootstrap_token(db, &token).await.unwrap_err().0,
            StatusCode::UNAUTHORIZED
        );
        runner_tokens::Entity::update_many()
            .col_expr(runner_tokens::Column::UsedAt, Expr::value(0_i64))
            .col_expr(runner_tokens::Column::ExpiresAt, Expr::value(db::now() - 1))
            .exec(db)
            .await
            .unwrap();
        assert_eq!(
            bootstrap_token(db, &token).await.unwrap_err().0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            bootstrap_token(db, &secret()).await.unwrap_err().0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            bootstrap_token(db, "bad").await.unwrap_err().0,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn an_enrolment_token_works_once_and_never_again() {
        let scratch = scratch_db().await;
        let db = &*scratch;
        let token = secret();
        mint(db, &token, db::now() + 60_000).await;

        let (_id, workspace_id, host_id) = consume_token(db, &token).await.expect("first use");
        assert_eq!(workspace_id, Id::from(1));
        assert_eq!(host_id, Id::from(0), "unbound token");

        let again = consume_token(db, &token).await;
        assert!(again.is_err(), "a token must not be reusable");
        assert_eq!(again.unwrap_err().0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn host_connection_token_survives_use_until_rotated() {
        let scratch = scratch_db().await;
        let db = &*scratch;
        let old = secret();
        hosts::Entity::insert(hosts::ActiveModel {
            id: Set(7),
            workspace_id: Set(1),
            name: Set("host".into()),
            address: Set(String::new()),
            port: Set(0),
            username: Set(String::new()),
            connection: Set("runner".into()),
            state: Set("offline".into()),
            group_id: Set(0),
            description: Set(String::new()),
            host_key: Set(String::new()),
            runner_key_hash: Set(String::new()),
            enroll_token: Set("sealed".into()),
            enroll_token_hash: Set(fingerprint(&old)),
            seen_at: Set(0),
            created_at: Set(0),
            updated_at: Set(0),
            version: Set(1),
            os: Set(String::new()),
            arch: Set(String::new()),
            agent_version: Set(String::new()),
            metrics: Set(String::new()),
            credential_id: Set(0),
            ..Default::default()
        })
        .exec(db)
        .await
        .unwrap();
        bootstrap_token(db, &old).await.unwrap();
        bootstrap_token(db, &old).await.unwrap();
        hosts::Entity::update_many()
            .col_expr(
                hosts::Column::EnrollTokenHash,
                Expr::value(fingerprint(&secret())),
            )
            .filter(hosts::Column::Id.eq(7))
            .exec(db)
            .await
            .unwrap();
        assert_eq!(
            bootstrap_token(db, &old).await.unwrap_err().0,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn an_expired_or_unknown_token_is_refused() {
        let scratch = scratch_db().await;
        let db = &*scratch;
        let expired = secret();
        mint(db, &expired, db::now() - 1).await;
        assert!(
            consume_token(db, &expired).await.is_err(),
            "an expired token must be refused"
        );
        assert!(
            consume_token(db, "not-a-token").await.is_err(),
            "garbage must be refused"
        );
        // A refused token must stay unused, not be quietly burned.
        let row = runner_tokens::Entity::find()
            .filter(runner_tokens::Column::TokenHash.eq(fingerprint(&expired)))
            .one(db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.used_at, 0);
    }

    #[tokio::test]
    async fn a_key_only_matches_its_own_host() {
        let scratch = scratch_db().await;
        let db = &*scratch;
        let (key_a, key_b) = (secret(), secret());
        for (id, workspace, key) in [(1_i64, 11_i64, &key_a), (2_i64, 12_i64, &key_b)] {
            hosts::Entity::insert(hosts::ActiveModel {
                id: Set(id),
                workspace_id: Set(workspace),
                name: Set(id.to_string()),
                address: Set(String::new()),
                port: Set(0),
                username: Set(String::new()),
                connection: Set("runner".into()),
                state: Set("online".into()),
                group_id: Set(0),
                description: Set(String::new()),
                host_key: Set(String::new()),
                runner_key_hash: Set(fingerprint(key)),
                os: Set(String::new()),
                arch: Set(String::new()),
                agent_version: Set(String::new()),
                metrics: Set(String::new()),
                seen_at: Set(0),
                created_at: Set(0),
                updated_at: Set(0),
                version: Set(1),
                ..Default::default()
            })
            .exec(db)
            .await
            .expect("insert host");
        }
        // Reaching across workspaces means presenting another host's id with
        // your own key, which has to fail.
        assert!(matches(db, 1, &key_a).await);
        assert!(!matches(db, 2, &key_a).await, "cross-host reuse");
        assert!(!matches(db, 1, &key_b).await, "cross-host reuse");
        assert!(!matches(db, 1, "").await, "an empty key matches nothing");
    }

    async fn matches(db: &DatabaseConnection, host_id: i64, key: &str) -> bool {
        hosts::Entity::find_by_id(host_id)
            .filter(hosts::Column::Connection.eq("runner"))
            .filter(hosts::Column::RunnerKeyHash.ne(""))
            .filter(hosts::Column::RunnerKeyHash.eq(fingerprint(key)))
            .one(db)
            .await
            .unwrap()
            .is_some()
    }

    #[test]
    fn a_reported_hostname_becomes_a_usable_name() {
        assert_eq!(reported_name("  web-01 "), "web-01");
        assert_eq!(reported_name(""), "未命名主机");
        assert_eq!(reported_name(&"x".repeat(200)).chars().count(), 60);
    }

    #[test]
    fn runner_address_accepts_only_usable_ips() {
        assert_eq!(
            reported_address("192.168.0.130"),
            Some("192.168.0.130".into())
        );
        for address in ["", "localhost", "127.0.0.1", "0.0.0.0"] {
            assert_eq!(reported_address(address), None);
        }
    }
}
