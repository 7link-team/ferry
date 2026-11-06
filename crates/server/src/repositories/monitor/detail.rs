//! Structured event detail shared by event push and polling: commits, merge
//! requests, tags and repositories normalised across Codeup, GitHub, GitLab,
//! Gitee and Gitea. Author e-mail addresses are never kept.
#[cfg(test)]
use serde_json::json;
use serde_json::{Map, Value};

/// Maximum commits kept per event; the total count is reported separately.
pub(super) const COMMITS: usize = 20;

/// Only application-owned notes are decoded; provider and user fields stay literal.
pub(super) fn display(detail: &Value) -> Value {
    let mut detail = detail.clone();
    if let Some(note) = detail.get("note").and_then(Value::as_str) {
        let displayed = crate::i18n::display_stored(note);
        detail["note"] = Value::from(displayed);
    }
    detail
}

fn clean(value: &str, limit: usize, multiline: bool) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() || (multiline && (*c == '\n' || *c == '\t')))
        .take(limit)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// First non-empty string (or number) among JSON pointers.
pub(super) fn field(value: &Value, paths: &[&str], limit: usize) -> String {
    paths
        .iter()
        .filter_map(|path| value.pointer(path))
        .find_map(|v| match v {
            Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .map(|s| clean(&s, limit, false))
        .unwrap_or_default()
}

fn message(value: &Value, paths: &[&str], limit: usize) -> String {
    paths
        .iter()
        .filter_map(|path| value.pointer(path).and_then(Value::as_str))
        .find(|s| !s.trim().is_empty())
        .map(|s| clean(s, limit, true))
        .unwrap_or_default()
}

/// Insert non-empty values only, so absent platform data stays absent.
fn put(map: &mut Map<String, Value>, key: &str, value: impl Into<Value>) {
    let value = value.into();
    let empty = match &value {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    };
    if !empty {
        map.insert(key.into(), value);
    }
}

fn person(value: &Value) -> String {
    field(
        value,
        &["/login", "/username", "/name", "/nickname", "/user_name"],
        200,
    )
}

/// One commit from a push payload or a commit listing.
pub(super) fn commit(value: &Value) -> Value {
    let id = field(value, &["/sha", "/id", "/commitId"], 64);
    let mut map = Map::new();
    put(&mut map, "id", id.clone());
    let short = field(value, &["/short_id", "/shortId"], 12);
    put(
        &mut map,
        "short_id",
        if short.is_empty() {
            id.chars().take(8).collect()
        } else {
            short
        },
    );
    put(
        &mut map,
        "message",
        message(value, &["/commit/message", "/message", "/title"], 2000),
    );
    put(
        &mut map,
        "author",
        field(
            value,
            &[
                "/commit/author/name",
                "/author/name",
                "/author_name",
                "/authorName",
                "/author/username",
            ],
            200,
        ),
    );
    put(
        &mut map,
        "author_username",
        field(
            value,
            &["/author/login", "/author/username", "/author_username"],
            200,
        ),
    );
    put(
        &mut map,
        "committer",
        field(
            value,
            &[
                "/commit/committer/name",
                "/committer/name",
                "/committer_name",
                "/committerName",
            ],
            200,
        ),
    );
    put(
        &mut map,
        "time",
        field(
            value,
            &[
                "/commit/author/date",
                "/timestamp",
                "/authored_date",
                "/authoredDate",
                "/committedDate",
                "/created_at",
                "/created",
            ],
            64,
        ),
    );
    put(
        &mut map,
        "url",
        field(value, &["/html_url", "/web_url", "/webUrl", "/url"], 1000),
    );
    Value::Object(map)
}

/// A merge request / pull request from a listing or webhook object.
pub(super) fn request(value: &Value) -> Value {
    let mut map = Map::new();
    put(
        &mut map,
        "number",
        field(value, &["/number", "/iid", "/localId", "/id"], 32),
    );
    put(&mut map, "title", field(value, &["/title"], 500));
    put(
        &mut map,
        "description",
        message(value, &["/description", "/body"], 4000),
    );
    put(&mut map, "state", field(value, &["/state", "/status"], 64));
    put(
        &mut map,
        "merged",
        value["merged"].as_bool().unwrap_or(false)
            || field(value, &["/state"], 64).eq_ignore_ascii_case("merged"),
    );
    put(
        &mut map,
        "source",
        field(
            value,
            &["/sourceBranch", "/source_branch", "/head/ref"],
            400,
        ),
    );
    put(
        &mut map,
        "target",
        field(
            value,
            &["/targetBranch", "/target_branch", "/base/ref"],
            400,
        ),
    );
    put(
        &mut map,
        "author",
        field(
            value,
            &[
                "/user/login",
                "/author/username",
                "/author/login",
                "/author/name",
                "/author/userName",
                "/creator/username",
                "/creator/name",
                "/user/username",
                "/user/name",
            ],
            200,
        ),
    );
    put(
        &mut map,
        "merged_by",
        field(
            value,
            &[
                "/merged_by/login",
                "/merged_by/username",
                "/merged_by/name",
                "/merger/username",
                "/merger/name",
                "/mergedBy/name",
            ],
            200,
        ),
    );
    let reviewers: Vec<Value> = ["/requested_reviewers", "/reviewers", "/assignees"]
        .iter()
        .filter_map(|path| value.pointer(path).and_then(Value::as_array))
        .flatten()
        .map(person)
        .filter(|name| !name.is_empty())
        .take(20)
        .map(Value::from)
        .collect();
    let mut unique = Vec::new();
    for reviewer in reviewers {
        if !unique.contains(&reviewer) {
            unique.push(reviewer);
        }
    }
    put(&mut map, "reviewers", unique);
    put(
        &mut map,
        "merge_commit",
        field(
            value,
            &["/merge_commit_sha", "/mergedRevision", "/merge_commit/sha"],
            64,
        ),
    );
    put(
        &mut map,
        "head",
        field(value, &["/head/sha", "/sha", "/last_commit/id"], 64),
    );
    put(
        &mut map,
        "url",
        field(
            value,
            &["/html_url", "/web_url", "/webUrl", "/detailUrl", "/url"],
            1000,
        ),
    );
    for (key, paths) in [
        (
            "created_at",
            &["/created_at", "/createdAt", "/createTime"][..],
        ),
        ("updated_at", &["/updated_at", "/updatedAt", "/updateTime"]),
        ("merged_at", &["/merged_at", "/mergedAt"]),
        ("closed_at", &["/closed_at", "/closedAt"]),
    ] {
        put(&mut map, key, field(value, paths, 64));
    }
    Value::Object(map)
}

/// A branch or tag listing row: the commit it points at and tag annotation.
pub(super) fn reference(value: &Value) -> Value {
    let mut map = Map::new();
    if let Some(row) = value.get("commit").filter(|c| c.is_object()) {
        put(&mut map, "commit", commit(row));
    }
    put(
        &mut map,
        "target",
        field(
            value,
            &["/target", "/commitId", "/commit/sha", "/commit/id"],
            64,
        ),
    );
    put(
        &mut map,
        "message",
        message(value, &["/message", "/release/description"], 2000),
    );
    put(
        &mut map,
        "tagger",
        field(value, &["/tagger/name", "/tagger/username"], 200),
    );
    Value::Object(map)
}

/// A repository listing row.
pub(super) fn repository(value: &Value) -> Value {
    let mut map = Map::new();
    put(
        &mut map,
        "description",
        message(value, &["/description"], 1000),
    );
    put(
        &mut map,
        "visibility",
        field(value, &["/visibility", "/visibilityLevel"], 32),
    );
    if map.get("visibility").is_none() {
        if let Some(private) = value["private"].as_bool() {
            map.insert(
                "visibility".into(),
                Value::from(if private { "private" } else { "public" }),
            );
        }
    }
    put(
        &mut map,
        "default_branch",
        field(value, &["/default_branch", "/defaultBranch"], 400),
    );
    put(
        &mut map,
        "url",
        field(value, &["/html_url", "/web_url", "/webUrl", "/url"], 1000),
    );
    Value::Object(map)
}

/// Detail of an event push payload, for every supported platform's shape.
pub(super) fn webhook(event: &str, payload: &Value) -> Value {
    let mut map = Map::new();
    put(&mut map, "source", "webhook");
    put(
        &mut map,
        "reference_type",
        field(payload, &["/ref_type"], 16),
    );
    if let (Some(added), Some(deleted)) = (
        payload.pointer("/stats/additions").and_then(Value::as_u64),
        payload.pointer("/stats/deletions").and_then(Value::as_u64),
    ) {
        map.insert(
            "line_stats".into(),
            serde_json::json!({"additions":added,"deletions":deleted}),
        );
    }
    put(
        &mut map,
        "repository_url",
        field(
            payload,
            &[
                "/repository/html_url",
                "/repository/web_url",
                "/repository/homepage",
                "/project/web_url",
                "/repository/webUrl",
                "/object_attributes/target/web_url",
            ],
            1000,
        ),
    );
    put(&mut map, "before", field(payload, &["/before"], 64));
    put(
        &mut map,
        "after",
        field(payload, &["/after", "/checkout_sha"], 64),
    );
    if payload["forced"] == true {
        map.insert("forced".into(), Value::Bool(true));
    }
    put(
        &mut map,
        "compare_url",
        field(payload, &["/compare", "/compare_url"], 1000),
    );
    let commits: Vec<Value> = payload["commits"]
        .as_array()
        .map(|rows| rows.iter().map(commit).collect())
        .unwrap_or_default();
    let total = payload["total_commits_count"]
        .as_u64()
        .unwrap_or(commits.len() as u64);
    if total > 0 {
        map.insert("total_commits".into(), Value::from(total));
    }
    if commits.len() > COMMITS {
        map.insert("commits_truncated".into(), Value::Bool(true));
    }
    // Newest first, matching polling listings.
    let mut commits: Vec<Value> = commits.into_iter().rev().take(COMMITS).collect();
    if commits.is_empty() {
        if let Some(head) = payload.get("head_commit").filter(|c| c.is_object()) {
            commits.push(commit(head));
        }
    }
    put(&mut map, "commits", commits);
    if let Some(object) = payload
        .get("pull_request")
        .or_else(|| {
            payload
                .get("object_attributes")
                .filter(|_| event == "merge" || event == "pull_request")
        })
        .filter(|v| v.is_object())
    {
        let mut detail = request(object);
        if let Value::Object(detail) = &mut detail {
            if !detail.contains_key("merged_by") && event == "merge" {
                put(
                    detail,
                    "merged_by",
                    field(
                        payload,
                        &["/user/username", "/user/name", "/sender/login"],
                        200,
                    ),
                );
            }
            if !detail.contains_key("reviewers") {
                let reviewers: Vec<Value> = ["/reviewers", "/assignees"]
                    .iter()
                    .filter_map(|path| payload.pointer(path).and_then(Value::as_array))
                    .flatten()
                    .map(person)
                    .filter(|name| !name.is_empty())
                    .map(Value::from)
                    .collect();
                put(detail, "reviewers", reviewers);
            }
        }
        map.insert("request".into(), detail);
    }
    if event.starts_with("tag.") {
        put(
            &mut map,
            "tag_message",
            message(payload, &["/message"], 2000),
        );
    }
    if event.starts_with("repository.") {
        if let Some(repository) = payload.get("repository").or_else(|| payload.get("project")) {
            map.insert("repository".into(), self::repository(repository));
        }
    }
    Value::Object(map)
}

/// Operator for an event push: who pushed, merged or deleted.
pub(super) fn webhook_actor(payload: &Value) -> String {
    field(
        payload,
        &[
            "/sender/login",
            "/pusher/username",
            "/pusher/name",
            "/user_username",
            "/user/username",
            "/user_name",
            "/user/name",
            "/operator/username",
            "/operator/name",
        ],
        200,
    )
}

/// Readable event name, matching the event options of the interface.
pub(super) fn label(event: &str) -> String {
    match event {
        "repository.created" => crate::i18n::render(
            crate::i18n::current_locale(),
            &crate::i18n::Message::new("repository.monitor.repositoryCreated"),
        ),
        "repository.deleted" => crate::i18n::render(
            crate::i18n::current_locale(),
            &crate::i18n::Message::new("repository.monitor.repositoryDeleted"),
        ),
        "branch.created" => crate::i18n::render(
            crate::i18n::current_locale(),
            &crate::i18n::Message::new("repository.monitor.branchCreated"),
        ),
        "branch.deleted" => crate::i18n::render(
            crate::i18n::current_locale(),
            &crate::i18n::Message::new("repository.monitor.branchDeleted"),
        ),
        "push" => crate::i18n::render(
            crate::i18n::current_locale(),
            &crate::i18n::Message::new("repository.monitor.push"),
        ),
        "pull_request" => crate::i18n::render(
            crate::i18n::current_locale(),
            &crate::i18n::Message::new("repository.monitor.pullRequest"),
        ),
        "merge" => crate::i18n::render(
            crate::i18n::current_locale(),
            &crate::i18n::Message::new("repository.monitor.merge"),
        ),
        "tag.created" => crate::i18n::render(
            crate::i18n::current_locale(),
            &crate::i18n::Message::new("repository.monitor.tagCreated"),
        ),
        "tag.deleted" => crate::i18n::render(
            crate::i18n::current_locale(),
            &crate::i18n::Message::new("repository.monitor.tagDeleted"),
        ),
        other => other.to_owned(),
    }
}

/// Readable notification lines for an event detail.
pub(super) fn lines(detail: &Value) -> Vec<(String, String)> {
    let mut lines = Vec::new();
    let mut add = |name: &str, value: String| {
        if !value.is_empty() {
            lines.push((name.to_owned(), value));
        }
    };
    if let Some(request) = detail.get("request") {
        add(
            &crate::i18n::render(
                crate::i18n::current_locale(),
                &crate::i18n::Message::new("repository.monitor.requestLabel"),
            ),
            format!(
                "#{} {}",
                field(request, &["/number"], 32),
                field(request, &["/title"], 500)
            )
            .trim_start_matches("# ")
            .trim()
            .to_owned(),
        );
        add(
            &crate::i18n::render(
                crate::i18n::current_locale(),
                &crate::i18n::Message::new("repository.monitor.branchLabel"),
            ),
            {
                let (source, target) = (
                    field(request, &["/source"], 400),
                    field(request, &["/target"], 400),
                );
                if source.is_empty() && target.is_empty() {
                    String::new()
                } else {
                    format!("{source} → {target}")
                }
            },
        );
        add(
            &crate::i18n::render(
                crate::i18n::current_locale(),
                &crate::i18n::Message::new("repository.monitor.authorLabel"),
            ),
            field(request, &["/author"], 200),
        );
        add(
            &crate::i18n::render(
                crate::i18n::current_locale(),
                &crate::i18n::Message::new("repository.monitor.mergerLabel"),
            ),
            field(request, &["/merged_by"], 200),
        );
        add(
            &crate::i18n::render(
                crate::i18n::current_locale(),
                &crate::i18n::Message::new("repository.monitor.noteLabel"),
            ),
            message(request, &["/description"], 500),
        );
        add(
            &crate::i18n::render(
                crate::i18n::current_locale(),
                &crate::i18n::Message::new("repository.monitor.linkLabel"),
            ),
            field(request, &["/url"], 1000),
        );
    }
    if let (Some(added), Some(deleted)) = (
        detail
            .pointer("/line_stats/additions")
            .and_then(Value::as_u64),
        detail
            .pointer("/line_stats/deletions")
            .and_then(Value::as_u64),
    ) {
        for (key, count) in [
            ("repository.monitor.addedLinesLabel", added),
            ("repository.monitor.deletedLinesLabel", deleted),
        ] {
            add(
                &crate::i18n::render(
                    crate::i18n::current_locale(),
                    &crate::i18n::Message::new(key),
                ),
                crate::i18n::render(
                    crate::i18n::current_locale(),
                    &crate::i18n::Message::new("repository.monitor.lineCount")
                        .with("count", count.to_string()),
                ),
            );
        }
    }
    if let Some(total) = detail["total_commits"].as_u64() {
        add(
            &crate::i18n::render(
                crate::i18n::current_locale(),
                &crate::i18n::Message::new("repository.monitor.commitCountLabel"),
            ),
            total.to_string(),
        );
    }
    if let Some(commits) = detail["commits"].as_array() {
        // One field per fact, so receivers can read or parse each one.
        for commit in commits.iter().take(5) {
            add(
                &crate::i18n::render(
                    crate::i18n::current_locale(),
                    &crate::i18n::Message::new("repository.monitor.commitLabel"),
                ),
                field(commit, &["/short_id", "/id"], 12),
            );
            add(
                &crate::i18n::render(
                    crate::i18n::current_locale(),
                    &crate::i18n::Message::new("repository.monitor.commitMessageLabel"),
                ),
                message(commit, &["/message"], 300)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_owned(),
            );
            add(
                &crate::i18n::render(
                    crate::i18n::current_locale(),
                    &crate::i18n::Message::new("repository.monitor.commitAuthorLabel"),
                ),
                field(commit, &["/author"], 200),
            );
        }
        if commits.len() > 5 {
            add(
                &crate::i18n::render(
                    crate::i18n::current_locale(),
                    &crate::i18n::Message::new("repository.monitor.moreCommitsLabel"),
                ),
                crate::i18n::render(
                    crate::i18n::current_locale(),
                    &crate::i18n::Message::new("repository.monitor.moreCommits")
                        .with("count", (commits.len() - 5).to_string()),
                ),
            );
        }
    }
    if let Some(commit) = detail.get("commit") {
        add(
            &crate::i18n::render(
                crate::i18n::current_locale(),
                &crate::i18n::Message::new("repository.monitor.latestCommitLabel"),
            ),
            field(commit, &["/short_id", "/id"], 12),
        );
        add(
            &crate::i18n::render(
                crate::i18n::current_locale(),
                &crate::i18n::Message::new("repository.monitor.commitMessageLabel"),
            ),
            message(commit, &["/message"], 300)
                .lines()
                .next()
                .unwrap_or("")
                .to_owned(),
        );
        add(
            &crate::i18n::render(
                crate::i18n::current_locale(),
                &crate::i18n::Message::new("repository.monitor.commitAuthorLabel"),
            ),
            field(commit, &["/author"], 200),
        );
    }
    add(
        &crate::i18n::render(
            crate::i18n::current_locale(),
            &crate::i18n::Message::new("repository.monitor.tagNoteLabel"),
        ),
        message(detail, &["/tag_message", "/message"], 500),
    );
    // One link to open the change: the comparison, else the newest commit.
    add(
        &crate::i18n::render(
            crate::i18n::current_locale(),
            &crate::i18n::Message::new("repository.monitor.linkLabel"),
        ),
        field(
            detail,
            &["/compare_url", "/commits/0/url", "/commit/url"],
            1000,
        ),
    );
    add(
        &crate::i18n::render(
            crate::i18n::current_locale(),
            &crate::i18n::Message::new("repository.monitor.noteLabel"),
        ),
        field(detail, &["/note"], 300),
    );
    lines
}

/// One-line summary for event lists: merge request title, newest commit
/// subject, or tag annotation.
pub(super) fn summary(detail: &Value) -> String {
    let first_line = |text: String| text.lines().next().unwrap_or("").trim().to_owned();
    let request = field(detail, &["/request/title"], 200);
    if !request.is_empty() {
        let number = field(detail, &["/request/number"], 32);
        return if number.is_empty() {
            request
        } else {
            format!("#{number} {request}")
        };
    }
    for path in [
        "/commits/0/message",
        "/commit/message",
        "/message",
        "/tag_message",
    ] {
        let text = first_line(message(detail, &[path], 300));
        if !text.is_empty() {
            return text;
        }
    }
    String::new()
}

/// Flattened detail for the shared read-only detail page.
pub(super) fn view(detail: &Value) -> Map<String, Value> {
    let mut map = Map::new();
    let text = |paths: &[&str], limit: usize| field(detail, paths, limit);
    put(&mut map, "source", text(&["/source"], 16));
    put(&mut map, "before", text(&["/before"], 64));
    put(&mut map, "after", text(&["/after"], 64));
    if detail["forced"] == true {
        map.insert("forced".into(), Value::Bool(true));
    }
    put(&mut map, "compare_url", text(&["/compare_url"], 1000));
    if let Some(total) = detail["total_commits"].as_u64() {
        map.insert("total_commits".into(), Value::from(total));
    }
    if detail["commits_truncated"] == true {
        map.insert(
            "commits_truncated".into(),
            Value::from(crate::i18n::render(
                crate::i18n::current_locale(),
                &crate::i18n::Message::new("repository.monitor.truncatedCommits")
                    .with("count", COMMITS.to_string()),
            )),
        );
    }
    if let Some(commits) = detail["commits"].as_array() {
        put(&mut map, "commits", commits.clone());
    }
    if let Some(commit) = detail.get("commit").filter(|c| c.is_object()) {
        map.insert("head_commit".into(), Value::from(vec![commit.clone()]));
    }
    if let Some(request) = detail.get("request") {
        for key in [
            "number",
            "title",
            "state",
            "author",
            "merged_by",
            "merge_commit",
            "head",
            "url",
            "created_at",
            "updated_at",
            "merged_at",
            "closed_at",
        ] {
            put(
                &mut map,
                &format!("request_{key}"),
                field(request, &[&format!("/{key}")], 1000),
            );
        }
        put(
            &mut map,
            "request_description",
            message(request, &["/description"], 4000),
        );
        let (source, target) = (
            field(request, &["/source"], 400),
            field(request, &["/target"], 400),
        );
        if !source.is_empty() || !target.is_empty() {
            map.insert(
                "request_branches".into(),
                Value::from(format!("{source} → {target}")),
            );
        }
        let reviewers: Vec<String> = request["reviewers"]
            .as_array()
            .map(|r| {
                r.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        put(&mut map, "request_reviewers", reviewers.join("、"));
    }
    put(&mut map, "tag_target", text(&["/target", "/commit/id"], 64));
    put(
        &mut map,
        "tag_message",
        message(detail, &["/message", "/tag_message"], 2000),
    );
    put(&mut map, "tagger", text(&["/tagger"], 200));
    if let Some(repository) = detail.get("repository").filter(|r| r.is_object()) {
        for key in ["visibility", "default_branch", "url"] {
            put(
                &mut map,
                &format!("repository_{key}"),
                field(repository, &[&format!("/{key}")], 1000),
            );
        }
        put(
            &mut map,
            "repository_description",
            message(repository, &["/description"], 1000),
        );
    }
    put(&mut map, "note", message(detail, &["/note"], 1000));
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_commits_from_each_platform_without_emails() {
        let github = json!({"sha":"abcdef1234567","commit":{"message":"fix: a\n\nbody","author":{"name":"Ann","email":"a@x","date":"2026-11-01T00:00:00Z"}},"author":{"login":"ann"},"html_url":"https://github.com/o/r/commit/abc"});
        assert_eq!(
            commit(&github),
            json!({"id":"abcdef1234567","short_id":"abcdef12","message":"fix: a\n\nbody","author":"Ann","author_username":"ann","time":"2026-11-01T00:00:00Z","url":"https://github.com/o/r/commit/abc"})
        );
        let gitlab = json!({"id":"1234567890","short_id":"1234567","title":"t","message":"t\nmore","author_name":"Bo","author_email":"b@x","authored_date":"d","web_url":"u"});
        assert_eq!(
            commit(&gitlab),
            json!({"id":"1234567890","short_id":"1234567","message":"t\nmore","author":"Bo","time":"d","url":"u"})
        );
        let codeup = json!({"id":"c1","shortId":"c1s","message":"m","authorName":"Cy","authorEmail":"c@x","committedDate":"t","webUrl":"w"});
        assert_eq!(commit(&codeup)["author"], "Cy");
        assert!(!commit(&codeup).to_string().contains("c@x"));
        let push = json!({"id":"p","message":"m","timestamp":"t","url":"u","author":{"name":"Dee","email":"d@x","username":"dee"}});
        assert_eq!(commit(&push)["author_username"], "dee");
    }

    #[test]
    fn webhook_detail_keeps_commits_newest_first_and_merge_request_text() {
        let push = json!({"before":"b0","after":"a1","total_commits_count":2,"compare":"https://x/compare","commits":[{"id":"old","message":"first"},{"id":"new","message":"second"}],"user_username":"eve"});
        let detail = webhook("push", &push);
        assert_eq!(detail["commits"][0]["id"], "new");
        assert_eq!(detail["total_commits"], 2);
        assert_eq!(detail["source"], "webhook");
        assert_eq!(webhook_actor(&push), "eve");
        let merge = json!({"object_attributes":{"iid":7,"title":"Add login","description":"Long\ndescription","state":"merged","source_branch":"feature","target_branch":"main","url":"https://x/mr/7","merge_commit_sha":"m1"},"user":{"username":"lead"},"reviewers":[{"username":"rev"}]});
        let detail = webhook("merge", &merge);
        assert_eq!(detail["request"]["title"], "Add login");
        assert_eq!(detail["request"]["description"], "Long\ndescription");
        assert_eq!(detail["request"]["merged_by"], "lead");
        assert_eq!(detail["request"]["reviewers"], json!(["rev"]));
        let pr = json!({"pull_request":{"number":3,"title":"T","body":"B","merged":true,"merged_by":{"login":"m"},"user":{"login":"u"},"head":{"ref":"h"},"base":{"ref":"main"},"html_url":"https://github.com/o/r/pull/3","requested_reviewers":[{"login":"r1"},{"login":"r1"}]}});
        let detail = webhook("merge", &pr)["request"].clone();
        assert_eq!(
            (
                detail["author"].as_str(),
                detail["merged_by"].as_str(),
                detail["reviewers"].clone()
            ),
            (Some("u"), Some("m"), json!(["r1"]))
        );
    }

    #[test]
    fn summary_and_view_flatten_detail_for_lists_and_detail_pages() {
        let push = json!({"source":"polling","before":"b","after":"a","total_commits":3,"commits_truncated":true,"commits":[{"id":"a","message":"feat: x

body"}],"note":"n"});
        assert_eq!(summary(&push), "feat: x");
        let view = view(&push);
        assert_eq!(
            view["commits_truncated"],
            format!("Only the latest {COMMITS} commits are shown")
        );
        assert_eq!(view["total_commits"], 3);
        assert_eq!(view["note"], "n");
        let merge = json!({"request":{"number":"9","title":"Add","source":"f","target":"main","reviewers":["a","b"],"description":"d"}});
        assert_eq!(summary(&merge), "#9 Add");
        let view = super::view(&merge);
        assert_eq!(
            (
                view["request_branches"].as_str(),
                view["request_reviewers"].as_str(),
                view["request_description"].as_str()
            ),
            (Some("f → main"), Some("a、b"), Some("d"))
        );
        assert_eq!(
            summary(&json!({"message":"release 1.0
notes"})),
            "release 1.0"
        );
        assert!(super::view(&json!({})).is_empty());
    }

    #[test]
    fn notification_lines_summarise_requests_and_commits() {
        let detail = json!({"total_commits":7,"commits":[{"short_id":"a1","message":"one\nbody","author":"Ann"},{"short_id":"a2","message":"two"}],"compare_url":"https://x/c"});
        let summary = lines(&detail);
        assert!(summary.contains(&("Commit count".into(), "7".into())));
        assert!(summary.contains(&("Commit".into(), "a1".into())));
        assert!(summary.contains(&("Commit message".into(), "one".into())));
        assert!(summary.contains(&("Commit author".into(), "Ann".into())));
        assert!(summary.contains(&("Commit message".into(), "two".into())));
        assert!(!summary.iter().any(|(_, value)| value.contains(" — ")));
        assert!(summary.contains(&("Link".into(), "https://x/c".into())));
        let polled =
            lines(&json!({"commits":[{"short_id":"b1","message":"m","url":"https://code/c/b1"}]}));
        assert!(polled.contains(&("Link".into(), "https://code/c/b1".into())));
        assert_eq!(
            (label("push"), label("tag.deleted"), label("custom")),
            (
                "Code pushed".to_owned(),
                "Tag deleted".to_owned(),
                "custom".to_owned()
            )
        );
        let request = lines(
            &json!({"request":{"number":"5","title":"Fix","source":"f","target":"main","author":"u","merged_by":"m","url":"https://x/5"}}),
        );
        assert!(request.contains(&("Merge request".into(), "#5 Fix".into())));
        assert!(request.contains(&("Branch".into(), "f → main".into())));
    }
}
