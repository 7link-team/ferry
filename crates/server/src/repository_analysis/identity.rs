//! Owner-scoped contributor resolution over immutable Git author facts.
use super::*;
use crate::entities::{
    repository_analysis_commit_authors as authors,
    repository_analysis_commit_identities as raw_authors,
    repository_analysis_identities as identities, repository_analysis_people as people,
};
use sea_orm::sea_query::{Alias as SqlAlias, OnConflict, Query as SqlQuery};

const BATCH: u64 = 500;
#[cfg(test)]
const CONFLICT: &str = "作者映射冲突：同一 Git 身份匹配多个贡献者，请检查此仓库各连接的作者映射。";
pub(super) const CONFLICT_KEY: &str = "repository.identity.conflict";

pub(super) fn is_mapping_conflict(value: &str) -> bool {
    crate::i18n::decode_stored(value)
        .map(|message| message.key == CONFLICT_KEY)
        .unwrap_or_else(|| value.starts_with("作者映射冲突："))
}

fn conflict_error() -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::CONFLICT,
        crate::i18n::Message::new(CONFLICT_KEY),
    )
}

pub(super) fn normalized(value: &str) -> String {
    value.trim().to_lowercase()
}
pub(super) fn same_rule(a: &Alias, b: &Alias) -> bool {
    normalized(&a.name) == normalized(&b.name) && normalized(&a.email) == normalized(&b.email)
}
fn fingerprint(name: &str, email: &str) -> Result<String> {
    Ok(hex::encode(Sha256::digest(
        encode(&(name, email))?.as_bytes(),
    )))
}

async fn legacy_person(
    s: &AppState,
    tx: &impl ConnectionTrait,
    workspace: i64,
    user: i64,
    nickname: &str,
) -> Result<people::Model> {
    let rows = people::Entity::find()
        .filter(people::Column::WorkspaceId.eq(workspace))
        .filter(people::Column::UserId.eq(user))
        .filter(people::Column::RawIdentityId.is_null())
        .filter(people::Column::DisplayName.eq(nickname))
        .all(tx)
        .await?;
    if rows.len() > 1 {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("repository.analysis.nicknameAmbiguous"),
        ));
    }
    if let Some(row) = rows.into_iter().next() {
        return Ok(row);
    }
    Ok(people::Entity::insert(
        people::Model {
            id: s.next_id()?.into(),
            workspace_id: workspace,
            user_id: user,
            display_name: nickname.into(),
            version: 1,
            raw_identity_id: None,
        }
        .into_active_model(),
    )
    .exec_with_returning(tx)
    .await?)
}

async fn alias_person(
    s: &AppState,
    tx: &impl ConnectionTrait,
    workspace: i64,
    user: i64,
    alias: &Alias,
) -> Result<people::Model> {
    if alias.contributor_id.is_empty() {
        return legacy_person(s, tx, workspace, user, alias.nickname.trim()).await;
    }
    let id: Id = alias.contributor_id.parse().map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.analysis.contributorInvalid"),
        )
    })?;
    people::Entity::find_by_id(i64::from(id))
        .filter(people::Column::WorkspaceId.eq(workspace))
        .filter(people::Column::UserId.eq(user))
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)
}

/// Enrich aliases before the caller persists config and rebuilds in the same transaction.
pub(super) async fn prepare_aliases(
    s: &AppState,
    tx: &impl ConnectionTrait,
    workspace: i64,
    user: i64,
    old: &[Alias],
    new: &mut [Alias],
) -> Result<()> {
    let mut labels: BTreeMap<i64, String> = BTreeMap::new();
    for alias in new.iter_mut() {
        alias.nickname = alias.nickname.trim().into();
        if (normalized(&alias.name).is_empty() && normalized(&alias.email).is_empty())
            || alias.nickname.is_empty()
            || alias.nickname.chars().count() > 255
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.analysis.aliasInvalid"),
            ));
        }
        let prior: Vec<&Alias> = old.iter().filter(|v| same_rule(v, alias)).collect();
        let mut ids = BTreeMap::new();
        if alias.contributor_id.is_empty() {
            for previous in &prior {
                let person = alias_person(s, tx, workspace, user, previous).await?;
                ids.insert(person.id, person);
            }
        }
        if ids.len() > 1 {
            return Err(conflict_error());
        }
        let mut person = if let Some((_, person)) = ids.into_iter().next() {
            person
        } else {
            alias_person(s, tx, workspace, user, alias).await?
        };
        // An unchanged alias label is a reference, not a request to undo a rename
        // made through another connection pointing at the same contributor.
        let rename = prior.is_empty() || prior.iter().any(|v| v.nickname.trim() != alias.nickname);
        if rename && person.display_name != alias.nickname {
            if person.raw_identity_id.is_some() {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.analysis.rawAuthorImmutable"),
                ));
            }
            person.display_name = alias.nickname.clone();
            person.version += 1;
            people::Entity::update(person.clone().into_active_model().reset_all())
                .exec(tx)
                .await?;
        }
        alias.nickname = person.display_name.clone();
        alias.contributor_id = person.id.to_string();
        if labels
            .insert(person.id, alias.nickname.clone())
            .is_some_and(|v| v != alias.nickname)
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("repository.analysis.nicknameConflict"),
            ));
        }
    }
    let rules = new
        .iter()
        .map(|a| Rule {
            name: normalized(&a.name),
            email: normalized(&a.email),
            person: a.contributor_id.parse::<i64>().unwrap(),
        })
        .collect::<Vec<_>>();
    validate_rules(&rules)
}

/// Present canonical person labels without altering collection config or fences.
pub(super) async fn hydrate_aliases(
    tx: &impl ConnectionTrait,
    workspace: i64,
    user: i64,
    aliases: &mut [Alias],
) -> Result<()> {
    let ids = aliases
        .iter()
        .filter(|a| !a.contributor_id.is_empty())
        .map(|a| {
            a.contributor_id.parse::<Id>().map(i64::from).map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.analysis.contributorInvalid"),
                )
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let people = people::Entity::find()
        .filter(people::Column::WorkspaceId.eq(workspace))
        .filter(people::Column::UserId.eq(user))
        .filter(people::Column::Id.is_in(ids))
        .all(tx)
        .await?
        .into_iter()
        .map(|v| (v.id, v.display_name))
        .collect::<HashMap<_, _>>();
    for alias in aliases {
        if alias.contributor_id.is_empty() {
            continue;
        }
        let id = alias.contributor_id.parse::<i64>().map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.analysis.contributorInvalid"),
            )
        })?;
        alias.nickname = people.get(&id).cloned().ok_or_else(ApiError::missing)?;
    }
    Ok(())
}

#[derive(Clone)]
pub(super) struct Rule {
    pub(super) name: String,
    pub(super) email: String,
    pub(super) person: i64,
}
impl Rule {
    pub(super) fn matches(&self, name: &str, email: &str) -> bool {
        (!self.name.is_empty() && self.name == name)
            || (!self.email.is_empty() && self.email == email)
    }
}
pub(super) fn validate_rules(rules: &[Rule]) -> Result<()> {
    let mut names = BTreeMap::new();
    let mut emails = BTreeMap::new();
    for rule in rules {
        for (key, map) in [(&rule.name, &mut names), (&rule.email, &mut emails)] {
            if !key.is_empty()
                && map
                    .insert(key, rule.person)
                    .is_some_and(|id| id != rule.person)
            {
                return Err(conflict_error());
            }
        }
    }
    Ok(())
}

struct Context {
    rules: BTreeMap<i64, Vec<Rule>>,
    sources: Vec<sources::Model>,
}
async fn context(
    s: &AppState,
    tx: &impl ConnectionTrait,
    workspace: i64,
    user: i64,
    raw_only: bool,
    selected: &[i64],
) -> Result<Context> {
    let connections = connections::Entity::find()
        .filter(connections::Column::WorkspaceId.eq(workspace))
        .filter(connections::Column::UserId.eq(user))
        .filter(connections::Column::State.eq("connected"))
        .all(tx)
        .await?
        .into_iter()
        .map(|v| v.id)
        .collect::<Vec<_>>();
    let mut select = sources::Entity::find()
        .filter(sources::Column::WorkspaceId.eq(workspace))
        .filter(sources::Column::ConnectionId.is_in(connections.clone()))
        .filter(sources::Column::State.ne("unavailable"));
    if !selected.is_empty() {
        select = select.filter(sources::Column::RepositoryId.is_in(selected.iter().copied()));
    }
    let mut source_rows = select.all(tx).await?;
    let settings = configs::Entity::find()
        .filter(configs::Column::Id.is_in(source_rows.iter().map(|v| v.connection_id)))
        .all(tx)
        .await?
        .into_iter()
        .map(|v| Ok((v.id, decode(&v)?)))
        .collect::<Result<HashMap<_, _>>>()?;
    source_rows.retain(|row| {
        settings
            .get(&row.connection_id)
            .is_some_and(|v| v.all_repositories || v.repositories.contains(&row.remote_path))
    });
    let mut resolved = HashMap::new();
    let mut rules: BTreeMap<i64, Vec<Rule>> = BTreeMap::new();
    for source in &source_rows {
        let target = rules.entry(source.repository_id).or_default();
        if raw_only {
            continue;
        }
        for alias in &settings[&source.connection_id].aliases {
            let key = encode(&(alias.contributor_id.as_str(), alias.nickname.as_str()))?;
            let person = if let Some(id) = resolved.get(&key) {
                *id
            } else {
                let id = alias_person(s, tx, workspace, user, alias).await?.id;
                resolved.insert(key, id);
                id
            };
            target.push(Rule {
                name: normalized(&alias.name),
                email: normalized(&alias.email),
                person,
            });
        }
    }
    Ok(Context {
        rules,
        sources: source_rows,
    })
}

pub(super) async fn raw_identities(
    s: &AppState,
    tx: &impl ConnectionTrait,
    workspace: i64,
    facts: &[commits::Model],
) -> Result<HashMap<String, identities::Model>> {
    let mut wanted = BTreeMap::new();
    for fact in facts {
        wanted.insert(fingerprint(&fact.author_name, &fact.author_email)?, fact);
    }
    let mut known = identities::Entity::find()
        .filter(identities::Column::WorkspaceId.eq(workspace))
        .filter(identities::Column::Fingerprint.is_in(wanted.keys().cloned()))
        .all(tx)
        .await?
        .into_iter()
        .map(|v| (v.fingerprint.clone(), v))
        .collect::<HashMap<_, _>>();
    let mut fresh = Vec::new();
    for (fingerprint, fact) in wanted {
        if known.contains_key(&fingerprint) {
            continue;
        }
        let row = identities::Model {
            id: s.next_id()?.into(),
            workspace_id: workspace,
            fingerprint: fingerprint.clone(),
            raw_name: fact.author_name.clone(),
            raw_email: fact.author_email.clone(),
        };
        fresh.push(row.clone().into_active_model());
        known.insert(fingerprint, row);
    }
    if !fresh.is_empty() {
        identities::Entity::insert_many(fresh).exec(tx).await?;
    }
    if !facts.is_empty() {
        let rows = facts
            .iter()
            .map(|fact| -> Result<_> {
                Ok(raw_authors::Model {
                    commit_id: fact.id,
                    workspace_id: workspace,
                    identity_id: known[&fingerprint(&fact.author_name, &fact.author_email)?].id,
                }
                .into_active_model())
            })
            .collect::<Result<Vec<_>>>()?;
        // Raw linkage is immutable; rebuilding a display projection never changes it.
        raw_authors::Entity::insert_many(rows)
            .on_conflict(
                OnConflict::column(raw_authors::Column::CommitId)
                    .do_nothing_on([raw_authors::Column::CommitId])
                    .to_owned(),
            )
            .exec_without_returning(tx)
            .await?;
    }
    Ok(known)
}

async fn project(
    s: &AppState,
    tx: &impl ConnectionTrait,
    workspace: i64,
    user: i64,
    context: &Context,
    facts: &[commits::Model],
    strict: bool,
    raw_only: bool,
) -> Result<BTreeSet<i64>> {
    let identities = raw_identities(s, tx, workspace, facts).await?;
    let mut defaults = people::Entity::find()
        .filter(people::Column::WorkspaceId.eq(workspace))
        .filter(people::Column::UserId.eq(user))
        .filter(people::Column::RawIdentityId.is_in(identities.values().map(|v| v.id)))
        .all(tx)
        .await?
        .into_iter()
        .map(|v| (v.raw_identity_id.unwrap(), v.id))
        .collect::<HashMap<_, _>>();
    let mut fresh = Vec::new();
    for identity in identities.values() {
        if defaults.contains_key(&identity.id) {
            continue;
        }
        let row = people::Model {
            id: s.next_id()?.into(),
            workspace_id: workspace,
            user_id: user,
            display_name: identity.raw_name.clone(),
            version: 1,
            raw_identity_id: Some(identity.id),
        };
        defaults.insert(identity.id, row.id);
        fresh.push(row.into_active_model());
    }
    if !fresh.is_empty() {
        people::Entity::insert_many(fresh).exec(tx).await?;
    }
    let mut rows = Vec::new();
    let mut conflicts = BTreeSet::new();
    for fact in facts {
        let Some(rules) = context.rules.get(&fact.repository_id) else {
            continue;
        };
        let identity = &identities[&fingerprint(&fact.author_name, &fact.author_email)?];
        let name = normalized(&fact.author_name);
        let email = normalized(&fact.author_email);
        let matches = if raw_only {
            BTreeSet::new()
        } else {
            rules
                .iter()
                .filter(|r| r.matches(&name, &email))
                .map(|r| r.person)
                .collect::<BTreeSet<_>>()
        };
        if matches.len() > 1 {
            if strict {
                return Err(conflict_error());
            }
            conflicts.insert(fact.repository_id);
        }
        let person = if matches.len() == 1 {
            *matches.first().unwrap()
        } else {
            defaults[&identity.id]
        };
        rows.push(
            authors::Model {
                commit_id: fact.id,
                user_id: user,
                workspace_id: workspace,
                repository_id: fact.repository_id,
                identity_id: identity.id,
                contributor_id: person,
            }
            .into_active_model(),
        );
    }
    if !rows.is_empty() {
        authors::Entity::insert_many(rows)
            .on_conflict(
                OnConflict::columns([authors::Column::CommitId, authors::Column::UserId])
                    .update_columns([
                        authors::Column::WorkspaceId,
                        authors::Column::RepositoryId,
                        authors::Column::IdentityId,
                        authors::Column::ContributorId,
                    ])
                    .to_owned(),
            )
            .exec_without_returning(tx)
            .await?;
    }
    Ok(conflicts)
}

async fn mapping_errors(
    tx: &impl ConnectionTrait,
    context: &Context,
    repositories: &BTreeSet<i64>,
    conflict: bool,
) -> Result<()> {
    for source in &context.sources {
        if !repositories.contains(&source.repository_id) {
            continue;
        }
        if conflict {
            sources::Entity::update_many()
                .col_expr(sources::Column::State, Expr::value("failed"))
                .col_expr(
                    sources::Column::LastError,
                    Expr::value(conflict_error().stored()),
                )
                .filter(sources::Column::Id.eq(source.id))
                .filter(sources::Column::State.ne("unavailable"))
                .exec(tx)
                .await?;
        } else if is_mapping_conflict(&source.last_error) {
            sources::Entity::update_many()
                .col_expr(
                    sources::Column::State,
                    Expr::value(if source.synced_at > 0 {
                        "complete"
                    } else {
                        "pending"
                    }),
                )
                .col_expr(sources::Column::LastError, Expr::value(""))
                .filter(sources::Column::Id.eq(source.id))
                .filter(sources::Column::State.ne("unavailable"))
                .exec(tx)
                .await?;
        }
    }
    Ok(())
}

async fn rebuild_owner(
    s: &AppState,
    tx: &impl ConnectionTrait,
    workspace: i64,
    user: i64,
    selected: &[i64],
    strict: bool,
    raw_only: bool,
) -> Result<()> {
    let context = context(s, tx, workspace, user, raw_only, selected).await?;
    let mut repositories = context
        .rules
        .keys()
        .copied()
        .filter(|id| selected.is_empty() || selected.contains(id))
        .collect::<BTreeSet<_>>();
    if strict && !raw_only {
        for id in &repositories {
            validate_rules(&context.rules[id])?;
        }
    }
    let mut stale = authors::Entity::delete_many()
        .filter(authors::Column::WorkspaceId.eq(workspace))
        .filter(authors::Column::UserId.eq(user));
    if !selected.is_empty() {
        stale = stale.filter(authors::Column::RepositoryId.is_in(selected.iter().copied()));
    }
    if !repositories.is_empty() {
        stale = stale.filter(authors::Column::RepositoryId.is_not_in(repositories.iter().copied()));
    }
    stale.exec(tx).await?;
    let mut cursor = 0;
    let mut conflicts = if raw_only {
        BTreeSet::new()
    } else {
        repositories
            .iter()
            .filter(|id| validate_rules(&context.rules[id]).is_err())
            .copied()
            .collect()
    };
    loop {
        let facts = commits::Entity::find()
            .filter(commits::Column::WorkspaceId.eq(workspace))
            .filter(commits::Column::RepositoryId.is_in(repositories.iter().copied()))
            .filter(commits::Column::Id.gt(cursor))
            .order_by_asc(commits::Column::Id)
            .limit(BATCH)
            .all(tx)
            .await?;
        if facts.is_empty() {
            break;
        }
        cursor = facts.last().unwrap().id;
        conflicts
            .extend(project(s, tx, workspace, user, &context, &facts, strict, raw_only).await?);
    }
    mapping_errors(tx, &context, &conflicts, true).await?;
    repositories.retain(|id| !conflicts.contains(id));
    if !raw_only {
        mapping_errors(tx, &context, &repositories, false).await?;
    }
    Ok(())
}

/// Detect local history omitted while its owner was disconnected during recovery.
pub(super) async fn needs_projection(
    tx: &impl ConnectionTrait,
    workspace: i64,
    user: i64,
    repository: i64,
) -> Result<bool> {
    let projection = SqlQuery::select()
        .expr(Expr::val(1))
        .from(authors::Entity)
        .and_where(
            Expr::col(authors::Column::CommitId)
                .eq(Expr::col((SqlAlias::new("c"), commits::Column::Id))),
        )
        .and_where(authors::Column::UserId.eq(user))
        .to_owned();
    let missing = SqlQuery::select()
        .expr(Expr::val(1))
        .from_as(commits::Entity, SqlAlias::new("c"))
        .and_where(Expr::col((SqlAlias::new("c"), commits::Column::WorkspaceId)).eq(workspace))
        .and_where(Expr::col((SqlAlias::new("c"), commits::Column::RepositoryId)).eq(repository))
        .and_where(Expr::exists(projection).not())
        .limit(1)
        .to_owned();
    Ok(tx
        .query_one_raw(tx.get_database_backend().build(&missing))
        .await?
        .is_some())
}

/// Rebuild selected repositories (empty means all) inside the config transaction.
pub(super) async fn rebuild(
    s: &AppState,
    tx: &impl ConnectionTrait,
    workspace: i64,
    user: i64,
    repositories: &[i64],
) -> Result<()> {
    rebuild_owner(s, tx, workspace, user, repositories, true, false).await
}

/// Refresh local identities after connection visibility changes without making
/// an existing mapping conflict prevent authorization or deletion.
pub(super) async fn reconcile_scope(
    s: &AppState,
    tx: &impl ConnectionTrait,
    workspace: i64,
    user: i64,
    repositories: &[i64],
) -> Result<()> {
    match rebuild_owner(s, tx, workspace, user, repositories, false, false).await {
        Ok(()) => Ok(()),
        Err(error) if error.0.is_client_error() => {
            rebuild_owner(s, tx, workspace, user, repositories, false, true).await?;
            let context = context(s, tx, workspace, user, true, repositories).await?;
            let affected = context.rules.keys().copied().collect();
            mapping_errors(tx, &context, &affected, true).await
        }
        Err(error) => Err(error),
    }
}

/// Called in the fact-store transaction. Each owner's mapping remains independent.
pub(super) async fn index_commit(
    s: &AppState,
    tx: &impl ConnectionTrait,
    fact: &commits::Model,
) -> Result<()> {
    raw_identities(s, tx, fact.workspace_id, std::slice::from_ref(fact)).await?;
    let visible_connections = sources::Entity::find()
        .filter(sources::Column::WorkspaceId.eq(fact.workspace_id))
        .filter(sources::Column::RepositoryId.eq(fact.repository_id))
        .filter(sources::Column::State.ne("unavailable"))
        .all(tx)
        .await?
        .into_iter()
        .map(|v| v.connection_id)
        .collect::<Vec<_>>();
    let owners = connections::Entity::find()
        .filter(connections::Column::WorkspaceId.eq(fact.workspace_id))
        .filter(connections::Column::Id.is_in(visible_connections))
        .filter(connections::Column::State.eq("connected"))
        .all(tx)
        .await?
        .into_iter()
        .map(|v| v.user_id)
        .collect::<BTreeSet<_>>();
    for user in owners {
        let context = context(s, tx, fact.workspace_id, user, false, &[fact.repository_id]).await?;
        if !context.rules.contains_key(&fact.repository_id) {
            continue;
        }
        let conflicts = project(
            s,
            tx,
            fact.workspace_id,
            user,
            &context,
            std::slice::from_ref(fact),
            true,
            false,
        )
        .await?;
        mapping_errors(tx, &context, &conflicts, true).await?;
    }
    Ok(())
}

/// Local, bounded keyset backfill; each owner's aliases and projection commit atomically.
#[cfg(test)]
pub(crate) async fn backfill(s: &AppState) -> Result<()> {
    backfill_mode(s, true).await
}

/// Startup recovery is a one-time local migration, not a history scan on every boot.
pub(crate) async fn recover(s: &AppState) -> Result<()> {
    backfill_mode(s, false).await
}

async fn mark_recovered(tx: &impl ConnectionTrait, connections: &[i64]) -> Result<()> {
    for row in configs::Entity::find()
        .filter(configs::Column::Id.is_in(connections.iter().copied()))
        .all(tx)
        .await?
    {
        let mut config = decode(&row)?;
        config.identity_version = 1;
        configs::Entity::update_many()
            .col_expr(configs::Column::Config, Expr::value(encode(&config)?))
            .filter(configs::Column::Id.eq(row.id))
            .exec(tx)
            .await?;
    }
    Ok(())
}

async fn backfill_mode(s: &AppState, force: bool) -> Result<()> {
    let owners = connections::Entity::find()
        .all(&s.db)
        .await?
        .into_iter()
        .map(|v| (v.workspace_id, v.user_id))
        .collect::<BTreeSet<_>>();
    for (workspace, user) in owners {
        let tx = crate::variables::transaction(s, workspace.into()).await?;
        let ids = connections::Entity::find()
            .filter(connections::Column::WorkspaceId.eq(workspace))
            .filter(connections::Column::UserId.eq(user))
            .all(&tx)
            .await?
            .into_iter()
            .map(|v| v.id)
            .collect::<Vec<_>>();
        let settings = configs::Entity::find()
            .filter(configs::Column::Id.is_in(ids.iter().copied()))
            .order_by_asc(configs::Column::Id)
            .all(&tx)
            .await?;
        if !force
            && settings
                .iter()
                .map(decode)
                .collect::<Result<Vec<_>>>()?
                .iter()
                .all(|v| v.identity_version == 1)
        {
            tx.commit().await?;
            continue;
        }
        let result = async {
            for row in settings {
                let mut config = decode(&row)?;
                let old = config.aliases.clone();
                prepare_aliases(s, &tx, workspace, user, &old, &mut config.aliases).await?;
                configs::Entity::update_many()
                    .col_expr(configs::Column::Config, Expr::value(encode(&config)?))
                    .filter(configs::Column::Id.eq(row.id))
                    .exec(&tx)
                    .await?;
            }
            rebuild_owner(s, &tx, workspace, user, &[], false, false).await?;
            mark_recovered(&tx, &ids).await
        }
        .await;
        match result {
            Ok(()) => tx.commit().await?,
            Err(error) if error.0.is_client_error() => {
                tx.rollback().await?;
                let raw = crate::variables::transaction(s, workspace.into()).await?;
                rebuild_owner(s, &raw, workspace, user, &[], false, true).await?;
                let context = context(s, &raw, workspace, user, true, &[]).await?;
                let repos = context.rules.keys().copied().collect();
                mapping_errors(&raw, &context, &repos, true).await?;
                mark_recovered(&raw, &ids).await?;
                raw.commit().await?;
                tracing::warn!(workspace,user,error=%error.1,"作者映射回填使用原始身份，请修正冲突配置");
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
