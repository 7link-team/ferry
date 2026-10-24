use crate::{
    db::{self, execute, one, text},
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
use sea_orm::QueryResult;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

const SPEC: Spec = Spec {
    table: "repositories",
    search: &["name", "url"],
    filters: &[],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "name",
};
const COLUMNS: &str =
    "id,name,url,default_branch,credential_id,description,created_at,updated_at,version";

fn to_json(row: &QueryResult) -> Value {
    json!({
        "id": text(row, "id"),
        "name": text(row, "name"),
        "url": text(row, "url"),
        "default_branch": text(row, "default_branch"),
        "credential_id": text(row, "credential_id"),
        "description": text(row, "description"),
        "created_at": db::int(row, "created_at"),
        "updated_at": db::int(row, "updated_at"),
        "version": db::int(row, "version"),
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
    if !body.credential_id.trim().is_empty() {
        let row = one(
            &s.db,
            "SELECT kind FROM credentials WHERE workspace_id = ? AND id = ?",
            vec![
                scope.workspace_id.clone().into(),
                body.credential_id.trim().into(),
            ],
        )
        .await?
        .ok_or_else(|| ApiError::bad("所选凭据不存在于当前工作空间。"))?;
        if !crate::credentials::KINDS.contains(&text(&row, "kind").as_str()) {
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
    let (rows, total) = resource::list(&s.db, &SPEC, COLUMNS, &scope.workspace_id, &query).await?;
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
    scope.require("repositories.read")?;
    let row = one(
        &s.db,
        &format!("SELECT {COLUMNS} FROM repositories WHERE workspace_id = ? AND id = ?"),
        vec![scope.workspace_id.into(), id.into()],
    )
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
    let (id, now) = (db::id(), db::now());
    execute(
        &s.db,
        "INSERT INTO repositories (id,workspace_id,name,url,default_branch,credential_id,description,created_at,updated_at,version)
         VALUES (?,?,?,?,?,?,?,?,?,1)",
        vec![
            id.clone().into(),
            scope.workspace_id.clone().into(),
            name.clone().into(),
            body.url.trim().into(),
            body.branch().into(),
            body.credential_id.trim().into(),
            body.description.trim().into(),
            now.into(),
            now.into(),
        ],
    )
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
    scope.require("repositories.update")?;
    let name = check(&s, &scope, &body).await?;
    let affected = execute(
        &s.db,
        "UPDATE repositories SET name=?,url=?,default_branch=?,credential_id=?,description=?,updated_at=?,version=version+1
         WHERE workspace_id=? AND id=? AND version=?",
        vec![
            name.clone().into(),
            body.url.trim().into(),
            body.branch().into(),
            body.credential_id.trim().into(),
            body.description.trim().into(),
            db::now().into(),
            scope.workspace_id.clone().into(),
            id.clone().into(),
            body.version.into(),
        ],
    ).await?.rows_affected();
    if affected == 0 {
        return Err(crate::hosts::stale(&s, "repositories", &scope.workspace_id, &id).await?);
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
    scope.require("repositories.delete")?;
    // Add a reference check here once pipelines can reference repositories.
    let affected = execute(
        &s.db,
        "DELETE FROM repositories WHERE workspace_id = ? AND id = ?",
        vec![scope.workspace_id.clone().into(), id.clone().into()],
    )
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(ApiError::missing());
    }
    crate::hosts::activity(&s, &scope, "delete", "repositories", &id, json!({})).await?;
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
