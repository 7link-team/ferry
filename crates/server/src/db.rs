//! Thin raw-SQL helpers over SeaORM. Every module queries through these so one
//! place decides parameter binding, identifier quoting and row extraction.

use crate::error::Result;
use sea_orm::{ConnectionTrait, ExecResult, QueryResult, Statement, Value};

pub fn statement(db: &impl ConnectionTrait, sql: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(db.get_database_backend(), sql, values)
}
pub async fn execute(db: &impl ConnectionTrait, sql: &str, values: Vec<Value>) -> Result<ExecResult> {
    Ok(db.execute_raw(statement(db, sql, values)).await?)
}
pub async fn one(db: &impl ConnectionTrait, sql: &str, values: Vec<Value>) -> Result<Option<QueryResult>> {
    Ok(db.query_one_raw(statement(db, sql, values)).await?)
}
pub async fn all(db: &impl ConnectionTrait, sql: &str, values: Vec<Value>) -> Result<Vec<QueryResult>> {
    Ok(db.query_all_raw(statement(db, sql, values)).await?)
}
pub fn text(row: &QueryResult, key: &str) -> String {
    row.try_get("", key).expect("validated text schema")
}
pub fn int(row: &QueryResult, key: &str) -> i64 {
    row.try_get("", key)
        .or_else(|_| row.try_get::<u64>("", key).map(|v| v as i64))
        .expect("validated integer schema")
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
