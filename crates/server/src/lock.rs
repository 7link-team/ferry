//! Portable UPDATE-then-INSERT locks. Callers release explicitly, or roll back
//! the enclosing transaction on error (worker allocation uses that path).
use sea_orm::{ActiveValue::Set, Condition, IntoActiveModel, QueryFilter, entity::prelude::*};

use crate::entities::locks::{ActiveModel, Column, Entity, Model};

pub async fn acquire(
    db: &impl ConnectionTrait,
    name: &str,
    owner: &str,
    now: i64,
    ttl: i64,
) -> Result<bool, DbErr> {
    if ttl <= 0 {
        return Err(DbErr::Custom("lock TTL must be positive".into()));
    }
    let expires = now
        .checked_add(ttl)
        .ok_or_else(|| DbErr::Custom("lock expiry overflow".into()))?;
    let changed = Entity::update_many()
        .col_expr(Column::Owner, Expr::value(owner))
        .col_expr(Column::AcquiredAt, Expr::value(now))
        .col_expr(Column::ExpiresAt, Expr::value(expires))
        .filter(Column::Name.eq(name))
        .filter(
            Condition::any()
                .add(Column::Owner.eq(owner))
                .add(Column::ExpiresAt.lt(now)),
        )
        .exec(db)
        .await?
        .rows_affected;
    if changed == 1 {
        return Ok(true);
    }
    let result = Entity::insert(
        Model {
            name: name.into(),
            owner: owner.into(),
            acquired_at: now,
            expires_at: expires,
        }
        .into_active_model(),
    )
    .exec(db)
    .await;
    match result {
        Ok(_) => Ok(true),
        Err(e)
            if matches!(
                e.sql_err(),
                Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
            ) =>
        {
            Ok(false)
        }
        Err(e) => Err(e),
    }
}
pub async fn release(db: &impl ConnectionTrait, name: &str, owner: &str) -> Result<u64, DbErr> {
    Ok(Entity::delete_many()
        .filter(Column::Name.eq(name))
        .filter(Column::Owner.eq(owner))
        .exec(db)
        .await?
        .rows_affected)
}
pub async fn renew(
    db: &impl ConnectionTrait,
    name: &str,
    owner: &str,
    now: i64,
    ttl: i64,
) -> Result<u64, DbErr> {
    if ttl <= 0 {
        return Err(DbErr::Custom("lock TTL must be positive".into()));
    }
    let expires = now
        .checked_add(ttl)
        .ok_or_else(|| DbErr::Custom("lock expiry overflow".into()))?;
    Ok(Entity::update_many()
        .set(ActiveModel {
            expires_at: Set(expires),
            ..Default::default()
        })
        .filter(Column::Name.eq(name))
        .filter(Column::Owner.eq(owner))
        .exec(db)
        .await?
        .rows_affected)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use sea_orm::{Database, DatabaseConnection};
    use sea_orm_migration::MigratorTrait;
    pub async fn database() -> (tempfile::TempDir, DatabaseConnection) {
        let dir = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("test.sqlite").display()
        );
        let db = Database::connect(url).await.unwrap();
        crate::migration::Migrator::up(&db, None).await.unwrap();
        (dir, db)
    }
    #[tokio::test]
    async fn concurrent_acquisition_has_one_winner() {
        let (_dir, db) = database().await;
        let (a, b) = tokio::join!(
            acquire(&db, "test", "a", 1000, 100),
            acquire(&db, "test", "b", 1000, 100)
        );
        assert_ne!(a.unwrap(), b.unwrap());
    }
    #[tokio::test]
    async fn non_owner_cannot_release_or_renew() {
        let (_dir, db) = database().await;
        assert!(acquire(&db, "test", "a", 1000, 100).await.unwrap());
        assert_eq!(release(&db, "test", "b").await.unwrap(), 0);
        assert_eq!(renew(&db, "test", "b", 1000, 500).await.unwrap(), 0);
        let row = Entity::find_by_id("test").one(&db).await.unwrap().unwrap();
        assert_eq!(row.owner, "a");
        assert_eq!(row.expires_at, 1100);
        assert!(!acquire(&db, "test", "b", 1100, 100).await.unwrap());
        assert_eq!(renew(&db, "test", "a", 1050, 100).await.unwrap(), 1);
        assert!(!acquire(&db, "test", "b", 1101, 100).await.unwrap());
        assert!(acquire(&db, "test", "b", 1151, 100).await.unwrap());
        assert_eq!(release(&db, "test", "a").await.unwrap(), 0);
        assert_eq!(release(&db, "test", "b").await.unwrap(), 1);
        assert!(acquire(&db, "test", "a", 1151, 100).await.unwrap());
    }
}
