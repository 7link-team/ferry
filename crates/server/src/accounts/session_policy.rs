//! Administrator-owned login defaults. Lifetimes are snapshotted on login;
//! the renewal switch is checked again for every refresh.
use crate::{
    db,
    entities::platform_state,
    error::{ApiError, Result},
};
use sea_orm::{ConnectionTrait, EntityTrait};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Configuration {
    pub access_minutes: i64,
    pub refresh_days: i64,
    pub auto_renew: bool,
}
impl Default for Configuration {
    fn default() -> Self {
        Self {
            access_minutes: 30,
            refresh_days: 14,
            auto_renew: true,
        }
    }
}
impl Configuration {
    pub fn read(value: Option<&str>) -> Result<Self> {
        let config: Self = value
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| invalid())?
            .unwrap_or_default();
        config.lifetimes(db::now())?;
        Ok(config)
    }
    pub async fn load(conn: &impl ConnectionTrait) -> Result<Self> {
        let row = platform_state::Entity::find_by_id(1)
            .one(conn)
            .await?
            .ok_or_else(ApiError::missing)?;
        Self::read(row.session_config.as_deref())
    }
    /// Keep arithmetic and JSON/browser timestamps exact, without an arbitrary
    /// product cap on the administrator's positive whole-minute/day choices.
    pub fn lifetimes(&self, now: i64) -> Result<(i64, i64)> {
        let access = self
            .access_minutes
            .checked_mul(60_000)
            .filter(|n| *n > 0)
            .ok_or_else(invalid)?;
        let refresh = self
            .refresh_days
            .checked_mul(86_400_000)
            .filter(|n| *n > 0)
            .ok_or_else(invalid)?;
        if access > refresh
            || now
                .checked_add(refresh)
                .is_none_or(|deadline| deadline > 8_640_000_000_000_000)
        {
            return Err(invalid());
        }
        Ok((access, refresh))
    }
    pub fn update(value: Option<&str>, patch: &Map<String, Value>) -> Result<Option<String>> {
        let before = Self::read(value)?;
        let mut fields = serde_json::to_value(&before).map_err(|_| invalid())?;
        for (key, field) in [
            ("session_access_minutes", "access_minutes"),
            ("session_refresh_days", "refresh_days"),
            ("session_auto_renew", "auto_renew"),
        ] {
            if let Some(value) = patch.get(key) {
                fields[field] = value.clone();
            }
        }
        let after: Self = serde_json::from_value(fields).map_err(|_| invalid())?;
        after.lifetimes(db::now())?;
        if before == after {
            return Ok(value.map(str::to_owned));
        }
        Ok(Some(serde_json::to_string(&after).map_err(|_| invalid())?))
    }
    pub fn view(&self) -> Value {
        json!({"session_access_minutes":self.access_minutes,"session_refresh_days":self.refresh_days,"session_auto_renew":self.auto_renew})
    }
}
fn invalid() -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        crate::i18n::Message::new("auth.sessionPolicyInvalid"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn settings_are_admin_only_versioned_private_and_audited() {
        use axum::{body::to_bytes, extract::State, response::IntoResponse};
        use sea_orm::{ColumnTrait, QueryFilter, sea_query::Expr};
        let (_dir, s) = crate::agents::tests::setup().await;
        crate::entities::users::Entity::update_many()
            .col_expr(crate::entities::users::Column::IsSuperadmin, Expr::value(1))
            .filter(crate::entities::users::Column::Id.eq(1))
            .exec(&s.db)
            .await
            .unwrap();
        let admin = || {
            super::super::platform::Admin(crate::accounts::auth::Account {
                user: crate::agents::tests::admin(1).user,
                session_id: 0.into(),
            })
        };
        let before = super::super::platform::get_settings(State(s.clone()), admin())
            .await
            .unwrap()
            .0;
        let body = || {
            serde_json::from_value(json!({"version":before["version"],
            "registration_enabled":true,"session_access_minutes":15,
            "session_refresh_days":7,"session_auto_renew":false}))
            .unwrap()
        };
        super::super::platform::update_settings(
            State(s.clone()),
            admin(),
            crate::error::Json(body()),
        )
        .await
        .unwrap();
        let loaded = Configuration::load(&s.db).await.unwrap();
        assert_eq!(loaded.access_minutes, 15);
        assert_eq!(loaded.refresh_days, 7);
        assert!(!loaded.auto_renew);
        let stale = super::super::platform::update_settings(
            State(s.clone()),
            admin(),
            crate::error::Json(body()),
        )
        .await
        .unwrap_err();
        assert_eq!(stale.0, axum::http::StatusCode::CONFLICT);
        let public = super::super::platform::footer(State(s.clone()))
            .await
            .unwrap()
            .into_response();
        let bytes = to_bytes(public.into_body(), 1024 * 1024).await.unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("session_access_minutes"));
        let audit = crate::entities::activity::Entity::find()
            .filter(crate::entities::activity::Column::Resource.eq("platform-settings"))
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let detail: Value = serde_json::from_str(&audit.details).unwrap();
        assert_eq!(detail["session_policy_updated"], true);
        assert_eq!(detail["session_policy"]["before"]["access_minutes"], 30);
        assert_eq!(detail["session_policy"]["after"]["access_minutes"], 15);
        crate::entities::users::Entity::update_many()
            .col_expr(crate::entities::users::Column::IsSuperadmin, Expr::value(0))
            .filter(crate::entities::users::Column::Id.eq(1))
            .exec(&s.db)
            .await
            .unwrap();
        let denied = super::super::platform::update_settings(
            State(s.clone()),
            admin(),
            crate::error::Json(body()),
        )
        .await
        .unwrap_err();
        assert_eq!(denied.0, axum::http::StatusCode::FORBIDDEN);
        assert_eq!(Configuration::load(&s.db).await.unwrap(), loaded);
    }
    #[test]
    fn defaults_and_patches_preserve_unrelated_saves_and_reject_invalid_lifetimes() {
        assert_eq!(
            Configuration::read(None).unwrap().lifetimes(0).unwrap(),
            (1_800_000, 1_209_600_000)
        );
        assert!(
            Configuration::update(None, json!({"site_name":"Other"}).as_object().unwrap())
                .unwrap()
                .is_none()
        );
        assert!(
            Configuration::update(None, Configuration::default().view().as_object().unwrap())
                .unwrap()
                .is_none()
        );
        let saved = Configuration::update(
            None,
            json!({"session_access_minutes":5,"session_refresh_days":2,"session_auto_renew":false})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        let config = Configuration::read(saved.as_deref()).unwrap();
        assert_eq!(config.lifetimes(0).unwrap(), (300_000, 172_800_000));
        assert!(!config.auto_renew);
        for patch in [
            json!({"session_access_minutes":0}),
            json!({"session_refresh_days":-1}),
            json!({"session_access_minutes":1.5}),
            json!({"session_access_minutes":1441,"session_refresh_days":1}),
            json!({"session_refresh_days":i64::MAX}),
            json!({"session_auto_renew":"true"}),
        ] {
            assert!(
                Configuration::update(None, patch.as_object().unwrap()).is_err(),
                "{patch}"
            );
        }
    }
}
