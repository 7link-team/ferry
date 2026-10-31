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

const TTL: Duration = Duration::from_secs(30);
const IO_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_FRAME: usize = 256 * 1024;
const MAX_MESSAGE: usize = 16 * 1024 * 1024;

pub fn unavailable() -> ApiError {
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "节点连接所属服务不可用，请等待代理重连后重新打开会话。".into(),
    )
}
fn failed(_: impl std::fmt::Display) -> ApiError {
    unavailable()
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
        let client = async_nats::ConnectOptions::new()
            .request_timeout(Some(IO_TIMEOUT))
            .connect(url)
            .await
            .map_err(|_| anyhow::anyhow!("cannot connect to cluster NATS"))?;
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
                    return Err(unavailable());
                }
                if append_packet(&mut bytes, &response.payload)? {
                    return Ok(bytes);
                }
            }
            Err(unavailable())
        })
        .await
        .map_err(failed)??;
        let reply: Reply = serde_json::from_slice(&bytes).map_err(failed)?;
        if let Some(error) = reply.error {
            return Err(ApiError(
                StatusCode::from_u16(reply.status).unwrap_or(StatusCode::BAD_GATEWAY),
                error,
            ));
        }
        serde_json::from_value(reply.value.unwrap_or(Value::Null)).map_err(failed)
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
                            Err(_) => Err(ApiError::bad("集群请求无效。")),
                        };
                        let reply_body = match answer {
                            Ok(value) => Reply { value: Some(value), status: 200, error: None },
                            Err(error) => Reply { value: None, status: error.0.as_u16(), error: Some(error.1) },
                        };
                        if let Ok(bytes) = serde_json::to_vec(&reply_body) {
                            if bytes.len()>MAX_MESSAGE {let reply_body=Reply {value:None,status:502,error:Some("集群查询响应超过 16 MiB，请缩小查询范围。".into())};if let Ok(bytes)=serde_json::to_vec(&reply_body){let _=this.client.publish(reply,packet(&bytes,true)).await;}}
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
                let (_, axum::Json(value)) = crate::runner::start_update(
                    axum::extract::State(s.clone()),
                    scope,
                    axum::extract::Path((workspace.to_string(), host)),
                    axum::Json(body),
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
                        if writer_lease.lock().unwrap_or_else(|error|error.into_inner()).as_ref().is_some_and(|lease|!lease.valid()) {break;}
                        if !this.owner(&host).await.ok().flatten().is_some_and(|owner| owner.fence == fence) { break; }
                        (Bytes::from_static(&[5]), None)
                    }
                };
                let result = async {
                    if this.client.connection_state() != async_nats::connection::State::Connected {
                        return Err(unavailable());
                    }
                    if bytes.as_ref() == [5] {
                        this.client
                            .request(outgoing.clone(), bytes)
                            .await
                            .map_err(failed)?;
                        return Ok(());
                    }
                    let chunks = bytes.chunks(MAX_FRAME);
                    let count = chunks.len();
                    for (index, chunk) in chunks.enumerate() {
                        let reply = this
                            .client
                            .request(outgoing.clone(), packet(chunk, index + 1 == count))
                            .await
                            .map_err(failed)?;
                        if reply.payload.as_ref() != b"ok" {
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
        });
        let this = self.clone();
        let mut stopped = stop.subscribe();
        let revoked_fence = route.owner.fence.clone();
        let reader = tokio::spawn(async move {
            let mut assembled = Vec::new();
            loop {
                tokio::select! {
                    _ = stopped.changed() => break,
                    notice = revoked.next() => {if notice.is_none_or(|notice|notice.payload.as_ref()==revoked_fence.as_bytes()) {break;}},
                    message = subscriber.next() => {
                        let Some(message) = message else {break;};
                        active.store(true,Ordering::Release);
                        if message.payload.as_ref() != [5] {
                            match append_packet(&mut assembled,&message.payload) {
                                Ok(true)=>{
                                    let frame=decode(&assembled);assembled.clear();
                                    if !matches!(tokio::time::timeout(IO_TIMEOUT,incoming_tx.send(frame)).await,Ok(Ok(()))) {break;}
                                },
                                Ok(false)=>{},Err(_)=>break,
                            }
                        }
                        if let Some(reply) = message.reply { if this.client.publish(reply, Bytes::from_static(b"ok")).await.is_err() {break;} }
                    }
                }
            }
            let _ = incoming_tx.send(Err(unavailable())).await;
        });
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
            Self::Local(socket) => socket.send(message).await.map_err(failed),
            Self::Forwarded(socket) => {
                let (tx, rx) = oneshot::channel();
                socket.send.send((message, tx)).await.map_err(failed)?;
                rx.await.map_err(failed)?
            }
        }
    }
    pub async fn recv(&mut self) -> Option<Result<Message>> {
        match self {
            Self::Local(socket) => socket.recv().await.map(|value| value.map_err(failed)),
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
        return Err(ApiError::bad("集群会话数据帧超过 16 MiB。"));
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
    #[ignore = "requires NATS_TEST_URL pointing at an isolated JetStream server"]
    async fn broker_fences_old_owners_and_coordinates_reader_writer_reservations() {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};
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
        b.revoke(&sb, "7", Some(&new)).await.unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), left.recv()).await,
            Ok(Some(Err(_))) | Ok(None)
        ));
        b.release("7", new_revision).await;
    }
}
