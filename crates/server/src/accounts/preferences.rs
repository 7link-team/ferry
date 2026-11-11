//! Presentation settings belong to the authenticated account, never a workspace.
use super::auth::Account;
use crate::{
    db,
    error::{ApiError, Json, Result},
    state::AppState,
};
use axum::extract::State;
use sea_orm::{Set, entity::prelude::*, sea_query::OnConflict};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod entity {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[sea_orm(table_name = "account_preferences")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: i64,
        pub theme: Option<String>,
        pub locale: Option<String>,
        pub avatar: Option<String>,
        pub updated_at: i64,
        pub updated_by: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Theme {
    palette: String,
    mode: String,
    #[serde(default)]
    custom_light: BTreeMap<String, String>,
    #[serde(default)]
    custom_dark: BTreeMap<String, String>,
}
#[derive(Default, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    avatar: Option<crate::avatar::Avatar>,
    theme: Option<Theme>,
    locale: Option<String>,
}
impl Preferences {
    fn validate(&self) -> Result<()> {
        if let Some(avatar) = &self.avatar { avatar.validate()?; }
        let valid = self.locale.as_ref().is_none_or(|v| {
            ["en-US", "zh-CN", "zh-TW", "ms-MY", "ja-JP", "ko-KR"].contains(&v.as_str())
        }) && self.theme.as_ref().is_none_or(|t| {
            [
                "modern",
                "business",
                "eyeCare",
                "pinkKawaii",
                "neoBrutalism",
                "custom",
            ]
            .contains(&t.palette.as_str())
                && ["light", "dark", "system"].contains(&t.mode.as_str())
                && [&t.custom_light, &t.custom_dark].iter().all(|vars| {
                    vars.len() <= 40
                        && vars
                            .iter()
                            .all(|(k, v)| k.starts_with("--") && k.len() <= 40 && v.len() <= 200)
                })
        });
        if !valid {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("request.jsonData"),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn account(id: i64) -> Account {
        Account {
            user: crate::accounts::auth::User {
                id: id.into(),
                email: "user@example.invalid".into(),
                name: "User".into(),
                is_superadmin: false,
            },
            session_id: 1.into(),
        }
    }
    #[tokio::test]
    async fn persists_per_account_and_partial_writes_preserve_other_fields() {
        let (_dir, s) = crate::agents::tests::setup().await;
        assert!(
            read(State(s.clone()), account(1))
                .await
                .unwrap()
                .0
                .theme
                .is_none()
        );
        let input = serde_json::json!({"theme":{"palette":"custom","mode":"dark","customLight":{},"customDark":{"--primary":"#123456"}},"locale":"ms-MY"});
        update(
            State(s.clone()),
            account(1),
            Json(serde_json::from_value(input).unwrap()),
        )
        .await
        .unwrap();
        let avatar: crate::avatar::Avatar = serde_json::from_value(serde_json::json!({"style":"emoji","emoji":"🤖"})).unwrap();
        save_avatar(&s.db, 1, &avatar).await.unwrap();
        update(
            State(s.clone()),
            account(1),
            Json(serde_json::from_value(serde_json::json!({"locale":"en-US"})).unwrap()),
        )
        .await
        .unwrap();
        let stored = read(State(s.clone()), account(1)).await.unwrap().0;
        assert_eq!(stored.locale.as_deref(), Some("en-US"));
        assert_eq!(stored.avatar.unwrap().emoji.as_deref(), Some("🤖"));
        assert!(read(State(s.clone()), account(2)).await.unwrap().0.avatar.is_none());
        assert_eq!(stored.theme.unwrap().custom_dark["--primary"], "#123456");
        assert!(
            read(State(s.clone()), account(2))
                .await
                .unwrap()
                .0
                .locale
                .is_none()
        );
        let row = entity::Entity::find_by_id(1)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.updated_by, 1);
        assert!(row.updated_at > 0);
    }
    #[test]
    fn rejects_unknown_fields_and_invalid_preferences() {
        assert!(serde_json::from_value::<Preferences>(serde_json::json!({"user_id":2})).is_err());
        for input in [
            serde_json::json!({"locale":"invalid"}),
            serde_json::json!({"theme":{"palette":"unknown","mode":"dark"}}),
            serde_json::json!({"theme":{"palette":"modern","mode":"invalid"}}),
        ] {
            assert!(
                serde_json::from_value::<Preferences>(input)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
    }
}
pub(crate) async fn avatars(conn: &impl ConnectionTrait, ids: Vec<i64>) -> Result<std::collections::HashMap<i64, serde_json::Value>> {
    let rows = entity::Entity::find().filter(entity::Column::UserId.is_in(ids)).all(conn).await?;
    Ok(rows.into_iter().filter_map(|row| Some((row.user_id, serde_json::from_str(row.avatar.as_deref()?).ok()?))).collect())
}
pub(crate) async fn save_avatar(conn: &impl ConnectionTrait, id: i64, avatar: &crate::avatar::Avatar) -> Result<()> {
    avatar.validate()?;
    entity::Entity::insert(entity::ActiveModel {
        user_id: Set(id), theme: Set(None), locale: Set(None),
        avatar: Set(Some(serde_json::to_string(avatar).expect("serializable avatar"))),
        updated_at: Set(db::now()), updated_by: Set(id),
    }).on_conflict(OnConflict::column(entity::Column::UserId)
        .update_columns([entity::Column::Avatar, entity::Column::UpdatedAt, entity::Column::UpdatedBy]).to_owned())
        .exec(conn).await?;
    Ok(())
}
pub async fn read(State(s): State<AppState>, account: Account) -> Result<Json<Preferences>> {
    let row = entity::Entity::find_by_id(i64::from(account.user.id))
        .one(&s.db)
        .await?;
    Ok(Json(match row {
        Some(row) => Preferences {
            avatar: row.avatar.and_then(|v| serde_json::from_str(&v).ok()),
            theme: row.theme.and_then(|v| serde_json::from_str(&v).ok()),
            locale: row.locale,
        },
        None => Preferences::default(),
    }))
}
pub async fn update(
    State(s): State<AppState>,
    account: Account,
    Json(body): Json<Preferences>,
) -> Result<Json<serde_json::Value>> {
    body.validate()?;
    let mut columns = vec![entity::Column::UpdatedAt, entity::Column::UpdatedBy];
    if body.theme.is_some() {
        columns.push(entity::Column::Theme);
    }
    if body.locale.is_some() {
        columns.push(entity::Column::Locale);
    }
    if body.avatar.is_some() { columns.push(entity::Column::Avatar); }
    let id = i64::from(account.user.id);
    entity::Entity::insert(entity::ActiveModel {
        user_id: Set(id),
        theme: Set(body
            .theme
            .map(|t| serde_json::to_string(&t).expect("serializable theme"))),
        locale: Set(body.locale),
        avatar: Set(body.avatar.map(|avatar| serde_json::to_string(&avatar).expect("serializable avatar"))),
        updated_at: Set(db::now()),
        updated_by: Set(id),
    })
    .on_conflict(
        OnConflict::column(entity::Column::UserId)
            .update_columns(columns)
            .to_owned(),
    )
    .exec(&s.db)
    .await?;
    Ok(Json(serde_json::json!({"ok": true})))
}
