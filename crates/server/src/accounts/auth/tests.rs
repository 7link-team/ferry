use super::*;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set, sea_query::Expr};

fn refresh_headers(value: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        format!("{REFRESH_COOKIE}={value}").parse().unwrap(),
    );
    headers
}

#[tokio::test]
async fn logout_revokes_a_rotation_even_when_its_old_cookies_arrive_after_the_rotation() {
    use axum::response::IntoResponse;
    let (_dir, s) = crate::agents::tests::setup().await;
    let issued = start_session(&s, &Id::from(1), &s.db).await.unwrap();
    let row = access_row(&s, &issued.token).await;
    let (headers, _) = refresh(State(s.clone()), refresh_headers(&issued.refresh_token))
        .await
        .unwrap();
    // The refresh committed, but its response has not yet reached a tab which
    // already submitted logout using the original cookies.
    let mut stale = refresh_headers(&issued.refresh_token);
    stale.insert(
        header::COOKIE,
        format!(
            "{COOKIE}={}; {REFRESH_COOKIE}={}",
            issued.token, issued.refresh_token
        )
        .parse()
        .unwrap(),
    );
    let response = logout(State(s.clone()), stale)
        .await
        .unwrap()
        .into_response();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert!(
        sessions::Entity::find_by_id(row.id)
            .one(&s.db)
            .await
            .unwrap()
            .is_none(),
        "a late refresh response must not restore a logged-out session"
    );
    assert!(
        refresh(
            State(s.clone()),
            refresh_headers(&issued_cookie(&headers, REFRESH_COOKIE))
        )
        .await
        .is_err()
    );
}
fn issued_cookie(headers: &HeaderMap, name: &str) -> String {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.split(';').next()?.split_once('='))
        .find(|(key, _)| *key == name)
        .unwrap()
        .1
        .to_owned()
}
async fn access_row(s: &AppState, token: &str) -> sessions::Model {
    sessions::Entity::find()
        .filter(sessions::Column::TokenHash.eq(crate::tokens::fingerprint(token)))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn refresh_rotates_both_tokens_rejects_replay_and_never_extends_the_original_deadline() {
    let (_dir, s) = crate::agents::tests::setup().await;
    let issued = start_session(&s, &Id::from(1), &s.db).await.unwrap();
    let row = access_row(&s, &issued.token).await;
    let original = session_refresh_tokens::Entity::find_by_id(row.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    sessions::Entity::update_many()
        .col_expr(sessions::Column::ExpiresAt, Expr::value(db::now() - 1))
        .filter(sessions::Column::Id.eq(row.id))
        .exec(&s.db)
        .await
        .unwrap();
    let (headers, Json(meta)) = refresh(State(s.clone()), refresh_headers(&issued.refresh_token))
        .await
        .unwrap();
    assert_eq!(headers.get_all(header::SET_COOKIE).iter().count(), 2);
    for value in headers.get_all(header::SET_COOKIE) {
        let value = value.to_str().unwrap();
        assert!(value.contains("HttpOnly") && value.contains("SameSite=Lax"));
    }
    let token = issued_cookie(&headers, COOKIE);
    let refresh_token = issued_cookie(&headers, REFRESH_COOKIE);
    assert!(token != issued.token && refresh_token != issued.refresh_token);
    assert_eq!(meta["refresh_expires_at_ms"], original.expires_at);
    assert_eq!(meta["auto_renew"], true);
    let stored = session_refresh_tokens::Entity::find_by_id(row.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.expires_at, original.expires_at);
    assert_eq!(
        stored.token_hash,
        crate::tokens::fingerprint(&refresh_token)
    );
    assert_eq!(access_row(&s, &token).await.id, row.id);
    assert_eq!(
        refresh(State(s.clone()), refresh_headers(&issued.refresh_token))
            .await
            .err()
            .unwrap()
            .0,
        axum::http::StatusCode::UNAUTHORIZED
    );
    let (mut parts, _) = axum::http::Request::builder()
        .header(header::COOKIE, format!("{COOKIE}={}", issued.token))
        .body(())
        .unwrap()
        .into_parts();
    assert!(Account::from_request_parts(&mut parts, &s).await.is_err());
    assert_eq!(
        access_row(&s, &token).await.id,
        row.id,
        "a stale request must not delete the winner"
    );
}

#[tokio::test]
async fn refresh_deadline_clamps_access_and_expiry_revokes_the_entire_pair() {
    let (_dir, s) = crate::agents::tests::setup().await;
    let issued = start_session(&s, &Id::from(1), &s.db).await.unwrap();
    let row = access_row(&s, &issued.token).await;
    let deadline = db::now() + 5000;
    session_refresh_tokens::Entity::update_many()
        .col_expr(
            session_refresh_tokens::Column::ExpiresAt,
            Expr::value(deadline),
        )
        .filter(session_refresh_tokens::Column::SessionId.eq(row.id))
        .exec(&s.db)
        .await
        .unwrap();
    let (headers, Json(meta)) = refresh(State(s.clone()), refresh_headers(&issued.refresh_token))
        .await
        .unwrap();
    assert_eq!(meta["access_expires_at_ms"], deadline);
    assert_eq!(meta["refresh_expires_at_ms"], deadline);
    session_refresh_tokens::Entity::update_many()
        .col_expr(
            session_refresh_tokens::Column::ExpiresAt,
            Expr::value(db::now() - 1),
        )
        .filter(session_refresh_tokens::Column::SessionId.eq(row.id))
        .exec(&s.db)
        .await
        .unwrap();
    let response = refresh(
        State(s.clone()),
        refresh_headers(&issued_cookie(&headers, REFRESH_COOKIE)),
    )
    .await;
    assert_eq!(
        response.err().unwrap().0,
        axum::http::StatusCode::UNAUTHORIZED
    );
    assert!(
        sessions::Entity::find_by_id(row.id)
            .one(&s.db)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        session_refresh_tokens::Entity::find_by_id(row.id)
            .one(&s.db)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn configured_lifetimes_apply_to_new_logins_but_disabling_renewal_is_immediate() {
    use crate::entities::platform_state;
    let (_dir, s) = crate::agents::tests::setup().await;
    let original = start_session(&s, &Id::from(1), &s.db).await.unwrap();
    let row = access_row(&s, &original.token).await;
    let first = session_refresh_tokens::Entity::find_by_id(row.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    platform_state::Entity::update_many()
        .col_expr(
            platform_state::Column::SessionConfig,
            Expr::value(json!({"access_minutes":5,"refresh_days":1,"auto_renew":true}).to_string()),
        )
        .filter(platform_state::Column::Id.eq(1))
        .exec(&s.db)
        .await
        .unwrap();
    let (headers, _) = refresh(State(s.clone()), refresh_headers(&original.refresh_token))
        .await
        .unwrap();
    let existing = session_refresh_tokens::Entity::find_by_id(row.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(existing.access_ttl_ms, first.access_ttl_ms);
    assert_eq!(existing.expires_at, first.expires_at);
    let later = start_session(&s, &Id::from(1), &s.db).await.unwrap();
    let later_row = access_row(&s, &later.token).await;
    assert_eq!(
        session_refresh_tokens::Entity::find_by_id(later_row.id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .access_ttl_ms,
        300_000
    );
    platform_state::Entity::update_many()
        .col_expr(
            platform_state::Column::SessionConfig,
            Expr::value(json!({"auto_renew":false}).to_string()),
        )
        .filter(platform_state::Column::Id.eq(1))
        .exec(&s.db)
        .await
        .unwrap();
    let denied = refresh(
        State(s.clone()),
        refresh_headers(&issued_cookie(&headers, REFRESH_COOKIE)),
    )
    .await;
    assert_eq!(denied.err().unwrap().0, axum::http::StatusCode::FORBIDDEN);
    assert_eq!(
        access_row(&s, &issued_cookie(&headers, COOKIE)).await.id,
        row.id
    );
    assert_eq!(
        session_refresh_tokens::Entity::find_by_id(row.id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .token_hash,
        existing.token_hash
    );
}

#[tokio::test]
async fn concurrent_refresh_has_one_winner_and_the_loser_never_clears_its_cookies() {
    use axum::response::IntoResponse;
    let (_dir, s) = crate::agents::tests::setup().await;
    let issued = start_session(&s, &Id::from(1), &s.db).await.unwrap();
    let headers = refresh_headers(&issued.refresh_token);
    let (a, b) = tokio::join!(
        refresh(State(s.clone()), headers.clone()),
        refresh(State(s.clone()), headers)
    );
    let (winner, loser) = match (a, b) {
        (Ok(winner), Err(loser)) | (Err(loser), Ok(winner)) => (winner, loser),
        _ => panic!("exactly one rotation must commit"),
    };
    let loser = loser.into_response();
    assert_eq!(loser.status(), axum::http::StatusCode::UNAUTHORIZED);
    assert_eq!(
        loser.headers().get_all(header::SET_COOKIE).iter().count(),
        0
    );
    let (_, Json(_)) = refresh(
        State(s.clone()),
        refresh_headers(&issued_cookie(&winner.0, REFRESH_COOKIE)),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn expired_access_can_log_out_and_logout_revokes_both_credentials_idempotently() {
    use axum::response::IntoResponse;
    let (_dir, s) = crate::agents::tests::setup().await;
    let issued = start_session(&s, &Id::from(1), &s.db).await.unwrap();
    let row = access_row(&s, &issued.token).await;
    sessions::Entity::update_many()
        .col_expr(sessions::Column::ExpiresAt, Expr::value(db::now() - 1))
        .filter(sessions::Column::Id.eq(row.id))
        .exec(&s.db)
        .await
        .unwrap();
    let response = logout(State(s.clone()), refresh_headers(&issued.refresh_token))
        .await
        .unwrap()
        .into_response();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .count(),
        2
    );
    assert!(
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .all(|v| v.to_str().unwrap().contains("Max-Age=0"))
    );
    assert!(
        sessions::Entity::find_by_id(row.id)
            .one(&s.db)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        session_refresh_tokens::Entity::find_by_id(row.id)
            .one(&s.db)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        logout(State(s.clone()), HeaderMap::new())
            .await
            .unwrap()
            .into_response()
            .status(),
        axum::http::StatusCode::OK
    );
}

#[tokio::test]
async fn disabled_users_cannot_refresh_and_their_pair_is_revoked() {
    let (_dir, s) = crate::agents::tests::setup().await;
    let issued = start_session(&s, &Id::from(1), &s.db).await.unwrap();
    let row = access_row(&s, &issued.token).await;
    users::Entity::update_many()
        .col_expr(users::Column::Active, Expr::value(0))
        .filter(users::Column::Id.eq(1))
        .exec(&s.db)
        .await
        .unwrap();
    assert_eq!(
        refresh(State(s.clone()), refresh_headers(&issued.refresh_token))
            .await
            .err()
            .unwrap()
            .0,
        axum::http::StatusCode::UNAUTHORIZED
    );
    assert!(
        sessions::Entity::find_by_id(row.id)
            .one(&s.db)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn new_logins_have_short_access_and_an_independent_fixed_refresh_deadline() {
    let (_dir, s) = crate::agents::tests::setup().await;
    let before = db::now();
    let _issued = start_session(&s, &Id::from(1), &s.db).await.unwrap();
    let access = sessions::Entity::find()
        .filter(sessions::Column::UserId.eq(1))
        .order_by_desc(sessions::Column::Id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert!((before + 1_800_000..=db::now() + 1_800_000).contains(&access.expires_at));
    let refresh = crate::entities::session_refresh_tokens::Entity::find_by_id(access.id)
        .one(&s.db)
        .await
        .unwrap();
    assert!(
        refresh.is_some(),
        "a new login must have its own revocable refresh token"
    );
    let refresh = refresh.unwrap();
    assert_eq!(refresh.access_ttl_ms, 1_800_000);
    assert!((before + 1_209_600_000..=db::now() + 1_209_600_000).contains(&refresh.expires_at));
    assert_eq!(
        (refresh.created_by, refresh.rotated_by, refresh.expires_by),
        (1, 1, 1)
    );
}

#[tokio::test]
async fn expired_access_does_not_delete_a_session_with_a_live_refresh_grant() {
    let (_dir, s) = crate::agents::tests::setup().await;
    let token = crate::hosts::runner::secret();
    let id = s.next_id().unwrap();
    sessions::Entity::insert(sessions::ActiveModel {
        id: Set(id.into()),
        token_hash: Set(crate::hosts::runner::fingerprint(&token)),
        user_id: Set(1),
        workspace_id: Set(0),
        created_at: Set(db::now()),
        seen_at: Set(db::now()),
        expires_at: Set(db::now() - 1),
    })
    .exec(&s.db)
    .await
    .unwrap();
    crate::entities::session_refresh_tokens::Entity::insert(
        crate::entities::session_refresh_tokens::ActiveModel {
            session_id: Set(id.into()),
            token_hash: Set("test-refresh-hash".into()),
            previous_hash: Set(None),
            access_ttl_ms: Set(1_800_000),
            created_at: Set(db::now()),
            created_by: Set(1),
            rotated_at: Set(db::now()),
            rotated_by: Set(1),
            expires_at: Set(db::now() + 100_000),
            expires_by: Set(1),
        },
    )
    .exec(&s.db)
    .await
    .unwrap();
    let (mut parts, _) = axum::http::Request::builder()
        .header(header::COOKIE, format!("{COOKIE}={token}"))
        .body(())
        .unwrap()
        .into_parts();
    assert_eq!(
        Account::from_request_parts(&mut parts, &s)
            .await
            .err()
            .unwrap()
            .0,
        axum::http::StatusCode::UNAUTHORIZED
    );
    assert!(
        sessions::Entity::find_by_id(id)
            .one(&s.db)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn platform_exposes_configurable_access_refresh_and_active_renewal_defaults() {
    let (_dir, s) = crate::agents::tests::setup().await;
    users::Entity::update_many()
        .col_expr(users::Column::IsSuperadmin, Expr::value(1))
        .filter(users::Column::Id.eq(1))
        .exec(&s.db)
        .await
        .unwrap();
    let Json(settings) = crate::accounts::platform::get_settings(
        State(s),
        crate::accounts::platform::Admin(Account {
            user: crate::agents::tests::admin(1).user,
            session_id: 0.into(),
        }),
    )
    .await
    .unwrap();
    assert_eq!(settings["session_access_minutes"], 30);
    assert_eq!(settings["session_refresh_days"], 14);
    assert_eq!(settings["session_auto_renew"], true);
}

#[tokio::test]
async fn authenticated_requests_do_not_slide_the_existing_session_deadline() {
    let (_dir, s) = crate::agents::tests::setup().await;
    let token = crate::hosts::runner::secret();
    let id = s.next_id().unwrap();
    let deadline = db::now() + 1234567;
    sessions::Entity::insert(sessions::ActiveModel {
        id: Set(id.into()),
        token_hash: Set(crate::hosts::runner::fingerprint(&token)),
        user_id: Set(1),
        workspace_id: Set(0),
        created_at: Set(db::now()),
        seen_at: Set(db::now()),
        expires_at: Set(deadline),
    })
    .exec(&s.db)
    .await
    .unwrap();
    let (mut parts, _) = axum::http::Request::builder()
        .header(header::COOKIE, format!("{COOKIE}={token}"))
        .body(())
        .unwrap()
        .into_parts();
    let account = Account::from_request_parts(&mut parts, &s).await.unwrap();
    assert_eq!(account.user.id, Id::from(1));
    let stored = sessions::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.expires_at, deadline,
        "reads and background polling must not renew a session"
    );
}
