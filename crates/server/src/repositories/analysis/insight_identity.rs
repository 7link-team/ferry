//! Insight-local display identities over immutable shared Git authors.
use super::*;
use crate::entities::{
    code_insight_identity_links as links, code_insight_people as people,
    code_insight_repositories as selections, code_insights,
    repository_analysis_commit_authors as legacy_authors,
    repository_analysis_commit_identities as raw_authors,
    repository_analysis_identities as identities, repository_analysis_people as legacy_people,
};
use sea_orm::{
    FromQueryResult,
    sea_query::{Alias as A, JoinType, OnConflict, Query as Q},
};

const BATCH: u64 = 500;

struct Rule {
    name: String,
    email: String,
    person: i64,
}
impl Rule {
    fn matches(&self, name: &str, email: &str) -> bool {
        self.name == name && self.email == email
    }
}
fn same_rule(a: &Alias, b: &Alias) -> bool {
    a.name == b.name && a.email == b.email
}
fn validate_rules(rules: &[Rule]) -> Result<()> {
    let mut pairs = BTreeMap::new();
    for rule in rules {
        if pairs
            .insert((&rule.name, &rule.email), rule.person)
            .is_some_and(|person| person != rule.person)
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("repository.analysis.aliasAmbiguous"),
            ));
        }
    }
    Ok(())
}

fn rules(aliases: &[Alias]) -> Result<Vec<Rule>> {
    aliases
        .iter()
        .map(|alias| {
            Ok(Rule {
                name: alias.name.clone(),
                email: alias.email.clone(),
                person: alias
                    .contributor_id
                    .parse::<Id>()
                    .map(i64::from)
                    .map_err(|_| {
                        ApiError::localized(
                            axum::http::StatusCode::BAD_REQUEST,
                            crate::i18n::Message::new("repository.analysis.contributorInvalid"),
                        )
                    })?,
            })
        })
        .collect()
}

/// Present newly collected exact Git author pairs without rewriting user mappings
/// or changing the insight's configuration version during background collection.
pub(super) async fn discover_aliases(
    tx: &impl ConnectionTrait,
    insight: &code_insights::Model,
    source_rows: &[sources::Model],
    aliases: &mut Vec<Alias>,
) -> Result<()> {
    let repositories: BTreeSet<_> = source_rows
        .iter()
        .filter(|row| row.id > 0)
        .map(|row| row.repository_id)
        .collect();
    if repositories.is_empty() {
        return Ok(());
    }
    let authors = commits::Entity::find()
        .select_only()
        .column(commits::Column::AuthorName)
        .column(commits::Column::AuthorEmail)
        .distinct()
        .filter(commits::Column::WorkspaceId.eq(insight.workspace_id))
        .filter(commits::Column::RepositoryId.is_in(repositories))
        .order_by_asc(commits::Column::AuthorName)
        .order_by_asc(commits::Column::AuthorEmail)
        .into_tuple::<(String, String)>()
        .all(tx)
        .await?;
    let mut known: BTreeSet<_> = aliases
        .iter()
        .map(|a| (a.name.clone(), a.email.clone()))
        .collect();
    for (name, email) in authors {
        if known.insert((name.clone(), email.clone())) {
            aliases.push(Alias {
                name,
                email,
                ..Default::default()
            });
        }
    }
    Ok(())
}

async fn person(
    tx: &impl ConnectionTrait,
    insight: &code_insights::Model,
    id: i64,
) -> Result<people::Model> {
    people::Entity::find_by_id((insight.id, id))
        .filter(people::Column::WorkspaceId.eq(insight.workspace_id))
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)
}

pub(super) async fn prepare_aliases(
    s: &AppState,
    tx: &impl ConnectionTrait,
    insight: &code_insights::Model,
    old: &[Alias],
    new: &mut [Alias],
) -> Result<()> {
    let mut labels = BTreeMap::new();
    for alias in new.iter_mut() {
        alias.nickname = alias.nickname.trim().into();
        if alias.nickname.is_empty() || alias.nickname.chars().count() > 255 {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.analysis.aliasInvalid"),
            ));
        }
        let prior: Vec<_> = old
            .iter()
            .filter(|previous| same_rule(previous, alias))
            .collect();
        let mut ids = BTreeSet::new();
        if !alias.contributor_id.is_empty() {
            ids.insert(
                alias
                    .contributor_id
                    .parse::<Id>()
                    .map(i64::from)
                    .map_err(|_| {
                        ApiError::localized(
                            axum::http::StatusCode::BAD_REQUEST,
                            crate::i18n::Message::new("repository.analysis.contributorInvalid"),
                        )
                    })?,
            );
        } else {
            for previous in &prior {
                ids.insert(
                    previous
                        .contributor_id
                        .parse::<Id>()
                        .map(i64::from)
                        .map_err(|_| {
                            ApiError::localized(
                                axum::http::StatusCode::BAD_REQUEST,
                                crate::i18n::Message::new("repository.analysis.contributorInvalid"),
                            )
                        })?,
                );
            }
        }
        if ids.len() > 1 {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("repository.analysis.aliasAmbiguous"),
            ));
        }
        let mut row = if let Some(id) = ids.first() {
            person(tx, insight, *id).await?
        } else {
            people::Entity::insert(
                people::Model {
                    insight_id: insight.id,
                    id: s.next_id()?.into(),
                    workspace_id: insight.workspace_id,
                    display_name: alias.nickname.clone(),
                    version: 1,
                    raw_identity_id: None,
                }
                .into_active_model(),
            )
            .exec_with_returning(tx)
            .await?
        };
        let rename = prior.is_empty()
            || prior
                .iter()
                .any(|previous| previous.nickname.trim() != alias.nickname);
        if rename && row.display_name != alias.nickname {
            if row.raw_identity_id.is_some() {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.analysis.rawAuthorImmutable"),
                ));
            }
            row.display_name = alias.nickname.clone();
            row.version += 1;
            people::Entity::update(row.clone().into_active_model().reset_all())
                .exec(tx)
                .await?;
        }
        alias.nickname = row.display_name;
        alias.contributor_id = row.id.to_string();
        if labels
            .insert(row.id, alias.nickname.clone())
            .is_some_and(|label| label != alias.nickname)
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("repository.analysis.nicknameConflict"),
            ));
        }
    }
    validate_rules(&rules(new)?)
}

pub(super) async fn hydrate_aliases(
    tx: &impl ConnectionTrait,
    insight: &code_insights::Model,
    aliases: &mut [Alias],
) -> Result<()> {
    for alias in aliases {
        let id = alias
            .contributor_id
            .parse::<Id>()
            .map(i64::from)
            .map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.analysis.contributorInvalid"),
                )
            })?;
        alias.nickname = person(tx, insight, id).await?.display_name;
    }
    Ok(())
}

/// Copy legacy labels once, retaining IDs without sharing mutable person rows.
pub(super) async fn import_aliases(
    s: &AppState,
    tx: &impl ConnectionTrait,
    insight: &code_insights::Model,
    aliases: &[Alias],
) -> Result<Vec<Alias>> {
    // Imported insight IDs are the original connection IDs. Copy that
    // connection's raw defaults too, so existing author filters keep their IDs.
    let identities = Q::select()
        .column(legacy_authors::Column::IdentityId)
        .from(legacy_authors::Entity)
        .join(
            JoinType::InnerJoin,
            sources::Entity,
            Expr::col((sources::Entity, sources::Column::RepositoryId)).eq(Expr::col((
                legacy_authors::Entity,
                legacy_authors::Column::RepositoryId,
            ))),
        )
        .and_where(Expr::col((sources::Entity, sources::Column::ConnectionId)).eq(insight.id))
        .and_where(
            Expr::col((sources::Entity, sources::Column::WorkspaceId)).eq(insight.workspace_id),
        )
        .and_where(
            Expr::col((legacy_authors::Entity, legacy_authors::Column::UserId)).eq(insight.user_id),
        )
        .to_owned();
    let mut cursor = 0;
    loop {
        let defaults = legacy_people::Entity::find()
            .filter(legacy_people::Column::WorkspaceId.eq(insight.workspace_id))
            .filter(legacy_people::Column::UserId.eq(insight.user_id))
            .filter(legacy_people::Column::RawIdentityId.in_subquery(identities.clone()))
            .filter(legacy_people::Column::Id.gt(cursor))
            .order_by_asc(legacy_people::Column::Id)
            .limit(BATCH)
            .all(tx)
            .await?;
        if defaults.is_empty() {
            break;
        }
        cursor = defaults.last().unwrap().id;
        for legacy in defaults {
            copy_person(tx, insight, &legacy).await?;
        }
    }
    let mut imported = aliases.to_vec();
    for alias in &mut imported {
        if alias.contributor_id.is_empty() {
            continue;
        }
        let id = alias
            .contributor_id
            .parse::<Id>()
            .map(i64::from)
            .map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.analysis.contributorInvalid"),
                )
            })?;
        let legacy = legacy_people::Entity::find_by_id(id)
            .filter(legacy_people::Column::WorkspaceId.eq(insight.workspace_id))
            .filter(legacy_people::Column::UserId.eq(insight.user_id))
            .one(tx)
            .await?
            .ok_or_else(ApiError::missing)?;
        copy_person(tx, insight, &legacy).await?;
        alias.nickname = legacy.display_name;
    }
    prepare_aliases(s, tx, insight, &[], &mut imported).await?;
    Ok(imported)
}

async fn copy_person(
    tx: &impl ConnectionTrait,
    insight: &code_insights::Model,
    legacy: &legacy_people::Model,
) -> Result<()> {
    people::Entity::insert(
        people::Model {
            insight_id: insight.id,
            id: legacy.id,
            workspace_id: insight.workspace_id,
            display_name: legacy.display_name.clone(),
            version: legacy.version,
            raw_identity_id: legacy.raw_identity_id,
        }
        .into_active_model(),
    )
    .on_conflict(
        OnConflict::columns([people::Column::InsightId, people::Column::Id])
            .do_nothing_on([people::Column::InsightId, people::Column::Id])
            .to_owned(),
    )
    .exec_without_returning(tx)
    .await?;
    Ok(())
}

/// Materialize missing links during configuration or publication, without remote work.
pub(super) async fn ensure(
    s: &AppState,
    tx: &impl ConnectionTrait,
    insight: &code_insights::Model,
    source_rows: &[sources::Model],
) -> Result<()> {
    ensure_mode(s, tx, insight, source_rows, true).await
}

async fn ensure_mode(
    s: &AppState,
    tx: &impl ConnectionTrait,
    insight: &code_insights::Model,
    source_rows: &[sources::Model],
    strict: bool,
) -> Result<()> {
    let config = insights::decode(insight)?;
    let rules = rules(&config.aliases)?;
    let mut conflict = validate_rules(&rules).is_err();
    if strict {
        validate_rules(&rules)?;
    }
    let repositories = source_rows
        .iter()
        .map(|row| row.repository_id)
        .collect::<BTreeSet<_>>();
    if repositories.is_empty() {
        return Ok(());
    }
    loop {
        let raw_link = Q::select()
            .expr(Expr::val(1))
            .from(raw_authors::Entity)
            .and_where(
                Expr::col(raw_authors::Column::CommitId)
                    .eq(Expr::col((commits::Entity, commits::Column::Id))),
            )
            .to_owned();
        let facts = commits::Entity::find()
            .filter(commits::Column::WorkspaceId.eq(insight.workspace_id))
            .filter(commits::Column::RepositoryId.is_in(repositories.iter().copied()))
            .filter(Expr::exists(raw_link).not())
            .limit(BATCH)
            .all(tx)
            .await?;
        if facts.is_empty() {
            break;
        }
        identity::raw_identities(s, tx, insight.workspace_id, &facts).await?;
    }
    loop {
        let q = Q::select()
            .distinct()
            .column((A::new("i"), sea_orm::sea_query::Asterisk))
            .from_as(identities::Entity, A::new("i"))
            .join_as(
                JoinType::InnerJoin,
                raw_authors::Entity,
                A::new("ci"),
                Expr::col((A::new("ci"), raw_authors::Column::IdentityId))
                    .eq(Expr::col((A::new("i"), identities::Column::Id))),
            )
            .join_as(
                JoinType::InnerJoin,
                commits::Entity,
                A::new("c"),
                Expr::col((A::new("c"), commits::Column::Id))
                    .eq(Expr::col((A::new("ci"), raw_authors::Column::CommitId))),
            )
            .join_as(
                JoinType::LeftJoin,
                links::Entity,
                A::new("l"),
                sea_orm::Condition::all()
                    .add(Expr::col((A::new("l"), links::Column::InsightId)).eq(insight.id))
                    .add(
                        Expr::col((A::new("l"), links::Column::IdentityId))
                            .eq(Expr::col((A::new("i"), identities::Column::Id))),
                    ),
            )
            .and_where(
                Expr::col((A::new("i"), identities::Column::WorkspaceId)).eq(insight.workspace_id),
            )
            .and_where(
                Expr::col((A::new("c"), commits::Column::WorkspaceId)).eq(insight.workspace_id),
            )
            .and_where(
                Expr::col((A::new("c"), commits::Column::RepositoryId))
                    .is_in(repositories.iter().copied()),
            )
            .and_where(Expr::col((A::new("l"), links::Column::IdentityId)).is_null())
            .limit(BATCH)
            .to_owned();
        let batch = tx
            .query_all_raw(tx.get_database_backend().build(&q))
            .await?
            .iter()
            .map(|row| identities::Model::from_query_result(row, ""))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if batch.is_empty() {
            break;
        }
        for raw in batch {
            let default = people::Entity::find()
                .filter(people::Column::InsightId.eq(insight.id))
                .filter(people::Column::WorkspaceId.eq(insight.workspace_id))
                .filter(people::Column::RawIdentityId.eq(raw.id))
                .one(tx)
                .await?;
            let default = if let Some(row) = default {
                row.id
            } else {
                let row = people::Model {
                    insight_id: insight.id,
                    id: s.next_id()?.into(),
                    workspace_id: insight.workspace_id,
                    display_name: raw.raw_name.clone(),
                    version: 1,
                    raw_identity_id: Some(raw.id),
                };
                let id = row.id;
                people::Entity::insert(row.into_active_model())
                    .exec(tx)
                    .await?;
                id
            };
            let matching: BTreeSet<_> = rules
                .iter()
                .filter(|rule| rule.matches(&raw.raw_name, &raw.raw_email))
                .map(|rule| rule.person)
                .collect();
            if matching.len() > 1 {
                if strict {
                    return Err(ApiError::localized(
                        axum::http::StatusCode::CONFLICT,
                        crate::i18n::Message::new("repository.identity.insightConflict"),
                    ));
                }
                conflict = true;
            }
            let contributor_id = if matching.len() == 1 {
                *matching.first().unwrap()
            } else {
                default
            };
            // A serialized config must never be able to reference another insight's person.
            person(tx, insight, contributor_id).await?;
            links::Entity::insert(
                links::Model {
                    insight_id: insight.id,
                    identity_id: raw.id,
                    workspace_id: insight.workspace_id,
                    contributor_id,
                }
                .into_active_model(),
            )
            .exec(tx)
            .await?;
        }
    }
    if conflict {
        set_error(
            tx,
            insight,
            &crate::i18n::store_message(&crate::i18n::Message::new(
                "repository.identity.insightProjectionConflict",
            )),
        )
        .await?;
    }
    Ok(())
}

async fn set_error(
    tx: &impl ConnectionTrait,
    insight: &code_insights::Model,
    error: &str,
) -> Result<()> {
    let mut config = insights::decode(insight)?;
    if config.identity_error == error {
        return Ok(());
    }
    config.identity_error = error.into();
    code_insights::Entity::update_many()
        .col_expr(code_insights::Column::Config, Expr::value(encode(&config)?))
        .filter(code_insights::Column::Id.eq(insight.id))
        .filter(code_insights::Column::WorkspaceId.eq(insight.workspace_id))
        .exec(tx)
        .await?;
    Ok(())
}

/// Refresh each affected insight once after a repository's refs are published.
pub(super) async fn refresh_repository(
    s: &AppState,
    tx: &impl ConnectionTrait,
    source: &sources::Model,
) -> Result<()> {
    let parents = selections::Entity::find()
        .filter(selections::Column::WorkspaceId.eq(source.workspace_id))
        .filter(selections::Column::ConnectionId.eq(source.connection_id))
        .all(tx)
        .await?
        .into_iter()
        .filter(|row| {
            row.all_repositories != 0
                || super::repository_matches(&row.repository, &source.remote_path)
        })
        .map(|row| row.insight_id)
        .collect::<BTreeSet<_>>();
    for insight in code_insights::Entity::find()
        .filter(code_insights::Column::WorkspaceId.eq(source.workspace_id))
        .filter(code_insights::Column::Id.is_in(parents))
        .all(tx)
        .await?
    {
        let scope = match Scope::for_actor_in(
            tx,
            insight.workspace_id.into(),
            insight.user_id.into(),
        )
        .await
        {
            Ok(scope) if scope.require("repositories.read").is_ok() => scope,
            Ok(_) => continue,
            Err(error) if error.0.is_client_error() => continue,
            Err(error) => return Err(error),
        };
        let selected = insights::selected_sources(tx, &scope, &insight).await?;
        ensure_mode(s, tx, &insight, &selected, false).await?;
    }
    Ok(())
}

/// Startup projects existing local facts once, never from a report request.
pub(super) async fn recover(s: &AppState) -> Result<()> {
    let mut cursor = 0;
    loop {
        let batch = code_insights::Entity::find()
            .filter(code_insights::Column::Id.gt(cursor))
            .order_by_asc(code_insights::Column::Id)
            .limit(BATCH)
            .all(&s.db)
            .await?;
        if batch.is_empty() {
            break;
        }
        cursor = batch.last().unwrap().id;
        for insight in batch {
            let tx = crate::variables::transaction(s, insight.workspace_id.into()).await?;
            let scope =
                match Scope::for_actor_in(&tx, insight.workspace_id.into(), insight.user_id.into())
                    .await
                {
                    Ok(scope) if scope.require("repositories.read").is_ok() => scope,
                    Ok(_) => continue,
                    Err(error) if error.0.is_client_error() => continue,
                    Err(error) => return Err(error),
                };
            let selected = insights::selected_sources(&tx, &scope, &insight).await?;
            ensure_mode(s, &tx, &insight, &selected, false).await?;
            tx.commit().await?;
        }
    }
    Ok(())
}

pub(super) async fn rebuild(
    s: &AppState,
    tx: &impl ConnectionTrait,
    insight: &code_insights::Model,
) -> Result<()> {
    let scope =
        Scope::for_actor_in(tx, insight.workspace_id.into(), insight.user_id.into()).await?;
    let selected = insights::selected_sources(tx, &scope, insight).await?;
    links::Entity::delete_many()
        .filter(links::Column::WorkspaceId.eq(insight.workspace_id))
        .filter(links::Column::InsightId.eq(insight.id))
        .exec(tx)
        .await?;
    ensure(s, tx, insight, &selected).await?;
    set_error(tx, insight, "").await
}
