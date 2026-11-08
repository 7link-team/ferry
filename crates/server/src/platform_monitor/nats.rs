//! Read-only metrics from this Ferry node's existing authenticated NATS client.
use super::*;

pub async fn overview(State(s): State<AppState>, _: Admin) -> Result<Json<Value>> {
    let Some(cluster) = &s.cluster else {
        return Ok(Json(json!({"configured":false,"connected":false,"sampled_at":db::now()})));
    };
    let client = &cluster.client;
    let connected = client.connection_state() == async_nats::connection::State::Connected
        && tokio::time::timeout(std::time::Duration::from_secs(2), client.flush()).await.is_ok_and(|result| result.is_ok());
    let stats = client.statistics();
    let mut result = json!({"configured":true,"connected":connected,"instance_id":s.instance_id,"sampled_at":db::now(),
        "client":{"in_messages":stats.in_messages.load(Ordering::Relaxed),"out_messages":stats.out_messages.load(Ordering::Relaxed),
        "in_bytes":stats.in_bytes.load(Ordering::Relaxed),"out_bytes":stats.out_bytes.load(Ordering::Relaxed),
        "reconnects":stats.connects.load(Ordering::Relaxed).saturating_sub(1)}});
    if !connected { return Ok(Json(result)); }
    let info = client.server_info();
    // Explicit allowlist: never expose credentials, nonce, subjects or message data.
    result["server"] = json!({"name":info.server_name,"id":info.server_id,"host":info.host,"port":info.port,
        "version":info.version,"cluster":info.cluster,"jetstream":info.jetstream,"tls_required":info.tls_required,
        "auth_required":info.auth_required,"max_payload":info.max_payload});
    if info.jetstream {
        let context = async_nats::jetstream::new(client.clone());
        match tokio::time::timeout(std::time::Duration::from_secs(2), context.query_account()).await {
            Ok(Ok(account)) => result["jetstream"] = json!({"available":true,"memory":account.memory,"storage":account.storage,
                "streams":account.streams,"consumers":account.consumers}),
            _ => result["jetstream"] = json!({"available":false}),
        }
    }
    Ok(Json(result))
}
