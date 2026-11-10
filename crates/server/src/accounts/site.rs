//! Public deployment identity, edited through the existing administrator settings.
use crate::error::{ApiError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Configuration {
    pub site_name: String,
    pub site_title_prefix_enabled: bool,
    pub site_title_visible_prefix: String,
    pub site_title_hidden_prefix: String,
}
impl Default for Configuration {
    fn default() -> Self {
        Self {
            site_name: String::new(),
            site_title_prefix_enabled: false,
            site_title_visible_prefix: "☀️".into(),
            site_title_hidden_prefix: "🌙".into(),
        }
    }
}
impl Configuration {
    pub fn read(value: Option<&str>) -> Result<Self> {
        Ok(value
            .map(serde_json::from_str)
            .transpose()
            .map_err(std::io::Error::other)?
            .unwrap_or_default())
    }
    pub fn update(value: Option<&str>, patch: &Map<String, Value>) -> Result<String> {
        let mut fields = serde_json::json!(Self::read(value)?);
        for key in [
            "site_name",
            "site_title_prefix_enabled",
            "site_title_visible_prefix",
            "site_title_hidden_prefix",
        ] {
            if let Some(value) = patch.get(key) {
                fields[key] = value.clone();
            }
        }
        let invalid = || {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("platform.siteInvalid"),
            )
        };
        let mut config: Self = serde_json::from_value(fields).map_err(|_| invalid())?;
        for (text, limit) in [
            (&mut config.site_name, 120),
            (&mut config.site_title_visible_prefix, 32),
            (&mut config.site_title_hidden_prefix, 32),
        ] {
            if text.chars().any(char::is_control) {
                return Err(invalid());
            }
            *text = text.trim().to_owned();
            if text.chars().count() > limit {
                return Err(invalid());
            }
        }
        Ok(serde_json::to_string(&config).map_err(std::io::Error::other)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn migration_preserves_existing_footer_scripts_and_is_repeatable() {
        use sea_orm::{ConnectionTrait, Database, DbBackend};
        use sea_orm_migration::{MigrationTrait, SchemaManager};
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("CREATE TABLE platform_state (id BIGINT PRIMARY KEY, footer_html TEXT NULL, page_scripts TEXT NULL)").await.unwrap();
        db.execute_unprepared("INSERT INTO platform_state VALUES (1, 'footer', 'scripts')")
            .await
            .unwrap();
        let manager = SchemaManager::new(&db);
        let migration = crate::migration::PlatformSite;
        migration.up(&manager).await.unwrap();
        migration.up(&manager).await.unwrap();
        let row = db
            .query_one_raw(sea_orm::Statement::from_string(
                DbBackend::Sqlite,
                "SELECT * FROM platform_state".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<String>("", "footer_html").unwrap(), "footer");
        assert_eq!(
            row.try_get::<String>("", "page_scripts").unwrap(),
            "scripts"
        );
        assert_eq!(
            row.try_get::<Option<String>>("", "site_config").unwrap(),
            None
        );
        assert!(migration.down(&manager).await.is_err());
    }
    #[test]
    fn defaults_partial_update_clear_and_unicode_limits() {
        assert_eq!(Configuration::read(None).unwrap(), Configuration::default());
        assert!(!Configuration::read(None).unwrap().site_title_prefix_enabled);
        assert!(
            !Configuration::read(Some("{\"site_name\":\"Example\"}"))
                .unwrap()
                .site_title_prefix_enabled
        );
        let saved = Configuration::update(
            None,
            json!({"site_name":" Example ","site_title_visible_prefix":"🌞"})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        let updated = Configuration::read(Some(&saved)).unwrap();
        assert_eq!(updated.site_name, "Example");
        assert_eq!(updated.site_title_hidden_prefix, "🌙");
        let saved = Configuration::update(
            Some(&saved),
            json!({"site_name":"", "site_title_prefix_enabled":false})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        let updated = Configuration::read(Some(&saved)).unwrap();
        assert!(updated.site_name.is_empty());
        assert!(!updated.site_title_prefix_enabled);
        assert_eq!(updated.site_title_visible_prefix, "🌞");
        for (key, limit) in [
            ("site_name", 120),
            ("site_title_visible_prefix", 32),
            ("site_title_hidden_prefix", 32),
        ] {
            assert!(
                Configuration::update(None, json!({key:"🌙".repeat(limit)}).as_object().unwrap())
                    .is_ok()
            );
            assert!(
                Configuration::update(None, json!({key:"🌙".repeat(limit+1)}).as_object().unwrap())
                    .is_err()
            );
            assert!(
                Configuration::update(None, json!({key:"line\nbreak"}).as_object().unwrap())
                    .is_err()
            );
            assert!(Configuration::update(None, json!({key:12}).as_object().unwrap()).is_err());
        }
        assert!(
            Configuration::update(
                None,
                json!({"site_title_prefix_enabled":"yes"})
                    .as_object()
                    .unwrap()
            )
            .is_err()
        );
    }
}
