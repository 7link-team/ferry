use super::*;

pub(super) use crate::hosts::runner::reservations::wait_for_reservation;

/// Whether a job may still run once its run is cancelled: GitHub evaluates a
/// pending job's `if` with cancelled() true, so only a condition calling
/// always() or cancelled() can pass. The worker evaluates the condition.
pub(super) fn cleanup_job(document: &crate::pipelines::actions::Document, key: &str) -> bool {
    crate::pipelines::actions::job_policies(document, key)
        .and_then(|policies| policies.condition)
        .is_some_and(|condition| {
            let condition = condition
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .to_ascii_lowercase();
            condition.contains("always()") || condition.contains("cancelled()")
        })
}

/// Run all ready jobs concurrently; retain every future until termination and
/// cleanup finish, including when scheduling or persistence encounters errors.
/// After the run is cancelled, ready jobs that `cleanup` admits still start
/// (with `run_cancelled`), as GitHub runs `if: always()` jobs of a cancelled
/// run; the others are skipped so their dependants can be evaluated.
pub(super) async fn schedule_jobs<F, Fut, A, Active, C>(
    jobs: &[Job],
    stop: &mut tokio::sync::watch::Receiver<bool>,
    mut active: A,
    cleanup: C,
    mut execute: F,
) -> Result<BTreeMap<String, Outcome>>
where
    F: FnMut(Job, BTreeMap<String, Outcome>, tokio::sync::watch::Receiver<bool>, bool) -> Fut,
    Fut: std::future::Future<Output = Result<Outcome>>,
    A: FnMut() -> Active,
    Active: std::future::Future<Output = Result<bool>>,
    C: Fn(&Job) -> bool,
{
    let mut completed = BTreeMap::new();
    let mut started = BTreeSet::new();
    let mut running = FuturesUnordered::new();
    // `cancel` stops ordinary jobs; `halt` (service stop or an internal
    // failure) also stops cleanup jobs.
    let (cancel, cancelled) = tokio::sync::watch::channel(false);
    let (halt, halted) = tokio::sync::watch::channel(false);
    let mut run_cancelled = false;
    let mut failure = None;
    let mut poll = tokio::time::interval(Duration::from_millis(100));
    // No jobs exist yet, so this initial check cannot wait for a resource
    // held by an unpolled job future.
    if !active().await? || *stop.borrow() {
        return Ok(completed);
    }
    loop {
        if *stop.borrow() {
            cancel.send_replace(true);
            halt.send_replace(true);
        }
        let mut progressed = true;
        while progressed && !*halted.borrow() {
            progressed = false;
            for job in jobs {
                if started.contains(&job.key)
                    || !job.needs.iter().all(|key| completed.contains_key(key))
                {
                    continue;
                }
                let receiver = if !*cancelled.borrow() {
                    cancelled.clone()
                } else if run_cancelled && cleanup(job) {
                    halted.clone()
                } else if run_cancelled {
                    started.insert(job.key.clone());
                    completed.insert(
                        job.key.clone(),
                        Outcome {
                            state: "skipped".into(),
                            steps: BTreeMap::new(),
                            artifacts: vec![],
                            outputs: BTreeMap::new(),
                            continue_on_error: false,
                            artifact_retention: BTreeMap::new(),
                            plan: Plan::default(),
                        },
                    );
                    progressed = true;
                    continue;
                } else {
                    continue;
                };
                started.insert(job.key.clone());
                let future = execute(
                    job.clone(),
                    completed.clone(),
                    receiver,
                    *cancelled.borrow(),
                );
                let key = job.key.clone();
                running.push(async move { (key, future.await) });
            }
        }
        if running.is_empty() {
            break;
        }
        tokio::select! {
            result = running.next() => {
                if let Some((key, result)) = result {
                    match result {
                        Ok(outcome) => { completed.insert(key, outcome); },
                        Err(error) => {
                            failure.get_or_insert(error);
                            cancel.send_replace(true);
                            halt.send_replace(true);
                        },
                    }
                }
            }
            _ = stop.changed(), if !*halted.borrow() => {
                cancel.send_replace(true);
                halt.send_replace(true);
            }
            // SQLite has one pooled connection by default. A pending job can
            // own that connection, so keep polling jobs while this read waits;
            // awaiting it outside select would deadlock the entire server.
            status = async { poll.tick().await; active().await }, if !*cancelled.borrow() => {
                match status {
                    Ok(true) => {},
                    Ok(false) => { run_cancelled = true; cancel.send_replace(true); },
                    Err(error) => {
                        failure.get_or_insert(error);
                        cancel.send_replace(true);
                        halt.send_replace(true);
                    },
                }
            }
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }
    if !*cancelled.borrow() && completed.len() != jobs.len() {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.dependenciesUnscheduled"),
        ));
    }
    Ok(completed)
}

#[cfg(test)]
mod tests {
    use super::super::tests::test_job;
    use super::*;
    use sea_orm::ConnectionTrait;

    #[tokio::test]
    async fn reservation_holding_only_connection_keeps_progressing_during_status_check() {
        use sea_orm::TransactionTrait;
        let (_dir, db, _) = crate::executions::lease::tests::database().await;
        let (_stop, mut stop) = tokio::sync::watch::channel(false);
        let pending = async {
            let tx = db.begin().await?;
            tokio::time::sleep(Duration::from_millis(350)).await;
            tx.rollback().await?;
            Ok(42)
        };
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            wait_for_reservation(pending, &mut stop, || async {
                db.execute_unprepared("SELECT 1").await?;
                Ok(true)
            }),
        )
        .await
        .expect("status must not suspend the connection owner")
        .unwrap();
        assert_eq!(result, Some(42));
        tokio::time::timeout(Duration::from_secs(1), db.execute_unprepared("SELECT 1"))
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn cancelled_reservation_drops_connection_before_finalization() {
        use sea_orm::TransactionTrait;
        let (_dir, db, _) = crate::executions::lease::tests::database().await;
        let (cancel, mut stop) = tokio::sync::watch::channel(false);
        let started = tokio::sync::Notify::new();
        let pending = async {
            let _tx = db.begin().await?;
            started.notify_one();
            std::future::pending::<Result<i32>>().await
        };
        let request = async {
            started.notified().await;
            tokio::time::sleep(Duration::from_millis(250)).await;
            cancel.send_replace(true);
        };
        let waiting = wait_for_reservation(pending, &mut stop, || async {
            db.execute_unprepared("SELECT 1").await?;
            Ok(true)
        });
        let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(waiting, request)
        })
        .await
        .expect("cancellation must not wait for the status connection");
        assert_eq!(result.unwrap(), None);
        tokio::time::timeout(Duration::from_secs(1), db.execute_unprepared("SELECT 1"))
            .await
            .unwrap()
            .unwrap();
    }
    fn success() -> Outcome {
        Outcome {
            state: "success".into(),
            outputs: BTreeMap::new(),
            continue_on_error: false,
            artifact_retention: BTreeMap::new(),
            plan: Plan::default(),
            artifacts: vec![],
            steps: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn dag_starts_parallel_branches_and_waits_for_both_before_join() {
        let jobs = vec![
            test_job("left", &[]),
            test_job("right", &[]),
            test_job("join", &["left", "right"]),
        ];
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (_stop, mut stop) = tokio::sync::watch::channel(false);
        let execution = schedule_jobs(
            &jobs,
            &mut stop,
            || async { Ok(true) },
            |_| false,
            |job, completed, _, _| {
                let barrier = barrier.clone();
                let finished = finished.clone();
                async move {
                    if job.key == "join" {
                        assert_eq!(finished.load(std::sync::atomic::Ordering::SeqCst), 2);
                        assert_eq!(completed.len(), 2);
                    } else {
                        // A serial implementation deadlocks here and fails the timeout.
                        barrier.wait().await;
                        if job.key == "right" {
                            tokio::time::sleep(Duration::from_millis(30)).await;
                        }
                        finished.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    Ok(success())
                }
            },
        );
        let result = tokio::time::timeout(Duration::from_secs(2), execution)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.len(), 3);
    }

    #[tokio::test]
    async fn scheduler_status_checks_do_not_starve_jobs_holding_the_only_db_connection() {
        use sea_orm::TransactionTrait;
        let mut options = sea_orm::ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = sea_orm::Database::connect(options).await.unwrap();
        let jobs = vec![test_job("database-user", &[])];
        let initial_status = std::sync::atomic::AtomicBool::new(true);
        let connection_owned = tokio::sync::Notify::new();
        let status_started = tokio::sync::Notify::new();
        let (_stop, mut stop) = tokio::sync::watch::channel(false);
        let execution = schedule_jobs(
            &jobs,
            &mut stop,
            || async {
                if !initial_status.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    // The status future can start before begin() completes.
                    // Wait for ownership instead of inspecting a racy flag.
                    connection_owned.notified().await;
                    status_started.notify_one();
                }
                db.execute_unprepared("SELECT 1").await?;
                Ok(true)
            },
            |_| false,
            |_, _, _, _| async {
                let tx = db.begin().await?;
                connection_owned.notify_one();
                // Release only after the scheduler starts a competing status
                // read. It must still poll this job while that read is pending.
                status_started.notified().await;
                tx.rollback().await?;
                Ok(success())
            },
        );
        let completed = tokio::time::timeout(Duration::from_secs(2), execution)
            .await
            .expect("scheduler stopped polling the job holding its database connection")
            .unwrap();
        assert_eq!(completed.len(), 1);
        db.execute_unprepared("SELECT 1").await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_run_still_runs_cleanup_jobs_and_skips_the_rest() {
        let jobs = vec![
            test_job("build", &[]),
            test_job("notify", &["build"]),
            test_job("deploy", &["build"]),
            test_job("report", &["deploy"]),
        ];
        let cancelled_run = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let started = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (_stop, mut stop) = tokio::sync::watch::channel(false);
        let execution = schedule_jobs(
            &jobs,
            &mut stop,
            || {
                let cancelled_run = cancelled_run.clone();
                async move { Ok(!cancelled_run.load(std::sync::atomic::Ordering::SeqCst)) }
            },
            |job| ["notify", "report"].contains(&job.key.as_str()),
            |job, completed, mut cancel, run_cancelled| {
                let cancelled_run = cancelled_run.clone();
                let started = started.clone();
                async move {
                    started
                        .lock()
                        .unwrap()
                        .push((job.key.clone(), run_cancelled));
                    if job.key == "build" {
                        // The user cancels while the first job runs.
                        cancelled_run.store(true, std::sync::atomic::Ordering::SeqCst);
                        while !*cancel.borrow() {
                            cancel.changed().await.unwrap();
                        }
                        return Ok(Outcome {
                            state: "cancelled".into(),
                            ..success()
                        });
                    }
                    // Cleanup jobs see their needs, including the skipped ones.
                    if job.key == "report" {
                        assert_eq!(completed["deploy"].state, "skipped");
                    }
                    assert!(
                        !*cancel.borrow(),
                        "cleanup jobs are not cancelled by the run"
                    );
                    Ok(success())
                }
            },
        );
        let completed = tokio::time::timeout(Duration::from_secs(5), execution)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(completed["build"].state, "cancelled");
        assert_eq!(completed["notify"].state, "success");
        assert_eq!(completed["deploy"].state, "skipped");
        assert_eq!(completed["report"].state, "success");
        let mut started = started.lock().unwrap().clone();
        started.sort();
        assert_eq!(
            started,
            vec![
                ("build".to_owned(), false),
                ("notify".to_owned(), true),
                ("report".to_owned(), true)
            ]
        );
    }

    #[test]
    fn cleanup_jobs_are_those_whose_condition_calls_always_or_cancelled() {
        let document = |condition: &str| crate::pipelines::actions::Document {
            engine: "actions".into(),
            workflow: format!(
                "on: workflow_dispatch\njobs:\n  a:\n    runs-on: self-hosted\n    if: {condition}\n    steps:\n      - run: echo\n"
            ),
            ..serde_json::from_value(json!({"engine":"actions","workflow":""})).unwrap()
        };
        assert!(cleanup_job(&document("${{ always() }}"), "a"));
        assert!(cleanup_job(&document("cancelled ( )"), "a"));
        assert!(cleanup_job(&document("${{ !cancelled() }}"), "a"));
        assert!(!cleanup_job(&document("success()"), "a"));
        assert!(!cleanup_job(
            &document("github.ref == 'refs/heads/main'"),
            "a"
        ));
    }

    #[tokio::test]
    async fn scheduler_failure_cancels_and_drains_other_running_jobs() {
        let jobs = vec![
            test_job("broken", &[]),
            test_job("running", &[]),
            test_job("dependent", &["broken"]),
        ];
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let cleaned = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_stop, mut stop) = tokio::sync::watch::channel(false);
        let execution = schedule_jobs(
            &jobs,
            &mut stop,
            || async { Ok(true) },
            |_| false,
            |job, _, mut cancelled, _| {
                let barrier = barrier.clone();
                let cleaned = cleaned.clone();
                async move {
                    assert_ne!(job.key, "dependent");
                    barrier.wait().await;
                    if job.key == "broken" {
                        return Err(ApiError::bad("persistence failure"));
                    }
                    while !*cancelled.borrow() {
                        cancelled.changed().await.unwrap();
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    cleaned.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok(success())
                }
            },
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(2), execution)
                .await
                .unwrap()
                .is_err()
        );
        assert!(cleaned.load(std::sync::atomic::Ordering::SeqCst));
    }
}
