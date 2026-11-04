use super::*;
use sea_orm::{Condition, QuerySelect};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(super) async fn work(s: &AppState) -> Result<()> {
    let now = db::now();
    let queue = runs::Entity::find()
        .filter(
            Condition::any().add(runs::Column::State.eq("queued")).add(
                Condition::all()
                    .add(runs::Column::State.eq("running"))
                    .add(runs::Column::LeaseUntil.lte(now)),
            ),
        )
        .order_by_asc(runs::Column::CreatedAt)
        .limit(10)
        .all(&s.db)
        .await?;
    for run in queue {
        let token = uuid::Uuid::new_v4().to_string();
        let claim_time = db::now();
        let changed = runs::Entity::update_many()
            .col_expr(runs::Column::State, Expr::value("running"))
            .col_expr(runs::Column::LeaseToken, Expr::value(token.clone()))
            .col_expr(runs::Column::LeaseUntil, Expr::value(claim_time + 300_000))
            .filter(runs::Column::Id.eq(run.id))
            .filter(runs::Column::State.eq(run.state.clone()))
            .filter(runs::Column::LeaseUntil.eq(run.lease_until))
            .exec(&s.db)
            .await?;
        if changed.rows_affected != 1 {
            continue;
        }
        // One transaction fixes identity/source/config reads and the resulting revision.
        let outcome = build(s, &run).await;
        match outcome {
            Ok(snapshot) => {
                let tx = s.db.begin().await?;
                let changed = runs::Entity::update_many()
                    .col_expr(runs::Column::State, Expr::value("complete"))
                    .col_expr(runs::Column::Snapshot, Expr::value(encode(&snapshot)?))
                    .col_expr(runs::Column::GeneratedAt, Expr::value(db::now()))
                    .col_expr(runs::Column::LeaseUntil, Expr::value(0_i64))
                    .filter(runs::Column::Id.eq(run.id))
                    .filter(runs::Column::LeaseToken.eq(token))
                    .filter(runs::Column::State.eq("running"))
                    .exec(&tx)
                    .await?;
                if changed.rows_affected == 1 {
                    defs::Entity::update_many()
                        .col_expr(defs::Column::LastGeneratedAt, Expr::value(db::now()))
                        .filter(defs::Column::Id.eq(run.definition_id))
                        .exec(&tx)
                        .await?;
                }
                tx.commit().await?;
            }
            Err(error) => {
                runs::Entity::update_many()
                    .col_expr(runs::Column::State, Expr::value("failed"))
                    .col_expr(runs::Column::Error, Expr::value(error.stored()))
                    .col_expr(runs::Column::LeaseUntil, Expr::value(0_i64))
                    .filter(runs::Column::Id.eq(run.id))
                    .filter(runs::Column::LeaseToken.eq(token))
                    .filter(runs::Column::State.eq("running"))
                    .exec(&s.db)
                    .await?;
            }
        }
    }
    Ok(())
}
fn digest<T: Serialize>(value: &T) -> Result<String> {
    Ok(hex::encode(Sha256::digest(encode(value)?.as_bytes())))
}
async fn build(s: &AppState, run: &runs::Model) -> Result<Value> {
    let tx = s.db.begin().await?;
    let scope = Scope::for_actor_in(&tx, run.workspace_id.into(), run.user_id.into()).await?;
    scope.require("insight-reports.generate")?;
    owned(&tx, &scope, run.definition_id.into()).await?;
    let config: Config =
        serde_json::from_str(&run.config_snapshot).map_err(|_| bad("configuration"))?;
    let insight = source(&tx, &scope, &config).await?;
    let sources = authorized_sources(&tx, &scope, &insight).await?;
    if config.dimension == "application" {
        scope.require("deployments.read")?;
        for app in &config.applications {
            crate::entities::deployments::Entity::find_by_id(key(&app.application_id)?)
                .filter(crate::entities::deployments::Column::WorkspaceId.eq(scope.workspace_id))
                .one(&tx)
                .await?
                .ok_or_else(ApiError::missing)?;
        }
    }
    let tz = period::timezone(&config.timezone)?;
    let previous_date = period::local_date(run.period_start, tz)? - chrono::Duration::days(1);
    let (previous_start, previous_end) = period::bounds(&run.period, previous_date, tz)?;
    let mut selected_repos = BTreeSet::new();
    let mut app_repos = BTreeMap::new();
    let mut missing = vec![];
    for app in &config.applications {
        let mut ids = BTreeSet::new();
        for repo in &app.repositories {
            match sources.iter().find(|s|s.id!=0&&s.connection_id.to_string()==repo.connection_id&&s.remote_path==repo.repository){Some(s)=>{ids.insert(s.repository_id.to_string());selected_repos.insert(s.repository_id.to_string());},None=>missing.push(json!({"application_id":app.application_id,"connection_id":repo.connection_id,"repository":repo.repository,"state":"unavailable"}))}
        }
        app_repos.insert(app.application_id.clone(), ids);
    }
    let report_sources: Vec<_> = sources
        .iter()
        .filter(|source| {
            config.dimension == "person"
                || selected_repos.contains(&source.repository_id.to_string())
        })
        .cloned()
        .collect();
    let facts = analysis::report_facts(
        &tx,
        &scope,
        &insight,
        &report_sources,
        previous_start,
        run.period_end,
        if config.dimension == "person" {
            Some(&config.author_keys)
        } else {
            None
        },
    )
    .await?;
    let current: Vec<_> = facts
        .iter()
        .filter(|f| f.author_date >= run.period_start && f.author_date < run.period_end)
        .collect();
    let previous: Vec<_> = facts
        .iter()
        .filter(|f| f.author_date >= previous_start && f.author_date < previous_end)
        .collect();
    let mut groups = vec![];
    if config.dimension == "person" {
        for author in &config.author_keys {
            let items: Vec<_> = current
                .iter()
                .filter(|f| &f.author_key == author)
                .cloned()
                .collect();
            let name = items
                .first()
                .map(|f| f.nickname.clone())
                .unwrap_or_else(|| author.clone());
            let mut group = metrics(&items, tz)?;
            group["key"] = json!(author);
            group["name"] = json!(name);
            groups.push(group);
        }
    } else {
        for app in &config.applications {
            let ids = &app_repos[&app.application_id];
            let items: Vec<_> = current
                .iter()
                .filter(|f| ids.contains(&f.repository_id))
                .cloned()
                .collect();
            let application =
                crate::entities::deployments::Entity::find_by_id(key(&app.application_id)?)
                    .one(&tx)
                    .await?
                    .ok_or_else(ApiError::missing)?;
            let mut group = metrics(&items, tz)?;
            group["key"] = json!(app.application_id);
            group["name"] = json!(application.name);
            groups.push(group);
        }
    }
    let mut days: BTreeMap<String, Vec<&analysis::ReportFact>> = BTreeMap::new();
    for fact in &current {
        let day = period::local_date(fact.author_date, tz)?.to_string();
        days.entry(day).or_default().push(*fact);
    }
    let trend = days
        .into_iter()
        .map(|(date, items)| {
            let mut m = metrics(&items, tz)?;
            m["date"] = json!(date);
            let mut authors: BTreeMap<&str, Vec<&analysis::ReportFact>> = BTreeMap::new();
            for fact in &items {
                authors.entry(&fact.author_key).or_default().push(*fact);
            }
            m["authors"] = json!(
                authors
                    .into_iter()
                    .map(|(key, values)| {
                        let mut value = metrics(&values, tz)?;
                        value["key"] = json!(key);
                        value["authorKey"] = json!(key);
                        value["name"] = json!(values[0].nickname);
                        value["nickname"] = json!(values[0].nickname);
                        Ok(value)
                    })
                    .collect::<Result<Vec<_>>>()?
            );
            Ok(m)
        })
        .collect::<Result<Vec<_>>>()?;
    let source_json:Vec<_>=sources.iter().filter(|source|config.dimension=="person"||selected_repos.contains(&source.repository_id.to_string())).map(|source|json!({"connection_id":Id::from(source.connection_id),"repository":source.remote_path,"repository_id":Id::from(source.repository_id),"state":source.state,"synced_at":source.synced_at,"coverage_start":source.coverage_start,"config_version":source.config_version,"last_error":source.last_error})).collect();
    let identity = insights::decode(&insight)?;
    let complete = !source_json.is_empty()
        && missing.is_empty()
        && identity.identity_error.is_empty()
        && source_json.iter().all(|s| {
            s["state"] == "complete"
                && s["coverage_start"]
                    .as_i64()
                    .is_some_and(|start| start == 0 || start <= previous_start)
                && s["synced_at"]
                    .as_i64()
                    .is_some_and(|at| at >= run.period_end)
        });
    let people = crate::entities::code_insight_people::Entity::find()
        .filter(crate::entities::code_insight_people::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(crate::entities::code_insight_people::Column::InsightId.eq(insight.id))
        .all(&tx)
        .await?;
    let active_people: BTreeSet<_> = facts.iter().map(|fact| fact.author_key.as_str()).collect();
    let identity_snapshot: Vec<_> = people
        .iter()
        .filter(|person| active_people.contains(person.id.to_string().as_str()))
        .map(|p| json!({"key":Id::from(p.id),"name":p.display_name,"version":p.version}))
        .collect();
    let aliases: Vec<_> = identity
        .aliases
        .iter()
        .filter(|alias| active_people.contains(alias.contributor_id.as_str()))
        .collect();
    let identity_version = digest(&json!({"people":identity_snapshot,"aliases":aliases}))?;
    let repository_scope:Vec<_>=source_json.iter().map(|source|json!({"connection_id":source["connection_id"],"repository":source["repository"],"repository_id":source["repository_id"],"config_version":source["config_version"],"coverage_start":source["coverage_start"]})).collect();
    let repository_scope_version =
        digest(&json!({"sources":repository_scope,"applications":config.applications}))?;
    let value = json!({"summary":metrics(&current,tz)?,"previous":metrics(&previous,tz)?,"trend":trend,"groups":groups,"commits":current,"coverage":{"complete":complete,"sources":source_json,"missing_repositories":missing,"identity_error":identity.identity_error,"date_filter":"author_date","date_bucket":"author_date"},"config":config,"identity_version":identity_version,"identities":identity_snapshot,"identity_aliases":aliases,"repository_scope_version":repository_scope_version,"source_insight_version":insight.version,"timezone":config.timezone,"preview":run.period_end>db::now(),"period_start":run.period_start,"period_end":run.period_end,"generated_at":db::now()});
    tx.commit().await?;
    Ok(value)
}
pub(super) fn metrics(facts: &[&analysis::ReportFact], tz: chrono_tz::Tz) -> Result<Value> {
    let mut days = BTreeSet::new();
    let mut repos = BTreeSet::new();
    let mut people = BTreeSet::new();
    let (mut additions, mut deletions, mut effectives) = (0_i64, 0_i64, 0_i64);
    for fact in facts {
        days.insert(period::local_date(fact.author_date, tz)?);
        repos.insert(&fact.repository_id);
        people.insert(&fact.author_key);
        additions += fact.additions;
        deletions += fact.deletions;
        effectives += fact.effectives;
    }
    Ok(
        json!({"commits":facts.len(),"additions":additions,"deletions":deletions,"effectives":effectives,"active_days":days.len(),"repository_count":repos.len(),"contributor_count":people.len()}),
    )
}
