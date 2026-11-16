use super::*;
use crate::{
    agents::tests::admin,
    entities::{asset_refs, workflow_executions},
    error::{Json, Path},
    platform_tools::tests::setup,
    snowflake::Id,
};
use axum::extract::State;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

fn graph(params: Value) -> Value {
    json!({"engine":"ferry-graph","schema":1,"settings":{"timezone":"Asia/Shanghai"},"nodes":[
        {"id":"start","name":"Start","type":"core.trigger.manual","type_version":1,
            "params":{"input_schema":{"type":"object"}},"position":[0,0]},
        {"id":"report","name":"Report","type":"report.render","type_version":1,"params":params,"position":[300,0]}
    ],"connections":[{"from":{"node":"start","port":"main","index":0},
        "to":{"node":"report","port":"main","index":0}}]})
}

fn params() -> Value {
    json!({"format":"pdf","title":"={{ $json.title }}","subtitle":"","period":"2026-11-09 至 2026-11-15",
        "summary":"={{ $json.summary }}","metrics":"={{ $json.metrics }}","sections":"={{ $json.sections }}",
        "language":"zh","file_name":"","output_field":"file","link_ttl_days":7})
}

fn literal() -> Value {
    let mut literal = params();
    literal["title"] = json!("周报");
    literal["summary"] = json!("");
    literal["metrics"] = json!([{"label":"提交","value":128}]);
    literal["sections"] = json!([{"title":"概览","markdown":"- a","page_break":false}]);
    literal
}

#[test]
fn the_contract_accepts_expressions_and_rejects_unknown_formats() {
    super::super::graph::validate(&graph(params()).to_string()).unwrap();
    super::super::graph::validate(&graph(literal()).to_string()).unwrap();
    let mut unknown = params();
    unknown["format"] = json!("docx");
    assert!(super::super::graph::validate(&graph(unknown).to_string()).is_err());
    assert!(supports(
        &json!({"type":"report.render","params":{"format":"pdf"}})
    ));
    assert!(!supports(
        &json!({"type":"report.render","params":{"format":"docx"}})
    ));
}

#[test]
fn generated_time_uses_the_workflow_time_zone() {
    let clock = json!({"iso":"2026-11-16T09:00:00.000+08:00","zone":"Asia/Shanghai"});
    assert_eq!(generated(&clock), "2026-11-16 09:00 (Asia/Shanghai)");
    assert_eq!(
        generated(&json!({"iso":"2026-11-16T01:00:00.000Z","zone":"local"})),
        "2026-11-16 01:00"
    );
}

#[tokio::test]
async fn renders_a_pdf_file_artifact_kept_by_the_workflow() {
    let (_dir, s) = setup().await;
    let (_, Json(workflow)) = super::super::create(
        State(s.clone()),
        admin(1),
        // Saving with expressions needs the JavaScript sandbox; the run below
        // passes already resolved parameters, so literal ones suffice here.
        Json(
            serde_json::from_value(
                json!({"name":"Weekly","definition":graph(literal()).to_string()}),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    let id: Id = serde_json::from_value(workflow["id"].clone()).unwrap();
    let (_, Json(created)) = super::super::run::create(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id)),
        Json(
            serde_json::from_value(
                json!({"mode":"manual","revision":"draft","version":1,"entry_node_id":"start"}),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    let execution: Id = serde_json::from_value(created["id"].clone()).unwrap();
    let run = workflow_executions::Entity::find_by_id(i64::from(execution))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let backend = crate::artifact_store::ArtifactStore::for_state(&s)
        .await
        .unwrap();
    let store = crate::content_assets::Store {
        db: &s.db,
        backend: &backend,
        vault: &s.vault,
        ids: &s.ids,
        quota: crate::content_assets::DEFAULT_QUOTA,
    };
    let input = Item {
        json: json!({"keep":"upstream"}).as_object().unwrap().clone(),
        binary: Default::default(),
        paired: vec![],
    };
    // Parameters as the run resolves them (expressions already evaluated).
    let resolved = json!({"format":"pdf","title":"Codeup 周报","period":"2026-11-09 至 2026-11-15",
        "summary":"本周 **128** 次提交。","metrics":[{"label":"提交","value":128}],
        "sections":[{"title":"物业系统","markdown":"- 新增：排班","page_break":true}],
        "language":"zh","file_name":"","output_field":"report","link_ttl_days":7});
    let origin = Origin {
        workspace: run.workspace_id,
        workflow: run.workflow_id,
        execution: run.id,
        node: "report",
    };
    let clock = json!({"iso":"2026-11-16T09:00:00.000+08:00","zone":"Asia/Shanghai"});
    let items = execute(&s, &store, &origin, &clock, &[input], &[resolved])
        .await
        .unwrap();
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.json["keep"], "upstream", "upstream data is kept");
    let file = &item.json["report"];
    assert_eq!(file["name"], "Codeup 周报.pdf");
    assert_eq!(file["mime"], "application/pdf");
    assert!(file["artifact_id"].as_str().unwrap().starts_with("sha256:"));
    assert_eq!(
        file["artifact_id"].as_str().unwrap(),
        format!("sha256:{}", file["sha256"].as_str().unwrap())
    );
    assert!(file["size"].as_i64().unwrap() > 1000);
    assert!(file["meta"]["pages"].as_u64().unwrap() >= 2);
    // The expiring link downloads the same bytes without a session.
    let url = file["download_url"].as_str().unwrap();
    let token = url
        .split("/api/files/")
        .nth(1)
        .expect("signed download link");
    assert!(!file["expires_at"].as_str().unwrap().is_empty());
    let response = crate::file_links::download(
        State(s.clone()),
        axum::extract::Path(token.to_owned()),
        Default::default(),
    )
    .await
    .unwrap();
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment"),
        "downloads, never renders inline"
    );
    let downloaded = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(downloaded.starts_with(b"%PDF-"));
    // A tampered token is refused with a plain message.
    let tampered = format!("{}x", &token[..token.len() - 1]);
    let error = crate::file_links::download(
        State(s.clone()),
        axum::extract::Path(tampered),
        Default::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.2.as_ref().unwrap().key, "fileLink.unavailable");
    let binary = &item.binary["report"];
    assert_eq!(binary.asset, file["artifact_id"].as_str().unwrap());
    // The bytes are a PDF in the content store, kept by the workflow.
    let asset = super::super::media::resolve(&s, run.workspace_id, run.workflow_id, binary)
        .await
        .unwrap();
    let bytes = store
        .read_bytes(&s.db, run.workspace_id, asset.id)
        .await
        .unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    assert!(
        asset_refs::Entity::find()
            .filter(asset_refs::Column::AssetId.eq(asset.id))
            .filter(asset_refs::Column::OwnerKind.eq("workflow"))
            .one(&s.db)
            .await
            .unwrap()
            .is_some()
    );
    // A missing title is a named, user-facing error.
    let mut untitled = json!({"format":"pdf","title":" ","output_field":"file"});
    untitled["sections"] = json!([]);
    let error = execute(
        &s,
        &store,
        &origin,
        &clock,
        &[items[0].clone()],
        &[untitled],
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.2.as_ref().unwrap().key,
        "workflow.reportTitleRequired"
    );
}
