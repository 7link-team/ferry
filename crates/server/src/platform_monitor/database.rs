use super::*;
use sea_orm::sea_query::{Alias, Expr, Query as SqlQuery};

/// Parameters are never inspected. Inline literals/comments are redacted too.
pub fn fingerprint(sql: &str) -> String {
    let mut out = String::new();
    let mut chars = sql.chars().take(16_384).peekable();
    while let Some(ch) = chars.next() {
        if out.len() > 1500 {
            break;
        }
        if ch == '\'' {
            out.push('?');
            while let Some(c) = chars.next() {
                if c == '\'' {
                    if chars.peek() == Some(&'\'') {
                        chars.next();
                    } else {
                        break;
                    }
                } else if c == '\\' {
                    chars.next();
                }
            }
        } else if ch == '-' && chars.peek() == Some(&'-') {
            chars.next();
            for c in chars.by_ref() {
                if c == '\n' {
                    break;
                }
            }
            out.push(' ');
        } else if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(c) = chars.next() {
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
            out.push(' ');
        } else if ch.is_ascii_digit() {
            out.push('?');
            while chars
                .peek()
                .is_some_and(|c| c.is_ascii_digit() || *c == '.')
            {
                chars.next();
            }
        } else {
            out.push(ch);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}
pub fn pool(db: &DatabaseConnection) -> Value {
    macro_rules! view {($p:expr)=>{{let p=$p;json!({"size":p.size(),"idle":p.num_idle(),"active":u64::from(p.size()).saturating_sub(p.num_idle() as u64),"max":p.options().get_max_connections(),"min":p.options().get_min_connections(),"closed":p.is_closed()})}};}
    match db.get_database_backend() {
        DbBackend::Sqlite => view!(db.get_sqlite_connection_pool()),
        DbBackend::Postgres => view!(db.get_postgres_connection_pool()),
        DbBackend::MySql => view!(db.get_mysql_connection_pool()),
        _ => Value::Null,
    }
}

async fn table_metadata(db: &DatabaseConnection) -> Result<Vec<Value>> {
    let sql = match db.get_database_backend() {
        DbBackend::Postgres => {
            "SELECT c.relname AS name, CASE WHEN c.relkind = 'v' THEN 'view' ELSE 'table' END AS kind, CASE WHEN c.reltuples >= 0 AND c.relkind <> 'v' THEN c.reltuples::bigint END AS estimated_rows, CASE WHEN c.relkind <> 'v' THEN pg_total_relation_size(c.oid)::bigint END AS bytes, CASE WHEN c.relkind <> 'v' THEN pg_table_size(c.oid)::bigint END AS data_bytes, CASE WHEN c.relkind <> 'v' THEN pg_indexes_size(c.oid)::bigint END AS index_bytes FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = current_schema() AND c.relkind IN ('r','p','v','m') ORDER BY c.relname"
        }
        DbBackend::MySql => {
            "SELECT TABLE_NAME AS name, CASE WHEN TABLE_TYPE = 'VIEW' THEN 'view' ELSE 'table' END AS kind, TABLE_ROWS AS estimated_rows, DATA_LENGTH + INDEX_LENGTH AS bytes, DATA_LENGTH AS data_bytes, INDEX_LENGTH AS index_bytes FROM information_schema.TABLES WHERE TABLE_SCHEMA = DATABASE() ORDER BY TABLE_NAME"
        }
        DbBackend::Sqlite => {
            "SELECT m.name AS name, m.type AS kind, NULL AS estimated_rows, SUM(d.pgsize) AS bytes, SUM(CASE WHEN d.name = m.name THEN d.pgsize ELSE 0 END) AS data_bytes, SUM(CASE WHEN d.name <> m.name THEN d.pgsize ELSE 0 END) AS index_bytes FROM sqlite_schema m LEFT JOIN sqlite_schema owned ON owned.tbl_name = m.name AND owned.type IN ('table','index') LEFT JOIN dbstat d ON d.name = owned.name WHERE m.type IN ('table','view') AND m.name NOT LIKE 'sqlite_%' GROUP BY m.name, m.type ORDER BY m.name"
        }
        _ => return Err(bad_query()),
    };
    let mut rows = match query(db, sql, vec![]).await {
        Ok(rows) => rows,
        // dbstat is an optional SQLite build feature. Keep the catalog usable
        // when unavailable; no table scans or invented zero-byte values.
        Err(_) if db.get_database_backend() == DbBackend::Sqlite => query(db, "SELECT name, type AS kind FROM sqlite_schema WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name", vec![]).await?,
        Err(error) => return Err(error),
    };
    for row in &mut rows {
        if row["kind"] == "view" {
            for key in ["bytes", "data_bytes", "index_bytes", "estimated_rows"] {
                row[key] = Value::Null;
            }
            row["size_status"] = json!("not_applicable");
        } else {
            row["size_status"] = json!(if row["bytes"].is_number() {
                "available"
            } else {
                "unavailable"
            });
        }
    }
    Ok(rows)
}

fn table_page(mut rows: Vec<Value>, raw: &HashMap<String, String>) -> Result<Value> {
    use crate::resources::resource::{ListQuery, Spec};
    let query = ListQuery::parse(
        &Spec {
            search: &["name"],
            filters: &["kind"],
            sortable: &[
                "name",
                "kind",
                "estimated_rows",
                "bytes",
                "data_bytes",
                "index_bytes",
            ],
            default_sort: "name",
        },
        raw,
    )?;
    if query
        .search
        .as_ref()
        .is_some_and(|q| q.chars().count() > 200)
        || query.filters.iter().any(|(_, kinds)| {
            kinds
                .iter()
                .any(|kind| !matches!(kind.as_str(), "table" | "view"))
        })
    {
        return Err(bad_query());
    }
    let needle = query.search.as_ref().map(|q| q.to_lowercase());
    rows.retain(|row| {
        needle.as_ref().is_none_or(|q| {
            row["name"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .contains(q)
        }) && query
            .filters
            .iter()
            .all(|(_, kinds)| kinds.iter().any(|kind| row["kind"].as_str() == Some(kind)))
    });
    rows.sort_by(|a, b| {
        let left = &a[&query.sort];
        let right = &b[&query.sort];
        let order = match (left.is_null(), right.is_null()) {
            (true, false) => return std::cmp::Ordering::Greater,
            (false, true) => return std::cmp::Ordering::Less,
            _ => match (left.as_u64(), right.as_u64()) {
                (Some(left), Some(right)) => left.cmp(&right),
                _ => left.as_str().cmp(&right.as_str()),
            },
        };
        (if query.descending {
            order.reverse()
        } else {
            order
        })
        .then_with(|| a["name"].as_str().cmp(&b["name"].as_str()))
    });
    let total = rows.len();
    let offset = query
        .page
        .saturating_sub(1)
        .saturating_mul(query.page_size)
        .min(total as u64) as usize;
    let items = rows
        .into_iter()
        .skip(offset)
        .take(query.page_size as usize)
        .collect::<Vec<_>>();
    Ok(json!({"items":items,"total":total,"page":query.page,"page_size":query.page_size}))
}

pub async fn tables(
    State(s): State<AppState>,
    _: Admin,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    // Validate before querying system catalogs, including on empty databases.
    table_page(vec![], &raw)?;
    Ok(Json(table_page(table_metadata(&s.db).await?, &raw)?))
}

async fn waits(db: &DatabaseConnection) -> Value {
    if db.get_database_backend() != DbBackend::Postgres {
        return json!({"status":"unsupported"});
    }
    match query(db, "SELECT COALESCE(state, 'unknown') AS state, COALESCE(wait_event_type, '') AS wait_type, COALESCE(wait_event, '') AS wait_event, COUNT(*)::bigint AS sessions, COUNT(*) FILTER (WHERE cardinality(pg_blocking_pids(pid)) > 0)::bigint AS blocked, COALESCE(MAX(EXTRACT(EPOCH FROM (CURRENT_TIMESTAMP - xact_start))),0)::double precision AS oldest_transaction_seconds FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid() GROUP BY state, wait_event_type, wait_event ORDER BY COUNT(*) DESC", vec![]).await {
        Ok(items) => json!({"status":"available","items":items}),
        Err(_) => json!({"status":"unavailable"}),
    }
}
pub async fn overview(State(s): State<AppState>, _: Admin) -> Result<Json<Value>> {
    let backend = s.db.get_database_backend();
    let (kind, instance_sql, tables_sql) = match backend {
        DbBackend::Sqlite => (
            "SQLite",
            "SELECT sqlite_version() AS version",
            "SELECT name AS name, type AS kind FROM sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name",
        ),
        DbBackend::Postgres => (
            "PostgreSQL",
            "SELECT version() AS version, current_database() AS database, inet_server_addr()::text AS host, inet_server_port()::bigint AS port, pg_database_size(current_database()) AS bytes",
            "SELECT tablename AS name, 'table' AS kind FROM pg_tables WHERE schemaname = current_schema() ORDER BY tablename",
        ),
        DbBackend::MySql => (
            "MySQL",
            "SELECT VERSION() AS version, DATABASE() AS `database`, @@hostname AS host, @@port AS port",
            "SELECT TABLE_NAME AS name, TABLE_TYPE AS kind, TABLE_ROWS AS estimated_rows, DATA_LENGTH + INDEX_LENGTH AS bytes FROM information_schema.TABLES WHERE TABLE_SCHEMA = DATABASE() ORDER BY TABLE_NAME",
        ),
        _ => return Err(bad_query()),
    };
    let instance = query(&s.db, instance_sql, vec![]).await?;
    let tables = query(&s.db, tables_sql, vec![]).await?;
    let waits = waits(&s.db).await;
    let mut size = Value::Null;
    if backend == DbBackend::Sqlite {
        let pages = query(&s.db, "PRAGMA page_count", vec![]).await?;
        let page_size = query(&s.db, "PRAGMA page_size", vec![]).await?;
        let count = pages
            .first()
            .and_then(|r| r["page_count"].as_i64())
            .unwrap_or(0);
        let bytes = page_size
            .first()
            .and_then(|r| r["page_size"].as_i64())
            .unwrap_or(0);
        size = json!(count * bytes);
    }
    let nodes=crate::entities::worker_ids::Entity::find().all(&s.db).await?.into_iter().filter_map(|r|{
        let info=serde_json::from_str::<Value>(r.info.as_deref()?).ok()?;
        Some(json!({"instance_id":r.instance_id,"heartbeat_at":r.heartbeat_at,"pool":info["database_pool"],"sql":info["sql"],"online":r.heartbeat_at>db::now()-60_000}))
    }).collect::<Vec<_>>();
    let snapshot = s.monitor.sql.lock().map(|m| m.clone()).unwrap_or_default();
    let mut sql=snapshot.iter().map(|(template,stat)|json!({"template":template,"count":stat.count,"errors":stat.failed,"slow":stat.slow,"total_ms":stat.total_ms,"max_ms":stat.max_ms})).collect::<Vec<_>>();
    sql.sort_by_key(|v| std::cmp::Reverse(v["max_ms"].as_u64().unwrap_or(0)));
    Ok(Json(
        json!({"instance_id":s.instance_id,"backend":kind,"instance":instance.first(),"sqlite_bytes":size,"pool":pool(&s.db),"tables":tables,"waits":waits,"sql":sql,"nodes":nodes,"sql_template_limit":200,"sql_skipped":s.monitor.sql_skipped.load(Ordering::Relaxed),"sql_reset_at":s.monitor.sql_reset_at.load(Ordering::Relaxed),"slow_threshold_ms":1000,"sampled_at":db::now()}),
    ))
}
#[derive(Deserialize)]
pub struct ResetSql {
    instance_id: String,
}
pub async fn reset_sql(
    State(s): State<AppState>,
    admin: Admin,
    Json(body): Json<ResetSql>,
) -> Result<Json<Value>> {
    if body.instance_id != s.instance_id {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("monitor.nodeChanged"),
        ));
    }
    crate::accounts::platform::audit(
        &s,
        &s.db,
        &admin.0.user,
        "reset",
        "sql-statistics",
        crate::snowflake::Id::from(0),
        json!({"scope":"current-node","instance_id":s.instance_id}),
    )
    .await?;
    let _report = s.monitor.sql_report.lock().await;
    use crate::entities::worker_ids;
    use sea_orm::sea_query::Expr;
    if let Some(node) = worker_ids::Entity::find()
        .filter(worker_ids::Column::InstanceId.eq(&s.instance_id))
        .one(&s.db)
        .await?
    {
        let mut info = node
            .info
            .as_deref()
            .and_then(|v| serde_json::from_str::<Value>(v).ok())
            .unwrap_or_else(|| json!({}));
        info["sql"] = json!([]);
        worker_ids::Entity::update_many()
            .col_expr(worker_ids::Column::Info, Expr::value(info.to_string()))
            .filter(worker_ids::Column::InstanceId.eq(&s.instance_id))
            .exec(&s.db)
            .await?;
    }
    let at = db::now();
    let old = {
        let mut rows = s.monitor.sql.lock().map_err(|_| bad_query())?;
        s.monitor.sql_reset_at.store(at, Ordering::Relaxed);
        std::mem::take(&mut *rows)
    };
    drop(old);
    s.monitor.sql_skipped.store(0, Ordering::Relaxed);
    Ok(Json(json!({"scope":"current-node","reset_at":at})))
}
pub async fn table(
    State(s): State<AppState>,
    _: Admin,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let name = raw.get("name").ok_or_else(bad_query)?;
    // Only metadata and non-sensitive operational rows are available. Secret
    // stores, sessions, variables and arbitrary SQL are not a web console.
    let allowed = [
        "worker_ids",
        "workspaces",
        "pipeline_runs",
        "workflow_executions",
        "repository_analysis_jobs",
        "platform_observations",
    ];
    if !allowed.contains(&name.as_str()) {
        return Err(bad_query());
    }
    let columns: &[&str] = match name.as_str() {
        "worker_ids" => &["worker_id", "instance_id", "heartbeat_at"],
        "workspaces" => &["id", "name", "slug", "created_at"],
        "pipeline_runs" => &["id", "workspace_id", "pipeline_id", "state", "created_at"],
        "workflow_executions" => &["id", "workspace_id", "workflow_id", "state", "created_at"],
        "repository_analysis_jobs" => &["id", "workspace_id", "state", "created_at"],
        _ => &[
            "at",
            "instance_id",
            "kind",
            "method",
            "route",
            "status",
            "elapsed_ms",
        ],
    };
    let mut select = SqlQuery::select();
    select.from(Alias::new(name)).limit(100);
    for col in columns {
        select.expr_as(Expr::col(Alias::new(*col)), Alias::new(*col));
    }
    let rows = s.db.query_all(&select).await?;
    let items = rows
        .into_iter()
        .map(|r| {
            let mut item = serde_json::Map::new();
            for col in columns {
                item.insert(
                    (*col).into(),
                    r.try_get::<i64>("", col)
                        .map(|v| json!(v.to_string()))
                        .or_else(|_| r.try_get::<String>("", col).map(|v| json!(v)))
                        .unwrap_or(Value::Null),
                );
            }
            Value::Object(item)
        })
        .collect::<Vec<_>>();
    Ok(Json(
        json!({"name":name,"columns":columns,"items":items,"limit":100,"readonly":true}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn sqlite_table_storage_includes_indexes_without_counting_rows() {
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        for sql in [
            "CREATE TABLE sample(id INTEGER PRIMARY KEY, payload TEXT)",
            "CREATE INDEX sample_payload ON sample(payload)",
            "INSERT INTO sample(payload) VALUES ('hello')",
            "CREATE VIEW sample_view AS SELECT id FROM sample",
        ] {
            db.execute_unprepared(sql).await.unwrap();
        }
        let rows = table_metadata(&db).await.unwrap();
        let table = rows.iter().find(|row| row["name"] == "sample").unwrap();
        assert_eq!(
            table["size_status"], "available",
            "bundled SQLite must expose dbstat sizes"
        );
        assert!(table["data_bytes"].as_u64().unwrap() > 0);
        assert!(table["index_bytes"].as_u64().unwrap() > 0);
        assert_eq!(
            table["bytes"].as_u64().unwrap(),
            table["data_bytes"].as_u64().unwrap() + table["index_bytes"].as_u64().unwrap()
        );
        assert!(table["estimated_rows"].is_null());
        let view = rows
            .iter()
            .find(|row| row["name"] == "sample_view")
            .unwrap();
        assert_eq!(view["size_status"], "not_applicable");
        assert!(view["bytes"].is_null());
        assert_eq!(waits(&db).await["status"], "unsupported");
    }

    #[test]
    fn catalog_pages_sort_numbers_filter_and_reject_unknown_fields() {
        let rows = vec![
            json!({"name":"large","kind":"table","bytes":100}),
            json!({"name":"small","kind":"table","bytes":9}),
            json!({"name":"view","kind":"view","bytes":null}),
        ];
        let raw = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<HashMap<_, _>>()
        };
        let page = table_page(rows.clone(), &raw(&[("sort", "bytes")])).unwrap();
        assert_eq!(page["items"][0]["name"], "small");
        let page = table_page(
            rows.clone(),
            &raw(&[
                ("sort", "-bytes"),
                ("page_size", "1"),
                ("page", "2"),
                ("kind", "table"),
            ]),
        )
        .unwrap();
        assert_eq!(page["total"], 2);
        assert_eq!(page["items"][0]["name"], "small");
        let page = table_page(rows.clone(), &raw(&[("q", "LARGE")])).unwrap();
        assert_eq!(page["total"], 1);
        assert_eq!(page["items"][0]["name"], "large");
        for params in [
            raw(&[("sort", "password")]),
            raw(&[("kind", "secret")]),
            raw(&[("page", "0")]),
            raw(&[("workspace_id", "1")]),
            raw(&[("q", &"x".repeat(201))]),
        ] {
            assert!(table_page(rows.clone(), &params).is_err());
        }
        assert_eq!(
            table_page(rows, &raw(&[("page", "18446744073709551615")])).unwrap()["items"],
            json!([])
        );
    }
    #[test]
    fn sql_templates_never_keep_literals_comments_or_parameters() {
        let value = fingerprint(
            "SELECT * FROM users WHERE password='secret''value' AND id=123 -- token=private\n /* hidden */ LIMIT 20",
        );
        assert!(!value.contains("secret"));
        assert!(!value.contains("private"));
        assert!(!value.contains("hidden"));
        assert!(!value.contains("123"));
        assert!(value.contains("password=?"));
    }
    #[test]
    fn statistics_are_bounded_and_do_not_block_on_contended_lock() {
        let store = Store::default();
        for n in 0..300 {
            let sql = format!(
                "SELECT column{}{} FROM table_name",
                (b'a' + (n / 26) as u8) as char,
                (b'a' + (n % 26) as u8) as char
            );
            let statement = Statement::from_string(DbBackend::Sqlite, sql);
            store.record(&sea_orm::metric::Info {
                statement: &statement,
                elapsed: std::time::Duration::from_millis(1100),
                failed: false,
            });
        }
        assert_eq!(store.sql.lock().unwrap().len(), 200);
        assert_eq!(store.sql_skipped.load(Ordering::Relaxed), 100);
        let held = store.sql.lock().unwrap();
        let statement = Statement::from_string(DbBackend::Sqlite, "SELECT blocked".to_owned());
        store.record(&sea_orm::metric::Info {
            statement: &statement,
            elapsed: std::time::Duration::ZERO,
            failed: false,
        });
        assert_eq!(store.sql_skipped.load(Ordering::Relaxed), 101);
        drop(held);
        assert!(fingerprint(&format!("SELECT '{}'", "s".repeat(1_000_000))).len() < 1505);
    }
}
