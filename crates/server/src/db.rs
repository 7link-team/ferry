//! Shared UTC connection options and database clock.
/// Enforce UTC for every pooled connection, including replacements. SQLite stores
/// epoch values and has no session timezone; its filename/WAL options stay intact.
pub fn utc_options(mut options: sea_orm::ConnectOptions) -> sea_orm::ConnectOptions {
    options.map_sqlx_postgres_opts(|opts| opts.options([("timezone", "UTC")]));
    options.map_sqlx_mysql_opts(|opts| opts.timezone(Some("+00:00".into())));
    options
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Evaluate UTC milliseconds when SQL executes, after connection/row-lock waits.
pub(crate) fn now_expr(backend: sea_orm::DbBackend) -> sea_orm::sea_query::SimpleExpr {
    use sea_orm::{DbBackend, sea_query::Expr};
    Expr::cust(match backend {
        DbBackend::Sqlite => {
            "(CAST(strftime('%s','now') AS INTEGER) * 1000 + CAST(substr(strftime('%f','now'),4,3) AS INTEGER))"
        }
        DbBackend::Postgres => {
            "CAST(FLOOR(EXTRACT(EPOCH FROM clock_timestamp()) * 1000) AS BIGINT)"
        }
        DbBackend::MySql => "CAST(UNIX_TIMESTAMP(SYSDATE(3)) * 1000 AS SIGNED)",
        _ => "NULL", // Unknown backends cannot authorize an execution write.
    })
}
