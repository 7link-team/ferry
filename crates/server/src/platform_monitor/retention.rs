//! Bounded raw monitoring history; deletion uses indexed, small batches.
use super::*;
#[cfg(test)]
use sea_orm::ActiveModelTrait;
use sea_orm::{PaginatorTrait, QueryOrder, QuerySelect};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Limits {
    pub days: u64,
    pub rows: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            days: 30,
            rows: 100_000,
        }
    }
}
impl Limits {
    pub fn read(stored: Option<&str>) -> Result<Self> {
        let value = if let Some(stored) = stored {
            serde_json::from_str(stored).map_err(|_| bad_query())?
        } else {
            Self {
                days: std::env::var("FERRY_MONITOR_RETENTION_DAYS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(30)
                    .clamp(1, 365),
                ..Self::default()
            }
        };
        let value: Self = value;
        if !(1..=365).contains(&value.days) || !(1000..=1_000_000).contains(&value.rows) {
            return Err(bad_query());
        }
        Ok(value)
    }
    pub fn update(stored: Option<&str>, patch: &serde_json::Map<String, Value>) -> Result<Self> {
        let mut value = Self::read(stored)?;
        if let Some(days) = patch.get("monitor_retention_days") {
            value.days = days.as_u64().ok_or_else(bad_query)?;
        }
        if let Some(rows) = patch.get("monitor_max_rows") {
            value.rows = rows.as_u64().ok_or_else(bad_query)?;
        }
        Self::read(Some(
            &serde_json::to_string(&value).map_err(std::io::Error::other)?,
        ))
    }
    pub fn view(&self) -> Value {
        json!({"monitor_retention_days":self.days,"monitor_max_rows":self.rows})
    }
}
pub async fn count(db: &DatabaseConnection, instance: &str) -> Result<u64> {
    Ok(events::Entity::find()
        .filter(events::Column::InstanceId.eq(instance))
        .count(db)
        .await?)
}
pub async fn trim(
    db: &DatabaseConnection,
    instance: Option<&str>,
    before: Option<i64>,
    limit: u64,
) -> Result<u64> {
    let mut query = events::Entity::find()
        .select_only()
        .column(events::Column::Id);
    if let Some(instance) = instance {
        query = query.filter(events::Column::InstanceId.eq(instance));
    }
    if let Some(before) = before {
        query = query.filter(events::Column::At.lt(before));
    }
    let ids = query
        .order_by_asc(events::Column::Id)
        .limit(limit.min(1000))
        .into_tuple::<i64>()
        .all(db)
        .await?;
    if ids.is_empty() {
        return Ok(0);
    }
    Ok(events::Entity::delete_many()
        .filter(events::Column::Id.is_in(ids))
        .exec(db)
        .await?
        .rows_affected)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_reject_unbounded_or_invalid_values() {
        assert_eq!(Limits::default().days, 30);
        assert!(Limits::read(Some(r#"{"days":0,"rows":1000}"#)).is_err());
        assert!(Limits::read(Some(r#"{"days":366,"rows":1000}"#)).is_err());
        assert!(Limits::read(Some(r#"{"days":30,"rows":1000001}"#)).is_err());
        assert!(Limits::read(Some(r#"{"days":365,"rows":1000}"#)).is_ok());
    }
    #[tokio::test]
    async fn history_eviction_is_bounded_and_isolates_instances() {
        let (_dir, db) = crate::lock::tests::database().await;
        for (id, instance, at) in [(1, "a", 1), (2, "b", 2), (3, "a", 3), (4, "a", 4)] {
            events::ActiveModel {
                id: Set(id),
                at: Set(at),
                event_day: Set(0),
                user_id: Set(0),
                instance_id: Set(instance.into()),
                kind: Set("api".into()),
                method: Set("GET".into()),
                route: Set("/api/example".into()),
                file_path: Set(String::new()),
                status: Set(200),
                elapsed_ms: Set(0),
                response_bytes: Set(0),
            }
            .insert(&db)
            .await
            .unwrap();
        }
        assert_eq!(trim(&db, Some("a"), None, 1).await.unwrap(), 1);
        assert!(
            events::Entity::find_by_id(1)
                .one(&db)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(count(&db, "b").await.unwrap(), 1);
        assert_eq!(trim(&db, None, Some(4), 1).await.unwrap(), 1);
        assert!(
            events::Entity::find_by_id(2)
                .one(&db)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(count(&db, "a").await.unwrap(), 2);
    }
}
