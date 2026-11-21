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
use sha2::{Digest, Sha256};

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
        json!({"items":rows.iter().map(call_view).collect::<Vec<_>>()}),
    ))
}
fn observation_flag(call: &calls::Model, name: &str) -> bool {
    serde_json::from_str::<Value>(&call.price_snapshot)
        .ok()
        .and_then(|snapshot| snapshot[name].as_bool())
        .unwrap_or(true)
}
fn call_view(call: &calls::Model) -> Value {
    let cost_known = observation_flag(call, "cost_known");
    json!({"id":Id::from(call.id),"provider_id":Id::from(call.provider_id),"execution_id":Id::from(call.execution_id),
        "source_type":if call.conversation_id == 0 { "workflow" } else { "conversation" },
        "conversation_id":Id::from(call.conversation_id),"conversation_turn_id":Id::from(call.conversation_turn_id),
        "state":call.state,"currency":call.currency,"period_key":call.period_key,"reserved_micros":call.reserved_micros,
        "actual_micros":cost_known.then_some(call.actual_micros),"cost_known":cost_known,
        "price_known":observation_flag(call,"price_known"),"created_at":call.created_at})
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
    let mut snapshot: Value = serde_json::from_str(&row.price_snapshot).map_err(|_| invalid())?;
    snapshot["cost_known"] = json!(true);
    let mut active = row.into_active_model();
    active.price_snapshot = Set(snapshot.to_string());
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
    /// Prices are optional and used for reporting, never for financial admission.
    #[serde(default = "default_currency")]
    currency: String,
    #[serde(default = "default_timezone")]
    timezone: String,
    #[serde(default)]
    /// Deprecated compatibility field; retained for round trips, not enforced.
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
    if input < 0
        || output < 0
        || price.unit_size <= 0
        || price.input_micros < 0
        || price.output_micros < 0
    {
        return Err(invalid());
    }
    let units = price.unit_size as i128;
    let numerator =
        input as i128 * price.input_micros as i128 + output as i128 * price.output_micros as i128;
    i64::try_from((numerator + units - 1) / units).map_err(|_| invalid())
}
/// Pricing is an observation; missing/partial usage and overflow are not an
/// execution failure and must never discard a known provider response.
fn observed_cost(body: &Value, price: &Price) -> Option<i64> {
    if price.input_micros == 0 && price.output_micros == 0 {
        return Some(0);
    }
    cost(
        body["usage"]["prompt_tokens"]
            .as_i64()
            .or_else(|| body["usage"]["input_tokens"].as_i64())?,
        body["usage"]["completion_tokens"]
            .as_i64()
            .or_else(|| body["usage"]["output_tokens"].as_i64())?,
        price,
    )
    .ok()
}
fn aad(call: &calls::Model) -> String {
    if call.conversation_id == 0 {
        format!("workflow:{}:ai-call:{}", call.workspace_id, call.id)
    } else {
        format!(
            "conversation:{}:{}:{}:{}:ai-call:{}",
            call.workspace_id,
            call.conversation_id,
            call.conversation_turn_id,
            call.operation_key,
            call.id
        )
    }
}
/// Actual caller provenance, independent of the workflow execution model.
struct CallOrigin {
    workspace_id: i64,
    actor_id: i64,
    execution_id: i64,
    node_run_id: i64,
    conversation_id: i64,
    conversation_turn_id: i64,
    key: String,
}
impl CallOrigin {
    fn workflow(run: &workflow_executions::Model, node: i64, index: usize) -> Self {
        Self {
            workspace_id: run.workspace_id,
            actor_id: run.triggered_by,
            execution_id: run.id,
            node_run_id: node,
            conversation_id: 0,
            conversation_turn_id: 0,
            key: format!("{}:{node}:{index}", run.id),
        }
    }
    fn conversation(turn: &crate::entities::ai_turns::Model, agent: i64, index: usize) -> Self {
        Self {
            workspace_id: turn.workspace_id,
            actor_id: turn.owner_id,
            execution_id: 0,
            node_run_id: 0,
            conversation_id: turn.conversation_id,
            conversation_turn_id: turn.id,
            key: format!("chat:{}:{agent}:{}:{index}", turn.conversation_id, turn.id),
        }
    }
}
fn agent_request_hash(params: &Value) -> Option<String> {
    params.get("agent_messages").map(|_| {
        // Internal Agent parameters are deterministic JSON, including all
        // preceding tool results. Never replay a round for a different input.
        hex::encode(Sha256::digest(params.to_string().as_bytes()))
    })
}
async fn reserve(
    s: &AppState,
    origin: &CallOrigin,
    p: &Value,
    operation: &str,
) -> Result<(providers::Model, Price, calls::Model)> {
    let provider = p["provider_id"]
        .as_str()
        .and_then(|x| x.parse::<i64>().ok())
        .ok_or_else(invalid)?;
    let scope =
        Scope::for_actor(s, Id::from(origin.workspace_id), Id::from(origin.actor_id)).await?;
    scope.require("credentials.use")?;
    let tx = crate::variables::transaction(s, Id::from(origin.workspace_id)).await?;
    let mut row = providers::Entity::find_by_id(provider)
        .filter(providers::Column::WorkspaceId.eq(origin.workspace_id))
        .one(&tx)
        .await?
        .ok_or_else(invalid)?;
    let name = format!("ai:{}:{}", origin.workspace_id, row.credential_id);
    let owner = s.next_id()?.to_string();
    let now = db::now();
    if !crate::lock::acquire(&tx, &name, &owner, now, 30_000).await? {
        return Err(error("workflow.aiLimit"));
    }
    let key = &origin.key;
    if let Some(call) = calls::Entity::find()
        .filter(calls::Column::WorkspaceId.eq(origin.workspace_id))
        .filter(calls::Column::OperationKey.eq(key.clone()))
        .one(&tx)
        .await?
    {
        let frozen: Value = serde_json::from_str(&call.price_snapshot).map_err(|_| invalid())?;
        if let Some(hash) = agent_request_hash(p) {
            if frozen["request_hash"] != hash {
                return Err(error("workflow.aiUnknown"));
            }
        }
        let price: Price =
            serde_json::from_value(frozen["price"].clone()).map_err(|_| invalid())?;
        row.base_url = frozen["base_url"].as_str().ok_or_else(invalid)?.into();
        row.protocol = frozen["protocol"].as_str().ok_or_else(invalid)?.into();
        crate::lock::release(&tx, &name, &owner).await?;
        tx.commit().await?;
        return Ok((row, price, call));
    }
    let prices = serde_json::from_str::<Vec<Price>>(&row.prices).map_err(|_| invalid())?;
    // Pricing is observational. Missing model prices must not block execution.
    let configured_price = prices
        .into_iter()
        .find(|price| price.model == p["model"] && price.operation == operation);
    let price_known = configured_price.is_some();
    let price = configured_price.unwrap_or(Price {
        model: p["model"].as_str().unwrap_or_default().into(),
        operation: operation.into(),
        input_micros: 0,
        output_micros: 0,
        unit_size: 1,
    });
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
        .filter(calls::Column::WorkspaceId.eq(origin.workspace_id))
        .filter(calls::Column::CredentialId.eq(row.credential_id))
        .all(&tx)
        .await?;
    if all
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
        workspace_id: origin.workspace_id,
        provider_id: row.id,
        credential_id: row.credential_id,
        execution_id: origin.execution_id,
        node_run_id: origin.node_run_id,
        conversation_id: origin.conversation_id,
        conversation_turn_id: origin.conversation_turn_id,
        operation_key: key.clone(),
        currency: row.currency.clone(),
        period_key: period,
        state: "reserved".into(),
        reserved_micros: 0,
        actual_micros: 0,
        price_snapshot: json!({"price":price,"base_url":row.base_url,"protocol":row.protocol,"request_hash":agent_request_hash(p),"price_known":price_known,"cost_known":false})
            .to_string(),
        result: None,
        created_at: now,
        created_by: origin.actor_id,
        updated_at: now,
        updated_by: origin.actor_id,
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
    actual: Option<i64>,
    result: Option<&Item>,
) -> Result<()> {
    let mut active = call.clone().into_active_model();
    active.state = Set(status.into());
    // The schema's numeric zero is storage only when cost_known is false.
    active.actual_micros = Set(actual.unwrap_or(0));
    let mut snapshot: Value = serde_json::from_str(&call.price_snapshot).map_err(|_| invalid())?;
    snapshot["cost_known"] = json!(actual.is_some());
    active.price_snapshot = Set(snapshot.to_string());
    active.updated_at = Set(db::now());
    active.version = Set(call.version + 1);
    if let Some(item) = result {
        active.result = Set(Some(s.vault.seal(
            &serde_json::to_string(item).map_err(|_| invalid())?,
            &aad(call),
        )?));
    }
    active.update(&s.db).await?;
    Ok(())
}

/// A dropped API Agent request no longer has a future which can record its
/// outcome. Release unsent reservations and make sent calls reconcilable;
/// neither state permits replay of that operation key.
pub(super) async fn interrupted(
    s: &AppState,
    run: &workflow_executions::Model,
    node: i64,
    index: usize,
) -> Result<()> {
    let call = calls::Entity::find()
        .filter(calls::Column::WorkspaceId.eq(run.workspace_id))
        .filter(calls::Column::OperationKey.eq(format!("{}:{node}:{index}", run.id)))
        .one(&s.db)
        .await?;
    if let Some(call) = call.filter(|call| matches!(call.state.as_str(), "reserved" | "sent")) {
        calls::Entity::update_many()
            .col_expr(
                calls::Column::State,
                sea_orm::sea_query::Expr::value(if call.state == "sent" {
                    "unknown"
                } else {
                    "reconciled"
                }),
            )
            .col_expr(
                calls::Column::Version,
                sea_orm::sea_query::Expr::value(call.version + 1),
            )
            .col_expr(
                calls::Column::UpdatedAt,
                sea_orm::sea_query::Expr::value(db::now()),
            )
            .col_expr(
                calls::Column::UpdatedBy,
                sea_orm::sea_query::Expr::value(run.triggered_by),
            )
            .filter(calls::Column::Id.eq(call.id))
            .filter(calls::Column::Version.eq(call.version))
            .filter(calls::Column::State.eq(call.state))
            .exec(&s.db)
            .await?;
    }
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
    execute_indexed(s, storage, run, node, spec, items, params, 0).await
}

/// API Agent rounds have distinct journal keys while sharing admission,
/// credentials, metering, encrypted results and uncertain-outcome handling.
#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_indexed(
    s: &AppState,
    storage: &Store<'_>,
    run: &workflow_executions::Model,
    node: i64,
    spec: &Value,
    items: &[Item],
    params: &[Value],
    offset: usize,
) -> Result<Vec<Item>> {
    let mut out = Vec::new();
    let operation = spec["type"].as_str().ok_or_else(invalid)?;
    for (index, item) in items.iter().enumerate() {
        let p = params.get(index).ok_or_else(invalid)?;
        let (provider, price, call) = reserve(
            s,
            &CallOrigin::workflow(run, node, offset + index),
            p,
            operation,
        )
        .await?;
        if call.state == "succeeded" {
            let text = s
                .vault
                .open_secret(call.result.as_deref().ok_or_else(invalid)?, &aad(&call))?;
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
                state(s, &call, "reconciled", Some(0), None).await?;
                return Err(e);
            }
        };
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
        let host = crate::upstream::host(&provider.base_url);
        let response = match request.send().await {
            Ok(v) => v,
            Err(e) => {
                state(s, &call, "unknown", None, None).await?;
                // Never reached: nothing was sent. Otherwise the outcome is unknown.
                return Err(crate::upstream::unreachable("ai", &host, &e)
                    .unwrap_or_else(|| error("workflow.aiUnknown")));
            }
        };
        if !response.status().is_success() {
            state(s, &call, "unknown", None, None).await?;
            return Err(crate::upstream::status(
                "ai",
                &host,
                response.status().as_u16(),
                true,
            ));
        }
        let parsed = finish(
            s, storage, run, spec, item, p, operation, response, &price, known,
        )
        .await;
        match parsed {
            Ok((output, actual)) => {
                state(
                    s,
                    &call,
                    "succeeded",
                    actual.filter(|_| observation_flag(&call, "price_known")),
                    Some(&output),
                )
                .await?;
                out.push(output);
            }
            Err(e) => {
                state(s, &call, "unknown", None, None).await?;
                return Err(e);
            }
        }
    }
    Ok(out)
}
/// Application chat uses the same admission, credential transport and encrypted
/// journal as workflows, with real non-workflow provenance and live turn authority.
pub(crate) async fn conversation_round(
    s: &AppState,
    turn: &crate::entities::ai_turns::Model,
    conversation: &crate::entities::ai_conversations::Model,
    owner: &str,
    round: usize,
    params: &Value,
) -> Result<Value> {
    let scope = crate::conversations::run::api_authorized(s, turn, owner).await?;
    scope.require("credentials.use")?;
    let origin = CallOrigin::conversation(turn, conversation.agent_id, round);
    let (provider, price, call) = reserve(s, &origin, params, "ai.llm.chat").await?;
    if call.state == "succeeded" {
        let plain = s
            .vault
            .open_secret(call.result.as_deref().ok_or_else(invalid)?, &aad(&call))?;
        let item: Item = serde_json::from_str(&plain).map_err(|_| invalid())?;
        return item.json.get("response").cloned().ok_or_else(invalid);
    }
    if call.state != "reserved" {
        return Err(error("workflow.aiUnknown"));
    }
    let prepared = async {
        if provider.protocol == "anthropic" && !params["max_tokens"].as_u64().is_some_and(|n| n > 0)
        {
            return Err(crate::conversations::error(
                StatusCode::BAD_REQUEST,
                "conversations.apiOutputRequired",
            ));
        }
        let mut body = json!({"model":params["model"], "messages":params["agent_messages"],
            "tools":params["agent_tools"], "stream":false});
        if !Value::is_null(&params["max_tokens"]) {
            body["max_tokens"] = params["max_tokens"].clone();
        }
        if !Value::is_null(&params["system"]) {
            body["system"] = params["system"].clone();
        }
        let request = provider_request(s, turn.workspace_id, &provider, "ai.llm.chat")
            .await?
            .json(&body);
        crate::conversations::run::api_authorized(s, turn, owner)
            .await?
            .require("credentials.use")?;
        Ok(request)
    }
    .await;
    let request = match prepared {
        Ok(request) => request,
        Err(error) => {
            state(s, &call, "reconciled", Some(0), None).await?;
            return Err(error);
        }
    };
    if calls::Entity::update_many()
        .col_expr(
            calls::Column::State,
            sea_orm::sea_query::Expr::value("sent"),
        )
        .filter(calls::Column::Id.eq(call.id))
        .filter(calls::Column::State.eq("reserved"))
        .exec(&s.db)
        .await?
        .rows_affected
        != 1
    {
        return Err(error("workflow.aiUnknown"));
    }
    let result = async {
        let response = request
            .send()
            .await
            .map_err(|_| error("workflow.aiUnknown"))?;
        if !response.status().is_success() {
            return Err(crate::upstream::status(
                "ai",
                &crate::upstream::host(&provider.base_url),
                response.status().as_u16(),
                true,
            ));
        }
        let data = bytes(response).await?;
        let body: Value = serde_json::from_slice(&data).map_err(|_| invalid())?;
        let actual = observed_cost(&body, &price);
        Ok((body, actual))
    }
    .await;
    match result {
        Ok((body, actual)) => {
            let item = Item {
                json: serde_json::Map::from_iter([("response".into(), body.clone())]),
                binary: Default::default(),
                paired: vec![],
            };
            state(
                s,
                &call,
                "succeeded",
                actual.filter(|_| observation_flag(&call, "price_known")),
                Some(&item),
            )
            .await?;
            Ok(body)
        }
        Err(error) => {
            state(s, &call, "unknown", None, None).await?;
            Err(error)
        }
    }
}
/// Recover a paid result already recorded before a worker lost its lease.
/// This is a local journal read and never contacts the provider.
pub(crate) enum ConversationOutcome {
    Known(Value),
    Unsent,
    Unknown,
}
pub(crate) async fn conversation_outcome(
    s: &AppState,
    turn: &crate::entities::ai_turns::Model,
    agent: i64,
    round: usize,
) -> Result<ConversationOutcome> {
    let origin = CallOrigin::conversation(turn, agent, round);
    let call = calls::Entity::find()
        .filter(calls::Column::WorkspaceId.eq(turn.workspace_id))
        .filter(calls::Column::ConversationId.eq(turn.conversation_id))
        .filter(calls::Column::ConversationTurnId.eq(turn.id))
        .filter(calls::Column::OperationKey.eq(origin.key))
        .one(&s.db)
        .await?;
    let Some(call) = call else {
        return Ok(ConversationOutcome::Unsent);
    };
    if call.state == "reserved" {
        return Ok(ConversationOutcome::Unsent);
    }
    // Reconciled can also mean a human settled an uncertain sent call; it does
    // not prove that a request was never sent.
    if call.state != "succeeded" {
        return Ok(ConversationOutcome::Unknown);
    }
    let plain = s
        .vault
        .open_secret(call.result.as_deref().ok_or_else(invalid)?, &aad(&call))?;
    let item: Item = serde_json::from_str(&plain).map_err(|_| invalid())?;
    Ok(ConversationOutcome::Known(
        item.json.get("response").cloned().ok_or_else(invalid)?,
    ))
}
/// Cancellation/crash may drop a sent future. Mark it unknown without replay.
pub(crate) async fn conversation_interrupted(
    s: &AppState,
    turn: &crate::entities::ai_turns::Model,
) -> Result<()> {
    conversation_interrupted_in(&s.db, turn).await
}
pub(crate) async fn conversation_interrupted_in(
    conn: &impl ConnectionTrait,
    turn: &crate::entities::ai_turns::Model,
) -> Result<()> {
    for (previous, next) in [("reserved", "reconciled"), ("sent", "unknown")] {
        calls::Entity::update_many()
            .col_expr(calls::Column::State, sea_orm::sea_query::Expr::value(next))
            .col_expr(
                calls::Column::UpdatedAt,
                sea_orm::sea_query::Expr::value(db::now()),
            )
            .col_expr(
                calls::Column::UpdatedBy,
                sea_orm::sea_query::Expr::value(turn.owner_id),
            )
            .col_expr(
                calls::Column::Version,
                sea_orm::sea_query::Expr::col(calls::Column::Version).add(1),
            )
            .filter(calls::Column::WorkspaceId.eq(turn.workspace_id))
            .filter(calls::Column::ConversationId.eq(turn.conversation_id))
            .filter(calls::Column::ConversationTurnId.eq(turn.id))
            .filter(calls::Column::State.eq(previous))
            .exec(conn)
            .await?;
    }
    Ok(())
}

async fn provider_request(
    s: &AppState,
    workspace: i64,
    provider: &providers::Model,
    operation: &str,
) -> Result<reqwest::RequestBuilder> {
    credential(&s.db, workspace, provider.credential_id).await?;
    let (_, _, secret) =
        crate::credentials::unseal(s, &Id::from(workspace), &Id::from(provider.credential_id))
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
    Ok(request)
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
    let request = provider_request(s, run.workspace_id, provider, operation).await?;
    let model = p["model"].as_str().ok_or_else(invalid)?;
    match operation {
        "ai.llm.chat"=>{
            if let Some(messages) = p.get("agent_messages") {
                let body = json!({"model":model,"messages":messages,"max_tokens":p["max_tokens"],"stream":false,"tools":p["agent_tools"]});
                return Ok((request.json(&body), None));
            }
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
        "ai.image.generate"=>Ok((request.json(&json!({"model":model,"prompt":p["prompt"],"n":1,"size":p["size"],"response_format":"b64_json"})),cost(1,0,price).ok())),
        "ai.audio.speech"=>Ok((request.json(&json!({"model":model,"input":p["prompt"],"voice":p["voice"],"response_format":"mp3"})),cost(p["prompt"].as_str().ok_or_else(invalid)?.chars().count() as i64,0,price).ok())),
        _=>{
            let file=item.binary.get(p["file_field"].as_str().ok_or_else(invalid)?).ok_or_else(invalid)?;
            if !file.mime.starts_with("audio/")&&!file.mime.starts_with("video/"){return Err(invalid());}
            let row=super::media::resolve(s,run.workspace_id,run.workflow_id,file).await?;let data=storage.read_bytes(&s.db,run.workspace_id,row.id).await?;
            let part=reqwest::multipart::Part::bytes(data).file_name(file.name.clone()).mime_str(&file.mime).map_err(|_|invalid())?;
            Ok((request.multipart(reqwest::multipart::Form::new().text("model",model.to_owned()).part("file",part)),cost(1,0,price).ok()))
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
) -> Result<(Item, Option<i64>)> {
    let data = bytes(response).await?;
    let mut output = item.clone();
    let field = p["output_field"].as_str().ok_or_else(invalid)?;
    let node = spec["id"].as_str().ok_or_else(invalid)?;
    if operation == "ai.audio.speech" {
        let file = asset(s, storage, run, node, &data, "audio/mpeg", "speech.mp3").await?;
        output.binary.insert(field.into(), file);
        return Ok((output, known));
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
                .or_else(|| p.get("agent_messages").map(|_| ""))
                .ok_or_else(invalid)?;
            let actual = observed_cost(&body, price);
            output.json.insert(
                field.into(),
                if p.get("agent_messages").is_some() {
                    body
                } else {
                    json!(text)
                },
            );
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
            Ok((output, known))
        }
        _ => {
            output.json.insert(
                field.into(),
                json!(body["text"].as_str().ok_or_else(invalid)?),
            );
            Ok((output, known))
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
    #[tokio::test]
    async fn normal_provider_operations_preserve_auth_assets_prices_and_cached_responses() {
        use crate::agents::tests::admin;
        use axum::response::IntoResponse;
        use std::sync::{Arc, Mutex};
        let (_dir, s) = crate::platform_tools::tests::setup().await;
        let png=STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jB6kAAAAASUVORK5CYII=").unwrap();
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let (captured, picture) = (seen.clone(), png.clone());
        let handler = move |uri: axum::http::Uri,
                            headers: axum::http::HeaderMap,
                            body: axum::body::Bytes| {
            let (captured, picture) = (captured.clone(), picture.clone());
            async move {
                captured.lock().unwrap().push(uri.path().to_owned());
                let value: Value = if uri.path() == "/audio/transcriptions" {
                    Value::Null
                } else {
                    serde_json::from_slice(&body).unwrap()
                };
                if uri.path() == "/messages" {
                    assert_eq!(headers["x-api-key"], "ordinary-secret");
                    assert_eq!(headers["anthropic-version"], "2023-06-01");
                    assert!(headers.get("authorization").is_none());
                    assert_eq!(
                        value["messages"][0]["content"][1]["source"]["media_type"],
                        "image/png"
                    );
                    return axum::Json(json!({"content":[{"type":"text","text":"anthropic answer"}],"usage":{"input_tokens":3,"output_tokens":2}})).into_response();
                }
                assert_eq!(headers["authorization"], "Bearer ordinary-secret");
                match uri.path() {
                    "/chat/completions" => {
                        assert_eq!(value["max_tokens"], 64);
                        assert!(
                            value["messages"][0]["content"][1]["image_url"]["url"]
                                .as_str()
                                .unwrap()
                                .starts_with("data:image/png;base64,")
                        );
                        axum::Json(json!({"choices":[{"message":{"role":"assistant","content":"plain answer"}}],"usage":{"prompt_tokens":3,"completion_tokens":2}})).into_response()
                    }
                    "/images/generations" => {
                        axum::Json(json!({"data":[{"b64_json":STANDARD.encode(picture)}]}))
                            .into_response()
                    }
                    "/audio/speech" => (
                        [("content-type", "audio/mpeg")],
                        b"ID3-known-audio".to_vec(),
                    )
                        .into_response(),
                    "/audio/transcriptions" => {
                        assert!(
                            headers["content-type"]
                                .to_str()
                                .unwrap()
                                .starts_with("multipart/form-data;")
                        );
                        assert!(String::from_utf8_lossy(&body).contains("RIFF"));
                        axum::Json(json!({"text":"transcribed audio"})).into_response()
                    }
                    _ => panic!("unexpected provider path"),
                }
            }
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new()
                    .route("/chat/completions", axum::routing::post(handler.clone()))
                    .route("/images/generations", axum::routing::post(handler.clone()))
                    .route("/audio/speech", axum::routing::post(handler.clone()))
                    .route(
                        "/audio/transcriptions",
                        axum::routing::post(handler.clone()),
                    )
                    .route("/messages", axum::routing::post(handler)),
            )
            .await
            .unwrap();
        });
        let prices = vec![
            ("ai.llm.chat", 2, 3),
            ("ai.image.generate", 7, 0),
            ("ai.audio.speech", 2, 0),
            ("ai.audio.transcribe", 13, 0),
        ];
        let model=crate::platform_tools::dispatch(&s,admin(1),crate::platform_tools::Call {method:"POST".into(),path:"/ai-models".into(),
            body:json!({"name":"Normal operator model","kind":"api","skip_check":true,"state":"enabled","default_model":"m",
                "connection":{"name":"Normal operator provider","api_key":"ordinary-secret","protocol":"openai","base_url":url,"qps":1000,"concurrency":10,"monthly_limit_micros":0,
                    "prices":prices.iter().map(|(operation,input,output)|json!({"model":"m","operation":operation,"input_micros":input,"output_micros":output,"unit_size":1})).collect::<Vec<_>>()}}),..Default::default()}).await.unwrap();
        assert_eq!(model["status"], 201, "{model}");
        let provider = model["data"]["provider_id"]
            .as_str()
            .unwrap()
            .parse::<i64>()
            .unwrap();
        let graph = json!({"engine":"ferry-graph","schema":1,"settings":{"timezone":"UTC"},"nodes":[{"id":"start","name":"Start","type":"core.trigger.manual","type_version":1,"params":{"input_schema":{"type":"object"}},"position":[0,0]}],"connections":[]});
        let workflow = crate::platform_tools::dispatch(
            &s,
            admin(1),
            crate::platform_tools::Call {
                method: "POST".into(),
                path: "/workflows".into(),
                body: json!({"name":"Normal AI operations","definition":graph.to_string()}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(workflow["status"], 201, "{workflow}");
        let workflow: Id = serde_json::from_value(workflow["data"]["id"].clone()).unwrap();
        let (_, Json(created)) = super::super::run::create(
            State(s.clone()),
            admin(1),
            Path(("1".into(), workflow)),
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
        workflow_executions::Entity::update_many()
            .col_expr(
                workflow_executions::Column::State,
                sea_orm::sea_query::Expr::value("running"),
            )
            .filter(workflow_executions::Column::Id.eq(execution))
            .exec(&s.db)
            .await
            .unwrap();
        let run = workflow_executions::Entity::find_by_id(execution)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let backend = crate::artifact_store::ArtifactStore::for_state(&s)
            .await
            .unwrap();
        let storage = Store {
            db: &s.db,
            backend: &backend,
            vault: &s.vault,
            ids: &s.ids,
            quota: crate::content_assets::DEFAULT_QUOTA,
        };
        let image = storage
            .stage_bytes(1, &png, "image/png", run.id, "picture", 1)
            .await
            .unwrap();
        let audio = storage
            .stage_bytes(1, b"RIFF-test-wave", "audio/wav", run.id, "audio", 1)
            .await
            .unwrap();
        let tx = s.db.begin().await.unwrap();
        storage
            .attach(&tx, 1, image.id, "workflow", i64::from(workflow))
            .await
            .unwrap();
        storage
            .attach(&tx, 1, audio.id, "workflow", i64::from(workflow))
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let input = Item {
            json: Default::default(),
            binary: std::collections::BTreeMap::from([
                (
                    "picture".into(),
                    Asset {
                        asset: format!("sha256:{}", image.sha256),
                        mime: "image/png".into(),
                        size: png.len() as u64,
                        name: "picture.png".into(),
                    },
                ),
                (
                    "audio".into(),
                    Asset {
                        asset: format!("sha256:{}", audio.sha256),
                        mime: "audio/wav".into(),
                        size: 14,
                        name: "voice.wav".into(),
                    },
                ),
            ]),
            paired: vec![],
        };
        for (node, operation, field) in [
            (1, "ai.llm.chat", "chat"),
            (2, "ai.image.generate", "image"),
            (3, "ai.audio.speech", "speech"),
            (4, "ai.audio.transcribe", "transcript"),
        ] {
            let p = json!({"provider_id":provider.to_string(),"model":"m","prompt":"hello","max_tokens":64,"image_field":"picture","file_field":"audio","voice":"alloy","size":"1024x1024","output_field":field});
            let output = execute(
                &s,
                &storage,
                &run,
                node,
                &json!({"id":format!("plain-{node}"),"type":operation}),
                std::slice::from_ref(&input),
                &[p],
            )
            .await
            .unwrap();
            match operation {
                "ai.llm.chat" => assert_eq!(output[0].json["chat"], "plain answer"),
                "ai.audio.transcribe" => {
                    assert_eq!(output[0].json["transcript"], "transcribed audio")
                }
                _ => {
                    let asset = super::super::media::resolve(
                        &s,
                        1,
                        i64::from(workflow),
                        &output[0].binary[field],
                    )
                    .await
                    .unwrap();
                    let bytes = storage.read_bytes(&s.db, 1, asset.id).await.unwrap();
                    assert_eq!(
                        bytes,
                        if field == "image" {
                            png.clone()
                        } else {
                            b"ID3-known-audio".to_vec()
                        }
                    );
                }
            }
        }
        providers::Entity::update_many()
            .col_expr(
                providers::Column::Protocol,
                sea_orm::sea_query::Expr::value("anthropic"),
            )
            .filter(providers::Column::Id.eq(provider))
            .exec(&s.db)
            .await
            .unwrap();
        let p = json!({"provider_id":provider.to_string(),"model":"m","prompt":"hello","max_tokens":64,"image_field":"picture","output_field":"chat"});
        let output = execute(
            &s,
            &storage,
            &run,
            5,
            &json!({"id":"anthropic","type":"ai.llm.chat"}),
            std::slice::from_ref(&input),
            std::slice::from_ref(&p),
        )
        .await
        .unwrap();
        assert_eq!(output[0].json["chat"], "anthropic answer");
        let cached = execute(
            &s,
            &storage,
            &run,
            1,
            &json!({"id":"plain-1","type":"ai.llm.chat"}),
            std::slice::from_ref(&input),
            &[p],
        )
        .await
        .unwrap();
        assert_eq!(cached[0].json["chat"], "plain answer");
        assert_eq!(
            seen.lock().unwrap().len(),
            5,
            "known workflow journal result never sends again"
        );
        let calls = calls::Entity::find().all(&s.db).await.unwrap();
        assert_eq!(calls.len(), 5);
        for (node, expected) in [(1, 12), (2, 7), (3, 10), (4, 13), (5, 12)] {
            let call = calls.iter().find(|call| call.node_run_id == node).unwrap();
            assert_eq!(call.state, "succeeded");
            assert_eq!(call.actual_micros, expected);
            assert!(observation_flag(call, "cost_known") && observation_flag(call, "price_known"));
            assert_eq!(call.execution_id, run.id);
            assert_eq!(call.conversation_id, 0);
            assert_eq!(call.conversation_turn_id, 0);
            assert!(
                s.vault
                    .open_secret(
                        call.result.as_deref().unwrap(),
                        &format!("workflow:1:ai-call:{}", call.id)
                    )
                    .is_ok(),
                "workflow AAD remains byte-compatible"
            );
        }
        server.abort();
    }
}

pub(crate) fn agent_tools(anthropic: bool, conversational: bool) -> Value {
    Value::Array(ferry_protocol::agent::platform_tools().as_array().unwrap().iter()
        .filter(|tool| conversational || matches!(tool["name"].as_str(), Some("ferry_discover" | "ferry_request")))
        .map(|tool| if anthropic {
            json!({"name":tool["name"],"description":tool["description"],"input_schema":tool["inputSchema"]})
        } else {
            json!({"type":"function","function":{"name":tool["name"],"description":tool["description"],"parameters":tool["inputSchema"]}})
        }).collect())
}

/// Validate the wire response before feeding it back to the provider. Tool
/// arguments remain untrusted and pass through platform_call's normal checks.
pub(crate) fn agent_response(
    body: &Value,
    anthropic: bool,
) -> Result<(Value, String, Vec<(String, String, Value)>)> {
    let malformed = || error("workflow.aiInvalid");
    let mut calls: Vec<(String, String, Value)> = Vec::new();
    let (message, text) = if anthropic {
        let content = body["content"].as_array().ok_or_else(malformed)?;
        for block in content.iter().filter(|block| block["type"] == "tool_use") {
            calls.push((
                block["id"].as_str().ok_or_else(malformed)?.into(),
                block["name"].as_str().ok_or_else(malformed)?.into(),
                block["input"].clone(),
            ));
        }
        (
            json!({"role":"assistant","content":content}),
            content
                .iter()
                .filter(|block| block["type"] == "text")
                .filter_map(|block| block["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        )
    } else {
        let message = body
            .pointer("/choices/0/message")
            .filter(|value| value.is_object())
            .ok_or_else(malformed)?;
        if message["role"] != "assistant" {
            return Err(malformed());
        }
        if let Some(raw) = message.get("tool_calls").filter(|raw| !raw.is_null()) {
            for call in raw.as_array().ok_or_else(malformed)? {
                if call["type"] != "function" {
                    return Err(malformed());
                }
                calls.push((
                    call["id"].as_str().ok_or_else(malformed)?.into(),
                    call["function"]["name"]
                        .as_str()
                        .ok_or_else(malformed)?
                        .into(),
                    serde_json::from_str(
                        call["function"]["arguments"]
                            .as_str()
                            .ok_or_else(malformed)?,
                    )
                    .map_err(|_| malformed())?,
                ));
            }
        }
        (
            message.clone(),
            message["content"].as_str().unwrap_or_default().into(),
        )
    };
    let mut ids = std::collections::BTreeSet::new();
    if calls
        .iter()
        .any(|(id, _, _)| id.is_empty() || id.len() > 128 || !ids.insert(id.clone()))
    {
        return Err(malformed());
    }
    Ok((message, text, calls))
}
