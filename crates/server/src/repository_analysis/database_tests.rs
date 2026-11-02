//! Opt-in integration coverage against a freshly provisioned local SQL database.
use super::*;
use crate::entities::repository_analysis_commit_authors as authors;
use sea_orm::{ConnectOptions, Database, DbBackend, PaginatorTrait, Statement, TransactionTrait};
use sea_orm_migration::MigratorTrait;
use std::num::NonZeroUsize;
use tests::{commit, fixture_with_database, publish_branches, queued_repository, replica};

async fn read(s: &AppState, kind: &str, params: &[(&str, &str)]) -> Value {
    let scope = Scope::for_actor(s, 1.into(), 1.into()).await.unwrap();
    report(
        State(s.clone()),
        scope,
        Path(("1".into(), kind.into())),
        Query(
            params
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ),
    )
    .await
    .unwrap()
    .0
}

async fn job(s: &AppState, id: i64) -> jobs::Model {
    jobs::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
}

async fn read_insight(s: &AppState, id: i64, kind: &str, params: &[(&str, &str)]) -> Value {
    crate::repository_analysis::insight_report(
        State(s.clone()),
        Scope::for_actor(s, 1.into(), 1.into()).await.unwrap(),
        Path(("1".into(), id.into(), kind.into())),
        Query(
            params
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ),
    )
    .await
    .unwrap()
    .0
}

async fn standalone_insight_runtime(s: &AppState, facts: &[commits::Model]) {
    use crate::entities::{code_insights, workspace_roles};
    use crate::repository_analysis::insights;
    workspace_roles::Entity::update_many()
        .col_expr(
            workspace_roles::Column::Permissions,
            Expr::value(
                encode(&[
                    "repositories.read",
                    "repositories.update",
                    "credentials.read",
                    "code-insights.read",
                    "code-insights.create",
                    "code-insights.update",
                ])
                .unwrap(),
            ),
        )
        .filter(workspace_roles::Column::Id.eq(2))
        .exec(&s.db)
        .await
        .unwrap();
    let mapped = edit_aliases(
        s,
        vec![Alias {
            name: "Author".into(),
            email: "other@example.test".into(),
            nickname: "Migrated team".into(),
            ..Default::default()
        }],
    )
    .await;
    let before_config = configs::Entity::find_by_id(4)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let before_job = job(s, 7).await;
    let before_source = sources::Entity::find_by_id(7)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    insights::recover(s).await.unwrap();
    let imported = code_insights::Entity::find_by_id(4)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(imported.coverage_start, before_config.coverage_start);
    assert_eq!(
        insights::decode(&imported).unwrap().aliases[0].contributor_id,
        mapped.aliases[0].contributor_id,
        "legacy contributor IDs survive migration"
    );
    let count = code_insights::Entity::find().count(&s.db).await.unwrap();
    insights::recover(s).await.unwrap();
    assert_eq!(
        code_insights::Entity::find().count(&s.db).await.unwrap(),
        count
    );
    assert_eq!(
        code_insights::Entity::find_by_id(4)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        imported
    );
    assert_eq!(job(s, 7).await, before_job);
    assert_eq!(
        sources::Entity::find_by_id(7)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        before_source
    );
    assert_eq!(
        configs::Entity::find_by_id(4)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .version,
        before_config.version
    );
    for (kind, total) in [
        ("overview", 1),
        ("contributors", 1),
        ("ranking", 1),
        ("branches", 2),
        ("heatmap", 3),
        ("period", 3),
        ("commits", 3),
    ] {
        let value = read_insight(s, 4, kind, &[("page_size", "1")]).await;
        assert_eq!(value["total"], total, "standalone {kind} SQL");
        assert_eq!(value["items"].as_array().unwrap().len(), 1);
    }
    assert_eq!(
        read_insight(s, 4, "commits", &[("history", "current")]).await["total"],
        1
    );
    assert_eq!(
        read_insight(
            s,
            4,
            "commits",
            &[("history", "current"), ("branches", "feature")]
        )
        .await["total"],
        0
    );
    let (_, Json(created)) = insights::create(
        State(s.clone()),
        Scope::for_actor(s, 1.into(), 1.into()).await.unwrap(),
        Json(insights::Body {
            name: "Independent SQL insight".into(),
            description: String::new(),
            version: 0,
            config: insights::InsightConfig {
                full_history: true,
                ..Default::default()
            },
            repositories: vec![
                insights::Selection {
                    connection_id: "4".into(),
                    all_repositories: true,
                    ..Default::default()
                },
                insights::Selection {
                    connection_id: "38".into(),
                    repository: "team/repo-6".into(),
                    ..Default::default()
                },
            ],
        }),
    )
    .await
    .unwrap();
    let id: i64 = created["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        read_insight(s, id, "commits", &[]).await["total"],
        3,
        "two connection selections of shared facts must count each SHA once"
    );
    assert_eq!(
        read_insight(s, id, "contributors", &[]).await["total"],
        2,
        "the migrated mapping belongs only to its own insight"
    );
    let (before_config, before_source, before_job) = (
        configs::Entity::find_by_id(4)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        sources::Entity::find_by_id(7)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        job(s, 7).await,
    );
    let mut value: insights::Body = serde_json::from_value(
        insights::get(
            State(s.clone()),
            Scope::for_actor(s, 1.into(), 1.into()).await.unwrap(),
            Path(("1".into(), id.into())),
        )
        .await
        .unwrap()
        .0,
    )
    .unwrap();
    value.config.aliases = vec![Alias {
        name: "Author".into(),
        email: "other@example.test".into(),
        nickname: "Independent team".into(),
        ..Default::default()
    }];
    let _ = insights::update(
        State(s.clone()),
        Scope::for_actor(s, 1.into(), 1.into()).await.unwrap(),
        Path(("1".into(), id.into())),
        Json(value),
    )
    .await
    .unwrap();
    assert_eq!(
        read_insight(s, id, "contributors", &[]).await["items"][0]["nickname"],
        "Independent team"
    );
    assert_eq!(
        read_insight(s, 4, "contributors", &[]).await["items"][0]["nickname"],
        "Migrated team"
    );
    assert_eq!(
        configs::Entity::find_by_id(4)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        before_config
    );
    assert_eq!(
        sources::Entity::find_by_id(7)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        before_source
    );
    assert_eq!(job(s, 7).await, before_job);
    for fact in facts {
        assert_eq!(
            commits::Entity::find_by_id(fact.id)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap(),
            *fact
        );
    }
    eprintln!(
        "PASS standalone insight recovery + seven SQL reports + historical membership + independent many-to-many identity scopes"
    );
    standalone_schedule_runtime(s, id).await;
}

async fn standalone_schedule_runtime(s: &AppState, id: i64) {
    use crate::entities::code_insights;
    use crate::repository_analysis::insights;
    let imported = code_insights::Entity::find_by_id(4)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(insights::decode(&imported).unwrap().update_mode, "event");
    assert_eq!(imported.next_run_at, 0);
    let scheduled = code_insights::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let config = insights::decode(&scheduled).unwrap();
    assert_eq!(config.update_mode, "scheduled");
    assert_eq!(config.schedule, "0 */4 * * *");
    assert!(scheduled.next_run_at > db::now());
    let count = code_insights::Entity::find().count(&s.db).await.unwrap();
    let before = job(s, 7).await;
    let invalid: insights::Body = serde_json::from_value(json!({
        "name":"Invalid SQL cron", "schedule":"not a cron", "enabled":true,
        "repositories":[{"connection_id":"4","repository":"team/repo"}]
    }))
    .unwrap();
    assert_eq!(
        insights::create(
            State(s.clone()),
            Scope::for_actor(s, 1.into(), 1.into()).await.unwrap(),
            Json(invalid)
        )
        .await
        .unwrap_err()
        .0,
        axum::http::StatusCode::BAD_REQUEST
    );
    assert_eq!(
        code_insights::Entity::find().count(&s.db).await.unwrap(),
        count
    );
    assert_eq!(job(s, 7).await, before);
    let source_ids = insights::selected_sources(
        &s.db,
        &Scope::for_actor(s, 1.into(), 1.into()).await.unwrap(),
        &scheduled,
    )
    .await
    .unwrap()
    .into_iter()
    .filter(|r| r.id > 0)
    .map(|r| r.id)
    .collect::<BTreeSet<_>>();
    let mut jobs_before = BTreeMap::new();
    for source in &source_ids {
        jobs_before.insert(*source, job(s, *source).await);
    }
    insights::schedule_due(s).await.unwrap();
    for (source, before) in &jobs_before {
        assert_eq!(job(s, *source).await, *before);
    }
    code_insights::Entity::update_many()
        .col_expr(code_insights::Column::NextRunAt, Expr::value(1_i64))
        .filter(code_insights::Column::Id.eq(id))
        .exec(&s.db)
        .await
        .unwrap();
    insights::schedule_due(s).await.unwrap();
    for (source, before) in &jobs_before {
        assert_eq!(job(s, *source).await.generation, before.generation + 1);
    }
    let consumed = code_insights::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert!(
        consumed.next_run_at > db::now(),
        "missed occurrences coalesce on real SQL engines"
    );
    insights::schedule_due(&replica(s)).await.unwrap();
    assert_eq!(
        code_insights::Entity::find_by_id(id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        consumed
    );
    for (source, before) in &jobs_before {
        assert_eq!(job(s, *source).await.generation, before.generation + 1);
    }
    eprintln!(
        "PASS persisted standalone cron default/validation + future idle + overdue one-run coalescing + replica restart on real SQL"
    );
}

async fn edit_aliases(s: &AppState, aliases: Vec<Alias>) -> Config {
    let row = configs::Entity::find_by_id(4)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let mut config = decode(&row).unwrap();
    config.aliases = aliases;
    let scope = Scope::for_actor(s, 1.into(), 1.into()).await.unwrap();
    let _ = update(
        State(s.clone()),
        scope,
        Path(("1".into(), 4.into())),
        Json(Body {
            config,
            version: row.version,
        }),
    )
    .await
    .unwrap();
    decode(
        &configs::Entity::find_by_id(4)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap()
}

fn dedicated_local_database(raw: &str) -> reqwest::Url {
    let url = reqwest::Url::parse(raw).expect("valid dedicated test database URL");
    assert!(
        matches!(url.scheme(), "postgres" | "postgresql" | "mysql"),
        "only PostgreSQL/MySQL protocols are supported"
    );
    assert!(
        matches!(
            url.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
        ),
        "test database must be on localhost"
    );
    let name = url.path().strip_prefix('/').unwrap_or_default();
    assert!(
        name.starts_with("ferry_contribution_test_")
            && name.len() > 24
            && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'),
        "test database name must start ferry_contribution_test_ and contain only ASCII letters/digits/underscores"
    );
    assert!(
        !url.query_pairs().any(|(k, _)| matches!(
            k.as_ref(),
            "host" | "hostaddr" | "socket" | "dbname" | "database"
        )),
        "connection options must not override the local test database"
    );
    url
}

#[tokio::test]
#[ignore = "requires FERRY_CONTRIBUTION_TEST_DATABASE_URL pointing to a dedicated empty local PostgreSQL or MySQL-protocol database"]
async fn real_database_migrations_identity_reports_and_cluster_queue() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("ferry_server=debug")
        .try_init();
    let raw = std::env::var("FERRY_CONTRIBUTION_TEST_DATABASE_URL")
        .expect("set FERRY_CONTRIBUTION_TEST_DATABASE_URL to a NEW empty dedicated local database");
    let url = dedicated_local_database(&raw);
    let mut options = ConnectOptions::new(raw);
    options
        .sqlx_logging(false)
        .min_connections(2)
        .max_connections(8);
    let db = Database::connect(crate::db::utc_options(options))
        .await
        .expect("connect dedicated local test database");
    let backend = db.get_database_backend();
    // Hold multiple transactions so these checks exercise distinct pooled
    // connections, even when the database or URL requests a non-UTC timezone.
    let mut connections = Vec::new();
    for _ in 0..4 {
        let connection = db.begin().await.unwrap();
        let (session_sql, expected) = if backend == DbBackend::Postgres {
            ("SELECT current_setting('TimeZone') AS timezone", "UTC")
        } else {
            ("SELECT @@session.time_zone AS timezone", "+00:00")
        };
        let row = connection
            .query_one_raw(Statement::from_string(backend, session_sql))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<String>("", "timezone").unwrap(), expected);
        let epoch_sql = if backend == DbBackend::Postgres {
            "SELECT EXTRACT(EPOCH FROM '2024-01-01 00:00:00'::timestamptz)::bigint AS epoch"
        } else {
            "SELECT CAST(UNIX_TIMESTAMP('2024-01-01 00:00:00') AS SIGNED) AS epoch"
        };
        let row = connection
            .query_one_raw(Statement::from_string(backend, epoch_sql))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<i64>("", "epoch").unwrap(), 1_704_067_200);
        connections.push(connection);
    }
    for connection in connections {
        connection.rollback().await.unwrap();
    }
    eprintln!("PASS UTC sessions and midnight epoch on four distinct pooled connections");
    let empty_sql = match backend {
        DbBackend::Postgres => {
            "SELECT COUNT(*) AS total FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog','information_schema')"
        }
        DbBackend::MySql => {
            "SELECT COUNT(*) AS total FROM information_schema.tables WHERE table_schema = DATABASE()"
        }
        _ => panic!("runtime coverage requires PostgreSQL or MySQL protocol"),
    };
    let existing = db
        .query_one_raw(Statement::from_string(backend, empty_sql))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        existing.try_get::<i64>("", "total").unwrap(),
        0,
        "refusing a nonempty database; this test never drops or clears a database"
    );
    let version_sql = if backend == DbBackend::Postgres {
        "SELECT version() AS version"
    } else {
        "SELECT VERSION() AS version"
    };
    let engine = db
        .query_one_raw(Statement::from_string(backend, version_sql))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "version")
        .unwrap();
    eprintln!(
        "real contribution runtime: engine={engine}; database={}",
        url.path()
    );
    // Start from the real historical VARCHAR(512), then run the actual widening
    // migration rather than testing only newly created schemas or generated DDL.
    use crate::{
        entities::repository_events,
        i18n::{self, Locale, Message},
    };
    let migrations = crate::migration::Migrator::migrations();
    let delivery_upgrade = migrations
        .iter()
        .position(|migration| migration.name() == "repository_event_delivery_messages_v1")
        .unwrap();
    crate::migration::Migrator::up(&db, Some(delivery_upgrade as u32))
        .await
        .expect("create historical delivery schema");
    let length_sql = if backend == DbBackend::Postgres {
        "SELECT character_maximum_length::bigint AS capacity FROM information_schema.columns WHERE table_schema = 'public' AND table_name = 'repository_events' AND column_name = 'delivery'"
    } else {
        "SELECT CAST(character_maximum_length AS SIGNED) AS capacity FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = 'repository_events' AND column_name = 'delivery'"
    };
    let capacity = db
        .query_one_raw(Statement::from_string(backend, length_sql))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(capacity.try_get::<i64>("", "capacity").unwrap(), 512);
    let legacy = repository_events::Model {
        id: 900001,
        workspace_id: 1,
        connection_id: 1,
        delivery_key: "legacy-delivery-migration".into(),
        event: "push".into(),
        repository: "user/repository".into(),
        reference: "main".into(),
        actor: String::new(),
        action: "user action".into(),
        delivery: "旧状态 raw \"quoted\" 사용자".into(),
        created_at: 0,
        detail: None,
    };
    repository_events::Entity::insert(legacy.clone().into_active_model())
        .exec(&db)
        .await
        .unwrap();
    crate::migration::Migrator::up(&db, None)
        .await
        .expect("execute every existing migration against real engine");
    migrations[delivery_upgrade]
        .up(&sea_orm_migration::SchemaManager::new(&db))
        .await
        .expect("delivery widening repeats after interrupted DDL");
    assert_eq!(
        repository_events::Entity::find_by_id(legacy.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap(),
        legacy
    );
    let diagnostic = format!("{}{}", "\"".repeat(150), "漢사용자".repeat(50));
    let message = Message::new("webhook.failed")
        .with("provider", "Google Chat")
        .with_message(
            "reason",
            Message::new("webhook.code")
                .with("code", i64::MIN.to_string())
                .with("detail", &diagnostic),
        );
    let stored = i18n::store_message(&message);
    assert!(stored.chars().count() > 512);
    let mut typed = legacy.clone();
    typed.id += 1;
    typed.delivery_key = "typed-delivery-migration".into();
    typed.delivery = stored.clone();
    repository_events::Entity::insert(typed.clone().into_active_model())
        .exec(&db)
        .await
        .unwrap();
    let actual = repository_events::Entity::find_by_id(typed.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(actual, typed);
    for locale in Locale::ALL {
        let rendered = i18n::render_stored(locale, &actual.delivery);
        assert!(rendered.contains(&diagnostic));
        assert_eq!(rendered, i18n::render(locale, &message));
        assert!(!rendered.contains("$system_message"));
    }
    assert_ne!(
        i18n::render_stored(Locale::EnUs, &stored),
        i18n::render_stored(Locale::JaJp, &stored)
    );
    eprintln!(
        "PASS historical delivery VARCHAR(512) widening + legacy preservation + >512 quoted Unicode typed value for six readers"
    );
    crate::migration::Migrator::up(&db, None)
        .await
        .expect("migrations repeat safely");
    let (_dir, s) = fixture_with_database(db).await;
    identity::recover(&s).await.unwrap();
    identity::recover(&s).await.unwrap();
    assert_eq!(
        decode(
            &configs::Entity::find_by_id(4)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap()
        .identity_version,
        1
    );
    eprintln!("PASS actual migrations + idempotent identity recovery");

    let claim = tests::claimed(&s).await;
    let day = 86_400_000;
    let start = chrono::NaiveDate::from_ymd_opt(2024, 1, 31)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();
    let mut facts = Vec::new();
    for (index, letter) in ['a', 'b', 'c'].into_iter().enumerate() {
        let mut value = commit(letter);
        value.author_date = start + index as i64 * day;
        value.committer_date = value.author_date + day;
        if index > 0 {
            value.author_name = "Other".into();
            value.author_email = "other@example.test".into();
        }
        facts.push(store(&s, &claim, value).await.unwrap());
    }
    let mut repeated = commit('a');
    repeated.additions = 999;
    assert_eq!(store(&s, &claim, repeated).await.unwrap(), facts[0]);
    assert_eq!(commits::Entity::find().count(&s.db).await.unwrap(), 3);
    publish_branches(
        &s,
        &claim,
        &[
            ("main", &facts[1].sha, &[facts[0].id, facts[1].id]),
            ("feature", &facts[2].sha, &[facts[0].id, facts[2].id]),
        ],
    )
    .await;
    publish_branches(
        &s,
        &claim,
        &[
            ("main", &facts[1].sha, &[facts[0].id, facts[1].id]),
            ("feature", &facts[2].sha, &[facts[0].id, facts[2].id]),
        ],
    )
    .await;
    assert_eq!(membership::Entity::find().count(&s.db).await.unwrap(), 4);
    let overview = read(&s, "overview", &[]).await;
    assert_eq!(overview["items"][0]["commits"], 3);
    assert_eq!(overview["items"][0]["additions"], 36);
    assert_eq!(
        read(&s, "overview", &[("branches", "main,feature")]).await["items"][0]["commits"],
        3
    );
    eprintln!("PASS immutable repeated facts + idempotent branch publication + SHA union");

    for (kind, total) in [
        ("overview", 1),
        ("contributors", 2),
        ("ranking", 2),
        ("branches", 2),
        ("heatmap", 3),
        ("period", 3),
        ("commits", 3),
    ] {
        let first = read(&s, kind, &[("page_size", "1")]).await;
        let second = read(&s, kind, &[("page_size", "1"), ("page", "2")]).await;
        assert_eq!(first["total"], total, "{kind}");
        assert_eq!(first["items"].as_array().unwrap().len(), 1, "{kind}");
        assert_eq!(
            second["items"].as_array().unwrap().len(),
            if total > 1 { 1 } else { 0 },
            "{kind}"
        );
        if total > 1 {
            assert_ne!(first["items"][0]["id"], second["items"][0]["id"], "{kind}");
        }
        if kind == "heatmap" || kind == "period" {
            assert_eq!(first["series"], second["series"], "{kind}");
        }
    }
    assert_eq!(
        read(
            &s,
            "commits",
            &[("since", "2024-02-02"), ("until", "2024-02-02")]
        )
        .await["total"],
        1
    );
    let filtered = read(
        &s,
        "heatmap",
        &[("since", "2024-02-02"), ("until", "2024-02-02")],
    )
    .await;
    assert_eq!(
        filtered["items"][0]["period"], "2024-02-01",
        "filter on committer date; bucket author date"
    );
    for period in ["day", "week", "month"] {
        let value = read(&s, "period", &[("period", period)]).await;
        assert_eq!(
            value["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["commits"].as_i64().unwrap())
                .sum::<i64>(),
            3,
            "{period}"
        );
    }
    assert_eq!(
        read(&s, "commits", &[("commitHash", &facts[1].sha)]).await["total"],
        1
    );
    eprintln!(
        "PASS all seven reports + database pages + author/committer date roles + day/week/month SQL"
    );

    let raw_person = authors::Entity::find_by_id((facts[0].id, 1))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
        .contributor_id;
    let job_before = job(&s, 7).await;
    let source_before = sources::Entity::find_by_id(7)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let mapped = edit_aliases(
        &s,
        vec![Alias {
            name: "Author".into(),
            email: "other@example.test".into(),
            nickname: "Team".into(),
            contributor_id: String::new(),
        }],
    )
    .await;
    let contributor = mapped.aliases[0].contributor_id.clone();
    assert!(!contributor.is_empty());
    assert_eq!(read(&s, "contributors", &[]).await["total"], 1);
    assert_eq!(
        read(&s, "commits", &[("authors", &contributor)]).await["total"],
        3
    );
    let mut aliases = mapped.aliases;
    aliases[0].nickname = "Renamed".into();
    assert_eq!(
        edit_aliases(&s, aliases).await.aliases[0].contributor_id,
        contributor
    );
    assert_eq!(
        job(&s, 7).await,
        job_before,
        "mapping edits must preserve collector lease and queue state"
    );
    assert_eq!(
        sources::Entity::find_by_id(7)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        source_before
    );
    assert_eq!(
        store(&s, &claim, commit('a')).await.unwrap().id,
        facts[0].id,
        "active collector remains authorized"
    );
    edit_aliases(&s, vec![]).await;
    assert_eq!(
        authors::Entity::find_by_id((facts[0].id, 1))
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .contributor_id,
        raw_person
    );
    assert_eq!(
        commits::Entity::find_by_id(facts[0].id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        facts[0]
    );
    publish_branches(&s, &claim, &[("main", &facts[2].sha, &[facts[2].id])]).await;
    assert_eq!(read(&s, "overview", &[]).await["items"][0]["commits"], 3);
    assert_eq!(
        read(&s, "overview", &[("history", "current")]).await["items"][0]["commits"],
        1
    );
    assert_eq!(
        read(
            &s,
            "overview",
            &[("history", "current"), ("branches", "feature")]
        )
        .await["total"],
        0
    );
    finish(&s, &claim, Ok(())).await.unwrap();
    eprintln!(
        "PASS stable contributor rename/unmap + no collector invalidation + rewrite/deleted branch history"
    );

    let second = queued_repository(&s, 17, 16, 2).await;
    let third = queued_repository(&s, 27, 26, 1).await;
    let other = replica(&s);
    dirty(&s, &s.db, 7, 1).await.unwrap();
    let one = NonZeroUsize::new(1).unwrap();
    let (a, b) = tokio::join!(
        try_claim(&s, job(&s, 7).await, one),
        try_claim(&other, second, one)
    );
    let active = a.unwrap().into_iter().chain(b.unwrap()).collect::<Vec<_>>();
    assert_eq!(
        active.len(),
        1,
        "global one-slot cap across concurrent replicas/workspaces"
    );
    assert!(
        try_claim(&other, third.clone(), one)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        locks::Entity::find_by_id("repository-analysis:26")
            .one(&s.db)
            .await
            .unwrap()
            .is_none(),
        "failed capacity claim must roll back repository lock"
    );
    assert_eq!(job(&s, 27).await.state, "queued");
    finish(&s, &active[0], Ok(())).await.unwrap();
    let replacement = try_claim(&other, third, one).await.unwrap().unwrap();
    finish(&other, &replacement, Ok(())).await.unwrap();
    for (id, workspace) in [(7, 1), (17, 2), (27, 1)] {
        dirty(&s, &s.db, id, workspace).await.unwrap();
    }
    let two = NonZeroUsize::new(2).unwrap();
    let (a, b) = tokio::join!(
        try_claim(&s, job(&s, 7).await, two),
        try_claim(&other, job(&s, 17).await, two)
    );
    let first = a.unwrap().unwrap();
    let second = b.unwrap().unwrap();
    assert_ne!(first.slot, second.slot);
    let foreign_fact = store(&other, &second, commit('a')).await.unwrap();
    assert_ne!(
        foreign_fact.id, facts[0].id,
        "same SHA in another repository is a separate fact"
    );
    publish_branches(
        &other,
        &second,
        &[("main", &foreign_fact.sha, &[foreign_fact.id])],
    )
    .await;
    assert_eq!(
        read(&s, "commits", &[]).await["total"],
        3,
        "other workspace facts remain inaccessible"
    );
    let duplicate = queued_repository(&s, 37, 6, 1).await;
    assert!(
        try_claim(&other, duplicate, NonZeroUsize::new(3).unwrap())
            .await
            .unwrap()
            .is_none(),
        "repository mutex applies even with spare capacity"
    );
    assert!(
        try_claim(&other, job(&s, 27).await, two)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        locks::Entity::find_by_id("repository-analysis:26")
            .one(&s.db)
            .await
            .unwrap()
            .is_none()
    );
    renew_claim(&s, &first).await.unwrap();
    finish(&s, &first, Ok(())).await.unwrap();
    let replacement = try_claim(&other, job(&s, 27).await, two)
        .await
        .unwrap()
        .unwrap();
    finish(&other, &replacement, Ok(())).await.unwrap();
    finish(&other, &second, Ok(())).await.unwrap();
    assert_eq!(locks::Entity::find().count(&s.db).await.unwrap(), 0);
    eprintln!(
        "PASS cross-workspace concurrent cap1/cap2 + repository mutex + contention rollback + renewal/release"
    );
    standalone_insight_runtime(&s, &facts).await;
    eprintln!("PASS real contribution runtime complete; test database retained for inspection");
}
