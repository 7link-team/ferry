use super::*;
use chrono::{NaiveDate, TimeZone, Utc};
use sea_orm::Database;
use sea_orm::IntoActiveModel;
use sea_orm_migration::{MigrationTrait, SchemaManager};

fn date(value: &str) -> NaiveDate {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
}
#[test]
fn calendar_bounds_cover_dst_leap_quarters_and_monday_cross_year() {
    let ny = period::timezone("America/New_York").unwrap();
    let (start, end) = period::bounds("day", date("2024-03-10"), ny).unwrap();
    assert_eq!(end - start, 23 * 3_600_000);
    let (start, end) = period::bounds("day", date("2024-11-03"), ny).unwrap();
    assert_eq!(end - start, 25 * 3_600_000);
    let tz = period::timezone("Asia/Kathmandu").unwrap();
    let (start, _) = period::bounds("day", date("2024-02-29"), tz).unwrap();
    assert_eq!(
        Utc.timestamp_millis_opt(start).unwrap().to_rfc3339(),
        "2024-02-28T18:15:00+00:00"
    );
    assert_eq!(
        period::floor(date("2021-01-01"), "week").unwrap(),
        date("2020-12-28")
    );
    let (start, end) = period::bounds("year", date("2024-02-29"), chrono_tz::UTC).unwrap();
    assert_eq!(end - start, 366 * 86_400_000);
    assert_eq!(
        period::advance(
            period::floor(date("2024-12-31"), "quarter").unwrap(),
            "quarter"
        )
        .unwrap(),
        date("2025-01-01")
    );
}
#[test]
fn fixed_schedule_preserves_collection_buffer_and_next_occurrence() {
    let now = Utc
        .with_ymd_and_hms(2024, 2, 29, 23, 0, 0)
        .unwrap()
        .timestamp_millis();
    let periods = vec!["day".into(), "month".into(), "year".into()];
    let next = period::next(&periods, chrono_tz::UTC, now).unwrap();
    assert_eq!(
        next,
        Utc.with_ymd_and_hms(2024, 3, 1, 0, 30, 0)
            .unwrap()
            .timestamp_millis()
    );
    assert_eq!(
        period::next(&periods, chrono_tz::UTC, next).unwrap(),
        Utc.with_ymd_and_hms(2024, 3, 2, 0, 30, 0)
            .unwrap()
            .timestamp_millis()
    );
    assert!(period::timezone("not-a-zone").is_err());
    assert!(period::floor(date("2024-01-01"), "bad").is_err());
}
#[tokio::test]
async fn migration_is_repeatable_and_queue_idempotency_is_enforced_in_sqlite() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);
    migration::InsightReports.up(&manager).await.unwrap();
    migration::InsightReports.up(&manager).await.unwrap();
    let model = runs::ActiveModel {
        id: Set(1),
        workspace_id: Set(5),
        user_id: Set(7),
        definition_id: Set(11),
        period: Set("day".into()),
        period_start: Set(1),
        period_end: Set(2),
        definition_version: Set(1),
        state: Set("queued".into()),
        trigger_kind: Set("scheduled".into()),
        idempotency_key: Set("schedule:11:1:day:1".into()),
        lease_token: Set(String::new()),
        lease_until: Set(0),
        config_snapshot: Set("{}".into()),
        snapshot: Set("{}".into()),
        error: Set(String::new()),
        created_at: Set(0),
        generated_at: Set(0),
    };
    model.clone().insert(&db).await.unwrap();
    let mut duplicate = model;
    duplicate.id = Set(2);
    assert!(duplicate.insert(&db).await.is_err());
    let first = runs::Entity::update_many()
        .col_expr(runs::Column::State, Expr::value("running"))
        .col_expr(runs::Column::LeaseToken, Expr::value("worker-a"))
        .filter(runs::Column::Id.eq(1))
        .filter(runs::Column::State.eq("queued"))
        .exec(&db)
        .await
        .unwrap();
    assert_eq!(first.rows_affected, 1);
    let second = runs::Entity::update_many()
        .col_expr(runs::Column::State, Expr::value("running"))
        .filter(runs::Column::Id.eq(1))
        .filter(runs::Column::State.eq("queued"))
        .exec(&db)
        .await
        .unwrap();
    assert_eq!(second.rows_affected, 0);
    let stale = runs::Entity::update_many()
        .col_expr(runs::Column::State, Expr::value("complete"))
        .filter(runs::Column::Id.eq(1))
        .filter(runs::Column::LeaseToken.eq("worker-b"))
        .exec(&db)
        .await
        .unwrap();
    assert_eq!(stale.rows_affected, 0);
}

async fn fixture() -> (tempfile::TempDir, AppState, Scope, Config) {
    fixture_from_state(analysis::report_test_fixture().await).await
}
async fn fixture_from_state(
    (dir, s): (tempfile::TempDir, AppState),
) -> (tempfile::TempDir, AppState, Scope, Config) {
    use crate::entities::{
        code_insight_people as people, code_insight_repositories as selection,
        code_insights as insight, workspace_roles,
    };
    migration::InsightReports
        .up(&SchemaManager::new(&s.db))
        .await
        .unwrap();
    let permissions = [
        "repositories.read",
        "repositories.update",
        "credentials.read",
        "code-insights.read",
        "insight-reports.read",
        "insight-reports.create",
        "insight-reports.update",
        "insight-reports.delete",
        "insight-reports.generate",
        "deployments.read",
    ];
    crate::entities::credentials::Model {
        id: 5,
        workspace_id: 1,
        name: "Report evidence access".into(),
        scope: "workspace".into(),
        kind: "token".into(),
        username: String::new(),
        description: String::new(),
        secret: String::new(),
        created_at: 0,
        updated_at: 0,
        version: 1,
    }
    .into_active_model()
    .insert(&s.db)
    .await
    .unwrap();
    workspace_roles::Entity::update_many()
        .col_expr(
            workspace_roles::Column::Permissions,
            Expr::value(encode(&permissions).unwrap()),
        )
        .filter(workspace_roles::Column::Id.eq(2))
        .exec(&s.db)
        .await
        .unwrap();
    insight::Model {
        id: 20,
        workspace_id: 1,
        user_id: 1,
        name: "Source".into(),
        description: String::new(),
        config: encode(&json!({"enabled":true,"full_history":true,"aliases":[]})).unwrap(),
        coverage_start: 0,
        next_run_at: 0,
        version: 1,
        created_at: 0,
        updated_at: 0,
    }
    .into_active_model()
    .insert(&s.db)
    .await
    .unwrap();
    selection::Model {
        id: 21,
        workspace_id: 1,
        insight_id: 20,
        connection_id: 4,
        repository: "team/repo".into(),
        all_repositories: 0,
        pipeline_id: String::new(),
    }
    .into_active_model()
    .insert(&s.db)
    .await
    .unwrap();
    people::Model {
        insight_id: 20,
        id: 22,
        workspace_id: 1,
        display_name: "Contributor".into(),
        version: 1,
        raw_identity_id: None,
    }
    .into_active_model()
    .insert(&s.db)
    .await
    .unwrap();
    let scope = Scope::for_actor(&s, 1.into(), 1.into()).await.unwrap();
    let config = Config {
        name: "Daily team".into(),
        insight_id: "20".into(),
        dimension: "person".into(),
        periods: vec!["day".into()],
        author_keys: vec!["22".into()],
        applications: vec![],
        timezone: "Asia/Kathmandu".into(),
        enabled: true,
        version: 0,
    };
    (dir, s, scope, config)
}
#[tokio::test]
async fn crud_revisions_owner_isolation_revocation_and_failed_generation_are_persisted() {
    let (_dir, s, scope, config) = fixture().await;
    let Json(created) = create(State(s.clone()), scope.clone(), Json(config.clone()))
        .await
        .unwrap();
    let id: Id = created["id"].as_str().unwrap().parse().unwrap();
    let mut wrong = scope.clone();
    wrong.user.id = 9.into();
    assert!(owned(&s.db, &wrong, id).await.is_err());
    wrong = scope.clone();
    wrong.workspace_id = 9.into();
    assert!(owned(&s.db, &wrong, id).await.is_err());
    assert!(
        update(
            State(s.clone()),
            scope.clone(),
            Path(("1".into(), id)),
            Json(config.clone())
        )
        .await
        .is_err()
    );
    let Json(queued) = generate(
        State(s.clone()),
        scope.clone(),
        Path(("1".into(), id)),
        Json(Generate {
            period: "day".into(),
            period_start: Some("2024-02-29".into()),
        }),
    )
    .await
    .unwrap();
    let run_id: Id = queued["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(queued["state"], "queued");
    generation::work(&s).await.unwrap();
    let first = load_run(&s, &scope, id, run_id).await.unwrap();
    assert_eq!(first["state"], "complete");
    assert_eq!(first["snapshot"]["coverage"]["complete"], false);
    assert_eq!(first["snapshot"]["config"]["version"], 1);
    let mut changed = config.clone();
    changed.version = 1;
    changed.name = "Renamed report".into();
    update(
        State(s.clone()),
        scope.clone(),
        Path(("1".into(), id)),
        Json(changed),
    )
    .await
    .unwrap();
    assert_eq!(
        load_run(&s, &scope, id, run_id).await.unwrap(),
        first,
        "saved snapshots remain immutable after config updates"
    );
    let Json(second) = generate(
        State(s.clone()),
        scope.clone(),
        Path(("1".into(), id)),
        Json(Generate {
            period: "day".into(),
            period_start: Some("2024-02-29".into()),
        }),
    )
    .await
    .unwrap();
    assert_ne!(second["id"], first["id"]);
    assert_eq!(second["definition_version"], 2);
    use crate::entities::repository_connections as connections;
    connections::Entity::update_many()
        .col_expr(connections::Column::State, Expr::value("disconnected"))
        .filter(connections::Column::Id.eq(4))
        .exec(&s.db)
        .await
        .unwrap();
    assert!(
        load_run(&s, &scope, id, run_id).await.is_err(),
        "history rechecks repository access"
    );
    use crate::entities::workspace_roles;
    workspace_roles::Entity::update_many()
        .col_expr(workspace_roles::Column::Permissions, Expr::value("[]"))
        .filter(workspace_roles::Column::Id.eq(2))
        .exec(&s.db)
        .await
        .unwrap();
    generation::work(&s).await.unwrap();
    let failed = runs::Entity::find_by_id(key(second["id"].as_str().unwrap()).unwrap())
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.state, "failed");
    assert!(!failed.error.is_empty());
}
#[tokio::test]
async fn schedule_catches_up_and_does_not_duplicate_occurrences() {
    let (_dir, s, scope, config) = fixture().await;
    let Json(created) = create(State(s.clone()), scope.clone(), Json(config))
        .await
        .unwrap();
    let id: Id = created["id"].as_str().unwrap().parse().unwrap();
    let mut row = owned(&s.db, &scope, id).await.unwrap();
    let tz = period::timezone("Asia/Kathmandu").unwrap();
    let date = date("2024-02-28");
    row.next_run_at = period::boundary(date, tz).unwrap() + 1_800_000;
    defs::Entity::update_many()
        .col_expr(defs::Column::NextRunAt, Expr::value(row.next_run_at))
        .filter(defs::Column::Id.eq(row.id))
        .exec(&s.db)
        .await
        .unwrap();
    let now = period::boundary(date + chrono::Duration::days(3), tz).unwrap() + 1_800_000;
    schedule(&s, &row, now).await.unwrap();
    schedule(&s, &row, now).await.unwrap();
    let rows = runs::Entity::find()
        .filter(runs::Column::DefinitionId.eq(row.id))
        .all(&s.db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|r| r.trigger_kind == "scheduled"));
    assert!(owned(&s.db, &scope, id).await.unwrap().next_run_at > now);
}

#[tokio::test]
async fn applications_share_evidence_without_double_counting_and_snapshot_mapping_revisions() {
    use crate::entities::{code_insights, deployments, repository_analysis_sources as sources};
    let (_dir, s, scope, mut config) = fixture().await;
    let insight = code_insights::Entity::find_by_id(20)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let at = Utc
        .with_ymd_and_hms(2024, 2, 29, 12, 0, 0)
        .unwrap()
        .timestamp_millis();
    analysis::report_test_seed(&s, &insight, at).await;
    sources::Entity::update_many()
        .col_expr(sources::Column::State, Expr::value("complete"))
        .col_expr(sources::Column::SyncedAt, Expr::value(db::now()))
        .filter(sources::Column::Id.eq(7))
        .exec(&s.db)
        .await
        .unwrap();
    for id in [30, 31] {
        deployments::Model {
            id,
            workspace_id: 1,
            name: format!("Application {id}"),
            category: String::new(),
            manifest: String::new(),
            preview_artifact: String::new(),
            mode: String::new(),
            target_kind: String::new(),
            target_ref: 0,
            workdir: String::new(),
            description: String::new(),
            state: String::new(),
            revision_number: 0,
            current_revision: 0,
            created_at: 0,
            updated_at: 0,
            version: 1,
        }
        .into_active_model()
        .insert(&s.db)
        .await
        .unwrap();
    }
    config.dimension = "application".into();
    config.applications = [30, 31]
        .into_iter()
        .map(|id| Application {
            application_id: id.to_string(),
            repositories: vec![Repository {
                connection_id: "4".into(),
                repository: "team/repo".into(),
            }],
        })
        .collect();
    let Json(created) = create(State(s.clone()), scope.clone(), Json(config))
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().parse().unwrap();
    let Json(queued) = generate(
        State(s.clone()),
        scope.clone(),
        Path(("1".into(), id)),
        Json(Generate {
            period: "day".into(),
            period_start: Some("2024-02-29".into()),
        }),
    )
    .await
    .unwrap();
    generation::work(&s).await.unwrap();
    let run_id = queued["id"].as_str().unwrap().parse().unwrap();
    let first = load_run(&s, &scope, id, run_id).await.unwrap();
    assert_eq!(first["state"], "complete");
    let snapshot = &first["snapshot"];
    assert_eq!(snapshot["summary"]["commits"], 1);
    assert_eq!(snapshot["groups"][0]["commits"], 1);
    assert_eq!(snapshot["groups"][1]["commits"], 1);
    assert_eq!(snapshot["coverage"]["complete"], true);
    assert_eq!(snapshot["commits"].as_array().unwrap().len(), 1);
    assert!(snapshot.get("review_evidence").is_none());
    use crate::entities::code_insight_people as people;
    people::Entity::update_many()
        .col_expr(people::Column::DisplayName, Expr::value("New nickname"))
        .col_expr(people::Column::Version, Expr::value(2_i64))
        .filter(people::Column::InsightId.eq(20))
        .exec(&s.db)
        .await
        .unwrap();
    let Json(queued) = generate(
        State(s.clone()),
        scope.clone(),
        Path(("1".into(), id)),
        Json(Generate {
            period: "day".into(),
            period_start: Some("2024-02-29".into()),
        }),
    )
    .await
    .unwrap();
    generation::work(&s).await.unwrap();
    let second = load_run(
        &s,
        &scope,
        id,
        queued["id"].as_str().unwrap().parse().unwrap(),
    )
    .await
    .unwrap();
    assert_ne!(
        snapshot["identity_version"],
        second["snapshot"]["identity_version"]
    );
    assert_ne!(
        snapshot["commits"][0]["nickname"],
        second["snapshot"]["commits"][0]["nickname"]
    );
    assert_eq!(first, load_run(&s, &scope, id, run_id).await.unwrap());
    let mut legacy: Value = serde_json::from_str(
        &runs::Entity::find_by_id(run_id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .snapshot,
    )
    .unwrap();
    legacy["review_evidence"] = json!({"available": true, "runs": [{"review_id":"50"}]});
    if let Some(groups) = legacy["groups"].as_array_mut() {
        for group in groups {
            group["review_evidence"] = json!({"available":true});
        }
    }
    runs::Entity::update_many()
        .col_expr(
            runs::Column::Snapshot,
            Expr::value(encode(&legacy).unwrap()),
        )
        .filter(runs::Column::Id.eq(run_id))
        .exec(&s.db)
        .await
        .unwrap();
    let historic = load_run(&s, &scope, id, run_id).await.unwrap();
    assert!(historic["snapshot"].get("review_evidence").is_none());
    for group in historic["snapshot"]["groups"].as_array().unwrap() {
        assert!(group.get("review_evidence").is_none());
    }
    crate::entities::credentials::Entity::delete_by_id(5)
        .exec(&s.db)
        .await
        .unwrap();
    assert!(
        load_run(&s, &scope, id, run_id).await.is_err(),
        "historical reports still enforce repository credentials"
    );
}

#[tokio::test]
async fn authenticated_http_routes_generate_read_export_and_reject_revoked_evidence() {
    http_scenario(fixture().await).await;
}
async fn http_scenario((_dir, s, _, mut config): (tempfile::TempDir, AppState, Scope, Config)) {
    use crate::entities::{code_insights, repository_connections as connections, sessions};
    let insight = code_insights::Entity::find_by_id(20)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    analysis::report_test_seed(
        &s,
        &insight,
        Utc.with_ymd_and_hms(2024, 2, 29, 12, 0, 0)
            .unwrap()
            .timestamp_millis(),
    )
    .await;
    let now = db::now();
    let token = "report-http-test";
    sessions::Model {
        id: 40,
        token_hash: crate::hosts::runner::fingerprint(token),
        user_id: 1,
        workspace_id: 1,
        created_at: now,
        seen_at: now,
        expires_at: now + 60_000,
    }
    .into_active_model()
    .insert(&s.db)
    .await
    .unwrap();
    let app = routes()
        .layer(axum::middleware::from_fn(crate::i18n::middleware))
        .with_state(s.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::new();
    let base = format!("http://{address}/1/insight-reports");
    let cookie = format!("{}={token}", crate::accounts::auth::COOKIE);
    assert_eq!(
        client.get(&base).send().await.unwrap().status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let options: Value = client
        .get(format!("{base}/options?insight_id=20"))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!options["people"].as_array().unwrap().is_empty());
    config.author_keys = vec![options["people"][0]["key"].as_str().unwrap().to_owned()];
    let created: Value = client
        .post(&base)
        .header("Cookie", &cookie)
        .json(&config)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap();
    let definition = format!("{base}/{id}");
    let saved: Value = client
        .get(&definition)
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(saved["version"], 1);
    assert_eq!(
        client
            .patch(&definition)
            .header("Cookie", &cookie)
            .json(&config)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::CONFLICT
    );
    config.version = 1;
    config.name = "Updated HTTP report".into();
    let updated: Value = client
        .patch(&definition)
        .header("Cookie", &cookie)
        .json(&config)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["version"], 2);
    let queued: Value = client
        .post(format!("{definition}/generate"))
        .header("Cookie", &cookie)
        .json(&json!({"period":"day","period_start":"2024-02-29"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(queued["state"], "queued");
    tick(&s).await.unwrap();
    let run = format!("{definition}/runs/{}", queued["id"].as_str().unwrap());
    let snapshot: Value = client
        .get(&run)
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(snapshot["state"], "complete");
    assert_eq!(snapshot["snapshot"]["summary"]["commits"], 1);
    let definitions: Value = client
        .get(&base)
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(definitions["total"], 1);
    assert_eq!(definitions["items"][0]["last_run_state"], "complete");
    let history: Value = client
        .get(format!(
            "{definition}/runs?period=day&period_start=2024-02-29"
        ))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(history["total"], 1);
    for format in ["json", "html", "csv"] {
        let response = client
            .get(format!("{run}/export?format={format}"))
            .header("Cookie", &cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert!(
            response
                .headers()
                .get("content-disposition")
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with(format)
        );
        assert!(!response.text().await.unwrap().is_empty());
    }
    let mut invalid = config.clone();
    invalid.timezone = "invalid".into();
    let response = client
        .post(&base)
        .header("Cookie", &cookie)
        .header("Accept-Language", "zh-CN")
        .json(&invalid)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let error: Value = response.json().await.unwrap();
    assert_eq!(error["error"], "报告时区无效");
    connections::Entity::update_many()
        .col_expr(connections::Column::State, Expr::value("disconnected"))
        .filter(connections::Column::Id.eq(4))
        .exec(&s.db)
        .await
        .unwrap();
    for format in ["json", "html", "csv"] {
        assert_eq!(
            client
                .get(format!("{run}/export?format={format}"))
                .header("Cookie", &cookie)
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        client
            .delete(&definition)
            .header("Cookie", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::OK
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires FERRY_REPORT_TEST_DATABASE_URL pointing to a new empty local PostgreSQL/MySQL database named with test"]
async fn real_database_reports_authenticated_http_and_persistent_schedule() {
    use sea_orm_migration::MigratorTrait;
    let raw = std::env::var("FERRY_REPORT_TEST_DATABASE_URL")
        .expect("set FERRY_REPORT_TEST_DATABASE_URL to a new empty local test database");
    let url = reqwest::Url::parse(&raw).expect("valid database URL");
    assert!(matches!(url.scheme(), "postgres" | "postgresql" | "mysql"));
    assert!(
        matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1")),
        "dedicated local database only"
    );
    assert!(
        url.path().contains("test"),
        "database name must include test"
    );
    let db = Database::connect(crate::db::utc_options(sea_orm::ConnectOptions::new(raw)))
        .await
        .unwrap();
    assert!(
        !SchemaManager::new(&db).has_table("users").await.unwrap(),
        "test database must be empty; do not reuse application data"
    );
    crate::migration::Migrator::up(&db, None).await.unwrap();
    let fixture = fixture_from_state(analysis::report_test_fixture_with_database(db).await).await;
    let s = fixture.1.clone();
    let scope = fixture.2.clone();
    let config = fixture.3.clone();
    // The same authenticated HTTP flow runs against each real engine, including
    // collector publication, author-time boundaries, immutable data and exports.
    http_scenario(fixture).await;
    // Restore access removed by the HTTP revocation assertion, then prove fixed
    // schedules persist through a simulated restart and duplicate delivery.
    use crate::entities::repository_connections as connections;
    connections::Entity::update_many()
        .col_expr(connections::Column::State, Expr::value("connected"))
        .filter(connections::Column::Id.eq(4))
        .exec(&s.db)
        .await
        .unwrap();
    let Json(created) = create(State(s.clone()), scope.clone(), Json(config))
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().parse().unwrap();
    let mut row = owned(&s.db, &scope, id).await.unwrap();
    let tz = period::timezone(&decode(&row).unwrap().timezone).unwrap();
    let date = date("2024-02-28");
    row.next_run_at = period::boundary(date, tz).unwrap() + 1_800_000;
    defs::Entity::update_many()
        .col_expr(defs::Column::NextRunAt, Expr::value(row.next_run_at))
        .filter(defs::Column::Id.eq(row.id))
        .exec(&s.db)
        .await
        .unwrap();
    let now = period::boundary(date + chrono::Duration::days(3), tz).unwrap() + 1_800_000;
    schedule(&s, &row, now).await.unwrap();
    schedule(&s, &row, now).await.unwrap();
    assert_eq!(
        runs::Entity::find()
            .filter(runs::Column::DefinitionId.eq(row.id))
            .all(&s.db)
            .await
            .unwrap()
            .len(),
        4
    );
    eprintln!(
        "PASS real report HTTP, evidence exports, revocation and schedule: {:?}",
        s.db.get_database_backend()
    );
}
