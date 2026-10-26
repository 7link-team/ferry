//! Workspace preferences and shared enumeration dictionaries.
use crate::{
    db,
    entities::{dictionaries, dictionary_items, settings, workspaces},
    error::{ApiError, Result},
    resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
    workspace::Scope,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set, TransactionTrait,
    sea_query::{Expr, ExprTrait},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

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
];
const SPEC: Spec = Spec {
    search: &["key"],
    filters: &[],
    sortable: &["key", "updated_at"],
    default_sort: "key",
};
const DICT_SPEC: Spec = Spec {
    search: &["key", "name", "description"],
    filters: &[],
    sortable: &["key", "name", "created_at", "updated_at"],
    default_sort: "key",
};

pub async fn setting(conn: &impl ConnectionTrait, workspace: Id, key: &str) -> Result<i64> {
    let fallback = SETTINGS
        .iter()
        .find(|v| v.0 == key)
        .ok_or_else(|| ApiError::bad("未知配置项。"))?
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
    for &(key, _, value, _, _) in SETTINGS {
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
                version: Set(1),
                created_at: Set(now),
                updated_at: Set(now),
            })
            .exec(conn)
            .await?;
        }
    }
    for (key, name, items) in [
        (
            "host-state",
            "主机状态",
            vec![("online", "在线"), ("offline", "离线"), ("unknown", "未知")],
        ),
        (
            "environment",
            "环境",
            vec![
                ("dev", "开发"),
                ("staging", "预发布"),
                ("production", "生产"),
            ],
        ),
        (
            "app-category",
            "应用分类",
            vec![
                ("web", "Web 服务"),
                ("worker", "后台任务"),
                ("database", "数据库"),
            ],
        ),
    ] {
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
fn setting_json(r: settings::Model) -> Value {
    let def = SETTINGS.iter().find(|v| v.0 == r.key).unwrap();
    json!({"id":Id::from(r.id),"key":r.key,"name":def.1,"type":"integer","value":r.value,"default":def.2,"min":def.3,"max":def.4,"version":r.version,"updated_at":r.updated_at})
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
    value: i64,
    version: i64,
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
    let def = SETTINGS.iter().find(|v| v.0 == row.key).unwrap();
    if !(def.3..=def.4).contains(&body.value) {
        return Err(ApiError::bad(format!(
            "{} 必须在 {} 到 {} 之间。",
            def.1, def.3, def.4
        )));
    }
    let n = settings::Entity::update_many()
        .col_expr(settings::Column::Value, Expr::value(body.value))
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
        return Err(ApiError::conflict("配置已变化，请刷新后重试。"));
    }
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "settings",
        &id,
        json!({"key":row.key,"value":body.value}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"version":body.version+1})))
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Item {
    value: String,
    label: String,
}
#[derive(Deserialize)]
pub struct DictionaryBody {
    key: String,
    name: String,
    #[serde(default)]
    description: String,
    items: String,
    #[serde(default)]
    version: i64,
}
fn validate(body: &DictionaryBody) -> Result<Vec<Item>> {
    if body.key.is_empty()
        || body.key.len() > 60
        || !body
            .key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        || body.name.trim().is_empty()
        || body.name.chars().count() > 100
    {
        return Err(ApiError::bad(
            "字典标识需为小写字母/数字/中划线，名称需为1到100字。",
        ));
    }
    let items: Vec<Item> = serde_json::from_str(&body.items)
        .map_err(|_| ApiError::bad("字典项需为含value和label的JSON数组。"))?;
    let mut values = HashSet::new();
    if items.len() > 200
        || items.iter().any(|i| {
            i.value.trim().is_empty()
                || i.value.chars().count() > 100
                || i.label.trim().is_empty()
                || i.label.chars().count() > 100
                || i.value.contains('\0')
                || i.label.contains('\0')
                || !values.insert(&i.value)
        })
    {
        return Err(ApiError::bad(
            "字典最多200项；值需唯一，值和标签均需为1到100字。",
        ));
    }
    Ok(items)
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
        })
        .collect())
}
fn dict_json(row: dictionaries::Model) -> Value {
    json!({"id":Id::from(row.id),"key":row.key,"name":row.name,"description":row.description,"builtin":row.builtin==1,"version":row.version,"created_at":row.created_at,"updated_at":row.updated_at})
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
    Ok(Json(
        json!({"items":rows.into_iter().map(dict_json).collect::<Vec<_>>(),"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
pub async fn get_dictionary(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("dictionaries.read")?;
    let mut row = dict_json(dict_row(&s.db, scope.workspace_id, id).await?);
    row["items"] =
        json!(serde_json::to_string_pretty(&items(&s.db, scope.workspace_id, id).await?).unwrap());
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
    scope.require("dictionaries.create")?;
    let values = validate(&body)?;
    let tx = s.db.begin().await?;
    lock(&tx, scope.workspace_id).await?;
    if dictionaries::Entity::find()
        .filter(dictionaries::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(dictionaries::Column::Key.eq(&body.key))
        .one(&tx)
        .await?
        .is_some()
    {
        return Err(ApiError::conflict("字典标识已存在。"));
    }
    let id = s.next_id()?;
    let now = db::now();
    dictionaries::Entity::insert(dictionaries::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(scope.workspace_id.into()),
        key: Set(body.key),
        name: Set(body.name),
        description: Set(body.description),
        builtin: Set(0),
        version: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(&tx)
    .await?;
    write_items(&s, &tx, scope.workspace_id, id, &values).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "create",
        "dictionaries",
        &id,
        json!({"items":values.len()}),
    )
    .await?;
    tx.commit().await?;
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
    let tx = s.db.begin().await?;
    lock(&tx, scope.workspace_id).await?;
    let old = dict_row(&tx, scope.workspace_id, id).await?;
    if old.key != body.key {
        return Err(ApiError::bad("字典标识创建后不可修改。"));
    }
    if old.key == "host-state"
        && values
            .iter()
            .map(|i| i.value.as_str())
            .collect::<HashSet<_>>()
            != HashSet::from(["online", "offline", "unknown"])
    {
        return Err(ApiError::bad(
            "主机状态由系统控制，只能修改显示标签和排序。",
        ));
    }
    let n = dictionaries::Entity::update_many()
        .col_expr(dictionaries::Column::Name, Expr::value(body.name))
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
        return Err(ApiError::conflict("字典已变化，请刷新后重试。"));
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
        return Err(ApiError::bad("系统字典不可删除；可修改其显示内容。"));
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
    for row in dictionaries::Entity::find()
        .filter(dictionaries::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(dictionaries::Column::Builtin.eq(1))
        .all(&s.db)
        .await?
    {
        dicts.insert(
            row.key,
            json!(items(&s.db, scope.workspace_id, row.id.into()).await?),
        );
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
                logs::Entity::delete_many()
                    .filter(logs::Column::RunId.eq(row.id))
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
    }
    Ok(())
}
