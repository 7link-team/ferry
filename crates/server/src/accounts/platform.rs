//! Global administration. Admin extraction is separate from tenant Scope;
//! every privileged mutation rechecks the actor under the singleton lock.
use crate::error::{Json, Path, Query};
use crate::{
    accounts::auth::{self, Account},
    db,
    entities::{platform_state, sessions, users, workspace_members, workspace_roles, workspaces},
    error::{ApiError, Result},
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{
    extract::{FromRequestParts, State},
    http::{header, request::Parts},
};
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, JoinType, PaginatorTrait,
    QueryFilter, QuerySelect, Set, TransactionTrait,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, io::Write};

pub struct Admin(pub Account);
impl FromRequestParts<AppState> for Admin {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, s: &AppState) -> Result<Self> {
        let account = Account::from_request_parts(parts, s).await?;
        if !account.user.is_superadmin {
            return Err(ApiError::forbidden());
        }
        Ok(Self(account))
    }
}

pub async fn lock(conn: &impl ConnectionTrait) -> Result<platform_state::Model> {
    // A real write serializes across SQLite, MySQL and PostgreSQL. Do not
    // inspect affected rows: MySQL may report zero for a no-op update.
    platform_state::Entity::update_many()
        .col_expr(
            platform_state::Column::Initialized,
            Expr::col(platform_state::Column::Initialized).into(),
        )
        .filter(platform_state::Column::Id.eq(1))
        .exec(conn)
        .await?;
    platform_state::Entity::find_by_id(1)
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
pub(crate) async fn check_admin(conn: &impl ConnectionTrait, admin: &Admin) -> Result<()> {
    let allowed = users::Entity::find_by_id(i64::from(admin.0.user.id))
        .filter(users::Column::Active.eq(1))
        .filter(users::Column::IsSuperadmin.eq(1))
        .count(conn)
        .await?;
    if allowed == 0 {
        return Err(ApiError::forbidden());
    }
    Ok(())
}

/// Generate only while setup is pending. No token or password enters logs.
pub async fn prepare(s: &AppState) -> Result<()> {
    let row = platform_state::Entity::find_by_id(1)
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let path = s.data_dir.join("setup-token");
    if row.initialized != 0 {
        let _ = std::fs::remove_file(path);
        return Ok(());
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(mut file) => {
            file.write_all(crate::hosts::runner::secret().as_bytes())?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    tracing::info!(path = %path.display(), "platform initialization required; operator token file");
    Ok(())
}

pub async fn status(State(s): State<AppState>) -> Result<impl axum::response::IntoResponse> {
    let row = platform_state::Entity::find_by_id(1)
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({
            "required": row.initialized == 0,
            "has_users": users::Entity::find().count(&s.db).await? > 0,
            "registration_enabled": row.initialized != 0 && row.registration_enabled != 0,
            "email_verification_required": super::mail::Configuration::read(&s,row.mail_config.as_deref())?.required(),
        })),
    ))
}
#[derive(Deserialize)]
pub struct Setup {
    token: String,
    #[serde(flatten)]
    credentials: auth::Credentials,
}
pub async fn initialize(
    State(s): State<AppState>,
    Json(body): Json<Setup>,
) -> Result<impl axum::response::IntoResponse> {
    let email = auth::check(&body.credentials, false)?;
    let tx = s.db.begin().await?;
    let state = lock(&tx).await?;
    if state.initialized != 0 {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("platform.initialized"),
        ));
    }
    let token = std::fs::read_to_string(s.data_dir.join("setup-token"))?;
    // Compare fixed-length digests without data-dependent early exits.
    let expected = crate::hosts::runner::fingerprint(token.trim());
    let supplied = crate::hosts::runner::fingerprint(body.token.trim());
    if token.trim().is_empty()
        || expected
            .bytes()
            .zip(supplied.bytes())
            .fold(0u8, |n, (a, b)| n | (a ^ b))
            != 0
    {
        return Err(ApiError::forbidden());
    }
    let existing = users::Entity::find()
        .filter(users::Column::Email.eq(email.clone()))
        .one(&tx)
        .await?;
    let (id, name) = if let Some(row) = existing {
        if row.active == 0 || !auth::verify(&body.credentials.password, &row.password_hash) {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("platform.accountInvalid"),
            ));
        }
        users::Entity::update_many()
            .col_expr(users::Column::IsSuperadmin, Expr::value(1))
            .col_expr(
                users::Column::Version,
                Expr::col(users::Column::Version).add(1),
            )
            .col_expr(users::Column::UpdatedAt, Expr::value(db::now()))
            .filter(users::Column::Id.eq(row.id))
            .exec(&tx)
            .await?;
        sessions::Entity::delete_many()
            .filter(sessions::Column::UserId.eq(row.id))
            .exec(&tx)
            .await?;
        (Id::from(row.id), row.name)
    } else {
        auth::check(&body.credentials, true)?;
        let (id, now) = (s.next_id()?, db::now());
        let name = body.credentials.name.trim().to_owned();
        users::Entity::insert(users::ActiveModel {
            id: Set(i64::from(id)),
            email: Set(email.clone()),
            name: Set(name.clone()),
            password_hash: Set(auth::hash(&body.credentials.password)?),
            last_login_at: Set(None),
            last_login_ip: Set(None),
            active: Set(1),
            is_superadmin: Set(1),
            version: Set(1),
            created_at: Set(now),
            updated_at: Set(now),
        })
        .exec(&tx)
        .await?;
        (id, name)
    };
    platform_state::Entity::update_many()
        .col_expr(platform_state::Column::Initialized, Expr::value(1))
        .filter(platform_state::Column::Id.eq(1))
        .exec(&tx)
        .await?;
    let session = auth::start_session(&s, &id, &tx).await?;
    let actor = auth::User {
        id,
        email: email.clone(),
        name,
        is_superadmin: true,
    };
    audit(&s, &tx, &actor, "initialize", "platform", id, json!({})).await?;
    tx.commit().await?;
    let _ = std::fs::remove_file(s.data_dir.join("setup-token"));
    Ok((
        auth::set_cookie(&s, &session, auth::SESSION_MS / 1000),
        Json(json!({"id": id, "email": email})),
    ))
}

pub(crate) async fn audit(
    s: &AppState,
    conn: &impl ConnectionTrait,
    actor: &auth::User,
    action: &str,
    resource: &str,
    id: Id,
    details: Value,
) -> Result<()> {
    crate::hosts::activity_in(
        s,
        conn,
        &crate::accounts::workspace::Scope {
            workspace_id: Id::from(0),
            user: actor.clone(),
            permissions: Default::default(),
        },
        action,
        resource,
        &id,
        details,
    )
    .await
}
const USER_SPEC: Spec = Spec {
    search: &["name", "email"],
    filters: &["active", "is_superadmin"],
    sortable: &["name", "email", "created_at", "last_login_at"],
    default_sort: "-created_at",
};
#[derive(FromQueryResult)]
struct PublicUser {
    id: i64,
    email: String,
    name: String,
    active: i64,
    is_superadmin: i64,
    version: i64,
    created_at: i64,
    last_login_at: Option<i64>,
    last_login_ip: Option<String>,
}
impl PublicUser {
    fn json(self) -> Value {
        json!({"id": Id::from(self.id), "email": self.email, "name": self.name, "active": self.active != 0, "is_superadmin": self.is_superadmin != 0, "version": self.version, "created_at": self.created_at, "last_login_at":self.last_login_at,"last_login_ip":self.last_login_ip})
    }
}
fn public_users() -> sea_orm::Select<users::Entity> {
    users::Entity::find().select_only().columns([
        users::Column::Id,
        users::Column::Email,
        users::Column::Name,
        users::Column::Active,
        users::Column::IsSuperadmin,
        users::Column::Version,
        users::Column::CreatedAt,
        users::Column::LastLoginAt,
        users::Column::LastLoginIp,
    ])
}
pub async fn list_users(
    State(s): State<AppState>,
    admin: Admin,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let mut q = ListQuery::parse(&USER_SPEC, &raw)?;
    let mut select = public_users();
    // These flags are stored as integers on all supported databases. Bind
    // integers rather than strings (PostgreSQL does not coerce text to bigint).
    for (name, values) in std::mem::take(&mut q.filters) {
        let column = match name.as_str() {
            "active" => users::Column::Active,
            "is_superadmin" => users::Column::IsSuperadmin,
            _ => unreachable!("validated by USER_SPEC"),
        };
        let flags = values
            .iter()
            .map(|value| match value.as_str() {
                "0" => Ok(0_i64),
                "1" => Ok(1_i64),
                _ => Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("request.query"),
                )),
            })
            .collect::<Result<Vec<_>>>()?;
        select = select.filter(column.is_in(flags));
    }
    let (rows, total) =
        resource::global_list::<users::Entity, PublicUser>(&s.db, &admin, &USER_SPEC, select, &q)
            .await?;
    Ok(Json(
        json!({"items": rows.into_iter().map(PublicUser::json).collect::<Vec<_>>(), "total": total, "page": q.page, "page_size": q.page_size}),
    ))
}
pub async fn get_user(
    State(s): State<AppState>,
    _: Admin,
    Path(id): Path<Id>,
) -> Result<Json<Value>> {
    let row = public_users()
        .filter(users::Column::Id.eq(i64::from(id)))
        .into_model::<PublicUser>()
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok(Json(row.json()))
}
#[derive(Deserialize)]
pub struct UserUpdate {
    version: i64,
    active: bool,
    is_superadmin: bool,
}
pub async fn update_user(
    State(s): State<AppState>,
    admin: Admin,
    Path(id): Path<Id>,
    Json(body): Json<UserUpdate>,
) -> Result<Json<Value>> {
    let tx = s.db.begin().await?;
    lock(&tx).await?;
    check_admin(&tx, &admin).await?;
    let old = public_users()
        .filter(users::Column::Id.eq(i64::from(id)))
        .into_model::<PublicUser>()
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    if old.version != body.version {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("contexts.changed"),
        ));
    }
    if old.active != 0
        && old.is_superadmin != 0
        && !(body.active && body.is_superadmin)
        && users::Entity::find()
            .filter(users::Column::Active.eq(1))
            .filter(users::Column::IsSuperadmin.eq(1))
            .count(&tx)
            .await?
            <= 1
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("platform.adminRequired"),
        ));
    }
    users::Entity::update_many()
        .col_expr(users::Column::Active, Expr::value(i64::from(body.active)))
        .col_expr(
            users::Column::IsSuperadmin,
            Expr::value(i64::from(body.is_superadmin)),
        )
        .col_expr(users::Column::Version, Expr::value(old.version + 1))
        .col_expr(users::Column::UpdatedAt, Expr::value(db::now()))
        .filter(users::Column::Id.eq(i64::from(id)))
        .exec(&tx)
        .await?;
    if old.active != i64::from(body.active) || old.is_superadmin != i64::from(body.is_superadmin) {
        sessions::Entity::delete_many()
            .filter(sessions::Column::UserId.eq(i64::from(id)))
            .exec(&tx)
            .await?;
    }
    audit(
        &s,
        &tx,
        &admin.0.user,
        "update",
        "platform-users",
        id,
        json!({"active": body.active, "is_superadmin": body.is_superadmin}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id": id, "version": old.version + 1})))
}
const WORKSPACE_SPEC: Spec = Spec {
    search: &["name", "slug"],
    filters: &[],
    sortable: &["name", "slug", "created_at"],
    default_sort: "-created_at",
};
#[derive(FromQueryResult)]
struct PublicWorkspace {
    id: i64,
    name: String,
    slug: String,
    owner_id: i64,
    updated_at: i64,
    created_at: i64,
}
impl PublicWorkspace {
    fn json(self) -> Value {
        json!({"id": Id::from(self.id), "name": self.name, "slug": self.slug, "version": self.updated_at, "created_at": self.created_at})
    }
}
fn public_workspaces() -> sea_orm::Select<workspaces::Entity> {
    workspaces::Entity::find().select_only().columns([
        workspaces::Column::Id,
        workspaces::Column::Name,
        workspaces::Column::Slug,
        workspaces::Column::OwnerId,
        workspaces::Column::UpdatedAt,
        workspaces::Column::CreatedAt,
    ])
}
/// The administrator's own place in each workspace: "owner" (created it; the
/// owner never changes), "member" with the role name, or "none" when they only
/// reach it as a platform administrator.
async fn relations(
    db: &impl ConnectionTrait,
    user: Id,
    rows: Vec<PublicWorkspace>,
) -> Result<Vec<Value>> {
    #[derive(FromQueryResult)]
    struct Membership {
        workspace_id: i64,
        role_name: String,
    }
    let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
    let roles: HashMap<i64, String> = workspace_members::Entity::find()
        .select_only()
        .column(workspace_members::Column::WorkspaceId)
        .column_as(workspace_roles::Column::Name, "role_name")
        .join(
            JoinType::InnerJoin,
            workspace_members::Entity::belongs_to(workspace_roles::Entity)
                .from(workspace_members::Column::RoleId)
                .to(workspace_roles::Column::Id)
                .into(),
        )
        .filter(workspace_members::Column::UserId.eq(i64::from(user)))
        .filter(workspace_members::Column::State.eq("active"))
        .filter(workspace_members::Column::WorkspaceId.is_in(ids))
        .into_model::<Membership>()
        .all(db)
        .await?
        .into_iter()
        .map(|row| (row.workspace_id, row.role_name))
        .collect();
    Ok(rows
        .into_iter()
        .map(|row| {
            let relation = if row.owner_id == i64::from(user) {
                "owner"
            } else if roles.contains_key(&row.id) {
                "member"
            } else {
                "none"
            };
            let role = roles.get(&row.id).cloned().unwrap_or_default();
            let mut value = row.json();
            value["relation"] = json!(relation);
            value["my_role"] = json!(role);
            value
        })
        .collect())
}
pub async fn list_workspaces(
    State(s): State<AppState>,
    admin: Admin,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let q = ListQuery::parse(&WORKSPACE_SPEC, &raw)?;
    let (rows, total) = resource::global_list::<workspaces::Entity, PublicWorkspace>(
        &s.db,
        &admin,
        &WORKSPACE_SPEC,
        public_workspaces(),
        &q,
    )
    .await?;
    let items = relations(&s.db, admin.0.user.id, rows).await?;
    Ok(Json(
        json!({"items": items, "total": total, "page": q.page, "page_size": q.page_size}),
    ))
}
pub async fn get_workspace(
    State(s): State<AppState>,
    admin: Admin,
    Path(id): Path<Id>,
) -> Result<Json<Value>> {
    let row = public_workspaces()
        .filter(workspaces::Column::Id.eq(i64::from(id)))
        .into_model::<PublicWorkspace>()
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let owner = users::Entity::find_by_id(row.owner_id).one(&s.db).await?;
    let mut value = relations(&s.db, admin.0.user.id, vec![row])
        .await?
        .remove(0);
    value["owner_name"] = json!(owner.as_ref().map(|user| &user.name));
    value["owner_id"] = json!(owner.as_ref().map(|user| Id::from(user.id)));
    let mut counts = serde_json::Map::new();
    macro_rules! count {
        ($entity:ident) => {
            counts.insert(
                stringify!($entity).into(),
                json!(
                    crate::entities::$entity::Entity::find()
                        .filter(crate::entities::$entity::Column::WorkspaceId.eq(i64::from(id)))
                        .count(&s.db)
                        .await?
                ),
            );
        };
    }
    counts.insert(
        "members".into(),
        json!(
            workspace_members::Entity::find()
                .filter(workspace_members::Column::WorkspaceId.eq(i64::from(id)))
                .filter(workspace_members::Column::State.eq("active"))
                .count(&s.db)
                .await?
        ),
    );
    count!(hosts);
    count!(repository_connections);
    count!(credentials);
    count!(pipelines);
    count!(workflows);
    let assets = crate::platform_monitor::query(
        &s.db,
        "SELECT COUNT(*) AS count FROM assets WHERE workspace_id = $1",
        vec![i64::from(id).into()],
    )
    .await?;
    counts.insert(
        "assets".into(),
        assets
            .first()
            .map(|row| row["count"].clone())
            .unwrap_or(json!(0)),
    );
    value["resource_counts"] = Value::Object(counts);
    Ok(Json(value))
}
pub async fn create_workspace(
    State(s): State<AppState>,
    admin: Admin,
    Json(body): Json<crate::accounts::workspace::NewWorkspace>,
) -> Result<Json<Value>> {
    let tx = s.db.begin().await?;
    lock(&tx).await?;
    check_admin(&tx, &admin).await?;
    let value = crate::accounts::workspace::create_in(&s, admin.0.user.id, body, &tx).await?;
    let id: Id = serde_json::from_value(value["id"].clone()).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("platform.workspaceInvalid"),
        )
    })?;
    audit(
        &s,
        &tx,
        &admin.0.user,
        "create",
        "platform-workspaces",
        id,
        json!({"name": value["name"], "slug": value["slug"]}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(value))
}
#[derive(Deserialize)]
pub struct WorkspaceUpdate {
    name: String,
    version: i64,
}
pub async fn update_workspace(
    State(s): State<AppState>,
    admin: Admin,
    Path(id): Path<Id>,
    Json(body): Json<WorkspaceUpdate>,
) -> Result<Json<Value>> {
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("workspace.error.name_invalid"),
        ));
    }
    let tx = s.db.begin().await?;
    lock(&tx).await?;
    check_admin(&tx, &admin).await?;
    let version = std::cmp::max(db::now(), body.version.saturating_add(1));
    let changed = workspaces::Entity::update_many()
        .col_expr(workspaces::Column::Name, Expr::value(name))
        .col_expr(workspaces::Column::UpdatedAt, Expr::value(version))
        .filter(workspaces::Column::Id.eq(i64::from(id)))
        .filter(workspaces::Column::UpdatedAt.eq(body.version))
        .exec(&tx)
        .await?;
    if changed.rows_affected != 1 {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("platform.recordChangedOrDeleted"),
        ));
    }
    audit(
        &s,
        &tx,
        &admin.0.user,
        "update",
        "platform-workspaces",
        id,
        json!({"name": name}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id": id, "version": version})))
}
pub async fn get_settings(State(s): State<AppState>, _: Admin) -> Result<Json<Value>> {
    let row = platform_state::Entity::find_by_id(1)
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let mut view =
        crate::repositories::auth::OAuthSettings::read(&s, row.repository_oauth.as_deref())?
            .view(&s, row.repository_oauth.is_some());
    view["id"] = json!("1");
    view["registration_enabled"] = json!(row.registration_enabled != 0);
    view["runner_auto_update"] = json!(row.runner_auto_update != 0);
    view["footer_html"] = json!(row.footer_html.as_deref().unwrap_or_default());
    view.as_object_mut().unwrap().extend(
        crate::platform_monitor::retention::Limits::read(row.monitor_config.as_deref())?
            .view()
            .as_object()
            .unwrap()
            .clone(),
    );
    view.as_object_mut().unwrap().extend(
        super::mail::Configuration::read(&s, row.mail_config.as_deref())?
            .view()
            .as_object()
            .unwrap()
            .clone(),
    );
    view.as_object_mut().unwrap().extend(
        json!(super::platform_scripts::Configuration::read(
            row.page_scripts.as_deref()
        )?)
        .as_object()
        .unwrap()
        .clone(),
    );
    view["version"] = json!(row.version);
    view["system_version"] = json!(env!("CARGO_PKG_VERSION"));
    view["system_protocol"] = json!(ferry_protocol::VERSION.to_string());
    view["system_instance"] = json!(s.instance_id);
    view["system_runtime"] = json!(format!(
        "{} / {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    view["system_mode"] = json!(if s.server_mode {
        "Server"
    } else {
        "Desktop / standalone"
    });
    let storage = crate::artifact_store::Configuration::read(&s, row.file_storage.as_deref())
        .map_err(|_| crate::content_assets::unavailable())?;
    view.as_object_mut()
        .unwrap()
        .extend(storage.view().as_object().unwrap().clone());
    Ok(Json(view))
}
pub async fn server_nodes(
    State(s): State<AppState>,
    _: Admin,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let now = db::now();
    let rows = crate::entities::worker_ids::Entity::find()
        .all(&s.db)
        .await?;
    let items=rows.into_iter().map(|r| {
        let info=r.info.as_deref().and_then(|v|serde_json::from_str::<Value>(v).ok()).unwrap_or(json!({}));
        json!({"worker_id":r.worker_id,"instance_id":r.instance_id,"heartbeat_at":r.heartbeat_at,"online":r.heartbeat_at>now-crate::snowflake::TTL.as_millis() as i64,"current":r.instance_id==s.instance_id,"info":info})
    }).collect::<Vec<_>>();
    let total = items.len();
    let online = items.iter().filter(|v| v["online"] == true).count();
    let size = raw
        .get("page_size")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(20)
        .clamp(1, 200);
    let page = raw
        .get("page")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, 1024);
    Ok(Json(
        json!({"items":items.into_iter().skip((page-1)*size).take(size).collect::<Vec<_>>(),"total":total,"page":page,"page_size":size,"coverage":{"online":online,"cluster":s.cluster.is_some()}}),
    ))
}
#[derive(Deserialize)]
pub struct SettingsUpdate {
    version: i64,
    registration_enabled: bool,
    #[serde(default)]
    runner_auto_update: Option<bool>,
    #[serde(default)]
    footer_html: Option<String>,
    #[serde(flatten)]
    oauth: serde_json::Map<String, Value>,
}
pub async fn update_settings(
    State(s): State<AppState>,
    admin: Admin,
    Json(body): Json<SettingsUpdate>,
) -> Result<Json<Value>> {
    let tx = s.db.begin().await?;
    let row = lock(&tx).await?;
    check_admin(&tx, &admin).await?;
    if row.version != body.version {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("platform.settingsChanged"),
        ));
    }
    if body
        .footer_html
        .as_ref()
        .is_some_and(|content| content.len() > 65536 || content.contains('\0'))
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("platform.footerInvalid"),
        ));
    }
    let footer_html = body
        .footer_html
        .clone()
        .or(row.footer_html.clone())
        .filter(|content| !content.trim().is_empty());
    let page_scripts =
        super::platform_scripts::Configuration::update(row.page_scripts.as_deref(), &body.oauth)?;
    let mail_config =
        super::mail::Configuration::update(&s, row.mail_config.as_deref(), &body.oauth)?;
    let monitor_limits = crate::platform_monitor::retention::Limits::update(
        row.monitor_config.as_deref(),
        &body.oauth,
    )?;
    let monitor_config = serde_json::to_string(&monitor_limits).map_err(std::io::Error::other)?;
    let runner_auto_update = body
        .runner_auto_update
        .map(i64::from)
        .unwrap_or(row.runner_auto_update);
    let mut repository_oauth = row.repository_oauth.clone();
    let file_storage = crate::artifact_store::Configuration::update(
        &s,
        &tx,
        row.file_storage.as_deref(),
        &body.oauth,
    )
    .await?;
    let oauth_changed = body.oauth.keys().any(|key| {
        key == "oauth_public_url"
            || ["github", "gitlab", "gitee", "gitea"]
                .iter()
                .any(|provider| key.starts_with(&format!("oauth_{provider}_")))
    });
    if oauth_changed {
        let mut config =
            crate::repositories::auth::OAuthSettings::read(&s, repository_oauth.as_deref())?;
        config.update(&s, &body.oauth)?;
        repository_oauth = Some(config.seal(&s)?);
    }
    platform_state::Entity::update_many()
        .col_expr(platform_state::Column::FooterHtml, Expr::value(footer_html))
        .col_expr(
            platform_state::Column::PageScripts,
            Expr::value(page_scripts),
        )
        .col_expr(platform_state::Column::MailConfig, Expr::value(mail_config))
        .col_expr(
            platform_state::Column::MonitorConfig,
            Expr::value(monitor_config),
        )
        .col_expr(
            platform_state::Column::FileStorage,
            Expr::value(file_storage),
        )
        .col_expr(
            platform_state::Column::RunnerAutoUpdate,
            Expr::value(runner_auto_update),
        )
        .col_expr(
            platform_state::Column::RepositoryOauth,
            Expr::value(repository_oauth),
        )
        .col_expr(
            platform_state::Column::RegistrationEnabled,
            Expr::value(i64::from(body.registration_enabled)),
        )
        .col_expr(
            platform_state::Column::Version,
            Expr::value(row.version + 1),
        )
        .filter(platform_state::Column::Id.eq(1))
        .exec(&tx)
        .await?;
    audit(
        &s,
        &tx,
        &admin.0.user,
        "update",
        "platform-settings",
        Id::from(1),
        json!({"registration_enabled": body.registration_enabled, "runner_auto_update":runner_auto_update != 0, "repository_oauth_updated": oauth_changed, "footer_updated":body.footer_html.is_some(), "scripts_updated":body.oauth.keys().any(|key|key.starts_with("analytics_"))}),
    )
    .await?;
    tx.commit().await?;
    if let Ok(mut limits) = s.monitor.limits.lock() {
        *limits = monitor_limits;
    }
    Ok(Json(json!({"id": "1", "version": row.version + 1})))
}

/// Deployment identity is public; never expose the rest of platform settings.
pub async fn footer(State(s): State<AppState>) -> Result<impl axum::response::IntoResponse> {
    let row = platform_state::Entity::find_by_id(1)
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({"content":row.footer_html.unwrap_or_default()})),
    ))
}
