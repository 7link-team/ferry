//! Approval policy never changes authorization. Unknown write operations remain
//! manual in assisted mode; only explicit draft/organization edits are ordinary.
use super::*;

fn request_mode() -> String {
    "request".into()
}
pub(super) fn default_mode() -> String {
    request_mode()
}
pub(super) fn mode(config: &str) -> Result<String> {
    let config: Value = serde_json::from_str(config).map_err(|_| invalid())?;
    let value = config["approval_mode"].as_str().unwrap_or("request");
    if !["request", "assisted", "yolo"].contains(&value) {
        return Err(invalid());
    }
    Ok(value.into())
}
pub(super) fn automatic(mode: &str, call: &crate::platform_tools::Call) -> bool {
    if call.read_only() {
        return false;
    }
    if mode == "yolo" {
        return true;
    }
    if mode != "assisted" || !["POST", "PATCH"].contains(&call.method.as_str()) {
        return false;
    }
    let path = call.path.split('?').next().unwrap_or("");
    let parts: Vec<_> = path.trim_matches('/').split('/').collect();
    // These APIs save drafts or organization metadata, and do not execute them.
    [
        "node-groups",
        "host-groups",
        "workflow-groups",
        "pipeline-groups",
        "workflows",
        "pipelines",
        "app-templates",
        "config-templates",
    ]
    .contains(&parts[0])
        && (parts.len() == 1 || (parts.len() == 2 && call.method == "PATCH"))
}

pub(super) async fn decide_automatically(
    s: &AppState,
    row: &turns::Model,
    owner: &str,
    request: &str,
    expected_mode: &str,
) -> Result<()> {
    let (tx, mut current, c) = run::leased(s, row, owner).await?;
    if mode(&c.config)? != expected_mode || current.state != "running" {
        return Err(conflict());
    }
    let mut decisions: Value = serde_json::from_str(&current.decisions).map_err(|_| invalid())?;
    let item = decisions
        .get_mut(request)
        .filter(|item| item["tool"] == "ferry_request" && item["decision"].is_null())
        .ok_or_else(conflict)?;
    item["decision"] = json!(true);
    item["dispatching"] = json!(true);
    item["automatic"] = json!(true);
    item["approval_mode"] = json!(expected_mode);
    item["decided_by"] = json!(Id::from(row.owner_id));
    current.decisions = decisions.to_string();
    current.version += 1;
    current.into_active_model().reset_all().update(&tx).await?;
    let scope = Scope::for_actor_in(&tx, row.workspace_id.into(), row.owner_id.into()).await?;
    crate::hosts::activity_in(s,&tx,&scope,"decision","conversations",&Id::from(row.conversation_id),
        json!({"turn_id":Id::from(row.id),"request_id":request,"allow":true,"automatic":true,"approval_mode":expected_mode})).await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Update {
    mode: String,
    version: i64,
}
/// Human owner only. Sets this conversation's MCP approval mode. Cannot change
/// during execution or through an Agent tool call; it never changes role grants.
pub(super) async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<Update>,
) -> Result<Json<Value>> {
    scope.require("conversations.update")?;
    if crate::platform_tools::current_agent().is_some() {
        return Err(ApiError::forbidden());
    }
    if !["request", "assisted", "yolo"].contains(&body.mode.as_str()) {
        return Err(bad("conversations.approvalModeInvalid"));
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut c = find(&tx, &scope, id).await?;
    if c.owner_id != i64::from(scope.user.id) {
        return Err(ApiError::missing());
    }
    if c.version != body.version || has_active(&tx, id).await? {
        return Err(conflict());
    }
    let old = mode(&c.config)?;
    let mut config: Value = serde_json::from_str(&c.config).map_err(|_| invalid())?;
    config["approval_mode"] = json!(body.mode);
    c.config = config.to_string();
    c.version += 1;
    c.updated_at = db::now();
    let version = c.version;
    c.into_active_model().reset_all().update(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "conversations",
        &id,
        json!({"changes":{"approval_mode":{"before":old,"after":body.mode}}}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true,"version":version})))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn assisted_mode_only_approves_known_nonexecuting_writes() {
        for (method, path, ordinary) in [
            ("POST", "/workflows", true),
            ("PATCH", "/workflows/12", true),
            ("POST", "/node-groups", true),
            ("DELETE", "/workflows/12", false),
            ("POST", "/workflows/12/executions", false),
            ("POST", "/workflows/12/publications", false),
            ("POST", "/roles", false),
            ("PATCH", "/credentials/12", false),
            ("POST", "/nodes/12/commands", false),
            ("POST", "/cron-jobs", false),
            ("PATCH", "/conversations/12", false),
            ("POST", "/future-api", false),
        ] {
            let call = crate::platform_tools::Call {
                method: method.into(),
                path: path.into(),
                ..Default::default()
            };
            assert_eq!(automatic("assisted", &call), ordinary, "{method} {path}");
            assert!(!automatic("request", &call));
            assert!(automatic("yolo", &call));
        }
        assert_eq!(mode("{}").unwrap(), "request");
        assert!(mode("{\"approval_mode\":\"unknown\"}").is_err());
    }
}
