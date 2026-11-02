use super::*;
use crate::entities::workspace_roles;
use crate::repository_analysis::{self as analysis, sync};
use sea_orm::PaginatorTrait;
use sync::tests::{fixture, replica};

async fn setup() -> (tempfile::TempDir, AppState) {
    let (dir, s) = fixture().await;
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
                    "code-insights.sync",
                ])
                .unwrap(),
            ),
        )
        .filter(workspace_roles::Column::Id.eq(2))
        .exec(&s.db)
        .await
        .unwrap();
    let mut row = configs::Entity::find_by_id(4)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    row.config = encode(&Config {
        insights_managed: true,
        all_repositories: false,
        enabled: false,
        ..Default::default()
    })
    .unwrap();
    configs::Entity::update(row.into_active_model().reset_all())
        .exec(&s.db)
        .await
        .unwrap();
    (dir, s)
}

async fn actor(s: &AppState) -> Scope {
    Scope::for_actor(s, 1.into(), 1.into()).await.unwrap()
}

fn request(name: &str, path: &str) -> Value {
    json!({"name":name,"enabled":true,"full_history":true,
        "repositories":[{"connection_id":"4","repository":path}]})
}

async fn new(s: &AppState, value: Value) -> i64 {
    let (_, Json(result)) = create(
        State(s.clone()),
        actor(s).await,
        Json(serde_json::from_value(value).unwrap()),
    )
    .await
    .unwrap();
    result["id"].as_str().unwrap().parse().unwrap()
}

async fn row(s: &AppState, id: i64) -> insights::Model {
    insights::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
}

async fn job(s: &AppState, id: i64) -> jobs::Model {
    jobs::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
}

async fn due(s: &AppState, id: i64, stamp: i64) {
    insights::Entity::update_many()
        .col_expr(insights::Column::NextRunAt, Expr::value(stamp))
        .filter(insights::Column::Id.eq(id))
        .exec(&s.db)
        .await
        .unwrap();
}

async fn edit(s: &AppState, id: i64, mutate: impl FnOnce(&mut Value)) {
    let mut value = get(
        State(s.clone()),
        actor(s).await,
        Path(("1".into(), id.into())),
    )
    .await
    .unwrap()
    .0;
    mutate(&mut value);
    let _ = update(
        State(s.clone()),
        actor(s).await,
        Path(("1".into(), id.into())),
        Json(serde_json::from_value(value).unwrap()),
    )
    .await
    .unwrap();
}

async fn send_event(s: &AppState, path: &str) {
    let connection = connections::Entity::find_by_id(4)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    analysis::event(s, &s.db, &connection, path).await.unwrap();
    analysis::observes(
        s,
        &s.db,
        &connection,
        path,
        &BTreeMap::from([("main".into(), "a".repeat(40))]),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn new_insights_default_to_four_hour_schedules_while_legacy_rows_keep_event_mode() {
    let (_dir, s) = setup().await;
    let default = serde_json::to_value(InsightConfig::default()).unwrap();
    assert_eq!(default["update_mode"], "scheduled");
    assert_eq!(default["schedule"], "0 */4 * * *");
    assert_eq!(default["timezone"], "");
    let id = new(&s, request("Scheduled by default", "team/repo")).await;
    let current = row(&s, id).await;
    assert!(current.next_run_at > db::now());
    assert!(current.next_run_at <= db::now() + 4 * 60 * 60 * 1000);
    let config = serde_json::to_value(decode(&current).unwrap()).unwrap();
    assert_eq!(config["update_mode"], "scheduled");
    let mut old = current.clone();
    let mut legacy: Value = serde_json::from_str(&old.config).unwrap();
    legacy.as_object_mut().unwrap().remove("update_mode");
    legacy.as_object_mut().unwrap().remove("schedule");
    legacy.as_object_mut().unwrap().remove("timezone");
    old.config = encode(&legacy).unwrap();
    assert_eq!(
        serde_json::to_value(decode(&old).unwrap()).unwrap()["update_mode"],
        "event"
    );
    let (_legacy_dir, legacy) = fixture().await;
    recover(&legacy).await.unwrap();
    let imported = row(&legacy, 4).await;
    assert_eq!(imported.next_run_at, 0);
    assert_eq!(
        serde_json::to_value(decode(&imported).unwrap()).unwrap()["update_mode"],
        "event"
    );
    assert!(
        analysis::monitor_subscription(&legacy.db, 4)
            .await
            .unwrap()
            .is_some(),
        "migration preserves the legacy event subscription"
    );
}

#[tokio::test]
async fn invalid_cron_timezone_or_mode_rolls_back_without_touching_collection() {
    let (_dir, s) = setup().await;
    let before_config = configs::Entity::find_by_id(4)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let before_job = job(&s, 7).await;
    for (field, bad) in [
        ("schedule", "this is not cron"),
        ("timezone", "Mars/Unknown"),
        ("update_mode", "immediate"),
    ] {
        let mut value = request("Invalid", "team/repo");
        value[field] = json!(bad);
        assert_eq!(
            create(
                State(s.clone()),
                actor(&s).await,
                Json(serde_json::from_value(value).unwrap())
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::BAD_REQUEST,
            "{field}"
        );
        assert_eq!(insights::Entity::find().count(&s.db).await.unwrap(), 0);
        assert_eq!(selections::Entity::find().count(&s.db).await.unwrap(), 0);
        assert_eq!(
            configs::Entity::find_by_id(4)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap(),
            before_config
        );
        assert_eq!(job(&s, 7).await, before_job);
    }
}

#[tokio::test]
async fn scheduled_scope_initializes_and_manual_sync_works_but_monitor_changes_are_ignored() {
    let (_dir, s) = setup().await;
    let initial = job(&s, 7).await;
    let id = new(&s, request("Scheduled", "team/repo")).await;
    let before = job(&s, 7).await;
    assert!(
        before.generation > initial.generation,
        "initial collection still starts immediately"
    );
    assert!(
        analysis::monitor_subscription(&s.db, 4)
            .await
            .unwrap()
            .is_none()
    );
    send_event(&s, "team/repo").await;
    assert_eq!(job(&s, 7).await, before);
    schedule_due(&s).await.unwrap();
    assert_eq!(
        job(&s, 7).await,
        before,
        "a future schedule cannot enqueue early"
    );
    let next = row(&s, id).await.next_run_at;
    let Json(result) = retry(
        State(s.clone()),
        actor(&s).await,
        Path(("1".into(), id.into())),
    )
    .await
    .unwrap();
    assert_eq!(result["queued"], true);
    assert_eq!(job(&s, 7).await.generation, before.generation + 1);
    assert_eq!(
        row(&s, id).await.next_run_at,
        next,
        "manual sync does not consume the next scheduled run"
    );
}

#[tokio::test]
async fn due_schedule_is_consumed_once_across_replicas_and_survives_restart() {
    let (_dir, s) = setup().await;
    let id = new(&s, request("Due", "team/repo")).await;
    due(&s, id, db::now() - 1).await;
    let before = job(&s, 7).await;
    let other = replica(&s);
    let (a, b) = tokio::join!(schedule_due(&s), schedule_due(&other));
    a.unwrap();
    b.unwrap();
    assert_eq!(
        job(&s, 7).await.generation,
        before.generation + 1,
        "competing replicas must consume a due occurrence once"
    );
    let scheduled = row(&s, id).await;
    assert!(scheduled.next_run_at > db::now());
    let after = job(&s, 7).await;
    schedule_due(&replica(&s)).await.unwrap();
    assert_eq!(row(&s, id).await.next_run_at, scheduled.next_run_at);
    assert_eq!(job(&s, 7).await, after);
    let overlapping = new(&s, request("Also due", "team/repo")).await;
    due(&s, id, 1).await;
    due(&s, overlapping, 1).await;
    let generation = job(&s, 7).await.generation;
    schedule_due(&s).await.unwrap();
    assert_eq!(
        job(&s, 7).await.generation,
        generation + 1,
        "a single scheduler pass coalesces overlapping scopes into one shared-source enqueue"
    );
    assert!(row(&s, id).await.next_run_at > db::now());
    assert!(row(&s, overlapping).await.next_run_at > db::now());
}

#[tokio::test]
async fn overdue_restart_coalesces_missed_occurrences_and_paused_scopes_do_not_run() {
    let (_dir, s) = setup().await;
    let id = new(&s, request("Overdue", "team/repo")).await;
    due(&s, id, db::now() - 10 * 86_400_000).await;
    let before = job(&s, 7).await;
    let restarted = replica(&s);
    schedule_due(&restarted).await.unwrap();
    assert_eq!(job(&s, 7).await.generation, before.generation + 1);
    assert!(
        row(&s, id).await.next_run_at > db::now(),
        "missed ticks coalesce and next occurrence starts from now"
    );
    let once = job(&s, 7).await;
    schedule_due(&restarted).await.unwrap();
    assert_eq!(job(&s, 7).await, once);
    edit(&s, id, |v| v["enabled"] = json!(false)).await;
    due(&s, id, 1).await;
    let paused = job(&s, 7).await;
    schedule_due(&restarted).await.unwrap();
    assert_eq!(job(&s, 7).await, paused);
}

#[tokio::test]
async fn shared_event_and_scheduled_scopes_reuse_one_job_and_mode_edits_do_not_rescan() {
    let (_dir, s) = setup().await;
    let scheduled = new(&s, request("Scheduled", "team/repo")).await;
    configs::Entity::update_many()
        .col_expr(
            configs::Column::NextReconcileAt,
            Expr::value(db::now() + 86_400_000),
        )
        .filter(configs::Column::Id.eq(4))
        .exec(&s.db)
        .await
        .unwrap();
    let before = job(&s, 7).await;
    let source_before = sources::Entity::find_by_id(7)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let config_before = configs::Entity::find_by_id(4)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let mut event = request("Events", "team/repo");
    event["update_mode"] = json!("event");
    let event = new(&s, event).await;
    assert_eq!(row(&s, event).await.next_run_at, 0);
    assert_eq!(jobs::Entity::find().count(&s.db).await.unwrap(), 1);
    assert_eq!(sources::Entity::find().count(&s.db).await.unwrap(), 1);
    assert_eq!(
        job(&s, 7).await,
        before,
        "adding event demand changes subscription, not collection coverage"
    );
    assert!(
        analysis::monitor_subscription(&s.db, 4)
            .await
            .unwrap()
            .is_some()
    );
    edit(&s, scheduled, |v| {
        v["schedule"] = json!("15 */4 * * *");
        v["timezone"] = json!("UTC");
    })
    .await;
    assert_eq!(job(&s, 7).await, before);
    edit(&s, event, |v| v["update_mode"] = json!("scheduled")).await;
    assert!(row(&s, event).await.next_run_at > db::now());
    assert!(
        analysis::monitor_subscription(&s.db, 4)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(job(&s, 7).await, before);
    assert_eq!(
        sources::Entity::find_by_id(7)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        source_before
    );
    let config_after = configs::Entity::find_by_id(4)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        config_after.next_reconcile_at, config_before.next_reconcile_at,
        "cron and mode changes must not trigger discovery"
    );
    assert_eq!(
        analysis::decode(&config_after).unwrap().collection_version,
        analysis::decode(&config_before).unwrap().collection_version
    );
    edit(&s, event, |v| v["update_mode"] = json!("event")).await;
    let queued = job(&s, 7).await.generation;
    send_event(&s, "team/repo").await;
    assert!(
        job(&s, 7).await.generation > queued,
        "event mode must admit monitor changes"
    );
}

#[tokio::test]
async fn due_schedule_retains_unavailable_source_demand_without_exposing_report_membership() {
    let (_dir, s) = setup().await;
    let id = new(&s, request("Unavailable", "team/repo")).await;
    sources::Entity::update_many()
        .col_expr(sources::Column::State, Expr::value("unavailable"))
        .filter(sources::Column::Id.eq(7))
        .exec(&s.db)
        .await
        .unwrap();
    let selected = selected_sources(&s.db, &actor(&s).await, &row(&s, id).await)
        .await
        .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id, 0);
    assert_eq!(selected[0].state, "unavailable");
    let before = job(&s, 7).await;
    due(&s, id, 1).await;
    schedule_due(&s).await.unwrap();
    assert_eq!(
        job(&s, 7).await.generation,
        before.generation + 1,
        "the due demand survives until discovery can confirm restored visibility"
    );
    assert_eq!(
        sources::Entity::find_by_id(7)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .state,
        "unavailable",
        "enqueue never authorizes an unavailable source itself"
    );
    let selected = selected_sources(&s.db, &actor(&s).await, &row(&s, id).await)
        .await
        .unwrap();
    assert_eq!(
        selected[0].id, 0,
        "report selection remains masked after a scheduled enqueue"
    );
}

#[tokio::test]
async fn scheduled_tick_only_dirties_its_selected_path_on_a_shared_connection() {
    let (_dir, s) = setup().await;
    let mut repository = repositories::Entity::find_by_id(6)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    repository.id = 16;
    repository.identity = "scheduled-other-repository".into();
    repository.path = "team/another".into();
    repository.url = "https://example.test/team/another".into();
    repositories::Entity::insert(repository.into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    let mut second = sources::Entity::find_by_id(7)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    second.id = 17;
    second.repository_id = 16;
    second.remote_path = "team/another".into();
    sources::Entity::insert(second.into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    analysis::dirty(&s, &s.db, 17, 1).await.unwrap();
    let a = new(&s, request("A", "team/repo")).await;
    let b = new(&s, request("B", "team/another")).await;
    let first = job(&s, 7).await;
    let second = job(&s, 17).await;
    let other_next = row(&s, b).await.next_run_at;
    due(&s, a, 1).await;
    schedule_due(&s).await.unwrap();
    assert_eq!(job(&s, 7).await.generation, first.generation + 1);
    assert_eq!(
        job(&s, 17).await,
        second,
        "one insight's tick must not refresh unrelated paths"
    );
    assert_eq!(row(&s, b).await.next_run_at, other_next);
}
