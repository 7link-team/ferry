use super::*;

fn field<'a>(value: &'a Value, paths: &[&str]) -> Option<&'a str> {
    paths
        .iter()
        .find_map(|p| value.pointer(p).and_then(Value::as_str))
}
fn timestamp(value: &Value, paths: &[&str]) -> Result<i64> {
    field(value, paths)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|v| v.timestamp_millis())
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.collector.commitTimeMissing"),
            )
        })
}
fn parents(value: &Value) -> Result<Vec<String>> {
    let values = value
        .get("parents")
        .or_else(|| value.get("parent_ids"))
        .or_else(|| value.get("parentIds"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.collector.parentsMissing"),
            )
        })?;
    values
        .iter()
        .map(|v| {
            v.as_str()
                .or_else(|| field(v, &["/sha", "/id"]))
                .filter(|v| valid_sha(v))
                .map(str::to_owned)
                .ok_or_else(|| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("repository.collector.parentRevisionInvalid"),
                    )
                })
        })
        .collect()
}

pub(super) async fn heads(
    access: &MonitorAccess,
    provider: &str,
    path: &str,
) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    let mut page = 1;
    loop {
        let size = if provider == "codeup" {
            "perPage"
        } else if provider == "gitea" {
            "limit"
        } else {
            "per_page"
        };
        let (value, more) = access
            .analysis_request(
                path,
                &["branches"],
                &[("page", page.to_string()), (size, "50".into())],
            )
            .await?;
        let rows = value.as_array().ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.collector.branchesInvalid"),
            )
        })?;
        for row in rows {
            let name = field(row, &["/name"])
                .filter(|v| !v.is_empty())
                .ok_or_else(|| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("repository.collector.branchNameInvalid"),
                    )
                })?;
            let sha = field(
                row,
                &["/commit/sha", "/commit/id", "/commitId", "/commit/commitId"],
            )
            .filter(|v| valid_sha(v))
            .ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.analysis.branchRevisionInvalid"),
                )
            })?;
            if result.insert(name.into(), sha.into()).is_some() {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.branchesRepeated"),
                ));
            }
        }
        if more == Some(false) || (more.is_none() && rows.len() < 50) {
            break;
        }
        if rows.is_empty() {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.collector.branchesIncomplete"),
            ));
        }
        page += 1;
    }
    Ok(result)
}

fn extension(path: &str) -> String {
    // Go filepath.Ext includes the leading period and treats a dotfile as an extension.
    let filename = path.rsplit('/').next().unwrap_or(path);
    filename
        .rfind('.')
        .map(|i| filename[i..].to_owned())
        .unwrap_or_default()
}
fn language(files: &[Value]) -> Result<BTreeMap<String, i64>> {
    let mut result = BTreeMap::new();
    for file in files {
        let path = field(
            file,
            &[
                "/filename",
                "/new_path",
                "/newPath",
                "/old_path",
                "/oldPath",
            ],
        )
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.collector.filePathMissing"),
            )
        })?;
        *result.entry(extension(path)).or_default() += 1;
    }
    Ok(result)
}
fn ensure_files(files: &[Value]) -> Result<()> {
    // Upstream DiffTree uses no rename detection. Platform rename-aware totals
    // cannot reconstruct the old/new complete blobs, so use Git for parity.
    if files.iter().any(|f| {
        f["status"].as_str() == Some("renamed")
            || f["renamed_file"].as_bool() == Some(true)
            || f["renamed"].as_bool() == Some(true)
    }) {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.collector.renameCombined"),
        ));
    }
    if files.iter().any(|f| {
        ["truncated", "too_large", "collapsed"]
            .iter()
            .any(|k| f[*k].as_bool() == Some(true))
    }) {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.collector.diffTruncated"),
        ));
    }
    Ok(())
}
fn patch_stats(patch: &str) -> (i64, i64) {
    let (mut additions, mut deletions, mut in_hunk) = (0, 0, false);
    for line in patch.lines() {
        if line.starts_with("@@") {
            in_hunk = true;
            continue;
        }
        if line.starts_with("diff --git ") {
            in_hunk = false;
            continue;
        }
        if in_hunk && line.starts_with('+') {
            additions += 1
        }
        if in_hunk && line.starts_with('-') {
            deletions += 1
        }
    }
    (additions, deletions)
}
pub(crate) fn totals(files: &[Value]) -> Result<(i64, i64)> {
    ensure_files(files)?;
    let (mut a, mut d) = (0_i64, 0_i64);
    for file in files {
        let (added, deleted) = match (file["additions"].as_i64(), file["deletions"].as_i64()) {
            (Some(a), Some(d)) => (a, d),
            _ => {
                if let Some(patch) = field(file, &["/diff", "/patch"]) {
                    patch_stats(patch)
                } else {
                    return Err(ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("repository.collector.fileDiffMissing"),
                    ));
                }
            }
        };
        if added < 0 || deleted < 0 {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.collector.negativeLines"),
            ));
        }
        a = a.checked_add(added).ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.collector.lineOverflow"),
            )
        })?;
        d = d.checked_add(deleted).ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.collector.lineOverflow"),
            )
        })?;
    }
    Ok((a, d))
}

pub(super) async fn commit(
    access: &MonitorAccess,
    provider: &str,
    path: &str,
    sha: &str,
    start: i64,
) -> Result<Commit> {
    let resource = if provider == "gitea" {
        vec!["git", "commits", sha]
    } else {
        vec!["commits", sha]
    };
    let (value, mut more) = access
        .analysis_request(
            path,
            &resource,
            &[("page", "1".into()), ("per_page", "100".into())],
        )
        .await?;
    let returned = field(&value, &["/sha", "/id"]).ok_or_else(|| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.collector.commitRevisionMissing"),
        )
    })?;
    if returned != sha {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.collector.commitMismatch"),
        ));
    }
    let mut commit = Commit {
        sha: sha.into(),
        parents: parents(&value)?,
        message: field(&value, &["/commit/message", "/message"])
            .unwrap_or("")
            .into(),
        author_name: field(
            &value,
            &[
                "/commit/author/name",
                "/author_name",
                "/authorName",
                "/author/name",
            ],
        )
        .unwrap_or("")
        .into(),
        author_email: field(
            &value,
            &[
                "/commit/author/email",
                "/author_email",
                "/authorEmail",
                "/author/email",
            ],
        )
        .unwrap_or("")
        .into(),
        author_date: timestamp(
            &value,
            &[
                "/commit/author/date",
                "/authored_date",
                "/authoredDate",
                "/author/date",
            ],
        )?,
        committer_date: timestamp(
            &value,
            &[
                "/commit/committer/date",
                "/committed_date",
                "/committedDate",
                "/committer/date",
            ],
        )?,
        ..Default::default()
    };
    if start != 0 && commit.committer_date < start {
        return Ok(commit);
    }
    if value["truncated"].as_bool() == Some(true) {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.collector.filesTruncated"),
        ));
    }
    let mut files = value["files"].as_array().cloned();
    if provider == "github" {
        let mut page = 2;
        while more == Some(true) {
            let (next, next_more) = access
                .analysis_request(
                    path,
                    &resource,
                    &[("page", page.to_string()), ("per_page", "100".into())],
                )
                .await?;
            let rows = next["files"].as_array().ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.filePageInvalid"),
                )
            })?;
            if rows.is_empty() {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.filePageIncomplete"),
                ));
            }
            files.get_or_insert_default().extend(rows.iter().cloned());
            more = next_more;
            page += 1;
        }
        if files.as_ref().is_some_and(|v| v.len() >= 3000) {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.collector.fileLimit"),
            ));
        }
    }
    if files.is_none() && provider == "gitlab" {
        let mut all = Vec::new();
        let mut page = 1;
        loop {
            let (diff, more) = access
                .analysis_request(
                    path,
                    &["commits", sha, "diff"],
                    &[("page", page.to_string()), ("per_page", "50".into())],
                )
                .await?;
            let rows = diff.as_array().ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.diffListInvalid"),
                )
            })?;
            all.extend(rows.iter().cloned());
            if more == Some(false) || (more.is_none() && rows.len() < 50) {
                break;
            }
            if rows.is_empty() {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.diffPageIncomplete"),
                ));
            }
            page += 1;
        }
        files = Some(all);
    }
    let files = files.ok_or_else(|| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.collector.filesMissing"),
        )
    })?;
    ensure_files(&files)?;
    commit.language_stats = language(&files)?;
    if commit.is_merge() {
        for parent in &commit.parents {
            let compare = format!("{parent}...{sha}");
            let (comparison, more) = if provider == "gitlab" {
                access
                    .analysis_request(
                        path,
                        &["compare"],
                        &[
                            ("from", parent.clone()),
                            ("to", sha.into()),
                            ("straight", "true".into()),
                        ],
                    )
                    .await?
            } else {
                access
                    .analysis_request(path, &["compare", &compare], &[])
                    .await?
            };
            if more == Some(true)
                || comparison["compare_timeout"].as_bool() == Some(true)
                || comparison["truncated"].as_bool() == Some(true)
            {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.mergeComparisonIncomplete"),
                ));
            }
            let changed = comparison["files"]
                .as_array()
                .or_else(|| comparison["diffs"].as_array())
                .ok_or_else(|| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("repository.collector.multiParentMissing"),
                    )
                })?;
            if provider == "github" && changed.len() >= 300 {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.mergeFileLimit"),
                ));
            }
            let (a, d) = totals(changed)?;
            commit.additions = commit.additions.checked_add(a).ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.lineOverflow"),
                )
            })?;
            commit.deletions = commit.deletions.checked_add(d).ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.lineOverflow"),
                )
            })?;
        }
    } else if commit.parents.is_empty() {
        // Upstream sums diffs against parents, so initial commits have zero line
        // totals while their changed-file extensions are still counted.
    } else if let (Some(a), Some(d)) = (
        value["stats"]["additions"].as_i64(),
        value["stats"]["deletions"].as_i64(),
    ) {
        commit.additions = a;
        commit.deletions = d;
    } else {
        (commit.additions, commit.deletions) = totals(&files)?;
    }
    commit.complete = true;
    if !commit.valid() {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.collector.commitStatsInvalid"),
        ));
    }
    Ok(commit)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patches_count_content_headers_and_extensions_like_upstream() {
        assert_eq!(
            patch_stats("--- a/x\n+++ b/x\n@@ -1 +1,2 @@\n-old\n++++literal\n+new\n"),
            (2, 1)
        );
        assert_eq!(extension("dir/.gitignore"), ".gitignore");
        assert_eq!(extension("dir/README"), "");
        assert_eq!(extension("a.ts"), ".ts");
        assert!(totals(&[json!({"additions":1,"deletions":0,"truncated":true})]).is_err());
        assert!(totals(&[json!({"filename":"x"})]).is_err());
    }
}
