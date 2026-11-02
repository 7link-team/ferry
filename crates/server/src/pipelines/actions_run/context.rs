use super::*;

/// The secrets and vars contexts the jobs of a run receive; jobs whose
/// `environment` names a Ferry environment get that environment's values.
pub(super) struct Delivered {
    secrets: BTreeMap<String, String>,
    vars: BTreeMap<String, String>,
    jobs: BTreeMap<String, (BTreeMap<String, String>, BTreeMap<String, String>)>,
}

impl Delivered {
    pub(super) fn for_job(
        &self,
        key: &str,
    ) -> (&BTreeMap<String, String>, &BTreeMap<String, String>) {
        self.jobs
            .get(key)
            .map(|(secrets, vars)| (secrets, vars))
            .unwrap_or((&self.secrets, &self.vars))
    }

    pub(super) fn new(opened: &history::VariableSnapshot, snapshot: &Snapshot) -> Result<Self> {
        if let (Some(secrets), Some(vars)) = (&opened.secrets, &opened.vars) {
            let jobs = opened
                .environments
                .iter()
                .filter_map(
                    |(key, environment)| match (&environment.secrets, &environment.vars) {
                        (Some(secrets), Some(vars)) => {
                            Some((key.clone(), (secrets.clone(), vars.clone())))
                        }
                        _ => None,
                    },
                )
                .collect();
            return Ok(Self {
                secrets: secrets.clone(),
                vars: vars.clone(),
                jobs,
            });
        }
        // Runs queued before contexts were snapshotted: static references only.
        let (_, references) = crate::pipelines::actions::parse(&snapshot.document)?;
        let mut delivered = Self {
            secrets: BTreeMap::new(),
            vars: BTreeMap::new(),
            jobs: BTreeMap::new(),
        };
        for reference in references {
            let (namespace, name) = reference.name.split_once(':').ok_or_else(|| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.referenceInvalid"),
                )
            })?;
            let value = opened
                .values
                .get(name)
                .ok_or_else(|| {
                    ApiError::localized(
                        StatusCode::BAD_REQUEST,
                        Message::new("execution.snapshotMissing"),
                    )
                })?
                .clone();
            if namespace == "secrets" {
                delivered.secrets.insert(name.to_owned(), value);
            } else {
                delivered.vars.insert(name.to_owned(), value);
            }
        }
        Ok(delivered)
    }
}

pub(super) struct GithubContext {
    pub(super) event_name: &'static str,
    pub(super) payload: Value,
    pub(super) actor: String,
    pub(super) git_ref: String,
    pub(super) repository: String,
    pub(super) server_url: String,
    pub(super) inputs: BTreeMap<String, String>,
}

/// The github context GitHub would give this run: its trigger as the event,
/// the person who started it, and the repository and ref it builds.
pub(super) fn github_context(run: &runs::Model) -> GithubContext {
    let inputs: BTreeMap<String, Value> = run
        .inputs
        .as_deref()
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_default();
    // The inputs context is typed; github.event.inputs carries strings.
    let inputs: BTreeMap<String, String> = inputs
        .into_iter()
        .map(|(name, value)| {
            let text = match value {
                Value::String(text) => text,
                other => other.to_string(),
            };
            (name, text)
        })
        .collect();
    let (server_url, repository) = repository_parts(&run.repo_url);
    let git_ref = if run.branch.is_empty() {
        String::new()
    } else {
        format!("refs/heads/{}", run.branch)
    };
    let event_name = if run.trigger_kind == "schedule" {
        "schedule"
    } else {
        "workflow_dispatch"
    };
    let mut payload = json!({
        "repository": {"full_name": repository, "html_url": if repository.is_empty() { String::new() } else { format!("{server_url}/{repository}") }},
        "sender": {"login": run.triggered_by_name},
    });
    if event_name == "workflow_dispatch" {
        payload["inputs"] = json!(inputs);
        payload["ref"] = json!(git_ref);
    }
    GithubContext {
        event_name,
        payload,
        actor: run.triggered_by_name.clone(),
        git_ref,
        repository,
        server_url,
        inputs,
    }
}

/// (`https://host`, `owner/repo`) from an HTTP(S), ssh:// or scp-style Git URL,
/// without any credentials it may contain.
pub(crate) fn repository_parts(url: &str) -> (String, String) {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    if let Some((scheme, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        let scheme = if scheme == "http" { "http" } else { "https" };
        return (format!("{scheme}://{host}"), path.to_owned());
    }
    if let Some((authority, path)) = url.split_once(':')
        && !authority.contains('/')
    {
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        return (format!("https://{host}"), path.to_owned());
    }
    (String::new(), String::new())
}

pub(super) fn repository_url_without_credentials(input: &str) -> String {
    if let Some((scheme, rest)) = input.split_once("://") {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        if scheme == "ssh" {
            let authority = authority
                .rsplit_once('@')
                .map(|(user, _)| format!("{}@{host}", user.split(':').next().unwrap_or(user)))
                .unwrap_or_else(|| host.to_owned());
            return format!("ssh://{authority}/{path}");
        }
        format!("{scheme}://{host}/{path}")
    } else if let Some((authority, path)) = input.split_once(':') {
        if !authority.contains('/') {
            return format!("{authority}:{path}");
        }
        input.into()
    } else {
        input.into()
    }
}

pub(crate) fn runtime_path(input: &str, run: &runs::Model) -> String {
    let repository = repository_url_without_credentials(&run.repo_url);
    let number = run.number.to_string();
    let tokens = [
        ("${{ github.run_number }}", number.as_str()),
        ("${{ github.ref_name }}", run.branch.as_str()),
        ("${{ github.sha }}", run.commit.as_str()),
        ("${{ env.FERRY_REPOSITORY_URL }}", repository.as_str()),
    ];
    let mut rest = input;
    let mut output = String::new();
    while let Some(start) = rest.find("${{") {
        output.push_str(&rest[..start]);
        let tail = &rest[start..];
        if let Some((token, value)) = tokens.iter().find(|(token, _)| tail.starts_with(token)) {
            output.push_str(value);
            rest = &tail[token.len()..];
        } else {
            output.push_str("${{");
            rest = &tail[3..];
        }
    }
    output.push_str(rest);
    output
}

pub(super) fn workflow_timeout(workflow: &str, job: &str) -> u64 {
    serde_json::from_str::<Value>(workflow)
        .ok()
        .and_then(|workflow| workflow["jobs"][job]["timeout-minutes"].as_u64())
        .and_then(|minutes| minutes.checked_mul(60))
        .map(|seconds| seconds.saturating_add(60).max(WORKER_SECONDS))
        .unwrap_or(WORKER_SECONDS)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn github_repository_comes_from_the_url_without_credentials() {
        use super::repository_parts;
        for (url, expected) in [
            (
                "https://github.com/acme/app.git",
                ("https://github.com", "acme/app"),
            ),
            (
                "https://user:token@codeup.aliyun.com/org/group/app",
                ("https://codeup.aliyun.com", "org/group/app"),
            ),
            (
                "git@github.com:acme/app.git",
                ("https://github.com", "acme/app"),
            ),
            (
                "ssh://git@git.example.com:2222/acme/app.git",
                ("https://git.example.com:2222", "acme/app"),
            ),
            (
                "http://git.local/acme/app/",
                ("http://git.local", "acme/app"),
            ),
            ("", ("", "")),
        ] {
            let (server, repository) = repository_parts(url);
            assert_eq!((server.as_str(), repository.as_str()), expected, "{url}");
        }
    }

    #[test]
    fn repository_placeholder_keeps_ssh_login_and_removes_passwords() {
        for (input, expected) in [
            (
                "https://user:password@example.test/team/repo.git",
                "https://example.test/team/repo.git",
            ),
            (
                "git@example.test:team/repo.git",
                "git@example.test:team/repo.git",
            ),
            (
                "ssh://git:password@example.test/team/repo.git",
                "ssh://git@example.test/team/repo.git",
            ),
        ] {
            assert_eq!(repository_url_without_credentials(input), expected);
        }
    }
}
