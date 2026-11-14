//! OpenAI/Anthropic calls share encrypted credentials, content assets and a durable cost ledger.
use crate::{
    accounts::workspace::Scope,
    content_assets::Store,
    db,
    entities::{ai_calls as calls, ai_providers as providers, credentials, workflow_executions},
    error::{ApiError, Json, Path, Result},
    snowflake::Id,
    state::AppState,
};
use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{get, put},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use ferry_protocol::sandbox::{Asset, Item};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, Set,
    TransactionTrait, sea_query::ExprTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/{workspace}/ai-providers", get(list).post(create))
        .route("/{workspace}/ai-providers/{id}", put(update))
        .route("/{workspace}/ai-calls", get(list_calls))
        .route(
            "/{workspace}/ai-calls/{id}/reconcile",
            axum::routing::post(reconcile),
        )
}
async fn list_calls(State(s): State<AppState>, scope: Scope) -> Result<Json<Value>> {
    use sea_orm::{QueryOrder, QuerySelect};
    scope.require("credentials.update_budget")?;
    let rows = calls::Entity::find()
        .filter(calls::Column::WorkspaceId.eq(scope.workspace_id))
        .order_by_desc(calls::Column::CreatedAt)
        .limit(100)
        .all(&s.db)
        .await?;
    Ok(Json(
        json!({"items":rows.iter().map(|c|json!({"id":Id::from(c.id),"provider_id":Id::from(c.provider_id),"execution_id":Id::from(c.execution_id),"state":c.state,"currency":c.currency,"period_key":c.period_key,"reserved_micros":c.reserved_micros,"actual_micros":c.actual_micros,"created_at":c.created_at})).collect::<Vec<_>>()}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reconciliation {
    actual_micros: i64,
    reason: String,
}
async fn reconcile(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<Reconciliation>,
) -> Result<Json<Value>> {
    scope.require("credentials.update_budget")?;
    if !(0..=1_000_000_000_000).contains(&body.actual_micros)
        || body.reason.trim().is_empty()
        || body.reason.len() > 2000
    {
        return Err(invalid());
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    crate::accounts::workspace_admin::authorize(&tx, &scope, "credentials.update_budget").await?;
    let row = calls::Entity::find_by_id(id)
        .filter(calls::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    if row.state != "unknown"
        && !(matches!(row.state.as_str(), "sent" | "reserved")
            && row.created_at < db::now() - 180_000)
    {
        return Err(invalid());
    }
    let mut active = row.into_active_model();
    active.state = Set("reconciled".into());
    active.actual_micros = Set(body.actual_micros);
    active.updated_at = Set(db::now());
    active.updated_by = Set(scope.user.id.into());
    active.update(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "reconcile",
        "ai-calls",
        &id,
        json!({"actual_micros":body.actual_micros,"reason":body.reason}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
fn error(key: &'static str) -> ApiError {
    ApiError::localized(StatusCode::BAD_REQUEST, crate::i18n::Message::new(key))
}
fn invalid() -> ApiError {
    error("workflow.aiInvalid")
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Price {
    model: String,
    operation: String,
    input_micros: i64,
    output_micros: i64,
    unit_size: i64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Body {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) credential_id: Id,
    /// Input only: never serialize into revisions, audit or responses.
    #[serde(default, skip_serializing)]
    pub(crate) api_key: Option<String>,
    pub(crate) base_url: String,
    pub(crate) protocol: String,
    /// Metering is optional: a connection without prices is unmetered and has no budget.
    #[serde(default = "default_currency")]
    currency: String,
    #[serde(default = "default_timezone")]
    timezone: String,
    #[serde(default)]
    monthly_limit_micros: i64,
    qps: i64,
    concurrency: i64,
    #[serde(default)]
    prices: Vec<Price>,
    #[serde(default)]
    version: i64,
}
fn default_currency() -> String {
    "USD".into()
}
fn default_timezone() -> String {
    "UTC".into()
}
pub(crate) fn view(row: &providers::Model) -> Value {
    json!({"id":Id::from(row.id),"name":row.name,"credential_id":Id::from(row.credential_id),"base_url":row.base_url,"protocol":row.protocol,"currency":row.currency,"timezone":row.timezone,"monthly_limit_micros":row.monthly_limit_micros,"qps":row.qps,"concurrency":row.concurrency,"prices":serde_json::from_str::<Value>(&row.prices).unwrap_or(Value::Null),"version":row.version})
}
async fn list(State(s): State<AppState>, scope: Scope) -> Result<Json<Value>> {
    scope.require("credentials.read")?;
    let rows = providers::Entity::find()
        .filter(providers::Column::WorkspaceId.eq(scope.workspace_id))
        .all(&s.db)
        .await?;
    Ok(Json(
        json!({"items":rows.iter().map(view).collect::<Vec<_>>()}),
    ))
}
async fn validate(conn: &impl ConnectionTrait, workspace: Id, body: &Body) -> Result<()> {
    let url = reqwest::Url::parse(&body.base_url).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || body.base_url.len() > 512
        || !matches!(body.protocol.as_str(), "openai" | "anthropic")
        || body.name.trim().is_empty()
        || body.name.len() > 120
        || body.currency.len() != 3
        || !body.currency.bytes().all(|b| b.is_ascii_uppercase())
        || body.timezone.parse::<chrono_tz::Tz>().is_err()
        || body.monthly_limit_micros < 0
        || body.monthly_limit_micros > 1_000_000_000_000
        || !(1..=1000).contains(&body.qps)
        || !(1..=100).contains(&body.concurrency)
        || body.prices.len() > 100
    {
        return Err(invalid());
    }
    let mut seen = std::collections::HashSet::new();
    for price in &body.prices {
        if price.model.is_empty()
            || price.model.len() > 128
            || !operations().contains(&price.operation.as_str())
            || !(0..=1_000_000_000_000).contains(&price.input_micros)
            || !(0..=1_000_000_000_000).contains(&price.output_micros)
            || !(1..=1_000_000_000).contains(&price.unit_size)
            || !seen.insert((&price.model, &price.operation))
        {
            return Err(invalid());
        }
    }
    credential(conn, workspace.into(), body.credential_id.into()).await?;
    Ok(())
}
async fn credential(conn: &impl ConnectionTrait, workspace: i64, id: i64) -> Result<()> {
    let row = credentials::Entity::find_by_id(id)
        .filter(credentials::Column::WorkspaceId.eq(workspace))
        .one(conn)
        .await?
        .ok_or_else(invalid)?;
    if row.scope != "ai" || row.kind != "token" || row.secret.is_empty() {
        return Err(invalid());
    }
    Ok(())
}
async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<Body>,
) -> Result<(StatusCode, Json<Value>)> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let row = save_in(&s, &scope, &tx, None, body).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(view(&row))))
}
async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<Body>,
) -> Result<Json<Value>> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let row = save_in(&s, &scope, &tx, Some(id), body).await?;
    tx.commit().await?;
    Ok(Json(view(&row)))
}
pub(crate) async fn save_in(
    s: &AppState,
    scope: &Scope,
    tx: &sea_orm::DatabaseTransaction,
    id: Option<Id>,
    mut body: Body,
) -> Result<providers::Model> {
    let permission = if id.is_some() {
        "credentials.update"
    } else {
        "credentials.create"
    };
    for permission in [
        permission,
        "credentials.update_price",
        "credentials.update_budget",
    ] {
        scope.require(permission)?;
        crate::accounts::workspace_admin::authorize(tx, scope, permission).await?;
    }
    // Keep the existing encrypted-store identity: reservations are keyed by it.
    // An omitted key on edit retains the secret; no stored secret is returned.
    if let Some(id) = id {
        let existing = providers::Entity::find_by_id(id)
            .filter(providers::Column::WorkspaceId.eq(scope.workspace_id))
            .one(tx)
            .await?
            .ok_or_else(ApiError::missing)?;
        if existing.version != body.version {
            return Err(ApiError::localized(
                StatusCode::CONFLICT,
                crate::i18n::Message::new("resource.conflict"),
            ));
        }
        if i64::from(body.credential_id) == 0 {
            body.credential_id = Id::from(existing.credential_id);
        }
        if body.credential_id != Id::from(existing.credential_id) {
            return Err(invalid());
        }
    }
    if let Some(key) = body
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        if key.len() > 16_384 || key.chars().any(char::is_control) {
            return Err(invalid());
        }
        if id.is_none() {
            let created = crate::credentials::create_in(
                s,
                tx,
                scope,
                serde_json::from_value(
                    json!({"scope":"ai", "name":body.name.chars().take(60).collect::<String>(),
                    "kind":"token", "secret":key}),
                )
                .map_err(|_| invalid())?,
            )
            .await?;
            body.credential_id =
                serde_json::from_value(created.1.0["id"].clone()).map_err(|_| invalid())?;
        } else {
            let sealed = s.vault.seal(
                key,
                &crate::vault::aad(
                    &scope.workspace_id.to_string(),
                    "credentials",
                    &body.credential_id.to_string(),
                ),
            )?;
            credentials::Entity::update_many()
                .col_expr(
                    credentials::Column::Secret,
                    sea_orm::sea_query::Expr::value(sealed),
                )
                .col_expr(
                    credentials::Column::UpdatedAt,
                    sea_orm::sea_query::Expr::value(db::now()),
                )
                .col_expr(
                    credentials::Column::Version,
                    sea_orm::sea_query::Expr::col(credentials::Column::Version).add(1),
                )
                .filter(credentials::Column::Id.eq(body.credential_id))
                .filter(credentials::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(credentials::Column::Scope.eq("ai"))
                .exec(tx)
                .await?;
        }
    }
    validate(tx, scope.workspace_id, &body).await?;
    if let Some(id) = id {
        let row = providers::Entity::find_by_id(id)
            .filter(providers::Column::WorkspaceId.eq(scope.workspace_id))
            .one(tx)
            .await?
            .ok_or_else(ApiError::missing)?;
        if row.version != body.version {
            return Err(ApiError::localized(
                StatusCode::CONFLICT,
                crate::i18n::Message::new("resource.conflict"),
            ));
        }
        // A provider's credential/currency/timezone identity cannot move existing reservations.
        if row.credential_id != i64::from(body.credential_id)
            || row.currency != body.currency
            || row.timezone != body.timezone
        {
            return Err(invalid());
        }
        let mut active = row.into_active_model();
        active.name = Set(body.name.trim().into());
        active.base_url = Set(body.base_url.trim_end_matches('/').into());
        active.protocol = Set(body.protocol);
        active.monthly_limit_micros = Set(body.monthly_limit_micros);
        active.qps = Set(body.qps);
        active.concurrency = Set(body.concurrency);
        active.prices = Set(serde_json::to_string(&body.prices).map_err(|_| invalid())?);
        active.version = Set(body.version + 1);
        active.updated_at = Set(db::now());
        active.updated_by = Set(scope.user.id.into());
        let row = active.update(tx).await?;
        crate::hosts::activity_in(
            s,
            tx,
            scope,
            "update",
            "ai-providers",
            &id,
            json!({"name":row.name}),
        )
        .await?;
        Ok(row)
    } else {
        let now = db::now();
        let id: i64 = s.next_id()?.into();
        let row = providers::Model {
            id,
            workspace_id: scope.workspace_id.into(),
            name: body.name.trim().into(),
            credential_id: body.credential_id.into(),
            base_url: body.base_url.trim_end_matches('/').into(),
            protocol: body.protocol,
            currency: body.currency,
            timezone: body.timezone,
            monthly_limit_micros: body.monthly_limit_micros,
            qps: body.qps,
            concurrency: body.concurrency,
            prices: serde_json::to_string(&body.prices).map_err(|_| invalid())?,
            created_at: now,
            created_by: scope.user.id.into(),
            updated_at: now,
            updated_by: scope.user.id.into(),
            version: 1,
        };
        providers::Entity::insert(row.clone().into_active_model())
            .exec(tx)
            .await?;
        crate::hosts::activity_in(
            s,
            tx,
            scope,
            "create",
            "ai-providers",
            &Id::from(id),
            json!({"name":row.name}),
        )
        .await?;
        Ok(row)
    }
}
fn operations() -> [&'static str; 4] {
    [
        "ai.llm.chat",
        "ai.image.generate",
        "ai.audio.speech",
        "ai.audio.transcribe",
    ]
}
pub(super) fn supports(spec: &Value) -> bool {
    operations().contains(&spec["type"].as_str().unwrap_or_default())
}
fn cost(input: i64, output: i64, price: &Price) -> Result<i64> {
    if input < 0 || output < 0 {
        return Err(invalid());
    }
    let units = price.unit_size as i128;
    let numerator =
        input as i128 * price.input_micros as i128 + output as i128 * price.output_micros as i128;
    i64::try_from((numerator + units - 1) / units).map_err(|_| invalid())
}
fn aad(workspace: i64, id: i64) -> String {
    format!("workflow:{workspace}:ai-call:{id}")
}
async fn reserve(
    s: &AppState,
    run: &workflow_executions::Model,
    node: i64,
    index: usize,
    p: &Value,
    operation: &str,
) -> Result<(providers::Model, Price, calls::Model)> {
    let provider = p["provider_id"]
        .as_str()
        .and_then(|x| x.parse::<i64>().ok())
        .ok_or_else(invalid)?;
    let cap = p["max_cost_micros"]
        .as_i64()
        .filter(|n| *n >= 0)
        .ok_or_else(invalid)?;
    let scope = Scope::for_actor(s, Id::from(run.workspace_id), Id::from(run.triggered_by)).await?;
    scope.require("credentials.use")?;
    let tx = crate::variables::transaction(s, Id::from(run.workspace_id)).await?;
    let mut row = providers::Entity::find_by_id(provider)
        .filter(providers::Column::WorkspaceId.eq(run.workspace_id))
        .one(&tx)
        .await?
        .ok_or_else(invalid)?;
    let name = format!("ai:{}:{}", run.workspace_id, row.credential_id);
    let owner = s.next_id()?.to_string();
    let now = db::now();
    if !crate::lock::acquire(&tx, &name, &owner, now, 30_000).await? {
        return Err(error("workflow.aiLimit"));
    }
    let key = format!("{}:{node}:{index}", run.id);
    if let Some(call) = calls::Entity::find()
        .filter(calls::Column::WorkspaceId.eq(run.workspace_id))
        .filter(calls::Column::OperationKey.eq(&key))
        .one(&tx)
        .await?
    {
        let frozen: Value = serde_json::from_str(&call.price_snapshot).map_err(|_| invalid())?;
        let price: Price =
            serde_json::from_value(frozen["price"].clone()).map_err(|_| invalid())?;
        row.base_url = frozen["base_url"].as_str().ok_or_else(invalid)?.into();
        row.protocol = frozen["protocol"].as_str().ok_or_else(invalid)?.into();
        crate::lock::release(&tx, &name, &owner).await?;
        tx.commit().await?;
        return Ok((row, price, call));
    }
    let prices = serde_json::from_str::<Vec<Price>>(&row.prices).map_err(|_| invalid())?;
    // An unpriced connection is unmetered: no cost, no budget. A priced connection
    // still refuses models it has no price for, so metered spend is never uncounted.
    let metered = !prices.is_empty();
    let price: Price = if metered {
        prices
            .into_iter()
            .find(|x| x.model == p["model"] && x.operation == operation)
            .ok_or_else(|| error("workflow.aiPriceMissing"))?
    } else {
        Price {
            model: p["model"].as_str().unwrap_or_default().into(),
            operation: operation.into(),
            input_micros: 0,
            output_micros: 0,
            unit_size: 1,
        }
    };
    let cap = if metered { cap } else { 0 };
    let tz = row
        .timezone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| invalid())?;
    let period = chrono::DateTime::from_timestamp_millis(now)
        .ok_or_else(invalid)?
        .with_timezone(&tz)
        .format("%Y-%m")
        .to_string();
    let all = calls::Entity::find()
        .filter(calls::Column::WorkspaceId.eq(run.workspace_id))
        .filter(calls::Column::CredentialId.eq(row.credential_id))
        .all(&tx)
        .await?;
    let used = all
        .iter()
        .filter(|c| c.currency == row.currency && c.period_key == period)
        .try_fold(0i64, |sum, c| {
            sum.checked_add(if c.state == "succeeded" || c.state == "reconciled" {
                c.actual_micros
            } else {
                c.reserved_micros
            })
        })
        .ok_or_else(invalid)?;
    if (metered
        && used
            .checked_add(cap)
            .is_none_or(|total| total > row.monthly_limit_micros))
        || all
            .iter()
            .filter(|c| matches!(c.state.as_str(), "reserved" | "sent" | "unknown"))
            .count()
            >= row.concurrency as usize
        || all.iter().filter(|c| c.created_at > now - 1000).count() >= row.qps as usize
    {
        return Err(error("workflow.aiLimit"));
    }
    let call = calls::Model {
        id: s.next_id()?.into(),
        workspace_id: run.workspace_id,
        provider_id: row.id,
        credential_id: row.credential_id,
        execution_id: run.id,
        node_run_id: node,
        operation_key: key,
        currency: row.currency.clone(),
        period_key: period,
        state: "reserved".into(),
        reserved_micros: cap,
        actual_micros: 0,
        price_snapshot: json!({"price":price,"base_url":row.base_url,"protocol":row.protocol})
            .to_string(),
        result: None,
        created_at: now,
        created_by: run.triggered_by,
        updated_at: now,
        updated_by: run.triggered_by,
        version: 1,
    };
    calls::Entity::insert(call.clone().into_active_model())
        .exec(&tx)
        .await?;
    crate::lock::release(&tx, &name, &owner).await?;
    tx.commit().await?;
    Ok((row, price, call))
}
async fn state(
    s: &AppState,
    call: &calls::Model,
    status: &str,
    actual: i64,
    result: Option<&Item>,
) -> Result<()> {
    let mut active = call.clone().into_active_model();
    active.state = Set(status.into());
    active.actual_micros = Set(actual);
    active.updated_at = Set(db::now());
    active.version = Set(call.version + 1);
    if let Some(item) = result {
        active.result = Set(Some(s.vault.seal(
            &serde_json::to_string(item).map_err(|_| invalid())?,
            &aad(call.workspace_id, call.id),
        )?));
    }
    active.update(&s.db).await?;
    Ok(())
}
async fn bytes(mut response: reqwest::Response) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| error("workflow.aiUnknown"))?
    {
        if chunk.len() > super::media::MAX_FILE_BYTES.saturating_sub(data.len()) {
            return Err(invalid());
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}
async fn asset(
    s: &AppState,
    storage: &Store<'_>,
    run: &workflow_executions::Model,
    node: &str,
    data: &[u8],
    mime: &str,
    name: &str,
) -> Result<Asset> {
    let file = storage
        .stage_bytes(run.workspace_id, data, mime, run.id, node, run.triggered_by)
        .await?;
    let tx = s.db.begin().await?;
    storage
        .attach(&tx, run.workspace_id, file.id, "workflow", run.workflow_id)
        .await?;
    tx.commit().await?;
    Ok(Asset {
        asset: format!("sha256:{}", file.sha256),
        mime: file.mime,
        size: file.size as u64,
        name: name.into(),
    })
}
pub(super) async fn execute(
    s: &AppState,
    storage: &Store<'_>,
    run: &workflow_executions::Model,
    node: i64,
    spec: &Value,
    items: &[Item],
    params: &[Value],
) -> Result<Vec<Item>> {
    let mut out = Vec::new();
    let operation = spec["type"].as_str().ok_or_else(invalid)?;
    for (index, item) in items.iter().enumerate() {
        let p = params.get(index).ok_or_else(invalid)?;
        let (provider, price, call) = reserve(s, run, node, index, p, operation).await?;
        if call.state == "succeeded" {
            let text = s.vault.open_secret(
                call.result.as_deref().ok_or_else(invalid)?,
                &aad(run.workspace_id, call.id),
            )?;
            out.push(serde_json::from_str(&text).map_err(|_| invalid())?);
            continue;
        }
        if call.state != "reserved" {
            return Err(error("workflow.aiUnknown"));
        }
        let prepared = prepare(s, storage, run, &provider, &price, operation, item, p).await;
        let (request, known) = match prepared {
            Ok(v) => v,
            Err(e) => {
                state(s, &call, "reconciled", 0, None).await?;
                return Err(e);
            }
        };
        if known.is_some_and(|v| v > call.reserved_micros) {
            state(s, &call, "reconciled", 0, None).await?;
            return Err(error("workflow.aiLimit"));
        }
        // Once sent, a crash/timeout never causes an automatic paid duplicate.
        let claimed = calls::Entity::update_many()
            .col_expr(
                calls::Column::State,
                sea_orm::sea_query::Expr::value("sent"),
            )
            .filter(calls::Column::Id.eq(call.id))
            .filter(calls::Column::State.eq("reserved"))
            .exec(&s.db)
            .await?
            .rows_affected;
        if claimed != 1 {
            return Err(error("workflow.aiUnknown"));
        }
        let response = match request.send().await {
            Ok(v) => v,
            Err(_) => {
                state(s, &call, "unknown", 0, None).await?;
                return Err(error("workflow.aiUnknown"));
            }
        };
        if !response.status().is_success() {
            state(s, &call, "unknown", 0, None).await?;
            return Err(error("workflow.aiRemote"));
        }
        let parsed = finish(
            s, storage, run, spec, item, p, operation, response, &price, known,
        )
        .await;
        match parsed {
            Ok((output, actual)) => {
                state(s, &call, "succeeded", actual, Some(&output)).await?;
                out.push(output);
            }
            Err(e) => {
                state(s, &call, "unknown", 0, None).await?;
                return Err(e);
            }
        }
    }
    Ok(out)
}
async fn prepare(
    s: &AppState,
    storage: &Store<'_>,
    run: &workflow_executions::Model,
    provider: &providers::Model,
    price: &Price,
    operation: &str,
    item: &Item,
    p: &Value,
) -> Result<(reqwest::RequestBuilder, Option<i64>)> {
    let scope = Scope::for_actor(s, Id::from(run.workspace_id), Id::from(run.triggered_by)).await?;
    scope.require("credentials.use")?;
    scope.require("workflows.run")?;
    let workflow =
        super::scoped(&s.db, Id::from(run.workspace_id), Id::from(run.workflow_id)).await?;
    crate::definitions::access::require(
        &s.db,
        &scope,
        &workflow,
        crate::definitions::access::Operation::Run,
    )
    .await?;
    let current = workflow_executions::Entity::find_by_id(run.id)
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    if workflow.lifecycle != "active" || current.state != "running" {
        return Err(ApiError::forbidden());
    }
    credential(&s.db, run.workspace_id, provider.credential_id).await?;
    let (_, _, secret) = crate::credentials::unseal(
        s,
        &Id::from(run.workspace_id),
        &Id::from(provider.credential_id),
    )
    .await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| invalid())?;
    let suffix = match operation {
        "ai.llm.chat" if provider.protocol == "anthropic" => "messages",
        "ai.llm.chat" => "chat/completions",
        "ai.image.generate" => "images/generations",
        "ai.audio.speech" => "audio/speech",
        _ => "audio/transcriptions",
    };
    if provider.protocol == "anthropic" && operation != "ai.llm.chat" {
        return Err(invalid());
    }
    let request = client.post(format!("{}/{suffix}", provider.base_url));
    let request = if provider.protocol == "anthropic" {
        request
            .header("x-api-key", secret)
            .header("anthropic-version", "2023-06-01")
    } else {
        request.bearer_auth(secret)
    };
    let model = p["model"].as_str().ok_or_else(invalid)?;
    match operation {
        "ai.llm.chat"=>{
            let mut content=vec![json!({"type":"text","text":p["prompt"]})];
            if let Some(field)=p["image_field"].as_str().filter(|s|!s.is_empty()) {
                let file=item.binary.get(field).ok_or_else(invalid)?;super::media::resolve(s,run.workspace_id,run.workflow_id,file).await?;
                if !matches!(file.mime.as_str(),"image/png"|"image/jpeg"|"image/webp"|"image/gif"){return Err(invalid());}
                let row=super::media::resolve(s,run.workspace_id,run.workflow_id,file).await?;let data=storage.read_bytes(&s.db,run.workspace_id,row.id).await?;
                if provider.protocol=="anthropic" {content.push(json!({"type":"image","source":{"type":"base64","media_type":file.mime,"data":STANDARD.encode(data)}}));}
                else {content.push(json!({"type":"image_url","image_url":{"url":format!("data:{};base64,{}",file.mime,STANDARD.encode(data))}}));}
            }
            Ok((request.json(&json!({"model":model,"messages":[{"role":"user","content":content}],"max_tokens":p["max_tokens"],"stream":false})),None))
        },
        "ai.image.generate"=>Ok((request.json(&json!({"model":model,"prompt":p["prompt"],"n":1,"size":p["size"],"response_format":"b64_json"})),Some(cost(1,0,price)?))),
        "ai.audio.speech"=>Ok((request.json(&json!({"model":model,"input":p["prompt"],"voice":p["voice"],"response_format":"mp3"})),Some(cost(p["prompt"].as_str().ok_or_else(invalid)?.chars().count() as i64,0,price)?))),
        _=>{
            let file=item.binary.get(p["file_field"].as_str().ok_or_else(invalid)?).ok_or_else(invalid)?;
            if !file.mime.starts_with("audio/")&&!file.mime.starts_with("video/"){return Err(invalid());}
            let row=super::media::resolve(s,run.workspace_id,run.workflow_id,file).await?;let data=storage.read_bytes(&s.db,run.workspace_id,row.id).await?;
            let part=reqwest::multipart::Part::bytes(data).file_name(file.name.clone()).mime_str(&file.mime).map_err(|_|invalid())?;
            Ok((request.multipart(reqwest::multipart::Form::new().text("model",model.to_owned()).part("file",part)),Some(cost(1,0,price)?)))
        }
    }
}
async fn finish(
    s: &AppState,
    storage: &Store<'_>,
    run: &workflow_executions::Model,
    spec: &Value,
    item: &Item,
    p: &Value,
    operation: &str,
    response: reqwest::Response,
    price: &Price,
    known: Option<i64>,
) -> Result<(Item, i64)> {
    let data = bytes(response).await?;
    let mut output = item.clone();
    let field = p["output_field"].as_str().ok_or_else(invalid)?;
    let node = spec["id"].as_str().ok_or_else(invalid)?;
    if operation == "ai.audio.speech" {
        let file = asset(s, storage, run, node, &data, "audio/mpeg", "speech.mp3").await?;
        output.binary.insert(field.into(), file);
        return Ok((output, known.ok_or_else(invalid)?));
    }
    let body: Value = serde_json::from_slice(&data).map_err(|_| invalid())?;
    match operation {
        "ai.llm.chat" => {
            let anthropic = body["content"].as_array().map(|parts| {
                parts
                    .iter()
                    .filter(|p| p["type"] == "text")
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            let text = body
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str)
                .or(anthropic.as_deref())
                .ok_or_else(invalid)?;
            let actual = if price.input_micros == 0 && price.output_micros == 0 {
                0
            } else {
                cost(
                    body["usage"]["prompt_tokens"]
                        .as_i64()
                        .or_else(|| body["usage"]["input_tokens"].as_i64())
                        .ok_or_else(|| error("workflow.aiUnknown"))?,
                    body["usage"]["completion_tokens"]
                        .as_i64()
                        .or_else(|| body["usage"]["output_tokens"].as_i64())
                        .ok_or_else(|| error("workflow.aiUnknown"))?,
                    price,
                )?
            };
            output.json.insert(field.into(), json!(text));
            Ok((output, actual))
        }
        "ai.image.generate" => {
            let encoded = body
                .pointer("/data/0/b64_json")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let image = STANDARD.decode(encoded).map_err(|_| invalid())?;
            let mime = if image.starts_with(b"\x89PNG\r\n\x1a\n") {
                "image/png"
            } else if image.starts_with(&[255, 216, 255]) {
                "image/jpeg"
            } else {
                return Err(invalid());
            };
            let file = asset(
                s,
                storage,
                run,
                node,
                &image,
                mime,
                if mime == "image/png" {
                    "generated.png"
                } else {
                    "generated.jpg"
                },
            )
            .await?;
            output.binary.insert(field.into(), file);
            Ok((output, known.ok_or_else(invalid)?))
        }
        _ => {
            output.json.insert(
                field.into(),
                json!(body["text"].as_str().ok_or_else(invalid)?),
            );
            Ok((output, known.ok_or_else(invalid)?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn microcurrency_rounds_up_without_floats() {
        let p = Price {
            model: "m".into(),
            operation: "ai.llm.chat".into(),
            input_micros: 3,
            output_micros: 7,
            unit_size: 1000,
        };
        assert_eq!(cost(1, 1, &p).unwrap(), 1);
        assert_eq!(cost(1000, 1000, &p).unwrap(), 10);
        assert!(cost(-1, 0, &p).is_err());
    }
}
