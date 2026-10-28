//! CSV/XLSX transfer reuses resource projections and transactional creation.
use crate::{
    db,
    entities::resource_imports as imports,
    error::{ApiError, Result},
    snowflake::Id,
    state::AppState,
    workspace::Scope,
};
use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use calamine::{Data, Reader, Xlsx};
use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, Set, TransactionTrait};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io::{Cursor, Read, Seek, SeekFrom},
};
use tokio::io::AsyncReadExt;
const MAX_UPLOAD: usize = 8 * 1024 * 1024;
const MAX_ROWS: usize = 1000;
#[derive(Clone, Copy)]
struct Resource(&'static str);
fn fields(resource: &str) -> Result<Vec<(&'static str, &'static str)>> {
    let spec = match resource {
        "hosts" => {
            "name connection address port:number username group_id credential_id description tag_ids:json"
        }
        "host-groups" => "name description parent_id",
        "pipeline-groups" => "name description",
        "pipeline-environments" => "name description target_kind target_ref",
        "tags" => "name color",
        "credentials" => "name kind username description secret:secret",
        "repositories" => "name provider url default_branch credential_id description",
        "pipelines" => {
            "name definition definition_format executor_bindings:json checkout_bindings:json graph_layout:json repository_id target_kind target_ref description group_id application_id tag_ids:json environment_ids:json"
        }
        "deployments" => "name manifest mode target_kind target_ref workdir description",
        "variables" => "name scope scope_ref kind description value:secret",
        "cron-jobs" => {
            "name description scope schedule timezone enabled:bool action target_kind target_ref command timeout_seconds:number max_retries:number retry_delay_seconds:number environment_id"
        }
        "alert-rules" => {
            "name description target_kind target_ref metric threshold:number enabled:bool webhook:secret"
        }
        "dictionaries" => "key name description items",
        "roles" => "name permissions:json",
        "marketplace-sources" => {
            "name repository_id provider api_base project organization branch index_path"
        }
        "app-templates" => "name description content parameters",
        "pipeline-templates" => "name description content parameters",
        "config-templates" => "name description content parameters",
        _ => return Err(ApiError::bad("此资源不支持导入。")),
    };
    Ok(spec
        .split_whitespace()
        .map(|s| s.split_once(':').unwrap_or((s, "text")))
        .collect())
}
pub fn routes() -> Router<AppState> {
    let mut router = Router::new();
    for resource in [
        "hosts",
        "host-groups",
        "tags",
        "credentials",
        "repositories",
        "pipelines",
        "pipeline-groups",
        "pipeline-environments",
        "deployments",
        "variables",
        "cron-jobs",
        "alert-rules",
        "dictionaries",
        "roles",
        "marketplace-sources",
        "app-templates",
        "pipeline-templates",
        "config-templates",
        "activity",
        "members",
        "settings",
        "marketplace",
        "alert-events",
        "cron-runs",
        "pipeline-runs",
        "pipeline-artifacts",
        "deployment-revisions",
    ] {
        let extension = Extension(Resource(resource));
        let path = format!("/{{workspace}}/{resource}");
        router = router.route(&format!("{path}/export"), get(export).layer(extension));
        if fields(resource).is_ok() {
            router = router
                .route(&format!("{path}/template"), get(template).layer(extension))
                .route(
                    &format!("{path}/imports"),
                    post(preview)
                        .layer::<_, std::convert::Infallible>(DefaultBodyLimit::max(
                            MAX_UPLOAD + 65536,
                        ))
                        .layer(extension),
                )
                .route(
                    &format!("{path}/imports/{{id}}/commit"),
                    post(commit).layer(extension),
                )
                .route(
                    &format!("{path}/imports/{{id}}"),
                    axum::routing::delete(cancel).layer(extension),
                );
        }
    }
    router
}
async fn page(s: AppState, scope: Scope, r: &str, q: HashMap<String, String>) -> Result<Value> {
    let Json(value) = match r {
        "hosts" => crate::hosts::list_hosts(State(s), scope, Query(q)).await?,
        "host-groups" => crate::hosts::list_groups(State(s), scope, Query(q)).await?,
        "tags" => crate::hosts::list_tags(State(s), scope, Query(q)).await?,
        "credentials" => crate::credentials::list(State(s), scope, Query(q)).await?,
        "repositories" => crate::repositories::list(State(s), scope, Query(q)).await?,
        "pipelines" => crate::pipelines::list(State(s), scope, Query(q)).await?,
        "pipeline-groups" => {
            crate::pipeline_contexts::groups::list(State(s), scope, Query(q)).await?
        }
        "pipeline-environments" => {
            crate::pipeline_contexts::environments::list(State(s), scope, Query(q)).await?
        }
        "deployments" => crate::deployments::list(State(s), scope, Query(q)).await?,
        "variables" => crate::variables::list(State(s), scope, Query(q)).await?,
        "cron-jobs" => crate::cron_jobs::list(State(s), scope, Query(q)).await?,
        "alert-rules" => crate::alerts::list(State(s), scope, Query(q)).await?,
        "dictionaries" => crate::maintenance::list_dictionaries(State(s), scope, Query(q)).await?,
        "roles" => crate::workspace_admin::roles(State(s), scope, Query(q)).await?,
        "marketplace-sources" => {
            crate::marketplace::list_sources(State(s), scope, Query(q)).await?
        }
        "app-templates" => crate::templates::apps::list(State(s), scope, Query(q)).await?,
        "pipeline-templates" => {
            crate::templates::pipelines::list(State(s), scope, Query(q)).await?
        }
        "config-templates" => crate::templates::configs::list(State(s), scope, Query(q)).await?,
        "deployment-revisions" => {
            crate::deployments::revision_list(State(s), scope, Query(q)).await?
        }
        "activity" => crate::hosts::list_activity(State(s), scope, Query(q)).await?,
        "members" => crate::workspace_admin::members(State(s), scope, Query(q)).await?,
        "settings" => crate::maintenance::list_settings(State(s), scope, Query(q)).await?,
        "marketplace" => crate::marketplace::list(State(s), scope, Query(q)).await?,
        "alert-events" => crate::alerts::history(State(s), scope, Query(q)).await?,
        "cron-runs" => crate::cron_jobs::list_runs(State(s), scope, Query(q)).await?,
        "pipeline-runs" => crate::pipeline_runs::list(State(s), scope, Query(q)).await?,
        "pipeline-artifacts" => crate::artifacts::list(State(s), scope, Query(q)).await?,
        _ => return Err(ApiError::missing()),
    };
    Ok(value)
}
async fn create(
    s: &AppState,
    tx: &DatabaseTransaction,
    scope: &Scope,
    r: &str,
    body: Value,
) -> Result<Value> {
    macro_rules! decode { () => {serde_json::from_value(body).map_err(|_|ApiError::bad("字段缺失或类型不正确，请按模板填写。"))?} }
    let Json(value) = match r {
        "hosts" => {
            crate::hosts::create_host_in(s, tx, scope, decode!())
                .await?
                .1
        }
        "host-groups" => {
            crate::hosts::create_group_in(s, tx, scope, decode!())
                .await?
                .1
        }
        "tags" => {
            crate::hosts::create_tag_in(s, tx, scope, decode!())
                .await?
                .1
        }
        "credentials" => {
            crate::credentials::create_in(s, tx, scope, decode!())
                .await?
                .1
        }
        "repositories" => {
            crate::repositories::create_in(s, tx, scope, decode!())
                .await?
                .1
        }
        "pipeline-groups" => {
            crate::pipeline_contexts::groups::create_in(s, tx, scope, decode!())
                .await?
                .1
        }
        "pipeline-environments" => {
            crate::pipeline_contexts::environments::create_in(s, tx, scope, decode!())
                .await?
                .1
        }
        "pipelines" => {
            crate::pipelines::create_in(s, tx, scope, decode!())
                .await?
                .1
        }
        "deployments" => {
            crate::deployments::create_in(s, tx, scope, decode!())
                .await?
                .1
        }
        "variables" => {
            crate::variables::create_in(s, tx, scope, decode!())
                .await?
                .1
        }
        "cron-jobs" => {
            crate::cron_jobs::create_in(s, tx, scope, decode!())
                .await?
                .1
        }
        "alert-rules" => crate::alerts::create_in(s, tx, scope, decode!()).await?.1,
        "dictionaries" => {
            crate::maintenance::create_dictionary_in(s, tx, scope, decode!())
                .await?
                .1
        }
        "roles" => crate::workspace_admin::create_role_in(s, tx, scope, decode!()).await?,
        "marketplace-sources" => {
            crate::marketplace::create_source_in(s, tx, scope, decode!())
                .await?
                .1
        }
        "app-templates" => {
            crate::templates::apps::create_in(s, tx, scope, decode!())
                .await?
                .1
        }
        "pipeline-templates" => {
            crate::templates::pipelines::create_in(s, tx, scope, decode!())
                .await?
                .1
        }
        "config-templates" => {
            crate::templates::configs::create_in(s, tx, scope, decode!())
                .await?
                .1
        }
        _ => return Err(ApiError::missing()),
    };
    Ok(value)
}

fn invalid_file() -> ApiError {
    ApiError::bad("文件格式无效，请使用下载的 CSV/XLSX 模板。")
}
fn parse_rows(bytes: Vec<u8>, xlsx: bool, r: &str) -> Result<Vec<(usize, Value)>> {
    let fields = fields(r)?;
    let mut records: Vec<(usize, Vec<String>)> = Vec::new();
    if xlsx {
        // Bound decompression and coordinates before calamine allocates its dense range.
        let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).map_err(|_| invalid_file())?;
        if archive.len() > 200 {
            return Err(invalid_file());
        }
        let mut total = 0u64;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(|_| invalid_file())?;
            total = total.checked_add(entry.size()).ok_or_else(invalid_file)?;
            if total > 32 * 1024 * 1024 {
                return Err(ApiError::bad("XLSX 解压内容超过32MB。"));
            }
            let mut content = Vec::new();
            (&mut entry)
                .take(32 * 1024 * 1024 + 1)
                .read_to_end(&mut content)?;
            if content.len() > 32 * 1024 * 1024 {
                return Err(invalid_file());
            }
            let entry_name = entry.name().map_err(|_| invalid_file())?;
            if entry_name.ends_with(".xml") {
                let mut reader = quick_xml::Reader::from_reader(content.as_slice());
                loop {
                    use quick_xml::events::Event;
                    match reader.read_event().map_err(|_| invalid_file())? {
                        Event::Start(e) | Event::Empty(e) => {
                            if e.local_name().as_ref() == "f" {
                                return Err(ApiError::bad("导入文件不能包含公式，请先粘贴为值。"));
                            }
                            for attr in e.attributes() {
                                let attr = attr.map_err(|_| invalid_file())?;
                                if matches!(e.local_name().as_ref(), "c" | "row" | "dimension")
                                    && (attr.key.as_ref() == "r" || attr.key.as_ref() == "ref")
                                {
                                    let value = quick_xml::escape::unescape(attr.value.as_ref())
                                        .map_err(|_| invalid_file())?;
                                    let value = value.replace('$', "");
                                    for coord in value.split(':') {
                                        let digits = coord.trim_start_matches(|c: char| {
                                            c.is_ascii_alphabetic() || c == '$'
                                        });
                                        if let Ok(row) = digits.parse::<usize>() {
                                            if row > MAX_ROWS + 1 {
                                                return Err(ApiError::bad(
                                                    "每次最多导入1000行，表格不得包含远端空白单元格。",
                                                ));
                                            }
                                        }
                                        let mut col = 0usize;
                                        for c in coord.bytes().take_while(u8::is_ascii_alphabetic) {
                                            col = col.saturating_mul(26).saturating_add(
                                                (c.to_ascii_uppercase() - b'A' + 1) as usize,
                                            );
                                        }
                                        if col > 64 {
                                            return Err(ApiError::bad("表格最多64列。"));
                                        }
                                    }
                                }
                            }
                        }
                        Event::DocType(_) => return Err(invalid_file()),
                        Event::Eof => break,
                        _ => {}
                    }
                }
            }
        }
        let mut book: Xlsx<_> = Xlsx::new(Cursor::new(bytes)).map_err(|_| invalid_file())?;
        let name = book
            .sheet_names()
            .first()
            .cloned()
            .ok_or_else(invalid_file)?;
        let formulas = book.worksheet_formula(&name).map_err(|_| invalid_file())?;
        if formulas.cells().any(|(_, _, v)| !v.is_empty()) {
            return Err(ApiError::bad("导入文件不能包含公式。"));
        }
        let range = book.worksheet_range(&name).map_err(|_| invalid_file())?;
        let start = range.start().map_or(0, |p| p.0) as usize;
        for (i, row) in range.rows().enumerate() {
            let values = row
                .iter()
                .map(|v| match v {
                    Data::Empty => Ok(String::new()),
                    Data::String(s) => Ok(s.clone()),
                    Data::Int(n) => Ok(n.to_string()),
                    Data::Float(n) => Ok(n.to_string()),
                    Data::Bool(b) => Ok(b.to_string()),
                    _ => Err(invalid_file()),
                })
                .collect::<Result<Vec<_>>>()?;
            records.push((start + i + 1, values));
        }
    } else {
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .from_reader(bytes);
        let mut previous_offset = 0usize;
        let mut line = 1usize;
        for row in reader.records() {
            let row = row.map_err(|_| invalid_file())?;
            let mut offset = row.position().map_or(0, |p| p.byte() as usize);
            while bytes
                .get(offset)
                .is_some_and(|b| matches!(b, b'\r' | b'\n'))
            {
                offset += 1;
            }
            line += bytes[previous_offset..offset]
                .iter()
                .enumerate()
                .filter(|(i, b)| {
                    **b == b'\r'
                        || (**b == b'\n'
                            && (previous_offset + *i == 0
                                || bytes[previous_offset + *i - 1] != b'\r'))
                })
                .count();
            previous_offset = offset;
            records.push((line, row.iter().map(str::to_owned).collect()));
            if records.len() > MAX_ROWS + 1 {
                return Err(ApiError::bad("每次最多导入1000行。"));
            }
        }
    }
    if records.is_empty() {
        return Err(ApiError::bad("文件为空。"));
    }
    let (_, headers) = records.remove(0);
    let mut used = HashSet::new();
    let mut kinds = Vec::new();
    for header in &headers {
        if !used.insert(header.as_str()) {
            return Err(ApiError::bad("表头重复。"));
        }
        let kind = fields
            .iter()
            .find(|(name, _)| name == header)
            .map(|(_, k)| *k)
            .ok_or_else(|| ApiError::bad("存在未知表头，请使用当前模板。"))?;
        kinds.push(kind);
    }
    if headers.is_empty() || headers.len() > 64 {
        return Err(invalid_file());
    }
    let mut output = Vec::new();
    for (line, row) in records {
        if row.iter().all(String::is_empty) {
            continue;
        }
        let mut body = serde_json::Map::new();
        for (i, key) in headers.iter().enumerate() {
            let value = row.get(i).map(String::as_str).unwrap_or("");
            // Blank optional fields use the same defaults as the normal form.
            if value.is_empty() && kinds[i] != "secret" {
                continue;
            }
            let parsed = match kinds[i] {
                "number" | "bool" | "json" => {
                    serde_json::from_str(value).unwrap_or_else(|_| json!(value))
                }
                _ => json!(value),
            };
            body.insert(key.clone(), parsed);
        }
        output.push((line, Value::Object(body)));
    }
    if output.is_empty() || output.len() > MAX_ROWS {
        return Err(ApiError::bad("请填写1–1000行数据。"));
    }
    Ok(output)
}

async fn preview(
    State(s): State<AppState>,
    scope: Scope,
    Extension(Resource(r)): Extension<Resource>,
    mut upload: Multipart,
) -> Result<Json<Value>> {
    scope.require(&format!(
        "{}.create",
        if r == "alert-rules" { "alerts" } else { r }
    ))?;
    let field = upload
        .next_field()
        .await
        .map_err(|_| invalid_file())?
        .ok_or_else(invalid_file)?;
    let name = field.file_name().unwrap_or("").to_ascii_lowercase();
    let xlsx = if name.ends_with(".xlsx") {
        true
    } else if name.ends_with(".csv") {
        false
    } else {
        return Err(invalid_file());
    };
    let bytes = field.bytes().await.map_err(|_| invalid_file())?;
    if bytes.len() > MAX_UPLOAD {
        return Err(ApiError::bad("文件不能超过8MB。"));
    }
    if upload
        .next_field()
        .await
        .map_err(|_| invalid_file())?
        .is_some()
    {
        return Err(ApiError::bad("每次只上传一个文件。"));
    }
    let rows = tokio::task::spawn_blocking(move || parse_rows(bytes.to_vec(), xlsx, r))
        .await
        .map_err(|_| invalid_file())??;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let scope = crate::workspace_admin::authorize(
        &tx,
        &scope,
        &format!("{}.create", if r == "alert-rules" { "alerts" } else { r }),
    )
    .await?;
    let mut results = Vec::new();
    let mut valid = true;
    for (line, body) in &rows {
        let savepoint = tx.begin().await?;
        let error = match create(&s, &savepoint, &scope, r, body.clone()).await {
            Ok(_) => {
                savepoint.commit().await?;
                None
            }
            Err(e) => {
                savepoint.rollback().await?;
                valid = false;
                Some(e.1)
            }
        };
        let mut masked = body.clone();
        for (key, kind) in fields(r)? {
            if kind == "secret" && masked.get(key).is_some() {
                masked[key] = json!("••••••");
            }
        }
        results.push(json!({"row":line,"values":masked,"error":error}));
    }
    tx.rollback().await?;
    let id = if valid {
        let id = s.next_id()?;
        let payload = s.vault.seal(
            &serde_json::to_string(&rows).map_err(|_| invalid_file())?,
            &crate::vault::aad(
                &scope.workspace_id.to_string(),
                "resource-imports",
                &id.to_string(),
            ),
        )?;
        imports::Entity::insert(imports::ActiveModel {
            id: Set(id.into()),
            workspace_id: Set(scope.workspace_id.into()),
            actor_id: Set(scope.user.id.into()),
            resource: Set(r.into()),
            payload: Set(payload),
            result: Set("[]".into()),
            created_at: Set(db::now()),
            committed_at: Set(0),
        })
        .exec(&s.db)
        .await?;
        Some(id)
    } else {
        None
    };
    Ok(Json(
        json!({"id":id,"valid":valid,"count":rows.len(),"rows":results}),
    ))
}
async fn owned(tx: &DatabaseTransaction, scope: &Scope, r: &str, id: Id) -> Result<imports::Model> {
    imports::Entity::find_by_id(i64::from(id))
        .filter(imports::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(imports::Column::ActorId.eq(scope.user.id))
        .filter(imports::Column::Resource.eq(r))
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)
}
async fn commit(
    State(s): State<AppState>,
    scope: Scope,
    Extension(Resource(r)): Extension<Resource>,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require(&format!(
        "{}.create",
        if r == "alert-rules" { "alerts" } else { r }
    ))?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let scope = crate::workspace_admin::authorize(
        &tx,
        &scope,
        &format!("{}.create", if r == "alert-rules" { "alerts" } else { r }),
    )
    .await?;
    let row = owned(&tx, &scope, r, id).await?;
    if row.committed_at != 0 {
        return Ok(Json(
            json!({"items":serde_json::from_str::<Value>(&row.result).map_err(|_|invalid_file())?}),
        ));
    }
    let plaintext = s.vault.open_secret(
        &row.payload,
        &crate::vault::aad(
            &scope.workspace_id.to_string(),
            "resource-imports",
            &id.to_string(),
        ),
    )?;
    let rows: Vec<(usize, Value)> = serde_json::from_str(&plaintext).map_err(|_| invalid_file())?;
    let mut results = Vec::new();
    for (line, body) in rows {
        results.push(
            create(&s, &tx, &scope, r, body)
                .await
                .map_err(|e| ApiError(e.0, format!("第{line}行：{}；整批未导入。", e.1)))?,
        );
    }
    use sea_orm::sea_query::Expr;
    imports::Entity::update_many()
        .col_expr(imports::Column::Payload, Expr::value(""))
        .col_expr(
            imports::Column::Result,
            Expr::value(json!(results).to_string()),
        )
        .col_expr(imports::Column::CommittedAt, Expr::value(db::now()))
        .filter(imports::Column::Id.eq(id))
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "import",
        r,
        &id,
        json!({"count":results.len()}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"items":results})))
}
async fn cancel(
    State(s): State<AppState>,
    scope: Scope,
    Extension(Resource(r)): Extension<Resource>,
    Path((_, id)): Path<(String, Id)>,
) -> Result<StatusCode> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let row = owned(&tx, &scope, r, id).await?;
    if row.committed_at != 0 {
        return Err(ApiError::conflict("已经导入，不能取消。"));
    }
    imports::Entity::delete_by_id(i64::from(id))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

fn format(q: &HashMap<String, String>) -> Result<bool> {
    match q.get("format").map(String::as_str).unwrap_or("xlsx") {
        "xlsx" => Ok(true),
        "csv" => Ok(false),
        _ => Err(ApiError::bad("格式必须为 csv 或 xlsx。")),
    }
}
fn cell(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(v) => v.clone(),
        v => v.to_string(),
    }
}
fn safe_csv(value: &str) -> String {
    if value.trim_start().starts_with(['=', '+', '-', '@']) || value.starts_with(['\t', '\r', '\n'])
    {
        format!("'{value}")
    } else {
        value.into()
    }
}
/// A bounded channel feeds a blocking writer, so exports don't retain all pages.
fn write_file(
    mut rx: tokio::sync::mpsc::Receiver<Vec<Vec<String>>>,
    xlsx: bool,
) -> Result<std::fs::File> {
    let mut file = tempfile::tempfile()?;
    if xlsx {
        let mut workbook = rust_xlsxwriter::Workbook::new();
        let sheet = workbook
            .add_worksheet_with_constant_memory()
            .map_err(|_| invalid_file())?;
        let mut row = 0u32;
        while let Some(batch) = rx.blocking_recv() {
            for values in batch {
                for (col, value) in values.iter().enumerate() {
                    sheet.write_string(row, col as u16, value).map_err(|_| {
                        ApiError::bad(
                            "导出内容超过 XLSX 单元格或行数限制，请缩小筛选范围或使用 CSV。",
                        )
                    })?;
                }
                row += 1;
            }
        }
        workbook
            .save_to_writer(&mut file)
            .map_err(|_| invalid_file())?;
    } else {
        use std::io::Write;
        file.write_all(&[0xef, 0xbb, 0xbf])?;
        let mut writer = csv::Writer::from_writer(&mut file);
        while let Some(batch) = rx.blocking_recv() {
            for row in batch {
                writer
                    .write_record(row.iter().map(|v| safe_csv(v)))
                    .map_err(|_| invalid_file())?;
            }
        }
        writer.flush()?;
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(file)
}
fn download(file: std::fs::File, r: &str, xlsx: bool) -> Result<Response> {
    let size = file.metadata()?.len();
    let file = tokio::fs::File::from_std(file);
    let stream = futures::stream::try_unfold(file, |mut file| async move {
        let mut buffer = vec![0u8; 65536];
        let size = file.read(&mut buffer).await?;
        if size == 0 {
            return Ok::<_, std::io::Error>(None);
        }
        buffer.truncate(size);
        Ok(Some((bytes::Bytes::from(buffer), file)))
    });
    Ok((
        [
            (
                header::CONTENT_TYPE,
                if xlsx {
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                } else {
                    "text/csv; charset=utf-8"
                }
                .to_owned(),
            ),
            (
                header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"{r}.{}\"",
                    if xlsx { "xlsx" } else { "csv" }
                ),
            ),
            (header::CONTENT_LENGTH, size.to_string()),
            (header::CACHE_CONTROL, "no-store".to_owned()),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}
async fn template(
    scope: Scope,
    Extension(Resource(r)): Extension<Resource>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response> {
    scope.require(&format!(
        "{}.create",
        if r == "alert-rules" { "alerts" } else { r }
    ))?;
    let xlsx = format(&q)?;
    let columns = fields(r)?;
    let file=tokio::task::spawn_blocking(move||->Result<_>{
        let mut file=tempfile::tempfile()?;
        if xlsx {
            let mut book=rust_xlsxwriter::Workbook::new();
            let sheet=book.add_worksheet();sheet.set_name("data").map_err(|_|invalid_file())?;
            for (i,(name,_)) in columns.iter().enumerate(){sheet.write_string(0,i as u16,*name).map_err(|_|invalid_file())?;}
            let sheet=book.add_worksheet();sheet.set_name("字段说明").map_err(|_|invalid_file())?;
            sheet.write_string(0,0,"仅新增，不覆盖现有记录。引用字段填写当前空间的 ID。空白字段使用创建默认值；密钥不会导出。").map_err(|_|invalid_file())?;
            for (i,(name,kind)) in columns.iter().enumerate(){sheet.write_string(i as u32+1,0,*name).map_err(|_|invalid_file())?;sheet.write_string(i as u32+1,1,match *kind{"number"=>"数字","bool"=>"true 或 false","json"=>"JSON 数组","secret"=>"敏感文本，仅用于导入，不回显",_=>"文本；定义/参数/字典项遵循创建表单格式"}).map_err(|_|invalid_file())?;}
            book.save_to_writer(&mut file).map_err(|_|invalid_file())?;
        } else {
            let mut writer=csv::Writer::from_writer(&mut file);
            writer.write_record(columns.iter().map(|(name,_)|*name)).map_err(|_|invalid_file())?;writer.flush()?;
        }
        file.seek(SeekFrom::Start(0))?;Ok(file)
    }).await.map_err(|_|invalid_file())??;
    download(file, &format!("{r}-template"), xlsx)
}
async fn export(
    State(s): State<AppState>,
    scope: Scope,
    Extension(Resource(r)): Extension<Resource>,
    Query(mut q): Query<HashMap<String, String>>,
) -> Result<Response> {
    let xlsx = format(&q)?;
    q.remove("format");
    q.insert("page".into(), "1".into());
    q.insert("page_size".into(), "200".into());
    let mut data = page(s.clone(), scope.clone(), r, q.clone()).await?;
    let mut columns: Vec<String> = data["items"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(Value::as_object)
        .map(|v| v.keys().cloned().collect())
        .unwrap_or_else(|| {
            fields(r)
                .unwrap_or_default()
                .into_iter()
                .map(|(name, _)| name.to_owned())
                .collect()
        });
    // Secret variables omit `value`; a first secret row must not hide later plain values.
    if r == "variables" && !columns.iter().any(|key| key == "value") {
        columns.push("value".into());
    }
    let (tx, rx) = tokio::sync::mpsc::channel(2);
    let writer = tokio::task::spawn_blocking(move || write_file(rx, xlsx));
    let produced = async {
        tx.send(vec![columns.clone()])
            .await
            .map_err(|_| invalid_file())?;
        let mut current = 1u64;
        loop {
            let items = data["items"].as_array().ok_or_else(invalid_file)?;
            if items.is_empty() {
                break;
            }
            let batch = items
                .iter()
                .map(|item| columns.iter().map(|key| cell(&item[key])).collect())
                .collect();
            tx.send(batch).await.map_err(|_| invalid_file())?;
            let total = data["total"].as_u64().ok_or_else(invalid_file)?;
            let size = data["page_size"].as_u64().unwrap_or(200);
            if current * size >= total {
                break;
            }
            current += 1;
            q.insert("page".into(), current.to_string());
            data = page(s.clone(), scope.clone(), r, q.clone()).await?;
        }
        Ok::<_, ApiError>(())
    }
    .await;
    drop(tx);
    let file = writer.await.map_err(|_| invalid_file())??;
    produced?;
    download(file, r, xlsx)
}
