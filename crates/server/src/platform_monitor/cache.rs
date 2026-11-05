use super::*;

fn configuration() -> Option<String> {
    if let Ok(url) = std::env::var("FERRY_REDIS_URL") {
        if !url.trim().is_empty() {
            return Some(url);
        }
    }
    if std::env::var("FERRY_REDIS_ENABLED")
        .is_ok_and(|v| matches!(v.as_str(), "false" | "0" | "off"))
    {
        return None;
    }
    let host = std::env::var("FERRY_REDIS_HOST")
        .ok()
        .filter(|s| !s.is_empty())?;
    let port = std::env::var("FERRY_REDIS_PORT").unwrap_or_else(|_| "6379".into());
    let password = std::env::var("FERRY_REDIS_PASSWORD").unwrap_or_default();
    let user = std::env::var("FERRY_REDIS_USERNAME").unwrap_or_default();
    let db = std::env::var("FERRY_REDIS_DB").unwrap_or_else(|_| "0".into());
    let mut url = reqwest::Url::parse(&format!("redis://{host}:{port}/{db}")).ok()?;
    if !password.is_empty() {
        url.set_username(&user).ok()?;
        url.set_password(Some(&password)).ok()?;
    }
    Some(url.to_string())
}
pub async fn overview(
    State(_s): State<AppState>,
    _: Admin,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let Some(url) = configuration() else {
        return Ok(Json(
            json!({"configured":false,"source":"compose redis profile","connected":false}),
        ));
    };
    let safe=reqwest::Url::parse(&url).map(|u|json!({"host":u.host_str(),"port":u.port().unwrap_or(6379),"database":u.path().trim_start_matches('/'),"tls":u.scheme()=="rediss","authentication":u.password().is_some()})).unwrap_or(Value::Null);
    let work = async {
        let client = redis::Client::open(url).map_err(|_| ())?;
        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|_| ())?;
        let info: String = redis::cmd("INFO")
            .arg("ALL")
            .query_async(&mut conn)
            .await
            .map_err(|_| ())?;
        let values = info
            .lines()
            .filter(|s| !s.starts_with('#'))
            .filter_map(|line| line.split_once(':'))
            .filter(|(key, _)| !key.contains("config_file") && !key.contains("executable"))
            .map(|(k, v)| (k.to_owned(), json!(v.trim())))
            .collect::<serde_json::Map<_, _>>();
        let cursor = raw
            .get("cursor")
            .map(|s| s.parse::<u64>().map_err(|_| ()))
            .transpose()?
            .unwrap_or(0);
        let pattern = raw.get("pattern").map(String::as_str).unwrap_or("*");
        if pattern.len() > 200 {
            return Err(());
        }
        let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(pattern)
            .arg("COUNT")
            .arg(50)
            .query_async(&mut conn)
            .await
            .map_err(|_| ())?;
        let mut items = Vec::new();
        for key in keys.into_iter().take(100) {
            let kind: String = redis::cmd("TYPE")
                .arg(&key)
                .query_async(&mut conn)
                .await
                .map_err(|_| ())?;
            let ttl: i64 = redis::cmd("TTL")
                .arg(&key)
                .query_async(&mut conn)
                .await
                .map_err(|_| ())?;
            items.push(json!({"key":key,"type":kind,"ttl":ttl}));
        }
        let slow_len: Option<i64> = redis::cmd("SLOWLOG")
            .arg("LEN")
            .query_async(&mut conn)
            .await
            .ok();
        // SLOWLOG GET includes command arguments (possibly secrets); only its
        // count is exposed. No KEYS *, MONITOR, CONFIG, EVAL or write commands.
        Ok(
            json!({"configured":true,"connected":true,"connection":safe,"info":values,"keys":items,"cursor":next.to_string(),"slowlog_count":slow_len,"readonly":true}),
        )
    };
    match tokio::time::timeout(std::time::Duration::from_secs(5), work).await {
        Ok(Ok(value)) => Ok(Json(value)),
        _ => Ok(Json(
            json!({"configured":true,"connected":false,"connection":safe,"error":"Redis connection or read permissions unavailable"}),
        )),
    }
}
