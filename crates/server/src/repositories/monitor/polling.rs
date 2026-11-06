//! Complete provider snapshots, durable baselines and cluster-safe polling.
use super::*;
use crate::{entities::locks, lock, repositories::auth::MonitorAccess};
use futures::{StreamExt, stream};
use sea_orm::QuerySelect;
use std::collections::BTreeMap;
use tokio::sync::watch;

const LEASE_MS: i64 = 60_000;
pub(super) type Snapshot = BTreeMap<String, Repository>;

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::IntoActiveModel;

    /// An isolated database-backed state with workspace 1.
    async fn test_state() -> (tempfile::TempDir, AppState) {
        let (dir, db) = crate::lock::tests::database().await;
        let s = std::sync::Arc::new(crate::state::State {
            ids: std::sync::Arc::new(
                crate::snowflake::Generator::new(1, tokio::time::Instant::now().into_std())
                    .unwrap(),
            ),
            db,
            data_dir: dir.path().into(),
            instance_id: "poll-test".into(),
            server_mode: false,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), false).unwrap(),
            host_monitor: Default::default(),
            monitor: Default::default(),
            cluster: None,
        });
        crate::entities::workspaces::Entity::insert(
            crate::entities::workspaces::Model {
                id: 1,
                slug: "poll-test".into(),
                name: "Poll".into(),
                owner_id: 1,
                join_code: "poll-test".into(),
                pipeline_run_number: 0,
                workflow_execution_number: 0,
                created_at: 0,
                updated_at: 0,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
        (dir, s)
    }

    #[tokio::test]
    async fn editing_notifications_preserves_poll_history_and_baseline() {
        let (_dir, s) = test_state().await;
        connections::Entity::insert(
            connections::Model {
                id: 2,
                workspace_id: 1,
                user_id: 1,
                provider: "codeup".into(),
                name: "Monitor".into(),
                description: String::new(),
                credential_id: 3,
                account: String::new(),
                refresh_secret: String::new(),
                expires_at: 0,
                state: "connected".into(),
                last_error: String::new(),
                client_id: String::new(),
                config_fingerprint: String::new(),
                token_config: None,
                version: 1,
                created_at: 0,
                updated_at: 0,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
        let scope = || Scope {
            workspace_id: 1.into(),
            user: crate::accounts::auth::User {
                id: 1.into(),
                email: "monitor@example.invalid".into(),
                name: "Monitor".into(),
                is_superadmin: false,
            },
            permissions: [
                "repositories.read",
                "repositories.update",
                "credentials.read",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        };
        let body = |version, enabled, interval, events: Vec<&str>| {
            serde_json::from_value::<Body>(json!({
                "version": version, "enabled": enabled, "mode": "poll",
                "poll_interval_minutes": interval, "all_repositories": true,
                "repositories": "", "events": events,
                "notification_language": "zh-CN", "notification_channels": [],
            }))
            .unwrap()
        };
        let mut initial = body(0, true, 5, vec!["push", "branch.created"]);
        initial.notification_language = Some("en-US".into());
        super::super::update(
            State(s.clone()),
            scope(),
            Path(("1".into(), 2.into())),
            Json(initial),
        )
        .await
        .unwrap();
        let baseline = s.vault.seal("{}", &poll_aad(1, 2)).unwrap();
        let next = db::now() + 600_000;
        polls::Entity::update_many()
            .col_expr(polls::Column::Snapshot, Expr::value(baseline.clone()))
            .col_expr(polls::Column::LastPolledAt, Expr::value(1234_i64))
            .col_expr(polls::Column::NextPollAt, Expr::value(next))
            .exec(&s.db)
            .await
            .unwrap();
        // Saving a notification edit (even reordered event selections) keeps
        // successful history and the next check, while advancing the version fence.
        super::super::update(
            State(s.clone()),
            scope(),
            Path(("1".into(), 2.into())),
            Json(body(1, true, 5, vec!["branch.created", "push"])),
        )
        .await
        .unwrap();
        let poll = polls::Entity::find_by_id(2)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(poll.config_version, 2);
        assert_eq!(poll.snapshot, baseline);
        assert_eq!(poll.last_polled_at, 1234);
        assert_eq!(poll.next_poll_at, next);
        let Json(view) = super::super::get(State(s.clone()), scope(), Path(("1".into(), 2.into())))
            .await
            .unwrap();
        assert_eq!(view["last_polled_at"], 1234);
        assert_eq!(
            view["poll_status"],
            crate::i18n::render(
                crate::i18n::current_locale(),
                &crate::i18n::Message::new("repository.monitor.ready")
            )
        );
        // Pausing, changing the interval, and enabling again retain the baseline
        // and real failure information; notification edits do not fake recovery.
        polls::Entity::update_many()
            .col_expr(polls::Column::LastError, Expr::value("previous error"))
            .exec(&s.db)
            .await
            .unwrap();
        for (version, enabled, interval) in [(2, false, 5), (3, false, 10), (4, true, 10)] {
            super::super::update(
                State(s.clone()),
                scope(),
                Path(("1".into(), 2.into())),
                Json(body(
                    version,
                    enabled,
                    interval,
                    vec!["push", "branch.created"],
                )),
            )
            .await
            .unwrap();
            let poll = polls::Entity::find_by_id(2)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(poll.config_version, version + 1);
            assert_eq!(poll.snapshot, baseline);
            assert_eq!(poll.last_polled_at, 1234);
            assert_eq!(poll.last_error, "previous error");
        }
        // A changed sampling scope still establishes a new baseline, rather than
        // mistaking newly sampled tags for new remote events.
        super::super::update(
            State(s.clone()),
            scope(),
            Path(("1".into(), 2.into())),
            Json(body(
                5,
                true,
                10,
                vec!["push", "branch.created", "tag.created"],
            )),
        )
        .await
        .unwrap();
        let poll = polls::Entity::find_by_id(2)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(poll.config_version, 6);
        assert!(poll.snapshot.is_empty());
        assert_eq!(poll.last_polled_at, 0);
    }

    #[tokio::test]
    async fn retention_removes_only_events_older_than_the_configured_days() {
        let (_dir, s) = test_state().await;
        let day = 86_400_000;
        for (id, age) in [(10, 8 * day), (11, day)] {
            events::Entity::insert(events::ActiveModel {
                id: Set(id),
                workspace_id: Set(1),
                connection_id: Set(2),
                delivery_key: Set(format!("key-{id}")),
                event: Set("push".into()),
                repository: Set("team/repo".into()),
                reference: Set("main".into()),
                actor: Set(String::new()),
                action: Set(String::new()),
                delivery: Set(String::new()),
                created_at: Set(db::now() - age),
                detail: Set(None),
            })
            .exec_without_returning(&s.db)
            .await
            .unwrap();
        }
        let remaining = || async {
            events::Entity::find()
                .all(&s.db)
                .await
                .unwrap()
                .into_iter()
                .map(|e| e.id)
                .collect::<Vec<_>>()
        };
        // Without a positive retention nothing is removed.
        prune(&s).await.unwrap();
        assert_eq!(remaining().await, vec![10, 11]);
        crate::entities::settings::Entity::insert(crate::entities::settings::ActiveModel {
            id: Set(20),
            workspace_id: Set(1),
            key: Set("repository_event_retention_days".into()),
            value: Set(7),
            version: Set(1),
            created_at: Set(0),
            updated_at: Set(0),
            ..Default::default()
        })
        .exec_without_returning(&s.db)
        .await
        .unwrap();
        prune(&s).await.unwrap();
        assert_eq!(remaining().await, vec![11]);
    }

    #[tokio::test]
    async fn shared_database_claim_and_fences_exclude_other_replicas_and_stale_results() {
        let (_dir, s) = test_state().await;
        connections::Entity::insert(
            connections::Model {
                id: 2,
                workspace_id: 1,
                user_id: 1,
                provider: "codeup".into(),
                name: "test".into(),
                description: String::new(),
                credential_id: 3,
                account: String::new(),
                refresh_secret: String::new(),
                expires_at: 0,
                state: "connected".into(),
                last_error: String::new(),
                client_id: String::new(),
                config_fingerprint: String::new(),
                token_config: None,
                version: 1,
                created_at: 0,
                updated_at: 0,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
        let config = Config {
            enabled: true,
            mode: Mode::Poll,
            poll_interval_minutes: 1,
            provider: "codeup".into(),
            ..Default::default()
        };
        monitors::Entity::insert(
            monitors::Model {
                id: 2,
                workspace_id: 1,
                config: s
                    .vault
                    .seal(&serde_json::to_string(&config).unwrap(), &aad(1, 2))
                    .unwrap(),
                version: 1,
                created_at: 0,
                updated_at: 0,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
        let row = polls::Model {
            id: 2,
            workspace_id: 1,
            config_version: 1,
            snapshot: String::new(),
            last_polled_at: 0,
            next_poll_at: 0,
            last_error: String::new(),
        };
        polls::Entity::insert(row.clone().into_active_model())
            .exec(&s.db)
            .await
            .unwrap();
        let (first, second) = tokio::join!(claim(&s, row.clone()), claim(&s, row.clone()));
        let claimed = [first.unwrap(), second.unwrap()];
        assert_eq!(claimed.iter().filter(|v| v.is_some()).count(), 1);
        let winner = claimed.into_iter().flatten().next().unwrap();
        current(&s.db, &winner).await.unwrap();
        // Even if another scheduler has stale due rows, an active lease wins.
        polls::Entity::update_many()
            .col_expr(polls::Column::NextPollAt, Expr::value(0))
            .exec(&s.db)
            .await
            .unwrap();
        assert!(claim(&s, row.clone()).await.unwrap().is_none());
        monitors::Entity::update_many()
            .col_expr(monitors::Column::Version, Expr::value(2))
            .exec(&s.db)
            .await
            .unwrap();
        assert!(current(&s.db, &winner).await.is_err());
        monitors::Entity::update_many()
            .col_expr(monitors::Column::Version, Expr::value(1))
            .exec(&s.db)
            .await
            .unwrap();
        locks::Entity::update_many()
            .col_expr(locks::Column::Owner, Expr::value("other-replica"))
            .exec(&s.db)
            .await
            .unwrap();
        assert!(current(&s.db, &winner).await.is_err());
        assert_eq!(
            lock::release(&s.db, &winner.name, &winner.owner)
                .await
                .unwrap(),
            0
        );
        lock::release(&s.db, &winner.name, "other-replica")
            .await
            .unwrap();
        polls::Entity::update_many()
            .col_expr(polls::Column::ConfigVersion, Expr::value(0))
            .exec(&s.db)
            .await
            .unwrap();
        assert!(claim(&s, row).await.unwrap().is_none());
    }

    #[test]
    fn legacy_configuration_remains_push_with_one_minute_default() {
        let config: Config = serde_json::from_value(json!({"enabled":false,"provider":"codeup","all_repositories":true,"repositories":[],"events":["push"],"secret":"","webhook":""})).unwrap();
        assert!(config.mode == Mode::Push);
        assert_eq!(config.poll_interval_minutes, 1);
        assert!(supported_events("codeup", Mode::Poll).contains(&"repository.created"));
        assert!(!supported_events("codeup", Mode::Push).contains(&"repository.created"));
    }

    #[tokio::test]
    async fn disappeared_repository_queues_only_event_analysis_without_notification_leak() {
        use crate::entities::{
            repository_analysis_configs as configs, repository_analysis_jobs as jobs,
            repository_analysis_sources as sources,
        };
        use sea_orm::PaginatorTrait;
        let (_dir, s) = test_state().await;
        let connection = connections::Model {
            id: 2,
            workspace_id: 1,
            user_id: 1,
            provider: "github".into(),
            name: "test".into(),
            description: String::new(),
            credential_id: 3,
            account: String::new(),
            refresh_secret: String::new(),
            expires_at: 0,
            state: "connected".into(),
            last_error: String::new(),
            client_id: String::new(),
            config_fingerprint: String::new(),
            token_config: None,
            version: 1,
            created_at: 0,
            updated_at: 0,
        };
        let analysis = crate::repositories::analysis::Config {
            enabled: true,
            event_repositories: Some(vec!["team/event".into()]),
            ..Default::default()
        };
        configs::Entity::insert(
            configs::Model {
                id: 2,
                workspace_id: 1,
                config: serde_json::to_string(&analysis).unwrap(),
                coverage_start: 0,
                next_reconcile_at: db::now() + 86_400_000,
                last_error: String::new(),
                version: 1,
                created_at: 0,
                updated_at: 0,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
        for (id, path) in [(10, "team/event"), (11, "team/scheduled")] {
            sources::Entity::insert(
                sources::Model {
                    id,
                    workspace_id: 1,
                    connection_id: 2,
                    repository_id: id + 10,
                    remote_path: path.into(),
                    coverage_start: 0,
                    config_version: 1,
                    state: "complete".into(),
                    last_error: String::new(),
                    synced_at: 1,
                }
                .into_active_model(),
            )
            .exec(&s.db)
            .await
            .unwrap();
            jobs::Entity::insert(
                jobs::Model {
                    id,
                    workspace_id: 1,
                    generation: 1,
                    completed_generation: 1,
                    state: "complete".into(),
                    cursor: String::new(),
                    retry_at: 0,
                    lease_owner: String::new(),
                    lease_until: 0,
                    last_error: String::new(),
                    updated_at: 0,
                }
                .into_active_model(),
            )
            .exec(&s.db)
            .await
            .unwrap();
        }
        let previous = BTreeMap::from([
            ("team/event".into(), Repository::default()),
            ("team/scheduled".into(), Repository::default()),
        ]);
        let next = Snapshot::new();
        let notification = Config {
            all_repositories: false,
            repositories: vec!["team/notification-only".into()],
            events: vec!["repository.deleted".into()],
            ..Default::default()
        };
        let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
        observe_analysis(&s, &tx, &connection, Some(&previous), &next)
            .await
            .unwrap();
        // The same notifications admission used by observe() must exclude the
        // analysis-only repository even when deletion notifications are enabled.
        for event in changes(&previous, &next, &notification)
            .into_iter()
            .filter(|event| {
                notification.all_repositories
                    || notification.repositories.contains(&event.repository)
            })
        {
            record(&s, &tx, &connection, &notification, &event, "removed-test")
                .await
                .unwrap();
        }
        tx.commit().await.unwrap();
        let event_job = jobs::Entity::find_by_id(10)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event_job.generation, 2);
        assert_eq!(event_job.state, "queued");
        let scheduled_job = jobs::Entity::find_by_id(11)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(scheduled_job.generation, 1);
        assert_eq!(scheduled_job.state, "complete");
        assert_eq!(events::Entity::find().count(&s.db).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn git_polling_shares_analysis_capacity_and_fences_a_lost_slot() {
        let (_dir, s) = test_state().await;
        connections::Entity::insert(
            connections::Model {
                id: 2,
                workspace_id: 1,
                user_id: 1,
                provider: "git".into(),
                name: "refs".into(),
                description: String::new(),
                credential_id: 0,
                account: String::new(),
                refresh_secret: String::new(),
                expires_at: 0,
                state: "connected".into(),
                last_error: String::new(),
                client_id: String::new(),
                config_fingerprint: String::new(),
                token_config: None,
                version: 1,
                created_at: 0,
                updated_at: 0,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
        let config = Config {
            enabled: true,
            mode: Mode::Poll,
            provider: "git".into(),
            ..Default::default()
        };
        monitors::Entity::insert(monitors::ActiveModel {
            id: Set(2),
            workspace_id: Set(1),
            version: Set(1),
            config: Set(s
                .vault
                .seal(&serde_json::to_string(&config).unwrap(), &aad(1, 2))
                .unwrap()),
            created_at: Set(0),
            updated_at: Set(0),
        })
        .exec(&s.db)
        .await
        .unwrap();
        let row = polls::Model {
            id: 2,
            workspace_id: 1,
            config_version: 1,
            snapshot: String::new(),
            last_polled_at: 0,
            next_poll_at: 0,
            last_error: String::new(),
        };
        polls::Entity::insert(row.clone().into_active_model())
            .exec(&s.db)
            .await
            .unwrap();
        let slot = "repository-analysis-slot:0";
        assert!(
            lock::acquire(&s.db, slot, "analysis", db::now(), LEASE_MS)
                .await
                .unwrap()
        );
        assert!(claim(&s, row.clone()).await.unwrap().is_none());
        assert!(
            locks::Entity::find_by_id(lease_name(2))
                .one(&s.db)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            polls::Entity::find_by_id(2)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap()
                .next_poll_at,
            0
        );
        lock::release(&s.db, slot, "analysis").await.unwrap();
        let claimed = claim(&s, row).await.unwrap().unwrap();
        current(&s.db, &claimed).await.unwrap();
        assert_eq!(claimed.slot.as_deref(), Some(slot));
        locks::Entity::update_many()
            .col_expr(locks::Column::Owner, Expr::value("replacement"))
            .filter(locks::Column::Name.eq(slot))
            .exec(&s.db)
            .await
            .unwrap();
        assert!(current(&s.db, &claimed).await.is_err());
        assert_eq!(lock::release(&s.db, slot, &claimed.owner).await.unwrap(), 0);
    }

    #[test]
    fn git_event_capabilities_and_moved_tags_preserve_actual_ref_changes() {
        assert!(supported_events("git", Mode::Push).is_empty());
        let allowed = supported_events("git", Mode::Poll);
        assert_eq!(allowed.len(), 6);
        assert!(!allowed.contains(&"pull_request"));
        assert!(!allowed.contains(&"repository.deleted"));
        assert!(!supported_events("github", Mode::Poll).contains(&"tag.updated"));
        let config = Config {
            provider: "git".into(),
            events: allowed.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        let state = |target: &str| {
            BTreeMap::from([(
                "https://git.example.test/project.git".into(),
                Repository {
                    tags: BTreeMap::from([("v1".into(), target.into())]),
                    ..Default::default()
                },
            )])
        };
        let old = state("a");
        let new = state("b");
        assert!(changes(&new, &new, &config).is_empty());
        let events = changes(&old, &new, &config);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "tag.updated");
        assert!(events[0].actor.is_empty());
        assert_eq!(events[0].detail["before"], "a");
        assert_eq!(events[0].detail["after"], "b");
    }

    #[test]
    fn incomplete_or_duplicate_refs_cannot_replace_a_complete_baseline() {
        assert!(references(vec![json!({"name":"main"})], true).is_err());
        assert!(
            references(
                vec![
                    json!({"name":"main","commit":{"sha":"a"}}),
                    json!({"name":"main","commit":{"sha":"b"}})
                ],
                true
            )
            .is_err()
        );
        for field in ["sha", "id"] {
            assert_eq!(
                references(vec![json!({"name":"main","commit":{field:"a"}})], true).unwrap()["main"],
                "a"
            );
        }
        assert!(requests(vec![json!({"localId":1})]).is_err());
    }

    #[test]
    fn diff_detects_repeated_head_changes_and_merges_without_inventing_actors() {
        let config = Config {
            events: KINDS.iter().map(|e| e.to_string()).collect(),
            ..Default::default()
        };
        let state = |head: &str, merged: bool| {
            BTreeMap::from([("team/repo".into(),Repository {
            branches: BTreeMap::from([("main".into(),head.into())]),
            requests: requests(vec![json!({"localId":1,"state":if merged {"MERGED"} else {"UNDER_REVIEW"},"sourceBranch":"feature","targetBranch":"main"})]).unwrap(),
            ..Default::default()
        })])
        };
        let a = state("a", false);
        let b = state("b", false);
        let merged = state("b", true);
        assert!(changes(&a, &a, &config).is_empty());
        for (before, after) in [(&a, &b), (&b, &a), (&a, &b)] {
            let events = changes(before, after, &config);
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].event, "push");
            assert!(events[0].actor.is_empty());
        }
        let events = changes(&b, &merged, &config);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "merge");
        assert!(changes(&merged, &merged, &config).is_empty());
        assert_eq!(
            changes(&Snapshot::new(), &a, &config).len(),
            1,
            "new repo contents establish their own baseline"
        );
        assert_eq!(
            changes(&a, &Snapshot::new(), &config)[0].event,
            "repository.deleted"
        );
    }

    fn all_events() -> Config {
        Config {
            events: KINDS.iter().map(|e| e.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn diff_names_only_known_operators_and_keeps_detail_out_of_the_baseline() {
        let before = BTreeMap::from([(
            "team/repo".to_owned(),
            Repository {
                branches: BTreeMap::from([("main".into(), "a".into()), ("old".into(), "x".into())]),
                tags: BTreeMap::from([("v1".into(), "t1".into())]),
                requests: requests(vec![json!({"iid":1,"state":"opened","title":"Add login","description":"Secret plan","author":{"username":"ann"},"source_branch":"f","target_branch":"main"})]).unwrap(),
                ..Default::default()
            },
        )]);
        let after = BTreeMap::from([(
            "team/repo".to_owned(),
            Repository {
                branches: BTreeMap::from([("main".into(), "a".into()), ("feature".into(), "c".into())]),
                tags: BTreeMap::from([("v2".into(), "t2".into())]),
                requests: requests(vec![json!({"iid":1,"state":"merged","title":"Add login","description":"Secret plan","author":{"username":"ann"},"merged_by":{"username":"lead"},"source_branch":"f","target_branch":"main","web_url":"https://x/mr/1"})]).unwrap(),
                branch_detail: BTreeMap::from([("feature".into(), json!({"commit":{"id":"c","message":"init feature","author":"Bob","author_username":"bob"}}))]),
                tag_detail: BTreeMap::from([("v2".into(), json!({"target":"t2","message":"release notes","tagger":"tom"}))]),
                ..Default::default()
            },
        )]);
        let events = changes(&before, &after, &all_events());
        let find = |kind: &str| events.iter().find(|e| e.event == kind).unwrap();
        let created = find("branch.created");
        // Polling cannot know who pushed: no operator, and no note saying so.
        assert!(created.actor.is_empty());
        assert_eq!(created.detail["commit"]["message"], "init feature");
        assert_eq!(created.detail["source"], "polling");
        assert!(created.detail.get("note").is_none());
        let deleted = find("branch.deleted");
        assert!(deleted.actor.is_empty());
        assert_eq!(deleted.detail["before"], "x");
        assert!(deleted.detail.get("note").is_none());
        let tag = find("tag.created");
        assert_eq!(
            (tag.actor.as_str(), tag.detail["message"].as_str()),
            ("tom", Some("release notes"))
        );
        assert!(find("tag.deleted").detail.get("note").is_none());
        let merge = find("merge");
        assert_eq!(merge.actor, "lead");
        assert_eq!(merge.detail["request"]["description"], "Secret plan");
        assert_eq!(merge.detail["request"]["url"], "https://x/mr/1");
        let baseline = serde_json::to_string(&after).unwrap();
        for private in ["Secret plan", "init feature", "release notes", "lead"] {
            assert!(
                !baseline.contains(private),
                "{private} must not be persisted in the baseline"
            );
        }
    }

    fn push(before: &str) -> Event {
        Event {
            event: "push".into(),
            repository: "team/repo".into(),
            reference: "main".into(),
            actor: String::new(),
            action: String::new(),
            detail: json!({"source":"polling","before":before,"after":"d"}),
        }
    }

    #[test]
    fn push_commits_stop_at_the_previous_revision_without_guessing_the_pusher() {
        let commits = vec![
            json!({"id":"d","message":"third","author":"Cy","author_username":"cy"}),
            json!({"id":"c","message":"second","author":"Bo"}),
            json!({"id":"b","message":"base"}),
        ];
        let mut event = push("b");
        apply_commits(&mut event, commits.clone());
        assert_eq!(event.detail["total_commits"], 2);
        assert_eq!(event.detail["commits"][0]["id"], "d");
        assert_eq!(event.detail["commits"].as_array().unwrap().len(), 2);
        assert!(event.actor.is_empty());
        assert!(event.detail.get("note").is_none());
        // Rewritten history: report the newest commits and say why.
        let mut forced = push("gone");
        apply_commits(&mut forced, commits);
        assert!(forced.detail.get("total_commits").is_none());
        assert_eq!(forced.detail["commits"].as_array().unwrap().len(), 3);
        assert!(
            crate::i18n::render_stored(
                crate::i18n::Locale::ZhCn,
                forced.detail["note"].as_str().unwrap()
            )
            .contains("强制推送")
        );
        let mut branch = Event {
            event: "branch.created".into(),
            detail: json!({}),
            ..push("")
        };
        apply_commits(
            &mut branch,
            vec![json!({"id":"h","message":"head","author":"Dee"})],
        );
        assert_eq!(
            (
                branch.actor.as_str(),
                branch.detail["commit"]["id"].as_str()
            ),
            ("", Some("h"))
        );
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Repository {
    pub(super) branches: BTreeMap<String, String>,
    pub(super) tags: BTreeMap<String, String>,
    requests: BTreeMap<String, Request>,
    // Listing detail of the current poll only, for event detail; never part of
    // the persisted baseline.
    #[serde(skip)]
    info: Value,
    #[serde(skip)]
    branch_detail: BTreeMap<String, Value>,
    #[serde(skip)]
    tag_detail: BTreeMap<String, Value>,
}
#[derive(Serialize, Deserialize)]
struct Request {
    fingerprint: String,
    merged: bool,
    reference: String,
    #[serde(skip)]
    detail: Value,
}

fn note(detail: &mut Value, message: crate::i18n::Message) {
    if let Value::Object(map) = detail {
        let joined = match map.get("note").and_then(Value::as_str) {
            Some(existing) if !existing.is_empty() => {
                let joined = crate::i18n::Message::new("repository.monitor.notes")
                    .with_message("second", message);
                match crate::i18n::decode_stored(existing) {
                    Some(first) => joined.with_message("first", first),
                    None => joined.with("first", existing),
                }
            }
            _ => message,
        };
        map.insert(
            "note".into(),
            Value::from(crate::i18n::store_message(&joined)),
        );
    }
}

/// Branch or tag listing detail keyed by name, for the current poll only.
fn reference_details(rows: &[Value]) -> BTreeMap<String, Value> {
    rows.iter()
        .filter_map(|row| {
            let name = text(row, &["/name", "/branchName"]);
            (!name.is_empty()).then(|| (name, detail::reference(row)))
        })
        .collect()
}

fn invalid() -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        crate::i18n::Message::new("repository.monitor.dataIncomplete"),
    )
}
fn bounded(value: &str) -> String {
    value.chars().take(400).collect()
}
fn poll_aad(workspace: i64, id: i64) -> String {
    format!("repository-poll:{workspace}:{id}")
}
fn lease_name(id: i64) -> String {
    format!("repository-poll:{id}")
}

async fn resource(access: &MonitorAccess, path: &str, name: &str) -> Result<Vec<Value>> {
    let mut result = Vec::new();
    for page in 1..=10000 {
        let rows = access.resource(path, name, page).await?;
        result.extend(rows.rows);
        if !rows.has_more {
            return Ok(result);
        }
    }
    Err(ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        crate::i18n::Message::new("repository.monitor.paginationUnsupported"),
    ))
}
fn references(rows: Vec<Value>, require_revision: bool) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    for row in rows {
        let name = text(&row, &["/name", "/branchName"]);
        let revision = text(&row, &["/commit/sha", "/commit/id", "/commitId", "/target"]);
        if name.is_empty()
            || (require_revision && revision.is_empty())
            || result.insert(name, revision).is_some()
        {
            return Err(invalid());
        }
    }
    Ok(result)
}
fn requests(rows: Vec<Value>) -> Result<BTreeMap<String, Request>> {
    let mut result = BTreeMap::new();
    for row in rows {
        let identity = ["localId", "iid", "number", "id"]
            .iter()
            .find_map(|key| {
                row.get(key)
                    .filter(|v| v.is_string() || v.is_number())
                    .map(|v| {
                        v.as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| v.to_string())
                    })
            })
            .ok_or_else(invalid)?;
        let state = text(&row, &["/state"]);
        if state.is_empty() {
            return Err(invalid());
        }
        let merged = state.eq_ignore_ascii_case("merged")
            || row["merged"].as_bool() == Some(true)
            || row["merged_at"].as_str().is_some_and(|v| !v.is_empty());
        let reference = bounded(&format!(
            "#{identity} {} → {}",
            text(&row, &["/sourceBranch", "/source_branch", "/head/ref"]),
            text(&row, &["/targetBranch", "/target_branch", "/base/ref"])
        ));
        // The baseline keeps only a hash; readable detail lives in the event record.
        let metadata = json!({"state":state,"merged":merged,"updated":text(&row,&["/updated_at","/updatedAt"]),"head":text(&row,&["/head/sha","/sha","/mergedRevision"]),"title":row["title"],"description":row["description"],"body":row["body"],"labels":row["labels"],"reviewers":row["reviewers"],"comments":row["totalCommentCount"],"reference":reference});
        let fingerprint = hex::encode(Sha256::digest(
            serde_json::to_vec(&metadata).map_err(|_| invalid())?,
        ));
        if result
            .insert(
                identity,
                Request {
                    fingerprint,
                    merged,
                    reference,
                    detail: detail::request(&row),
                },
            )
            .is_some()
        {
            return Err(invalid());
        }
    }
    Ok(result)
}
async fn snapshot(
    s: &AppState,
    connection: &connections::Model,
    config: &Config,
) -> Result<(Snapshot, Option<MonitorAccess>)> {
    if connection.state != "connected" {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.monitor.authorizationInvalid"),
        ));
    }
    let scope =
        Scope::for_actor(s, connection.workspace_id.into(), connection.user_id.into()).await?;
    scope.require("repositories.read")?;
    scope.require("credentials.read")?;
    if connection.provider == "git" {
        return super::git::snapshot(s, connection, config)
            .await
            .map(|snapshot| (snapshot, None));
    }
    let access = MonitorAccess::load(s, connection).await?;
    let analysis =
        crate::repositories::analysis::monitor_subscription(&s.db, connection.id).await?;
    let branches = analysis.is_some()
        || config
            .events
            .iter()
            .any(|e| e == "push" || e.starts_with("branch."));
    let tags = config.events.iter().any(|e| e.starts_with("tag."));
    let prs = config
        .events
        .iter()
        .any(|e| matches!(e.as_str(), "pull_request" | "merge"));
    let mut result = BTreeMap::new();
    for page in 1..=10000 {
        let repositories = access.repositories(page).await?;
        let items = repositories["items"].as_array().ok_or_else(invalid)?;
        for row in items {
            let path = row["full_name"]
                .as_str()
                .filter(|p| !p.is_empty())
                .ok_or_else(invalid)?;
            let notification_selected =
                config.all_repositories || config.repositories.iter().any(|p| p == path);
            if !notification_selected && !analysis.as_ref().is_some_and(|v| v.selects(path)) {
                continue;
            }
            if result.contains_key(path) {
                return Err(invalid());
            }
            let (branches, branch_detail) = if branches {
                let rows = resource(&access, path, "branches").await?;
                let detail = reference_details(&rows);
                (references(rows, true)?, detail)
            } else {
                Default::default()
            };
            let (tags, tag_detail) = if tags && notification_selected {
                let rows = resource(&access, path, "tags").await?;
                let detail = reference_details(&rows);
                (references(rows, false)?, detail)
            } else {
                Default::default()
            };
            let repository = Repository {
                branches,
                tags,
                requests: if prs && notification_selected {
                    requests(resource(&access, path, "requests").await?)?
                } else {
                    BTreeMap::new()
                },
                info: detail::repository(row),
                branch_detail,
                tag_detail,
            };
            result.insert(path.into(), repository);
        }
        if repositories["has_more"].as_bool() == Some(false) {
            return Ok((result, Some(access)));
        }
    }
    Err(ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        crate::i18n::Message::new("repository.monitor.paginationUnsupported"),
    ))
}

fn changes(old: &Snapshot, new: &Snapshot, config: &Config) -> Vec<Event> {
    let mut events = Vec::new();
    let mut add = |kind: &str,
                   repository: &str,
                   reference: &str,
                   action: &str,
                   actor: String,
                   mut detail: Value| {
        if config
            .events
            .iter()
            .any(|e| e == kind || (kind == "merge" && e == "pull_request"))
        {
            if let Value::Object(map) = &mut detail {
                map.insert("source".into(), Value::from("polling"));
                if let Some(snapshot) = new.get(repository).or_else(|| old.get(repository)) {
                    let url = detail::field(&snapshot.info, &["/url"], 1000);
                    if !url.is_empty() {
                        map.insert("repository_url".into(), Value::from(url));
                    }
                }
            }
            events.push(Event {
                event: kind.into(),
                repository: bounded(repository),
                reference: bounded(reference),
                actor: bounded(&actor),
                action: crate::i18n::store_message(&crate::i18n::Message::new(action)),
                detail,
            });
        }
    };
    for (path, current) in new {
        let Some(previous) = old.get(path) else {
            let detail = json!({ "repository": current.info });
            add(
                "repository.created",
                path,
                "",
                "repository.monitor.detectedRepository",
                String::new(),
                detail,
            );
            // Existing contents of a newly visible repository are a baseline.
            continue;
        };
        for (name, revision) in &current.branches {
            match previous.branches.get(name) {
                None => {
                    let mut detail = json!({ "after": revision });
                    // Polling cannot tell who created the branch; the commit
                    // author is shown with the commit, not as the operator.
                    if let Some(commit) = current
                        .branch_detail
                        .get(name)
                        .and_then(|d| d.get("commit"))
                    {
                        detail["commit"] = commit.clone();
                    }
                    add(
                        "branch.created",
                        path,
                        name,
                        "repository.monitor.detectedBranch",
                        String::new(),
                        detail,
                    )
                }
                Some(before) if before != revision => add(
                    "push",
                    path,
                    name,
                    "repository.monitor.detectedPush",
                    String::new(),
                    json!({ "before": before, "after": revision }),
                ),
                _ => {}
            }
        }
        for (name, revision) in previous
            .branches
            .iter()
            .filter(|(name, _)| !current.branches.contains_key(*name))
        {
            let detail = json!({ "before": revision });
            add(
                "branch.deleted",
                path,
                name,
                "repository.monitor.detectedBranchDeleted",
                String::new(),
                detail,
            );
        }
        if config.provider == "git" {
            for (name, revision) in &current.tags {
                if let Some(before) = previous.tags.get(name).filter(|before| *before != revision) {
                    add(
                        "tag.updated",
                        path,
                        name,
                        "repository.monitor.detectedTagChanged",
                        String::new(),
                        json!({"before":before,"after":revision,"target":revision}),
                    );
                }
            }
        }
        for name in current
            .tags
            .keys()
            .filter(|name| !previous.tags.contains_key(*name))
        {
            let detail = current.tag_detail.get(name).cloned().unwrap_or_else(|| {
                if config.provider == "git" {
                    json!({"target":current.tags[name]})
                } else {
                    json!({})
                }
            });
            let tagger = detail::field(&detail, &["/tagger"], 200);
            let actor = if tagger.is_empty() {
                String::new()
            } else {
                tagger
            };
            add(
                "tag.created",
                path,
                name,
                "repository.monitor.detectedTag",
                actor,
                detail,
            );
        }
        for (name, target) in previous
            .tags
            .iter()
            .filter(|(name, _)| !current.tags.contains_key(*name))
        {
            let detail = json!({ "target": target });
            add(
                "tag.deleted",
                path,
                name,
                "repository.monitor.detectedTagDeleted",
                String::new(),
                detail,
            );
        }
        for (identity, request) in &current.requests {
            let prior = previous.requests.get(identity);
            if prior.is_none_or(|p| p.fingerprint != request.fingerprint) {
                let merged = request.merged && prior.is_none_or(|p| !p.merged);
                let detail = json!({ "request": request.detail });
                // Polling sees the author and the merger; other editors are unknown.
                let actor = if merged {
                    detail::field(&request.detail, &["/merged_by"], 200)
                } else if prior.is_none() {
                    detail::field(&request.detail, &["/author"], 200)
                } else {
                    String::new()
                };
                add(
                    if merged { "merge" } else { "pull_request" },
                    path,
                    &request.reference,
                    if merged {
                        "repository.monitor.detectedMerged"
                    } else if prior.is_none() {
                        "repository.monitor.detectedRequest"
                    } else {
                        "repository.monitor.detectedRequestChanged"
                    },
                    actor,
                    detail,
                );
            }
        }
    }
    for path in old.keys().filter(|path| !new.contains_key(*path)) {
        let detail = json!({});
        add(
            "repository.deleted",
            path,
            "",
            "repository.monitor.detectedRepositoryGone",
            String::new(),
            detail,
        );
    }
    events
}

/// Fetch the commits behind pushes and new branches. Provider failures leave a
/// note on the event; they never fail the poll, whose baseline must advance.
async fn enrich(access: &MonitorAccess, events: &mut [Event]) {
    for event in events
        .iter_mut()
        .filter(|e| {
            e.event == "push" || (e.event == "branch.created" && e.detail.get("commit").is_none())
        })
        .take(20)
    {
        match access.commits(&event.repository, &event.reference).await {
            Ok(rows) => apply_commits(event, rows.iter().map(detail::commit).collect()),
            Err(error) => {
                let message = crate::i18n::Message::new("repository.monitor.commitsFailed");
                let message = match error.2 {
                    Some(cause) => message.with_message("error", cause),
                    None => message.with("error", bounded(&error.1)),
                };
                note(&mut event.detail, message);
            }
        }
    }
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        for event in events.iter_mut().filter(|e| e.event == "push").take(20) {
            enrich_stats(access, event).await;
        }
    })
    .await;
}

pub(super) async fn enrich_stats(access: &MonitorAccess, event: &mut Event) {
    let before = detail::field(&event.detail, &["/before"], 64);
    let after = detail::field(&event.detail, &["/after"], 64);
    if let Some((additions, deletions)) = access
        .change_stats(&event.repository, &before, &after)
        .await
    {
        event.detail["line_stats"] = json!({"additions":additions,"deletions":deletions});
    }
}

/// Attach a branch's newest-first commits: the head of a new branch, or the
/// commits a push added since its previous revision.
fn apply_commits(event: &mut Event, commits: Vec<Value>) {
    if event.event == "branch.created" {
        if let Some(head) = commits.first() {
            event.detail["commit"] = head.clone();
        }
        return;
    }
    let before = detail::field(&event.detail, &["/before"], 64);
    let found = commits
        .iter()
        .position(|c| detail::field(c, &["/id"], 64) == before);
    let pushed = &commits[..found.unwrap_or(commits.len())];
    match found {
        Some(count) => event.detail["total_commits"] = json!(count),
        None => note(
            &mut event.detail,
            crate::i18n::Message::new("repository.monitor.historyRewritten"),
        ),
    }
    if pushed.len() > detail::COMMITS {
        event.detail["commits_truncated"] = Value::Bool(true);
    }
    event.detail["commits"] = Value::from(
        pushed
            .iter()
            .take(detail::COMMITS)
            .cloned()
            .collect::<Vec<_>>(),
    );
}

struct Claim {
    monitor: monitors::Model,
    connection: connections::Model,
    config: Config,
    state: polls::Model,
    owner: String,
    name: String,
    slot: Option<String>,
}
#[cfg(test)]
async fn claim(s: &AppState, row: polls::Model) -> Result<Option<Claim>> {
    claim_with_limit(s, row, std::num::NonZeroUsize::new(1).unwrap()).await
}
async fn claim_with_limit(
    s: &AppState,
    row: polls::Model,
    concurrency: std::num::NonZeroUsize,
) -> Result<Option<Claim>> {
    let tx = crate::variables::transaction(s, row.workspace_id.into()).await?;
    let Some(monitor) = monitors::Entity::find_by_id(row.id).one(&tx).await? else {
        return Ok(None);
    };
    let Some(connection) = connections::Entity::find_by_id(row.id).one(&tx).await? else {
        return Ok(None);
    };
    let Some(state) = polls::Entity::find_by_id(row.id).one(&tx).await? else {
        return Ok(None);
    };
    let config = decode(s, &monitor)?;
    let now = db::now();
    if !config.enabled
        || config.mode != Mode::Poll
        || config.provider != connection.provider
        || state.config_version != monitor.version
        || state.next_poll_at > now
        || connection.workspace_id != row.workspace_id
    {
        return Ok(None);
    }
    let name = lease_name(row.id);
    if locks::Entity::find_by_id(&name)
        .one(&tx)
        .await?
        .is_some_and(|v| v.expires_at >= now)
    {
        return Ok(None);
    }
    let owner = s.next_id()?.to_string();
    if !lock::acquire(&tx, &name, &owner, now, LEASE_MS).await? {
        return Ok(None);
    }
    let mut slot = None;
    if connection.provider == "git" {
        for index in 0..concurrency.get() {
            let candidate = format!("repository-analysis-slot:{index}");
            if lock::acquire(&tx, &candidate, &owner, now, LEASE_MS).await? {
                slot = Some(candidate);
                break;
            }
        }
        if slot.is_none() {
            return Ok(None); // Roll back the tentative connection lock; keep the task due.
        }
    }
    polls::Entity::update_many()
        .col_expr(
            polls::Column::NextPollAt,
            Expr::value(now + i64::from(config.poll_interval_minutes.max(1)) * 60_000),
        )
        .filter(polls::Column::Id.eq(row.id))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    Ok(Some(Claim {
        monitor,
        connection,
        config,
        state,
        owner,
        name,
        slot,
    }))
}
async fn current(tx: &impl sea_orm::ConnectionTrait, claim: &Claim) -> Result<()> {
    let monitor = monitors::Entity::find_by_id(claim.monitor.id)
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let connection = connections::Entity::find_by_id(claim.connection.id)
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let lease = locks::Entity::find_by_id(&claim.name)
        .lock_exclusive()
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    if let Some(slot) = &claim.slot {
        let slot = locks::Entity::find_by_id(slot)
            .lock_exclusive()
            .one(tx)
            .await?
            .ok_or_else(ApiError::missing)?;
        if slot.owner != claim.owner || slot.expires_at <= db::now() {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("repository.monitor.gitSlotExpired"),
            ));
        }
    }
    if monitor.version != claim.monitor.version
        || connection.version != claim.connection.version
        || lease.owner != claim.owner
        || lease.expires_at <= db::now()
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("repository.monitor.pollOwnershipChanged"),
        ));
    }
    Ok(())
}
async fn observe(s: &AppState, claim: &Claim) -> Result<()> {
    let previous: Option<Snapshot> = if claim.state.snapshot.is_empty() {
        None
    } else {
        Some(
            serde_json::from_str(&s.vault.open_secret(
                &claim.state.snapshot,
                &poll_aad(claim.state.workspace_id, claim.state.id),
            )?)
            .map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.monitor.baselineRead"),
                )
            })?,
        )
    };
    // Diff and fetch commit detail before taking the workspace lock: provider
    // I/O must not hold it. current() below still fences these candidates.
    let (mut observed, candidates) = match snapshot(s, &claim.connection, &claim.config).await {
        Ok((next, access)) => {
            let mut events = previous
                .as_ref()
                .map(|previous| changes(previous, &next, &claim.config))
                .unwrap_or_default();
            // The sampled snapshot includes analysis-only repositories too;
            // notification admission still follows only its own selection.
            events.retain(|event| {
                claim.config.all_repositories
                    || claim.config.repositories.contains(&event.repository)
            });
            if let Some(access) = access {
                enrich(&access, &mut events).await;
            }
            (Ok(next), events)
        }
        Err(error) => (Err(error), Vec::new()),
    };
    let tx = crate::variables::transaction(s, claim.connection.workspace_id.into()).await?;
    current(&tx, claim).await?;
    // Resolve permissions again after network I/O. Permission failures may update
    // scheduling/error metadata, but may never admit events or replace a baseline.
    let authorized = async {
        let scope = Scope::for_actor_in(
            &tx,
            claim.connection.workspace_id.into(),
            claim.connection.user_id.into(),
        )
        .await?;
        scope.require("repositories.read")?;
        scope.require("credentials.read")?;
        if claim.connection.provider == "git" {
            scope.require("hosts.terminal")?;
            let connection = connections::Entity::find_by_id(claim.connection.id)
                .one(&tx)
                .await?
                .ok_or_else(ApiError::missing)?;
            if connection.state != "connected" {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.monitor.authorizationBaseline"),
                ));
            }
            let runner: Id = claim.config.runner_id.parse().map_err(|_| invalid())?;
            if crate::entities::hosts::Entity::find_by_id(i64::from(runner))
                .filter(
                    crate::entities::hosts::Column::WorkspaceId.eq(claim.connection.workspace_id),
                )
                .filter(crate::entities::hosts::Column::Connection.eq("runner"))
                .one(&tx)
                .await?
                .is_none()
            {
                return Err(ApiError::missing());
            }
        }
        Ok(())
    }
    .await;
    if let Err(error) = authorized {
        observed = Err(error);
    }
    let mut deliveries = Vec::new();
    match observed {
        Ok(next) => {
            observe_analysis(s, &tx, &claim.connection, previous.as_ref(), &next).await?;
            for (index, event) in candidates.into_iter().enumerate() {
                // Unique persisted sampling identity distinguishes A→B→A→B.
                let key = format!("poll:{}:{index}", claim.owner);
                if let Some(id) =
                    record(s, &tx, &claim.connection, &claim.config, &event, &key).await?
                {
                    deliveries.push((id, event));
                }
            }
            let bytes = serde_json::to_string(&next).map_err(|_| invalid())?;
            let sealed = s
                .vault
                .seal(&bytes, &poll_aad(claim.state.workspace_id, claim.state.id))?;
            polls::Entity::update_many()
                .col_expr(polls::Column::Snapshot, Expr::value(sealed))
                .col_expr(polls::Column::LastError, Expr::value(""))
                .filter(polls::Column::Id.eq(claim.state.id))
                .exec(&tx)
                .await?;
        }
        Err(error) => {
            polls::Entity::update_many()
                .col_expr(
                    polls::Column::LastError,
                    Expr::value(if error.2.is_some() {
                        error.stored()
                    } else {
                        crate::i18n::store_raw(&bounded(&error.1))
                    }),
                )
                .filter(polls::Column::Id.eq(claim.state.id))
                .exec(&tx)
                .await?;
        }
    }
    let now = db::now();
    polls::Entity::update_many()
        .col_expr(polls::Column::LastPolledAt, Expr::value(now))
        .col_expr(
            polls::Column::NextPollAt,
            Expr::value(now + i64::from(claim.config.poll_interval_minutes.max(1)) * 60_000),
        )
        .filter(polls::Column::Id.eq(claim.state.id))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    for (id, event) in deliveries {
        deliver(s, &claim.connection, &claim.config, id, &event).await?;
    }
    Ok(())
}

/// Admit analysis hints independently of notification selection, after the poll's
/// ownership and permission fences. Disappearance can mean deletion or lost access.
async fn observe_analysis(
    s: &AppState,
    tx: &impl sea_orm::ConnectionTrait,
    connection: &connections::Model,
    previous: Option<&Snapshot>,
    next: &Snapshot,
) -> Result<()> {
    for (path, repository) in next {
        crate::repositories::analysis::observes(s, tx, connection, path, &repository.branches)
            .await?;
    }
    if let Some(previous) = previous {
        for path in previous.keys().filter(|path| !next.contains_key(*path)) {
            crate::repositories::analysis::event(s, tx, connection, path).await?;
        }
    }
    Ok(())
}
async fn poll(
    s: &AppState,
    row: polls::Model,
    drain: &mut watch::Receiver<bool>,
    concurrency: std::num::NonZeroUsize,
) -> Result<()> {
    if *drain.borrow() {
        return Ok(());
    }
    let Some(claim) = claim_with_limit(s, row, concurrency).await? else {
        return Ok(());
    };
    let result = {
        // Keep observation advancing while a renewal waits for the SQL pool.
        let renewal = async {
            let mut interval = tokio::time::interval(Duration::from_secs(15));
            interval.tick().await;
            loop {
                interval.tick().await;
                for name in std::iter::once(&claim.name).chain(claim.slot.iter()) {
                    if lock::renew(&s.db, name, &claim.owner, db::now(), LEASE_MS).await? != 1 {
                        return Err(ApiError::localized(
                            axum::http::StatusCode::CONFLICT,
                            crate::i18n::Message::new("repository.monitor.pollLeaseExpired"),
                        ));
                    }
                }
            }
        };
        tokio::select! {
            result = observe(s, &claim) => result,
            _ = drain.changed() => Ok(()),
            result = renewal => result,
        }
    };
    for name in std::iter::once(&claim.name).chain(claim.slot.iter()) {
        lock::release(&s.db, name, &claim.owner).await?;
    }
    result
}
/// Remove repository events older than each workspace's configured retention.
async fn prune(s: &AppState) -> Result<()> {
    use crate::entities::settings;
    use sea_orm::QuerySelect;
    const KEY: &str = "repository_event_retention_days";
    for setting in settings::Entity::find()
        .filter(settings::Column::Key.eq(KEY))
        .filter(settings::Column::Value.gt(0))
        .all(&s.db)
        .await?
    {
        let workspace = Id::from(setting.workspace_id);
        let tx = crate::variables::transaction(s, workspace).await?;
        let days = crate::maintenance::setting(&tx, workspace, KEY).await?;
        if days > 0 {
            let rows = events::Entity::find()
                .filter(events::Column::WorkspaceId.eq(workspace))
                .filter(events::Column::CreatedAt.lt(db::now() - days * 86_400_000))
                .limit(200)
                .all(&tx)
                .await?;
            if !rows.is_empty() {
                let removed = events::Entity::delete_many()
                    .filter(events::Column::WorkspaceId.eq(workspace))
                    .filter(events::Column::Id.is_in(rows.iter().map(|r| r.id)))
                    .exec(&tx)
                    .await?
                    .rows_affected;
                crate::hosts::activity_record(
                    s,
                    &tx,
                    workspace,
                    (Id::default(), "系统"),
                    "retention-cleanup",
                    "repository-events",
                    Id::default(),
                    json!({"removed":removed,"retention_days":days}),
                )
                .await?;
            }
        }
        tx.commit().await?;
    }
    Ok(())
}

pub async fn scheduler(
    s: AppState,
    concurrency: std::num::NonZeroUsize,
    mut drain: watch::Receiver<bool>,
) {
    let mut pruned_at = 0;
    loop {
        if *drain.borrow() {
            break;
        }
        if db::now() - pruned_at > 600_000 {
            pruned_at = db::now();
            if let Err(error) = prune(&s).await {
                tracing::warn!(error=%error.1,"仓库事件保留期清理未完成");
            }
        }
        match polls::Entity::find()
            .filter(polls::Column::NextPollAt.lte(db::now()))
            .all(&s.db)
            .await
        {
            Ok(rows) => {
                let mut work = stream::iter(rows)
                    .map(|row| {
                        let s = s.clone();
                        let mut drain = drain.clone();
                        async move { poll(&s, row, &mut drain, concurrency).await }
                    })
                    .buffer_unordered(4);
                while let Some(result) = work.next().await {
                    if let Err(error) = result {
                        tracing::warn!(error=%error.1,"仓库轮询未完成");
                    }
                }
            }
            Err(error) => tracing::error!(%error,"读取仓库轮询计划失败"),
        }
        tokio::select! { _=drain.changed()=>{}, _=tokio::time::sleep(Duration::from_secs(2))=>{} }
    }
}
