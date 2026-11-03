//! Workspace preferences and shared enumeration dictionaries.
pub(crate) mod files;
use crate::error::{Json, Path, Query};
use crate::{
    accounts::workspace::Scope,
    db,
    entities::{dictionaries, dictionary_items, settings, workspaces},
    error::{ApiError, Result},
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{extract::State, http::StatusCode};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set, TransactionTrait,
    sea_query::{Expr, ExprTrait},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

mod states;
mod values;

#[cfg(test)]
mod locale_tests {
    use super::*;
    use crate::i18n::{Locale, with_locale};

    #[tokio::test]
    async fn localized_builtin_names_roundtrip_across_reader_locales_without_rewriting_user_labels()
    {
        let (dir, db) = crate::lock::tests::database().await;
        let s = std::sync::Arc::new(crate::state::State {
            ids: std::sync::Arc::new(
                crate::snowflake::Generator::new(1, tokio::time::Instant::now().into_std())
                    .unwrap(),
            ),
            db,
            data_dir: dir.path().into(),
            instance_id: "maintenance-test".into(),
            server_mode: false,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), false).unwrap(),
            host_monitor: Default::default(),
            cluster: None,
        });
        workspaces::Entity::insert(workspaces::ActiveModel {
            id: Set(1),
            slug: Set("maintenance-test".into()),
            name: Set("Workspace".into()),
            owner_id: Set(1),
            join_code: Set("maintenance-test".into()),
            pipeline_run_number: Set(0),
            created_at: Set(0),
            updated_at: Set(0),
        })
        .exec(&s.db)
        .await
        .unwrap();
        seed(&s, &s.db, 1.into()).await.unwrap();
        let scope = || Scope {
            workspace_id: 1.into(),
            user: crate::accounts::auth::User {
                id: 1.into(),
                email: "test@example.invalid".into(),
                name: "User 原文".into(),
                is_superadmin: false,
            },
            permissions: ["settings.update".to_owned()].into_iter().collect(),
        };
        let row = settings::Entity::find()
            .filter(settings::Column::Key.eq("default_page_size"))
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let id = Id::from(row.id);
        for locale in Locale::ALL {
            let current = settings::Entity::find_by_id(row.id)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap();
            let mut view = with_locale(locale, async { setting_json(current.clone()) }).await;
            assert_eq!(view["name"], setting_name("default_page_size", locale));
            view["value_json"] = json!("21");
            let body: SettingBody = serde_json::from_value(view).unwrap();
            // A user may switch language after loading the draft and before saving it.
            let Json(saved) = with_locale(
                Locale::JaJp,
                update_setting(
                    State(s.clone()),
                    scope(),
                    Path(("1".into(), id)),
                    Json(body),
                ),
            )
            .await
            .unwrap();
            assert_eq!(saved["version"], current.version + 1);
            let stored = settings::Entity::find_by_id(row.id)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stored.name, row.name);
            assert_eq!(stored.value, 21);
            assert_eq!(stored.version, current.version + 1);
            with_locale(locale, seed(&s, &s.db, 1.into()))
                .await
                .unwrap();
            let dictionary = dictionaries::Entity::find()
                .filter(dictionaries::Column::Key.eq("environment"))
                .one(&s.db)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                dictionary.name, "Environment",
                "existing editable seed labels stay literal"
            );
        }
        let current = settings::Entity::find_by_id(row.id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let mut view = setting_json(current);
        view["name"] = json!("Custom heartbeat label");
        view["category"] = json!("operations");
        view["description"] = json!("User description");
        update_setting(
            State(s.clone()),
            scope(),
            Path(("1".into(), id)),
            Json(serde_json::from_value(view).unwrap()),
        )
        .await
        .unwrap();
        for locale in Locale::ALL {
            with_locale(locale, seed(&s, &s.db, 1.into()))
                .await
                .unwrap();
            let stored = settings::Entity::find_by_id(row.id)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap();
            let view = with_locale(locale, async { setting_json(stored) }).await;
            assert_eq!(view["name"], "Custom heartbeat label");
            assert_eq!(view["category"], "operations");
            assert_eq!(view["description"], "User description");
        }
        let mut custom = row;
        custom.key = "custom".into();
        custom.name = "用户 {name} 原文".into();
        for locale in Locale::ALL {
            assert_eq!(
                with_locale(locale, async { setting_json(custom.clone()) }).await["name"],
                custom.name
            );
        }
    }
}

const SETTINGS: &[(&str, &str, i64, i64, i64)] = &[
    ("default_page_size", "默认每页条数", 20, 1, 200),
    (
        "heartbeat_timeout_seconds",
        "Runner 心跳超时（秒）",
        60,
        40,
        600,
    ),
    (
        "history_retention_days",
        "执行历史保留天数（0 为永久）",
        0,
        0,
        36500,
    ),
    (
        "repository_event_retention_days",
        "仓库事件保留天数（0 为永久）",
        0,
        0,
        36500,
    ),
    (
        "pipeline_concurrent_runs",
        "同时执行的流水线数（0 为不限）",
        10,
        0,
        10000,
    ),
];
fn setting_category(key: &str) -> &'static str {
    match key {
        "heartbeat_timeout_seconds" => "hosts",
        "history_retention_days" => "history",
        "pipeline_concurrent_runs" => "pipelines",
        "repository_event_retention_days" => "repositories",
        _ => "general",
    }
}
const SPEC: Spec = Spec {
    search: &["key", "name", "description"],
    filters: &["category", "value_type"],
    sortable: &["key", "name", "category", "updated_at"],
    default_sort: "key",
};
const DICT_SPEC: Spec = Spec {
    search: &["key", "name", "description"],
    filters: &["category", "value_type"],
    sortable: &["key", "name", "category", "created_at", "updated_at"],
    default_sort: "key",
};

pub async fn setting(conn: &impl ConnectionTrait, workspace: Id, key: &str) -> Result<i64> {
    let fallback = SETTINGS
        .iter()
        .find(|v| v.0 == key)
        .ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("maintenance.settingUnknown"),
            )
        })?
        .2;
    Ok(settings::Entity::find()
        .filter(settings::Column::WorkspaceId.eq(workspace))
        .filter(settings::Column::Key.eq(key))
        .one(conn)
        .await?
        .map(|r| r.value)
        .unwrap_or(fallback))
}
pub async fn seed(s: &AppState, conn: &impl ConnectionTrait, workspace: Id) -> Result<()> {
    let now = db::now();
    for &(key, name, value, _, _) in SETTINGS {
        if settings::Entity::find()
            .filter(settings::Column::WorkspaceId.eq(workspace))
            .filter(settings::Column::Key.eq(key))
            .one(conn)
            .await?
            .is_none()
        {
            settings::Entity::insert(settings::ActiveModel {
                id: Set(s.next_id()?.into()),
                workspace_id: Set(workspace.into()),
                key: Set(key.into()),
                value: Set(value),
                name: Set(name.into()),
                description: Set(String::new()),
                category: Set(setting_category(key).into()),
                value_type: Set("num".into()),
                value_json: Set(None),
                schema_json: Set(None),
                version: Set(1),
                created_at: Set(now),
                updated_at: Set(now),
            })
            .exec(conn)
            .await?;
        }
        // Split the legacy catch-all category without replacing custom categories.
        settings::Entity::update_many()
            .col_expr(
                settings::Column::Category,
                Expr::value(setting_category(key)),
            )
            .filter(settings::Column::WorkspaceId.eq(workspace))
            .filter(settings::Column::Key.eq(key))
            .filter(settings::Column::Category.eq("general"))
            .exec(conn)
            .await?;
        // Added metadata must match the displayed built-in names for list search.
        // Preserve the operational value, optimistic version and timestamps.
        settings::Entity::update_many()
            .col_expr(settings::Column::Name, Expr::value(name))
            .filter(settings::Column::WorkspaceId.eq(workspace))
            .filter(settings::Column::Key.eq(key))
            .filter(settings::Column::Name.eq(""))
            .exec(conn)
            .await?;
    }
    let mut builtin_dictionaries = vec![
        (
            "host-state",
            crate::i18n::message("maintenance.hostStateName"),
            vec![
                ("online", crate::i18n::message("maintenance.online")),
                ("offline", crate::i18n::message("maintenance.offline")),
                ("unknown", crate::i18n::message("maintenance.unknown")),
            ],
        ),
        (
            "environment",
            crate::i18n::message("maintenance.environmentName"),
            vec![
                ("dev", crate::i18n::message("maintenance.development")),
                ("staging", crate::i18n::message("maintenance.staging")),
                ("production", crate::i18n::message("maintenance.production")),
            ],
        ),
        (
            "app-category",
            crate::i18n::message("maintenance.appCategoryName"),
            vec![
                ("web", crate::i18n::message("maintenance.webService")),
                ("worker", crate::i18n::message("maintenance.backgroundTask")),
                ("database", crate::i18n::message("maintenance.database")),
            ],
        ),
    ];
    builtin_dictionaries.extend(states::definitions());
    for (key, name, items) in builtin_dictionaries {
        if dictionaries::Entity::find()
            .filter(dictionaries::Column::WorkspaceId.eq(workspace))
            .filter(dictionaries::Column::Key.eq(key))
            .one(conn)
            .await?
            .is_none()
        {
            let id = s.next_id()?;
            dictionaries::Entity::insert(dictionaries::ActiveModel {
                id: Set(id.into()),
                workspace_id: Set(workspace.into()),
                key: Set(key.into()),
                name: Set(name.into()),
                category: Set("general".into()),
                value_type: Set("selection".into()),
                value_json: Set(None),
                schema_json: Set(None),
                description: Set(String::new()),
                builtin: Set(1),
                version: Set(1),
                created_at: Set(now),
                updated_at: Set(now),
            })
            .exec(conn)
            .await?;
            write_items(
                s,
                conn,
                workspace,
                id,
                &items
                    .into_iter()
                    .map(|(v, l)| Item {
                        value: v.into(),
                        label: l.into(),
                        color: None,
                    })
                    .collect::<Vec<_>>(),
            )
            .await?;
        }
    }
    Ok(())
}
pub async fn prepare(s: &AppState) -> Result<()> {
    for row in workspaces::Entity::find().all(&s.db).await? {
        let tx = s.db.begin().await?;
        seed(s, &tx, row.id.into()).await?;
        tx.commit().await?;
    }
    Ok(())
}
fn setting_name(key: &str, locale: crate::i18n::Locale) -> String {
    crate::i18n::render(
        locale,
        &crate::i18n::Message::new(match key {
            "default_page_size" => "maintenance.pageSize",
            "heartbeat_timeout_seconds" => "maintenance.heartbeatTimeout",
            "history_retention_days" => "maintenance.historyRetention",
            "repository_event_retention_days" => "maintenance.repositoryRetention",
            "pipeline_concurrent_runs" => "maintenance.pipelineConcurrency",
            _ => return key.to_owned(),
        }),
    )
}

fn is_default_setting_name(key: &str, name: &str) -> bool {
    SETTINGS.iter().any(|d| d.0 == key && d.1 == name)
        || crate::i18n::Locale::ALL
            .into_iter()
            .any(|locale| name == setting_name(key, locale))
}

fn setting_json(r: settings::Model) -> Value {
    let def = SETTINGS.iter().find(|v| v.0 == r.key);
    let raw = if def.is_some() {
        r.value.to_string()
    } else {
        r.value_json.unwrap_or_else(|| r.value.to_string())
    };
    let schema = def
        .map(|d| json!({"min":d.3,"max":d.4,"decimals":0}).to_string())
        .unwrap_or_else(|| r.schema_json.unwrap_or_else(|| "{}".into()));
    let name = if def.is_some() && is_default_setting_name(&r.key, &r.name) {
        setting_name(&r.key, crate::i18n::current_locale())
    } else {
        r.name
    };
    json!({"id":Id::from(r.id),"key":r.key,"name":name,
        "description":r.description,"category":r.category,"value_type":r.value_type,
        "value_json":raw,"schema_json":schema,"value_summary":values::summary(&raw),
        "builtin":def.is_some(),"type":if r.value_type=="num" {"integer"} else {r.value_type.as_str()},
        "value":r.value,"default":def.map(|d|d.2),"min":def.map(|d|d.3),"max":def.map(|d|d.4),
        "version":r.version,"created_at":r.created_at,"updated_at":r.updated_at})
}
pub async fn list_settings(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("settings.read")?;
    let q = ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) = resource::list::<settings::Entity, settings::Model>(
        &s.db,
        &SPEC,
        settings::Entity::find(),
        &scope.workspace_id,
        &q,
    )
    .await?;
    Ok(Json(
        json!({"items":rows.into_iter().map(setting_json).collect::<Vec<_>>(),"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
async fn setting_row(
    conn: &impl ConnectionTrait,
    workspace: Id,
    id: Id,
) -> Result<settings::Model> {
    settings::Entity::find_by_id(i64::from(id))
        .filter(settings::Column::WorkspaceId.eq(workspace))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
pub async fn get_setting(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("settings.read")?;
    Ok(Json(setting_json(
        setting_row(&s.db, scope.workspace_id, id).await?,
    )))
}
#[derive(Deserialize)]
pub struct SettingBody {
    #[serde(default)]
    value: Option<i64>,
    #[serde(default)]
    version: i64,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    value_type: Option<String>,
    #[serde(default)]
    value_json: Option<String>,
    #[serde(default)]
    schema_json: Option<String>,
}

fn metadata(key: &str, name: &str, category: &str, description: &str) -> Result<()> {
    if key.is_empty()
        || key.len() > 60
        || !key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        || name.trim().is_empty()
        || name.chars().count() > 100
        || category.trim().is_empty()
        || category.chars().count() > 100
        || description.chars().count() > 10000
        || [key, name, category, description]
            .iter()
            .any(|s| s.contains('\0'))
    {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("maintenance.metadataInvalid"),
        ));
    }
    Ok(())
}

pub async fn create_setting(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<SettingBody>,
) -> Result<(StatusCode, Json<Value>)> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let result = create_setting_in(&s, &tx, &scope, body).await?;
    tx.commit().await?;
    Ok(result)
}

pub(crate) async fn create_setting_in(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    scope: &Scope,
    body: SettingBody,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("settings.create")?;
    let key = body.key.unwrap_or_default();
    let name = body.name.unwrap_or_default();
    let category = body.category.unwrap_or_else(|| "general".into());
    let description = body.description.unwrap_or_default();
    metadata(&key, &name, &category, &description)?;
    if SETTINGS.iter().any(|d| d.0 == key) {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("maintenance.systemKeyReserved"),
        ));
    }
    let kind = body.value_type.unwrap_or_else(|| "num".into());
    let raw = body
        .value_json
        .unwrap_or_else(|| body.value.unwrap_or_default().to_string());
    let typed = values::validate(
        &kind,
        &raw,
        &body.schema_json.unwrap_or_else(|| "{}".into()),
    )?;
    if matches!(kind.as_str(), "image" | "video" | "file") {
        crate::maintenance::files::validate_references(
            s,
            scope.workspace_id,
            "settings",
            &typed.value,
            &typed.schema,
        )
        .await?;
    }
    lock(tx, scope.workspace_id).await?;
    if settings::Entity::find()
        .filter(settings::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(settings::Column::Key.eq(&key))
        .one(tx)
        .await?
        .is_some()
    {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new("maintenance.settingExists"),
        ));
    }
    let id = s.next_id()?;
    let now = db::now();
    settings::Entity::insert(settings::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(scope.workspace_id.into()),
        key: Set(key),
        name: Set(name),
        description: Set(description),
        category: Set(category),
        value_type: Set(kind),
        value: Set(typed.value.as_i64().unwrap_or_default()),
        value_json: Set(Some(typed.value.to_string())),
        schema_json: Set(Some(typed.schema.to_string())),
        version: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(tx)
    .await?;
    crate::hosts::activity_in(s, tx, scope, "create", "settings", &id, json!({})).await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id,"version":1}))))
}
pub async fn update_setting(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<SettingBody>,
) -> Result<Json<Value>> {
    scope.require("settings.update")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let row = setting_row(&tx, scope.workspace_id, id).await?;
    let def = SETTINGS.iter().find(|v| v.0 == row.key);
    if body.key.as_ref().is_some_and(|k| k != &row.key) {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("maintenance.settingKeyImmutable"),
        ));
    }
    if let Some(d) = def {
        let fixed_schema = json!({"min":d.3,"max":d.4,"decimals":0});
        if body
            .schema_json
            .as_deref()
            .is_some_and(|v| serde_json::from_str::<Value>(v).ok().as_ref() != Some(&fixed_schema))
        {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("maintenance.systemMetadataImmutable"),
            ));
        }
    }
    let incoming_name = body.name.unwrap_or_else(|| row.name.clone());
    let name = if let Some(d) = def.filter(|d| is_default_setting_name(d.0, &incoming_name)) {
        d.1.to_owned()
    } else {
        incoming_name
    };
    let description = body.description.unwrap_or_else(|| row.description.clone());
    let category = body.category.unwrap_or_else(|| row.category.clone());
    metadata(&row.key, &name, &category, &description)?;
    let kind = body.value_type.unwrap_or_else(|| row.value_type.clone());
    if def.is_some() && kind != "num" {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("maintenance.systemTypeImmutable"),
        ));
    }
    let schema = if let Some(d) = def {
        json!({"min":d.3,"max":d.4,"decimals":0}).to_string()
    } else {
        body.schema_json
            .unwrap_or_else(|| row.schema_json.clone().unwrap_or_else(|| "{}".into()))
    };
    let raw = body
        .value_json
        .or_else(|| body.value.map(|n| n.to_string()))
        .unwrap_or_else(|| {
            row.value_json
                .clone()
                .unwrap_or_else(|| row.value.to_string())
        });
    let typed = values::validate(&kind, &raw, &schema)?;
    if matches!(kind.as_str(), "image" | "video" | "file") {
        crate::maintenance::files::validate_references(
            &s,
            scope.workspace_id,
            "settings",
            &typed.value,
            &typed.schema,
        )
        .await?;
    }
    let value = if def.is_some() {
        typed.value.as_i64().ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("maintenance.systemIntegerRequired"),
            )
        })?
    } else {
        typed.value.as_i64().unwrap_or_default()
    };
    let n = settings::Entity::update_many()
        .col_expr(settings::Column::Value, Expr::value(value))
        .col_expr(settings::Column::Name, Expr::value(name))
        .col_expr(settings::Column::Description, Expr::value(description))
        .col_expr(settings::Column::Category, Expr::value(category))
        .col_expr(settings::Column::ValueType, Expr::value(kind))
        .col_expr(
            settings::Column::ValueJson,
            Expr::value(typed.value.to_string()),
        )
        .col_expr(
            settings::Column::SchemaJson,
            Expr::value(typed.schema.to_string()),
        )
        .col_expr(
            settings::Column::Version,
            Expr::col(settings::Column::Version).add(1),
        )
        .col_expr(settings::Column::UpdatedAt, Expr::value(db::now()))
        .filter(settings::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(settings::Column::Id.eq(id))
        .filter(settings::Column::Version.eq(body.version))
        .exec(&tx)
        .await?
        .rows_affected;
    if n == 0 {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new("maintenance.settingChanged"),
        ));
    }
    let audit = if def.is_some() {
        json!({"key":row.key,"value":value})
    } else {
        json!({"key":row.key})
    };
    crate::hosts::activity_in(&s, &tx, &scope, "update", "settings", &id, audit).await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"version":body.version+1})))
}

pub async fn delete_setting(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<StatusCode> {
    scope.require("settings.delete")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let old = setting_row(&tx, scope.workspace_id, id).await?;
    if SETTINGS.iter().any(|d| d.0 == old.key) {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("maintenance.systemSettingDeleteForbidden"),
        ));
    }
    settings::Entity::delete_many()
        .filter(settings::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(settings::Column::Id.eq(id))
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(&s, &tx, &scope, "delete", "settings", &id, json!({})).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Item {
    value: String,
    label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    color: Option<String>,
}
#[derive(Deserialize)]
pub struct DictionaryBody {
    key: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    items: Option<String>,
    #[serde(default = "default_category")]
    category: String,
    #[serde(default = "default_dictionary_type")]
    value_type: String,
    #[serde(default)]
    value_json: Option<String>,
    #[serde(default)]
    schema_json: Option<String>,
    #[serde(default)]
    version: i64,
}
fn default_category() -> String {
    "general".into()
}
fn default_dictionary_type() -> String {
    "selection".into()
}

fn dictionary_value(body: &DictionaryBody) -> Result<values::TypedValue> {
    metadata(&body.key, &body.name, &body.category, &body.description)?;
    let raw = if matches!(body.value_type.as_str(), "selection" | "tab") {
        body.items.as_deref().or(body.value_json.as_deref())
    } else {
        body.value_json.as_deref()
    };
    values::validate(
        &body.value_type,
        raw.unwrap_or("[]"),
        body.schema_json.as_deref().unwrap_or("{}"),
    )
}
fn validate(body: &DictionaryBody) -> Result<Vec<Item>> {
    let typed = dictionary_value(body)?;
    if matches!(body.value_type.as_str(), "selection" | "tab") {
        serde_json::from_value(values::options(&typed.value)).map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("maintenance.optionsInvalid"),
            )
        })
    } else {
        Ok(Vec::new())
    }
}
async fn write_items(
    s: &AppState,
    conn: &impl ConnectionTrait,
    workspace: Id,
    id: Id,
    items: &[Item],
) -> Result<()> {
    dictionary_items::Entity::delete_many()
        .filter(dictionary_items::Column::WorkspaceId.eq(workspace))
        .filter(dictionary_items::Column::DictionaryId.eq(id))
        .exec(conn)
        .await?;
    for (position, item) in items.iter().enumerate() {
        dictionary_items::Entity::insert(dictionary_items::ActiveModel {
            id: Set(s.next_id()?.into()),
            workspace_id: Set(workspace.into()),
            dictionary_id: Set(id.into()),
            value: Set(item.value.clone()),
            label: Set(item.label.clone()),
            position: Set(position as i64),
            version: Set(1),
            created_at: Set(db::now()),
            updated_at: Set(db::now()),
        })
        .exec(conn)
        .await?;
    }
    Ok(())
}
async fn dict_row(
    conn: &impl ConnectionTrait,
    workspace: Id,
    id: Id,
) -> Result<dictionaries::Model> {
    dictionaries::Entity::find_by_id(i64::from(id))
        .filter(dictionaries::Column::WorkspaceId.eq(workspace))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
async fn items(conn: &impl ConnectionTrait, workspace: Id, id: Id) -> Result<Vec<Item>> {
    Ok(dictionary_items::Entity::find()
        .filter(dictionary_items::Column::WorkspaceId.eq(workspace))
        .filter(dictionary_items::Column::DictionaryId.eq(id))
        .order_by_asc(dictionary_items::Column::Position)
        .all(conn)
        .await?
        .into_iter()
        .map(|r| Item {
            value: r.value,
            label: r.label,
            color: None,
        })
        .collect())
}
pub(crate) async fn dictionary_contains(
    conn: &impl ConnectionTrait,
    workspace: Id,
    key: &str,
    value: &str,
) -> Result<bool> {
    let row = dictionaries::Entity::find()
        .filter(dictionaries::Column::WorkspaceId.eq(workspace))
        .filter(dictionaries::Column::Key.eq(key))
        .filter(dictionaries::Column::Builtin.eq(1))
        .one(conn)
        .await?;
    let Some(row) = row else { return Ok(false) };
    if let Some(raw) = row.value_json {
        let parsed = serde_json::from_str::<Value>(&raw).unwrap_or(Value::Null);
        return Ok(values::options(&parsed).as_array().is_some_and(|options| {
            options
                .iter()
                .any(|option| option["value"].as_str() == Some(value))
        }));
    }
    Ok(items(conn, workspace, row.id.into())
        .await?
        .iter()
        .any(|item| item.value == value))
}
fn dict_json(row: dictionaries::Model) -> Value {
    let raw = row.value_json.clone().unwrap_or_else(|| "[]".into());
    json!({"id":Id::from(row.id),"key":row.key,"name":row.name,"description":row.description,
        "category":row.category,"value_type":row.value_type,"value_json":raw,
        "schema_json":row.schema_json.unwrap_or_else(|| "{}".into()),"value_summary":values::summary(&raw),
        "builtin":row.builtin==1,"version":row.version,"created_at":row.created_at,"updated_at":row.updated_at})
}
pub async fn list_dictionaries(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("dictionaries.read")?;
    let q = ListQuery::parse(&DICT_SPEC, &raw)?;
    let (rows, total) = resource::list::<dictionaries::Entity, dictionaries::Model>(
        &s.db,
        &DICT_SPEC,
        dictionaries::Entity::find(),
        &scope.workspace_id,
        &q,
    )
    .await?;
    let legacy_ids: Vec<i64> = rows
        .iter()
        .filter(|r| r.value_json.is_none())
        .map(|r| r.id)
        .collect();
    let mut old_items: HashMap<i64, Vec<Item>> = HashMap::new();
    if !legacy_ids.is_empty() {
        for item in dictionary_items::Entity::find()
            .filter(dictionary_items::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(dictionary_items::Column::DictionaryId.is_in(legacy_ids))
            .order_by_asc(dictionary_items::Column::Position)
            .all(&s.db)
            .await?
        {
            old_items.entry(item.dictionary_id).or_default().push(Item {
                value: item.value,
                label: item.label,
                color: None,
            });
        }
    }
    let records: Vec<Value> = rows
        .into_iter()
        .map(|mut r| {
            if r.value_json.is_none() {
                r.value_json = Some(
                    serde_json::to_string(&old_items.remove(&r.id).unwrap_or_default()).unwrap(),
                );
            }
            dict_json(r)
        })
        .collect();
    Ok(Json(
        json!({"items":records,"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
pub async fn get_dictionary(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("dictionaries.read")?;
    let model = dict_row(&s.db, scope.workspace_id, id).await?;
    let legacy = model.value_json.is_none();
    let mut row = dict_json(model);
    let options = if legacy {
        serde_json::to_string_pretty(&items(&s.db, scope.workspace_id, id).await?).unwrap()
    } else {
        values::options(
            &serde_json::from_str::<Value>(row["value_json"].as_str().unwrap_or("[]"))
                .unwrap_or(Value::Null),
        )
        .to_string()
    };
    row["items"] = json!(options);
    if legacy {
        row["value_json"] = json!(options);
        row["value_summary"] = json!(values::summary(&options));
    }
    Ok(Json(row))
}
// Serialize dictionary writers on the existing workspace row, also protecting key uniqueness.
async fn lock(conn: &impl ConnectionTrait, workspace: Id) -> Result<()> {
    workspaces::Entity::update_many()
        .col_expr(
            workspaces::Column::UpdatedAt,
            Expr::col(workspaces::Column::UpdatedAt),
        )
        .filter(workspaces::Column::Id.eq(workspace))
        .exec(conn)
        .await?;
    Ok(())
}
pub async fn create_dictionary(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<DictionaryBody>,
) -> Result<(StatusCode, Json<Value>)> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let result = create_dictionary_in(&s, &tx, &scope, body).await?;
    tx.commit().await?;
    Ok(result)
}
pub(crate) async fn create_dictionary_in(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    scope: &Scope,
    body: DictionaryBody,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("dictionaries.create")?;
    let values = validate(&body)?;
    let typed = dictionary_value(&body)?;
    if matches!(body.value_type.as_str(), "image" | "video" | "file") {
        crate::maintenance::files::validate_references(
            s,
            scope.workspace_id,
            "dictionaries",
            &typed.value,
            &typed.schema,
        )
        .await?;
    }
    lock(tx, scope.workspace_id).await?;
    if dictionaries::Entity::find()
        .filter(dictionaries::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(dictionaries::Column::Key.eq(&body.key))
        .one(tx)
        .await?
        .is_some()
    {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new("maintenance.dictionaryExists"),
        ));
    }
    let id = s.next_id()?;
    let now = db::now();
    dictionaries::Entity::insert(dictionaries::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(scope.workspace_id.into()),
        key: Set(body.key),
        name: Set(body.name),
        description: Set(body.description),
        category: Set(body.category),
        value_type: Set(body.value_type),
        value_json: Set(Some(typed.value.to_string())),
        schema_json: Set(Some(typed.schema.to_string())),
        builtin: Set(0),
        version: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(tx)
    .await?;
    write_items(&s, tx, scope.workspace_id, id, &values).await?;
    crate::hosts::activity_in(
        &s,
        tx,
        &scope,
        "create",
        "dictionaries",
        &id,
        json!({"items":values.len()}),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id,"version":1}))))
}
pub async fn update_dictionary(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<DictionaryBody>,
) -> Result<Json<Value>> {
    scope.require("dictionaries.update")?;
    let values = validate(&body)?;
    let typed = dictionary_value(&body)?;
    if matches!(body.value_type.as_str(), "image" | "video" | "file") {
        crate::maintenance::files::validate_references(
            &s,
            scope.workspace_id,
            "dictionaries",
            &typed.value,
            &typed.schema,
        )
        .await?;
    }
    let tx = s.db.begin().await?;
    lock(&tx, scope.workspace_id).await?;
    let old = dict_row(&tx, scope.workspace_id, id).await?;
    if old.key != body.key {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("maintenance.dictionaryKeyImmutable"),
        ));
    }
    if old.builtin == 1 && !matches!(body.value_type.as_str(), "selection" | "tab") {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("maintenance.systemDictionaryTypeRequired"),
        ));
    }
    if let Some(expected) = states::controlled_values(&old.key) {
        if values
            .iter()
            .map(|item| item.value.as_str())
            .collect::<HashSet<_>>()
            != expected.iter().copied().collect::<HashSet<_>>()
        {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("maintenance.stateControlled"),
            ));
        }
    }
    let n = dictionaries::Entity::update_many()
        .col_expr(dictionaries::Column::Name, Expr::value(body.name))
        .col_expr(dictionaries::Column::Category, Expr::value(body.category))
        .col_expr(
            dictionaries::Column::ValueType,
            Expr::value(body.value_type),
        )
        .col_expr(
            dictionaries::Column::ValueJson,
            Expr::value(typed.value.to_string()),
        )
        .col_expr(
            dictionaries::Column::SchemaJson,
            Expr::value(typed.schema.to_string()),
        )
        .col_expr(
            dictionaries::Column::Description,
            Expr::value(body.description),
        )
        .col_expr(
            dictionaries::Column::Version,
            Expr::col(dictionaries::Column::Version).add(1),
        )
        .col_expr(dictionaries::Column::UpdatedAt, Expr::value(db::now()))
        .filter(dictionaries::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(dictionaries::Column::Id.eq(id))
        .filter(dictionaries::Column::Version.eq(body.version))
        .exec(&tx)
        .await?
        .rows_affected;
    if n == 0 {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new("maintenance.dictionaryChanged"),
        ));
    }
    write_items(&s, &tx, scope.workspace_id, id, &values).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "dictionaries",
        &id,
        json!({"items":values.len()}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"version":body.version+1})))
}
pub async fn delete_dictionary(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<StatusCode> {
    scope.require("dictionaries.delete")?;
    let tx = s.db.begin().await?;
    lock(&tx, scope.workspace_id).await?;
    let old = dict_row(&tx, scope.workspace_id, id).await?;
    if old.builtin == 1 {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("maintenance.systemDictionaryDeleteForbidden"),
        ));
    }
    dictionary_items::Entity::delete_many()
        .filter(dictionary_items::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(dictionary_items::Column::DictionaryId.eq(id))
        .exec(&tx)
        .await?;
    dictionaries::Entity::delete_many()
        .filter(dictionaries::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(dictionaries::Column::Id.eq(id))
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(&s, &tx, &scope, "delete", "dictionaries", &id, json!({})).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
/// Safe presentation preferences are available to every workspace member.
pub async fn preferences(State(s): State<AppState>, scope: Scope) -> Result<Json<Value>> {
    let mut dicts = serde_json::Map::new();
    let rows = dictionaries::Entity::find()
        .filter(dictionaries::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(dictionaries::Column::Builtin.eq(1))
        .all(&s.db)
        .await?;
    let mut grouped: HashMap<i64, Vec<Item>> = HashMap::new();
    if !rows.is_empty() {
        for item in dictionary_items::Entity::find()
            .filter(dictionary_items::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(dictionary_items::Column::DictionaryId.is_in(rows.iter().map(|row| row.id)))
            .order_by_asc(dictionary_items::Column::Position)
            .all(&s.db)
            .await?
        {
            grouped.entry(item.dictionary_id).or_default().push(Item {
                value: item.value,
                label: item.label,
                color: None,
            });
        }
    }
    for row in rows {
        let options = row
            .value_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .filter(Value::is_array)
            .map(|value| values::options(&value))
            .unwrap_or_else(|| json!(grouped.remove(&row.id).unwrap_or_default()));
        dicts.insert(row.key, options);
    }
    Ok(Json(
        json!({"default_page_size":setting(&s.db,scope.workspace_id,"default_page_size").await?,"dictionaries":dicts}),
    ))
}

/// Called between executions by the owning scheduler, never during remote cleanup.
pub async fn prune_history(s: &AppState, deployments: bool) -> Result<()> {
    use crate::entities::{
        cron_runs, deployment_revisions as revisions, deployments as apps,
        pipeline_run_logs as logs, pipeline_run_steps as steps, pipeline_runs as runs,
    };
    use sea_orm::{QuerySelect, QueryTrait};
    for setting in settings::Entity::find()
        .filter(settings::Column::Key.eq("history_retention_days"))
        .filter(settings::Column::Value.gt(0))
        .all(&s.db)
        .await?
    {
        let workspace = Id::from(setting.workspace_id);
        // Shares the variable/deployment lock with apply/rollback; recheck retention under lock.
        let tx = crate::variables::transaction(s, workspace).await?;
        let days = self::setting(&tx, workspace, "history_retention_days").await?;
        if days == 0 {
            continue;
        }
        let cutoff = db::now() - days * 86_400_000;
        let mut removed = 0;
        let mut artifact_dirs = Vec::new();
        if deployments {
            let rows = revisions::Entity::find()
                .filter(revisions::Column::WorkspaceId.eq(workspace))
                .filter(
                    Expr::col(revisions::Column::Id).not_in_subquery(
                        cron_runs::Entity::find()
                            .select_only()
                            .column(cron_runs::Column::DownstreamId)
                            .filter(cron_runs::Column::WorkspaceId.eq(workspace))
                            .filter(cron_runs::Column::DownstreamKind.eq("deployment-revisions"))
                            .filter(cron_runs::Column::State.is_in([
                                "queued",
                                "running",
                                "needs-review",
                            ]))
                            .into_query(),
                    ),
                )
                .filter(
                    revisions::Column::Id.not_in_subquery(
                        crate::entities::deployment_target_states::Entity::find()
                            .select_only()
                            .column(
                                crate::entities::deployment_target_states::Column::CurrentRevision,
                            )
                            .filter(
                                crate::entities::deployment_target_states::Column::WorkspaceId
                                    .eq(workspace),
                            )
                            .into_query(),
                    ),
                )
                .filter(revisions::Column::FinishedAt.gt(0))
                .filter(revisions::Column::FinishedAt.lt(cutoff))
                .filter(revisions::Column::State.is_in(["succeeded", "failed"]))
                .filter(
                    Expr::col(revisions::Column::Id).not_in_subquery(
                        apps::Entity::find()
                            .select_only()
                            .column(apps::Column::CurrentRevision)
                            .filter(apps::Column::WorkspaceId.eq(workspace))
                            .into_query(),
                    ),
                )
                .filter(
                    Expr::col(revisions::Column::DeploymentId).in_subquery(
                        apps::Entity::find()
                            .select_only()
                            .column(apps::Column::Id)
                            .filter(apps::Column::WorkspaceId.eq(workspace))
                            .filter(apps::Column::State.is_not_in([
                                "queued",
                                "running",
                                "needs-review",
                            ]))
                            .into_query(),
                    ),
                )
                .order_by_asc(revisions::Column::FinishedAt)
                .limit(200)
                .all(&tx)
                .await?;
            for row in rows {
                crate::entities::deployment_revision_targets::Entity::delete_many()
                    .filter(
                        crate::entities::deployment_revision_targets::Column::RevisionId.eq(row.id),
                    )
                    .filter(
                        crate::entities::deployment_revision_targets::Column::WorkspaceId
                            .eq(workspace),
                    )
                    .exec(&tx)
                    .await?;
                crate::entities::deployment_artifact_refs::Entity::delete_many()
                    .filter(
                        crate::entities::deployment_artifact_refs::Column::RevisionId.eq(row.id),
                    )
                    .exec(&tx)
                    .await?;
                removed += revisions::Entity::delete_many()
                    .filter(revisions::Column::WorkspaceId.eq(workspace))
                    .filter(revisions::Column::Id.eq(row.id))
                    .exec(&tx)
                    .await?
                    .rows_affected;
            }
        } else {
            let rows = runs::Entity::find()
                .filter(runs::Column::WorkspaceId.eq(workspace))
                .filter(
                    runs::Column::Id.not_in_subquery(
                        crate::entities::deployment_artifact_refs::Entity::find()
                            .select_only()
                            .column(crate::entities::deployment_artifact_refs::Column::RunId)
                            .filter(
                                crate::entities::deployment_artifact_refs::Column::WorkspaceId
                                    .eq(workspace),
                            )
                            .into_query(),
                    ),
                )
                .filter(
                    Expr::col(runs::Column::Id).not_in_subquery(
                        cron_runs::Entity::find()
                            .select_only()
                            .column(cron_runs::Column::DownstreamId)
                            .filter(cron_runs::Column::WorkspaceId.eq(workspace))
                            .filter(cron_runs::Column::DownstreamKind.eq("pipeline-runs"))
                            .filter(cron_runs::Column::State.is_in([
                                "queued",
                                "running",
                                "needs-review",
                            ]))
                            .into_query(),
                    ),
                )
                .filter(runs::Column::FinishedAt.gt(0))
                .filter(runs::Column::FinishedAt.lt(cutoff))
                .filter(runs::Column::State.is_in(["succeeded", "failed", "cancelled"]))
                .order_by_asc(runs::Column::FinishedAt)
                .limit(200)
                .all(&tx)
                .await?;
            for row in rows {
                crate::entities::pipeline_artifacts::Entity::delete_many()
                    .filter(crate::entities::pipeline_artifacts::Column::RunId.eq(row.id))
                    .exec(&tx)
                    .await?;
                artifact_dirs.push(crate::pipelines::artifacts::directory(s, row.id));
                logs::Entity::delete_many()
                    .filter(logs::Column::RunId.eq(row.id))
                    .exec(&tx)
                    .await?;
                crate::entities::pipeline_run_jobs::Entity::delete_many()
                    .filter(crate::entities::pipeline_run_jobs::Column::RunId.eq(row.id))
                    .exec(&tx)
                    .await?;
                steps::Entity::delete_many()
                    .filter(steps::Column::RunId.eq(row.id))
                    .exec(&tx)
                    .await?;
                removed += runs::Entity::delete_many()
                    .filter(runs::Column::WorkspaceId.eq(workspace))
                    .filter(runs::Column::Id.eq(row.id))
                    .exec(&tx)
                    .await?
                    .rows_affected;
            }
        }
        if removed > 0 {
            crate::hosts::activity_record(
                s,
                &tx,
                workspace,
                (Id::default(), "系统"),
                "retention-cleanup",
                if deployments {
                    "deployment-revisions"
                } else {
                    "pipeline-runs"
                },
                Id::default(),
                json!({"removed":removed,"retention_days":days}),
            )
            .await?;
        }
        tx.commit().await?;
        for directory in artifact_dirs {
            if let Err(error) = tokio::fs::remove_dir_all(&directory).await {
                if error.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(path=%directory.display(), %error, "无法清理已删除构建历史的产物");
                }
            }
        }
    }
    Ok(())
}
