//! Workspaces, roles, and the `Scope` extractor.
//!
//! `Scope` is the tenancy invariant of the whole system: a workspace handler
//! can only obtain a `workspace_id` by going through it, and it only yields
//! one after proving the caller is an active member. No handler reads a
//! workspace id from a request body, and none is expected to remember to
//! filter — `resource::list` always filters by the scope's id.

use crate::snowflake::Id;

use crate::error::{Json, Path};
use crate::{
    accounts::auth::Account,
    db,
    entities::{workspace_members, workspace_roles, workspaces},
    error::{ApiError, Result},
    state::AppState,
};
use axum::{
    extract::{FromRequestParts, State},
    http::request::Parts,
};
use sea_orm::{ColumnTrait, EntityTrait, FromQueryResult, JoinType, QueryFilter, QuerySelect, Set};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;

/// Every permission in the product, with translation keys for the role editor.
/// `{resource}.{action}` where the resource is the REST resource name.
pub const PERMISSIONS: &[(&str, &str, &str)] = &[
    (
        "dashboard.read",
        "permission.group.dashboard",
        "permission.action.read",
    ),
    (
        "host-groups.read",
        "permission.group.host-groups",
        "permission.action.read",
    ),
    (
        "host-groups.create",
        "permission.group.host-groups",
        "permission.action.create",
    ),
    (
        "host-groups.update",
        "permission.group.host-groups",
        "permission.action.update",
    ),
    (
        "host-groups.delete",
        "permission.group.host-groups",
        "permission.action.delete",
    ),
    (
        "hosts.read",
        "permission.group.hosts",
        "permission.action.read",
    ),
    (
        "hosts.create",
        "permission.group.hosts",
        "permission.action.create",
    ),
    (
        "hosts.update",
        "permission.group.hosts",
        "permission.action.update",
    ),
    (
        "hosts.delete",
        "permission.group.hosts",
        "permission.action.delete",
    ),
    (
        "hosts.enroll",
        "permission.group.hosts",
        "permission.action.enroll",
    ),
    (
        "hosts.terminal",
        "permission.group.hosts",
        "permission.action.terminal",
    ),
    (
        "hosts.files",
        "permission.group.hosts",
        "permission.action.files",
    ),
    (
        "tags.read",
        "permission.group.tags",
        "permission.action.read",
    ),
    (
        "tags.create",
        "permission.group.tags",
        "permission.action.create",
    ),
    (
        "tags.update",
        "permission.group.tags",
        "permission.action.update",
    ),
    (
        "tags.delete",
        "permission.group.tags",
        "permission.action.delete",
    ),
    (
        "repositories.read",
        "permission.group.repositories",
        "permission.action.read",
    ),
    (
        "repositories.create",
        "permission.group.repositories",
        "permission.action.create",
    ),
    (
        "repositories.update",
        "permission.group.repositories",
        "permission.action.update",
    ),
    (
        "repositories.delete",
        "permission.group.repositories",
        "permission.action.delete",
    ),
    (
        "code-insights.read",
        "permission.group.code-insights",
        "permission.action.read",
    ),
    (
        "code-insights.create",
        "permission.group.code-insights",
        "permission.action.create",
    ),
    (
        "code-insights.update",
        "permission.group.code-insights",
        "permission.action.update",
    ),
    (
        "code-insights.delete",
        "permission.group.code-insights",
        "permission.action.delete",
    ),
    (
        "code-insights.sync",
        "permission.group.code-insights",
        "permission.action.sync",
    ),
    (
        "pipeline-groups.read",
        "permission.group.pipeline-groups",
        "permission.action.read",
    ),
    (
        "pipeline-groups.create",
        "permission.group.pipeline-groups",
        "permission.action.create",
    ),
    (
        "pipeline-groups.update",
        "permission.group.pipeline-groups",
        "permission.action.update",
    ),
    (
        "pipeline-groups.delete",
        "permission.group.pipeline-groups",
        "permission.action.delete",
    ),
    (
        "pipeline-environments.read",
        "permission.group.pipeline-environments",
        "permission.action.read",
    ),
    (
        "pipeline-environments.create",
        "permission.group.pipeline-environments",
        "permission.action.create",
    ),
    (
        "pipeline-environments.update",
        "permission.group.pipeline-environments",
        "permission.action.update",
    ),
    (
        "pipeline-environments.delete",
        "permission.group.pipeline-environments",
        "permission.action.delete",
    ),
    (
        "pipelines.read",
        "permission.group.pipelines",
        "permission.action.read",
    ),
    (
        "pipelines.create",
        "permission.group.pipelines",
        "permission.action.create",
    ),
    (
        "pipelines.update",
        "permission.group.pipelines",
        "permission.action.update",
    ),
    (
        "pipelines.delete",
        "permission.group.pipelines",
        "permission.action.delete",
    ),
    (
        "pipelines.run",
        "permission.group.pipelines",
        "permission.action.run",
    ),
    (
        "pipeline-templates.read",
        "permission.group.pipeline-templates",
        "permission.action.read",
    ),
    (
        "pipeline-templates.create",
        "permission.group.pipeline-templates",
        "permission.action.create",
    ),
    (
        "pipeline-templates.update",
        "permission.group.pipeline-templates",
        "permission.action.update",
    ),
    (
        "pipeline-templates.delete",
        "permission.group.pipeline-templates",
        "permission.action.delete",
    ),
    (
        "credentials.read",
        "permission.group.credentials",
        "permission.action.read",
    ),
    (
        "credentials.create",
        "permission.group.credentials",
        "permission.action.create",
    ),
    (
        "credentials.update",
        "permission.group.credentials",
        "permission.action.update",
    ),
    (
        "credentials.delete",
        "permission.group.credentials",
        "permission.action.delete",
    ),
    (
        "deployments.read",
        "permission.group.deployments",
        "permission.action.read",
    ),
    (
        "deployments.create",
        "permission.group.deployments",
        "permission.action.create",
    ),
    (
        "deployments.update",
        "permission.group.deployments",
        "permission.action.update",
    ),
    (
        "deployments.delete",
        "permission.group.deployments",
        "permission.action.delete",
    ),
    (
        "deployments.apply",
        "permission.group.deployments",
        "permission.action.apply",
    ),
    (
        "deployments.rollback",
        "permission.group.deployments",
        "permission.action.rollback",
    ),
    (
        "config-templates.read",
        "permission.group.config-templates",
        "permission.action.read",
    ),
    (
        "config-templates.create",
        "permission.group.config-templates",
        "permission.action.create",
    ),
    (
        "config-templates.update",
        "permission.group.config-templates",
        "permission.action.update",
    ),
    (
        "config-templates.delete",
        "permission.group.config-templates",
        "permission.action.delete",
    ),
    (
        "app-templates.read",
        "permission.group.app-templates",
        "permission.action.read",
    ),
    (
        "app-templates.create",
        "permission.group.app-templates",
        "permission.action.create",
    ),
    (
        "app-templates.update",
        "permission.group.app-templates",
        "permission.action.update",
    ),
    (
        "app-templates.delete",
        "permission.group.app-templates",
        "permission.action.delete",
    ),
    (
        "marketplace.read",
        "permission.group.marketplace",
        "permission.action.read",
    ),
    (
        "marketplace.install",
        "permission.group.marketplace",
        "permission.action.install",
    ),
    (
        "marketplace-sources.read",
        "permission.group.marketplace-sources",
        "permission.action.read",
    ),
    (
        "marketplace-sources.create",
        "permission.group.marketplace-sources",
        "permission.action.create",
    ),
    (
        "marketplace-sources.update",
        "permission.group.marketplace-sources",
        "permission.action.update",
    ),
    (
        "marketplace-sources.delete",
        "permission.group.marketplace-sources",
        "permission.action.delete",
    ),
    (
        "marketplace-sources.sync",
        "permission.group.marketplace-sources",
        "permission.action.sync",
    ),
    (
        "alerts.read",
        "permission.group.alerts",
        "permission.action.read",
    ),
    (
        "alerts.create",
        "permission.group.alerts",
        "permission.action.create",
    ),
    (
        "alerts.update",
        "permission.group.alerts",
        "permission.action.update",
    ),
    (
        "alerts.delete",
        "permission.group.alerts",
        "permission.action.delete",
    ),
    (
        "cron-jobs.read",
        "permission.group.cron-jobs",
        "permission.action.read",
    ),
    (
        "cron-jobs.create",
        "permission.group.cron-jobs",
        "permission.action.create",
    ),
    (
        "cron-jobs.update",
        "permission.group.cron-jobs",
        "permission.action.update",
    ),
    (
        "cron-jobs.delete",
        "permission.group.cron-jobs",
        "permission.action.delete",
    ),
    (
        "cron-jobs.run",
        "permission.group.cron-jobs",
        "permission.action.run_now",
    ),
    (
        "variables.read",
        "permission.group.variables",
        "permission.action.read",
    ),
    (
        "variables.create",
        "permission.group.variables",
        "permission.action.create",
    ),
    (
        "variables.update",
        "permission.group.variables",
        "permission.action.update",
    ),
    (
        "variables.delete",
        "permission.group.variables",
        "permission.action.delete",
    ),
    (
        "variables.reveal",
        "permission.group.variables",
        "permission.action.reveal",
    ),
    (
        "members.read",
        "permission.group.members",
        "permission.action.read",
    ),
    (
        "members.create",
        "permission.group.members",
        "permission.action.invite",
    ),
    (
        "members.update",
        "permission.group.members",
        "permission.action.change_role",
    ),
    (
        "members.delete",
        "permission.group.members",
        "permission.action.remove",
    ),
    (
        "roles.read",
        "permission.group.roles",
        "permission.action.read",
    ),
    (
        "roles.create",
        "permission.group.roles",
        "permission.action.create",
    ),
    (
        "roles.update",
        "permission.group.roles",
        "permission.action.update",
    ),
    (
        "roles.delete",
        "permission.group.roles",
        "permission.action.delete",
    ),
    (
        "settings.create",
        "permission.group.settings",
        "permission.action.create",
    ),
    (
        "settings.delete",
        "permission.group.settings",
        "permission.action.delete",
    ),
    (
        "settings.read",
        "permission.group.settings",
        "permission.action.read",
    ),
    (
        "settings.update",
        "permission.group.settings",
        "permission.action.update",
    ),
    (
        "dictionaries.read",
        "permission.group.dictionaries",
        "permission.action.read",
    ),
    (
        "dictionaries.create",
        "permission.group.dictionaries",
        "permission.action.create",
    ),
    (
        "dictionaries.update",
        "permission.group.dictionaries",
        "permission.action.update",
    ),
    (
        "dictionaries.delete",
        "permission.group.dictionaries",
        "permission.action.delete",
    ),
    (
        "activity.read",
        "permission.group.activity",
        "permission.action.read",
    ),
    (
        "workspace.manage",
        "permission.group.workspace",
        "permission.action.manage",
    ),
    (
        "insight-reports.read",
        "permission.group.insight-reports",
        "permission.action.read",
    ),
    (
        "insight-reports.create",
        "permission.group.insight-reports",
        "permission.action.create",
    ),
    (
        "insight-reports.update",
        "permission.group.insight-reports",
        "permission.action.update",
    ),
    (
        "insight-reports.delete",
        "permission.group.insight-reports",
        "permission.action.delete",
    ),
    (
        "insight-reports.generate",
        "permission.group.insight-reports",
        "permission.action.generate",
    ),
    (
        "code-reviews.read",
        "permission.group.code-reviews",
        "permission.action.read",
    ),
    (
        "code-reviews.create",
        "permission.group.code-reviews",
        "permission.action.create",
    ),
    (
        "code-reviews.update",
        "permission.group.code-reviews",
        "permission.action.update",
    ),
    (
        "code-reviews.delete",
        "permission.group.code-reviews",
        "permission.action.delete",
    ),
    (
        "code-reviews.run",
        "permission.group.code-reviews",
        "permission.action.run",
    ),
];

/// Permissions a chosen permission cannot work without. The role editor adds
/// these automatically, so a role cannot be saved in a state that looks
/// granted but fails at runtime.
pub fn dependencies(permission: &str) -> Vec<String> {
    let mut result = Vec::new();
    if let Some((module, action)) = permission.split_once('.') {
        let read = format!("{module}.read");
        if action != "read" && PERMISSIONS.iter().any(|p| p.0 == read) {
            result.push(read);
        }
    }
    let extra: &[&str] = match permission {
        "insight-reports.read" => &[
            "code-insights.read",
            "repositories.read",
            "credentials.read",
        ],
        "insight-reports.create" | "insight-reports.update" | "insight-reports.generate" => &[
            "code-insights.read",
            "repositories.read",
            "deployments.read",
        ],
        "code-reviews.read" => &["repositories.read", "pipelines.read", "credentials.read"],
        "code-reviews.create" | "code-reviews.update" => &[
            "repositories.read",
            "pipelines.create",
            "pipelines.update",
            "pipelines.run",
            "credentials.read",
            "hosts.read",
        ],
        "code-reviews.run" => &[
            "repositories.read",
            "pipelines.run",
            "credentials.read",
            "hosts.terminal",
        ],
        "code-insights.read" => &["repositories.read"],
        "code-insights.sync" => &[
            "code-insights.read",
            "repositories.read",
            "credentials.read",
        ],
        "hosts.terminal" | "hosts.files" | "hosts.enroll" => &["hosts.read"],
        "pipelines.run" => &["repositories.read"],
        "deployments.apply" | "deployments.rollback" => &["hosts.read", "app-templates.read"],
        "marketplace.install" => &[
            "app-templates.read",
            "app-templates.create",
            "deployments.create",
        ],
        "marketplace-sources.create"
        | "marketplace-sources.update"
        | "marketplace-sources.sync" => &["marketplace-sources.read", "repositories.read"],
        "variables.reveal" => &["variables.read"],
        "cron-jobs.run" => &["hosts.read"],
        "cron-jobs.create" | "cron-jobs.update" => &["cron-jobs.run"],
        _ => &[],
    };
    result.extend(extra.iter().map(|v| v.to_string()));
    result
}

/// Expands a permission set with everything its members depend on.
pub fn expand(selected: &[String]) -> Vec<String> {
    let mut set: HashSet<String> = selected.iter().cloned().collect();
    let mut queue: Vec<String> = selected.to_vec();
    while let Some(permission) = queue.pop() {
        for dependency in dependencies(&permission) {
            if set.insert(dependency.clone()) {
                queue.push(dependency);
            }
        }
    }
    let mut result: Vec<String> = set.into_iter().collect();
    result.sort();
    result
}

const BUILTIN: &[(&str, &str)] = &[
    ("owner", "拥有者"),
    ("admin", "管理员"),
    ("developer", "开发"),
    ("operator", "运维"),
    ("viewer", "只读"),
];

/// Built-in names remain stable in storage; custom role names are user data.
pub(crate) fn role_name(name: &str, builtin: bool) -> String {
    if builtin {
        let key = match name {
            "拥有者" => Some("workspace.role.owner"),
            "管理员" => Some("workspace.role.admin"),
            "开发" => Some("workspace.role.developer"),
            "运维" => Some("workspace.role.operator"),
            "只读" => Some("workspace.role.viewer"),
            _ => None,
        };
        if let Some(key) = key {
            return crate::i18n::message(key);
        }
    }
    name.to_owned()
}

fn builtin_permissions(role: &str) -> Vec<String> {
    let all = || {
        PERMISSIONS
            .iter()
            .map(|p| p.0.to_string())
            .collect::<Vec<_>>()
    };
    let reads = || {
        PERMISSIONS
            .iter()
            .filter(|p| p.0.ends_with(".read"))
            .map(|p| p.0.to_string())
            .collect::<Vec<_>>()
    };
    match role {
        "owner" | "admin" => {
            let mut set = all();
            if role == "admin" {
                set.retain(|p| p != "workspace.manage");
            }
            set
        }
        "developer" => expand(
            &[
                "repositories",
                "code-insights",
                "insight-reports",
                "code-reviews",
                "pipelines",
                "pipeline-templates",
                "credentials",
                "deployments",
                "app-templates",
                "config-templates",
                "marketplace",
                "alerts",
                "cron-jobs",
                "variables",
            ]
            .iter()
            .flat_map(|m| {
                ["read", "create", "update", "delete"]
                    .iter()
                    .map(move |a| format!("{m}.{a}"))
            })
            .chain([
                "pipelines.run".into(),
                "code-insights.sync".into(),
                "insight-reports.generate".into(),
                "code-reviews.run".into(),
                "deployments.apply".into(),
                "dashboard.read".into(),
            ])
            .filter(|p| PERMISSIONS.iter().any(|x| x.0 == *p))
            .collect::<Vec<_>>(),
        ),
        "operator" => expand(
            &["host-groups", "hosts", "tags", "cron-jobs"]
                .iter()
                .flat_map(|m| {
                    ["read", "create", "update", "delete"]
                        .iter()
                        .map(move |a| format!("{m}.{a}"))
                })
                .chain([
                    "hosts.terminal".into(),
                    "hosts.files".into(),
                    "hosts.enroll".into(),
                    "dashboard.read".into(),
                    "activity.read".into(),
                ])
                .collect::<Vec<_>>(),
        ),
        _ => reads()
            .into_iter()
            .filter(|p| p != "variables.read")
            .collect(),
    }
}

/// New resource permissions must reach existing immutable built-in roles.
/// Preserve all old grants and never change custom roles.
pub async fn prepare_permissions(s: &AppState) -> Result<()> {
    use sea_orm::sea_query::{Expr, ExprTrait};
    let rows = workspace_roles::Entity::find()
        .filter(workspace_roles::Column::Builtin.eq(1))
        .all(&s.db)
        .await?;
    for row in rows {
        let Some((key, _)) = BUILTIN.iter().find(|(_, name)| *name == row.name) else {
            continue;
        };
        let additions = builtin_permissions(key)
            .into_iter()
            .filter(|p| {
                p.starts_with("marketplace-sources.")
                    || p.starts_with("code-insights.")
                    || p.starts_with("insight-reports.")
                    || p.starts_with("code-reviews.")
                    || p.starts_with("config-templates.")
                    || p.starts_with("pipeline-groups.")
                    || p.starts_with("pipeline-environments.")
            })
            .collect::<Vec<_>>();
        if additions.is_empty() {
            continue;
        }
        let tx = crate::variables::transaction(s, row.workspace_id.into()).await?;
        let current = workspace_roles::Entity::find_by_id(row.id)
            .one(&tx)
            .await?
            .ok_or_else(ApiError::missing)?;
        let mut permissions: Vec<String> =
            serde_json::from_str(&current.permissions).map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("workspace.error.builtin_corrupt"),
                )
            })?;
        let previous = permissions.len();
        for permission in additions {
            if !permissions.contains(&permission) {
                permissions.push(permission);
            }
        }
        if previous != permissions.len() {
            workspace_roles::Entity::update_many()
                .col_expr(
                    workspace_roles::Column::Permissions,
                    Expr::value(serde_json::to_string(&permissions).unwrap()),
                )
                .col_expr(
                    workspace_roles::Column::Version,
                    Expr::col(workspace_roles::Column::Version).add(1),
                )
                .col_expr(workspace_roles::Column::UpdatedAt, Expr::value(db::now()))
                .filter(workspace_roles::Column::Id.eq(row.id))
                .exec(&tx)
                .await?;
        }
        tx.commit().await?;
    }
    Ok(())
}

/// An authenticated caller acting inside one workspace, with that workspace's
/// permissions already resolved.
#[derive(Clone)]
pub struct Scope {
    pub workspace_id: Id,
    pub user: crate::accounts::auth::User,
    pub permissions: HashSet<String>,
}

impl Scope {
    pub fn require(&self, permission: &str) -> Result<()> {
        if self.permissions.contains(permission) {
            Ok(())
        } else {
            Err(ApiError::forbidden())
        }
    }
}

impl FromRequestParts<AppState> for Scope {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, s: &AppState) -> Result<Self> {
        let account = Account::from_request_parts(parts, s).await?;
        let Path(params) =
            Path::<std::collections::HashMap<String, String>>::from_request_parts(parts, s)
                .await
                .map_err(|_| ApiError::missing())?;
        let workspace = params
            .get("workspace")
            .filter(|value| value.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|value| value.parse::<Id>().ok())
            .filter(|id| i64::from(*id) > 0)
            .ok_or_else(ApiError::missing)?;
        Self::for_user(&s.db, workspace, account.user).await
    }
}

impl Scope {
    /// Scheduled work re-resolves account activity, membership and current permissions.
    pub async fn for_actor(s: &AppState, workspace: Id, actor: Id) -> Result<Self> {
        Self::for_actor_in(&s.db, workspace, actor).await
    }
    pub(crate) async fn for_actor_in(
        db: &impl sea_orm::ConnectionTrait,
        workspace: Id,
        actor: Id,
    ) -> Result<Self> {
        use crate::entities::users;
        let row = users::Entity::find_by_id(i64::from(actor))
            .filter(users::Column::Active.eq(1))
            .one(db)
            .await?
            .ok_or_else(ApiError::forbidden)?;
        Self::for_user(
            db,
            workspace,
            crate::accounts::auth::User {
                id: row.id.into(),
                email: row.email,
                name: row.name,
                is_superadmin: row.is_superadmin == 1,
            },
        )
        .await
    }
    async fn for_user(
        db: &impl sea_orm::ConnectionTrait,
        workspace: Id,
        user: crate::accounts::auth::User,
    ) -> Result<Self> {
        if user.is_superadmin {
            let row = workspaces::Entity::find_by_id(i64::from(workspace))
                .one(db)
                .await?
                .ok_or_else(ApiError::missing)?;
            return Ok(Self {
                workspace_id: Id::from(row.id),
                user: user,
                permissions: PERMISSIONS.iter().map(|p| p.0.to_owned()).collect(),
            });
        }
        #[derive(FromQueryResult)]
        struct Membership {
            id: i64,
            permissions: String,
        }
        let row = workspaces::Entity::find()
            .select_only()
            .column(workspaces::Column::Id)
            .column(workspace_roles::Column::Permissions)
            .join(
                JoinType::InnerJoin,
                sea_orm::RelationDef::from(
                    workspace_members::Entity::belongs_to(workspaces::Entity)
                        .from(workspace_members::Column::WorkspaceId)
                        .to(workspaces::Column::Id),
                )
                .rev(),
            )
            .join(
                JoinType::InnerJoin,
                workspace_members::Entity::belongs_to(workspace_roles::Entity)
                    .from(workspace_members::Column::RoleId)
                    .to(workspace_roles::Column::Id)
                    .into(),
            )
            .filter(workspaces::Column::Id.eq(i64::from(workspace)))
            .filter(workspace_members::Column::UserId.eq(i64::from(user.id)))
            .filter(workspace_members::Column::State.eq("active"))
            .into_model::<Membership>()
            .one(db)
            .await?
            // A workspace the caller is not a member of is reported as missing,
            // not forbidden: otherwise the response confirms it exists.
            .ok_or_else(ApiError::missing)?;
        let permissions: HashSet<String> = serde_json::from_str::<Vec<String>>(&row.permissions)
            .unwrap_or_default()
            .into_iter()
            .collect();
        Ok(Self {
            workspace_id: Id::from(row.id),
            user: user,
            permissions,
        })
    }
}

fn valid_slug(slug: &str) -> bool {
    (2..=40).contains(&slug.len())
        && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        // Reserved so a workspace can never shadow an application route.
        && !matches!(slug, "api" | "assets" | "new" | "login" | "register" | "join" | "settings")
}

pub(crate) fn join_code() -> String {
    use rand::Rng;
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut rng = rand::thread_rng();
    (0..10)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

#[derive(Deserialize)]
pub struct NewWorkspace {
    name: String,
    slug: String,
}

pub async fn create(
    State(s): State<AppState>,
    account: Account,
    Json(body): Json<NewWorkspace>,
) -> Result<Json<Value>> {
    use sea_orm::TransactionTrait;
    let tx = s.db.begin().await?;
    let value = create_in(&s, account.user.id, body, &tx).await?;
    tx.commit().await?;
    Ok(Json(value))
}

pub(crate) async fn create_in(
    s: &AppState,
    user_id: Id,
    body: NewWorkspace,
    conn: &impl sea_orm::ConnectionTrait,
) -> Result<Value> {
    let slug = body.slug.trim().to_lowercase();
    let name = body.name.trim().to_owned();
    if !valid_slug(&slug) {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("workspace.error.slug_invalid"),
        ));
    }
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("workspace.error.name_invalid"),
        ));
    }
    if workspaces::Entity::find()
        .filter(workspaces::Column::Slug.eq(slug.clone()))
        .one(conn)
        .await?
        .is_some()
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("workspace.error.slug_used"),
        ));
    }
    let (id, now, code) = (s.next_id()?, db::now(), join_code());
    workspaces::Entity::insert(workspaces::ActiveModel {
        id: Set(i64::from(id)),
        slug: Set(slug.clone()),
        name: Set(name.clone()),
        owner_id: Set(i64::from(user_id)),
        join_code: Set(code.clone()),
        pipeline_run_number: Set(0),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(conn)
    .await?;
    let mut owner_role = Id::default();
    for (key, label) in BUILTIN {
        let role_id = s.next_id()?;
        if *key == "owner" {
            owner_role = role_id.clone();
        }
        workspace_roles::Entity::insert(workspace_roles::ActiveModel {
            id: Set(i64::from(role_id)),
            workspace_id: Set(i64::from(id)),
            name: Set((*label).to_owned()),
            permissions: Set(serde_json::to_string(&builtin_permissions(key)).unwrap()),
            builtin: Set(1),
            created_at: Set(now),
            updated_at: Set(now),
            version: Set(1),
        })
        .exec(conn)
        .await?;
    }
    workspace_members::Entity::insert(workspace_members::ActiveModel {
        id: Set(i64::from(s.next_id()?)),
        workspace_id: Set(i64::from(id)),
        user_id: Set(i64::from(user_id)),
        role_id: Set(i64::from(owner_role)),
        state: Set("active".to_owned()),
        created_at: Set(now),
        updated_at: Set(now),
        version: Set(1),
    })
    .exec(conn)
    .await?;
    crate::maintenance::seed(s, conn, id).await?;
    Ok(json!({"id": id, "slug": slug, "name": name, "join_code": code}))
}

#[derive(Deserialize)]
pub struct JoinRequest {
    code: String,
}

/// Join by invite code. The code identifies the workspace, so an unauthorized
/// caller never learns whether a given slug exists.
pub async fn join(
    State(s): State<AppState>,
    account: Account,
    Json(body): Json<JoinRequest>,
) -> Result<Json<Value>> {
    let code = body.code.trim().to_uppercase();
    let row = workspaces::Entity::find()
        .filter(workspaces::Column::JoinCode.eq(code.clone()))
        .filter(workspaces::Column::JoinCode.ne(""))
        .one(&s.db)
        .await?
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("workspace.error.invite_invalid"),
            )
        })?;
    use sea_orm::TransactionTrait;
    let tx = s.db.begin().await?;
    let row = crate::accounts::workspace_admin::lock_workspace(&tx, Id::from(row.id)).await?;
    if row.join_code != code {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("workspace.error.invite_invalid"),
        ));
    }
    let workspace_id = Id::from(row.id);
    if workspace_members::Entity::find()
        .filter(workspace_members::Column::WorkspaceId.eq(i64::from(workspace_id)))
        .filter(workspace_members::Column::UserId.eq(i64::from(account.user.id)))
        .one(&tx)
        .await?
        .is_some()
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("workspace.error.already_member"),
        ));
    }
    // New members land on the read-only role; an admin promotes them.
    let viewer = workspace_roles::Entity::find()
        .filter(workspace_roles::Column::WorkspaceId.eq(i64::from(workspace_id)))
        .filter(workspace_roles::Column::Name.eq("只读"))
        .filter(workspace_roles::Column::Builtin.eq(1))
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let now = db::now();
    workspace_members::Entity::insert(workspace_members::ActiveModel {
        id: Set(i64::from(s.next_id()?)),
        workspace_id: Set(i64::from(workspace_id)),
        user_id: Set(i64::from(account.user.id)),
        role_id: Set(viewer.id),
        state: Set("active".to_owned()),
        created_at: Set(now),
        updated_at: Set(now),
        version: Set(1),
    })
    .exec(&tx)
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"id": Id::from(row.id), "slug": row.slug, "name": row.name}),
    ))
}

/// The permission catalogue the role editor renders. Static, but served so the
/// interface never keeps its own copy that can drift from the server's.
pub async fn permission_catalogue() -> Json<Value> {
    let items: Vec<Value> = PERMISSIONS
        .iter()
        .map(|(key, group, action)| {
            json!({"key": key, "group": crate::i18n::message(group), "action": crate::i18n::message(action), "requires": dependencies(key)})
        })
        .collect();
    Json(json!({"items": items}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn legacy_builtin_roles_receive_insight_permissions_once_without_changing_custom_roles() {
        use sea_orm::ActiveModelTrait;

        let (dir, db) = crate::lock::tests::database().await;
        let s = std::sync::Arc::new(crate::state::State {
            ids: std::sync::Arc::new(
                crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap(),
            ),
            db,
            data_dir: dir.path().into(),
            instance_id: "legacy-role-permissions-test".into(),
            server_mode: false,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), false).unwrap(),
            host_monitor: Default::default(),
            cluster: None,
        });
        workspaces::Entity::insert(workspaces::ActiveModel {
            id: Set(10),
            slug: Set("legacy-permissions".into()),
            name: Set("Legacy workspace".into()),
            owner_id: Set(1),
            join_code: Set("legacy-permissions-code".into()),
            pipeline_run_number: Set(0),
            created_at: Set(1234),
            updated_at: Set(1234),
        })
        .exec(&s.db)
        .await
        .unwrap();

        let mut legacy = Vec::new();
        for (index, (key, label)) in BUILTIN.iter().enumerate() {
            let mut permissions = builtin_permissions(key);
            permissions.retain(|permission| {
                !["code-insights.", "insight-reports.", "code-reviews."]
                    .iter()
                    .any(|prefix| permission.starts_with(prefix))
            });
            permissions.push("legacy.existing.read".into());
            let role = workspace_roles::ActiveModel {
                id: Set(100 + index as i64),
                workspace_id: Set(10),
                name: Set((*label).into()),
                permissions: Set(serde_json::to_string(&permissions).unwrap()),
                builtin: Set(1),
                created_at: Set(1234),
                updated_at: Set(1234),
                version: Set(17),
            }
            .insert(&s.db)
            .await
            .unwrap();
            legacy.push((*key, role, permissions));
        }
        let custom = workspace_roles::ActiveModel {
            id: Set(200),
            workspace_id: Set(10),
            name: Set("Custom role".into()),
            permissions: Set(r#"["dashboard.read","code-insights.sync","legacy.custom"]"#.into()),
            builtin: Set(0),
            created_at: Set(1234),
            updated_at: Set(1234),
            version: Set(23),
        }
        .insert(&s.db)
        .await
        .unwrap();

        prepare_permissions(&s).await.unwrap();

        let mut upgraded = Vec::new();
        for (key, previous, old_permissions) in &legacy {
            let row = workspace_roles::Entity::find_by_id(previous.id)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap();
            let permissions: Vec<String> = serde_json::from_str(&row.permissions).unwrap();
            let mut expected = old_permissions.clone();
            match *key {
                "owner" | "admin" | "developer" => expected.extend(
                    ["read", "create", "update", "delete", "sync"]
                        .map(|action| format!("code-insights.{action}")),
                ),
                "viewer" => expected.push("code-insights.read".into()),
                "operator" => {}
                _ => unreachable!(),
            }
            for (resource, action) in [("insight-reports", "generate"), ("code-reviews", "run")] {
                match *key {
                    "owner" | "admin" | "developer" => {
                        expected.extend(
                            ["read", "create", "update", "delete", action]
                                .map(|verb| format!("{resource}.{verb}")),
                        );
                    }
                    "viewer" => expected.push(format!("{resource}.read")),
                    "operator" => {}
                    _ => unreachable!(),
                }
            }
            assert_eq!(
                permissions.iter().collect::<HashSet<_>>(),
                expected.iter().collect::<HashSet<_>>(),
                "unexpected grants for {key}"
            );
            assert_eq!(
                permissions.len(),
                expected.len(),
                "duplicate grants for {key}"
            );
            assert_eq!(
                row.version,
                previous.version + i64::from(*key != "operator")
            );
            assert_eq!(row.created_at, previous.created_at);
            if *key == "operator" {
                assert_eq!(&row, previous);
            } else {
                assert!(row.updated_at > previous.updated_at);
            }
            upgraded.push(row);
        }
        assert_eq!(
            workspace_roles::Entity::find_by_id(custom.id)
                .one(&s.db)
                .await
                .unwrap(),
            Some(custom.clone())
        );

        prepare_permissions(&s).await.unwrap();

        for previous in upgraded.into_iter().chain([custom]) {
            assert_eq!(
                workspace_roles::Entity::find_by_id(previous.id)
                    .one(&s.db)
                    .await
                    .unwrap(),
                Some(previous),
                "a repeated upgrade changed the role"
            );
        }
    }

    #[test]
    fn dependencies_are_transitively_expanded() {
        let expanded = expand(&["marketplace.install".to_string()]);
        for required in [
            "marketplace.read",
            "app-templates.read",
            "deployments.create",
            "deployments.read",
        ] {
            assert!(
                expanded.contains(&required.to_string()),
                "missing {required}"
            );
        }
    }

    #[test]
    fn every_dependency_is_a_real_permission() {
        for (key, _, _) in PERMISSIONS {
            for dependency in dependencies(key) {
                assert!(
                    PERMISSIONS.iter().any(|p| p.0 == dependency),
                    "{key} depends on unknown {dependency}"
                );
            }
        }
    }

    #[test]
    fn builtin_roles_are_closed_under_dependencies() {
        for (role, _) in BUILTIN {
            let granted = builtin_permissions(role);
            for permission in &granted {
                for dependency in dependencies(permission) {
                    assert!(
                        granted.contains(&dependency),
                        "role {role} grants {permission} without {dependency}"
                    );
                }
            }
        }
    }

    #[test]
    fn viewer_cannot_read_secret_values_and_only_owner_manages_the_workspace() {
        let viewer = builtin_permissions("viewer");
        assert!(!viewer.contains(&"variables.read".to_string()));
        assert!(viewer.iter().all(|p| p.ends_with(".read")));
        assert!(builtin_permissions("owner").contains(&"workspace.manage".to_string()));
        assert!(!builtin_permissions("admin").contains(&"workspace.manage".to_string()));
    }

    #[test]
    fn reserved_and_malformed_slugs_are_refused() {
        for good in ["acme", "acme-ops", "a1", "team-7"] {
            assert!(valid_slug(good), "{good}");
        }
        for bad in [
            "a",
            "API",
            "api",
            "new",
            "-x",
            "x-",
            "has space",
            "有中文",
            &"x".repeat(41),
        ] {
            assert!(!valid_slug(bad), "{bad}");
        }
    }
}
