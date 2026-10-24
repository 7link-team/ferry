//! Thin raw-SQL helpers over SeaORM. Every module queries through these so one
//! place decides parameter binding, identifier quoting and row extraction.

use crate::error::Result;
use sea_orm::{ConnectionTrait, ExecResult, QueryResult, Statement, Value};

pub fn statement(db: &impl ConnectionTrait, sql: &str, values: Vec<Value>) -> Statement {
    // A hand-written INSERT that gains a column is easy to update in three
    // places and miss the fourth. Without this the symptom is a NOT NULL
    // violation on whichever column fell off the end — which points at the
    // wrong column entirely. debug_assert so it fires in every test and dev
    // run and costs nothing in release.
    debug_assert_eq!(
        crate::dialect::placeholder_count(sql),
        values.len(),
        "SQL has {} placeholders but {} values were bound:\n{sql}",
        crate::dialect::placeholder_count(sql),
        values.len()
    );
    let backend = db.get_database_backend();
    Statement::from_sql_and_values(backend, crate::dialect::query(backend, sql), values)
}
pub async fn execute(
    db: &impl ConnectionTrait,
    sql: &str,
    values: Vec<Value>,
) -> Result<ExecResult> {
    Ok(db.execute_raw(statement(db, sql, values)).await?)
}
pub async fn one(
    db: &impl ConnectionTrait,
    sql: &str,
    values: Vec<Value>,
) -> Result<Option<QueryResult>> {
    Ok(db.query_one_raw(statement(db, sql, values)).await?)
}
pub async fn all(
    db: &impl ConnectionTrait,
    sql: &str,
    values: Vec<Value>,
) -> Result<Vec<QueryResult>> {
    Ok(db.query_all_raw(statement(db, sql, values)).await?)
}
pub fn text(row: &QueryResult, key: &str) -> String {
    row.try_get("", key).expect("validated text schema")
}
/// Integer widths differ per backend: SQLite hands back i64, Postgres gives
/// i32 for INTEGER, and MySQL returns u64 from COUNT(). Try each rather than
/// making every caller care which database it is talking to.
pub fn int(row: &QueryResult, key: &str) -> i64 {
    row.try_get::<i64>("", key)
        .or_else(|_| row.try_get::<i32>("", key).map(i64::from))
        .or_else(|_| row.try_get::<u64>("", key).map(|v| v as i64))
        .or_else(|_| row.try_get::<u32>("", key).map(i64::from))
        .expect("validated integer schema")
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
