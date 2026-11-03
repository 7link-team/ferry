use super::*;
use crate::entities::{users, workspace_members, workspace_roles, workspaces};
use sea_orm::{PaginatorTrait, TransactionTrait};

#[test]
fn shared_history_demand_unions_wildcard_and_explicit_selection() {
    let mut config = Config::default();
    assert_eq!(config.required_coverage_start("team/repo", 42), 42);
    config.coverage_starts.insert("team/repo".into(), 100);
    assert_eq!(config.required_coverage_start("team/repo", 42), 100);
    config.coverage_starts.insert(String::new(), 0);
    assert_eq!(config.required_coverage_start("team/repo", 42), 0);
    assert_eq!(config.required_coverage_start("team/new", 42), 0);
    config.coverage_starts.insert("team/repo".into(), -100);
    assert_eq!(config.required_coverage_start("team/repo", 42), 0);
    config.coverage_starts.insert(String::new(), 100);
    assert_eq!(config.required_coverage_start("team/repo", 42), -100);
    assert_eq!(coverage_union(-100, 0), 0);
    assert_eq!(coverage_union(0, -100), 0);
    assert_eq!(coverage_intersection(0, -100), -100);
    assert_eq!(coverage_intersection(-100, 0), -100);
    assert_eq!(coverage_intersection(-100, -200), -100);
}

pub(crate) async fn fixture() -> (tempfile::TempDir, AppState) {
    let (dir, db) = crate::lock::tests::database().await;
    fixture_in(dir, db).await
}

pub(super) async fn fixture_with_database(
    db: sea_orm::DatabaseConnection,
) -> (tempfile::TempDir, AppState) {
    fixture_in(tempfile::tempdir().unwrap(), db).await
}

async fn fixture_in(
    dir: tempfile::TempDir,
    db: sea_orm::DatabaseConnection,
) -> (tempfile::TempDir, AppState) {
    let s = std::sync::Arc::new(crate::state::State {
        ids: std::sync::Arc::new(
            crate::snowflake::Generator::new(1, tokio::time::Instant::now().into_std()).unwrap(),
        ),
        db,
        data_dir: dir.path().into(),
        instance_id: "analysis-test".into(),
        server_mode: false,
        secure_cookie: false,
        bind: "127.0.0.1:0".parse().unwrap(),
        vault: crate::vault::Vault::open(dir.path(), false).unwrap(),
        host_monitor: Default::default(),
        cluster: None,
    });
    users::Entity::insert(
        users::Model {
            id: 1,
            email: "author@example.test".into(),
            name: "Author".into(),
            password_hash: String::new(),
            is_superadmin: 0,
            version: 1,
            active: 1,
            created_at: 0,
            updated_at: 0,
        }
        .into_active_model(),
    )
    .exec(&s.db)
    .await
    .unwrap();
    workspaces::Entity::insert(
        workspaces::Model {
            id: 1,
            slug: "analysis-test".into(),
            name: "Analysis".into(),
            owner_id: 1,
            join_code: "analysis-test".into(),
            pipeline_run_number: 0,
            created_at: 0,
            updated_at: 0,
        }
        .into_active_model(),
    )
    .exec(&s.db)
    .await
    .unwrap();
    workspace_roles::Entity::insert(
        workspace_roles::Model {
            id: 2,
            workspace_id: 1,
            name: "Analyst".into(),
            permissions: encode(&[
                "repositories.read",
                "repositories.update",
                "credentials.read",
            ])
            .unwrap(),
            builtin: 0,
            version: 1,
            created_at: 0,
            updated_at: 0,
        }
        .into_active_model(),
    )
    .exec(&s.db)
    .await
    .unwrap();
    workspace_members::Entity::insert(
        workspace_members::Model {
            id: 3,
            workspace_id: 1,
            user_id: 1,
            role_id: 2,
            state: "active".into(),
            version: 1,
            created_at: 0,
            updated_at: 0,
        }
        .into_active_model(),
    )
    .exec(&s.db)
    .await
    .unwrap();
    connections::Entity::insert(
        connections::Model {
            id: 4,
            workspace_id: 1,
            user_id: 1,
            provider: "github".into(),
            name: "test".into(),
            description: String::new(),
            credential_id: 5,
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
    configs::Entity::insert(
        configs::Model {
            id: 4,
            workspace_id: 1,
            config: encode(&Config {
                enabled: true,
                ..Default::default()
            })
            .unwrap(),
            coverage_start: 0,
            next_reconcile_at: 0,
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
    repositories::Entity::insert(
        repositories::Model {
            id: 6,
            workspace_id: 1,
            identity: "test-repository".into(),
            provider: "github".into(),
            path: "team/repo".into(),
            url: "https://example.test/team/repo".into(),
            created_at: 0,
        }
        .into_active_model(),
    )
    .exec(&s.db)
    .await
    .unwrap();
    sources::Entity::insert(
        sources::Model {
            id: 7,
            workspace_id: 1,
            connection_id: 4,
            repository_id: 6,
            remote_path: "team/repo".into(),
            coverage_start: 0,
            config_version: 1,
            state: "pending".into(),
            last_error: String::new(),
            synced_at: 0,
        }
        .into_active_model(),
    )
    .exec(&s.db)
    .await
    .unwrap();
    dirty(&s, &s.db, 7, 1).await.unwrap();
    (dir, s)
}

async fn job(s: &AppState) -> jobs::Model {
    jobs::Entity::find_by_id(7)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
}

pub(crate) async fn claimed(s: &AppState) -> Claim {
    claim(s, job(s).await).await.unwrap().unwrap()
}

pub(crate) fn commit(letter: char) -> Commit {
    Commit {
        sha: letter.to_string().repeat(40),
        message: "feat: contribution fixture".into(),
        author_name: "Author".into(),
        author_email: "author@example.test".into(),
        author_date: 1_730_678_400_000,
        committer_date: 1_730_678_400_000,
        additions: 12,
        deletions: 3,
        complete: true,
        ..Default::default()
    }
}

pub(crate) async fn report_for(s: &AppState, kind: &str, params: &[(&str, &str)]) -> Value {
    // Tests mutate legacy configs directly; production prepares projections at startup/update.
    identity::backfill(s).await.unwrap();
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

pub(crate) async fn publish_branches(
    s: &AppState,
    claim: &Claim,
    branches: &[(&str, &str, &[i64])],
) {
    let old = refs::Entity::find()
        .filter(refs::Column::SourceId.eq(claim.source.id))
        .all(&s.db)
        .await
        .unwrap();
    publish(
        s,
        claim,
        &old,
        branches
            .iter()
            .map(|(name, sha, ids)| {
                (
                    name.to_string(),
                    (sha.to_string(), ids.iter().copied().collect()),
                )
            })
            .collect(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn repeated_fact_and_branch_publication_count_each_sha_once() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    let first = store(&s, &claim, commit('a')).await.unwrap();
    let mut duplicate = commit('a');
    duplicate.additions = 999;
    let repeated = store(&s, &claim, duplicate).await.unwrap();
    assert_eq!(first.id, repeated.id);
    assert_eq!(
        repeated.additions, 12,
        "complete facts are immutable within an analysis version"
    );
    let second = store(&s, &claim, commit('b')).await.unwrap();
    let branches = [
        ("main", first.sha.as_str(), &[first.id][..]),
        ("feature", second.sha.as_str(), &[first.id, second.id][..]),
    ];
    publish_branches(&s, &claim, &branches).await;
    publish_branches(&s, &claim, &branches).await;
    assert_eq!(commits::Entity::find().count(&s.db).await.unwrap(), 2);
    assert_eq!(refs::Entity::find().count(&s.db).await.unwrap(), 2);
    assert_eq!(membership::Entity::find().count(&s.db).await.unwrap(), 3);
    let overview = report_for(&s, "overview", &[]).await;
    assert_eq!(overview["items"][0]["commits"], 2);
    assert_eq!(overview["items"][0]["additions"], 24);
    let selected = report_for(&s, "overview", &[("branches", "main")]).await;
    assert_eq!(selected["items"][0]["commits"], 1);
    let both = report_for(&s, "overview", &[("branches", "main,feature")]).await;
    assert_eq!(both["items"][0]["commits"], 2);
}

#[tokio::test]
async fn branch_rewrite_and_deletion_preserve_history_but_remove_current_membership() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    let a = store(&s, &claim, commit('a')).await.unwrap();
    let b = store(&s, &claim, commit('b')).await.unwrap();
    publish_branches(
        &s,
        &claim,
        &[("main", &a.sha, &[a.id]), ("removed", &a.sha, &[a.id])],
    )
    .await;
    publish_branches(&s, &claim, &[("main", &b.sha, &[b.id])]).await;
    let historical = report_for(&s, "overview", &[]).await;
    assert_eq!(historical["items"][0]["commits"], 2);
    let current = report_for(&s, "overview", &[("history", "current")]).await;
    assert_eq!(current["items"][0]["commits"], 1);
    let deleted = report_for(
        &s,
        "overview",
        &[("history", "current"), ("branches", "removed")],
    )
    .await;
    assert_eq!(deleted["total"], 0);
    let previous = report_for(&s, "overview", &[("branches", "removed")]).await;
    assert_eq!(previous["items"][0]["commits"], 1);
}

#[tokio::test]
async fn new_events_preserve_running_lease_and_advance_generation() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    assert!(super::claim(&s, job(&s).await).await.unwrap().is_none());
    dirty(&s, &s.db, 7, 1).await.unwrap();
    dirty(&s, &s.db, 7, 1).await.unwrap();
    let running = job(&s).await;
    assert_eq!(running.generation, 3);
    assert_eq!(running.completed_generation, 0);
    assert_eq!(running.state, "running");
    assert_eq!(running.lease_owner, claim.owner);
    assert_eq!(
        claim.job.generation, 1,
        "the worker only acknowledges its captured generation"
    );
    let a = store(&s, &claim, commit('a')).await.unwrap();
    publish_branches(&s, &claim, &[("main", &a.sha, &[a.id])]).await;
    let published = refs::Entity::find().one(&s.db).await.unwrap().unwrap();
    assert_eq!(published.generation, 1);
    assert_eq!(
        job(&s).await.generation,
        3,
        "publishing must not erase intervening events"
    );
}

#[tokio::test]
async fn alias_edits_do_not_enqueue_or_invalidate_an_active_collector() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    let before = job(&s).await;
    let source_before = sources::Entity::find_by_id(claim.source.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let mut config = decode(&claim.config).unwrap();
    config.aliases = vec![Alias {
        name: "Author".into(),
        email: String::new(),
        nickname: "Contributor".into(),
        contributor_id: String::new(),
    }];
    let scope = Scope::for_actor(&s, 1.into(), 1.into()).await.unwrap();
    let _ = update(
        State(s.clone()),
        scope,
        Path(("1".into(), 4.into())),
        Json(Body {
            config,
            version: claim.config.version,
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        job(&s).await,
        before,
        "local identity edits must leave the durable collection job untouched"
    );
    assert_eq!(
        sources::Entity::find_by_id(claim.source.id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        source_before
    );
    let updated = configs::Entity::find_by_id(4)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.next_reconcile_at, claim.config.next_reconcile_at);
    assert_eq!(
        collection_version(&updated).unwrap(),
        collection_version(&claim.config).unwrap()
    );
    assert_eq!(updated.version, claim.config.version + 1);
    store(&s, &claim, commit('a')).await.unwrap();
    // A persisted act cursor uses the collector fence, not the editable mapping version.
    let cursor: pipeline::Cursor = serde_json::from_value(json!({"generation":before.generation,
        "config_version":collection_version(&updated).unwrap(),"run_id":12}))
    .unwrap();
    assert_eq!(cursor.config_version, claim.source.config_version);
}

#[tokio::test]
async fn config_version_change_fences_all_fact_and_ref_writes() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    configs::Entity::update_many()
        .col_expr(configs::Column::Version, Expr::value(2_i64))
        .filter(configs::Column::Id.eq(4))
        .exec(&s.db)
        .await
        .unwrap();
    let error = store(&s, &claim, commit('a')).await.unwrap_err();
    assert_eq!(error.0, axum::http::StatusCode::CONFLICT);
    let error = publish(&s, &claim, &[], BTreeMap::new()).await.unwrap_err();
    assert_eq!(error.0, axum::http::StatusCode::CONFLICT);
    assert_eq!(commits::Entity::find().count(&s.db).await.unwrap(), 0);
    assert_eq!(refs::Entity::find().count(&s.db).await.unwrap(), 0);
}

#[tokio::test]
async fn cached_collection_cannot_clear_an_unresolved_identity_mapping_error() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    sources::Entity::update_many()
        .col_expr(sources::Column::State, Expr::value("failed"))
        .col_expr(
            sources::Column::LastError,
            Expr::value("作者映射冲突：待修正映射"),
        )
        .filter(sources::Column::Id.eq(claim.source.id))
        .exec(&s.db)
        .await
        .unwrap();
    assert!(publish(&s, &claim, &[], BTreeMap::new()).await.is_err());
    assert!(store(&s, &claim, commit('a')).await.is_err());
    let source = sources::Entity::find_by_id(claim.source.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(source.state, "failed");
    assert!(source.last_error.starts_with("作者映射冲突："));
    let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
    identity::rebuild(&s, &tx, 1, 1, &[claim.source.repository_id])
        .await
        .unwrap();
    tx.commit().await.unwrap();
    store(&s, &claim, commit('a')).await.unwrap();
}

#[tokio::test]
async fn revoked_credential_permission_fences_a_claimed_worker() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    workspace_roles::Entity::update_many()
        .col_expr(
            workspace_roles::Column::Permissions,
            Expr::value(encode(&["repositories.read"]).unwrap()),
        )
        .filter(workspace_roles::Column::Id.eq(2))
        .exec(&s.db)
        .await
        .unwrap();
    let error = store(&s, &claim, commit('a')).await.unwrap_err();
    assert_eq!(error.0, axum::http::StatusCode::FORBIDDEN);
    assert_eq!(commits::Entity::find().count(&s.db).await.unwrap(), 0);
    lock::release(&s.db, &claim.lock, &claim.owner)
        .await
        .unwrap();
    jobs::Entity::update_many()
        .col_expr(jobs::Column::LeaseUntil, Expr::value(0_i64))
        .filter(jobs::Column::Id.eq(7))
        .exec(&s.db)
        .await
        .unwrap();
    let error = match super::claim(&s, job(&s).await).await {
        Err(error) => error,
        Ok(_) => panic!("a worker without credential permission must not claim a job"),
    };
    assert_eq!(error.0, axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn reports_exclude_other_connection_owners_and_require_read_permission() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    let a = store(&s, &claim, commit('a')).await.unwrap();
    publish_branches(&s, &claim, &[("main", &a.sha, &[a.id])]).await;
    assert_eq!(report_for(&s, "commits", &[]).await["total"], 1);
    connections::Entity::update_many()
        .col_expr(connections::Column::UserId, Expr::value(99_i64))
        .filter(connections::Column::Id.eq(4))
        .exec(&s.db)
        .await
        .unwrap();
    assert_eq!(report_for(&s, "commits", &[]).await["total"], 0);
    let mut scope = Scope::for_actor(&s, 1.into(), 1.into()).await.unwrap();
    scope.permissions.clear();
    let error = report(
        State(s.clone()),
        scope,
        Path(("1".into(), "overview".into())),
        Query(HashMap::new()),
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, axum::http::StatusCode::FORBIDDEN);
}

fn artifact(
    claim: &Claim,
    head: &str,
    members: Vec<String>,
    commits: Vec<Commit>,
) -> ferry_protocol::contribution::Artifact {
    ferry_protocol::contribution::Artifact {
        schema_version: ferry_protocol::contribution::SCHEMA_VERSION,
        analysis_version: ANALYSIS_VERSION,
        task_id: claim.source.id.to_string(),
        generation: claim.job.generation,
        repository: claim.source.remote_path.clone(),
        coverage_start: claim.source.coverage_start,
        refs: BTreeMap::from([("main".into(), head.into())]),
        ref_commits: BTreeMap::from([("main".into(), members)]),
        commits,
        complete: true,
    }
}

#[tokio::test]
async fn importer_rejects_wrong_task_generation_and_missing_ancestry_without_publication() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    let a = commit('a');
    let mut value = artifact(&claim, &a.sha, vec![a.sha.clone()], vec![a.clone()]);
    value.task_id = "another-task".into();
    assert!(pipeline::import(&s, &claim, value).await.is_err());
    let mut value = artifact(&claim, &a.sha, vec![a.sha.clone()], vec![a.clone()]);
    value.generation += 1;
    assert!(pipeline::import(&s, &claim, value).await.is_err());
    assert_eq!(commits::Entity::find().count(&s.db).await.unwrap(), 0);
    let mut b = commit('b');
    b.parents.push(a.sha.clone());
    let missing_parent = artifact(&claim, &b.sha, vec![b.sha.clone()], vec![b.clone()]);
    assert!(pipeline::import(&s, &claim, missing_parent).await.is_err());
    assert_eq!(refs::Entity::find().count(&s.db).await.unwrap(), 0);
    assert_eq!(report_for(&s, "commits", &[]).await["total"], 0);
    let incomplete_set = artifact(&claim, &b.sha, vec![b.sha.clone()], vec![a]);
    assert!(pipeline::import(&s, &claim, incomplete_set).await.is_err());
    assert_eq!(refs::Entity::find().count(&s.db).await.unwrap(), 0);
    let complete_set = artifact(&claim, &b.sha, vec![b.sha.clone(), "a".repeat(40)], vec![]);
    pipeline::import(&s, &claim, complete_set).await.unwrap();
    assert_eq!(commits::Entity::find().count(&s.db).await.unwrap(), 2);
    assert_eq!(report_for(&s, "commits", &[]).await["total"], 2);
}

#[tokio::test]
async fn importer_traverses_old_metadata_to_find_newer_ancestor_despite_timestamp_skew() {
    let (_dir, s) = fixture().await;
    sources::Entity::update_many()
        .col_expr(sources::Column::CoverageStart, Expr::value(100_i64))
        .filter(sources::Column::Id.eq(7))
        .exec(&s.db)
        .await
        .unwrap();
    let claim = claimed(&s).await;
    let mut ancestor = commit('a');
    ancestor.committer_date = 200;
    let mut old = commit('b');
    old.committer_date = 50;
    old.complete = false;
    old.parents.push(ancestor.sha.clone());
    let mut head = commit('c');
    head.committer_date = 300;
    head.parents.push(old.sha.clone());
    let value = artifact(
        &claim,
        &head.sha,
        vec![head.sha.clone(), ancestor.sha.clone()],
        vec![head.clone(), old, ancestor],
    );
    pipeline::import(&s, &claim, value).await.unwrap();
    assert_eq!(commits::Entity::find().count(&s.db).await.unwrap(), 3);
    assert_eq!(membership::Entity::find().count(&s.db).await.unwrap(), 2);
    let result = report_for(&s, "commits", &[]).await;
    assert_eq!(result["total"], 2);
    assert_eq!(result["items"][0]["commitHash"], head.sha);
    assert_eq!(result["items"][0]["branches"], json!(["main"]));
    assert_eq!(
        result["items"][0]["commitUrl"],
        format!("https://example.test/team/repo/commit/{}", head.sha)
    );
}

#[tokio::test]
async fn all_aggregate_reports_paginate_while_chart_series_remains_complete() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    let a = store(&s, &claim, commit('a')).await.unwrap();
    let mut next = commit('b');
    next.author_name = "Another author".into();
    next.author_date += 86_400_000;
    let b = store(&s, &claim, next).await.unwrap();
    publish_branches(
        &s,
        &claim,
        &[("main", &a.sha, &[a.id]), ("feature", &b.sha, &[b.id])],
    )
    .await;
    for kind in ["contributors", "ranking", "branches", "heatmap", "period"] {
        let first = report_for(&s, kind, &[("page_size", "1")]).await;
        let second = report_for(&s, kind, &[("page_size", "1"), ("page", "2")]).await;
        assert_eq!(first["total"], 2, "{kind}");
        assert_eq!(first["items"].as_array().unwrap().len(), 1, "{kind}");
        assert_eq!(second["page"], 2, "{kind}");
        assert_ne!(first["items"][0]["id"], second["items"][0]["id"], "{kind}");
        if kind == "heatmap" || kind == "period" {
            assert_eq!(first["series"].as_array().unwrap().len(), 2, "{kind}");
            assert_eq!(first["series"], second["series"], "{kind}");
        }
    }
}

async fn additional_source(s: &AppState, repository: i64, alias: &str) -> Claim {
    let mut connection = connections::Entity::find_by_id(4)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    connection.id = 8;
    connections::Entity::insert(connection.into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    let mut config = configs::Entity::find_by_id(4)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    config.id = 8;
    config.config = encode(&Config {
        enabled: true,
        aliases: vec![Alias {
            name: "Author".into(),
            email: String::new(),
            nickname: alias.into(),
            contributor_id: String::new(),
        }],
        ..Default::default()
    })
    .unwrap();
    configs::Entity::insert(config.into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    if repository != 6 {
        let mut repo = repositories::Entity::find_by_id(6)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        repo.id = repository;
        repo.identity = "other-repository".into();
        repo.path = "team/other".into();
        repo.url = "https://example.test/team/other.git".into();
        repositories::Entity::insert(repo.into_active_model())
            .exec(&s.db)
            .await
            .unwrap();
    }
    let mut source = sources::Entity::find_by_id(7)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    source.id = 9;
    source.connection_id = 8;
    source.repository_id = repository;
    if repository != 6 {
        source.remote_path = "team/other".into();
    }
    sources::Entity::insert(source.into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    dirty(s, &s.db, 9, 1).await.unwrap();
    let pending = jobs::Entity::find_by_id(9)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    try_claim(s, pending, std::num::NonZeroUsize::new(2).unwrap())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn duplicate_sources_union_branch_members_and_deduplicate_repository_facts() {
    let (_dir, s) = fixture().await;
    let first = claimed(&s).await;
    let a = store(&s, &first, commit('a')).await.unwrap();
    publish_branches(&s, &first, &[("main", &a.sha, &[a.id])]).await;
    finish(&s, &first, Ok(())).await.unwrap();
    let second = additional_source(&s, 6, "Alias").await;
    let same = store(&s, &second, commit('a')).await.unwrap();
    assert_eq!(a.id, same.id);
    let b = store(&s, &second, commit('b')).await.unwrap();
    publish_branches(&s, &second, &[("main", &b.sha, &[a.id, b.id])]).await;
    let branches = report_for(&s, "branches", &[]).await;
    assert_eq!(branches["total"], 1);
    assert_eq!(branches["items"][0]["commits"], 2);
    assert_eq!(
        report_for(&s, "overview", &[]).await["items"][0]["commits"],
        2
    );
    sources::Entity::update_many()
        .col_expr(sources::Column::State, Expr::value("unavailable"))
        .filter(sources::Column::Id.eq(7))
        .exec(&s.db)
        .await
        .unwrap();
    assert_eq!(
        report_for(&s, "overview", &[]).await["items"][0]["commits"],
        2,
        "another authorized source preserves visibility of shared facts"
    );
    sources::Entity::update_many()
        .col_expr(sources::Column::State, Expr::value("unavailable"))
        .filter(sources::Column::Id.eq(9))
        .exec(&s.db)
        .await
        .unwrap();
    assert_eq!(
        report_for(&s, "commits", &[]).await["total"],
        0,
        "revoked repository sources must not reveal cached facts"
    );
}

#[tokio::test]
async fn commit_aliases_are_repository_scoped_and_match_author_filters() {
    let (_dir, s) = fixture().await;
    let first = claimed(&s).await;
    let a = store(&s, &first, commit('a')).await.unwrap();
    publish_branches(&s, &first, &[("main", &a.sha, &[a.id])]).await;
    let second = additional_source(&s, 10, "Other alias").await;
    let b = store(&s, &second, commit('a')).await.unwrap();
    assert_ne!(
        a.id, b.id,
        "the same SHA in different repositories is a distinct fact"
    );
    publish_branches(&s, &second, &[("main", &b.sha, &[b.id])]).await;
    let original = report_for(&s, "commits", &[("repos", "team/repo")]).await;
    assert_eq!(original["items"][0]["nickname"], "Author");
    let other = report_for(&s, "commits", &[("authors", "Other alias")]).await;
    assert_eq!(other["total"], 1);
    assert_eq!(other["items"][0]["nickname"], "Other alias");
    assert_eq!(other["items"][0]["repository"], "team/other");
    assert_eq!(
        report_for(&s, "contributors", &[("authors", "Other alias")]).await["items"][0]["nickname"],
        "Other alias"
    );
}

#[tokio::test]
async fn deleting_a_connection_removes_its_alias_rules_before_the_response() {
    use crate::entities::repository_analysis_commit_authors as authors;
    let (_dir, s) = fixture().await;
    let first = claimed(&s).await;
    let fact = store(&s, &first, commit('a')).await.unwrap();
    let raw = authors::Entity::find_by_id((fact.id, 1))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
        .contributor_id;
    finish(&s, &first, Ok(())).await.unwrap();
    let second = additional_source(&s, 6, "Mapped contributor").await;
    identity::backfill(&s).await.unwrap();
    let mapped = authors::Entity::find_by_id((fact.id, 1))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
        .contributor_id;
    assert_ne!(raw, mapped);
    finish(&s, &second, Ok(())).await.unwrap();
    workspace_roles::Entity::update_many()
        .col_expr(
            workspace_roles::Column::Permissions,
            Expr::value(
                encode(&[
                    "repositories.read",
                    "repositories.update",
                    "repositories.delete",
                    "credentials.read",
                ])
                .unwrap(),
            ),
        )
        .filter(workspace_roles::Column::Id.eq(2))
        .exec(&s.db)
        .await
        .unwrap();
    let scope = Scope::for_actor(&s, 1.into(), 1.into()).await.unwrap();
    let status = crate::repositories::auth::delete_connection(
        State(s.clone()),
        scope,
        Path(("1".into(), "8".into())),
    )
    .await
    .unwrap();
    assert_eq!(status, axum::http::StatusCode::NO_CONTENT);
    let removed_job = jobs::Entity::find_by_id(9)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        removed_job.generation, removed_job.completed_generation,
        "deleted connections must not leave tasks retrying a missing authorization"
    );
    let restored = authors::Entity::find_by_id((fact.id, 1))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
        .contributor_id;
    assert_eq!(
        restored, raw,
        "the delete route must reconcile remaining sources without a report-side write or later scan"
    );
    assert_eq!(
        commits::Entity::find_by_id(fact.id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        fact
    );
}

#[tokio::test]
async fn incomplete_cached_facts_are_upgraded_in_place() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    let mut partial = commit('a');
    partial.complete = false;
    partial.additions = 0;
    let old = store(&s, &claim, partial).await.unwrap();
    let complete = store(&s, &claim, commit('a')).await.unwrap();
    assert_eq!(old.id, complete.id);
    let saved = cached(&s, 6, &complete.sha).await.unwrap().unwrap();
    assert_eq!(saved.complete, 1);
    assert_eq!(saved.additions, 12);
    assert_eq!(commits::Entity::find().count(&s.db).await.unwrap(), 1);
}

#[tokio::test]
async fn failed_collection_keeps_generation_pending_releases_lease_and_manual_retry_requeues() {
    let (_dir, s) = fixture().await;
    connections::Entity::update_many()
        .col_expr(connections::Column::Provider, Expr::value("git"))
        .filter(connections::Column::Id.eq(4))
        .exec(&s.db)
        .await
        .unwrap();
    assert!(run(&s, job(&s).await).await.is_err());
    let failed = job(&s).await;
    assert_eq!(failed.state, "failed");
    assert_eq!(failed.completed_generation, 0);
    assert_eq!(failed.generation, 1);
    assert!(failed.retry_at > db::now());
    assert!(failed.lease_owner.is_empty());
    assert_eq!(failed.lease_until, 0);
    assert_eq!(locks::Entity::find().count(&s.db).await.unwrap(), 0);
    assert!(
        claim(&s, failed).await.unwrap().is_none(),
        "backoff must stop immediate retries"
    );
    let scope = Scope::for_actor(&s, 1.into(), 1.into()).await.unwrap();
    let response = retry(State(s.clone()), scope, Path(("1".into(), 4.into())))
        .await
        .unwrap()
        .0;
    assert_eq!(response["queued"], true);
    let queued = job(&s).await;
    assert_eq!(queued.state, "queued");
    assert_eq!(queued.generation, 2);
    assert_eq!(queued.retry_at, 0);
    assert_eq!(queued.completed_generation, 0);
}

#[tokio::test]
async fn pipeline_resume_claim_uses_frozen_generation_without_discarding_later_events() {
    let (_dir, s) = fixture().await;
    jobs::Entity::update_many()
        .col_expr(
            jobs::Column::Cursor,
            Expr::value(json!({"generation":1,"config_version":1,"run_id":12}).to_string()),
        )
        .filter(jobs::Column::Id.eq(7))
        .exec(&s.db)
        .await
        .unwrap();
    dirty(&s, &s.db, 7, 1).await.unwrap();
    dirty(&s, &s.db, 7, 1).await.unwrap();
    let resumed = claimed(&s).await;
    assert_eq!(resumed.job.generation, 1);
    let pending = job(&s).await;
    assert_eq!(pending.generation, 3);
    assert_eq!(pending.completed_generation, 0);
    assert_eq!(pending.state, "running");
    assert_eq!(pending.lease_owner, resumed.owner);
}

#[tokio::test]
async fn full_history_includes_pre_epoch_commits() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    let mut value = commit('a');
    value.author_date = -1;
    value.committer_date = -1;
    let a = store(&s, &claim, value).await.unwrap();
    publish_branches(&s, &claim, &[("main", &a.sha, &[a.id])]).await;
    assert_eq!(report_for(&s, "commits", &[]).await["total"], 1);
    let heatmap = report_for(&s, "heatmap", &[]).await;
    assert_eq!(heatmap["items"][0]["period"], "1969-12-31");
    assert_eq!(heatmap["items"][0]["commits"], 1);
}

#[tokio::test]
async fn same_named_authors_remain_distinct_until_explicit_alias_mapping() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    let a = store(&s, &claim, commit('a')).await.unwrap();
    let mut other = commit('b');
    other.author_email = "another@example.test".into();
    let b = store(&s, &claim, other).await.unwrap();
    publish_branches(&s, &claim, &[("main", &b.sha, &[a.id, b.id])]).await;
    for kind in ["contributors", "ranking", "period"] {
        let result = report_for(&s, kind, &[]).await;
        assert_eq!(result["total"], 2, "{kind}");
        assert_ne!(result["items"][0]["id"], result["items"][1]["id"], "{kind}");
        assert_ne!(
            result["items"][0]["authorKey"], result["items"][1]["authorKey"],
            "{kind}"
        );
        assert_eq!(result["items"][0]["nickname"], "Author");
        assert_eq!(result["items"][1]["nickname"], "Author");
    }
    let filtered = report_for(&s, "contributors", &[("authors", "another@example.test")]).await;
    assert_eq!(filtered["total"], 1);
    assert_eq!(filtered["items"][0]["email"], "another@example.test");
    configs::Entity::update_many()
        .col_expr(
            configs::Column::Config,
            Expr::value(
                encode(&Config {
                    enabled: true,
                    aliases: vec![Alias {
                        name: "Author".into(),
                        email: String::new(),
                        nickname: "Combined".into(),
                        contributor_id: String::new(),
                    }],
                    ..Default::default()
                })
                .unwrap(),
            ),
        )
        .filter(configs::Column::Id.eq(4))
        .exec(&s.db)
        .await
        .unwrap();
    for kind in ["contributors", "ranking", "period"] {
        let result = report_for(&s, kind, &[]).await;
        assert_eq!(result["total"], 1, "{kind}");
        assert_eq!(result["items"][0]["nickname"], "Combined", "{kind}");
        assert_eq!(result["items"][0]["commits"], 2, "{kind}");
    }
}

pub(crate) async fn queued_repository(
    s: &AppState,
    source_id: i64,
    repository: i64,
    workspace: i64,
) -> jobs::Model {
    if workspace != 1
        && workspaces::Entity::find_by_id(workspace)
            .one(&s.db)
            .await
            .unwrap()
            .is_none()
    {
        workspaces::Entity::insert(
            workspaces::Model {
                id: workspace,
                slug: format!("workspace-{workspace}"),
                name: "Other workspace".into(),
                owner_id: 1,
                join_code: format!("join-{workspace}"),
                pipeline_run_number: 0,
                created_at: 0,
                updated_at: 0,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
        let mut role = workspace_roles::Entity::find_by_id(2)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        role.id = source_id + 2;
        role.workspace_id = workspace;
        workspace_roles::Entity::insert(role.into_active_model())
            .exec(&s.db)
            .await
            .unwrap();
        workspace_members::Entity::insert(
            workspace_members::Model {
                id: source_id + 3,
                workspace_id: workspace,
                user_id: 1,
                role_id: source_id + 2,
                state: "active".into(),
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
    let mut connection = connections::Entity::find_by_id(4)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    connection.id = source_id + 1;
    connection.workspace_id = workspace;
    connections::Entity::insert(connection.into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    let mut config = configs::Entity::find_by_id(4)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    config.id = source_id + 1;
    config.workspace_id = workspace;
    configs::Entity::insert(config.into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    if repositories::Entity::find_by_id(repository)
        .one(&s.db)
        .await
        .unwrap()
        .is_none()
    {
        repositories::Entity::insert(
            repositories::Model {
                id: repository,
                workspace_id: workspace,
                identity: format!("repo-{repository}"),
                provider: "github".into(),
                path: format!("team/repo-{repository}"),
                url: format!("https://example.test/team/repo-{repository}"),
                created_at: 0,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
    }
    let mut source = sources::Entity::find_by_id(7)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    source.id = source_id;
    source.workspace_id = workspace;
    source.connection_id = source_id + 1;
    source.repository_id = repository;
    source.remote_path = format!("team/repo-{repository}");
    sources::Entity::insert(source.into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    dirty(s, &s.db, source_id, workspace).await.unwrap();
    jobs::Entity::find_by_id(source_id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
}

pub(crate) fn replica(s: &AppState) -> AppState {
    std::sync::Arc::new(crate::state::State {
        ids: std::sync::Arc::new(
            crate::snowflake::Generator::new(2, tokio::time::Instant::now().into_std()).unwrap(),
        ),
        db: s.db.clone(),
        data_dir: s.data_dir.clone(),
        instance_id: "analysis-other-replica".into(),
        server_mode: false,
        secure_cookie: false,
        bind: "127.0.0.1:0".parse().unwrap(),
        vault: crate::vault::Vault::open(&s.data_dir, false).unwrap(),
        host_monitor: Default::default(),
        cluster: None,
    })
}

#[tokio::test]
async fn concurrency_slots_enforce_cap_across_repositories_workspaces_and_replicas() {
    for limit in [1, 2] {
        let (_dir, s) = fixture().await;
        let other = replica(&s);
        let second_job = queued_repository(&s, 17, 16, 2).await;
        let third_job = queued_repository(&s, 27, 26, 1).await;
        let concurrency = std::num::NonZeroUsize::new(limit).unwrap();
        let (first, second) = tokio::join!(
            try_claim(&s, job(&s).await, concurrency),
            try_claim(&other, second_job, concurrency),
        );
        let mut active = first
            .unwrap()
            .into_iter()
            .chain(second.unwrap())
            .collect::<Vec<_>>();
        assert_eq!(active.len(), limit);
        assert!(
            try_claim(&other, third_job.clone(), concurrency)
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
            "a full queue must roll back its temporary repository lock"
        );
        assert_eq!(
            jobs::Entity::find_by_id(27)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap()
                .state,
            "queued"
        );
        let first = active.pop().unwrap();
        finish(&s, &first, Ok(())).await.unwrap();
        let replacement = try_claim(&other, third_job, concurrency)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            replacement.slot, first.slot,
            "completed jobs release global capacity"
        );
        finish(&other, &replacement, Ok(())).await.unwrap();
        for claim in active {
            finish(&s, &claim, Ok(())).await.unwrap();
        }
        assert_eq!(locks::Entity::find().count(&s.db).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn repository_mutex_serializes_duplicate_sources_even_with_spare_slots() {
    let (_dir, s) = fixture().await;
    let concurrency = std::num::NonZeroUsize::new(2).unwrap();
    let first = try_claim(&s, job(&s).await, concurrency)
        .await
        .unwrap()
        .unwrap();
    let duplicate = queued_repository(&s, 17, 6, 1).await;
    assert!(
        try_claim(&replica(&s), duplicate, concurrency)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        locks::Entity::find_by_id("repository-analysis-slot:1")
            .one(&s.db)
            .await
            .unwrap()
            .is_none()
    );
    let independent = queued_repository(&s, 27, 26, 1).await;
    let second = try_claim(&s, independent, concurrency)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(first.slot, second.slot);
    assert_ne!(first.lock, second.lock);
}

#[tokio::test]
async fn expired_slot_is_reclaimed_and_fences_old_worker_publication_renewal_and_completion() {
    let (_dir, s) = fixture().await;
    let old = claimed(&s).await;
    locks::Entity::update_many()
        .col_expr(locks::Column::ExpiresAt, Expr::value(db::now() - 1))
        .filter(locks::Column::Name.eq(&old.slot))
        .exec(&s.db)
        .await
        .unwrap();
    let pending = queued_repository(&s, 17, 16, 2).await;
    let new = try_claim(
        &replica(&s),
        pending,
        std::num::NonZeroUsize::new(1).unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(old.slot, new.slot);
    assert_ne!(old.owner, new.owner);
    let current_lease = locks::Entity::find_by_id(&new.slot)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert!(renew_claim(&s, &old).await.is_err());
    assert!(store(&s, &old, commit('a')).await.is_err());
    assert!(publish(&s, &old, &[], BTreeMap::new()).await.is_err());
    assert!(finish(&s, &old, Ok(())).await.is_err());
    let unfinished = job(&s).await;
    assert_eq!(unfinished.completed_generation, 0);
    assert_eq!(unfinished.state, "failed");
    assert!(unfinished.lease_owner.is_empty());
    assert!(
        locks::Entity::find_by_id(&old.lock)
            .one(&s.db)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        locks::Entity::find_by_id(&new.slot)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        current_lease,
        "stale cleanup must preserve the replacement worker's lease"
    );
    assert_eq!(commits::Entity::find().count(&s.db).await.unwrap(), 0);
    finish(&s, &new, Ok(())).await.unwrap();
}

#[tokio::test]
async fn heartbeat_waiting_for_sqlite_never_starves_the_transaction_owner() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    let previous = jobs::Entity::find_by_id(7)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
        .lease_until;
    let process = async {
        let tx = s.db.begin().await?;
        tokio::time::sleep(std::time::Duration::from_millis(75)).await;
        tx.commit().await?;
        tokio::time::sleep(std::time::Duration::from_millis(75)).await;
        Ok(())
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        with_heartbeat(&s, &claim, process, std::time::Duration::from_millis(10)),
    )
    .await
    .expect("renewal must keep polling the connection owner")
    .unwrap();
    assert!(
        jobs::Entity::find_by_id(7)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .lease_until
            > previous
    );
    finish(&s, &claim, Ok(())).await.unwrap();
}

#[tokio::test]
async fn heartbeat_renews_both_leases_and_never_resurrects_an_expired_lease() {
    for expired in [false, true] {
        let (_dir, s) = fixture().await;
        let claim = claimed(&s).await;
        let short = db::now() + 10_000;
        locks::Entity::update_many()
            .col_expr(locks::Column::ExpiresAt, Expr::value(short))
            .filter(locks::Column::Name.is_in([claim.lock.clone(), claim.slot.clone()]))
            .exec(&s.db)
            .await
            .unwrap();
        if expired {
            locks::Entity::update_many()
                .col_expr(locks::Column::ExpiresAt, Expr::value(db::now() - 1))
                .filter(locks::Column::Name.eq(&claim.lock))
                .exec(&s.db)
                .await
                .unwrap();
            assert!(renew_claim(&s, &claim).await.is_err());
            assert_eq!(
                locks::Entity::find_by_id(&claim.slot)
                    .one(&s.db)
                    .await
                    .unwrap()
                    .unwrap()
                    .expires_at,
                short,
                "failed renewal must not extend only one of the two leases"
            );
            assert!(finish(&s, &claim, Ok(())).await.is_err());
            assert_eq!(job(&s).await.completed_generation, 0);
        } else {
            renew_claim(&s, &claim).await.unwrap();
            let repo = locks::Entity::find_by_id(&claim.lock)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap();
            let slot = locks::Entity::find_by_id(&claim.slot)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap();
            assert!(repo.expires_at > short);
            assert_eq!(repo.expires_at, slot.expires_at);
            assert_eq!(job(&s).await.lease_until, slot.expires_at);
            finish(&s, &claim, Ok(())).await.unwrap();
        }
    }
}

#[tokio::test]
async fn repeated_events_preserve_pending_queue_age() {
    let (_dir, s) = fixture().await;
    jobs::Entity::update_many()
        .col_expr(jobs::Column::UpdatedAt, Expr::value(123_i64))
        .filter(jobs::Column::Id.eq(7))
        .exec(&s.db)
        .await
        .unwrap();
    dirty(&s, &s.db, 7, 1).await.unwrap();
    assert_eq!(job(&s).await.updated_at, 123);
    let claim = claimed(&s).await;
    finish(&s, &claim, Ok(())).await.unwrap();
    jobs::Entity::update_many()
        .col_expr(jobs::Column::UpdatedAt, Expr::value(123_i64))
        .filter(jobs::Column::Id.eq(7))
        .exec(&s.db)
        .await
        .unwrap();
    dirty(&s, &s.db, 7, 1).await.unwrap();
    assert!(
        job(&s).await.updated_at > 123,
        "a newly pending job gets a fresh queue age"
    );
}

#[test]
fn analysis_concurrency_startup_rejects_zero_and_invalid_values() {
    use clap::Parser;
    for value in ["0", "-1", "not-a-number"] {
        assert!(
            crate::Config::try_parse_from(["server", "--repository-analysis-concurrency", value])
                .is_err()
        );
    }
    let config =
        crate::Config::try_parse_from(["server", "--repository-analysis-concurrency", "2"])
            .unwrap();
    assert_eq!(config.repository_analysis_concurrency.get(), 2);
}

#[tokio::test]
async fn failed_worker_finalization_preserves_revoked_source_visibility() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    let a = store(&s, &claim, commit('a')).await.unwrap();
    publish_branches(&s, &claim, &[("main", &a.sha, &[a.id])]).await;
    sources::Entity::update_many()
        .col_expr(sources::Column::State, Expr::value("unavailable"))
        .filter(sources::Column::Id.eq(claim.source.id))
        .exec(&s.db)
        .await
        .unwrap();
    assert!(finish(&s, &claim, Ok(())).await.is_err());
    assert_eq!(
        sources::Entity::find_by_id(claim.source.id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .state,
        "unavailable"
    );
    assert_eq!(report_for(&s, "commits", &[]).await["total"], 0);
    assert_eq!(job(&s).await.completed_generation, 0);
    assert_eq!(locks::Entity::find().count(&s.db).await.unwrap(), 0);
}
