//! Administrator metadata catalog. Content remains behind the existing workspace/object ACLs.
use super::*;

pub async fn list(
    State(s): State<AppState>,
    _: Admin,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let q = raw.get("q").map(String::as_str).unwrap_or("");
    let kind = raw
        .get("kind")
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("all");
    if q.len() > 200 || !matches!(kind, "all" | "asset" | "artifact") {
        return Err(bad_query());
    }
    let page = raw
        .get("page")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1)
        .clamp(1, 1_000_000);
    let size = raw
        .get("page_size")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(20)
        .clamp(1, 100);
    let union = "(SELECT a.id,a.workspace_id,w.name AS workspace,a.sha256 AS name,a.sha256,a.mime,a.size,a.created_at,'asset' AS kind,(SELECT MIN(r.owner_id) FROM asset_refs r WHERE r.asset_id=a.id AND r.owner_kind='workflow') AS workflow_id,a.execution_id AS run_id FROM assets a JOIN workspaces w ON w.id=a.workspace_id UNION ALL SELECT a.id,a.workspace_id,w.name AS workspace,a.path AS name,a.sha256,'application/octet-stream' AS mime,a.size,a.created_at,'artifact' AS kind,a.pipeline_id AS workflow_id,a.run_id FROM pipeline_artifacts a JOIN workspaces w ON w.id=a.workspace_id) catalog";
    let predicate =
        "WHERE ($1 = 'all' OR kind = $2) AND (name LIKE $3 OR workspace LIKE $4 OR mime LIKE $5)";
    let pattern = format!("%{q}%");
    let values = vec![
        kind.to_owned().into(),
        kind.to_owned().into(),
        pattern.clone().into(),
        pattern.clone().into(),
        pattern.into(),
    ];
    let totals = query(
        &s.db,
        &format!("SELECT COUNT(*) AS count FROM {union} {predicate}"),
        values.clone(),
    )
    .await?;
    let mut items=query(&s.db,&format!("SELECT * FROM {union} {predicate} ORDER BY created_at DESC,id DESC LIMIT {size} OFFSET {}",(page-1)*size),values).await?;
    for item in &mut items {
        for key in ["id", "workspace_id", "workflow_id", "run_id"] {
            if let Some(id) = item[key].as_i64() {
                item[key] = json!(id.to_string());
            }
        }
    }
    Ok(Json(
        json!({"items":items,"total":totals.first().and_then(|v|v["count"].as_i64()).unwrap_or(0),"page":page,"page_size":size}),
    ))
}
