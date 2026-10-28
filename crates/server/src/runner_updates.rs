//! Host reservations keep agent upgrades outside complete tasks, including gaps
//! between Actions jobs and between individual command/file sessions.

use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, LazyLock, Mutex, Weak},
};
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

static HOSTS: LazyLock<Mutex<HashMap<String, Weak<RwLock<()>>>>> = LazyLock::new(Default::default);
pub(crate) type Lease = Arc<OwnedRwLockReadGuard<()>>;
tokio::task_local! { static RESERVED: HashMap<String, Lease>; }

fn host_lock(host: &str) -> Arc<RwLock<()>> {
    let mut hosts = HOSTS.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(existing) = hosts.get(host).and_then(Weak::upgrade) {
        return existing;
    }
    hosts.retain(|_, lock| lock.strong_count() > 0);
    let lock = Arc::new(RwLock::new(()));
    hosts.insert(host.to_owned(), Arc::downgrade(&lock));
    lock
}

pub(crate) async fn lease(host: &str) -> Lease {
    if let Ok(Some(existing)) = RESERVED.try_with(|hosts| hosts.get(host).cloned()) {
        return existing;
    }
    Arc::new(host_lock(host).read_owned().await)
}

pub(crate) async fn exclusive(host: &str) -> OwnedRwLockWriteGuard<()> {
    host_lock(host).write_owned().await
}

pub(crate) async fn with_hosts<T>(mut hosts: Vec<String>, task: impl Future<Output = T>) -> T {
    hosts.sort();
    hosts.dedup();
    let mut guards = RESERVED.try_with(Clone::clone).unwrap_or_default();
    // A common acquisition order prevents two multi-host runs deadlocking.
    for host in &hosts {
        guards.insert(host.clone(), lease(host).await);
    }
    RESERVED.scope(guards, task).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn upgrade_waits_for_whole_task_and_blocks_new_tasks() {
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (finish, finishing) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(with_hosts(vec!["fixture-host-1".into()], async move {
            entered.send(()).unwrap();
            finishing.await.unwrap();
            // A later job in this same reserved task must still open channels,
            // even after the upgrade has queued its exclusive reservation.
            let _later_job = lease("fixture-host-1").await;
        }));
        ready.await.unwrap();
        let (requested, request) = tokio::sync::oneshot::channel();
        let (upgraded, upgrade_done) = tokio::sync::oneshot::channel();
        let upgrade = tokio::spawn(async move {
            requested.send(()).unwrap();
            let _exclusive = exclusive("fixture-host-1").await;
            upgrade_done.await.unwrap();
        });
        request.await.unwrap();
        tokio::task::yield_now().await;
        assert!(!upgrade.is_finished());
        let mut new_task = tokio::spawn(async { lease("fixture-host-1").await });
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut new_task)
                .await
                .is_err()
        );
        finish.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut new_task)
                .await
                .is_err()
        );
        upgraded.send(()).unwrap();
        upgrade.await.unwrap();
        drop(
            tokio::time::timeout(Duration::from_secs(2), new_task)
                .await
                .unwrap()
                .unwrap(),
        );
    }

    #[tokio::test]
    async fn different_hosts_do_not_block_each_other() {
        let _exclusive = exclusive("fixture-host-2").await;
        let other = tokio::time::timeout(Duration::from_secs(1), lease("fixture-host-3"))
            .await
            .unwrap();
        drop(other);
    }

    #[tokio::test]
    async fn session_outliving_its_task_keeps_upgrade_blocked() {
        let session = with_hosts(vec!["fixture-detached".into()], async {
            lease("fixture-detached").await
        })
        .await;
        assert!(
            tokio::time::timeout(Duration::from_millis(30), exclusive("fixture-detached"))
                .await
                .is_err()
        );
        drop(session);
        let _update = tokio::time::timeout(Duration::from_secs(1), exclusive("fixture-detached"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cancelling_a_waiting_task_releases_already_reserved_hosts() {
        let update = exclusive("fixture-cancel-b").await;
        let task = tokio::spawn(with_hosts(
            vec!["fixture-cancel-a".into(), "fixture-cancel-b".into()],
            async { panic!("must not start while host b is upgrading") },
        ));
        tokio::task::yield_now().await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let _other_update =
            tokio::time::timeout(Duration::from_secs(1), exclusive("fixture-cancel-a"))
                .await
                .unwrap();
        drop(update);
    }
}
