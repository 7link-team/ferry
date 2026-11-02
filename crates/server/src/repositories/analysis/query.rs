use super::*;
use sea_orm::{
    Condition, DbBackend, FromQueryResult,
    sea_query::{Alias as A, ExprTrait, JoinType, Order, Query as Q, SelectStatement, SimpleExpr},
};

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;

fn a(name: &str) -> A {
    A::new(name)
}
fn col(table: &str, name: &str) -> SimpleExpr {
    Expr::col((a(table), a(name))).into()
}
fn pagination(raw: &HashMap<String, String>) -> (u64, u64, u64) {
    let page = std::cmp::max(
        raw.get("page")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(1),
        1,
    );
    let size = raw
        .get("page_size")
        .or_else(|| raw.get("limit"))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(50)
        .clamp(1, 500);
    (
        page,
        size,
        raw.get("offset")
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| (page - 1).saturating_mul(size)),
    )
}
fn list(raw: &HashMap<String, String>, key: &str) -> Vec<String> {
    raw.get(key)
        .map(|v| {
            v.split(',')
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
fn date(value: &str, end: bool) -> Result<i64> {
    if let Ok(time) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(time.timestamp_millis());
    }
    let day = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.analysis.dateInvalid"),
        )
    })?;
    Ok(day
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis()
        + if end { 86_399_999 } else { 0 })
}
fn period_expression(backend: DbBackend, period: &str) -> Result<SimpleExpr> {
    if period.is_empty() {
        return Ok(Expr::val("").into());
    }
    let day = match backend {
        DbBackend::Sqlite => "date(f.author_date / 1000.0, 'unixepoch')",
        DbBackend::MySql => "DATE_ADD('1970-01-01', INTERVAL FLOOR(f.author_date / 1000) SECOND)",
        DbBackend::Postgres => "(TO_TIMESTAMP(f.author_date / 1000.0) AT TIME ZONE 'UTC')",
        _ => {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.analysis.databasePeriodUnsupported"),
            ));
        }
    };
    let expression = match (backend, period) {
        (DbBackend::Sqlite, "day" | "daily") => day.to_owned(),
        (DbBackend::Sqlite, "week" | "weekly") => format!("date({day}, 'weekday 0')"),
        (DbBackend::Sqlite, "month" | "monthly") => format!("strftime('%Y-%m', {day})"),
        (DbBackend::MySql, "day" | "daily") => format!("DATE_FORMAT({day}, '%Y-%m-%d')"),
        (DbBackend::MySql, "week" | "weekly") => {
            format!("DATE_FORMAT(DATE_ADD({day}, INTERVAL (6-WEEKDAY({day})) DAY), '%Y-%m-%d')")
        }
        (DbBackend::MySql, "month" | "monthly") => format!("DATE_FORMAT({day}, '%Y-%m')"),
        (DbBackend::Postgres, "day" | "daily") => format!("TO_CHAR({day}, 'YYYY-MM-DD')"),
        (DbBackend::Postgres, "week" | "weekly") => {
            format!("TO_CHAR(DATE_TRUNC('week', {day}) + INTERVAL '6 days', 'YYYY-MM-DD')")
        }
        (DbBackend::Postgres, "month" | "monthly") => format!("TO_CHAR({day}, 'YYYY-MM')"),
        _ => {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.analysis.periodInvalid"),
            ));
        }
    };
    Ok(Expr::cust(expression))
}

/// One authorized, deduplicated fact cohort serves every report. Source coverage
/// and reachability use a semi-join; owner-scoped author resolution is persisted.
#[derive(Clone, Copy)]
enum Projection {
    Legacy(i64),
    Insight { id: i64, coverage_start: i64 },
}

struct FactCohort {
    query: SelectStatement,
    scoped_facts: SelectStatement,
    source_ids: Vec<i64>,
    branches: Vec<String>,
    current: bool,
}
impl FactCohort {
    fn membership(source_ids: &[i64], branches: &[String], current: bool) -> SelectStatement {
        let mut q = Q::select();
        q.expr(Expr::val(1))
            .from_as(a("repository_analysis_ref_commits"), a("m"))
            .join_as(
                JoinType::InnerJoin,
                a("repository_analysis_sources"),
                a("s"),
                col("s", "id").eq(col("m", "source_id")),
            )
            .join_as(
                JoinType::InnerJoin,
                a("repository_analysis_refs"),
                a("r"),
                col("r", "id").eq(col("m", "ref_id")),
            )
            .and_where(col("m", "source_id").is_in(source_ids.iter().copied()))
            .and_where(col("m", "commit_id").eq(col("c", "id")))
            .and_where(col("s", "repository_id").eq(col("c", "repository_id")))
            .cond_where(
                Condition::any()
                    .add(col("s", "coverage_start").eq(0))
                    .add(col("c", "committer_date").gte(col("s", "coverage_start"))),
            );
        if !branches.is_empty() {
            q.and_where(col("r", "name").is_in(branches.iter().cloned()));
        }
        if current {
            q.and_where(col("r", "present").eq(1))
                .and_where(col("m", "current").eq(1));
        }
        q.to_owned()
    }
    #[cfg(test)]
    fn new(
        workspace: i64,
        user: i64,
        source_ids: Vec<i64>,
        raw: &HashMap<String, String>,
    ) -> Result<Self> {
        Self::with_projection(workspace, source_ids, raw, Projection::Legacy(user))
    }
    fn with_projection(
        workspace: i64,
        source_ids: Vec<i64>,
        raw: &HashMap<String, String>,
        projection: Projection,
    ) -> Result<Self> {
        let branches = list(raw, "branches");
        let current = raw.get("history").is_some_and(|v| v == "current");
        let mut q = Q::select();
        q.column((a("c"), sea_orm::sea_query::Asterisk))
            .from_as(a("repository_analysis_commits"), a("c"));
        match projection {
            Projection::Legacy(user) => {
                q.join_as(
                    JoinType::InnerJoin,
                    a("repository_analysis_commit_authors"),
                    a("ca"),
                    Condition::all()
                        .add(col("ca", "commit_id").eq(col("c", "id")))
                        .add(col("ca", "user_id").eq(user))
                        .add(col("ca", "workspace_id").eq(workspace)),
                )
                .join_as(
                    JoinType::InnerJoin,
                    a("repository_analysis_people"),
                    a("p"),
                    Condition::all()
                        .add(col("p", "id").eq(col("ca", "contributor_id")))
                        .add(col("p", "workspace_id").eq(workspace))
                        .add(col("p", "user_id").eq(user)),
                );
            }
            Projection::Insight { id, coverage_start } => {
                q.join_as(
                    JoinType::InnerJoin,
                    a("repository_analysis_commit_identities"),
                    a("ci"),
                    Condition::all()
                        .add(col("ci", "commit_id").eq(col("c", "id")))
                        .add(col("ci", "workspace_id").eq(workspace)),
                )
                .join_as(
                    JoinType::InnerJoin,
                    a("code_insight_identity_links"),
                    a("ca"),
                    Condition::all()
                        .add(col("ca", "identity_id").eq(col("ci", "identity_id")))
                        .add(col("ca", "insight_id").eq(id))
                        .add(col("ca", "workspace_id").eq(workspace)),
                )
                .join_as(
                    JoinType::InnerJoin,
                    a("code_insight_people"),
                    a("p"),
                    Condition::all()
                        .add(col("p", "id").eq(col("ca", "contributor_id")))
                        .add(col("p", "workspace_id").eq(workspace))
                        .add(col("p", "insight_id").eq(id)),
                );
                if coverage_start != 0 {
                    q.and_where(col("c", "committer_date").gte(coverage_start));
                }
            }
        }
        q.expr_as(col("ca", "contributor_id"), a("contributor_id"))
            .expr_as(col("p", "display_name"), a("nickname"))
            .and_where(col("c", "workspace_id").eq(workspace))
            .and_where(col("c", "complete").eq(1));
        let scoped_facts = q.clone();
        q.and_where(Expr::exists(Self::membership(
            &source_ids,
            &branches,
            current,
        )));
        for (key, end) in [("since", false), ("until", true)] {
            if let Some(value) = raw.get(key).filter(|v| !v.is_empty()) {
                let value = date(value, end)?;
                q.and_where(if end {
                    col("c", "committer_date").lte(value)
                } else {
                    col("c", "committer_date").gte(value)
                });
            }
        }
        let merge = list(raw, "isMerge");
        let merge = if merge.is_empty() {
            vec![0]
        } else {
            merge
                .iter()
                .map(|v| {
                    v.parse::<i64>()
                        .ok()
                        .filter(|v| *v == 0 || *v == 1)
                        .ok_or_else(|| {
                            ApiError::localized(
                                axum::http::StatusCode::BAD_REQUEST,
                                crate::i18n::Message::new("repository.analysis.mergeFilterInvalid"),
                            )
                        })
                })
                .collect::<Result<Vec<_>>>()?
        };
        q.and_where(col("c", "is_merge").is_in(merge));
        let types = list(raw, "messageType");
        if !types.is_empty() {
            q.and_where(col("c", "message_type").is_in(types));
        }
        if let Some(value) = raw.get("commitHash").filter(|v| !v.is_empty()) {
            q.and_where(col("c", "sha").eq(value.clone()));
        }
        for (key, lower) in [("geEffective", true), ("leEffective", false)] {
            if let Some(value) = raw.get(key).filter(|v| !v.is_empty()) {
                let value = value.parse::<i64>().map_err(|_| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("repository.analysis.effectiveFilterInvalid"),
                    )
                })?;
                q.and_where(if lower {
                    col("c", "effectives").gte(value)
                } else {
                    col("c", "effectives").lte(value)
                });
            }
        }
        let authors = list(raw, "authors");
        if !authors.is_empty() {
            let ids: Vec<i64> = authors.iter().filter_map(|v| v.parse().ok()).collect();
            q.cond_where(
                Condition::any()
                    .add(col("p", "display_name").is_in(authors.clone()))
                    .add(col("c", "author_email").is_in(authors))
                    .add(col("ca", "contributor_id").is_in(ids)),
            );
        }
        Ok(Self {
            query: q.to_owned(),
            scoped_facts,
            source_ids,
            branches,
            current,
        })
    }
    fn select(&self) -> SelectStatement {
        Q::select()
            .from_subquery(self.query.clone(), a("f"))
            .to_owned()
    }
    fn branch_catalog(&self) -> SelectStatement {
        let mut q = Q::select();
        q.from_as(a("repository_analysis_refs"), a("r"))
            .join_as(
                JoinType::InnerJoin,
                a("repository_analysis_sources"),
                a("s"),
                col("s", "id").eq(col("r", "source_id")),
            )
            .expr_as(col("s", "repository_id"), a("repository_id"))
            .expr_as(col("r", "name"), a("branch_name"))
            .expr_as(col("r", "id").min(), a("id"))
            .expr_as(col("r", "present").max(), a("present"))
            .expr_as(col("s", "remote_path").min(), a("repository"))
            .and_where(col("s", "id").is_in(self.source_ids.iter().copied()))
            .group_by_columns([(a("s"), a("repository_id")), (a("r"), a("name"))]);
        if !self.branches.is_empty() {
            q.and_where(col("r", "name").is_in(self.branches.iter().cloned()));
        }
        if self.current {
            q.and_where(col("r", "present").eq(1));
        }
        q.to_owned()
    }
    fn branch_facts(&self, page: &[(i64, String)]) -> SelectStatement {
        let mut bridge = Q::select();
        bridge
            .distinct()
            .from_as(a("repository_analysis_ref_commits"), a("m"))
            .join_as(
                JoinType::InnerJoin,
                a("repository_analysis_refs"),
                a("r"),
                col("r", "id").eq(col("m", "ref_id")),
            )
            .join_as(
                JoinType::InnerJoin,
                a("repository_analysis_sources"),
                a("s"),
                col("s", "id").eq(col("m", "source_id")),
            )
            .expr_as(col("s", "repository_id"), a("repository_id"))
            .expr_as(col("r", "name"), a("branch_name"))
            .expr_as(col("m", "commit_id"), a("commit_id"))
            .and_where(col("s", "id").is_in(self.source_ids.iter().copied()));
        if !self.branches.is_empty() {
            bridge.and_where(col("r", "name").is_in(self.branches.iter().cloned()));
        }
        if self.current {
            bridge
                .and_where(col("r", "present").eq(1))
                .and_where(col("m", "current").eq(1));
        }
        if !page.is_empty() {
            let mut selected = Condition::any();
            for (repository, branch) in page {
                selected = selected.add(
                    Condition::all()
                        .add(col("s", "repository_id").eq(*repository))
                        .add(col("r", "name").eq(branch.clone())),
                );
            }
            bridge.cond_where(selected);
        }
        // Resolve the filtered cohort into an ID set before branch expansion.
        // Keeping only IDs in this semi-join avoids repeating coverage checks
        // for every branch while allowing indexed fact lookups on the outside.
        let mut eligible_ids = self.query.clone();
        eligible_ids.clear_selects().column((a("c"), a("id")));
        let mut facts = self.scoped_facts.clone();
        facts.and_where(col("c", "id").in_subquery(eligible_ids));
        Q::select()
            .columns(
                [
                    "id",
                    "repository_id",
                    "contributor_id",
                    "nickname",
                    "additions",
                    "deletions",
                    "effectives",
                ]
                .map(|name| (a("f"), a(name))),
            )
            .expr_as(col("b", "branch_name"), a("branch_name"))
            .from_subquery(facts, a("f"))
            .join_subquery(
                JoinType::InnerJoin,
                bridge.to_owned(),
                a("b"),
                col("b", "commit_id").eq(col("f", "id")),
            )
            .to_owned()
    }
}
fn sum(backend: DbBackend, table: &str, name: &str) -> SimpleExpr {
    col(table, name)
        .sum()
        .if_null(0)
        .cast_as(a(if backend == DbBackend::MySql {
            "SIGNED"
        } else {
            "BIGINT"
        }))
}
#[derive(FromQueryResult)]
struct Total {
    total: i64,
}
async fn rows<T: FromQueryResult>(s: &AppState, q: &SelectStatement) -> Result<Vec<T>> {
    Ok(T::find_by_statement(s.db.get_database_backend().build(q))
        .all(&s.db)
        .await?)
}
async fn count(s: &AppState, mut q: SelectStatement) -> Result<i64> {
    q.clear_order_by();
    let q = Q::select()
        .expr_as(Expr::cust("COUNT(*)"), a("total"))
        .from_subquery(q, a("counted"))
        .to_owned();
    Ok(rows::<Total>(s, &q)
        .await?
        .first()
        .map(|row| row.total)
        .unwrap_or(0))
}
fn grouped(
    cohort: &FactCohort,
    backend: DbBackend,
    period: &str,
    totals: bool,
) -> Result<SelectStatement> {
    let mut q = cohort.select();
    q.expr_as(
        if totals {
            Expr::val(0_i64).into()
        } else {
            col("f", "contributor_id")
        },
        a("contributor_id"),
    )
    .expr_as(
        if totals {
            Expr::val("").into()
        } else {
            col("f", "nickname").max()
        },
        a("nickname"),
    )
    .expr_as(period_expression(backend, period)?, a("period"))
    .expr_as(col("f", "repository_id").count_distinct(), a("projects"))
    .expr_as(Expr::cust("COUNT(*)"), a("commits"));
    for name in ["additions", "deletions", "effectives"] {
        q.expr_as(sum(backend, "f", name), a(name));
    }
    if !totals {
        q.group_by_col((a("f"), a("contributor_id")));
    }
    if !period.is_empty() {
        q.add_group_by([period_expression(backend, period)?]);
    }
    if totals && period.is_empty() {
        q.and_having(Expr::cust("COUNT(*) > 0"));
    }
    Ok(q)
}
#[derive(FromQueryResult)]
struct Aggregate {
    contributor_id: i64,
    nickname: String,
    period: String,
    projects: i64,
    commits: i64,
    additions: i64,
    deletions: i64,
    effectives: i64,
}
#[derive(FromQueryResult)]
struct AuthorMetadata {
    contributor_id: i64,
    period: String,
    author_name: String,
    author_email: String,
}
async fn aggregate_items(
    s: &AppState,
    cohort: &FactCohort,
    period: &str,
    totals: bool,
    aggregates: Vec<Aggregate>,
) -> Result<Vec<Value>> {
    if aggregates.is_empty() {
        return Ok(vec![]);
    }
    let mut q = cohort.select();
    q.distinct()
        .expr_as(
            if totals {
                Expr::val(0_i64).into()
            } else {
                col("f", "contributor_id")
            },
            a("contributor_id"),
        )
        .expr_as(
            period_expression(s.db.get_database_backend(), period)?,
            a("period"),
        )
        .column((a("f"), a("author_name")))
        .column((a("f"), a("author_email")));
    if !totals {
        q.and_where(
            col("f", "contributor_id").is_in(aggregates.iter().map(|row| row.contributor_id)),
        );
    }
    if !period.is_empty() {
        q.and_where(
            period_expression(s.db.get_database_backend(), period)?
                .is_in(aggregates.iter().map(|row| row.period.clone())),
        );
    }
    let mut names: BTreeMap<(i64, String), (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    for row in rows::<AuthorMetadata>(s, &q).await? {
        let entry = names.entry((row.contributor_id, row.period)).or_default();
        entry.0.insert(row.author_name);
        entry.1.insert(row.author_email);
    }
    aggregates.into_iter().map(|row|{
        let(names,emails)=names.remove(&(row.contributor_id,row.period.clone())).unwrap_or_default();
        let author=if totals{String::new()}else{Id::from(row.contributor_id).to_string()};
        Ok(json!({"id":encode(&(&row.period,&author))?,"authorKey":author,"period":row.period,"date":row.period,"nickname":row.nickname,
            "name":names.into_iter().collect::<Vec<_>>().join(","),"email":emails.into_iter().collect::<Vec<_>>().join(","),"projects":row.projects,
            "commits":row.commits,"additions":row.additions,"deletions":row.deletions,"effectives":row.effectives}))
    }).collect()
}
#[derive(FromQueryResult)]
struct BranchAggregate {
    id: i64,
    repository_id: i64,
    repository: String,
    branch_name: String,
    present: i64,
    commits: i64,
    additions: i64,
    deletions: i64,
    effectives: i64,
}
#[derive(FromQueryResult)]
struct BranchAuthor {
    repository_id: i64,
    branch_name: String,
    nickname: String,
}
fn branch_aggregate(cohort: &FactCohort, backend: DbBackend) -> SelectStatement {
    // Aggregate before the catalog LEFT JOIN: empty branches survive without
    // repeatedly scanning a materialized branch/commit payload for each branch.
    let mut facts = Q::select();
    facts
        .from_subquery(cohort.branch_facts(&[]), a("f"))
        .columns(["repository_id", "branch_name"].map(|name| (a("f"), a(name))))
        .expr_as(col("f", "id").count(), a("commits"))
        .group_by_columns(["repository_id", "branch_name"].map(|name| (a("f"), a(name))));
    for name in ["additions", "deletions", "effectives"] {
        facts.expr_as(sum(backend, "f", name), a(name));
    }
    let mut q = Q::select();
    q.from_subquery(cohort.branch_catalog(), a("b"))
        .join_subquery(
            JoinType::LeftJoin,
            facts.to_owned(),
            a("f"),
            Condition::all()
                .add(col("b", "repository_id").eq(col("f", "repository_id")))
                .add(col("b", "branch_name").eq(col("f", "branch_name"))),
        )
        .columns(
            [
                "id",
                "repository_id",
                "repository",
                "branch_name",
                "present",
            ]
            .map(|name| (a("b"), a(name))),
        );
    for name in ["commits", "additions", "deletions", "effectives"] {
        q.expr_as(col("f", name).if_null(0), a(name));
    }
    q.to_owned()
}

fn validate_kind(kind: &str) -> Result<()> {
    if ![
        "overview",
        "contributors",
        "ranking",
        "branches",
        "commits",
        "heatmap",
        "period",
    ]
    .contains(&kind)
    {
        return Err(ApiError::missing());
    }
    Ok(())
}

pub async fn report(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, kind)): Path<(String, String)>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("repositories.read")?;
    validate_kind(&kind)?;
    let mut owned = connections::Entity::find()
        .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(connections::Column::UserId.eq(scope.user.id))
        .filter(connections::Column::State.eq("connected"));
    if let Some(id) = raw.get("connection_id").filter(|v| !v.is_empty()) {
        let id: Id = id.parse().map_err(|_| ApiError::missing())?;
        owned = owned.filter(connections::Column::Id.eq(id));
    }
    let ids: Vec<i64> = owned.all(&s.db).await?.iter().map(|row| row.id).collect();
    let settings: HashMap<i64, Config> = configs::Entity::find()
        .filter(configs::Column::Id.is_in(ids.clone()))
        .all(&s.db)
        .await?
        .iter()
        .map(|row| decode(row).map(|config| (row.id, config)))
        .collect::<Result<_>>()?;
    let mut source_rows = sources::Entity::find()
        .filter(sources::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(sources::Column::State.ne("unavailable"))
        .filter(sources::Column::ConnectionId.is_in(ids))
        .order_by_asc(sources::Column::CoverageStart)
        .all(&s.db)
        .await?;
    source_rows.retain(|row| {
        settings.get(&row.connection_id).is_some_and(|config| {
            config.all_repositories || config.repositories.contains(&row.remote_path)
        })
    });
    let projection = Projection::Legacy(scope.user.id.into());
    render(s, scope, kind, raw, source_rows, projection).await
}

pub async fn insight_report(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, kind)): Path<(String, Id, String)>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("code-insights.read")?;
    scope.require("repositories.read")?;
    validate_kind(&kind)?;
    let insight = insights::owned(&s.db, &scope, id).await?;
    let source_rows = insights::selected_sources(&s.db, &scope, &insight).await?;
    let identity_error = insights::decode(&insight)?.identity_error;
    let Json(mut result) = render(
        s,
        scope,
        kind,
        raw,
        source_rows,
        Projection::Insight {
            id: insight.id,
            coverage_start: insight.coverage_start,
        },
    )
    .await?;
    if !identity_error.is_empty() {
        result["coverage"]["complete"] = json!(false);
        result["coverage"]["last_error"] = json!(crate::i18n::display_stored(&identity_error));
    }
    Ok(Json(result))
}

async fn render(
    s: AppState,
    scope: Scope,
    kind: String,
    raw: HashMap<String, String>,
    mut source_rows: Vec<sources::Model>,
    projection: Projection,
) -> Result<Json<Value>> {
    let repo_rows: HashMap<i64, repositories::Model> = repositories::Entity::find()
        .filter(repositories::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(repositories::Column::Id.is_in(source_rows.iter().map(|row| row.repository_id)))
        .all(&s.db)
        .await?
        .into_iter()
        .map(|row| (row.id, row))
        .collect();
    let selected = list(&raw, "repos");
    source_rows.retain(|row| {
        selected.is_empty()
            || selected.contains(&row.remote_path)
            || repo_rows.get(&row.repository_id).is_some_and(|repo| {
                selected.contains(&repo.url) || selected.contains(&repo.id.to_string())
            })
    });
    let cohort = FactCohort::with_projection(
        scope.workspace_id.into(),
        source_rows.iter().map(|row| row.id).collect(),
        &raw,
        projection,
    )?;
    let coverage = json!({"start":source_rows.iter().map(|row|row.coverage_start).reduce(coverage_union),"complete":!source_rows.is_empty()&&source_rows.iter().all(|row|row.state=="complete"),
        "sources":source_rows.iter().map(|row|json!({"repository":row.remote_path,"state":row.state,"synced_at":row.synced_at,"coverage_start":row.coverage_start,"last_error":crate::i18n::display_stored(&row.last_error)})).collect::<Vec<_>>(),
        "date_filter":"committer_date","date_bucket":"author_date","timezone":"UTC","history":if cohort.current{"current"}else{"observed"}});
    let (page, size, offset) = pagination(&raw);
    let backend = s.db.get_database_backend();
    if kind == "commits" {
        let total = count(&s, cohort.query.clone()).await?;
        let mut q = cohort.select();
        q.column((a("f"), sea_orm::sea_query::Asterisk))
            .order_by((a("f"), a("committer_date")), Order::Desc)
            .order_by((a("f"), a("sha")), Order::Asc)
            .limit(size)
            .offset(offset);
        let result = s.db.query_all(&q).await?;
        let mut items = Vec::new();
        let mut commit_ids = Vec::new();
        for row in result {
            let v = commits::Model::from_query_result(&row, "")?;
            commit_ids.push(v.id);
            let repo = repo_rows.get(&v.repository_id);
            let commit_url = repo.and_then(|repo| {
                let url = repo.url.trim_end_matches('/').trim_end_matches(".git");
                (url.starts_with("https://") || url.starts_with("http://")).then(|| {
                    format!(
                        "{url}/{}commit/{}",
                        if repo.provider == "gitlab" { "-/" } else { "" },
                        v.sha
                    )
                })
            });
            items.push(json!({"id":Id::from(v.id),"repoUrl":repo.map(|r|&r.url),"repository":repo.map(|r|&r.path),"provider":repo.map(|r|&r.provider),"commitUrl":commit_url,
                "commitHash":v.sha,"message":v.message,"messageType":v.message_type,"isMerge":v.is_merge==1,"authorName":v.author_name,"authorEmail":v.author_email,
                "nickname":row.try_get::<String>("","nickname")?,"authorKey":Id::from(row.try_get::<i64>("","contributor_id")?).to_string(),"date":v.author_date,"committerDate":v.committer_date,
                "additions":v.additions,"deletions":v.deletions,"effectives":v.effectives,"languageStats":serde_json::from_str::<Value>(&v.language_stats).unwrap_or(json!({}))}));
        }
        let mut q = Q::select();
        q.from_as(a("repository_analysis_ref_commits"), a("m"))
            .join_as(
                JoinType::InnerJoin,
                a("repository_analysis_refs"),
                a("r"),
                col("r", "id").eq(col("m", "ref_id")),
            )
            .column((a("m"), a("commit_id")))
            .expr_as(col("r", "name"), a("branch_name"))
            .and_where(col("m", "source_id").is_in(cohort.source_ids.iter().copied()))
            .and_where(col("m", "commit_id").is_in(commit_ids));
        if cohort.current {
            q.and_where(col("m", "current").eq(1))
                .and_where(col("r", "present").eq(1));
        }
        if !cohort.branches.is_empty() {
            q.and_where(col("r", "name").is_in(cohort.branches.iter().cloned()));
        }
        #[derive(FromQueryResult)]
        struct Membership {
            commit_id: i64,
            branch_name: String,
        }
        let mut membership: HashMap<i64, BTreeSet<String>> = HashMap::new();
        for row in rows::<Membership>(&s, &q).await? {
            membership
                .entry(row.commit_id)
                .or_default()
                .insert(row.branch_name);
        }
        for item in &mut items {
            let id = item["id"]
                .as_str()
                .and_then(|id| id.parse::<i64>().ok())
                .unwrap_or_default();
            item["branches"] = json!(membership.remove(&id).unwrap_or_default());
        }
        return Ok(Json(
            json!({"items":items,"total":total,"page":page,"page_size":size,"coverage":coverage}),
        ));
    }
    if kind == "branches" {
        let mut q = branch_aggregate(&cohort, backend);
        let total = count(&s, cohort.branch_catalog()).await?;
        q.order_by(a("effectives"), Order::Desc)
            .order_by((a("b"), a("repository_id")), Order::Asc)
            .order_by((a("b"), a("branch_name")), Order::Asc)
            .limit(size)
            .offset(offset);
        let aggregates = rows::<BranchAggregate>(&s, &q).await?;
        let selected_branches: Vec<_> = aggregates
            .iter()
            .map(|row| (row.repository_id, row.branch_name.clone()))
            .collect();
        let mut q = Q::select();
        q.distinct()
            .from_subquery(cohort.branch_facts(&selected_branches), a("f"))
            .columns(["repository_id", "branch_name", "nickname"].map(|name| (a("f"), a(name))));
        let mut nicknames: HashMap<(i64, String), BTreeSet<String>> = HashMap::new();
        if !aggregates.is_empty() {
            for row in rows::<BranchAuthor>(&s, &q).await? {
                nicknames
                    .entry((row.repository_id, row.branch_name))
                    .or_default()
                    .insert(row.nickname);
            }
        }
        let items:Vec<Value>=aggregates.into_iter().map(|row|json!({"id":Id::from(row.id),"repository":row.repository,"repoUrl":repo_rows.get(&row.repository_id).map(|repo|&repo.url),"branchName":row.branch_name,"present":row.present==1,
            "nicknames":nicknames.remove(&(row.repository_id,row.branch_name.clone())).unwrap_or_default(),"commits":row.commits,"additions":row.additions,"deletions":row.deletions,"effectives":row.effectives})).collect();
        return Ok(Json(
            json!({"items":items,"total":total,"page":page,"page_size":size,"coverage":coverage}),
        ));
    }
    let period = if kind == "heatmap" {
        "day"
    } else if kind == "period" {
        raw.get("period").map(String::as_str).unwrap_or("day")
    } else {
        ""
    };
    let totals = kind == "overview" || kind == "heatmap";
    let mut q = grouped(&cohort, backend, period, totals)?;
    if period.is_empty() {
        q.order_by(a("effectives"), Order::Desc)
            .order_by(a("contributor_id"), Order::Asc);
    } else {
        q.order_by(a("period"), Order::Asc)
            .order_by(a("contributor_id"), Order::Asc);
    }
    if kind == "period" || kind == "heatmap" {
        // Complete chart series are an explicit contract. Only chart endpoints
        // materialize all groups; list-only reports group and page in the DB.
        let series = aggregate_items(
            &s,
            &cohort,
            period,
            totals,
            rows::<Aggregate>(&s, &q).await?,
        )
        .await?;
        let total = series.len();
        let items: Vec<_> = series
            .iter()
            .skip(usize::try_from(offset).unwrap_or(usize::MAX))
            .take(size as usize)
            .cloned()
            .collect();
        return Ok(Json(
            json!({"items":items,"total":total,"page":page,"page_size":size,"coverage":coverage,"series":series}),
        ));
    }
    let total = count(&s, q.clone()).await?;
    q.limit(size).offset(offset);
    let items = aggregate_items(
        &s,
        &cohort,
        period,
        totals,
        rows::<Aggregate>(&s, &q).await?,
    )
    .await?;
    Ok(Json(
        json!({"items":items,"total":total,"page":page,"page_size":size,"coverage":coverage,"series":null}),
    ))
}
