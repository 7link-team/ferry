use crate::entities::{credentials, pipelines, repositories};
use crate::snowflake::Id;
use crate::{
    db,
    error::{ApiError, Result},
    resource::{self, ListQuery, Spec},
    state::AppState,
    workspace::Scope,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use sea_orm::{
    ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set,
    sea_query::{Expr, ExprTrait},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

const SPEC: Spec = Spec {
    search: &["name", "url"],
    filters: &[],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "name",
};

fn to_json(row: &repositories::Model) -> Value {
    json!({
        "id": Id::from(row.id),
        "name": row.name,
        "url": row.url,
        "default_branch": row.default_branch,
        "credential_id": Id::from(row.credential_id),
        "description": row.description,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
        "version": row.version,
    })
}

#[derive(Deserialize)]
pub struct Body {
    name: String,
    url: String,
    #[serde(default)]
    default_branch: String,
    #[serde(default)]
    credential_id: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    version: i64,
}

impl Body {
    fn branch(&self) -> &str {
        match self.default_branch.trim() {
            "" => "main",
            branch => branch,
        }
    }
}

fn valid_url(url: &str) -> bool {
    // Match MySQL's VARCHAR(255), so every backend accepts the same URLs.
    if url.is_empty()
        || url.chars().count() > 255
        || url.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return false;
    }
    for prefix in ["https://", "http://", "ssh://"] {
        if let Some(rest) = url.strip_prefix(prefix) {
            return !rest.is_empty() && !rest.starts_with('/');
        }
    }
    url.strip_prefix("git@")
        .and_then(|rest| rest.split_once(':'))
        .is_some_and(|(host, path)| !host.is_empty() && !host.contains('/') && !path.is_empty())
}

async fn check(s: &AppState, scope: &Scope, body: &Body) -> Result<String> {
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::bad("请填写名称（60 字以内）。"));
    }
    if !valid_url(body.url.trim()) {
        return Err(ApiError::bad(
            "请填写有效的远程仓库地址（255 字以内，仅支持 HTTP、HTTPS、SSH 或 git@host:path）。",
        ));
    }
    if Id::reference(&body.credential_id)? != Id::from(0) {
        crate::repository_auth::validate_target(
            s,
            scope.workspace_id,
            Id::reference(&body.credential_id)?,
            body.url.trim(),
            false,
        )
        .await?;
        let row = credentials::Entity::find()
            .filter(credentials::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(credentials::Column::Id.eq(Id::reference(&body.credential_id)?))
            .one(&s.db)
            .await?
            .ok_or_else(|| ApiError::bad("所选凭据不存在于当前工作空间。"))?;
        if !crate::credentials::KINDS.contains(&row.kind.as_str()) {
            return Err(ApiError::bad("仓库凭据必须是密码、SSH 私钥或令牌。"));
        }
    }
    Ok(name.to_owned())
}

pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("repositories.read")?;
    let query = ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) = resource::list(
        &s.db,
        &SPEC,
        repositories::Entity::find(),
        &scope.workspace_id,
        &query,
    )
    .await?;
    Ok(Json(json!({
        "items": rows.iter().map(to_json).collect::<Vec<_>>(),
        "total": total, "page": query.page, "page_size": query.page_size,
    })))
}

pub async fn get(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("repositories.read")?;
    let row = repositories::Entity::find()
        .filter(repositories::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(repositories::Column::Id.eq(id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok(Json(to_json(&row)))
}

pub async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<Body>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("repositories.create")?;
    let name = check(&s, &scope, &body).await?;
    let (id, now) = (s.next_id()?, db::now());
    repositories::Entity::insert(repositories::ActiveModel {
        id: Set(i64::from(id)),
        workspace_id: Set(i64::from(scope.workspace_id)),
        name: Set(name.clone()),
        url: Set(body.url.trim().to_owned()),
        default_branch: Set(body.branch().to_owned()),
        credential_id: Set(i64::from(Id::reference(&body.credential_id)?)),
        description: Set(body.description.trim().to_owned()),
        created_at: Set(now),
        updated_at: Set(now),
        version: Set(1),
        ..Default::default()
    })
    .exec(&s.db)
    .await?;
    // The details of an audit entry are readable by anyone with activity.read,
    // so they carry the name and never the secret.
    crate::hosts::activity(
        &s,
        &scope,
        "create",
        "repositories",
        &id,
        json!({"name": name}),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

pub async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Json(body): Json<Body>,
) -> Result<Json<Value>> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("repositories.update")?;
    let name = check(&s, &scope, &body).await?;
    let affected = repositories::Entity::update_many()
        .col_expr(repositories::Column::Name, Expr::value(name.clone()))
        .col_expr(repositories::Column::Url, Expr::value(body.url.trim()))
        .col_expr(
            repositories::Column::DefaultBranch,
            Expr::value(body.branch()),
        )
        .col_expr(
            repositories::Column::CredentialId,
            Expr::value(Id::reference(&body.credential_id)?),
        )
        .col_expr(
            repositories::Column::Description,
            Expr::value(body.description.trim()),
        )
        .col_expr(repositories::Column::UpdatedAt, Expr::value(db::now()))
        .col_expr(
            repositories::Column::Version,
            Expr::col(repositories::Column::Version).add(1),
        )
        .filter(repositories::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(repositories::Column::Id.eq(id))
        .filter(repositories::Column::Version.eq(body.version))
        .exec(&s.db)
        .await?
        .rows_affected;
    if affected == 0 {
        return Err(crate::hosts::stale(
            &s,
            repositories::Entity::find()
                .filter(repositories::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(repositories::Column::Id.eq(id)),
        )
        .await?);
    }
    crate::hosts::activity(
        &s,
        &scope,
        "update",
        "repositories",
        &id,
        json!({"name": name}),
    )
    .await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn remove(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<StatusCode> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("repositories.delete")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    if crate::entities::marketplace_sources::Entity::find()
        .filter(crate::entities::marketplace_sources::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(crate::entities::marketplace_sources::Column::RepositoryId.eq(id))
        .one(&tx)
        .await?
        .is_some()
    {
        return Err(ApiError::conflict("仓库仍被市场来源引用，请先删除来源。"));
    }
    let used = pipelines::Entity::find()
        .filter(pipelines::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(pipelines::Column::RepositoryId.eq(id))
        .count(&tx)
        .await?;
    if used > 0 {
        return Err(ApiError::conflict(format!(
            "还有 {used} 条流水线在用这个仓库，请先改掉再删除。"
        )));
    }
    let affected = repositories::Entity::delete_many()
        .filter(repositories::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(repositories::Column::Id.eq(id))
        .exec(&tx)
        .await?
        .rows_affected;
    if affected == 0 {
        return Err(ApiError::missing());
    }
    crate::hosts::activity_in(&s, &tx, &scope, "delete", "repositories", &id, json!({})).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Acceptance check added during review: the URL is interpolated into a
    /// `git clone` on the target host, so anything that could become a second
    /// argument or a git option must not get through.
    #[test]
    fn refuses_anything_that_could_become_a_git_argument() {
        for hostile in [
            // Option injection — the classic git clone RCE vector.
            "--upload-pack=/bin/sh",
            "-u/bin/sh",
            "--config=core.sshCommand=/bin/sh",
            "https://example.com/a.git --upload-pack=/bin/sh",
            "https://example.com/a.git\t--upload-pack=x",
            // A second argument smuggled past the first.
            "https://example.com/a.git /etc/shadow",
            // Local disk on the machine that runs the clone.
            "file:///etc/passwd",
            "/etc/passwd",
            "../../etc/passwd",
            "~/.ssh/id_rsa",
            // Newline could end one shell line and start another.
            "https://example.com/a.git\nrm -rf /",
            "https://example.com/a.git\rwhoami",
            // Scheme that is not a remote.
            "ext::sh -c whoami",
            "HTTPS://example.com/a.git",
        ] {
            assert!(!valid_url(hostile), "must refuse: {hostile:?}");
        }
    }

    #[test]
    fn accepts_remote_git_urls() {
        for url in [
            "https://github.com/a/b.git",
            "http://example.com/repo",
            "git@github.com:a/b.git",
            "ssh://git@example.com/repo",
        ] {
            assert!(valid_url(url), "{url}");
        }
        assert!(valid_url(&format!("https://{}", "a".repeat(247))));
    }

    #[test]
    fn refuses_local_empty_malformed_and_overlong_urls() {
        for url in [
            "file:///etc",
            "/etc/passwd",
            "",
            "./repo",
            "C:\\repo",
            "https://",
            "ssh:///etc",
            "git@:repo",
            "git@host:",
            "https://host/has space",
            "https://host/\nrepo",
            "ftp://host/repo",
        ] {
            assert!(!valid_url(url), "{url}");
        }
        assert!(!valid_url(&format!("https://{}", "a".repeat(248))));
    }

    #[test]
    fn blank_branch_defaults_to_main() {
        let mut body: Body =
            serde_json::from_value(json!({"name":"repo", "url":"https://example.com/repo"}))
                .unwrap();
        assert_eq!(body.branch(), "main");
        body.default_branch = "  ".into();
        assert_eq!(body.branch(), "main");
        body.default_branch = " develop ".into();
        assert_eq!(body.branch(), "develop");
    }
}
