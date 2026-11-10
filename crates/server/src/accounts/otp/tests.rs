use super::*;
use sea_orm::{IntoActiveModel, PaginatorTrait};

#[test]
fn rfc6238_vectors_window_format_and_replay() {
    let key = b"12345678901234567890";
    for (time, code) in [
        (59, "287082"),
        (1111111109, "081804"),
        (1111111111, "050471"),
        (1234567890, "005924"),
        (2000000000, "279037"),
        (20000000000, "353130"),
    ] {
        let step = matching_step(key, code, time, None).unwrap().unwrap();
        assert!(
            matching_step(key, code, time, Some(step))
                .unwrap()
                .is_none()
        );
    }
    let time = 3000;
    let totp = generator(
        key.to_vec(),
        "Demo: Site".into(),
        "test:account@example.invalid".into(),
    )
    .unwrap();
    assert!(!totp.get_url().is_empty());
    for offset in [-30i64, 0, 30] {
        assert!(
            matching_step(
                key,
                &totp.generate((time as i64 + offset) as u64),
                time,
                None
            )
            .unwrap()
            .is_some()
        );
    }
    for code in ["", "12345", "1234567", "abcdef", "１２３４５６", " 287082"] {
        assert!(matching_step(key, code, time, None).unwrap().is_none());
    }
    assert!(
        matching_step(key, &totp.generate(time + 60), time, None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn recovery_codes_are_account_bound_one_use_and_reset_invalidates_old() {
    let (codes, hashes) = recovery_codes(1);
    assert_eq!(codes.len(), 10);
    assert_eq!(
        codes.iter().collect::<std::collections::HashSet<_>>().len(),
        10
    );
    let mut data = Stored {
        secret: Some(secret()),
        recovery_hashes: hashes,
        ..Default::default()
    };
    assert!(!consume(&mut data, 2, &codes[0], true, 3000).unwrap());
    assert!(consume(&mut data, 1, &codes[0].to_uppercase(), true, 3000).unwrap());
    assert!(!consume(&mut data, 1, &codes[0], true, 3000).unwrap());
    let (_, new) = recovery_codes(1);
    data.recovery_hashes = new;
    assert!(!consume(&mut data, 1, &codes[1], true, 3000).unwrap());
    assert!(!consume(&mut data, 1, &"a".repeat(129), true, 3000).unwrap());
}

async fn fixture() -> (tempfile::TempDir, AppState) {
    let (dir, db) = crate::lock::tests::database().await;
    let s = std::sync::Arc::new(crate::state::State {
        ids: std::sync::Arc::new(
            crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap(),
        ),
        db,
        data_dir: dir.path().into(),
        instance_id: "otp-test".into(),
        server_mode: false,
        secure_cookie: false,
        bind: "127.0.0.1:0".parse().unwrap(),
        vault: crate::vault::Vault::open(dir.path(), false).unwrap(),
        cluster: None,
        host_monitor: Default::default(),
        monitor: Default::default(),
    });
    let hash = auth::hash("test-password").unwrap();
    for id in [1, 2] {
        users::Entity::insert(
            users::Model {
                id,
                email: format!("user{id}@example.invalid"),
                name: format!("User {id}"),
                password_hash: hash.clone(),
                is_superadmin: 0,
                active: 1,
                version: 1,
                created_at: 0,
                updated_at: 0,
                last_login_at: None,
                last_login_ip: None,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
        sessions::Entity::insert(sessions::ActiveModel {
            id: Set(id + 10),
            user_id: Set(id),
            workspace_id: Set(0),
            token_hash: Set(format!("session-{id}")),
            created_at: Set(0),
            seen_at: Set(0),
            expires_at: Set(db::now() + auth::SESSION_MS),
        })
        .exec(&s.db)
        .await
        .unwrap();
    }
    (dir, s)
}
fn account(id: i64) -> Account {
    Account {
        user: auth::User {
            id: Id::from(id),
            email: format!("user{id}@example.invalid"),
            name: format!("User {id}"),
            is_superadmin: false,
        },
        session_id: Id::from(id + 10),
    }
}
async fn enabled(s: &AppState, value: bool) {
    let sealed =
        guard::Configuration::update(s, None, json!({"otp_enabled":value}).as_object().unwrap())
            .unwrap();
    platform_state::Entity::update_many()
        .col_expr(
            platform_state::Column::LoginSecurity,
            sea_orm::sea_query::Expr::value(sealed),
        )
        .exec(&s.db)
        .await
        .unwrap();
}
async fn json_body(value: impl IntoResponse) -> Value {
    let response = value.into_response();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap(),
    )
    .unwrap()
}
async fn bound(s: &AppState, id: i64) -> Vec<String> {
    let (codes, hashes) = recovery_codes(id);
    save(
        s,
        &s.db,
        id,
        &Stored {
            secret: Some(secret()),
            recovery_hashes: hashes,
            generation: nonce(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    codes
}
async fn challenge(s: &AppState, id: i64) -> String {
    let user = users::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let tx = s.db.begin().await.unwrap();
    platform::lock(&tx).await.unwrap();
    let result = issue(s, &tx, &user, "127.0.0.1").await.unwrap().unwrap();
    tx.commit().await.unwrap();
    result["otp_token"].as_str().unwrap().into()
}
async fn sign_in(s: &AppState, token: &str, code: &str) -> axum::response::Response {
    match login(
        State(s.clone()),
        ConnectInfo("127.0.0.1:1234".parse().unwrap()),
        Extension(auth::TrustedProxies(vec![])),
        HeaderMap::new(),
        Json(Login {
            token: token.into(),
            code: code.into(),
            use_recovery: true,
        }),
    )
    .await
    {
        Ok(value) => value.into_response(),
        Err(error) => error.into_response(),
    }
}

#[tokio::test]
async fn enrollment_is_verified_session_bound_encrypted_and_does_not_cross_accounts() {
    let (_dir, s) = fixture().await;
    assert!(!allowed(&s, &s.db).await.unwrap());
    assert!(
        enroll(
            State(s.clone()),
            account(1),
            Json(Password {
                current_password: "test-password".into()
            })
        )
        .await
        .is_err()
    );
    enabled(&s, true).await;
    assert!(
        enroll(
            State(s.clone()),
            account(1),
            Json(Password {
                current_password: "wrong".into()
            })
        )
        .await
        .is_err()
    );
    let response = json_body(
        enroll(
            State(s.clone()),
            account(1),
            Json(Password {
                current_password: "test-password".into(),
            }),
        )
        .await
        .unwrap(),
    )
    .await;
    assert!(response["qr_svg"].as_str().unwrap().contains("<svg"));
    let data = load(&s, &s.db, 1).await.unwrap();
    assert!(data.secret.is_none());
    let raw = data.pending.as_ref().unwrap().secret.clone();
    let code = generator(raw, "test".into(), "account".into())
        .unwrap()
        .generate((db::now() / 1000) as u64);
    assert!(
        confirm(
            State(s.clone()),
            account(2),
            Json(Code { code: code.clone() })
        )
        .await
        .is_err()
    );
    let mut other_session = account(1);
    other_session.session_id = Id::from(999);
    assert!(
        confirm(
            State(s.clone()),
            other_session,
            Json(Code { code: code.clone() })
        )
        .await
        .is_err()
    );
    let result = json_body(
        confirm(
            State(s.clone()),
            account(1),
            Json(Code { code: code.clone() }),
        )
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(result["recovery_codes"].as_array().unwrap().len(), 10);
    let row = user_otp::Entity::find_by_id(1)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert!(!row.payload.contains(response["secret"].as_str().unwrap()));
    assert!(s.vault.open_secret(&row.payload, &aad(2)).is_err());
    assert!(load(&s, &s.db, 2).await.unwrap().secret.is_none());
    assert!(
        !consume(
            &mut load(&s, &s.db, 1).await.unwrap(),
            1,
            &code,
            false,
            (db::now() / 1000) as u64
        )
        .unwrap()
    );
    enabled(&s, false).await;
    assert!(!challenge(&s, 1).await.is_empty());
}

#[tokio::test]
async fn challenge_attempts_commit_and_recovery_consumption_is_atomic() {
    let (_dir, s) = fixture().await;
    let codes = bound(&s, 1).await;
    let token = challenge(&s, 1).await;
    for _ in 0..5 {
        let response = sign_in(&s, &token, "wrong").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(response.headers().get("set-cookie").is_none());
    }
    assert_eq!(
        sign_in(&s, &token, &codes[0]).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(sessions::Entity::find().count(&s.db).await.unwrap(), 2);
    let a = challenge(&s, 1).await;
    let b = challenge(&s, 1).await;
    let (a, b) = tokio::join!(sign_in(&s, &a, &codes[0]), sign_in(&s, &b, &codes[0]));
    let statuses = [a.status(), b.status()];
    assert_eq!(statuses.iter().filter(|&&v| v == StatusCode::OK).count(), 1);
    assert_eq!(sessions::Entity::find().count(&s.db).await.unwrap(), 3);
    assert_eq!(load(&s, &s.db, 1).await.unwrap().recovery_hashes.len(), 9);
    let row = users::Entity::find_by_id(1)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert!(row.last_login_at.is_some());
    assert_eq!(row.last_login_ip.as_deref(), Some("127.0.0.1"));
}

#[tokio::test]
async fn password_change_disable_generation_expiry_and_ip_invalidate_challenges() {
    let (_dir, s) = fixture().await;
    let codes = bound(&s, 1).await;
    let token = challenge(&s, 1).await;
    users::Entity::update_many()
        .col_expr(
            users::Column::PasswordHash,
            sea_orm::sea_query::Expr::value("changed"),
        )
        .filter(users::Column::Id.eq(1))
        .exec(&s.db)
        .await
        .unwrap();
    assert_eq!(
        sign_in(&s, &token, &codes[0]).await.status(),
        StatusCode::BAD_REQUEST
    );
    let token = challenge(&s, 1).await;
    users::Entity::update_many()
        .col_expr(users::Column::Active, sea_orm::sea_query::Expr::value(0))
        .filter(users::Column::Id.eq(1))
        .exec(&s.db)
        .await
        .unwrap();
    assert_eq!(
        sign_in(&s, &token, &codes[0]).await.status(),
        StatusCode::BAD_REQUEST
    );
    users::Entity::update_many()
        .col_expr(users::Column::Active, sea_orm::sea_query::Expr::value(1))
        .filter(users::Column::Id.eq(1))
        .exec(&s.db)
        .await
        .unwrap();
    let token = challenge(&s, 1).await;
    let mut data = load(&s, &s.db, 1).await.unwrap();
    data.generation = nonce();
    save(&s, &s.db, 1, &data).await.unwrap();
    assert_eq!(
        sign_in(&s, &token, &codes[0]).await.status(),
        StatusCode::BAD_REQUEST
    );
    let token = challenge(&s, 1).await;
    login_guards::Entity::update_many()
        .col_expr(
            login_guards::Column::ExpiresAt,
            sea_orm::sea_query::Expr::value(0),
        )
        .exec(&s.db)
        .await
        .unwrap();
    assert_eq!(
        sign_in(&s, &token, &codes[0]).await.status(),
        StatusCode::GONE
    );
    let user = users::Entity::find_by_id(1)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let token = issue(&s, &s.db, &user, "127.0.0.2").await.unwrap().unwrap()["otp_token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        sign_in(&s, &token, &codes[0]).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(load(&s, &s.db, 1).await.unwrap().recovery_hashes.len(), 10);
}

#[tokio::test]
async fn reset_and_remove_require_password_and_revoke_other_sessions() {
    let (_dir, s) = fixture().await;
    let codes = bound(&s, 1).await;
    auth::start_session(&s, &Id::from(1), &s.db).await.unwrap();
    assert_eq!(sessions::Entity::find().count(&s.db).await.unwrap(), 3);
    assert!(
        regenerate(
            State(s.clone()),
            account(1),
            Json(Manage {
                current_password: "wrong".into(),
                code: codes[0].clone(),
                use_recovery: true
            })
        )
        .await
        .is_err()
    );
    let result = json_body(
        regenerate(
            State(s.clone()),
            account(1),
            Json(Manage {
                current_password: "test-password".into(),
                code: codes[0].clone(),
                use_recovery: true,
            }),
        )
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(sessions::Entity::find().count(&s.db).await.unwrap(), 2);
    assert_eq!(
        sign_in(&s, &challenge(&s, 1).await, &codes[1])
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let new = result["recovery_codes"][0].as_str().unwrap();
    assert!(
        disable(
            State(s.clone()),
            account(2),
            Json(Manage {
                current_password: "test-password".into(),
                code: new.into(),
                use_recovery: true
            })
        )
        .await
        .is_err()
    );
    disable(
        State(s.clone()),
        account(1),
        Json(Manage {
            current_password: "test-password".into(),
            code: new.into(),
            use_recovery: true,
        }),
    )
    .await
    .unwrap();
    assert!(
        user_otp::Entity::find_by_id(1)
            .one(&s.db)
            .await
            .unwrap()
            .is_none()
    );
    let user = users::Entity::find_by_id(1)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert!(
        issue(&s, &s.db, &user, "127.0.0.1")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn account_attempt_limits_span_challenges_and_pending_password_and_expiry_are_checked() {
    let (_dir, s) = fixture().await;
    let codes = bound(&s, 1).await;
    for _ in 0..10 {
        let token = challenge(&s, 1).await;
        assert_eq!(
            sign_in(&s, &token, "wrong").await.status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        sign_in(&s, &challenge(&s, 1).await, &codes[0])
            .await
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(load(&s, &s.db, 1).await.unwrap().recovery_hashes.len(), 10);
    enabled(&s, true).await;
    enroll(
        State(s.clone()),
        account(2),
        Json(Password {
            current_password: "test-password".into(),
        }),
    )
    .await
    .unwrap();
    let mut data = load(&s, &s.db, 2).await.unwrap();
    let raw = data.pending.as_ref().unwrap().secret.clone();
    let code = generator(raw, "test".into(), "account".into())
        .unwrap()
        .generate((db::now() / 1000) as u64);
    data.pending.as_mut().unwrap().expires_millis = 0;
    save(&s, &s.db, 2, &data).await.unwrap();
    assert!(
        confirm(
            State(s.clone()),
            account(2),
            Json(Code { code: code.clone() })
        )
        .await
        .is_err()
    );
    data.pending.as_mut().unwrap().expires_millis = db::now() + ENROLL_TTL;
    save(&s, &s.db, 2, &data).await.unwrap();
    users::Entity::update_many()
        .col_expr(
            users::Column::PasswordHash,
            sea_orm::sea_query::Expr::value("changed"),
        )
        .filter(users::Column::Id.eq(2))
        .exec(&s.db)
        .await
        .unwrap();
    assert!(
        confirm(State(s.clone()), account(2), Json(Code { code }))
            .await
            .is_err()
    );
    assert!(load(&s, &s.db, 2).await.unwrap().secret.is_none());
}
