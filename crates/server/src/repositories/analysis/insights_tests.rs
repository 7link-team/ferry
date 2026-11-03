use super::*;
use crate::entities::{
    code_insight_people as people, pipelines, repository_analysis_people as legacy_people, users,
    workspace_members, workspace_roles,
};
use crate::repositories::analysis::{self as analysis, sync};
use sea_orm::PaginatorTrait;
use sync::tests::{claimed, commit, fixture, queued_repository};

#[tokio::test]
async fn insight_collection_status_distinguishes_pending_revisions_retries_and_native_fallback() {
    let (_dir, s) = fixture().await;
    grant(&s).await;
    let id = create_one(&s, body("Status", vec![member(4, "team/repo")])).await;
    let mut source = sources::Entity::find_by_id(7)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let mut job = jobs::Entity::find_by_id(7)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let now = db::now();
    source.state = "failed".into();
    source.synced_at = now - 100_000;
    source.last_error = crate::i18n::store_message(&crate::i18n::Message::new(
        "repository.collector.filesMissing",
    ));
    job.state = "queued".into();
    job.generation = 10;
    job.completed_generation = 0;
    job.updated_at = now - 2000;
    job.last_error = source.last_error.clone();
    job.retry_at = now + 60_000;
    sources::Entity::update(source.clone().into_active_model().reset_all())
        .exec(&s.db)
        .await
        .unwrap();
    jobs::Entity::update(job.clone().into_active_model().reset_all())
        .exec(&s.db)
        .await
        .unwrap();
    let detail = detail(&s, id).await;
    let status = &detail["sources"][0];
    assert_eq!(status["pipeline_required"], false);
    assert_eq!(status["state"], "failed");
    assert_eq!(status["synced_at"], source.synced_at);
    assert_eq!(status["job"]["state"], "queued");
    assert_eq!(status["job"]["effective_state"], "retry_wait");
    assert_eq!(status["job"]["updated_at"], job.updated_at);
    assert_eq!(status["job"]["retry_at"], job.retry_at);
    // Native fallback requires no pipeline binding and preserves the retry deadline.
    assert_eq!(
        source_status(&source, Some(&job), true, now)["job"]["effective_state"],
        "retry_wait"
    );
    job.retry_at = 0;
    assert_eq!(
        source_status(&source, Some(&job), true, now)["job"]["effective_state"],
        "queued"
    );
    job.state = "running".into();
    job.lease_until = now + 60_000;
    job.cursor = r#"{"run_id":9007199254740993}"#.into();
    let running = source_status(&source, Some(&job), false, now);
    assert_eq!(running["job"]["effective_state"], "running");
    assert_eq!(running["job"]["pipeline_run_id"], "9007199254740993");
    assert_eq!(running["job"]["lease_until"], job.lease_until);
    // An expired lease cannot continue to present a live worker to the UI.
    assert_eq!(
        source_status(&source, Some(&job), true, job.lease_until)["job"]["effective_state"],
        "queued"
    );
    job.state = "complete".into();
    job.completed_generation = job.generation;
    job.last_error.clear();
    source.last_error.clear();
    source.state = "complete".into();
    assert_eq!(
        source_status(&source, Some(&job), false, now)["job"]["effective_state"],
        "complete"
    );
    assert!(Value::is_null(
        &source_status(&source, None, false, now)["job"]
    ));
    // A transient provider/network error must not be mislabeled as missing setup.
    source.last_error = "Network unavailable".into();
    assert_eq!(
        source_status(&source, Some(&job), false, now)["pipeline_required"],
        false
    );
    source.last_error = crate::i18n::render(
        crate::i18n::Locale::ZhCn,
        &crate::i18n::Message::new("repository.collector.filesMissing"),
    );
    assert_eq!(
        source_status(&source, Some(&job), false, now)["pipeline_required"],
        false
    );
}

async fn grant(s: &AppState) {
    let permissions = [
        "repositories.read",
        "repositories.update",
        "credentials.read",
        "pipelines.run",
        "code-insights.read",
        "code-insights.create",
        "code-insights.update",
        "code-insights.delete",
        "code-insights.sync",
    ];
    workspace_roles::Entity::update_many()
        .col_expr(
            workspace_roles::Column::Permissions,
            Expr::value(encode(&permissions).unwrap()),
        )
        .filter(workspace_roles::Column::Id.eq(2))
        .exec(&s.db)
        .await
        .unwrap();
}

async fn scope(s: &AppState) -> Scope {
    Scope::for_actor(s, 1.into(), 1.into()).await.unwrap()
}

async fn managed(s: &AppState, connection: i64) {
    let mut row = configs::Entity::find_by_id(connection)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    row.config = encode(&Config {
        insights_managed: true,
        enabled: false,
        all_repositories: false,
        ..Default::default()
    })
    .unwrap();
    configs::Entity::update(row.into_active_model().reset_all())
        .exec(&s.db)
        .await
        .unwrap();
}

fn member(connection: i64, repository: &str) -> Selection {
    Selection {
        connection_id: connection.to_string(),
        repository: repository.into(),
        ..Default::default()
    }
}

fn body(name: &str, repositories: Vec<Selection>) -> Body {
    Body {
        name: name.into(),
        description: String::new(),
        version: 0,
        config: InsightConfig {
            full_history: true,
            ..Default::default()
        },
        repositories,
    }
}

async fn create_one(s: &AppState, value: Body) -> i64 {
    let (status, Json(result)) = create(State(s.clone()), scope(s).await, Json(value))
        .await
        .unwrap();
    assert_eq!(status, StatusCode::CREATED);
    result["id"].as_str().unwrap().parse().unwrap()
}

async fn detail(s: &AppState, id: i64) -> Value {
    get(
        State(s.clone()),
        scope(s).await,
        Path(("1".into(), id.into())),
    )
    .await
    .unwrap()
    .0
}

async fn edit(s: &AppState, id: i64, change: impl FnOnce(&mut Body)) -> Value {
    let mut value: Body = serde_json::from_value(detail(s, id).await).unwrap();
    change(&mut value);
    update(
        State(s.clone()),
        scope(s).await,
        Path(("1".into(), id.into())),
        Json(value),
    )
    .await
    .unwrap()
    .0
}

async fn collector(s: &AppState) -> (configs::Model, sources::Model, jobs::Model) {
    (
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
        jobs::Entity::find_by_id(7)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
    )
}

async fn pipeline(s: &AppState, id: i64, workspace: i64) {
    pipelines::Entity::insert(
        pipelines::Model {
            id,
            workspace_id: workspace,
            name: format!("collector-{id}"),
            group_id: 0,
            application_id: 0,
            repository_id: 0,
            definition: String::new(),
            target_kind: "host".into(),
            target_ref: 0,
            description: String::new(),
            created_at: 0,
            updated_at: 0,
            version: 1,
        }
        .into_active_model(),
    )
    .exec(&s.db)
    .await
    .unwrap();
}

#[tokio::test]
async fn insight_crud_checks_versions_permissions_owner_and_workspace() {
    let (_dir, s) = fixture().await;
    grant(&s).await;
    managed(&s, 4).await;
    let id = create_one(&s, body(" First ", vec![member(4, "team/repo")])).await;
    assert_eq!(detail(&s, id).await["name"], "First");
    let listed = list(State(s.clone()), scope(&s).await, Query(HashMap::new()))
        .await
        .unwrap()
        .0;
    assert_eq!(listed["total"], 1);
    let stale: Body = serde_json::from_value(detail(&s, id).await).unwrap();
    edit(&s, id, |b| b.description = "Owned report".into()).await;
    assert_eq!(
        update(
            State(s.clone()),
            scope(&s).await,
            Path(("1".into(), id.into())),
            Json(stale)
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::CONFLICT
    );
    let mut denied = scope(&s).await;
    denied.permissions.remove("code-insights.create");
    assert_eq!(
        create(
            State(s.clone()),
            denied,
            Json(body("Denied", vec![member(4, "team/repo")]))
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::FORBIDDEN
    );
    let mut denied = scope(&s).await;
    denied.permissions.remove("code-insights.update");
    let value = serde_json::from_value(detail(&s, id).await).unwrap();
    assert_eq!(
        update(
            State(s.clone()),
            denied,
            Path(("1".into(), id.into())),
            Json(value)
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::FORBIDDEN
    );
    let mut denied = scope(&s).await;
    denied.permissions.remove("code-insights.delete");
    assert_eq!(
        delete(State(s.clone()), denied, Path(("1".into(), id.into())))
            .await
            .unwrap_err()
            .0,
        StatusCode::FORBIDDEN
    );

    let mut user = users::Entity::find_by_id(1)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    user.id = 99;
    user.email = "other@example.test".into();
    users::Entity::insert(user.into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    let mut membership = workspace_members::Entity::find_by_id(3)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    membership.id = 100;
    membership.user_id = 99;
    workspace_members::Entity::insert(membership.into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    let other = Scope::for_actor(&s, 1.into(), 99.into()).await.unwrap();
    assert_eq!(
        get(State(s.clone()), other, Path(("1".into(), id.into())))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
    let other = Scope::for_actor(&s, 1.into(), 99.into()).await.unwrap();
    assert_eq!(
        list(State(s.clone()), other, Query(HashMap::new()))
            .await
            .unwrap()
            .0["total"],
        0
    );
    queued_repository(&s, 20, 16, 2).await;
    let foreign = Scope::for_actor(&s, 2.into(), 1.into()).await.unwrap();
    assert_eq!(
        delete(State(s.clone()), foreign, Path(("2".into(), id.into())))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
    let _ = delete(
        State(s.clone()),
        scope(&s).await,
        Path(("1".into(), id.into())),
    )
    .await
    .unwrap();
    assert!(
        insights::Entity::find_by_id(id)
            .one(&s.db)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(selections::Entity::find().count(&s.db).await.unwrap(), 0);
}

#[tokio::test]
async fn pre_epoch_insight_filters_shared_unlimited_history() {
    let (_dir, s) = fixture().await;
    grant(&s).await;
    let claim = claimed(&s).await;
    let mut old = commit('a');
    old.author_date = -200;
    old.committer_date = -200;
    let old = sync::store(&s, &claim, old).await.unwrap();
    let mut recent = commit('b');
    recent.author_date = -50;
    recent.committer_date = -50;
    let recent = sync::store(&s, &claim, recent).await.unwrap();
    sync::tests::publish_branches(&s, &claim, &[("main", &recent.sha, &[old.id, recent.id])]).await;
    managed(&s, 4).await;
    let id = create_one(&s, body("Pre-epoch range", vec![member(4, "team/repo")])).await;
    insights::Entity::update_many()
        .col_expr(insights::Column::CoverageStart, Expr::value(-100_i64))
        .filter(insights::Column::Id.eq(id))
        .exec(&s.db)
        .await
        .unwrap();
    let selected = selected_sources(
        &s.db,
        &scope(&s).await,
        &insights::Entity::find_by_id(id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(selected[0].coverage_start, -100);
    let report = analysis::insight_report(
        State(s.clone()),
        scope(&s).await,
        Path(("1".into(), id.into(), "commits".into())),
        Query(HashMap::from([("isMerge".into(), "0,1".into())])),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(report["total"], 1);
    assert_eq!(report["items"][0]["commitHash"], recent.sha);
}

#[tokio::test]
async fn overlapping_insights_reuse_collection_and_local_edits_do_not_requeue() {
    let (_dir, s) = fixture().await;
    grant(&s).await;
    let claim = claimed(&s).await;
    let fact = sync::store(&s, &claim, commit('a')).await.unwrap();
    managed(&s, 4).await;
    let first = create_one(&s, body("All history", vec![member(4, "team/repo")])).await;
    let before = collector(&s).await;
    let mut recent = body("Recent", vec![member(4, "team/repo")]);
    recent.config.full_history = false;
    recent.config.initial_days = 7;
    let second = create_one(&s, recent).await;
    assert_eq!(
        collector(&s).await,
        before,
        "a newer coverage demand reuses the full-history collector unchanged"
    );
    assert_eq!(configs::Entity::find().count(&s.db).await.unwrap(), 1);
    assert_eq!(sources::Entity::find().count(&s.db).await.unwrap(), 1);
    assert_eq!(jobs::Entity::find().count(&s.db).await.unwrap(), 1);
    assert_eq!(detail(&s, first).await["coverage_start"], 0);
    assert!(detail(&s, second).await["coverage_start"].as_i64().unwrap() > 0);
    edit(&s, second, |b| {
        b.name = "Recent team".into();
        b.config.aliases = vec![Alias {
            email: "author@example.test".into(),
            nickname: "Maintainer".into(),
            ..Default::default()
        }];
    })
    .await;
    assert_eq!(
        collector(&s).await,
        before,
        "display identity edits never invalidate shared collection"
    );
    assert_eq!(
        commits::Entity::find_by_id(fact.id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        fact
    );
    let value = detail(&s, second).await;
    assert_eq!(value["aliases"][0]["nickname"], "Maintainer");
    assert_eq!(
        detail(&s, first).await["aliases"],
        json!([]),
        "identity rules belong to one insight"
    );
}

#[tokio::test]
async fn pausing_or_deleting_one_scope_preserves_other_demand_and_last_removal_retains_facts() {
    let (_dir, s) = fixture().await;
    grant(&s).await;
    let fact = sync::store(&s, &claimed(&s).await, commit('b'))
        .await
        .unwrap();
    managed(&s, 4).await;
    let a = create_one(&s, body("A", vec![member(4, "team/repo")])).await;
    let b = create_one(&s, body("B", vec![member(4, "team/repo")])).await;
    let before = collector(&s).await;
    edit(&s, a, |v| v.config.enabled = false).await;
    assert_eq!(
        collector(&s).await,
        before,
        "another enabled insight still owns identical demand"
    );
    let _ = delete(
        State(s.clone()),
        scope(&s).await,
        Path(("1".into(), a.into())),
    )
    .await
    .unwrap();
    assert_eq!(collector(&s).await, before);
    let _ = delete(
        State(s.clone()),
        scope(&s).await,
        Path(("1".into(), b.into())),
    )
    .await
    .unwrap();
    let after = collector(&s).await;
    assert!(!analysis::decode(&after.0).unwrap().enabled);
    assert_eq!(
        after.2, before.2,
        "removing the last demand must not enqueue a collection"
    );
    assert_eq!(
        commits::Entity::find_by_id(fact.id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        fact
    );
    assert_eq!(sources::Entity::find().count(&s.db).await.unwrap(), 1);
}

#[tokio::test]
async fn insight_multi_connection_pipeline_bindings_and_conflicts_are_atomic() {
    let (_dir, s) = fixture().await;
    grant(&s).await;
    managed(&s, 4).await;
    queued_repository(&s, 20, 16, 1).await;
    managed(&s, 21).await;
    pipeline(&s, 501, 1).await;
    pipeline(&s, 502, 1).await;
    let mut one = member(4, "team/repo");
    one.pipeline_id = "501".into();
    let mut two = member(21, "team/repo-16");
    two.pipeline_id = "502".into();
    let mut third = member(4, "team/another");
    third.pipeline_id = "502".into();
    let id = create_one(&s, body("Two connections", vec![one.clone(), two, third])).await;
    let c4 = configs::Entity::find_by_id(4)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let c21 = configs::Entity::find_by_id(21)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        analysis::decode(&c4).unwrap().pipeline_bindings["team/repo"],
        "501"
    );
    assert_eq!(
        analysis::decode(&c4).unwrap().pipeline_bindings["team/another"],
        "502"
    );
    assert_eq!(
        analysis::decode(&c21).unwrap().pipeline_bindings["team/repo-16"],
        "502"
    );
    assert_eq!(
        detail(&s, id).await["repositories"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let jobs_before = jobs::Entity::find()
        .order_by_asc(jobs::Column::Id)
        .all(&s.db)
        .await
        .unwrap();
    one.pipeline_id = "502".into();
    assert_eq!(
        create(
            State(s.clone()),
            scope(&s).await,
            Json(body("Conflict", vec![one]))
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(insights::Entity::find().count(&s.db).await.unwrap(), 1);
    assert_eq!(selections::Entity::find().count(&s.db).await.unwrap(), 3);
    assert_eq!(
        configs::Entity::find_by_id(4)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        c4
    );
    assert_eq!(
        configs::Entity::find_by_id(21)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        c21
    );
    assert_eq!(
        jobs::Entity::find()
            .order_by_asc(jobs::Column::Id)
            .all(&s.db)
            .await
            .unwrap(),
        jobs_before
    );
}

#[tokio::test]
async fn forbidden_selection_rolls_back_scope_and_shared_collection() {
    let (_dir, s) = fixture().await;
    grant(&s).await;
    managed(&s, 4).await;
    queued_repository(&s, 20, 16, 2).await;
    let before = collector(&s).await;
    assert_eq!(
        create(
            State(s.clone()),
            scope(&s).await,
            Json(body(
                "Invalid mixed scope",
                vec![member(4, "team/repo"), member(21, "team/repo-16")]
            ))
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(insights::Entity::find().count(&s.db).await.unwrap(), 0);
    assert_eq!(selections::Entity::find().count(&s.db).await.unwrap(), 0);
    assert_eq!(collector(&s).await, before);
    let mut denied = scope(&s).await;
    denied.permissions.remove("credentials.read");
    assert_eq!(
        create(
            State(s.clone()),
            denied,
            Json(body(
                "No credential permission",
                vec![member(4, "team/repo")]
            ))
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(collector(&s).await, before);
}

#[tokio::test]
async fn legacy_recovery_preserves_mapping_coverage_jobs_and_is_idempotent() {
    let (_dir, s) = fixture().await;
    grant(&s).await;
    legacy_people::Entity::insert(
        legacy_people::Model {
            id: 90,
            workspace_id: 1,
            user_id: 1,
            display_name: "Legacy maintainer".into(),
            version: 3,
            raw_identity_id: None,
        }
        .into_active_model(),
    )
    .exec(&s.db)
    .await
    .unwrap();
    let mut old = configs::Entity::find_by_id(4)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    old.coverage_start = 1_700_000_000_000;
    old.next_reconcile_at = 1_999_000_000_000;
    old.config = encode(&Config {
        enabled: true,
        all_repositories: false,
        repositories: vec!["team/repo".into()],
        pipeline_id: "501".into(),
        aliases: vec![Alias {
            email: "author@example.test".into(),
            nickname: "Legacy maintainer".into(),
            contributor_id: "90".into(),
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap();
    configs::Entity::update(old.clone().into_active_model().reset_all())
        .exec(&s.db)
        .await
        .unwrap();
    let before = collector(&s).await;
    recover(&s).await.unwrap();
    let insight = insights::Entity::find_by_id(4)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(insight.coverage_start, old.coverage_start);
    let config = decode(&insight).unwrap();
    assert_eq!(config.aliases[0].contributor_id, "90");
    assert_eq!(config.aliases[0].nickname, "Legacy maintainer");
    assert_eq!(
        people::Entity::find_by_id((4, 90))
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .version,
        3
    );
    let members = members(&s.db, &insight).await.unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].repository, "team/repo");
    assert_eq!(members[0].pipeline_id, "501");
    let after = collector(&s).await;
    assert_eq!(after.0.coverage_start, before.0.coverage_start);
    assert_eq!(after.0.next_reconcile_at, before.0.next_reconcile_at);
    assert_eq!(after.0.version, before.0.version);
    assert_eq!((&after.1, &after.2), (&before.1, &before.2));
    assert!(analysis::decode(&after.0).unwrap().insights_managed);
    recover(&s).await.unwrap();
    assert_eq!(collector(&s).await, after);
    assert_eq!(insights::Entity::find().count(&s.db).await.unwrap(), 1);
    assert_eq!(
        selections::Entity::find().all(&s.db).await.unwrap(),
        members
    );
}

#[tokio::test]
async fn missing_or_revoked_repository_is_visible_as_unavailable_without_a_usable_source() {
    let (_dir, s) = fixture().await;
    grant(&s).await;
    managed(&s, 4).await;
    let id = create_one(
        &s,
        body(
            "Coverage",
            vec![member(4, "team/repo"), member(4, "team/not-discovered")],
        ),
    )
    .await;
    let value = detail(&s, id).await;
    let rows = value["sources"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let missing = rows
        .iter()
        .find(|r| r["repository"] == "team/not-discovered")
        .unwrap();
    assert_eq!(missing["state"], "unavailable");
    assert_eq!(missing["id"], "0");
    assert!(serde_json::Value::is_null(&missing["job"]));
    connections::Entity::update_many()
        .col_expr(connections::Column::State, Expr::value("revoked"))
        .filter(connections::Column::Id.eq(4))
        .exec(&s.db)
        .await
        .unwrap();
    let value = detail(&s, id).await;
    for row in value["sources"].as_array().unwrap() {
        assert_eq!(row["id"], "0");
        assert_eq!(row["state"], "unavailable");
        assert!(serde_json::Value::is_null(&row["job"]));
    }
    assert_eq!(
        sources::Entity::find().count(&s.db).await.unwrap(),
        1,
        "revocation hides rather than deletes persisted facts and source history"
    );
}

#[tokio::test]
async fn dynamic_group_publication_projects_new_authors_into_its_insight() {
    let (_dir, s) = fixture().await;
    grant(&s).await;
    managed(&s, 4).await;
    let id = create_one(&s, body("Dynamic authors", vec![member(4, "team/*")])).await;
    let claim = claimed(&s).await;
    sync::store(&s, &claim, commit('a')).await.unwrap();
    let source = sources::Entity::find_by_id(7)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    analysis::insight_identity::refresh_repository(&s, &s.db, &source)
        .await
        .unwrap();
    assert_eq!(
        crate::entities::code_insight_identity_links::Entity::find()
            .filter(crate::entities::code_insight_identity_links::Column::InsightId.eq(id))
            .count(&s.db)
            .await
            .unwrap(),
        1,
        "group membership must project authors arriving after insight creation"
    );
}

#[tokio::test]
async fn dynamic_insight_groups_include_future_sources_without_editing_membership() {
    let (_dir, s) = fixture().await;
    grant(&s).await;
    managed(&s, 4).await;
    let id = create_one(&s, body("Dynamic group", vec![member(4, "team/*")])).await;
    let parent = insights::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let actor = scope(&s).await;
    let before = selected_sources(&s.db, &actor, &parent).await.unwrap();
    assert_eq!(
        before
            .iter()
            .map(|r| r.remote_path.as_str())
            .collect::<Vec<_>>(),
        vec!["team/repo"]
    );
    let config = configs::Entity::find_by_id(4)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let config = analysis::decode(&config).unwrap();
    assert!(config.selects("team/future/new"));
    assert!(!config.selects("team-other/repo"));

    let original = before[0].clone();
    for (source_id, repository_id, path) in
        [(41, 42, "team/future/new"), (43, 44, "team-other/repo")]
    {
        let mut repo = repositories::Entity::find_by_id(original.repository_id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        repo.id = repository_id;
        repo.path = path.into();
        repo.identity = format!("future-{repository_id}");
        repositories::Entity::insert(repo.into_active_model())
            .exec(&s.db)
            .await
            .unwrap();
        let mut source = original.clone();
        source.id = source_id;
        source.repository_id = repository_id;
        source.remote_path = path.into();
        sources::Entity::insert(source.into_active_model())
            .exec(&s.db)
            .await
            .unwrap();
    }
    let after = selected_sources(&s.db, &actor, &parent).await.unwrap();
    assert_eq!(
        after
            .iter()
            .map(|r| r.remote_path.as_str())
            .collect::<Vec<_>>(),
        vec!["team/repo", "team/future/new"]
    );
    assert_eq!(members(&s.db, &parent).await.unwrap().len(), 1);
    let mut queued = BTreeSet::new();
    enqueue_scheduled(&s, &s.db, &parent, &mut queued)
        .await
        .unwrap();
    assert!(queued.contains(&41));
    assert!(!queued.contains(&43));
    connections::Entity::update_many()
        .col_expr(connections::Column::State, Expr::value("disconnected"))
        .filter(connections::Column::Id.eq(4))
        .exec(&s.db)
        .await
        .unwrap();
    assert!(
        selected_sources(&s.db, &actor, &parent)
            .await
            .unwrap()
            .iter()
            .all(|r| r.id == 0)
    );
}
