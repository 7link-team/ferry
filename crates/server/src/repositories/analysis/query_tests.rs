use super::*;
use crate::repositories::analysis::{
    identity,
    sync::{
        store,
        tests::{claimed, commit, fixture, publish_branches},
    },
};
use sea_orm::Statement;
use std::sync::{Arc, Mutex};

fn instant(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .unwrap()
        .timestamp_millis()
}

// Dense test input deliberately represents actual occupied UTC days; production
// never generates this range from cohort min/max.
fn calendar_range(
    zone: chrono_tz::Tz,
    period: &str,
    min: i64,
    max: i64,
) -> Result<Vec<CalendarBucket>> {
    chrono::DateTime::from_timestamp_millis(min).ok_or_else(date_error)?;
    chrono::DateTime::from_timestamp_millis(max).ok_or_else(date_error)?;
    let days = (min.div_euclid(86_400_000)..=max.div_euclid(86_400_000)).collect::<Vec<_>>();
    Ok(calendar_buckets(zone, period, &days)?
        .into_iter()
        .filter(|bucket| bucket.start <= max && bucket.end > min)
        .collect())
}

#[test]
fn business_date_filters_use_complete_local_days_and_preserve_absolute_instants() {
    let zone = Some(chrono_tz::America::New_York);
    for (day, start, end) in [
        (
            "2026-03-08",
            "2026-03-08T05:00:00Z",
            "2026-03-09T03:59:59.999Z",
        ),
        (
            "2026-11-01",
            "2026-11-01T04:00:00Z",
            "2026-11-02T04:59:59.999Z",
        ),
    ] {
        assert_eq!(date_in_zone(day, false, zone).unwrap(), instant(start));
        assert_eq!(date_in_zone(day, true, zone).unwrap(), instant(end));
    }
    assert_eq!(
        date_in_zone("2026-03-08", false, None).unwrap(),
        instant("2026-03-08T00:00:00Z")
    );
    for end in [false, true] {
        assert_eq!(
            date_in_zone("2026-03-08T01:30:00+08:00", end, zone).unwrap(),
            instant("2026-03-08T01:30:00+08:00")
        );
    }
    // Midnight is skipped in Sao Paulo; Apia skips an entire calendar date.
    assert_eq!(
        date_in_zone("2018-11-04", false, Some(chrono_tz::America::Sao_Paulo)).unwrap(),
        instant("2018-11-04T03:00:00Z")
    );
    let skipped = Some(chrono_tz::Pacific::Apia);
    assert_eq!(
        date_in_zone("2011-12-30", false, skipped).unwrap(),
        date_in_zone("2011-12-30", true, skipped).unwrap() + 1
    );
    assert_eq!(
        date_in_zone("2026-11-01", false, Some(chrono_tz::America::Havana)).unwrap(),
        instant("2026-11-01T04:00:00Z"),
        "repeated midnight starts at its earliest instant"
    );
    assert!(date_in_zone("invalid", false, zone).is_err());
    assert!(calendar_range(chrono_tz::UTC, "day", i64::MIN, i64::MAX).is_err());
}

#[test]
fn calendar_ranges_cover_dst_midnight_week_and_month_without_utc_rebucketing() {
    let zone = chrono_tz::America::New_York;
    let daily = calendar_range(
        zone,
        "day",
        instant("2026-03-08T04:30:00Z"),
        instant("2026-03-09T04:30:00Z"),
    )
    .unwrap();
    assert_eq!(
        daily.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(),
        ["2026-03-07", "2026-03-08", "2026-03-09"]
    );
    assert_eq!(daily[1].end - daily[1].start, 23 * 3_600_000);
    let weekly = calendar_range(
        zone,
        "week",
        instant("2026-11-02T04:30:00Z"),
        instant("2026-11-02T05:30:00Z"),
    )
    .unwrap();
    assert_eq!(
        weekly.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(),
        ["2026-11-01", "2026-11-08"]
    );
    assert_eq!(weekly[0].end - weekly[0].start, (7 * 24 + 1) * 3_600_000);
    let monthly = calendar_range(
        chrono_tz::Asia::Kuala_Lumpur,
        "month",
        instant("2026-10-31T15:59:59Z"),
        instant("2026-10-31T16:00:00Z"),
    )
    .unwrap();
    assert_eq!(
        monthly.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(),
        ["2026-10", "2026-11"]
    );
}

#[tokio::test]
async fn local_midnight_metadata_and_sparse_extreme_dates_keep_all_authors_and_counts() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    let mut ids = Vec::new();
    let ancient = chrono::NaiveDate::from_ymd_opt(-1, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();
    let distant = chrono::NaiveDate::from_ymd_opt(100_000, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();
    for (sha, authored, name, email) in [
        ('a', ancient, "Ancient", "ancient@example.test"),
        ('b', 0, "Epoch", "epoch@example.test"),
        (
            'c',
            instant("2026-01-31T23:30:00Z"),
            "Midnight",
            "midnight@example.test",
        ),
        ('d', distant, "Distant", "distant@example.test"),
    ] {
        let mut value = commit(sha);
        value.author_date = authored;
        value.committer_date = instant("2026-02-01T12:00:00Z");
        value.author_name = name.into();
        value.author_email = email.into();
        ids.push(store(&s, &claim, value).await.unwrap().id);
    }
    publish_branches(&s, &claim, &[("main", &"d".repeat(40), &ids)]).await;
    identity::backfill(&s).await.unwrap();
    s.db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "UPDATE workspaces SET timezone = ? WHERE id = ?",
        vec!["Asia/Kuala_Lumpur".into(), 1_i64.into()],
    ))
    .await
    .unwrap();
    let rendered = read(&s, "heatmap", &[]).await;
    assert_eq!(rendered["total"], 4);
    let series = rendered["series"].as_array().unwrap();
    for (label, name, email) in [
        ("-0001-01-01", "Ancient", "ancient@example.test"),
        ("1970-01-01", "Epoch", "epoch@example.test"),
        ("2026-02-01", "Midnight", "midnight@example.test"),
        ("+100000-01-01", "Distant", "distant@example.test"),
    ] {
        let day = series.iter().find(|row| row["period"] == label).unwrap();
        assert_eq!(day["commits"], 1);
        assert_eq!(day["name"], name);
        assert_eq!(day["email"], email);
        assert_eq!(day["authors"][0]["name"], name);
        assert_eq!(day["authors"][0]["email"], email);
    }
    for period in ["day", "week", "month"] {
        let mut cohort = FactCohort::with_timezone_projection(
            1,
            vec![7],
            &HashMap::new(),
            Projection::Legacy(1),
            Some(chrono_tz::Asia::Kuala_Lumpur),
        )
        .unwrap();
        cohort
            .prepare_calendar(&s, chrono_tz::Asia::Kuala_Lumpur, period)
            .await
            .unwrap();
        assert!(cohort.calendar.as_ref().unwrap().len() < 32);
        let grouped = grouped(&cohort, DbBackend::Sqlite, period, true).unwrap();
        let values = rows::<Aggregate>(&s, &grouped).await.unwrap();
        assert_eq!(values.iter().map(|row| row.commits).sum::<i64>(), 4);
        assert_eq!(count(&s, grouped).await.unwrap(), 4);
        for backend in [DbBackend::Sqlite, DbBackend::Postgres, DbBackend::MySql] {
            let projection = backend.build(&occupied_day_query(&cohort, backend));
            assert!(projection.sql.contains("DISTINCT"));
            assert!(projection.sql.contains("utc_day"));
            assert!(!projection.sql.contains("MIN("));
            let statement = backend.build(&super::grouped(&cohort, backend, period, true).unwrap());
            assert!(statement.sql.matches(" WHEN ").count() < 64);
            assert!(statement.sql.len() < 15_000);
        }
    }
}

#[tokio::test]
async fn calendar_case_is_portable_and_keeps_sql_aggregation_count_and_pagination() {
    let (_dir, s) = fixture().await;
    let claim = claimed(&s).await;
    let mut ids = Vec::new();
    for (sha, authored) in [
        ('a', "2026-03-08T04:30:00Z"),
        ('b', "2026-03-08T05:30:00Z"),
        ('c', "2026-03-09T03:30:00Z"),
    ] {
        let mut value = commit(sha);
        value.author_date = instant(authored);
        value.committer_date = instant("2026-03-09T05:30:00Z");
        ids.push(store(&s, &claim, value).await.unwrap().id);
    }
    publish_branches(&s, &claim, &[("main", &"c".repeat(40), &ids)]).await;
    identity::backfill(&s).await.unwrap();
    let raw = HashMap::from([
        ("since".into(), "2026-03-09".into()),
        ("until".into(), "2026-03-09".into()),
    ]);
    let zone = chrono_tz::America::New_York;
    let mut cohort =
        FactCohort::with_timezone_projection(1, vec![7], &raw, Projection::Legacy(1), Some(zone))
            .unwrap();
    cohort.prepare_calendar(&s, zone, "day").await.unwrap();
    let q = grouped(&cohort, DbBackend::Sqlite, "day", true).unwrap();
    assert_eq!(count(&s, q.clone()).await.unwrap(), 2);
    let mut page = q;
    page.order_by(a("period"), Order::Asc).limit(1).offset(1);
    let selected = rows::<Aggregate>(&s, &page).await.unwrap();
    assert_eq!(selected[0].period, "2026-03-08");
    assert_eq!(selected[0].commits, 2);
    let metadata = aggregate_items(&s, &cohort, "day", true, selected)
        .await
        .unwrap();
    assert_eq!(metadata[0]["commits"], 2);
    s.db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "UPDATE workspaces SET timezone = ? WHERE id = ?",
        vec!["America/New_York".into(), 1_i64.into()],
    ))
    .await
    .unwrap();
    let rendered = read(
        &s,
        "heatmap",
        &[("since", "2026-03-09"), ("until", "2026-03-09")],
    )
    .await;
    assert_eq!(rendered["coverage"]["timezone"], "America/New_York");
    assert_eq!(rendered["total"], 2);
    assert_eq!(rendered["series"][0]["period"], "2026-03-07");
    assert_eq!(rendered["series"][1]["commits"], 2);
    assert_eq!(rendered["series"][1]["authors"][0]["commits"], 2);
    for backend in [DbBackend::Sqlite, DbBackend::Postgres, DbBackend::MySql] {
        let mut query = grouped(&cohort, backend, "day", false).unwrap();
        query.limit(1).offset(1);
        let statement = backend.build(&query);
        assert!(statement.sql.contains("CASE WHEN"));
        assert!(statement.sql.contains("GROUP BY"));
        assert!(statement.sql.contains("LIMIT"));
        assert!(!statement.sql.contains("CONVERT_TZ"));
        assert!(!statement.sql.contains("AT TIME ZONE"));
        assert!(statement.sql.len() < 10_000);
    }
    // Full-history charts must not hit bind limits or nested-expression depth
    // limits merely because ten years of calendar dates are represented.
    cohort.calendar = Some(
        calendar_range(
            zone,
            "day",
            instant("2020-01-01T00:00:00Z"),
            instant("2030-01-01T00:00:00Z"),
        )
        .unwrap(),
    );
    let decade = grouped(&cohort, DbBackend::Sqlite, "day", true).unwrap();
    let decade_rows = rows::<Aggregate>(&s, &decade).await.unwrap();
    assert_eq!(decade_rows.len(), 2);
    assert_eq!(decade_rows.iter().map(|row| row.commits).sum::<i64>(), 3);
    for backend in [DbBackend::Sqlite, DbBackend::Postgres, DbBackend::MySql] {
        let statement = backend.build(&grouped(&cohort, backend, "day", true).unwrap());
        assert!(statement.sql.len() < 1_000_000);
        assert!(statement.values.unwrap().0.len() < 100);
    }
}

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
        name: "Alice".into(),
        email: "one@example.test".into(),
        nickname: "Member".into(),
        contributor_id: String::new(),
    }];
    change_insight_aliases(&s, &mut first, &[], &mut aliases).await;
    let old = aliases.clone();
    aliases.push(Alias {
        name: "Alice".into(),
        email: "two@example.test".into(),
        nickname: "Member".into(),
        contributor_id: String::new(),
    });
    change_insight_aliases(&s, &mut first, &old, &mut aliases).await;
    let independent = insight_contributors(&s, &first).await;
    assert_eq!(
        independent.len(),
        2,
        "equal nicknames never merge author pairs"
    );
    let old = aliases.clone();
    aliases[1].contributor_id = aliases[0].contributor_id.clone();
    change_insight_aliases(&s, &mut first, &old, &mut aliases).await;
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
    aliases[1].nickname = "Renamed".into();
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
    // Simulate a conflicting persisted configuration; normal saves reject this.
    aliases[0].email = "mail@example.test".into();
    aliases[1].name = "Alice".into();
    first.config = encode(&json!({"enabled":true,"full_history":true,"aliases":aliases})).unwrap();
    insight_rows::Entity::update(first.clone().into_active_model().reset_all())
        .exec(&s.db)
        .await
        .unwrap();
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
    let repos = read(&s, "repositories", &[]).await;
    assert_eq!(repos["total"], 1);
    assert_eq!(repos["items"][0]["commits"], 128);
    assert_eq!(repos["items"][0]["contributors"], 16);
    let heatmap = read(&s, "heatmap", &[]).await;
    let day = &heatmap["series"][0];
    assert_eq!(day["commits"], 128);
    assert_eq!(day["contributors"], 16);
    assert_eq!(day["authors"].as_array().unwrap().len(), 16);
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
