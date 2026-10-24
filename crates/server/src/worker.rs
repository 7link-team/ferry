//! Worker slots are allocated under a transactional database lock. A lease is
//! refreshed only by an owner-matched database write, never just by a timer.
use crate::{
    lock,
    snowflake::{Generator, TTL},
};
use sea_orm::{
    ActiveValue::Set, IntoActiveModel, QueryOrder, TransactionTrait, entity::prelude::*,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use crate::entities::worker_ids::{ActiveModel, Column, Entity, Model};

pub struct Worker {
    pub generator: Arc<Generator>,
    id: i64,
    instance: String,
}
impl Worker {
    pub async fn allocate(db: &DatabaseConnection, instance: &str) -> anyhow::Result<Self> {
        let started = Instant::now();
        let now = crate::db::now();
        let tx = db.begin().await?;
        // The row write lock lives until commit, even if allocation exceeds its
        // TTL. Any error/cancellation drops and rolls back this transaction.
        anyhow::ensure!(
            lock::acquire(&tx, "worker-id", instance, now, TTL.as_millis() as i64).await?,
            "worker-id allocation lock is busy; refusing startup"
        );
        let rows = Entity::find()
            .order_by_asc(Column::WorkerId)
            .all(&tx)
            .await?;
        let id = rows
            .iter()
            .find(|r| r.instance_id == instance)
            .map(|r| r.worker_id)
            .or_else(|| {
                (0..1024).find(|id| {
                    rows.iter().all(|r| {
                        r.worker_id != *id || r.heartbeat_at < now - TTL.as_millis() as i64
                    })
                })
            })
            .ok_or_else(|| {
                anyhow::anyhow!("all 1024 worker id slots are occupied; refusing startup")
            })?;
        anyhow::ensure!((0..1024).contains(&id), "invalid stored worker id");
        let model = Model {
            worker_id: id,
            instance_id: instance.into(),
            heartbeat_at: now,
        };
        if let Some(previous) = rows.iter().find(|r| r.worker_id == id) {
            let changed = Entity::update_many()
                .set(ActiveModel {
                    instance_id: Set(instance.into()),
                    heartbeat_at: Set(now),
                    ..Default::default()
                })
                .filter(Column::WorkerId.eq(id))
                .filter(Column::InstanceId.eq(&previous.instance_id))
                .filter(Column::HeartbeatAt.eq(previous.heartbeat_at))
                .exec(&tx)
                .await?
                .rows_affected;
            // A heartbeat may have renewed an expired candidate after the
            // SELECT on MySQL/Postgres. Never steal that freshly renewed lease.
            anyhow::ensure!(
                changed == 1,
                "worker slot renewed during allocation; refusing startup"
            );
        } else {
            Entity::insert(model.into_active_model()).exec(&tx).await?;
        }
        // Verify ownership and renew before completing the allocation.
        anyhow::ensure!(
            lock::renew(
                &tx,
                "worker-id",
                instance,
                crate::db::now(),
                TTL.as_millis() as i64
            )
            .await?
                == 1,
            "lost worker-id allocation lock"
        );
        lock::release(&tx, "worker-id", instance).await?;
        tx.commit().await?;
        let generator = Arc::new(Generator::new(id as u16, started)?);
        // Also refuse startup when a database operation took longer than TTL.
        generator.next_id()?;
        Ok(Self {
            generator,
            id,
            instance: instance.into(),
        })
    }
    async fn heartbeat(&self, db: &DatabaseConnection) -> anyhow::Result<()> {
        let started = Instant::now();
        let changed = Entity::update_many()
            .col_expr(Column::HeartbeatAt, Expr::value(crate::db::now()))
            .filter(Column::WorkerId.eq(self.id))
            .filter(Column::InstanceId.eq(&self.instance))
            .exec(db)
            .await?
            .rows_affected;
        if changed != 1 {
            self.generator.stop();
            anyhow::bail!("worker slot was reclaimed; refusing to issue IDs");
        }
        // Start time, not completion time: a slow DB response cannot extend
        // the local lease beyond the heartbeat actually stored in the DB.
        self.generator.heartbeat(started);
        Ok(())
    }
    pub async fn release(&self, db: &DatabaseConnection) -> anyhow::Result<()> {
        self.generator.stop();
        // Retain identity for restart affinity, but make the slot reclaimable.
        Entity::update_many()
            .col_expr(Column::HeartbeatAt, Expr::value(0_i64))
            .filter(Column::WorkerId.eq(self.id))
            .filter(Column::InstanceId.eq(&self.instance))
            .exec(db)
            .await?;
        Ok(())
    }
    pub async fn run(&self, db: &DatabaseConnection, mut stop: tokio::sync::watch::Receiver<bool>) {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                _ = stop.changed() => break,
                _ = interval.tick() => if let Err(error) = self.heartbeat(db).await { tracing::error!(%error, "worker heartbeat failed"); }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::tests::database;
    #[tokio::test]
    async fn allocation_restart_reclaim_and_release() {
        let (_dir, db) = database().await;
        let a = Worker::allocate(&db, "a").await.unwrap();
        let b = Worker::allocate(&db, "b").await.unwrap();
        assert_eq!(a.id, 0);
        assert_eq!(b.id, 1);
        let restarted = Worker::allocate(&db, "a").await.unwrap();
        assert_eq!(restarted.id, a.id);
        a.generator
            .heartbeat(Instant::now() - TTL - Duration::from_secs(1));
        assert!(a.generator.next_id().is_err());
        a.heartbeat(&db).await.unwrap();
        assert!(a.generator.next_id().is_ok());
        Entity::update_many()
            .col_expr(
                Column::HeartbeatAt,
                Expr::value(crate::db::now() - TTL.as_millis() as i64 - 1),
            )
            .filter(Column::WorkerId.eq(a.id))
            .exec(&db)
            .await
            .unwrap();
        let c = Worker::allocate(&db, "c").await.unwrap();
        assert_eq!(c.id, a.id);
        assert!(a.heartbeat(&db).await.is_err());
        assert!(a.generator.next_id().is_err());
        a.release(&db).await.unwrap();
        assert_ne!(
            Entity::find_by_id(c.id)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .heartbeat_at,
            0
        );
        c.release(&db).await.unwrap();
        assert!(c.generator.next_id().is_err());
        let c = Worker::allocate(&db, "c").await.unwrap();
        assert_eq!(c.id, 0);
        c.release(&db).await.unwrap();
        assert_eq!(Worker::allocate(&db, "d").await.unwrap().id, 0);
    }
    #[tokio::test]
    async fn exhaustion_rolls_back_allocation_lock() {
        let (_dir, db) = database().await;
        let now = crate::db::now();
        // Chunk to stay below SQLite's bind parameter limit.
        for batch in 0..8 {
            Entity::insert_many((batch * 128..(batch + 1) * 128).map(|id| {
                Model {
                    worker_id: id,
                    instance_id: format!("instance-{id}"),
                    heartbeat_at: now,
                }
                .into_active_model()
            }))
            .exec(&db)
            .await
            .unwrap();
        }
        let error = Worker::allocate(&db, "overflow").await.err().unwrap();
        assert!(error.to_string().contains("1024"));
        assert!(
            crate::entities::locks::Entity::find_by_id("worker-id")
                .one(&db)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(Worker::allocate(&db, "instance-42").await.unwrap().id, 42);
    }
    #[tokio::test]
    async fn concurrent_allocations_and_background_shutdown() {
        let (_dir, db) = database().await;
        let (a, b) = tokio::join!(Worker::allocate(&db, "a"), Worker::allocate(&db, "b"));
        let a = a.unwrap();
        let b = b.unwrap();
        assert_ne!(a.id, b.id);
        a.generator
            .heartbeat(Instant::now() - TTL - Duration::from_secs(1));
        let (send, receive) = tokio::sync::watch::channel(false);
        let worker = Arc::new(a);
        let task_worker = worker.clone();
        let task_db = db.clone();
        let task = tokio::spawn(async move {
            task_worker.run(&task_db, receive).await;
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while worker.generator.next_id().is_err() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        send.send(true).unwrap();
        task.await.unwrap();
        worker.release(&db).await.unwrap();
        assert!(worker.generator.next_id().is_err());
    }
}
