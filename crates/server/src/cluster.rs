//! Optional inter-server transport. JetStream stores expiring ownership only;
//! live streams use acknowledged Core NATS messages and are never replayed.
use crate::{
    error::{ApiError, Result},
    snowflake::Id,
    state::AppState,
};
use async_nats::jetstream::{
    self,
    kv::{Operation, Store},
};
use axum::{
    extract::ws::{Message, WebSocket},
    http::StatusCode,
};
use bytes::Bytes;
use futures::StreamExt;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
use tracing::Instrument;

const TTL: Duration = Duration::from_secs(30);
const IO_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_FRAME: usize = 256 * 1024;
const MAX_MESSAGE: usize = 16 * 1024 * 1024;

pub fn unavailable() -> ApiError {
    ApiError::localized(
        StatusCode::SERVICE_UNAVAILABLE,
        crate::i18n::Message::new("runner.clusterUnavailable"),
    )
}
fn failed(_: impl std::fmt::Display) -> ApiError {
    diagnostic("nats", "coordination", "operation_failed");
    unavailable()
}

/// Intentionally accept only static classifications: transport errors can carry
/// URLs, payloads and bearer session identifiers in their Display/Debug output.
pub(crate) fn diagnostic(transport: &'static str, phase: &'static str, reason: &'static str) {
    tracing::warn!(
        transport,
        phase,
        reason,
        "Runner transport operation failed"
    );
}

fn local_failed(phase: Option<&'static str>, _: axum::Error) -> ApiError {
    if let Some(phase) = phase {
        diagnostic("websocket", phase, "websocket_error");
    }
    ApiError::localized(
        StatusCode::BAD_GATEWAY,
        crate::i18n::Message::new("runner.transportFailed"),
    )
}

#[derive(Clone)]
pub struct Cluster {
    pub client: async_nats::Client,
    owners: Store,
    sessions: Store,
    pub coordination: Store,
    prefix: String,
    pub node: String,
    db: sea_orm::DatabaseConnection,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Owner {
    pub node: String,
    pub fence: String,
    pub workspace: Id,
    #[serde(default)]
    pub info: Option<RunnerInfo>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct RunnerInfo {
    pub managed: bool,
    pub ready: bool,
    pub version: String,
    pub failed_version: Option<String>,
}
#[derive(Serialize, Deserialize)]
pub struct SessionRoute {
    pub host: String,
    pub owner: Owner,
}

#[derive(Serialize, Deserialize)]
pub enum Request {
    Open {
        host: String,
        fence: String,
        session: String,
        kind: ferry_protocol::SessionKind,
    },
    Monitor {
        workspace: Id,
        host: Id,
        query: crate::host_monitor::MonitorQuery,
    },
    Update {
        workspace: Id,
        actor: Id,
        host: Id,
        body: crate::runner::UpdateBody,
    },
}

#[derive(Serialize, Deserialize)]
struct Reply {
    value: Option<Value>,
    status: u16,
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message: Option<crate::i18n::Message>,
}

impl Reply {
    fn into_value(self) -> Result<Value> {
        if let Some(error) = self.error {
            return Err(ApiError(
                StatusCode::from_u16(self.status).unwrap_or(StatusCode::BAD_GATEWAY),
                error,
                self.message,
            ));
        }
        Ok(self.value.unwrap_or(Value::Null))
    }
}
#[derive(Serialize, Deserialize)]
struct Envelope<T> {
    expires_at: i64,
    request: T,
}

#[derive(Default, Serialize, Deserialize)]
struct Reservations {
    readers: BTreeMap<String, i64>,
    writer: Option<(String, i64)>,
    waiting: Option<(String, i64)>,
}

pub struct Reservation {
    valid: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
    cluster: Arc<Cluster>,
    key: String,
    token: String,
}
impl Reservation {
    pub fn valid(&self) -> bool {
        self.valid.load(Ordering::Acquire)
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.task.abort();
        let cluster = self.cluster.clone();
        let key = self.key.clone();
        let token = self.token.clone();
        tokio::spawn(async move {
            let _ = cluster.edit_reservation(&key, &token, None, false).await;
        });
    }
}

impl Cluster {
    /// CAS the entire small reader/writer set. Expiry never creates permission
    /// to replay an operation, and a queued writer prevents new readers.
    async fn edit_reservation(
        &self,
        key: &str,
        token: &str,
        write: Option<bool>,
        renew: bool,
    ) -> Result<bool> {
        for _ in 0..64 {
            let entry = self
                .coordination
                .entry(key)
                .await
                .map_err(failed)?
                .filter(|entry| entry.operation == Operation::Put);
            let mut locks: Reservations = match &entry {
                Some(entry) => serde_json::from_slice(&entry.value).map_err(failed)?,
                None => Reservations::default(),
            };
            let now = crate::db::now();
            let expires = now + TTL.as_millis() as i64;
            locks.readers.retain(|_, expires| *expires > now);
            if locks
                .writer
                .as_ref()
                .is_some_and(|(_, expiry)| *expiry <= now)
            {
                locks.writer = None;
            }
            if locks
                .waiting
                .as_ref()
                .is_some_and(|(_, expiry)| *expiry <= now)
            {
                locks.waiting = None;
            }
            let acquired = match write {
                None => {
                    locks.readers.remove(token);
                    if locks.writer.as_ref().is_some_and(|(who, _)| who == token) {
                        locks.writer = None;
                    }
                    if locks.waiting.as_ref().is_some_and(|(who, _)| who == token) {
                        locks.waiting = None;
                    }
                    true
                }
                Some(false) => {
                    if locks.readers.contains_key(token) {
                        locks.readers.insert(token.into(), expires);
                        true
                    } else if !renew
                        && locks.writer.is_none()
                        && locks.waiting.is_none()
                        && locks.readers.len() < 256
                    {
                        locks.readers.insert(token.into(), expires);
                        true
                    } else {
                        false
                    }
                }
                Some(true) => {
                    if locks.writer.as_ref().is_some_and(|(who, _)| who == token) {
                        locks.writer = Some((token.into(), expires));
                        true
                    } else if !renew
                        && locks.writer.is_none()
                        && locks.readers.is_empty()
                        && locks.waiting.as_ref().is_none_or(|(who, _)| who == token)
                    {
                        locks.writer = Some((token.into(), expires));
                        locks.waiting = None;
                        true
                    } else {
                        if !renew && locks.waiting.as_ref().is_none_or(|(who, _)| who == token) {
                            locks.waiting = Some((token.into(), expires));
                        }
                        false
                    }
                }
            };
            let bytes = serde_json::to_vec(&locks).map_err(failed)?.into();
            let result = match entry {
                Some(entry) => self
                    .coordination
                    .update(key, bytes, entry.revision)
                    .await
                    .map(|_| ())
                    .map_err(|_| ()),
                None => self
                    .coordination
                    .create(key, bytes)
                    .await
                    .map(|_| ())
                    .map_err(|_| ()),
            };
            if result.is_ok() {
                return Ok(acquired);
            }
            tokio::task::yield_now().await;
        }
        Err(unavailable())
    }

    pub async fn reserve(self: &Arc<Self>, host: &str, write: bool) -> Result<Reservation> {
        let key = format!("reservation.{host}");
        let token = uuid::Uuid::new_v4().simple().to_string();
        // A cancelled waiter expires without holding an execution reservation.
        loop {
            if !write {
                use sea_orm::EntityTrait;
                if let Ok(id) = host.parse::<i64>() {
                    if let Some(host) = crate::entities::hosts::Entity::find_by_id(id)
                        .one(&self.db)
                        .await?
                    {
                        if host
                            .agent_update
                            .as_deref()
                            .and_then(|value| serde_json::from_str::<Value>(value).ok())
                            .is_some_and(|job| {
                                matches!(
                                    job["state"].as_str(),
                                    Some("draining" | "updating" | "unknown")
                                )
                            })
                        {
                            tokio::time::sleep(Duration::from_millis(200)).await;
                            continue;
                        }
                    }
                }
            }
            if self
                .edit_reservation(&key, &token, Some(write), false)
                .await?
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let valid = Arc::new(AtomicBool::new(true));
        let task_valid = valid.clone();
        let cluster = self.clone();
        let task_key = key.clone();
        let task_token = token.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                if !matches!(
                    cluster
                        .edit_reservation(&task_key, &task_token, Some(write), true)
                        .await,
                    Ok(true)
                ) {
                    diagnostic("nats", "lease_renew", "reservation_lost");
                    task_valid.store(false, Ordering::Release);
                    break;
                }
            }
        });
        Ok(Reservation {
            valid,
            task,
            cluster: self.clone(),
            key,
            token,
        })
    }
}

impl Cluster {
    pub async fn acquire_owner(
        &self,
        s: &AppState,
        host: &str,
        workspace: Id,
        fence: String,
    ) -> Result<(Owner, u64)> {
        let name = format!("runner-owner:{host}");
        if !crate::lock::acquire(
            &s.db,
            &name,
            &fence,
            crate::db::now(),
            TTL.as_millis() as i64,
        )
        .await?
        {
            return Err(unavailable());
        }
        let owner = Owner {
            node: self.node.clone(),
            fence: fence.clone(),
            workspace,
            info: None,
        };
        match self.claim(host, &owner).await {
            Ok(revision) => Ok((owner, revision)),
            Err(error) => {
                let _ = crate::lock::release(&s.db, &name, &fence).await;
                Err(error)
            }
        }
    }

    pub async fn renew_owner(
        &self,
        s: &AppState,
        host: &str,
        owner: &Owner,
        revision: u64,
    ) -> Result<u64> {
        use crate::entities::locks;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};
        let now = crate::db::now();
        let renewed = locks::Entity::update_many()
            .col_expr(
                locks::Column::ExpiresAt,
                Expr::value(now + TTL.as_millis() as i64),
            )
            .filter(locks::Column::Name.eq(format!("runner-owner:{host}")))
            .filter(locks::Column::Owner.eq(&owner.fence))
            .filter(locks::Column::ExpiresAt.gt(now))
            .exec(&s.db)
            .await?;
        if renewed.rows_affected != 1 {
            diagnostic("nats", "ownership_renew", "lease_expired_or_owner_changed");
            return Err(unavailable());
        }
        self.renew(host, owner, revision).await
    }
    pub async fn connect(
        url: &str,
        prefix: &str,
        replicas: usize,
        db: sea_orm::DatabaseConnection,
    ) -> anyhow::Result<Arc<Self>> {
        anyhow::ensure!(
            !prefix.is_empty()
                && prefix
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "cluster namespace must contain only letters, numbers or underscores"
        );
        anyhow::ensure!(
            [1, 3, 5].contains(&replicas),
            "NATS replicas must be 1, 3 or 5"
        );
        // async-nats ignores credentials embedded in the URL, so pass them
        // explicitly: user:password, or a lone user as a token.
        let addr: async_nats::ServerAddr = url
            .parse()
            .map_err(|_| anyhow::anyhow!("cluster NATS URL is invalid"))?;
        let decode = |part: &str| {
            percent_encoding::percent_decode_str(part)
                .decode_utf8_lossy()
                .into_owned()
        };
        let options = match (addr.username(), addr.password()) {
            (Some(user), Some(password)) => {
                async_nats::ConnectOptions::with_user_and_password(decode(user), decode(password))
            }
            (Some(token), None) => async_nats::ConnectOptions::with_token(decode(token)),
            _ => async_nats::ConnectOptions::new(),
        };
        // The error kind never contains the URL or its credentials.
        let client = options
            .request_timeout(Some(IO_TIMEOUT))
            .connect(addr)
            .await
            .map_err(|e| anyhow::anyhow!("cannot connect to cluster NATS: {}", e.kind()))?;
        let js = jetstream::new(client.clone());
        let mut stores = Vec::new();
        for suffix in ["owners", "sessions", "coordination"] {
            let bucket = format!("{prefix}_{suffix}");
            let config = jetstream::kv::Config {
                bucket: bucket.clone(),
                history: 1,
                max_age: TTL,
                max_value_size: 256 * 1024,
                num_replicas: replicas,
                ..Default::default()
            };
            let store = match js.create_key_value(config).await {
                Ok(store) => store,
                Err(_) => js.get_key_value(&bucket).await.map_err(|_| {
                    anyhow::anyhow!("cannot create/open cluster KV bucket {bucket}")
                })?,
            };
            let status = store.status().await?;
            anyhow::ensure!(
                status.max_age() == TTL
                    && status.history() == 1
                    && status.info.config.num_replicas == replicas,
                "cluster KV bucket {bucket} has incompatible expiry/history/replica configuration"
            );
            stores.push(store);
        }
        let cluster = Arc::new(Self {
            client,
            owners: stores.remove(0),
            sessions: stores.remove(0),
            coordination: stores.remove(0),
            prefix: prefix.into(),
            node: uuid::Uuid::new_v4().simple().to_string(),
            db,
        });
        cluster
            .coordination
            .put(
                format!("node.{}", cluster.node),
                Bytes::from_static(b"alive"),
            )
            .await?;
        Ok(cluster)
    }

    pub async fn owner(&self, host: &str) -> Result<Option<Owner>> {
        self.owners
            .get(host)
            .await
            .map_err(failed)?
            .map(|bytes| serde_json::from_slice(&bytes).map_err(failed))
            .transpose()
    }

    pub async fn collector_owner(
        &self,
        s: &AppState,
        host: &crate::entities::hosts::Model,
    ) -> Result<Option<Owner>> {
        let id = host.id.to_string();
        if host.connection == "runner" {
            return Ok(self
                .owner(&id)
                .await?
                .filter(|owner| owner.node == self.node));
        }
        let entry = self
            .owners
            .entry(&id)
            .await
            .map_err(failed)?
            .filter(|entry| entry.operation == Operation::Put);
        if let Some(entry) = entry {
            let owner: Owner = serde_json::from_slice(&entry.value).map_err(failed)?;
            if owner.node != self.node {
                return Ok(None);
            }
            self.renew_owner(s, &id, &owner, entry.revision).await?;
            return Ok(Some(owner));
        }
        match self
            .acquire_owner(s, &id, host.workspace_id.into(), self.node.clone())
            .await
        {
            Ok((owner, _)) => Ok(Some(owner)),
            Err(_) => Ok(None),
        }
    }

    pub async fn claim(&self, host: &str, owner: &Owner) -> Result<u64> {
        self.owners
            .create(host, serde_json::to_vec(owner).map_err(failed)?.into())
            .await
            .map_err(failed)
    }

    pub async fn renew(&self, host: &str, owner: &Owner, revision: u64) -> Result<u64> {
        self.owners
            .update(
                host,
                serde_json::to_vec(owner).map_err(failed)?.into(),
                revision,
            )
            .await
            .map_err(failed)
    }

    pub async fn release(&self, host: &str, revision: u64) {
        let _ = self
            .owners
            .delete_expect_revision(host, Some(revision))
            .await;
    }

    pub async fn rpc<T: DeserializeOwned>(&self, node: &str, request: &Request) -> Result<T> {
        if self.client.connection_state() != async_nats::connection::State::Connected {
            diagnostic("nats", "rpc", "broker_disconnected");
            return Err(unavailable());
        }
        let inbox = self.client.new_inbox();
        let mut responses = self.client.subscribe(inbox.clone()).await.map_err(failed)?;
        self.client.flush().await.map_err(failed)?;
        self.client
            .publish_with_reply(
                format!("{}.rpc.{node}", self.prefix),
                inbox,
                serde_json::to_vec(&Envelope {
                    expires_at: crate::db::now() + IO_TIMEOUT.as_millis() as i64,
                    request,
                })
                .map_err(failed)?
                .into(),
            )
            .await
            .map_err(failed)?;
        let bytes = tokio::time::timeout(IO_TIMEOUT, async {
            let mut bytes = Vec::new();
            while let Some(response) = responses.next().await {
                if response
                    .status
                    .is_some_and(|status| status.is_server_error())
                {
                    diagnostic("nats", "rpc", "broker_error_response");
                    return Err(unavailable());
                }
                if append_packet(&mut bytes, &response.payload)? {
                    return Ok(bytes);
                }
            }
            diagnostic("nats", "rpc", "response_subscription_closed");
            Err(unavailable())
        })
        .await
        .map_err(|_| {
            diagnostic("nats", "rpc", "response_timeout");
            unavailable()
        })??;
        let reply: Reply = serde_json::from_slice(&bytes).map_err(failed)?;
        serde_json::from_value(reply.into_value()?).map_err(failed)
    }

    pub async fn serve(
        self: Arc<Self>,
        s: AppState,
        mut stop: tokio::sync::watch::Receiver<bool>,
    ) -> Result<()> {
        let mut requests = self
            .client
            .subscribe(format!("{}.rpc.{}", self.prefix, self.node))
            .await
            .map_err(failed)?;
        self.client.flush().await.map_err(failed)?;
        let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                _ = stop.changed() => break,
                _ = heartbeat.tick()=>{if self.coordination.put(format!("node.{}",self.node),Bytes::from_static(b"alive")).await.is_err() {tracing::warn!("NATS cluster heartbeat unavailable");}},
                message = requests.next() => {
                    let Some(message) = message else { break; };
                    let Some(reply) = message.reply else { continue; };
                    let this = self.clone(); let s = s.clone();
                    tokio::spawn(async move {
                        let answer = match serde_json::from_slice::<Envelope<Request>>(&message.payload) {
                            Ok(envelope) if envelope.expires_at>=crate::db::now()=>this.dispatch(&s,envelope.request).await,
                            Ok(_)=>Err(unavailable()),
                            Err(_) => Err(ApiError::localized(StatusCode::BAD_REQUEST, crate::i18n::Message::new("runner.clusterRequest"))),
                        };
                        let reply_body = match answer {
                            Ok(value) => Reply { value: Some(value), status: 200, error: None, message: None },
                            Err(error) => Reply { value: None, status: error.0.as_u16(), error: Some(error.1), message: error.2 },
                        };
                        if let Ok(bytes) = serde_json::to_vec(&reply_body) {
                            if bytes.len()>MAX_MESSAGE {let error=ApiError::localized(StatusCode::BAD_GATEWAY,crate::i18n::Message::new("runner.clusterResponse"));let reply_body=Reply {value:None,status:502,error:Some(error.1),message:error.2};if let Ok(bytes)=serde_json::to_vec(&reply_body){let _=this.client.publish(reply,packet(&bytes,true)).await;}}
                            else {let chunks=bytes.chunks(MAX_FRAME);let count=chunks.len();for(index,chunk)in chunks.enumerate(){if this.client.publish(reply.clone(),packet(chunk,index+1==count)).await.is_err(){break;}}}
                        }
                    });
                }
            }
        }
        Ok(())
    }

    pub async fn node_alive(&self, node: &str) -> Result<bool> {
        Ok(self
            .coordination
            .get(format!("node.{node}"))
            .await
            .map_err(failed)?
            .is_some())
    }

    async fn dispatch(&self, s: &AppState, request: Request) -> Result<Value> {
        match request {
            Request::Open {
                host,
                fence,
                session,
                kind,
            } => {
                let owner = self.owner(&host).await?.ok_or_else(unavailable)?;
                if owner.node != self.node || owner.fence != fence {
                    diagnostic("nats", "open", "ownership_changed");
                    return Err(unavailable());
                }
                crate::runner::cluster_open(&host, &session, kind).await?;
                Ok(Value::Null)
            }
            Request::Monitor {
                workspace,
                host,
                query,
            } => {
                query.validate()?;
                if !self
                    .owner(&host.to_string())
                    .await?
                    .is_some_and(|owner| owner.node == self.node && owner.workspace == workspace)
                {
                    return Err(unavailable());
                }
                use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
                crate::entities::hosts::Entity::find_by_id(i64::from(host))
                    .filter(crate::entities::hosts::Column::WorkspaceId.eq(workspace))
                    .one(&s.db)
                    .await?
                    .ok_or_else(ApiError::missing)?;
                Ok(s.host_monitor
                    .snapshot(workspace, host, &query, crate::db::now()))
            }
            Request::Update {
                workspace,
                actor,
                host,
                body,
            } => {
                let scope = crate::workspace::Scope::for_actor(s, workspace, actor).await?;
                let (_, crate::error::Json(value)) = crate::runner::start_update(
                    axum::extract::State(s.clone()),
                    scope,
                    crate::error::Path((workspace.to_string(), host)),
                    crate::error::Json(body),
                )
                .await?;
                Ok(value)
            }
        }
    }

    pub async fn new_session(&self, session: &str, route: &SessionRoute) -> Result<()> {
        self.sessions
            .create(session, serde_json::to_vec(route).map_err(failed)?.into())
            .await
            .map_err(failed)?;
        Ok(())
    }

    pub async fn wait_session(&self, session: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            if self
                .client
                .request(
                    format!("{}.stream.{session}.1", self.prefix),
                    Bytes::from_static(&[5]),
                )
                .await
                .is_ok()
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                diagnostic("nats", "wait", "session_not_ready");
                return Err(unavailable());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn claim_session(&self, session: &str, host: &str) -> Result<SessionRoute> {
        let entry = self
            .sessions
            .entry(session)
            .await
            .map_err(failed)?
            .filter(|entry| entry.operation == Operation::Put)
            .ok_or_else(unavailable)?;
        let route: SessionRoute = serde_json::from_slice(&entry.value).map_err(failed)?;
        if route.host != host {
            return Err(unavailable());
        }
        let owner = self.owner(host).await?.ok_or_else(unavailable)?;
        if owner.fence != route.owner.fence || owner.node != route.owner.node {
            diagnostic("nats", "claim", "ownership_changed");
            return Err(unavailable());
        }
        self.sessions
            .delete_expect_revision(session, Some(entry.revision))
            .await
            .map_err(failed)?;
        Ok(route)
    }

    pub async fn revoke(&self, s: &AppState, host: &str, owner: Option<&Owner>) -> Result<()> {
        // Broadcast only closes sessions. Ownership is released by its fenced owner.
        let Some(owner) = owner else {
            return Ok(());
        };
        self.client
            .publish(
                format!("{}.revoke.{host}", self.prefix),
                Bytes::copy_from_slice(owner.fence.as_bytes()),
            )
            .await
            .map_err(failed)?;
        self.client.flush().await.map_err(failed)?;
        for _ in 0..16 {
            let entry = self
                .owners
                .entry(host)
                .await
                .map_err(failed)?
                .filter(|entry| entry.operation == Operation::Put);
            let Some(entry) = entry else {
                break;
            };
            let current: Owner = serde_json::from_slice(&entry.value).map_err(failed)?;
            if current.fence != owner.fence {
                break;
            }
            if self
                .owners
                .delete_expect_revision(host, Some(entry.revision))
                .await
                .is_ok()
            {
                break;
            }
        }
        crate::lock::release(&s.db, &format!("runner-owner:{host}"), &owner.fence).await?;
        Ok(())
    }

    pub async fn subscribe_revoke(&self, host: &str) -> Result<async_nats::Subscriber> {
        self.client
            .subscribe(format!("{}.revoke.{host}", self.prefix))
            .await
            .map_err(failed)
    }

    /// Each direction has at most two queued frames plus one acknowledged frame.
    /// Losing a peer closes the stream; request timeouts are never retried.
    pub async fn socket(
        self: &Arc<Self>,
        session: &str,
        side: bool,
        route: &SessionRoute,
    ) -> Result<Socket> {
        let incoming = format!("{}.stream.{session}.{}", self.prefix, u8::from(side));
        let outgoing = format!("{}.stream.{session}.{}", self.prefix, u8::from(!side));
        let mut subscriber = self.client.subscribe(incoming).await.map_err(failed)?;
        let mut revoked = self.subscribe_revoke(&route.host).await?;
        self.client.flush().await.map_err(failed)?;
        let (send, mut outgoing_rx) = mpsc::channel::<(Message, oneshot::Sender<Result<()>>)>(2);
        let (incoming_tx, receive) = mpsc::channel(2);
        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        let active = Arc::new(AtomicBool::new(false));
        let writer_active = active.clone();
        let lease = Arc::new(std::sync::Mutex::new(None::<crate::runner_updates::Lease>));
        let writer_lease = lease.clone();
        let this = self.clone();
        let host = route.host.clone();
        let fence = route.owner.fence.clone();
        let writer_error = incoming_tx.clone();
        let diagnostic_span =
            tracing::info_span!("runner_stream", host_id = %route.host, node_id = %self.node);
        let writer = tokio::spawn(async move {
            let mut pulse = tokio::time::interval(Duration::from_secs(5));
            pulse.tick().await;
            loop {
                let (bytes, ack) = tokio::select! {
                    _ = stopped.changed() => break,
                    value = outgoing_rx.recv() => match value {
                        Some((message, ack)) => match encode(message) { Ok(bytes) => (bytes, Some(ack)), Err(error) => {let _=ack.send(Err(error)); break;} },
                        None => break,
                    },
                    _ = pulse.tick() => {
                        if !writer_active.load(Ordering::Acquire) {continue;}
                        if writer_lease.lock().unwrap_or_else(|error|error.into_inner()).as_ref().is_some_and(|lease|!lease.valid()) {
                            diagnostic("nats", "stream_heartbeat", "lease_invalid");
                            break;
                        }
                        match this.owner(&host).await {
                            Ok(Some(owner)) if owner.fence == fence => {},
                            Ok(_) => {diagnostic("nats", "stream_heartbeat", "ownership_changed"); break;},
                            Err(_) => {diagnostic("nats", "stream_heartbeat", "ownership_lookup_failed"); break;},
                        }
                        (Bytes::from_static(&[5]), None)
                    }
                };
                let closing = bytes.first() == Some(&2);
                let result = async {
                    if this.client.connection_state() != async_nats::connection::State::Connected {
                        if !closing { diagnostic("nats", "stream_send", "broker_disconnected"); }
                        return Err(unavailable());
                    }
                    if bytes.as_ref() == [5] {
                        this.client
                            .request(outgoing.clone(), bytes)
                            .await
                            .map_err(|error| {
                                tracing::warn!(transport = "nats", phase = "stream_heartbeat", error_kind = %error.kind(), "Runner transport request failed");
                                diagnostic("nats", "stream_heartbeat", "request_failed");
                                unavailable()
                            })?;
                        return Ok(());
                    }
                    let chunks = bytes.chunks(MAX_FRAME);
                    let count = chunks.len();
                    for (index, chunk) in chunks.enumerate() {
                        let reply = this
                            .client
                            .request(outgoing.clone(), packet(chunk, index + 1 == count))
                            .await
                            .map_err(|error| {
                                if !closing {
                                    tracing::warn!(transport = "nats", phase = "stream_send", error_kind = %error.kind(), "Runner transport request failed");
                                }
                                unavailable()
                            })?;
                        if reply.payload.as_ref() != b"ok" {
                            if !closing { diagnostic("nats", "stream_send", "invalid_acknowledgement"); }
                            return Err(unavailable());
                        }
                    }
                    Ok(())
                }
                .await;
                let broken = result.is_err();
                if !broken {
                    writer_active.store(true, Ordering::Release);
                }
                if let Some(ack) = ack {
                    let _ = ack.send(result);
                }
                if broken {
                    break;
                }
            }
            let _ = writer_error.send(Err(unavailable())).await;
        }.instrument(diagnostic_span.clone()));
        let this = self.clone();
        let mut stopped = stop.subscribe();
        let revoked_fence = route.owner.fence.clone();
        let reader = tokio::spawn(async move {
            let mut assembled = Vec::new();
            loop {
                tokio::select! {
                    _ = stopped.changed() => break,
                    notice = revoked.next() => {if notice.is_none_or(|notice|notice.payload.as_ref()==revoked_fence.as_bytes()) {
                        diagnostic("nats", "stream_recv", "revocation_or_subscription_closed");
                        break;
                    }},
                    message = subscriber.next() => {
                        let Some(message) = message else {
                            diagnostic("nats", "stream_recv", "subscription_closed");
                            break;
                        };
                        active.store(true,Ordering::Release);
                        if message.payload.as_ref() != [5] {
                            match append_packet(&mut assembled,&message.payload) {
                                Ok(true)=>{
                                    let frame=decode(&assembled);assembled.clear();
                                    match tokio::time::timeout(IO_TIMEOUT,incoming_tx.send(frame)).await {
                                        Ok(Ok(())) => {},
                                        Ok(Err(_)) => {
                                            tracing::debug!(transport="nats", phase="stream_recv", "Runner stream consumer closed");
                                            break;
                                        },
                                        Err(_) => {
                                            diagnostic("nats", "stream_recv", "consumer_timeout");
                                            break;
                                        },
                                    }
                                },
                                Ok(false)=>{},Err(_)=>{
                                    diagnostic("nats", "stream_recv", "invalid_or_expired_packet");
                                    break;
                                },
                            }
                        }
                        if let Some(reply) = message.reply { if this.client.publish(reply, Bytes::from_static(b"ok")).await.is_err() {break;} }
                    }
                }
            }
            let _ = incoming_tx.send(Err(unavailable())).await;
        }.instrument(diagnostic_span));
        Ok(Socket::Forwarded(Forwarded {
            send,
            receive,
            _stop: stop,
            lease,
            tasks: vec![writer, reader],
        }))
    }
}

/// Put the ownership check in the same SQL statement as a status write. An old
/// server can never mark a newly owned connection offline after checking a lease.
pub fn fence_condition(host: &str, fence: &str) -> sea_orm::sea_query::SimpleExpr {
    use crate::entities::locks;
    use sea_orm::{
        ColumnTrait,
        sea_query::{Expr, Query},
    };
    Expr::exists(
        Query::select()
            .expr(Expr::val(1))
            .from(locks::Entity)
            .and_where(locks::Column::Name.eq(format!("runner-owner:{host}")))
            .and_where(locks::Column::Owner.eq(fence))
            .and_where(locks::Column::ExpiresAt.gt(crate::db::now()))
            .to_owned(),
    )
}

pub enum Socket {
    Local(WebSocket),
    Forwarded(Forwarded),
}
pub struct Forwarded {
    send: mpsc::Sender<(Message, oneshot::Sender<Result<()>>)>,
    receive: mpsc::Receiver<Result<Message>>,
    _stop: tokio::sync::watch::Sender<bool>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    lease: Arc<std::sync::Mutex<Option<crate::runner_updates::Lease>>>,
}
impl Drop for Forwarded {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl From<WebSocket> for Socket {
    fn from(value: WebSocket) -> Self {
        Self::Local(value)
    }
}
impl Socket {
    pub(crate) fn hold(&mut self, lease: crate::runner_updates::Lease) {
        if let Self::Forwarded(socket) = self {
            *socket
                .lease
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(lease);
        }
    }
    pub async fn send(&mut self, message: Message) -> Result<()> {
        match self {
            Self::Local(socket) => {
                // The peer may already have closed after returning its result.
                let phase = (!matches!(message, Message::Close(_))).then_some("stream_send");
                socket
                    .send(message)
                    .await
                    .map_err(|error| local_failed(phase, error))
            }
            Self::Forwarded(socket) => {
                let closing = matches!(message, Message::Close(_));
                let (tx, rx) = oneshot::channel();
                socket.send.send((message, tx)).await.map_err(|_| {
                    if !closing {
                        diagnostic("nats", "stream_send", "writer_closed");
                    }
                    unavailable()
                })?;
                rx.await.map_err(|_| {
                    if !closing {
                        diagnostic("nats", "stream_send", "writer_acknowledgement_dropped");
                    }
                    unavailable()
                })?
            }
        }
    }
    pub async fn recv(&mut self) -> Option<Result<Message>> {
        match self {
            Self::Local(socket) => socket
                .recv()
                .await
                .map(|value| value.map_err(|error| local_failed(Some("stream_recv"), error))),
            Self::Forwarded(socket) => socket.receive.recv().await,
        }
    }
}

pub async fn bridge(mut left: Socket, mut right: Socket) {
    loop {
        tokio::select! {
            message = left.recv() => match message { Some(Ok(message)) => {let close=matches!(message,Message::Close(_)); if right.send(message).await.is_err() || close {break;}}, _=>break },
            message = right.recv() => match message { Some(Ok(message)) => {let close=matches!(message,Message::Close(_)); if left.send(message).await.is_err() || close {break;}}, _=>break },
        }
    }
    let _ = left.send(Message::Close(None)).await;
    let _ = right.send(Message::Close(None)).await;
}

fn encode(message: Message) -> Result<Bytes> {
    let (tag, data): (u8, Bytes) = match message {
        Message::Text(value) => (0, Bytes::copy_from_slice(value.as_bytes())),
        Message::Binary(value) => (1, value),
        Message::Close(_) => (2, Bytes::new()),
        Message::Ping(value) => (3, value),
        Message::Pong(value) => (4, value),
    };
    if data.len() > MAX_MESSAGE - 1 {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("runner.clusterFrame"),
        ));
    }
    let mut output = Vec::with_capacity(data.len() + 1);
    output.push(tag);
    output.extend_from_slice(&data);
    Ok(output.into())
}
fn decode(bytes: &[u8]) -> Result<Message> {
    if bytes.len() > MAX_MESSAGE {
        return Err(unavailable());
    }
    let (&tag, data) = bytes.split_first().ok_or_else(unavailable)?;
    Ok(match tag {
        0 => Message::Text(std::str::from_utf8(data).map_err(failed)?.to_owned().into()),
        1 => Message::Binary(Bytes::copy_from_slice(data)),
        2 => Message::Close(None),
        3 => Message::Ping(Bytes::copy_from_slice(data)),
        4 => Message::Pong(Bytes::copy_from_slice(data)),
        _ => return Err(unavailable()),
    })
}

fn packet(bytes: &[u8], last: bool) -> Bytes {
    let mut result = Vec::with_capacity(bytes.len() + 9);
    result.push(if last { 7 } else { 6 });
    result.extend_from_slice(&(crate::db::now() + IO_TIMEOUT.as_millis() as i64).to_be_bytes());
    result.extend_from_slice(bytes);
    result.into()
}
fn append_packet(output: &mut Vec<u8>, bytes: &[u8]) -> Result<bool> {
    let (&tag, bytes) = bytes.split_first().ok_or_else(unavailable)?;
    if bytes.len() < 8 {
        return Err(unavailable());
    }
    let (deadline, bytes) = bytes.split_at(8);
    let expires = i64::from_be_bytes(deadline.try_into().map_err(failed)?);
    if expires < crate::db::now() {
        return Err(unavailable());
    }
    if ![6, 7].contains(&tag) || bytes.len() > MAX_FRAME || output.len() + bytes.len() > MAX_MESSAGE
    {
        return Err(unavailable());
    }
    output.extend_from_slice(bytes);
    Ok(tag == 7)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
    struct Capture(Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn local_websocket_failures_keep_transport_identity_and_private_data_out_of_logs() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tracing::instrument::WithSubscriber;

        // Tracing caches callsite interest process-wide. Other parallel tests emit
        // through this same callsite without a subscriber, so capture in an isolated
        // test process while retaining every real WebSocket and privacy assertion.
        if std::env::var_os("FERRY_TEST_TRANSPORT_LOG_CHILD").is_none() {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", "cluster::tests::local_websocket_failures_keep_transport_identity_and_private_data_out_of_logs", "--nocapture"])
                .env("FERRY_TEST_TRANSPORT_LOG_CHILD", "1");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000);
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"));
            return;
        }
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        async {
            let (tx, mut rx) = mpsc::channel(1);
            let app = axum::Router::new().route("/socket", axum::routing::get(
                move |upgrade: axum::extract::WebSocketUpgrade| {
                    let tx = tx.clone();
                    async move { upgrade.on_upgrade(move |socket| async move {
                        let _ = tx.send(Socket::from(socket)).await;
                    }) }
                }
            ));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
            client.write_all(format!("GET /socket?token=private-session-token HTTP/1.1\r\nHost: {address}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n").as_bytes()).await.unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") { headers.push(client.read_u8().await.unwrap()); }
            assert!(headers.starts_with(b"HTTP/1.1 101"));
            let mut socket = rx.recv().await.unwrap();
            // A real peer disconnects without a WebSocket close handshake.
            drop(client);
            let error = tokio::time::timeout(Duration::from_secs(2), socket.recv())
                .await.unwrap().unwrap().unwrap_err();
            assert_eq!(error.0, StatusCode::BAD_GATEWAY);
            assert_eq!(error.2.as_ref().unwrap().key, "runner.transportFailed");
            for locale in crate::i18n::Locale::ALL {
                let text = error.render(locale);
                assert!(text.contains("WebSocket"));
                assert_ne!(text, unavailable().render(locale));
                assert!(!text.contains("runner.transportFailed"));
            }
            // The send path uses the same classification after an actual socket
            // has entered its closed state; the payload must never be logged.
            let error = socket.send(Message::Text("private-command-content".into())).await.unwrap_err();
            assert_eq!(error.2.unwrap().key, "runner.transportFailed");
            let before_close = capture.0.lock().unwrap().len();
            let _ = socket.send(Message::Close(None)).await;
            assert_eq!(capture.0.lock().unwrap().len(), before_close, "normal local close must not warn");
            let error = local_failed(Some("stream_recv"), axum::Error::new(std::io::Error::other(
                "nats://private-user:private-password@host/private-session-token private-command-content"
            )));
            assert!(!error.1.contains("private"));
            server.abort();
        }.with_subscriber(subscriber).await;
        let log = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        for required in ["websocket", "stream_send", "stream_recv"] {
            assert!(
                log.contains(required),
                "missing diagnostic {required}: {log}"
            );
        }
        for private in [
            "private-session-token",
            "private-command-content",
            "private-password",
            "private-user",
            "nats://",
        ] {
            assert!(!log.contains(private), "private data in diagnostic");
        }
    }

    #[test]
    fn rpc_error_identity_survives_wire_and_legacy_replies_stay_raw() {
        let identity =
            crate::i18n::Message::new("runner.offline").with("name", "literal {runner.timeout}");
        let original = ApiError::localized(StatusCode::SERVICE_UNAVAILABLE, identity.clone());
        let reply = Reply {
            value: None,
            status: original.0.as_u16(),
            error: Some(original.1),
            message: original.2,
        };
        let error = serde_json::from_slice::<Reply>(&serde_json::to_vec(&reply).unwrap())
            .unwrap()
            .into_value()
            .unwrap_err();
        for locale in crate::i18n::Locale::ALL {
            assert_eq!(error.render(locale), crate::i18n::render(locale, &identity));
            assert!(error.render(locale).contains("literal {runner.timeout}"));
        }
        let legacy: Reply = serde_json::from_value(
            serde_json::json!({"value":null,"status":502,"error":"raw upstream {runner.offline}"}),
        )
        .unwrap();
        assert_eq!(
            legacy
                .into_value()
                .unwrap_err()
                .render(crate::i18n::Locale::JaJp),
            "raw upstream {runner.offline}"
        );
    }

    #[test]
    fn fragmented_metadata_preserves_bytes_and_rejects_oversize_or_invalid_packets() {
        let bytes = encode(Message::Text("x".repeat(MAX_FRAME * 3 + 123).into())).unwrap();
        let mut received = Vec::new();
        let chunks = bytes.chunks(MAX_FRAME);
        let count = chunks.len();
        for (index, chunk) in chunks.enumerate() {
            assert_eq!(
                append_packet(&mut received, &packet(chunk, index + 1 == count)).unwrap(),
                index + 1 == count
            );
        }
        let Message::Text(text) = decode(&received).unwrap() else {
            panic!("text frame");
        };
        assert_eq!(text.len(), MAX_FRAME * 3 + 123);
        assert!(append_packet(&mut Vec::new(), &[42]).is_err());
        assert!(append_packet(&mut vec![0; MAX_MESSAGE], &packet(&[1], true)).is_err());
        let mut expired = packet(&[1], true).to_vec();
        expired[1..9].copy_from_slice(&(crate::db::now() - 1).to_be_bytes());
        assert!(append_packet(&mut Vec::new(), &expired).is_err());
        assert!(encode(Message::Binary(vec![0; MAX_MESSAGE].into())).is_err());
        assert!(decode(&[0, 255]).is_err());
    }

    #[tokio::test]
    #[ignore = "requires NATS_TEST_AUTH_URL: nats://USER:PASSWORD@... of a JetStream server with that user"]
    async fn connects_with_credentials_in_the_url() {
        let url = std::env::var("NATS_TEST_AUTH_URL").expect("authenticated NATS URL");
        let (_dir, db) = crate::lock::tests::database().await;
        let prefix = format!("test_{}", uuid::Uuid::new_v4().simple());
        Cluster::connect(&url, &prefix, 1, db.clone())
            .await
            .expect("credentials from the URL are sent");
        let addr: async_nats::ServerAddr = url.parse().unwrap();
        let password = addr.password().expect("URL has a password");
        let wrong = url.replacen(&format!(":{password}@"), ":wrong@", 1);
        let error = Cluster::connect(&wrong, &prefix, 1, db)
            .await
            .err()
            .expect("a wrong password is rejected")
            .to_string();
        assert!(error.contains("authorization violation"), "{error}");
        assert!(
            !error.contains("wrong"),
            "the error never echoes the URL: {error}"
        );
    }

    #[tokio::test]
    #[ignore = "requires NATS_TEST_URL pointing at an isolated JetStream server"]
    async fn broker_fences_old_owners_and_coordinates_reader_writer_reservations() {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .with_writer(move || writer.clone())
            .finish();
        let _logs = tracing::subscriber::set_default(subscriber);
        let url = std::env::var("NATS_TEST_URL").expect("isolated NATS URL");
        let (dir, db) = crate::lock::tests::database().await;
        let prefix = format!("test_{}", uuid::Uuid::new_v4().simple());
        let a = Cluster::connect(&url, &prefix, 1, db.clone())
            .await
            .unwrap();
        let b = Cluster::connect(&url, &prefix, 1, db.clone())
            .await
            .unwrap();
        let state = |cluster: Arc<Cluster>| {
            Arc::new(crate::state::State {
                ids: Arc::new(
                    crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap(),
                ),
                db: db.clone(),
                data_dir: dir.path().into(),
                instance_id: uuid::Uuid::new_v4().to_string(),
                server_mode: false,
                secure_cookie: false,
                bind: "127.0.0.1:0".parse().unwrap(),
                vault: crate::vault::Vault::open(dir.path(), false).unwrap(),
                host_monitor: Default::default(),
                cluster: Some(cluster),
            })
        };
        let sa = state(a.clone());
        let sb = state(b.clone());
        let (old, revision) = a
            .acquire_owner(&sa, "7", 1.into(), "old".into())
            .await
            .unwrap();
        assert!(
            b.acquire_owner(&sb, "7", 1.into(), "new".into())
                .await
                .is_err()
        );
        // Simulate lease expiry; a later owner must fence every old SQL write.
        crate::entities::locks::Entity::update_many()
            .col_expr(crate::entities::locks::Column::ExpiresAt, Expr::value(0i64))
            .filter(crate::entities::locks::Column::Name.eq("runner-owner:7"))
            .exec(&db)
            .await
            .unwrap();
        a.release("7", revision).await;
        let (new, new_revision) = b
            .acquire_owner(&sb, "7", 1.into(), "new".into())
            .await
            .unwrap();
        assert!(a.renew_owner(&sa, "7", &old, revision).await.is_err());
        a.release("7", revision).await;
        assert_eq!(b.owner("7").await.unwrap().unwrap().fence, "new");
        let stale = crate::entities::locks::Entity::update_many()
            .col_expr(
                crate::entities::locks::Column::AcquiredAt,
                Expr::value(123i64),
            )
            .filter(fence_condition("7", &old.fence))
            .exec(&db)
            .await
            .unwrap();
        assert_eq!(stale.rows_affected, 0);
        let valid = crate::entities::locks::Entity::update_many()
            .col_expr(
                crate::entities::locks::Column::AcquiredAt,
                Expr::value(456i64),
            )
            .filter(fence_condition("7", &new.fence))
            .exec(&db)
            .await
            .unwrap();
        assert_eq!(valid.rows_affected, 1);
        let route = SessionRoute {
            host: "7".into(),
            owner: new.clone(),
        };
        b.new_session("once", &route).await.unwrap();
        assert!(a.claim_session("once", "8").await.is_err());
        a.claim_session("once", "7").await.unwrap();
        assert!(b.claim_session("once", "7").await.is_err());
        let read_a = a.reserve("7", false).await.unwrap();
        let read_b = b.reserve("7", false).await.unwrap();
        let cluster = b.clone();
        let mut writer = tokio::spawn(async move { cluster.reserve("7", true).await.unwrap() });
        assert!(
            tokio::time::timeout(Duration::from_millis(300), &mut writer)
                .await
                .is_err()
        );
        let cluster = a.clone();
        let mut later = tokio::spawn(async move { cluster.reserve("7", false).await.unwrap() });
        assert!(
            tokio::time::timeout(Duration::from_millis(300), &mut later)
                .await
                .is_err()
        );
        drop(read_a);
        drop(read_b);
        let write = tokio::time::timeout(Duration::from_secs(3), writer)
            .await
            .unwrap()
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(300), &mut later)
                .await
                .is_err()
        );
        drop(write);
        let read = tokio::time::timeout(Duration::from_secs(3), later)
            .await
            .unwrap()
            .unwrap();
        assert!(read.valid());
        drop(read);
        let mut left = a.socket("duplex", false, &route).await.unwrap();
        let mut right = b.socket("duplex", true, &route).await.unwrap();
        let data = "metadata".repeat(100_000);
        left.send(Message::Text(data.clone().into())).await.unwrap();
        let Message::Text(received) = right.recv().await.unwrap().unwrap() else {
            panic!("text frame");
        };
        assert_eq!(received.as_str(), data);
        right
            .send(Message::Binary(Bytes::from_static(b"reply")))
            .await
            .unwrap();
        assert!(
            matches!(left.recv().await,Some(Ok(Message::Binary(bytes)))if bytes.as_ref()==b"reply")
        );
        capture.0.lock().unwrap().clear();
        // Both an acknowledged Close and a Close after the peer has already
        // completed its operation are normal teardown, not transport warnings.
        let mut closing = a.socket("normal-close", false, &route).await.unwrap();
        let mut peer = b.socket("normal-close", true, &route).await.unwrap();
        closing.send(Message::Close(None)).await.unwrap();
        assert!(matches!(peer.recv().await, Some(Ok(Message::Close(_)))));
        drop(closing);
        drop(peer);
        let mut closing = a.socket("peer-completed", false, &route).await.unwrap();
        let peer = b.socket("peer-completed", true, &route).await.unwrap();
        drop(peer);
        tokio::task::yield_now().await;
        b.client.flush().await.unwrap();
        let _ = closing.send(Message::Close(None)).await;
        drop(closing);
        tokio::task::yield_now().await;
        let mut closing = a.socket("writer-completed", false, &route).await.unwrap();
        if let Socket::Forwarded(socket) = &closing {
            socket.tasks[0].abort();
        }
        tokio::task::yield_now().await;
        assert!(closing.send(Message::Close(None)).await.is_err());
        drop(closing);
        // A writer that ends after consuming Close drops its acknowledgement.
        // Exercise that separate outer send branch with a real broker request.
        let mut closing = a.socket("writer-ack-dropped", false, &route).await.unwrap();
        let Socket::Forwarded(socket) = &closing else {
            unreachable!()
        };
        let writer = socket.tasks[0].abort_handle();
        let mut peer = b
            .client
            .subscribe(format!("{prefix}.stream.writer-ack-dropped.1"))
            .await
            .unwrap();
        b.client.flush().await.unwrap();
        let (result, ()) = tokio::join!(closing.send(Message::Close(None)), async {
            tokio::time::timeout(Duration::from_secs(2), peer.next())
                .await
                .unwrap()
                .unwrap();
            writer.abort();
        });
        assert!(result.is_err());
        drop(closing);
        assert!(
            !String::from_utf8_lossy(&capture.0.lock().unwrap()).contains("WARN"),
            "normal teardown must not warn"
        );
        b.revoke(&sb, "7", Some(&new)).await.unwrap();
        let error = tokio::time::timeout(Duration::from_secs(2), left.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.2.unwrap().key, "runner.clusterUnavailable");
        let log = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(log.contains("stream_recv") && log.contains("revocation_or_subscription_closed"));
        assert!(log.contains("host_id=7") && log.contains("node_id="));
        assert!(!log.contains("duplex") && !log.contains(&data));
        b.release("7", new_revision).await;
    }
}
