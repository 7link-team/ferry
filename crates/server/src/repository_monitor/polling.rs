//! Complete provider snapshots, durable baselines and cluster-safe polling.
use super::*;
use crate::{entities::locks, lock, repository_auth::MonitorAccess};
use futures::{StreamExt, stream};
use std::collections::BTreeMap;
use tokio::sync::watch;

const LEASE_MS: i64 = 60_000;
type Snapshot = BTreeMap<String, Repository>;

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::IntoActiveModel;

    /// An isolated database-backed state with workspace 1.
    async fn test_state() -> (tempfile::TempDir, AppState) {
        let (dir, db) = crate::lock::tests::database().await;
        let s = std::sync::Arc::new(crate::state::State {
            ids: std::sync::Arc::new(
                crate::snowflake::Generator::new(1, tokio::time::Instant::now().into_std())
                    .unwrap(),
            ),
            db,
            data_dir: dir.path().into(),
            instance_id: "poll-test".into(),
            server_mode: false,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), false).unwrap(),
            host_monitor: Default::default(),
            cluster: None,
        });
        crate::entities::workspaces::Entity::insert(
            crate::entities::workspaces::Model {
                id: 1,
                slug: "poll-test".into(),
                name: "Poll".into(),
                owner_id: 1,
                join_code: "poll-test".into(),
                pipeline_run_number: 0,
                created_at: 0,
                updated_at: 0,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
        (dir, s)
    }

    #[tokio::test]
    async fn retention_removes_only_events_older_than_the_configured_days() {
        let (_dir, s) = test_state().await;
        let day = 86_400_000;
        for (id, age) in [(10, 8 * day), (11, day)] {
            events::Entity::insert(events::ActiveModel {
                id: Set(id),
                workspace_id: Set(1),
                connection_id: Set(2),
                delivery_key: Set(format!("key-{id}")),
                event: Set("push".into()),
                repository: Set("team/repo".into()),
                reference: Set("main".into()),
                actor: Set(String::new()),
                action: Set(String::new()),
                delivery: Set(String::new()),
                created_at: Set(db::now() - age),
                detail: Set(None),
            })
            .exec_without_returning(&s.db)
            .await
            .unwrap();
        }
        let remaining = || async {
            events::Entity::find()
                .all(&s.db)
                .await
                .unwrap()
                .into_iter()
                .map(|e| e.id)
                .collect::<Vec<_>>()
        };
        // Without a positive retention nothing is removed.
        prune(&s).await.unwrap();
        assert_eq!(remaining().await, vec![10, 11]);
        crate::entities::settings::Entity::insert(crate::entities::settings::ActiveModel {
            id: Set(20),
            workspace_id: Set(1),
            key: Set("repository_event_retention_days".into()),
            value: Set(7),
            version: Set(1),
            created_at: Set(0),
            updated_at: Set(0),
        })
        .exec_without_returning(&s.db)
        .await
        .unwrap();
        prune(&s).await.unwrap();
        assert_eq!(remaining().await, vec![11]);
    }

    #[tokio::test]
    async fn shared_database_claim_and_fences_exclude_other_replicas_and_stale_results() {
        let (_dir, s) = test_state().await;
        connections::Entity::insert(
            connections::Model {
                id: 2,
                workspace_id: 1,
                user_id: 1,
                provider: "codeup".into(),
                name: "test".into(),
                description: String::new(),
                credential_id: 3,
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
        let config = Config {
            enabled: true,
            mode: Mode::Poll,
            poll_interval_minutes: 1,
            provider: "codeup".into(),
            ..Default::default()
        };
        monitors::Entity::insert(
            monitors::Model {
                id: 2,
                workspace_id: 1,
                config: s
                    .vault
                    .seal(&serde_json::to_string(&config).unwrap(), &aad(1, 2))
                    .unwrap(),
                version: 1,
                created_at: 0,
                updated_at: 0,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
        let row = polls::Model {
            id: 2,
            workspace_id: 1,
            config_version: 1,
            snapshot: String::new(),
            last_polled_at: 0,
            next_poll_at: 0,
            last_error: String::new(),
        };
        polls::Entity::insert(row.clone().into_active_model())
            .exec(&s.db)
            .await
            .unwrap();
        let (first, second) = tokio::join!(claim(&s, row.clone()), claim(&s, row.clone()));
        let claimed = [first.unwrap(), second.unwrap()];
        assert_eq!(claimed.iter().filter(|v| v.is_some()).count(), 1);
        let winner = claimed.into_iter().flatten().next().unwrap();
        current(&s.db, &winner).await.unwrap();
        // Even if another scheduler has stale due rows, an active lease wins.
        polls::Entity::update_many()
            .col_expr(polls::Column::NextPollAt, Expr::value(0))
            .exec(&s.db)
            .await
            .unwrap();
        assert!(claim(&s, row.clone()).await.unwrap().is_none());
        monitors::Entity::update_many()
            .col_expr(monitors::Column::Version, Expr::value(2))
            .exec(&s.db)
            .await
            .unwrap();
        assert!(current(&s.db, &winner).await.is_err());
        monitors::Entity::update_many()
            .col_expr(monitors::Column::Version, Expr::value(1))
            .exec(&s.db)
            .await
            .unwrap();
        locks::Entity::update_many()
            .col_expr(locks::Column::Owner, Expr::value("other-replica"))
            .exec(&s.db)
            .await
            .unwrap();
        assert!(current(&s.db, &winner).await.is_err());
        assert_eq!(
            lock::release(&s.db, &winner.name, &winner.owner)
                .await
                .unwrap(),
            0
        );
        lock::release(&s.db, &winner.name, "other-replica")
            .await
            .unwrap();
        polls::Entity::update_many()
            .col_expr(polls::Column::ConfigVersion, Expr::value(0))
            .exec(&s.db)
            .await
            .unwrap();
        assert!(claim(&s, row).await.unwrap().is_none());
    }

    #[test]
    fn legacy_configuration_remains_push_with_one_minute_default() {
        let config: Config = serde_json::from_value(json!({"enabled":false,"provider":"codeup","all_repositories":true,"repositories":[],"events":["push"],"secret":"","webhook":""})).unwrap();
        assert!(config.mode == Mode::Push);
        assert_eq!(config.poll_interval_minutes, 1);
        assert!(supported_events("codeup", Mode::Poll).contains(&"repository.created"));
        assert!(!supported_events("codeup", Mode::Push).contains(&"repository.created"));
    }

    #[test]
    fn incomplete_or_duplicate_refs_cannot_replace_a_complete_baseline() {
        assert!(references(vec![json!({"name":"main"})], true).is_err());
        assert!(
            references(
                vec![
                    json!({"name":"main","commit":{"sha":"a"}}),
                    json!({"name":"main","commit":{"sha":"b"}})
                ],
                true
            )
            .is_err()
        );
        for field in ["sha", "id"] {
            assert_eq!(
                references(vec![json!({"name":"main","commit":{field:"a"}})], true).unwrap()["main"],
                "a"
            );
        }
        assert!(requests(vec![json!({"localId":1})]).is_err());
    }

    #[test]
    fn diff_detects_repeated_head_changes_and_merges_without_inventing_actors() {
        let config = Config {
            events: KINDS.iter().map(|e| e.to_string()).collect(),
            ..Default::default()
        };
        let state = |head: &str, merged: bool| {
            BTreeMap::from([("team/repo".into(),Repository {
            branches: BTreeMap::from([("main".into(),head.into())]),
            requests: requests(vec![json!({"localId":1,"state":if merged {"MERGED"} else {"UNDER_REVIEW"},"sourceBranch":"feature","targetBranch":"main"})]).unwrap(),
            ..Default::default()
        })])
        };
        let a = state("a", false);
        let b = state("b", false);
        let merged = state("b", true);
        assert!(changes(&a, &a, &config).is_empty());
        for (before, after) in [(&a, &b), (&b, &a), (&a, &b)] {
            let events = changes(before, after, &config);
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].event, "push");
            assert!(events[0].actor.is_empty());
        }
        let events = changes(&b, &merged, &config);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "merge");
        assert!(changes(&merged, &merged, &config).is_empty());
        assert_eq!(
            changes(&Snapshot::new(), &a, &config).len(),
            1,
            "new repo contents establish their own baseline"
        );
        assert_eq!(
            changes(&a, &Snapshot::new(), &config)[0].event,
            "repository.deleted"
        );
    }

    fn all_events() -> Config {
        Config {
            events: KINDS.iter().map(|e| e.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn diff_names_known_operators_marks_unknown_ones_and_keeps_detail_out_of_the_baseline() {
        let before = BTreeMap::from([(
            "team/repo".to_owned(),
            Repository {
                branches: BTreeMap::from([("main".into(), "a".into()), ("old".into(), "x".into())]),
                tags: BTreeMap::from([("v1".into(), "t1".into())]),
                requests: requests(vec![json!({"iid":1,"state":"opened","title":"Add login","description":"Secret plan","author":{"username":"ann"},"source_branch":"f","target_branch":"main"})]).unwrap(),
                ..Default::default()
            },
        )]);
        let after = BTreeMap::from([(
            "team/repo".to_owned(),
            Repository {
                branches: BTreeMap::from([("main".into(), "a".into()), ("feature".into(), "c".into())]),
                tags: BTreeMap::from([("v2".into(), "t2".into())]),
                requests: requests(vec![json!({"iid":1,"state":"merged","title":"Add login","description":"Secret plan","author":{"username":"ann"},"merged_by":{"username":"lead"},"source_branch":"f","target_branch":"main","web_url":"https://x/mr/1"})]).unwrap(),
                branch_detail: BTreeMap::from([("feature".into(), json!({"commit":{"id":"c","message":"init feature","author":"Bob","author_username":"bob"}}))]),
                tag_detail: BTreeMap::from([("v2".into(), json!({"target":"t2","message":"release notes","tagger":"tom"}))]),
                ..Default::default()
            },
        )]);
        let events = changes(&before, &after, &all_events());
        let find = |kind: &str| events.iter().find(|e| e.event == kind).unwrap();
        let created = find("branch.created");
        assert_eq!(created.actor, "bob");
        assert_eq!(created.detail["commit"]["message"], "init feature");
        assert_eq!(created.detail["source"], "polling");
        assert!(created.detail["note"].as_str().unwrap().contains("推送人"));
        let deleted = find("branch.deleted");
        assert!(deleted.actor.is_empty());
        assert_eq!(deleted.detail["before"], "x");
        assert!(
            deleted.detail["note"]
                .as_str()
                .unwrap()
                .contains("事件推送")
        );
        let tag = find("tag.created");
        assert_eq!(
            (tag.actor.as_str(), tag.detail["message"].as_str()),
            ("tom", Some("release notes"))
        );
        assert!(find("tag.deleted").detail["note"].is_string());
        let merge = find("merge");
        assert_eq!(merge.actor, "lead");
        assert_eq!(merge.detail["request"]["description"], "Secret plan");
        assert_eq!(merge.detail["request"]["url"], "https://x/mr/1");
        let baseline = serde_json::to_string(&after).unwrap();
        for private in ["Secret plan", "init feature", "release notes", "lead"] {
            assert!(
                !baseline.contains(private),
                "{private} must not be persisted in the baseline"
            );
        }
    }

    fn push(before: &str) -> Event {
        Event {
            event: "push".into(),
            repository: "team/repo".into(),
            reference: "main".into(),
            actor: String::new(),
            action: String::new(),
            detail: json!({"source":"polling","before":before,"after":"d"}),
        }
    }

    #[test]
    fn push_commits_stop_at_the_previous_revision_and_name_the_head_author() {
        let commits = vec![
            json!({"id":"d","message":"third","author":"Cy","author_username":"cy"}),
            json!({"id":"c","message":"second","author":"Bo"}),
            json!({"id":"b","message":"base"}),
        ];
        let mut event = push("b");
        apply_commits(&mut event, commits.clone());
        assert_eq!(event.detail["total_commits"], 2);
        assert_eq!(event.detail["commits"][0]["id"], "d");
        assert_eq!(event.detail["commits"].as_array().unwrap().len(), 2);
        assert_eq!(event.actor, "cy");
        // Rewritten history: report the newest commits and say why.
        let mut forced = push("gone");
        apply_commits(&mut forced, commits);
        assert!(forced.detail.get("total_commits").is_none());
        assert_eq!(forced.detail["commits"].as_array().unwrap().len(), 3);
        assert!(forced.detail["note"].as_str().unwrap().contains("强制推送"));
        let mut branch = Event {
            event: "branch.created".into(),
            detail: json!({}),
            ..push("")
        };
        apply_commits(
            &mut branch,
            vec![json!({"id":"h","message":"head","author":"Dee"})],
        );
        assert_eq!(
            (
                branch.actor.as_str(),
                branch.detail["commit"]["id"].as_str()
            ),
            ("Dee", Some("h"))
        );
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Repository {
    branches: BTreeMap<String, String>,
    tags: BTreeMap<String, String>,
    requests: BTreeMap<String, Request>,
    // Listing detail of the current poll only, for event detail; never part of
    // the persisted baseline.
    #[serde(skip)]
    info: Value,
    #[serde(skip)]
    branch_detail: BTreeMap<String, Value>,
    #[serde(skip)]
    tag_detail: BTreeMap<String, Value>,
}
#[derive(Serialize, Deserialize)]
struct Request {
    fingerprint: String,
    merged: bool,
    reference: String,
    #[serde(skip)]
    detail: Value,
}

const NOTE_OPERATOR: &str =
    "轮询只能发现变化结果，平台未提供执行该操作的账号；如需操作人请改用事件推送模式。";
const NOTE_PUSH_AUTHOR: &str = "操作人为最新提交的作者；轮询无法获知实际推送人。";

fn note(detail: &mut Value, text: &str) {
    if let Value::Object(map) = detail {
        let joined = match map.get("note").and_then(Value::as_str) {
            Some(existing) if !existing.is_empty() => format!("{existing}；{text}"),
            _ => text.to_owned(),
        };
        map.insert("note".into(), Value::from(joined));
    }
}

/// Branch or tag listing detail keyed by name, for the current poll only.
fn reference_details(rows: &[Value]) -> BTreeMap<String, Value> {
    rows.iter()
        .filter_map(|row| {
            let name = text(row, &["/name", "/branchName"]);
            (!name.is_empty()).then(|| (name, detail::reference(row)))
        })
        .collect()
}

fn invalid() -> ApiError {
    ApiError::bad("代码平台返回了不完整的监控数据，保留上次基线。")
}
fn bounded(value: &str) -> String {
    value.chars().take(400).collect()
}
fn poll_aad(workspace: i64, id: i64) -> String {
    format!("repository-poll:{workspace}:{id}")
}
fn lease_name(id: i64) -> String {
    format!("repository-poll:{id}")
}

async fn resource(access: &MonitorAccess, path: &str, name: &str) -> Result<Vec<Value>> {
    let mut result = Vec::new();
    for page in 1..=10000 {
        let rows = access.resource(path, name, page).await?;
        result.extend(rows.rows);
        if !rows.has_more {
            return Ok(result);
        }
    }
    Err(ApiError::bad("代码平台分页超出支持范围，保留上次基线。"))
}
fn references(rows: Vec<Value>, require_revision: bool) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    for row in rows {
        let name = text(&row, &["/name", "/branchName"]);
        let revision = text(&row, &["/commit/sha", "/commit/id", "/commitId", "/target"]);
        if name.is_empty()
            || (require_revision && revision.is_empty())
            || result.insert(name, revision).is_some()
        {
            return Err(invalid());
        }
    }
    Ok(result)
}
fn requests(rows: Vec<Value>) -> Result<BTreeMap<String, Request>> {
    let mut result = BTreeMap::new();
    for row in rows {
        let identity = ["localId", "iid", "number", "id"]
            .iter()
            .find_map(|key| {
                row.get(key)
                    .filter(|v| v.is_string() || v.is_number())
                    .map(|v| {
                        v.as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| v.to_string())
                    })
            })
            .ok_or_else(invalid)?;
        let state = text(&row, &["/state"]);
        if state.is_empty() {
            return Err(invalid());
        }
        let merged = state.eq_ignore_ascii_case("merged")
            || row["merged"].as_bool() == Some(true)
            || row["merged_at"].as_str().is_some_and(|v| !v.is_empty());
        let reference = bounded(&format!(
            "#{identity} {} → {}",
            text(&row, &["/sourceBranch", "/source_branch", "/head/ref"]),
            text(&row, &["/targetBranch", "/target_branch", "/base/ref"])
        ));
        // The baseline keeps only a hash; readable detail lives in the event record.
        let metadata = json!({"state":state,"merged":merged,"updated":text(&row,&["/updated_at","/updatedAt"]),"head":text(&row,&["/head/sha","/sha","/mergedRevision"]),"title":row["title"],"description":row["description"],"body":row["body"],"labels":row["labels"],"reviewers":row["reviewers"],"comments":row["totalCommentCount"],"reference":reference});
        let fingerprint = hex::encode(Sha256::digest(
            serde_json::to_vec(&metadata).map_err(|_| invalid())?,
        ));
        if result
            .insert(
                identity,
                Request {
                    fingerprint,
                    merged,
                    reference,
                    detail: detail::request(&row),
                },
            )
            .is_some()
        {
            return Err(invalid());
        }
    }
    Ok(result)
}
async fn snapshot(
    s: &AppState,
    connection: &connections::Model,
    config: &Config,
) -> Result<(Snapshot, MonitorAccess)> {
    if connection.state != "connected" {
        return Err(ApiError::bad("仓库连接授权已失效，请重新授权。"));
    }
    let scope =
        Scope::for_actor(s, connection.workspace_id.into(), connection.user_id.into()).await?;
    scope.require("repositories.read")?;
    scope.require("credentials.read")?;
    let access = MonitorAccess::load(s, connection).await?;
    let branches = config
        .events
        .iter()
        .any(|e| e == "push" || e.starts_with("branch."));
    let tags = config.events.iter().any(|e| e.starts_with("tag."));
    let prs = config
        .events
        .iter()
        .any(|e| matches!(e.as_str(), "pull_request" | "merge"));
    let mut result = BTreeMap::new();
    for page in 1..=10000 {
        let repositories = access.repositories(page).await?;
        let items = repositories["items"].as_array().ok_or_else(invalid)?;
        for row in items {
            let path = row["full_name"]
                .as_str()
                .filter(|p| !p.is_empty())
                .ok_or_else(invalid)?;
            if !config.all_repositories && !config.repositories.iter().any(|p| p == path) {
                continue;
            }
            if result.contains_key(path) {
                return Err(invalid());
            }
            let (branches, branch_detail) = if branches {
                let rows = resource(&access, path, "branches").await?;
                let detail = reference_details(&rows);
                (references(rows, true)?, detail)
            } else {
                Default::default()
            };
            let (tags, tag_detail) = if tags {
                let rows = resource(&access, path, "tags").await?;
                let detail = reference_details(&rows);
                (references(rows, false)?, detail)
            } else {
                Default::default()
            };
            let repository = Repository {
                branches,
                tags,
                requests: if prs {
                    requests(resource(&access, path, "requests").await?)?
                } else {
                    BTreeMap::new()
                },
                info: detail::repository(row),
                branch_detail,
                tag_detail,
            };
            result.insert(path.into(), repository);
        }
        if repositories["has_more"].as_bool() == Some(false) {
            return Ok((result, access));
        }
    }
    Err(ApiError::bad("代码平台分页超出支持范围，保留上次基线。"))
}

fn changes(old: &Snapshot, new: &Snapshot, config: &Config) -> Vec<Event> {
    let mut events = Vec::new();
    let mut add = |kind: &str,
                   repository: &str,
                   reference: &str,
                   action: &str,
                   actor: String,
                   mut detail: Value| {
        if config
            .events
            .iter()
            .any(|e| e == kind || (kind == "merge" && e == "pull_request"))
        {
            if let Value::Object(map) = &mut detail {
                map.insert("source".into(), Value::from("polling"));
            }
            events.push(Event {
                event: kind.into(),
                repository: bounded(repository),
                reference: bounded(reference),
                actor: bounded(&actor),
                action: action.into(),
                detail,
            });
        }
    };
    for (path, current) in new {
        let Some(previous) = old.get(path) else {
            let mut detail = json!({ "repository": current.info });
            note(&mut detail, NOTE_OPERATOR);
            add(
                "repository.created",
                path,
                "",
                "轮询检测：仓库首次出现在当前凭据的可见列表中",
                String::new(),
                detail,
            );
            // Existing contents of a newly visible repository are a baseline.
            continue;
        };
        for (name, revision) in &current.branches {
            match previous.branches.get(name) {
                None => {
                    let mut detail = json!({ "after": revision });
                    let mut actor = String::new();
                    if let Some(commit) = current
                        .branch_detail
                        .get(name)
                        .and_then(|d| d.get("commit"))
                    {
                        actor = detail::commit_author(commit);
                        detail["commit"] = commit.clone();
                        if !actor.is_empty() {
                            note(&mut detail, NOTE_PUSH_AUTHOR);
                        }
                    }
                    add(
                        "branch.created",
                        path,
                        name,
                        "轮询检测：新增分支",
                        actor,
                        detail,
                    )
                }
                Some(before) if before != revision => add(
                    "push",
                    path,
                    name,
                    "轮询检测：分支提交发生变化",
                    String::new(),
                    json!({ "before": before, "after": revision }),
                ),
                _ => {}
            }
        }
        for (name, revision) in previous
            .branches
            .iter()
            .filter(|(name, _)| !current.branches.contains_key(*name))
        {
            let mut detail = json!({ "before": revision });
            note(&mut detail, NOTE_OPERATOR);
            add(
                "branch.deleted",
                path,
                name,
                "轮询检测：分支已移除",
                String::new(),
                detail,
            );
        }
        for name in current
            .tags
            .keys()
            .filter(|name| !previous.tags.contains_key(*name))
        {
            let mut detail = current
                .tag_detail
                .get(name)
                .cloned()
                .unwrap_or_else(|| json!({}));
            let tagger = detail::field(&detail, &["/tagger"], 200);
            let actor = if tagger.is_empty() {
                note(&mut detail, NOTE_OPERATOR);
                String::new()
            } else {
                tagger
            };
            add(
                "tag.created",
                path,
                name,
                "轮询检测：新增标签",
                actor,
                detail,
            );
        }
        for (name, target) in previous
            .tags
            .iter()
            .filter(|(name, _)| !current.tags.contains_key(*name))
        {
            let mut detail = json!({ "target": target });
            note(&mut detail, NOTE_OPERATOR);
            add(
                "tag.deleted",
                path,
                name,
                "轮询检测：标签已移除",
                String::new(),
                detail,
            );
        }
        for (identity, request) in &current.requests {
            let prior = previous.requests.get(identity);
            if prior.is_none_or(|p| p.fingerprint != request.fingerprint) {
                let merged = request.merged && prior.is_none_or(|p| !p.merged);
                let mut detail = json!({ "request": request.detail });
                // Polling sees the author and the merger; other editors are unknown.
                let actor = if merged {
                    detail::field(&request.detail, &["/merged_by"], 200)
                } else if prior.is_none() {
                    detail::field(&request.detail, &["/author"], 200)
                } else {
                    String::new()
                };
                if actor.is_empty() {
                    note(&mut detail, NOTE_OPERATOR);
                }
                add(
                    if merged { "merge" } else { "pull_request" },
                    path,
                    &request.reference,
                    if merged {
                        "轮询检测：合并请求已合并"
                    } else if prior.is_none() {
                        "轮询检测：新增合并请求"
                    } else {
                        "轮询检测：合并请求发生变化"
                    },
                    actor,
                    detail,
                );
            }
        }
    }
    for path in old.keys().filter(|path| !new.contains_key(*path)) {
        let mut detail = json!({});
        note(&mut detail, NOTE_OPERATOR);
        add(
            "repository.deleted",
            path,
            "",
            "轮询检测：仓库已不可见，可能已删除或访问权限发生变化",
            String::new(),
            detail,
        );
    }
    events
}

/// Fetch the commits behind pushes and new branches. Provider failures leave a
/// note on the event; they never fail the poll, whose baseline must advance.
async fn enrich(access: &MonitorAccess, events: &mut [Event]) {
    for event in events
        .iter_mut()
        .filter(|e| {
            e.event == "push" || (e.event == "branch.created" && e.detail.get("commit").is_none())
        })
        .take(20)
    {
        match access.commits(&event.repository, &event.reference).await {
            Ok(rows) => apply_commits(event, rows.iter().map(detail::commit).collect()),
            Err(error) => note(
                &mut event.detail,
                &format!("提交明细获取失败：{}", bounded(&error.1)),
            ),
        }
    }
}

/// Attach a branch's newest-first commits: the head of a new branch, or the
/// commits a push added since its previous revision.
fn apply_commits(event: &mut Event, commits: Vec<Value>) {
    if event.event == "branch.created" {
        if let Some(head) = commits.first() {
            event.actor = bounded(&detail::commit_author(head));
            event.detail["commit"] = head.clone();
            if !event.actor.is_empty() {
                note(&mut event.detail, NOTE_PUSH_AUTHOR);
            }
        }
        return;
    }
    let before = detail::field(&event.detail, &["/before"], 64);
    let found = commits
        .iter()
        .position(|c| detail::field(c, &["/id"], 64) == before);
    let pushed = &commits[..found.unwrap_or(commits.len())];
    match found {
        Some(count) => event.detail["total_commits"] = json!(count),
        None => note(
            &mut event.detail,
            "最近的提交中未找到推送前的版本，可能是强制推送或一次推送了大量提交；以下为该分支最新的提交",
        ),
    }
    if pushed.len() > detail::COMMITS {
        event.detail["commits_truncated"] = Value::Bool(true);
    }
    event.detail["commits"] = Value::from(
        pushed
            .iter()
            .take(detail::COMMITS)
            .cloned()
            .collect::<Vec<_>>(),
    );
    if let Some(head) = pushed.first() {
        event.actor = bounded(&detail::commit_author(head));
        if !event.actor.is_empty() {
            note(&mut event.detail, NOTE_PUSH_AUTHOR);
        }
    }
}

struct Claim {
    monitor: monitors::Model,
    connection: connections::Model,
    config: Config,
    state: polls::Model,
    owner: String,
    name: String,
}
async fn claim(s: &AppState, row: polls::Model) -> Result<Option<Claim>> {
    let tx = crate::variables::transaction(s, row.workspace_id.into()).await?;
    let Some(monitor) = monitors::Entity::find_by_id(row.id).one(&tx).await? else {
        return Ok(None);
    };
    let Some(connection) = connections::Entity::find_by_id(row.id).one(&tx).await? else {
        return Ok(None);
    };
    let Some(state) = polls::Entity::find_by_id(row.id).one(&tx).await? else {
        return Ok(None);
    };
    let config = decode(s, &monitor)?;
    let now = db::now();
    if !config.enabled
        || config.mode != Mode::Poll
        || config.provider != connection.provider
        || state.config_version != monitor.version
        || state.next_poll_at > now
        || connection.workspace_id != row.workspace_id
    {
        return Ok(None);
    }
    let name = lease_name(row.id);
    if locks::Entity::find_by_id(&name)
        .one(&tx)
        .await?
        .is_some_and(|v| v.expires_at >= now)
    {
        return Ok(None);
    }
    let owner = s.next_id()?.to_string();
    if !lock::acquire(&tx, &name, &owner, now, LEASE_MS).await? {
        return Ok(None);
    }
    polls::Entity::update_many()
        .col_expr(
            polls::Column::NextPollAt,
            Expr::value(now + i64::from(config.poll_interval_minutes.max(1)) * 60_000),
        )
        .filter(polls::Column::Id.eq(row.id))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    Ok(Some(Claim {
        monitor,
        connection,
        config,
        state,
        owner,
        name,
    }))
}
async fn current(tx: &impl sea_orm::ConnectionTrait, claim: &Claim) -> Result<()> {
    let monitor = monitors::Entity::find_by_id(claim.monitor.id)
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let connection = connections::Entity::find_by_id(claim.connection.id)
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let lease = locks::Entity::find_by_id(&claim.name)
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    if monitor.version != claim.monitor.version
        || connection.version != claim.connection.version
        || lease.owner != claim.owner
        || lease.expires_at <= db::now()
    {
        return Err(ApiError::conflict("轮询归属或连接配置已变更，结果已丢弃。"));
    }
    Ok(())
}
async fn observe(s: &AppState, claim: &Claim) -> Result<()> {
    let previous: Option<Snapshot> = if claim.state.snapshot.is_empty() {
        None
    } else {
        Some(
            serde_json::from_str(&s.vault.open_secret(
                &claim.state.snapshot,
                &poll_aad(claim.state.workspace_id, claim.state.id),
            )?)
            .map_err(|_| ApiError::bad("轮询基线无法读取。"))?,
        )
    };
    // Diff and fetch commit detail before taking the workspace lock: provider
    // I/O must not hold it. current() below still fences these candidates.
    let (mut observed, candidates) = match snapshot(s, &claim.connection, &claim.config).await {
        Ok((next, access)) => {
            let mut events = previous
                .as_ref()
                .map(|previous| changes(previous, &next, &claim.config))
                .unwrap_or_default();
            enrich(&access, &mut events).await;
            (Ok(next), events)
        }
        Err(error) => (Err(error), Vec::new()),
    };
    let tx = crate::variables::transaction(s, claim.connection.workspace_id.into()).await?;
    current(&tx, claim).await?;
    // Resolve permissions again after network I/O. Permission failures may update
    // scheduling/error metadata, but may never admit events or replace a baseline.
    let authorized = async {
        let scope = Scope::for_actor_in(
            &tx,
            claim.connection.workspace_id.into(),
            claim.connection.user_id.into(),
        )
        .await?;
        scope.require("repositories.read")?;
        scope.require("credentials.read")
    }
    .await;
    if let Err(error) = authorized {
        observed = Err(error);
    }
    let mut deliveries = Vec::new();
    match observed {
        Ok(next) => {
            for (index, event) in candidates.into_iter().enumerate() {
                // Unique persisted sampling identity distinguishes A→B→A→B.
                let key = format!("poll:{}:{index}", claim.owner);
                if let Some(id) =
                    record(s, &tx, &claim.connection, &claim.config, &event, &key).await?
                {
                    deliveries.push((id, event));
                }
            }
            let bytes = serde_json::to_string(&next).map_err(|_| invalid())?;
            let sealed = s
                .vault
                .seal(&bytes, &poll_aad(claim.state.workspace_id, claim.state.id))?;
            polls::Entity::update_many()
                .col_expr(polls::Column::Snapshot, Expr::value(sealed))
                .col_expr(polls::Column::LastError, Expr::value(""))
                .filter(polls::Column::Id.eq(claim.state.id))
                .exec(&tx)
                .await?;
        }
        Err(error) => {
            polls::Entity::update_many()
                .col_expr(polls::Column::LastError, Expr::value(bounded(&error.1)))
                .filter(polls::Column::Id.eq(claim.state.id))
                .exec(&tx)
                .await?;
        }
    }
    let now = db::now();
    polls::Entity::update_many()
        .col_expr(polls::Column::LastPolledAt, Expr::value(now))
        .col_expr(
            polls::Column::NextPollAt,
            Expr::value(now + i64::from(claim.config.poll_interval_minutes.max(1)) * 60_000),
        )
        .filter(polls::Column::Id.eq(claim.state.id))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    for (id, event) in deliveries {
        deliver(s, &claim.connection, &claim.config, id, &event).await?;
    }
    Ok(())
}
async fn poll(s: &AppState, row: polls::Model, drain: &mut watch::Receiver<bool>) -> Result<()> {
    if *drain.borrow() {
        return Ok(());
    }
    let Some(claim) = claim(s, row).await? else {
        return Ok(());
    };
    let result = {
        // Keep observation advancing while a renewal waits for the SQL pool.
        let renewal = async {
            let mut interval = tokio::time::interval(Duration::from_secs(15));
            interval.tick().await;
            loop {
                interval.tick().await;
                if lock::renew(&s.db, &claim.name, &claim.owner, db::now(), LEASE_MS).await? != 1 {
                    return Err(ApiError::conflict("轮询租约已失效，结果已丢弃。"));
                }
            }
        };
        tokio::select! {
            result = observe(s, &claim) => result,
            _ = drain.changed() => Ok(()),
            result = renewal => result,
        }
    };
    lock::release(&s.db, &claim.name, &claim.owner).await?;
    result
}
/// Remove repository events older than each workspace's configured retention.
async fn prune(s: &AppState) -> Result<()> {
    use crate::entities::settings;
    use sea_orm::QuerySelect;
    const KEY: &str = "repository_event_retention_days";
    for setting in settings::Entity::find()
        .filter(settings::Column::Key.eq(KEY))
        .filter(settings::Column::Value.gt(0))
        .all(&s.db)
        .await?
    {
        let workspace = Id::from(setting.workspace_id);
        let tx = crate::variables::transaction(s, workspace).await?;
        let days = crate::maintenance::setting(&tx, workspace, KEY).await?;
        if days > 0 {
            let rows = events::Entity::find()
                .filter(events::Column::WorkspaceId.eq(workspace))
                .filter(events::Column::CreatedAt.lt(db::now() - days * 86_400_000))
                .limit(200)
                .all(&tx)
                .await?;
            if !rows.is_empty() {
                let removed = events::Entity::delete_many()
                    .filter(events::Column::WorkspaceId.eq(workspace))
                    .filter(events::Column::Id.is_in(rows.iter().map(|r| r.id)))
                    .exec(&tx)
                    .await?
                    .rows_affected;
                crate::hosts::activity_record(
                    s,
                    &tx,
                    workspace,
                    (Id::default(), "系统"),
                    "retention-cleanup",
                    "repository-events",
                    Id::default(),
                    json!({"removed":removed,"retention_days":days}),
                )
                .await?;
            }
        }
        tx.commit().await?;
    }
    Ok(())
}

pub async fn scheduler(s: AppState, mut drain: watch::Receiver<bool>) {
    let mut pruned_at = 0;
    loop {
        if *drain.borrow() {
            break;
        }
        if db::now() - pruned_at > 600_000 {
            pruned_at = db::now();
            if let Err(error) = prune(&s).await {
                tracing::warn!(error=%error.1,"仓库事件保留期清理未完成");
            }
        }
        match polls::Entity::find()
            .filter(polls::Column::NextPollAt.lte(db::now()))
            .all(&s.db)
            .await
        {
            Ok(rows) => {
                let mut work = stream::iter(rows)
                    .map(|row| {
                        let s = s.clone();
                        let mut drain = drain.clone();
                        async move { poll(&s, row, &mut drain).await }
                    })
                    .buffer_unordered(4);
                while let Some(result) = work.next().await {
                    if let Err(error) = result {
                        tracing::warn!(error=%error.1,"仓库轮询未完成");
                    }
                }
            }
            Err(error) => tracing::error!(%error,"读取仓库轮询计划失败"),
        }
        tokio::select! { _=drain.changed()=>{}, _=tokio::time::sleep(Duration::from_secs(2))=>{} }
    }
}
