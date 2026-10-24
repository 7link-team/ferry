//! Shared database clock.
pub fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
