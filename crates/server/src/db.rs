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
