use super::*;
use crate::entities::workspaces;
use sea_orm::PaginatorTrait;
use sea_orm_migration::MigratorTrait;

async fn fixture() -> (tempfile::TempDir, AppState) {
    fixture_at(false).await
}

async fn fixture_at(legacy: bool) -> (tempfile::TempDir, AppState) {
    let (dir, db) = if legacy {
        let dir = tempfile::tempdir().unwrap();
        let db = sea_orm::Database::connect(format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("test.sqlite").display()
        ))
        .await
        .unwrap();
        let count = crate::migration::Migrator::migrations()
            .iter()
            .position(|v| v.name() == "repository_analysis_identity_v1")
            .unwrap();
        crate::migration::Migrator::up(&db, Some(count as u32))
            .await
            .unwrap();
        (dir, db)
    } else {
        crate::lock::tests::database().await
    };
    let s = std::sync::Arc::new(crate::state::State {
        ids: std::sync::Arc::new(
            crate::snowflake::Generator::new(1, tokio::time::Instant::now().into_std()).unwrap(),
        ),
        db,
        data_dir: dir.path().into(),
        instance_id: "identity-test".into(),
        server_mode: false,
        secure_cookie: false,
        bind: "127.0.0.1:0".parse().unwrap(),
        vault: crate::vault::Vault::open(dir.path(), false).unwrap(),
        host_monitor: Default::default(),
        monitor: Default::default(),
        cluster: None,
    });
    let mut workspace = workspaces::Model {
        id: 1,
        slug: "identity-test".into(),
        name: "Identity".into(),
        owner_id: 1,
        join_code: "identity-test".into(),
        pipeline_run_number: 0,
        workflow_execution_number: 0,
        created_at: 0,
        updated_at: 0,
    }
    .into_active_model();
    if legacy {
        // This fixture intentionally stops before the Workflow migration.
        // Do not write a future column into the historical schema under test.
        workspace.workflow_execution_number = sea_orm::ActiveValue::NotSet;
    }
    workspaces::Entity::insert(workspace)
        .exec(&s.db)
        .await
        .unwrap();
    source(&s, 4, 7, 6, 1, vec![]).await;
    (dir, s)
}

fn alias(name: &str, email: &str, nickname: &str) -> Alias {
    Alias {
        name: name.into(),
        email: email.into(),
        nickname: nickname.into(),
        contributor_id: String::new(),
    }
}

async fn source(
    s: &AppState,
    connection: i64,
    source: i64,
    repo: i64,
    user: i64,
    aliases: Vec<Alias>,
) {
    connections::Entity::insert(
        connections::Model {
            id: connection,
            workspace_id: 1,
            user_id: user,
            provider: "github".into(),
            name: format!("connection-{connection}"),
            description: String::new(),
            credential_id: 100,
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
            id: connection,
            workspace_id: 1,
            config: encode(&Config {
                enabled: true,
                aliases,
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
    if repositories::Entity::find_by_id(repo)
        .one(&s.db)
        .await
        .unwrap()
        .is_none()
    {
        repositories::Entity::insert(
            repositories::Model {
                id: repo,
                workspace_id: 1,
                identity: format!("repo-{repo}"),
                provider: "github".into(),
                path: format!("team/repo-{repo}"),
                url: format!("https://example.test/team/repo-{repo}"),
                created_at: 0,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
    }
    sources::Entity::insert(
        sources::Model {
            id: source,
            workspace_id: 1,
            connection_id: connection,
            repository_id: repo,
            remote_path: format!("team/repo-{repo}"),
            coverage_start: 0,
            config_version: 1,
            state: "complete".into(),
            last_error: String::new(),
            synced_at: 100,
        }
        .into_active_model(),
    )
    .exec(&s.db)
    .await
    .unwrap();
}

async fn fact(s: &AppState, id: i64, repo: i64, name: &str, email: &str) -> commits::Model {
    let row = commits::Model {
        id,
        workspace_id: 1,
        repository_id: repo,
        sha: format!("{id:040x}"),
        parents: "[]".into(),
        message: "feat: immutable".into(),
        message_type: "feat".into(),
        author_name: name.into(),
        author_email: email.into(),
        author_date: 1000,
        committer_date: 1000,
        is_merge: 0,
        additions: 10,
        deletions: 2,
        effectives: 8,
        language_stats: "{\".rs\":1}".into(),
        analysis_version: 1,
        complete: 1,
    };
    commits::Entity::insert(row.clone().into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    row
}

async fn setting(s: &AppState, id: i64) -> Config {
    decode(
        &configs::Entity::find_by_id(id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap()
}
async fn save(tx: &impl ConnectionTrait, id: i64, config: &Config) {
    configs::Entity::update_many()
        .col_expr(
            configs::Column::Config,
            Expr::value(encode(config).unwrap()),
        )
        .filter(configs::Column::Id.eq(id))
        .exec(tx)
        .await
        .unwrap();
}
async fn projected(s: &AppState, commit: i64, user: i64) -> authors::Model {
    authors::Entity::find_by_id((commit, user))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn migration_upgrade_and_backfill_are_idempotent_and_preserve_raw_facts() {
    let (_dir, s) = fixture().await;
    let before = vec![
        fact(&s, 10, 6, "Same", "one@example.test").await,
        fact(&s, 11, 6, "Same", "two@example.test").await,
        fact(&s, 12, 6, "Same", "one@example.test").await,
    ];
    backfill(&s).await.unwrap();
    let first = authors::Entity::find()
        .order_by_asc(authors::Column::CommitId)
        .all(&s.db)
        .await
        .unwrap();
    assert_ne!(first[0].contributor_id, first[1].contributor_id);
    assert_eq!(first[0].contributor_id, first[2].contributor_id);
    assert_eq!(identities::Entity::find().count(&s.db).await.unwrap(), 2);
    assert_eq!(people::Entity::find().count(&s.db).await.unwrap(), 2);
    crate::migration::Migrator::up(&s.db, None).await.unwrap();
    backfill(&s).await.unwrap();
    assert_eq!(
        authors::Entity::find()
            .order_by_asc(authors::Column::CommitId)
            .all(&s.db)
            .await
            .unwrap(),
        first
    );
    assert_eq!(
        commits::Entity::find()
            .order_by_asc(commits::Column::Id)
            .all(&s.db)
            .await
            .unwrap(),
        before
    );
    assert_eq!(people::Entity::find().count(&s.db).await.unwrap(), 2);
    let duplicate = identities::Entity::find()
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let mut invalid = duplicate.clone();
    invalid.id = 999;
    assert!(
        identities::Entity::insert(invalid.into_active_model())
            .exec(&s.db)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn legacy_same_nickname_merges_identities_and_rename_retains_stable_id() {
    let (_dir, s) = fixture().await;
    let mut first = setting(&s, 4).await;
    first.aliases = vec![alias("Alice", "", "Team member")];
    save(&s.db, 4, &first).await;
    source(
        &s,
        8,
        9,
        16,
        1,
        vec![alias("", "OTHER@example.test", "Team member")],
    )
    .await;
    fact(&s, 10, 6, "ALICE", "one@example.test").await;
    fact(&s, 11, 16, "Other", "other@example.test").await;
    backfill(&s).await.unwrap();
    let old = setting(&s, 4).await;
    let mapped = projected(&s, 10, 1).await.contributor_id;
    assert_eq!(projected(&s, 11, 1).await.contributor_id, mapped);
    assert_eq!(old.aliases[0].contributor_id, mapped.to_string());
    let raw = people::Entity::find()
        .filter(people::Column::RawIdentityId.is_not_null())
        .filter(people::Column::DisplayName.eq("ALICE"))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
        .id;
    let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
    let mut renamed = vec![alias(" alice ", "", "Renamed")];
    prepare_aliases(&s, &tx, 1, 1, &old.aliases, &mut renamed)
        .await
        .unwrap();
    assert_eq!(renamed[0].contributor_id, mapped.to_string());
    let mut config = old.clone();
    config.aliases = renamed;
    save(&tx, 4, &config).await;
    rebuild(&s, &tx, 1, 1, &[]).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(projected(&s, 10, 1).await.contributor_id, mapped);
    assert_eq!(
        people::Entity::find_by_id(mapped)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .display_name,
        "Renamed"
    );
    let mut references = setting(&s, 8).await.aliases;
    hydrate_aliases(&s.db, 1, 1, &mut references).await.unwrap();
    assert_eq!(references[0].nickname, "Renamed");
    backfill(&s).await.unwrap();
    assert_eq!(
        people::Entity::find_by_id(mapped)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .display_name,
        "Renamed",
        "another connection's old alias label cannot undo a canonical rename"
    );
    let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
    config.aliases.clear();
    save(&tx, 4, &config).await;
    rebuild(&s, &tx, 1, 1, &[6]).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        projected(&s, 10, 1).await.contributor_id,
        raw,
        "removing an alias restores the original raw contributor ID"
    );
    assert_eq!(projected(&s, 11, 1).await.contributor_id, mapped);
}

#[tokio::test]
async fn shared_commit_projection_is_owner_scoped_and_supplied_ids_cannot_cross_owners() {
    let (_dir, s) = fixture().await;
    let mut config = setting(&s, 4).await;
    config.aliases = vec![alias("Author", "", "First")];
    save(&s.db, 4, &config).await;
    source(&s, 8, 9, 6, 2, vec![alias("Author", "", "Second")]).await;
    let fact = fact(&s, 10, 6, "Author", "author@example.test").await;
    backfill(&s).await.unwrap();
    let first = projected(&s, 10, 1).await;
    let second = projected(&s, 10, 2).await;
    assert_eq!(first.identity_id, second.identity_id);
    assert_ne!(first.contributor_id, second.contributor_id);
    let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
    let mut invalid = vec![Alias {
        contributor_id: first.contributor_id.to_string(),
        ..alias("Author", "", "First")
    }];
    assert_eq!(
        prepare_aliases(&s, &tx, 1, 2, &[], &mut invalid)
            .await
            .unwrap_err()
            .0,
        axum::http::StatusCode::NOT_FOUND
    );
    tx.rollback().await.unwrap();
    let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
    index_commit(&s, &tx, &fact).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(authors::Entity::find().count(&s.db).await.unwrap(), 2);
}

#[tokio::test]
async fn startup_conflicts_keep_raw_projection_and_strict_rebuild_rejects_them() {
    let (_dir, s) = fixture().await;
    let mut config = setting(&s, 4).await;
    config.aliases = vec![alias("Author", "", "First")];
    save(&s.db, 4, &config).await;
    source(&s, 8, 9, 6, 1, vec![alias("author", "", "Second")]).await;
    fact(&s, 10, 6, "AUTHOR", "author@example.test").await;
    backfill(&s).await.unwrap();
    let original = projected(&s, 10, 1).await;
    let person = people::Entity::find_by_id(original.contributor_id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert!(person.raw_identity_id.is_some());
    for id in [7, 9] {
        let source = sources::Entity::find_by_id(id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(source.state, "failed");
        assert_eq!(
            crate::i18n::decode_stored(&source.last_error).unwrap().key,
            CONFLICT_KEY
        );
        assert_eq!(
            crate::i18n::render_stored(crate::i18n::Locale::ZhCn, &source.last_error),
            CONFLICT
        );
    }
    let count = people::Entity::find().count(&s.db).await.unwrap();
    backfill(&s).await.unwrap();
    assert_eq!(people::Entity::find().count(&s.db).await.unwrap(), count);
    let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
    assert_eq!(
        rebuild(&s, &tx, 1, 1, &[6]).await.unwrap_err().0,
        axum::http::StatusCode::CONFLICT
    );
    tx.rollback().await.unwrap();
    assert_eq!(projected(&s, 10, 1).await, original);
    let mut config = setting(&s, 8).await;
    config.aliases.clear();
    let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
    save(&tx, 8, &config).await;
    rebuild(&s, &tx, 1, 1, &[6]).await.unwrap();
    tx.commit().await.unwrap();
    assert_ne!(projected(&s, 10, 1).await.contributor_id, person.id);
    assert_eq!(
        sources::Entity::find_by_id(7)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .state,
        "complete"
    );
}

#[tokio::test]
async fn disabled_collection_preserves_projection_but_revoked_or_deselected_sources_do_not() {
    let (_dir, s) = fixture().await;
    fact(&s, 10, 6, "Author", "author@example.test").await;
    backfill(&s).await.unwrap();
    let before = projected(&s, 10, 1).await;
    let mut config = setting(&s, 4).await;
    config.enabled = false;
    save(&s.db, 4, &config).await;
    let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
    rebuild(&s, &tx, 1, 1, &[]).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(projected(&s, 10, 1).await, before);
    sources::Entity::update_many()
        .col_expr(sources::Column::State, Expr::value("unavailable"))
        .filter(sources::Column::Id.eq(7))
        .exec(&s.db)
        .await
        .unwrap();
    let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
    rebuild(&s, &tx, 1, 1, &[]).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(authors::Entity::find().count(&s.db).await.unwrap(), 0);
    sources::Entity::update_many()
        .col_expr(sources::Column::State, Expr::value("complete"))
        .filter(sources::Column::Id.eq(7))
        .exec(&s.db)
        .await
        .unwrap();
    config.all_repositories = false;
    config.repositories.clear();
    save(&s.db, 4, &config).await;
    backfill(&s).await.unwrap();
    assert_eq!(authors::Entity::find().count(&s.db).await.unwrap(), 0);
}

#[tokio::test]
async fn overlapping_name_or_email_rules_cannot_choose_a_person_by_iteration_order() {
    let (_dir, s) = fixture().await;
    let mut config = setting(&s, 4).await;
    config.aliases = vec![
        alias("Name", "", "One"),
        alias("", "mail@example.test", "Two"),
    ];
    save(&s.db, 4, &config).await;
    fact(&s, 10, 6, "NAME", "MAIL@example.test").await;
    backfill(&s).await.unwrap();
    let raw = projected(&s, 10, 1).await.contributor_id;
    assert!(
        people::Entity::find_by_id(raw)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .raw_identity_id
            .is_some()
    );
    let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
    assert!(rebuild(&s, &tx, 1, 1, &[6]).await.is_err());
    tx.rollback().await.unwrap();
    let future = fact(&s, 11, 6, "NAME", "MAIL@example.test").await;
    let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
    assert!(index_commit(&s, &tx, &future).await.is_err());
    tx.rollback().await.unwrap();
    assert!(
        authors::Entity::find_by_id((11, 1))
            .one(&s.db)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn keyset_backfill_projects_more_than_one_batch_without_touching_facts() {
    let (_dir, s) = fixture().await;
    let first = fact(&s, 10, 6, "Author", "author@example.test").await;
    let rows = (11..=520)
        .map(|id| {
            let mut row = first.clone();
            row.id = id;
            row.sha = format!("{id:040x}");
            row.into_active_model()
        })
        .collect::<Vec<_>>();
    for chunk in rows.chunks(200) {
        commits::Entity::insert_many(chunk.to_vec())
            .exec(&s.db)
            .await
            .unwrap();
    }
    backfill(&s).await.unwrap();
    assert_eq!(authors::Entity::find().count(&s.db).await.unwrap(), 511);
    assert_eq!(identities::Entity::find().count(&s.db).await.unwrap(), 1);
    assert_eq!(people::Entity::find().count(&s.db).await.unwrap(), 1);
}

#[tokio::test]
async fn legacy_schema_upgrade_keeps_facts_and_runs_identity_recovery_once() {
    let (_dir, s) = fixture_at(true).await;
    let before = fact(&s, 10, 6, "Author", "author@example.test").await;
    crate::migration::Migrator::up(&s.db, None).await.unwrap();
    recover(&s).await.unwrap();
    let projection = projected(&s, 10, 1).await;
    assert_eq!(setting(&s, 4).await.identity_version, 1);
    s.db.execute_unprepared("CREATE TABLE identity_audit (changes BIGINT NOT NULL)")
        .await
        .unwrap();
    s.db.execute_unprepared("CREATE TRIGGER identity_projection_audit AFTER UPDATE ON repository_analysis_commit_authors BEGIN INSERT INTO identity_audit (changes) VALUES (1); END").await.unwrap();
    recover(&s).await.unwrap();
    let row =
        s.db.query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DbBackend::Sqlite,
            "SELECT COUNT(*) AS total FROM identity_audit",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.try_get::<i64>("", "total").unwrap(),
        0,
        "normal restarts must not rewrite historical projections"
    );
    assert_eq!(projected(&s, 10, 1).await, projection);
    assert_eq!(
        commits::Entity::find_by_id(10)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        before
    );
}

#[tokio::test]
async fn reconnect_detects_and_repairs_history_skipped_during_legacy_recovery() {
    let (_dir, s) = fixture_at(true).await;
    let before = fact(&s, 10, 6, "Author", "author@example.test").await;
    fact(&s, 11, 6, "Other", "other@example.test").await;
    connections::Entity::update_many()
        .col_expr(connections::Column::State, Expr::value("disconnected"))
        .filter(connections::Column::Id.eq(4))
        .exec(&s.db)
        .await
        .unwrap();
    crate::migration::Migrator::up(&s.db, None).await.unwrap();
    recover(&s).await.unwrap();
    assert_eq!(setting(&s, 4).await.identity_version, 1);
    assert_eq!(authors::Entity::find().count(&s.db).await.unwrap(), 0);
    connections::Entity::update_many()
        .col_expr(connections::Column::State, Expr::value("connected"))
        .filter(connections::Column::Id.eq(4))
        .exec(&s.db)
        .await
        .unwrap();
    let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
    assert!(needs_projection(&tx, 1, 1, 6).await.unwrap());
    assert!(!needs_projection(&tx, 2, 1, 6).await.unwrap());
    assert!(!needs_projection(&tx, 1, 1, 99).await.unwrap());
    rebuild(&s, &tx, 1, 1, &[6]).await.unwrap();
    assert!(!needs_projection(&tx, 1, 1, 6).await.unwrap());
    assert!(needs_projection(&tx, 1, 2, 6).await.unwrap());
    tx.commit().await.unwrap();
    assert_eq!(authors::Entity::find().count(&s.db).await.unwrap(), 2);
    assert_eq!(
        commits::Entity::find_by_id(10)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        before
    );
    assert_eq!(
        sources::Entity::find_by_id(7)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .state,
        "complete"
    );
}

#[tokio::test]
async fn connection_visibility_reconciles_rules_and_preserves_raw_person_identity() {
    let (_dir, s) = fixture().await;
    let before = fact(&s, 10, 6, "Author", "author@example.test").await;
    backfill(&s).await.unwrap();
    let raw = projected(&s, 10, 1).await.contributor_id;
    source(&s, 5, 8, 6, 1, vec![]).await;
    let mut config = setting(&s, 4).await;
    let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
    config.aliases = vec![alias("Author", "", "Mapped")];
    prepare_aliases(&s, &tx, 1, 1, &[], &mut config.aliases)
        .await
        .unwrap();
    save(&tx, 4, &config).await;
    reconcile_scope(&s, &tx, 1, 1, &[6]).await.unwrap();
    tx.commit().await.unwrap();
    let mapped = projected(&s, 10, 1).await.contributor_id;
    assert_ne!(raw, mapped);
    for (state, expected) in [("disconnected", raw), ("connected", mapped)] {
        let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
        connections::Entity::update_many()
            .col_expr(connections::Column::State, Expr::value(state))
            .filter(connections::Column::Id.eq(4))
            .exec(&tx)
            .await
            .unwrap();
        reconcile_scope(&s, &tx, 1, 1, &[6]).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(projected(&s, 10, 1).await.contributor_id, expected);
    }
    let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
    config.aliases[0].contributor_id = "999".into();
    save(&tx, 4, &config).await;
    reconcile_scope(&s, &tx, 1, 1, &[6]).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(projected(&s, 10, 1).await.contributor_id, raw);
    for source in [7, 8] {
        let row = sources::Entity::find_by_id(source)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, "failed");
        assert_eq!(
            crate::i18n::decode_stored(&row.last_error).unwrap().key,
            CONFLICT_KEY
        );
        assert_eq!(
            crate::i18n::render_stored(crate::i18n::Locale::ZhCn, &row.last_error),
            CONFLICT
        );
    }
    assert_eq!(
        commits::Entity::find_by_id(10)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        before
    );
}
#[test]
fn persisted_mapping_conflicts_use_identity_and_accept_legacy_rows() {
    let stored = conflict_error().stored();
    assert!(is_mapping_conflict(&stored));
    assert!(is_mapping_conflict("作者映射冲突：待修正映射"));
    assert!(!is_mapping_conflict(&crate::i18n::store_message(
        &crate::i18n::Message::new("repository.analysis.unavailable")
    )));
    assert!(!is_mapping_conflict(&crate::i18n::store_raw(&stored)));
    for locale in crate::i18n::Locale::ALL {
        let displayed = crate::i18n::render_stored(locale, &stored);
        assert!(!displayed.contains("$system_message"));
        assert_eq!(
            crate::i18n::decode_stored(&stored).unwrap().key,
            CONFLICT_KEY
        );
    }
}
