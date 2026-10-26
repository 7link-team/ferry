//! Shared scoped variables. Stored values use the same vault as credentials.
use crate::{
    db,
    entities::{deployments, pipelines, variables},
    error::{ApiError, Result},
    resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
    vault,
    workspace::Scope,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{StatusCode, header},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, FromQueryResult, QueryFilter,
    QuerySelect, Select, Set, TransactionTrait,
    sea_query::{Expr, ExprTrait},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

const SPEC: Spec = Spec {
    search: &["name", "description"],
    filters: &["scope", "kind"],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "name",
};

#[derive(FromQueryResult)]
struct Summary {
    id: i64,
    name: String,
    scope: String,
    scope_ref: i64,
    kind: String,
    description: String,
    has_value: bool,
    plain_value: String,
    created_at: i64,
    updated_at: i64,
    version: i64,
}
fn summary_query() -> Select<variables::Entity> {
    use variables::Column as C;
    variables::Entity::find()
        .select_only()
        .columns([
            C::Id,
            C::Name,
            C::Scope,
            C::ScopeRef,
            C::Kind,
            C::Description,
            C::CreatedAt,
            C::UpdatedAt,
            C::Version,
        ])
        .expr_as(C::Value.ne(""), "has_value")
        // Secret ciphertext does not cross the database projection boundary.
        .expr_as(
            Expr::case(C::Kind.eq("plain"), Expr::col(C::Value)).finally(""),
            "plain_value",
        )
}
fn to_json(s: &AppState, workspace: Id, row: &Summary) -> Result<Value> {
    let mut result = json!({"id": Id::from(row.id), "name": row.name, "scope": row.scope,
        "scope_ref": Id::from(row.scope_ref), "kind": row.kind, "description": row.description,
        "has_value": row.has_value, "created_at": row.created_at, "updated_at": row.updated_at,
        "version": row.version});
    if row.kind == "plain" {
        result["value"] = json!(s.vault.open_secret(
            &row.plain_value,
            &vault::aad(&workspace.to_string(), "variables", &row.id.to_string())
        )?);
    }
    Ok(result)
}
async fn summary(db: &impl ConnectionTrait, workspace: Id, id: Id) -> Result<Summary> {
    summary_query()
        .filter(variables::Column::WorkspaceId.eq(workspace))
        .filter(variables::Column::Id.eq(id))
        .into_model::<Summary>()
        .one(db)
        .await?
        .ok_or_else(ApiError::missing)
}

/// Serialize scope mutations and snapshot selection using the existing portable lock.
/// The database write lock is retained until commit even after releasing the row.
pub async fn transaction(s: &AppState, workspace: Id) -> Result<DatabaseTransaction> {
    let tx = s.db.begin().await?;
    let name = format!("variables:{workspace}");
    let owner = s.next_id()?.to_string();
    if !crate::lock::acquire(&tx, &name, &owner, db::now(), 60_000).await? {
        return Err(ApiError::conflict("变量配置正在变化，请稍后重试。"));
    }
    crate::lock::release(&tx, &name, &owner).await?;
    Ok(tx)
}

#[derive(Deserialize)]
pub struct Body {
    name: String,
    scope: Option<String>,
    scope_ref: Option<String>,
    kind: Option<String>,
    #[serde(default)]
    description: String,
    value: Option<String>,
    #[serde(default)]
    version: i64,
}
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 96
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}
fn check_value(value: &str) -> Result<()> {
    if value.len() > 64 * 1024 || value.contains('\0') {
        return Err(ApiError::bad("变量值不得超过64KB或包含空字符。"));
    }
    Ok(())
}
async fn check_scope(
    db: &impl ConnectionTrait,
    workspace: Id,
    layer: &str,
    reference: Id,
) -> Result<()> {
    if !["global", "deploy", "build"].contains(&layer)
        || i64::from(reference) < 0
        || (layer == "global" && i64::from(reference) != 0)
    {
        return Err(ApiError::bad("变量作用域或对象引用无效。"));
    }
    if i64::from(reference) == 0 {
        return Ok(());
    }
    let exists = match layer {
        "deploy" => deployments::Entity::find_by_id(reference)
            .filter(deployments::Column::WorkspaceId.eq(workspace))
            .one(db)
            .await?
            .is_some(),
        "build" => pipelines::Entity::find_by_id(reference)
            .filter(pipelines::Column::WorkspaceId.eq(workspace))
            .one(db)
            .await?
            .is_some(),
        _ => false,
    };
    if !exists {
        return Err(ApiError::bad("作用域对象不存在或不属于当前工作空间。"));
    }
    Ok(())
}

pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(mut raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("variables.read")?;
    let reference = raw
        .remove("scope_ref")
        .map(|v| Id::reference(&v))
        .transpose()?;
    let query = ListQuery::parse(&SPEC, &raw)?;
    let mut select = summary_query();
    if let Some(reference) = reference {
        select = select.filter(variables::Column::ScopeRef.eq(reference));
    }
    let (rows, total) = resource::list(&s.db, &SPEC, select, &scope.workspace_id, &query).await?;
    Ok(Json(
        json!({"items": rows.iter().map(|r| to_json(&s,scope.workspace_id,r)).collect::<Result<Vec<_>>>()?,
        "total":total,"page":query.page,"page_size":query.page_size}),
    ))
}
pub async fn get(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    scope.require("variables.read")?;
    let row = summary(
        &s.db,
        scope.workspace_id,
        id.parse().map_err(|_| ApiError::missing())?,
    )
    .await?;
    Ok(Json(to_json(&s, scope.workspace_id, &row)?))
}
pub async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<Body>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("variables.create")?;
    if !valid_name(&body.name) {
        return Err(ApiError::bad("变量名限96个字母、数字、下划线或连字符。"));
    }
    let layer = body.scope.as_deref().unwrap_or("global");
    let reference = Id::reference(body.scope_ref.as_deref().unwrap_or("0"))?;
    let kind = body.kind.as_deref().unwrap_or("secret");
    if !["plain", "secret"].contains(&kind) {
        return Err(ApiError::bad("变量类型必须为普通或密钥。"));
    }
    let value = body
        .value
        .as_deref()
        .ok_or_else(|| ApiError::bad("请提供变量值，允许明确设置为空字符串。"))?;
    check_value(value)?;
    let (id, now) = (s.next_id()?, db::now());
    let sealed = s.vault.seal(
        value,
        &vault::aad(
            &scope.workspace_id.to_string(),
            "variables",
            &id.to_string(),
        ),
    )?;
    let tx = transaction(&s, scope.workspace_id).await?;
    check_scope(&tx, scope.workspace_id, layer, reference).await?;
    let result = variables::Entity::insert(variables::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(scope.workspace_id.into()),
        name: Set(body.name.clone()),
        scope: Set(layer.into()),
        scope_ref: Set(reference.into()),
        kind: Set(kind.into()),
        description: Set(body.description),
        value: Set(sealed),
        created_at: Set(now),
        updated_at: Set(now),
        version: Set(1),
    })
    .exec(&tx)
    .await;
    if let Err(error) = result {
        if matches!(
            error.sql_err(),
            Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
        ) {
            return Err(ApiError::conflict("当前作用域已存在同名变量。"));
        }
        return Err(error.into());
    }
    tx.commit().await?;
    crate::hosts::activity(
        &s,
        &scope,
        "create",
        "variables",
        &id,
        json!({"name":body.name,"scope":layer,"scope_ref":reference,"kind":kind}),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id}))))
}
pub async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Json(body): Json<Body>,
) -> Result<Json<Value>> {
    scope.require("variables.update")?;
    let id = id.parse().map_err(|_| ApiError::missing())?;
    if let Some(value) = &body.value {
        check_value(value)?;
    }
    let tx = transaction(&s, scope.workspace_id).await?;
    let old = summary(&tx, scope.workspace_id, id).await?;
    if body.name != old.name
        || body.scope.as_ref().is_some_and(|v| v != &old.scope)
        || body.kind.as_ref().is_some_and(|v| v != &old.kind)
        || body
            .scope_ref
            .as_deref()
            .map(Id::reference)
            .transpose()?
            .is_some_and(|v| i64::from(v) != old.scope_ref)
    {
        return Err(ApiError::bad("变量名称、类型与作用域创建后不可修改。"));
    }
    if old.version != body.version {
        return Err(ApiError::conflict("记录已变化，请刷新后重试。"));
    }
    let mut update = variables::Entity::update_many()
        .col_expr(
            variables::Column::Description,
            Expr::value(body.description),
        )
        .col_expr(variables::Column::UpdatedAt, Expr::value(db::now()))
        .col_expr(
            variables::Column::Version,
            Expr::col(variables::Column::Version).add(1),
        )
        .filter(variables::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(variables::Column::Id.eq(id))
        .filter(variables::Column::Version.eq(body.version));
    if let Some(value) = &body.value {
        let sealed = s.vault.seal(
            value,
            &vault::aad(
                &scope.workspace_id.to_string(),
                "variables",
                &id.to_string(),
            ),
        )?;
        update = update.col_expr(variables::Column::Value, Expr::value(sealed));
    }
    if update.exec(&tx).await?.rows_affected != 1 {
        return Err(ApiError::conflict("记录已变化，请刷新后重试。"));
    }
    tx.commit().await?;
    crate::hosts::activity(
        &s,
        &scope,
        "update",
        "variables",
        &id,
        json!({"name":old.name,"value_rotated":body.value.is_some()}),
    )
    .await?;
    Ok(Json(json!({"ok":true})))
}
pub async fn reveal(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<([(header::HeaderName, &'static str); 1], Json<Value>)> {
    scope.require("variables.reveal")?;
    let id: Id = id.parse().map_err(|_| ApiError::missing())?;
    let row = variables::Entity::find_by_id(id)
        .filter(variables::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let value = s.vault.open_secret(
        &row.value,
        &vault::aad(
            &scope.workspace_id.to_string(),
            "variables",
            &id.to_string(),
        ),
    )?;
    // No successful plaintext response without a successful audit write.
    crate::hosts::activity(
        &s,
        &scope,
        "reveal",
        "variables",
        &id,
        json!({"name":row.name,"version":row.version}),
    )
    .await?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({"value":value})),
    ))
}

/// Resolve only requested names; most-specific rows win regardless of query order.
pub async fn resolve(
    db: &impl ConnectionTrait,
    workspace: Id,
    layer: &str,
    reference: Id,
    names: &std::collections::BTreeSet<String>,
) -> Result<std::collections::BTreeMap<String, variables::Model>> {
    use sea_orm::Condition;
    use variables::Column as C;
    if names.is_empty() {
        return Ok(Default::default());
    }
    let rows = variables::Entity::find()
        .filter(C::WorkspaceId.eq(workspace))
        .filter(C::Name.is_in(names.iter().cloned()))
        .filter(
            Condition::any()
                .add(
                    Condition::all()
                        .add(C::Scope.eq("global"))
                        .add(C::ScopeRef.eq(0)),
                )
                .add(
                    Condition::all()
                        .add(C::Scope.eq(layer))
                        .add(C::ScopeRef.is_in([0, i64::from(reference)])),
                ),
        )
        .all(db)
        .await?;
    let rank = |r: &variables::Model| {
        if r.scope == "global" {
            0
        } else if r.scope_ref == 0 {
            1
        } else {
            2
        }
    };
    let mut chosen = std::collections::BTreeMap::<String, variables::Model>::new();
    for row in rows {
        if chosen
            .get(&row.name)
            .is_none_or(|old| rank(&row) > rank(old))
        {
            chosen.insert(row.name.clone(), row);
        }
    }
    for name in names {
        if !chosen.contains_key(name) {
            return Err(ApiError::bad(format!("变量 {name} 未定义，请检查作用域。")));
        }
    }
    Ok(chosen)
}

pub async fn remove(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<StatusCode> {
    scope.require("variables.delete")?;
    let id: Id = id.parse().map_err(|_| ApiError::missing())?;
    let tx = transaction(&s, scope.workspace_id).await?;
    let row = summary(&tx, scope.workspace_id, id).await?;
    if row.scope != "build" {
        let apps = deployments::Entity::find()
            .filter(deployments::Column::WorkspaceId.eq(scope.workspace_id))
            .all(&tx)
            .await?;
        for app in apps {
            if row.scope == "deploy" && row.scope_ref != 0 && row.scope_ref != app.id {
                continue;
            }
            let manifest = crate::manifest::parse(&app.manifest)?;
            if manifest.references().values().any(|name| name == &row.name) {
                let names = std::collections::BTreeSet::from([row.name.clone()]);
                let chosen =
                    resolve(&tx, scope.workspace_id, "deploy", app.id.into(), &names).await?;
                if chosen[&row.name].id == row.id {
                    return Err(ApiError::conflict(
                        "应用配置仍引用此变量，请先修改引用再删除。",
                    ));
                }
            }
        }
    }
    variables::Entity::delete_many()
        .filter(variables::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(variables::Column::Id.eq(id))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    crate::hosts::activity(
        &s,
        &scope,
        "delete",
        "variables",
        &id,
        json!({"name":row.name}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Object-scoped variables must be removed explicitly before their owner disappears.
pub async fn check_object_removal(
    db: &impl ConnectionTrait,
    workspace: Id,
    layer: &str,
    reference: Id,
) -> Result<()> {
    let found = variables::Entity::find()
        .select_only()
        .column(variables::Column::Id)
        .filter(variables::Column::WorkspaceId.eq(workspace))
        .filter(variables::Column::Scope.eq(layer))
        .filter(variables::Column::ScopeRef.eq(reference))
        .into_tuple::<i64>()
        .one(db)
        .await?;
    if found.is_some() {
        return Err(ApiError::conflict(
            "对象仍有专属变量，请先删除这些变量再删除对象。",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::PaginatorTrait;
    use std::{
        collections::{BTreeSet, HashSet},
        sync::Arc,
        time::Instant,
    };
    async fn fixture() -> (tempfile::TempDir, AppState) {
        let (dir, db) = crate::lock::tests::database().await;
        let state = Arc::new(crate::state::State {
            ids: Arc::new(crate::snowflake::Generator::new(1, Instant::now()).unwrap()),
            db,
            data_dir: dir.path().into(),
            instance_id: "test".into(),
            server_mode: false,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), false).unwrap(),
        });
        (dir, state)
    }
    fn scope(workspace: i64, permissions: &[&str]) -> Scope {
        Scope {
            workspace_id: workspace.into(),
            user: crate::auth::User {
                id: 1.into(),
                email: "test@example.invalid".into(),
                name: "Test".into(),
                is_superadmin: false,
            },
            permissions: permissions
                .iter()
                .map(|v| v.to_string())
                .collect::<HashSet<_>>(),
        }
    }
    const ALL: &[&str] = &[
        "variables.read",
        "variables.create",
        "variables.update",
        "variables.delete",
        "variables.reveal",
    ];
    fn body(value: Value) -> Body {
        serde_json::from_value(value).unwrap()
    }
    fn path(id: Id) -> Path<(String, String)> {
        Path(("space".into(), id.to_string()))
    }
    async fn insert(
        s: &AppState,
        id: i64,
        workspace: i64,
        layer: &str,
        reference: i64,
        name: &str,
        value: &str,
    ) {
        variables::Entity::insert(variables::ActiveModel {
            id: Set(id),
            workspace_id: Set(workspace),
            name: Set(name.into()),
            scope: Set(layer.into()),
            scope_ref: Set(reference),
            kind: Set("secret".into()),
            description: Set(String::new()),
            value: Set(s
                .vault
                .seal(
                    value,
                    &vault::aad(&workspace.to_string(), "variables", &id.to_string()),
                )
                .unwrap()),
            version: Set(1),
            created_at: Set(1),
            updated_at: Set(1),
        })
        .exec(&s.db)
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn secret_projection_reveal_audit_and_exact_values() {
        let (_dir, s) = fixture().await;
        let value = "  $${ENV}\n'quoted' \\ tail\n";
        let (_, Json(result)) = create(
            State(s.clone()),
            scope(1, ALL),
            Json(body(json!({"name":"db-url","kind":"secret","value":value}))),
        )
        .await
        .unwrap();
        let id: Id = serde_json::from_value(result["id"].clone()).unwrap();
        let stored = variables::Entity::find_by_id(id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert!(!stored.value.contains(value));
        struct Projection;
        impl FromQueryResult for Projection {
            fn from_query_result(
                r: &sea_orm::QueryResult,
                p: &str,
            ) -> std::result::Result<Self, sea_orm::DbErr> {
                assert!(r.try_get::<String>(p, "value").is_err());
                assert_eq!(r.try_get::<String>(p, "plain_value")?, "");
                assert!(r.try_get::<bool>(p, "has_value")?);
                Ok(Self)
            }
        }
        summary_query()
            .into_model::<Projection>()
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let Json(detail) = get(State(s.clone()), scope(1, ALL), path(id))
            .await
            .unwrap();
        assert_eq!(detail["has_value"], true);
        assert!(detail.get("value").is_none());
        assert!(!detail.to_string().contains(&stored.value));
        let (headers, Json(revealed)) = reveal(State(s.clone()), scope(1, ALL), path(id))
            .await
            .unwrap();
        assert_eq!(headers[0].1, "no-store");
        assert_eq!(revealed["value"], value);
        let audit = crate::entities::activity::Entity::find()
            .filter(crate::entities::activity::Column::Action.eq("reveal"))
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(audit.resource_id, i64::from(id));
        assert!(!audit.details.contains("value"));
        // Omitted value preserves it; empty value is an explicit replacement.
        let _ = update(
            State(s.clone()),
            scope(1, ALL),
            path(id),
            Json(body(
                json!({"name":"db-url","version":1,"description":"updated"}),
            )),
        )
        .await
        .unwrap();
        assert_eq!(
            reveal(State(s.clone()), scope(1, ALL), path(id))
                .await
                .unwrap()
                .1
                .0["value"],
            value
        );
        let _ = update(
            State(s.clone()),
            scope(1, ALL),
            path(id),
            Json(body(json!({"name":"db-url","version":2,"value":""}))),
        )
        .await
        .unwrap();
        assert_eq!(
            reveal(State(s.clone()), scope(1, ALL), path(id))
                .await
                .unwrap()
                .1
                .0["value"],
            ""
        );
        assert_eq!(
            get(State(s.clone()), scope(1, ALL), path(id))
                .await
                .unwrap()
                .0["has_value"],
            true
        );
        let stale = update(
            State(s.clone()),
            scope(1, ALL),
            path(id),
            Json(body(json!({"name":"db-url","version":2,"value":"stale"}))),
        )
        .await
        .unwrap_err();
        assert_eq!(stale.0, StatusCode::CONFLICT);
    }
    #[tokio::test]
    async fn tenancy_permissions_duplicates_and_immutable_identity() {
        let (_dir, s) = fixture().await;
        insert(&s, 10, 1, "global", 0, "key", "secret").await;
        for operation in ["get", "reveal", "delete"] {
            let result = match operation {
                "get" => get(State(s.clone()), scope(2, ALL), path(10.into()))
                    .await
                    .map(|_| ()),
                "reveal" => reveal(State(s.clone()), scope(2, ALL), path(10.into()))
                    .await
                    .map(|_| ()),
                _ => remove(State(s.clone()), scope(2, ALL), path(10.into()))
                    .await
                    .map(|_| ()),
            };
            assert_eq!(result.unwrap_err().0, StatusCode::NOT_FOUND);
        }
        assert_eq!(
            get(State(s.clone()), scope(1, &[]), path(10.into()))
                .await
                .unwrap_err()
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            reveal(
                State(s.clone()),
                scope(1, &["variables.read"]),
                path(10.into())
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            remove(State(s.clone()), scope(1, &[]), path(10.into()))
                .await
                .unwrap_err()
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            create(
                State(s.clone()),
                scope(1, &[]),
                Json(body(json!({"name":"new","value":""})))
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            update(
                State(s.clone()),
                scope(1, &[]),
                path(10.into()),
                Json(body(json!({"name":"key","version":1})))
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            create(
                State(s.clone()),
                scope(1, ALL),
                Json(body(json!({"name":"key","value":""})))
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            create(
                State(s.clone()),
                scope(1, ALL),
                Json(body(
                    json!({"name":"object","scope":"deploy","scope_ref":"99","value":""})
                ))
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::BAD_REQUEST
        );
        for patch in [
            json!({"name":"renamed","version":1}),
            json!({"name":"key","kind":"plain","version":1}),
            json!({"name":"key","scope":"deploy","version":1}),
        ] {
            assert_eq!(
                update(
                    State(s.clone()),
                    scope(1, ALL),
                    path(10.into()),
                    Json(body(patch))
                )
                .await
                .unwrap_err()
                .0,
                StatusCode::BAD_REQUEST
            );
        }
        assert_eq!(
            crate::entities::activity::Entity::find()
                .filter(crate::entities::activity::Column::Action.eq("reveal"))
                .count(&s.db)
                .await
                .unwrap(),
            0
        );
        remove(State(s.clone()), scope(1, ALL), path(10.into()))
            .await
            .unwrap();
        assert!(
            variables::Entity::find_by_id(10)
                .one(&s.db)
                .await
                .unwrap()
                .is_none()
        );
    }
    #[tokio::test]
    async fn precedence_is_global_module_object_and_workspace_isolated() {
        let (_dir, s) = fixture().await;
        insert(&s, 1, 1, "global", 0, "name", "global").await;
        insert(&s, 2, 1, "deploy", 0, "name", "module").await;
        insert(&s, 3, 1, "deploy", 42, "name", "object").await;
        insert(&s, 4, 2, "deploy", 42, "name", "foreign").await;
        insert(&s, 5, 1, "build", 42, "name", "build").await;
        let names = BTreeSet::from(["name".into()]);
        assert_eq!(
            resolve(&s.db, 1.into(), "deploy", 42.into(), &names)
                .await
                .unwrap()["name"]
                .id,
            3
        );
        assert_eq!(
            resolve(&s.db, 1.into(), "deploy", 43.into(), &names)
                .await
                .unwrap()["name"]
                .id,
            2
        );
        assert_eq!(
            resolve(&s.db, 1.into(), "build", 42.into(), &names)
                .await
                .unwrap()["name"]
                .id,
            5
        );
        assert_eq!(
            resolve(&s.db, 1.into(), "build", 43.into(), &names)
                .await
                .unwrap()["name"]
                .id,
            1
        );
        assert!(
            resolve(&s.db, 3.into(), "deploy", 42.into(), &names)
                .await
                .is_err()
        );
        assert!(
            resolve(
                &s.db,
                1.into(),
                "deploy",
                42.into(),
                &BTreeSet::from(["missing".into()])
            )
            .await
            .is_err()
        );
    }
    #[tokio::test]
    async fn deletion_respects_effective_references_and_object_ownership() {
        let (_dir, s) = fixture().await;
        let manifest = json!({"apiVersion":"ferry/v1","kind":"Application","metadata":{"name":"web"},
            "services":[{"name":"web","image":"busybox:1.36","env":[{"name":"KEY","valueFrom":"variable/name"}]}]});
        deployments::Entity::insert(deployments::ActiveModel {
            id: Set(42),
            workspace_id: Set(1),
            name: Set("web".into()),
            manifest: Set(manifest.to_string()),
            preview_artifact: Set(String::new()),
            mode: Set("compose".into()),
            target_kind: Set("host".into()),
            target_ref: Set(99),
            workdir: Set("/tmp/test".into()),
            description: Set(String::new()),
            state: Set("idle".into()),
            revision_number: Set(0),
            current_revision: Set(0),
            created_at: Set(1),
            updated_at: Set(1),
            version: Set(1),
        })
        .exec(&s.db)
        .await
        .unwrap();
        insert(&s, 1, 1, "global", 0, "name", "global").await;
        insert(&s, 2, 1, "deploy", 0, "name", "module").await;
        insert(&s, 3, 1, "deploy", 42, "name", "object").await;
        // Shadowed variables are not active references.
        remove(State(s.clone()), scope(1, ALL), path(1.into()))
            .await
            .unwrap();
        remove(State(s.clone()), scope(1, ALL), path(2.into()))
            .await
            .unwrap();
        assert_eq!(
            remove(State(s.clone()), scope(1, ALL), path(3.into()))
                .await
                .unwrap_err()
                .0,
            StatusCode::CONFLICT
        );
        assert!(
            check_object_removal(&s.db, 1.into(), "deploy", 42.into())
                .await
                .is_err()
        );
        assert!(
            check_scope(&s.db, 1.into(), "deploy", 42.into())
                .await
                .is_ok()
        );
        assert!(
            check_scope(&s.db, 2.into(), "deploy", 42.into())
                .await
                .is_err()
        );
        let mut cleared = manifest;
        cleared["services"][0]["env"] = json!([]);
        deployments::Entity::update_many()
            .col_expr(
                deployments::Column::Manifest,
                Expr::value(cleared.to_string()),
            )
            .filter(deployments::Column::Id.eq(42))
            .exec(&s.db)
            .await
            .unwrap();
        remove(State(s.clone()), scope(1, ALL), path(3.into()))
            .await
            .unwrap();
        assert!(
            check_object_removal(&s.db, 1.into(), "deploy", 42.into())
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn concurrent_creates_do_not_duplicate_scope_identity() {
        let (_dir, s) = fixture().await;
        let a = create(
            State(s.clone()),
            scope(1, ALL),
            Json(body(json!({"name":"same","value":"a"}))),
        );
        let b = create(
            State(s.clone()),
            scope(1, ALL),
            Json(body(json!({"name":"same","value":"b"}))),
        );
        let (a, b) = tokio::join!(a, b);
        assert_ne!(a.is_ok(), b.is_ok());
        assert_eq!(a.err().or_else(|| b.err()).unwrap().0, StatusCode::CONFLICT);
        assert_eq!(variables::Entity::find().count(&s.db).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn ordinary_values_round_trip_without_a_reveal() {
        let (_dir, s) = fixture().await;
        let (_, Json(created)) = create(
            State(s.clone()),
            scope(1, ALL),
            Json(body(
                json!({"name":"plain","kind":"plain","value":" \n$VALUE\t "}),
            )),
        )
        .await
        .unwrap();
        let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
        let Json(row) = get(State(s.clone()), scope(1, ALL), path(id))
            .await
            .unwrap();
        assert_eq!(row["value"], " \n$VALUE\t ");
        let Json(rows) = list(
            State(s.clone()),
            scope(1, ALL),
            Query(HashMap::from([("scope_ref".into(), "0".into())])),
        )
        .await
        .unwrap();
        assert_eq!(rows["items"][0]["value"], row["value"]);
        assert!(check_value("\0").is_err());
        assert!(check_value(&"x".repeat(65537)).is_err());
        assert!(check_value("").is_ok());
    }
}
