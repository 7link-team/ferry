//! Administrator-owned defaults, read afresh for every conversation turn/tool call.
use crate::{
    entities::platform_state,
    error::{ApiError, Result},
    i18n::Message,
};
use sea_orm::{ConnectionTrait, EntityTrait};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub const DEFAULT_INSTRUCTIONS: &str =
    include_str!("../../../../src/shared/ai/conversation-instructions.md");
pub const DEFAULT_INTRODUCTION_INSTRUCTIONS: &str = "Briefly greet the user, introduce your actual name and configured role/description, explain the help grounded in that configuration and your effective permissions, and ask what the user wants. Never claim to have performed checks or actions, and never invent capabilities.";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Configuration {
    pub instructions: String,
    pub tool_response_limit_kib: usize,
    pub default_approval_mode: String,
    pub introduction_enabled: bool,
    pub introduction_instructions: String,
}
impl Default for Configuration {
    fn default() -> Self {
        Self {
            instructions: DEFAULT_INSTRUCTIONS.trim().into(),
            tool_response_limit_kib: 1024,
            default_approval_mode: "request".into(),
            introduction_enabled: true,
            introduction_instructions: DEFAULT_INTRODUCTION_INSTRUCTIONS.into(),
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
        config.validate()?;
        Ok(config)
    }
    pub async fn load(conn: &impl ConnectionTrait) -> Result<Self> {
        let row = platform_state::Entity::find_by_id(1)
            .one(conn)
            .await?
            .ok_or_else(ApiError::missing)?;
        Self::read(row.conversation_config.as_deref())
    }
    fn validate(&self) -> Result<()> {
        if self.introduction_instructions.len() > 16 * 1024
            || self
                .introduction_instructions
                .chars()
                .any(|c| c.is_control() && !['\n', '\r', '\t'].contains(&c))
            || self.instructions.len() > 16 * 1024
            || self
                .instructions
                .chars()
                .any(|c| c.is_control() && !['\n', '\r', '\t'].contains(&c))
            || !(256..=8192).contains(&self.tool_response_limit_kib)
            || !["request", "assisted", "yolo"].contains(&self.default_approval_mode.as_str())
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn update(value: Option<&str>, patch: &Map<String, Value>) -> Result<String> {
        let mut values = json!(Self::read(value)?);
        for (key, field) in [
            ("conversation_instructions", "instructions"),
            (
                "conversation_tool_response_limit_kib",
                "tool_response_limit_kib",
            ),
            ("conversation_approval_mode", "default_approval_mode"),
            ("conversation_introduction_enabled", "introduction_enabled"),
            (
                "conversation_introduction_instructions",
                "introduction_instructions",
            ),
        ] {
            if let Some(value) = patch.get(key) {
                values[field] = value.clone();
            }
        }
        let config: Self = serde_json::from_value(values).map_err(|_| invalid())?;
        config.validate()?;
        serde_json::to_string(&config).map_err(|error| std::io::Error::other(error).into())
    }
    pub fn view(&self) -> Value {
        json!({"conversation_instructions":self.instructions,
            "conversation_instructions_default":DEFAULT_INSTRUCTIONS.trim(),
            "conversation_tool_response_limit_kib":self.tool_response_limit_kib,
            "conversation_approval_mode":self.default_approval_mode,
            "conversation_introduction_enabled":self.introduction_enabled,
            "conversation_introduction_instructions":self.introduction_instructions,
            "conversation_introduction_instructions_default":DEFAULT_INTRODUCTION_INSTRUCTIONS})
    }
}
fn invalid() -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        Message::new("platform.conversationInvalid"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn administrator_policy_roundtrip_is_private_versioned_and_rechecked() {
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
            serde_json::from_value(json!({"version":before["version"],"registration_enabled":true,
            "conversation_instructions":"Internal policy acceptance marker","conversation_tool_response_limit_kib":4096,"conversation_approval_mode":"assisted",
            "conversation_introduction_enabled":false,"conversation_introduction_instructions":"Private introduction settings marker"})).unwrap()
        };
        super::super::platform::update_settings(
            State(s.clone()),
            admin(),
            crate::error::Json(body()),
        )
        .await
        .unwrap();
        let loaded = Configuration::load(&s.db).await.unwrap();
        assert_eq!(loaded.instructions, "Internal policy acceptance marker");
        assert_eq!(loaded.default_approval_mode, "assisted");
        assert!(!loaded.introduction_enabled);
        assert_eq!(
            loaded.introduction_instructions,
            "Private introduction settings marker"
        );
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
        assert!(!String::from_utf8_lossy(&bytes).contains("Internal policy acceptance marker"));
        assert!(!String::from_utf8_lossy(&bytes).contains("conversation_instructions"));
        assert!(!String::from_utf8_lossy(&bytes).contains("Private introduction settings marker"));
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
    fn defaults_edit_clear_restore_and_bounded_configuration() {
        let original = Configuration::default();
        assert_eq!(original.tool_response_limit_kib, 1024);
        assert_eq!(original.default_approval_mode, "request");
        assert!(original.introduction_enabled);
        assert!(
            Configuration::read(Some("{}"))
                .unwrap()
                .introduction_enabled
        );
        let intro = Configuration::update(None, json!({"conversation_introduction_enabled":false,"conversation_introduction_instructions":""}).as_object().unwrap()).unwrap();
        let intro = Configuration::read(Some(&intro)).unwrap();
        assert!(!intro.introduction_enabled);
        assert!(intro.introduction_instructions.is_empty());
        assert!(original.instructions.contains("ferry_ask_user"));
        let edited = Configuration::update(None, json!({"conversation_instructions":"Custom policy", "conversation_tool_response_limit_kib":4096,"conversation_approval_mode":"assisted"}).as_object().unwrap()).unwrap();
        let read = Configuration::read(Some(&edited)).unwrap();
        assert_eq!(read.instructions, "Custom policy");
        assert_eq!(read.tool_response_limit_kib, 4096);
        let cleared = Configuration::update(
            Some(&edited),
            json!({"conversation_instructions":""}).as_object().unwrap(),
        )
        .unwrap();
        assert_eq!(
            Configuration::read(Some(&cleared)).unwrap().instructions,
            ""
        );
        let restored = Configuration::update(
            Some(&cleared),
            json!({"conversation_instructions":DEFAULT_INSTRUCTIONS.trim()})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            Configuration::read(Some(&restored)).unwrap().instructions,
            original.instructions
        );
        for patch in [
            json!({"conversation_instructions":"x".repeat(16385)}),
            json!({"conversation_instructions":"bad\u{0000}"}),
            json!({"conversation_introduction_instructions":"x".repeat(16385)}),
            json!({"conversation_introduction_instructions":"bad\u{0000}"}),
            json!({"conversation_introduction_enabled":"true"}),
            json!({"conversation_tool_response_limit_kib":255}),
            json!({"conversation_tool_response_limit_kib":8193}),
            json!({"conversation_tool_response_limit_kib":"1024"}),
            json!({"conversation_approval_mode":"unknown"}),
        ] {
            assert!(
                Configuration::update(None, patch.as_object().unwrap()).is_err(),
                "{patch}"
            );
        }
    }
}
