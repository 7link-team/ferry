//! Workspaces, roles, and the `Scope` extractor.
//!
//! `Scope` is the tenancy invariant of the whole system: a workspace handler
//! can only obtain a `workspace_id` by going through it, and it only yields
//! one after proving the caller is an active member. No handler reads a
//! workspace id from a request body, and none is expected to remember to
//! filter — `resource::list` always filters by the scope's id.

use crate::snowflake::Id;

use crate::{
    auth::Account,
    db,
    entities::{workspace_members, workspace_roles, workspaces},
    error::{ApiError, Result},
    state::AppState,
};
use axum::{
    Json,
    extract::{FromRequestParts, Path, State},
    http::request::Parts,
};
use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, JoinType, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;

/// Every permission in the product, with the labels the role editor shows.
/// `{resource}.{action}` where the resource is the REST resource name.
pub const PERMISSIONS: &[(&str, &str, &str)] = &[
    ("dashboard.read", "仪表盘", "查看"),
    ("host-groups.read", "主机组", "查看"),
    ("host-groups.create", "主机组", "新增"),
    ("host-groups.update", "主机组", "修改"),
    ("host-groups.delete", "主机组", "删除"),
    ("hosts.read", "主机", "查看"),
    ("hosts.create", "主机", "新增"),
    ("hosts.update", "主机", "修改"),
    ("hosts.delete", "主机", "删除"),
    ("hosts.enroll", "主机", "生成接入命令"),
    ("hosts.terminal", "主机", "远程终端"),
    ("hosts.files", "主机", "文件传输"),
    ("tags.read", "标签", "查看"),
    ("tags.create", "标签", "新增"),
    ("tags.update", "标签", "修改"),
    ("tags.delete", "标签", "删除"),
    ("repositories.read", "仓库", "查看"),
    ("repositories.create", "仓库", "新增"),
    ("repositories.update", "仓库", "修改"),
    ("repositories.delete", "仓库", "删除"),
    ("pipelines.read", "流水线", "查看"),
    ("pipelines.create", "流水线", "新增"),
    ("pipelines.update", "流水线", "修改"),
    ("pipelines.delete", "流水线", "删除"),
    ("pipelines.run", "流水线", "执行"),
    ("pipeline-templates.read", "流水线模版", "查看"),
    ("pipeline-templates.create", "流水线模版", "新增"),
    ("pipeline-templates.update", "流水线模版", "修改"),
    ("pipeline-templates.delete", "流水线模版", "删除"),
    ("credentials.read", "授权认证", "查看"),
    ("credentials.create", "授权认证", "新增"),
    ("credentials.update", "授权认证", "修改"),
    ("credentials.delete", "授权认证", "删除"),
    ("deployments.read", "应用部署", "查看"),
    ("deployments.create", "应用部署", "新增"),
    ("deployments.update", "应用部署", "修改"),
    ("deployments.delete", "应用部署", "删除"),
    ("deployments.apply", "应用部署", "执行部署"),
    ("deployments.rollback", "应用部署", "回滚"),
    ("app-templates.read", "应用模版", "查看"),
    ("app-templates.create", "应用模版", "新增"),
    ("app-templates.update", "应用模版", "修改"),
    ("app-templates.delete", "应用模版", "删除"),
    ("marketplace.read", "应用市场", "查看"),
    ("marketplace.install", "应用市场", "安装"),
    ("alerts.read", "报警监控", "查看"),
    ("alerts.create", "报警监控", "新增"),
    ("alerts.update", "报警监控", "修改"),
    ("alerts.delete", "报警监控", "删除"),
    ("cron-jobs.read", "定时任务", "查看"),
    ("cron-jobs.create", "定时任务", "新增"),
    ("cron-jobs.update", "定时任务", "修改"),
    ("cron-jobs.delete", "定时任务", "删除"),
    ("cron-jobs.run", "定时任务", "立即执行"),
    ("variables.read", "密钥变量", "查看"),
    ("variables.create", "密钥变量", "新增"),
    ("variables.update", "密钥变量", "修改"),
    ("variables.delete", "密钥变量", "删除"),
    ("variables.reveal", "密钥变量", "查看明文"),
    ("members.read", "成员管理", "查看"),
    ("members.create", "成员管理", "邀请"),
    ("members.update", "成员管理", "改角色"),
    ("members.delete", "成员管理", "移除"),
    ("roles.read", "角色管理", "查看"),
    ("roles.create", "角色管理", "新增"),
    ("roles.update", "角色管理", "修改"),
    ("roles.delete", "角色管理", "删除"),
    ("settings.read", "配置项", "查看"),
    ("settings.update", "配置项", "修改"),
    ("dictionaries.read", "字典", "查看"),
    ("dictionaries.create", "字典", "新增"),
    ("dictionaries.update", "字典", "修改"),
    ("dictionaries.delete", "字典", "删除"),
    ("activity.read", "操作记录", "查看"),
    ("workspace.manage", "工作空间", "改名/邀请码/转让"),
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
        "hosts.terminal" | "hosts.files" | "hosts.enroll" => &["hosts.read"],
        "pipelines.run" => &["repositories.read"],
        "deployments.apply" | "deployments.rollback" => &["hosts.read", "app-templates.read"],
        "marketplace.install" => &["app-templates.read", "deployments.create"],
        "variables.reveal" => &["variables.read"],
        "cron-jobs.run" => &["hosts.read"],
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
                "pipelines",
                "pipeline-templates",
                "credentials",
                "deployments",
                "app-templates",
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

/// An authenticated caller acting inside one workspace, with that workspace's
/// permissions already resolved.
pub struct Scope {
    pub workspace_id: Id,
    pub user: crate::auth::User,
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
        let slug = params
            .get("workspace")
            .ok_or_else(ApiError::missing)?
            .clone();
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
            .filter(workspaces::Column::Slug.eq(slug.clone()))
            .filter(workspace_members::Column::UserId.eq(i64::from(account.user.id)))
            .filter(workspace_members::Column::State.eq("active"))
            .into_model::<Membership>()
            .one(&s.db)
            .await?
            // A workspace the caller is not a member of is reported as missing,
            // not forbidden: otherwise the response confirms it exists.
            .ok_or_else(ApiError::missing)?;
        let permissions: HashSet<String> = serde_json::from_str::<Vec<String>>(&row.permissions)
            .unwrap_or_default()
            .into_iter()
            .collect();
        let _ = slug;
        Ok(Self {
            workspace_id: Id::from(row.id),
            user: account.user,
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

fn join_code() -> String {
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
    let slug = body.slug.trim().to_lowercase();
    let name = body.name.trim().to_owned();
    if !valid_slug(&slug) {
        return Err(ApiError::bad(
            "标识只能用小写字母、数字和中划线，2–40 个字符，且不能用保留词。",
        ));
    }
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::bad("请填写工作空间名称（60 字以内）。"));
    }
    if workspaces::Entity::find()
        .filter(workspaces::Column::Slug.eq(slug.clone()))
        .one(&s.db)
        .await?
        .is_some()
    {
        return Err(ApiError::conflict("该标识已被占用，请换一个。"));
    }
    let (id, now, code) = (s.next_id()?, db::now(), join_code());
    workspaces::Entity::insert(workspaces::ActiveModel {
        id: Set(i64::from(id)),
        slug: Set(slug.clone()),
        name: Set(name.clone()),
        owner_id: Set(i64::from(account.user.id)),
        join_code: Set(code.clone()),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(&s.db)
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
        })
        .exec(&s.db)
        .await?;
    }
    workspace_members::Entity::insert(workspace_members::ActiveModel {
        id: Set(i64::from(s.next_id()?)),
        workspace_id: Set(i64::from(id)),
        user_id: Set(i64::from(account.user.id)),
        role_id: Set(i64::from(owner_role)),
        state: Set("active".to_owned()),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(&s.db)
    .await?;
    Ok(Json(
        json!({"id": id, "slug": slug, "name": name, "join_code": code}),
    ))
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
        .filter(workspaces::Column::JoinCode.eq(code))
        .filter(workspaces::Column::JoinCode.ne(""))
        .one(&s.db)
        .await?
        .ok_or_else(|| ApiError::bad("邀请码无效或已失效。"))?;
    let workspace_id = Id::from(row.id);
    if workspace_members::Entity::find()
        .filter(workspace_members::Column::WorkspaceId.eq(i64::from(workspace_id)))
        .filter(workspace_members::Column::UserId.eq(i64::from(account.user.id)))
        .one(&s.db)
        .await?
        .is_some()
    {
        return Err(ApiError::conflict("你已经是该工作空间的成员。"));
    }
    // New members land on the read-only role; an admin promotes them.
    let viewer = workspace_roles::Entity::find()
        .filter(workspace_roles::Column::WorkspaceId.eq(i64::from(workspace_id)))
        .filter(workspace_roles::Column::Name.eq("只读"))
        .one(&s.db)
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
    })
    .exec(&s.db)
    .await?;
    Ok(Json(json!({"slug": row.slug, "name": row.name})))
}

/// The permission catalogue the role editor renders. Static, but served so the
/// interface never keeps its own copy that can drift from the server's.
pub async fn permission_catalogue() -> Json<Value> {
    let items: Vec<Value> = PERMISSIONS
        .iter()
        .map(|(key, group, action)| {
            json!({"key": key, "group": group, "action": action, "requires": dependencies(key)})
        })
        .collect();
    Json(json!({"items": items}))
}

pub async fn roles(State(s): State<AppState>, scope: Scope) -> Result<Json<Value>> {
    scope.require("roles.read")?;
    let rows = workspace_roles::Entity::find()
        .filter(workspace_roles::Column::WorkspaceId.eq(i64::from(scope.workspace_id)))
        .order_by_desc(workspace_roles::Column::Builtin)
        .order_by_asc(workspace_roles::Column::Name)
        .all(&s.db)
        .await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": Id::from(row.id),
                "name": row.name,
                "builtin": row.builtin == 1,
                "permissions": serde_json::from_str::<Value>(&row.permissions).unwrap_or(json!([])),
            })
        })
        .collect();
    let total = items.len() as u64;
    Ok(Json(
        json!({"items": items, "total": total, "page": 1, "page_size": total.max(1)}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
