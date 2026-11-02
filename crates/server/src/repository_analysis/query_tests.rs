use super::*;
use crate::repository_analysis::{
    identity,
    sync::{
        store,
        tests::{claimed, commit, fixture, publish_branches},
    },
};
use sea_orm::Statement;
use std::sync::{Arc, Mutex};

#[test]
fn insight_projection_is_scoped_and_bound_for_every_database() {
    let cohort = FactCohort::with_projection(
        1,
        vec![7],
        &HashMap::new(),
        Projection::Insight {
            id: 88,
            coverage_start: 12345,
        },
    )
    .unwrap();
    for backend in [DbBackend::Sqlite, DbBackend::Postgres, DbBackend::MySql] {
        let statement = backend.build(&cohort.query);
        assert!(
            statement
                .sql
                .contains("repository_analysis_commit_identities")
        );
        assert!(statement.sql.contains("code_insight_identity_links"));
        assert!(statement.sql.contains("code_insight_people"));
        assert!(!statement.sql.contains("repository_analysis_commit_authors"));
        assert!(!statement.sql.contains("repository_analysis_people"));
        assert!(statement.sql.contains("insight_id"));
        assert!(!statement.sql.contains("12345"));
    }
}

async fn insight_fixture(s: &AppState, id: i64) -> crate::entities::code_insights::Model {
    use crate::entities::{code_insight_repositories as selection, code_insights as insights};
    let row = insights::Model {
        id,
        workspace_id: 1,
        user_id: 1,
        name: format!("Insight {id}"),
        description: String::new(),
        config: encode(&json!({"enabled":true,"full_history":true,"aliases":[]})).unwrap(),
        coverage_start: 0,
        next_run_at: 0,
        version: 1,
        created_at: 0,
        updated_at: 0,
    };
    insights::Entity::insert(row.clone().into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    selection::Entity::insert(
        selection::Model {
            id: id + 1000,
            workspace_id: 1,
            insight_id: id,
            connection_id: 4,
            repository: "team/repo".into(),
            all_repositories: 0,
            pipeline_id: String::new(),
        }
        .into_active_model(),
    )
    .exec(&s.db)
    .await
    .unwrap();
    row
}

async fn insight_contributors(
    s: &AppState,
    insight: &crate::entities::code_insights::Model,
) -> Vec<Aggregate> {
    let source_rows = sources::Entity::find().all(&s.db).await.unwrap();
    let tx = crate::variables::transaction(s, 1.into()).await.unwrap();
    insight_identity::ensure(s, &tx, insight, &source_rows)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let cohort = FactCohort::with_projection(
        1,
        vec![7],
        &HashMap::new(),
        Projection::Insight {
            id: insight.id,
            coverage_start: insight.coverage_start,
        },
    )
    .unwrap();
    rows::<Aggregate>(s, &grouped(&cohort, DbBackend::Sqlite, "", false).unwrap())
        .await
        .unwrap()
}

async fn change_insight_aliases(
    s: &AppState,
    insight: &mut crate::entities::code_insights::Model,
    old: &[Alias],
    new: &mut [Alias],
) {
    let tx = crate::variables::transaction(s, 1.into()).await.unwrap();
    insight_identity::prepare_aliases(s, &tx, insight, old, new)
        .await
        .unwrap();
    insight.config = encode(&json!({"enabled":true,"full_history":true,"aliases":new})).unwrap();
    crate::entities::code_insights::Entity::update(insight.clone().into_active_model().reset_all())
        .exec(&tx)
        .await
        .unwrap();
    insight_identity::rebuild(s, &tx, insight).await.unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn insights_merge_rename_and_unmap_independently_without_changing_raw_facts() {
    use crate::entities::{
        repository_analysis_commit_authors as legacy, repository_analysis_commit_identities as raw,
    };
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    let mut one = commit('a');
    one.author_name = "Alice".into();
    one.author_email = "one@example.test".into();
    let a = store(&s, &claim, one).await.unwrap();
    let mut two = commit('b');
    two.author_name = "Alice".into();
    two.author_email = "two@example.test".into();
    let b = store(&s, &claim, two).await.unwrap();
    publish_branches(&s, &claim, &[("main", &b.sha, &[a.id, b.id])]).await;
    let raw_before = raw::Entity::find()
        .order_by_asc(raw::Column::CommitId)
        .all(&s.db)
        .await
        .unwrap();
    let legacy_before = legacy::Entity::find()
        .order_by_asc(legacy::Column::CommitId)
        .all(&s.db)
        .await
        .unwrap();
    let mut first = insight_fixture(&s, 80).await;
    let second = insight_fixture(&s, 81).await;
    let initial = insight_contributors(&s, &first).await;
    let initial_ids = initial
        .iter()
        .map(|row| row.contributor_id)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        initial_ids.len(),
        2,
        "distinct raw emails remain distinct until mapped"
    );
    let other_initial = insight_contributors(&s, &second).await;
    let other_ids = other_initial
        .iter()
        .map(|row| row.contributor_id)
        .collect::<BTreeSet<_>>();
    let mut aliases = vec![Alias {
        name: " alice ".into(),
        email: String::new(),
        nickname: "Member".into(),
        contributor_id: String::new(),
    }];
    change_insight_aliases(&s, &mut first, &[], &mut aliases).await;
    let merged = insight_contributors(&s, &first).await;
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].commits, 2);
    assert_eq!(merged[0].nickname, "Member");
    let merged_id = merged[0].contributor_id;
    let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
    let mut foreign = vec![Alias {
        name: "Alice".into(),
        email: String::new(),
        nickname: "Spoofed".into(),
        contributor_id: merged_id.to_string(),
    }];
    assert!(
        insight_identity::prepare_aliases(&s, &tx, &second, &[], &mut foreign)
            .await
            .is_err(),
        "a contributor ID from another insight is not an authorized mapping target"
    );
    tx.rollback().await.unwrap();
    let old = aliases.clone();
    aliases[0].nickname = "Renamed".into();
    change_insight_aliases(&s, &mut first, &old, &mut aliases).await;
    let renamed = insight_contributors(&s, &first).await;
    assert_eq!(renamed[0].contributor_id, merged_id);
    assert_eq!(renamed[0].nickname, "Renamed");
    let other = insight_contributors(&s, &second).await;
    assert_eq!(
        other
            .iter()
            .map(|row| row.contributor_id)
            .collect::<BTreeSet<_>>(),
        other_ids
    );
    assert!(other.iter().all(|row| row.nickname == "Alice"));
    change_insight_aliases(&s, &mut first, &aliases, &mut []).await;
    let unmapped = insight_contributors(&s, &first).await;
    assert_eq!(
        unmapped
            .iter()
            .map(|row| row.contributor_id)
            .collect::<BTreeSet<_>>(),
        initial_ids
    );
    assert_eq!(
        raw::Entity::find()
            .order_by_asc(raw::Column::CommitId)
            .all(&s.db)
            .await
            .unwrap(),
        raw_before
    );
    assert_eq!(
        legacy::Entity::find()
            .order_by_asc(legacy::Column::CommitId)
            .all(&s.db)
            .await
            .unwrap(),
        legacy_before
    );
    assert_eq!(
        commits::Entity::find_by_id(a.id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        a
    );
    assert_eq!(
        commits::Entity::find_by_id(b.id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap(),
        b
    );
}

#[tokio::test]
async fn insight_coverage_excludes_older_shared_facts_in_all_report_shapes() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    let mut old = commit('a');
    old.committer_date = 1000;
    let a = store(&s, &claim, old).await.unwrap();
    let mut recent = commit('b');
    recent.committer_date = 3000;
    let b = store(&s, &claim, recent).await.unwrap();
    publish_branches(&s, &claim, &[("main", &b.sha, &[a.id, b.id])]).await;
    let mut insight = insight_fixture(&s, 90).await;
    insight.coverage_start = 2000;
    let contributors = insight_contributors(&s, &insight).await;
    assert_eq!(contributors.iter().map(|row| row.commits).sum::<i64>(), 1);
    let cohort = FactCohort::with_projection(
        1,
        vec![7],
        &HashMap::new(),
        Projection::Insight {
            id: insight.id,
            coverage_start: insight.coverage_start,
        },
    )
    .unwrap();
    assert_eq!(count(&s, cohort.query.clone()).await.unwrap(), 1);
    let branches = rows::<BranchAggregate>(&s, &branch_aggregate(&cohort, DbBackend::Sqlite))
        .await
        .unwrap();
    assert_eq!(branches.len(), 1);
    assert_eq!(branches[0].commits, 1);
}

#[tokio::test]
async fn legacy_alias_ids_are_copied_without_sharing_mutable_insight_labels() {
    use crate::entities::{code_insight_people as people, repository_analysis_people as legacy};
    let (_dir, s) = fixture().await;
    legacy::Entity::insert(
        legacy::Model {
            id: 400,
            workspace_id: 1,
            user_id: 1,
            display_name: "Legacy label".into(),
            version: 3,
            raw_identity_id: None,
        }
        .into_active_model(),
    )
    .exec(&s.db)
    .await
    .unwrap();
    let first = insight_fixture(&s, 82).await;
    let second = insight_fixture(&s, 83).await;
    let old = vec![Alias {
        name: "Alice".into(),
        email: String::new(),
        nickname: "Legacy label".into(),
        contributor_id: "400".into(),
    }];
    let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
    let imported = insight_identity::import_aliases(&s, &tx, &first, &old)
        .await
        .unwrap();
    let second_imported = insight_identity::import_aliases(&s, &tx, &second, &old)
        .await
        .unwrap();
    assert_eq!(imported[0].contributor_id, "400");
    assert_eq!(second_imported[0].contributor_id, "400");
    let mut renamed = imported.clone();
    renamed[0].nickname = "First insight label".into();
    insight_identity::prepare_aliases(&s, &tx, &first, &imported, &mut renamed)
        .await
        .unwrap();
    assert_eq!(
        people::Entity::find_by_id((first.id, 400))
            .one(&tx)
            .await
            .unwrap()
            .unwrap()
            .display_name,
        "First insight label"
    );
    assert_eq!(
        people::Entity::find_by_id((second.id, 400))
            .one(&tx)
            .await
            .unwrap()
            .unwrap()
            .display_name,
        "Legacy label"
    );
    assert_eq!(
        legacy::Entity::find_by_id(400)
            .one(&tx)
            .await
            .unwrap()
            .unwrap()
            .display_name,
        "Legacy label"
    );
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn publication_projects_insights_and_ambiguous_mapping_never_blocks_shared_facts_or_writes_on_read()
 {
    use crate::entities::{code_insights as insight_rows, workspace_roles};
    let (_dir, mut s) = fixture().await;
    let claim = claimed(&s).await;
    let mut first = insight_fixture(&s, 84).await;
    let second = insight_fixture(&s, 85).await;
    let mut aliases = vec![
        Alias {
            name: "Alice".into(),
            email: String::new(),
            nickname: "Name target".into(),
            contributor_id: String::new(),
        },
        Alias {
            name: String::new(),
            email: "mail@example.test".into(),
            nickname: "Email target".into(),
            contributor_id: String::new(),
        },
    ];
    change_insight_aliases(&s, &mut first, &[], &mut aliases).await;
    let mut value = commit('a');
    value.author_name = "Alice".into();
    value.author_email = "mail@example.test".into();
    let fact = store(&s, &claim, value).await.unwrap();
    publish_branches(&s, &claim, &[("main", &fact.sha, &[fact.id])]).await;
    let conflicted = insight_rows::Entity::find_by_id(first.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !insights::decode(&conflicted)
            .unwrap()
            .identity_error
            .is_empty()
    );
    assert!(
        insights::decode(
            &insight_rows::Entity::find_by_id(second.id)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap()
        .identity_error
        .is_empty()
    );
    let role = workspace_roles::Entity::find_by_id(2)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let mut role = role.into_active_model();
    role.permissions = sea_orm::Set(encode(&["repositories.read", "code-insights.read"]).unwrap());
    workspace_roles::Entity::update(role)
        .exec(&s.db)
        .await
        .unwrap();
    let statements = Arc::new(Mutex::new(Vec::<String>::new()));
    let callback = statements.clone();
    Arc::get_mut(&mut s)
        .unwrap()
        .db
        .set_metric_callback(move |info| callback.lock().unwrap().push(info.statement.sql.clone()));
    for (id, complete) in [(first.id, false), (second.id, true)] {
        let scope = Scope::for_actor(&s, 1.into(), 1.into()).await.unwrap();
        let report = insight_report(
            State(s.clone()),
            scope,
            Path(("1".into(), id.into(), "commits".into())),
            Query(HashMap::new()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(report["total"], 1);
        assert_eq!(report["coverage"]["complete"], complete);
        assert_eq!(
            report["items"][0]["nickname"], "Alice",
            "runtime ambiguity falls back to raw identity"
        );
    }
    assert!(
        statements
            .lock()
            .unwrap()
            .iter()
            .all(|sql| sql.trim_start().to_ascii_uppercase().starts_with("SELECT")),
        "report requests must only read precomputed local projections: {:?}",
        statements.lock().unwrap()
    );
    let old = aliases.clone();
    aliases.truncate(1);
    change_insight_aliases(&s, &mut first, &old, &mut aliases).await;
    let repaired = insight_rows::Entity::find_by_id(first.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert!(
        insights::decode(&repaired)
            .unwrap()
            .identity_error
            .is_empty()
    );
    statements.lock().unwrap().clear();
    let scope = Scope::for_actor(&s, 1.into(), 1.into()).await.unwrap();
    let report = insight_report(
        State(s.clone()),
        scope,
        Path(("1".into(), first.id.into(), "commits".into())),
        Query(HashMap::new()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(report["items"][0]["nickname"], "Name target");
    assert_eq!(report["coverage"]["complete"], true);
    assert!(
        statements
            .lock()
            .unwrap()
            .iter()
            .all(|sql| sql.trim_start().to_ascii_uppercase().starts_with("SELECT"))
    );
}

#[tokio::test]
async fn imported_raw_defaults_keep_legacy_contributor_ids() {
    use crate::entities::{code_insight_people as people, repository_analysis_people as legacy};
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    let fact = store(&s, &claim, commit('a')).await.unwrap();
    publish_branches(&s, &claim, &[("main", &fact.sha, &[fact.id])]).await;
    let legacy = legacy::Entity::find()
        .filter(legacy::Column::RawIdentityId.is_not_null())
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let insight = insight_fixture(&s, 4).await;
    let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
    insight_identity::import_aliases(&s, &tx, &insight, &[])
        .await
        .unwrap();
    insight_identity::rebuild(&s, &tx, &insight).await.unwrap();
    tx.commit().await.unwrap();
    let imported = people::Entity::find_by_id((insight.id, legacy.id))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(imported.raw_identity_id, legacy.raw_identity_id);
    let report = insight_contributors(&s, &insight).await;
    assert_eq!(report.len(), 1);
    assert_eq!(report[0].contributor_id, legacy.id);
}

async fn read(s: &AppState, kind: &str, params: &[(&str, &str)]) -> Value {
    let scope = Scope::for_actor(s, 1.into(), 1.into()).await.unwrap();
    report(
        State(s.clone()),
        scope,
        Path(("1".into(), kind.into())),
        Query(
            params
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        ),
    )
    .await
    .unwrap()
    .0
}

#[test]
fn every_dialect_uses_bound_filters_and_stable_contributor_grouping() {
    let raw = HashMap::from([
        ("authors".into(), "a'); DROP TABLE x; --".into()),
        ("messageType".into(), "feat,fix".into()),
        ("branches".into(), "main".into()),
    ]);
    let cohort = FactCohort::new(1, 2, vec![3, 4], &raw).unwrap();
    for backend in [DbBackend::Sqlite, DbBackend::Postgres, DbBackend::MySql] {
        let query = grouped(&cohort, backend, "month", false).unwrap();
        let statement = backend.build(&query);
        assert!(!statement.sql.contains("DROP TABLE"));
        assert!(
            !statement.sql.contains("CASE"),
            "mapping rules must not be expanded into report SQL"
        );
        assert!(statement.sql.contains("contributor_id"));
        assert!(statement.sql.contains("EXISTS"));
        assert!(
            statement
                .values
                .unwrap()
                .0
                .iter()
                .any(|value| value.to_string().contains("DROP TABLE"))
        );
        let branches = backend.build(&branch_aggregate(&cohort, backend));
        assert!(branches.sql.contains("LEFT JOIN"));
        assert!(branches.sql.contains("DISTINCT"));
    }
}

#[tokio::test]
async fn branch_queries_stay_constant_as_branch_count_grows_and_fact_union_is_deduplicated() {
    let (_dir, mut s) = fixture().await;
    let claim = claimed(&s).await;
    let mut facts = Vec::new();
    for index in 0..128 {
        let mut value = commit('a');
        value.sha = format!("{:040x}", index + 1);
        value.author_name = format!("Author {}", index % 16);
        value.author_email = format!("author{}@example.test", index % 16);
        facts.push(store(&s, &claim, value).await.unwrap());
    }
    let ids: Vec<i64> = facts.iter().map(|fact| fact.id).collect();
    publish_branches(
        &s,
        &claim,
        &[("branch-000", facts[0].sha.as_str(), ids.as_slice())],
    )
    .await;
    identity::backfill(&s).await.unwrap();
    let statements = Arc::new(Mutex::new(Vec::<String>::new()));
    let callback = statements.clone();
    Arc::get_mut(&mut s)
        .unwrap()
        .db
        .set_metric_callback(move |info| callback.lock().unwrap().push(info.statement.sql.clone()));
    let one = read(&s, "branches", &[("page_size", "10")]).await;
    let one_count = statements.lock().unwrap().len();
    assert_eq!(one["total"], 1);
    assert_eq!(one["items"][0]["commits"], 128);
    let names: Vec<String> = (0..64).map(|index| format!("branch-{index:03}")).collect();
    let branches: Vec<(&str, &str, &[i64])> = names
        .iter()
        .map(|name| (name.as_str(), facts[0].sha.as_str(), ids.as_slice()))
        .collect();
    publish_branches(&s, &claim, &branches).await;
    statements.lock().unwrap().clear();
    let many = read(&s, "branches", &[("page_size", "10"), ("page", "2")]).await;
    let many_sql = statements.lock().unwrap().clone();
    assert_eq!(many["total"], 64);
    assert_eq!(many["items"].as_array().unwrap().len(), 10);
    assert_eq!(many["items"][0]["commits"], 128);
    assert_eq!(many["items"][0]["nicknames"].as_array().unwrap().len(), 16);
    assert_eq!(
        many_sql.len(),
        one_count,
        "database round trips must not grow with branch count"
    );
    assert!(
        many_sql.len() <= 16,
        "unexpected query fan-out: {} statements",
        many_sql.len()
    );
    assert!(
        many_sql
            .iter()
            .any(|sql| sql.contains("LIMIT") && sql.contains("GROUP BY")),
        "grouped pages must be bounded in SQL"
    );
    let overview = read(&s, "overview", &[]).await;
    assert_eq!(
        overview["items"][0]["commits"], 128,
        "64 branch memberships must not multiply facts"
    );
    let ranking = read(&s, "ranking", &[("page_size", "5"), ("page", "2")]).await;
    assert_eq!(ranking["total"], 16);
    assert_eq!(ranking["items"].as_array().unwrap().len(), 5);
    eprintln!(
        "branch load fixture: 128 commits, 64 branches, 8192 memberships, {one_count} report statements at both branch counts"
    );
}

#[tokio::test]
async fn sqlite_cohort_query_plan_uses_membership_and_identity_indexes() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    let value = store(&s, &claim, commit('a')).await.unwrap();
    publish_branches(&s, &claim, &[("main", value.sha.as_str(), &[value.id])]).await;
    identity::backfill(&s).await.unwrap();
    let cohort = FactCohort::new(1, 1, vec![7], &HashMap::new()).unwrap();
    let statement = DbBackend::Sqlite.build(&cohort.query);
    let explain = Statement::from_sql_and_values(
        DbBackend::Sqlite,
        format!("EXPLAIN QUERY PLAN {}", statement.sql),
        statement.values.unwrap().0,
    );
    let details: Vec<String> =
        s.db.query_all_raw(explain)
            .await
            .unwrap()
            .iter()
            .map(|row| row.try_get::<String>("", "detail").unwrap())
            .collect();
    assert!(
        details
            .iter()
            .any(|detail| detail.contains("analysis_5_1") || detail.contains("analysis_5_0")),
        "membership bridge should use an index: {details:?}"
    );
    assert!(
        details
            .iter()
            .any(|detail| detail.contains("ca") && detail.contains("INDEX")),
        "owner-scoped identity projection should use an index: {details:?}"
    );
    eprintln!("cohort EXPLAIN QUERY PLAN: {details:?}");
}

/// Manual, bounded SQLite workload; timings are observations, not latency gates.
#[tokio::test]
#[ignore = "manual 10k-fact/320k-membership query workload"]
async fn large_fact_cohort_reports_preserve_counts_and_sql_pagination() {
    let (_dir, mut s) = fixture().await;
    let claim = claimed(&s).await;
    let mut seed = commit('a');
    seed.author_name = "Author 0".into();
    seed.author_email = "author0@example.test".into();
    let first = store(&s, &claim, seed).await.unwrap();
    let mut ids = vec![first.id];
    let mut batch = Vec::with_capacity(500);
    for index in 1..10_000 {
        let mut row = first.clone();
        row.id = s.next_id().unwrap().into();
        row.sha = format!("{index:040x}");
        row.author_name = format!("Author {}", index % 100);
        row.author_email = format!("author{}@example.test", index % 100);
        ids.push(row.id);
        batch.push(row.into_active_model());
        if batch.len() == 500 || index == 9_999 {
            commits::Entity::insert_many(std::mem::take(&mut batch))
                .exec(&s.db)
                .await
                .unwrap();
        }
    }
    let names: Vec<String> = (0..32).map(|index| format!("branch-{index:03}")).collect();
    let branches: Vec<(&str, &str, &[i64])> = names
        .iter()
        .map(|name| (name.as_str(), first.sha.as_str(), ids.as_slice()))
        .collect();
    publish_branches(&s, &claim, &branches).await;
    identity::backfill(&s).await.unwrap();
    let statements = Arc::new(Mutex::new(Vec::<(Statement, std::time::Duration)>::new()));
    let callback = statements.clone();
    Arc::get_mut(&mut s)
        .unwrap()
        .db
        .set_metric_callback(move |info| {
            callback
                .lock()
                .unwrap()
                .push((info.statement.clone(), info.elapsed));
        });
    for (kind, total, page_len) in [
        ("overview", 1, 1),
        ("ranking", 100, 10),
        ("branches", 32, 10),
    ] {
        statements.lock().unwrap().clear();
        let start = std::time::Instant::now();
        let result = read(&s, kind, &[("page_size", "10")]).await;
        let elapsed = start.elapsed();
        let sql = statements.lock().unwrap().clone();
        assert_eq!(result["total"], total, "{kind}");
        assert_eq!(
            result["items"].as_array().unwrap().len(),
            page_len,
            "{kind}"
        );
        let expected = if kind == "ranking" { 100 } else { 10_000 };
        for row in result["items"].as_array().unwrap() {
            assert_eq!(row["commits"], expected, "{kind}");
            assert_eq!(row["additions"], expected * 12, "{kind}");
            assert_eq!(row["deletions"], expected * 3, "{kind}");
            if kind == "branches" {
                assert_eq!(row["nicknames"].as_array().unwrap().len(), 100);
            }
        }
        assert!(sql.len() <= 16, "{kind}: unexpected query fan-out");
        assert!(
            sql.iter()
                .any(|q| q.0.sql.contains("LIMIT") && q.0.sql.contains("COUNT(")),
            "{kind}: aggregate page must be bounded in SQL"
        );
        for (index, (statement, duration)) in sql.iter().enumerate() {
            eprintln!("{kind} statement {index}: {duration:?}");
            if kind == "branches" && duration.as_millis() > 50 {
                let explain = Statement::from_sql_and_values(
                    DbBackend::Sqlite,
                    format!("EXPLAIN QUERY PLAN {}", statement.sql),
                    statement.values.clone().unwrap().0,
                );
                let plan: Vec<String> =
                    s.db.query_all_raw(explain)
                        .await
                        .unwrap()
                        .iter()
                        .map(|row| row.try_get("", "detail").unwrap())
                        .collect();
                eprintln!(
                    "branch statement {index}: {}\nplan: {plan:?}",
                    statement.sql
                );
            }
        }
        eprintln!(
            "SQLite manual load: 10000 facts, 100 authors, 32 branches, 320000 memberships; {kind}: {elapsed:?}, {} statements, total={total}, page={page_len}",
            sql.len()
        );
    }
}
