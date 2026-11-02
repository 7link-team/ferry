use super::*;

/// Replace bound checkout nodes only; act still evaluates their conditions,
/// reports their original step IDs, and runs them in the selected container.
pub(super) fn checkout_workflow(
    snapshot: &Snapshot,
    job: &Job,
    variables: &BTreeMap<String, String>,
    secrets: &mut BTreeMap<String, String>,
) -> Result<String> {
    if snapshot.checkouts.is_empty() && !snapshot.jobs.iter().any(|item| item.manual) {
        return Ok(snapshot.document.workflow.clone());
    }
    let mut workflow: Value =
        serde_yaml_ng::from_str(&snapshot.document.workflow).map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.yamlInvalid"),
            )
        })?;
    for gate in snapshot.jobs.iter().filter(|item| item.manual) {
        workflow["jobs"][&gate.key] = json!({
            "name": gate.name,
            "needs": gate.needs,
            "runs-on": "self-hosted",
            "steps": [{"run": "echo approved"}],
        });
    }
    for (index, checkout) in snapshot.checkouts.iter().enumerate() {
        let binding = &checkout.binding;
        if binding.job_id != job.key {
            continue;
        }
        let step = workflow["jobs"][&job.key]["steps"]
            .as_array_mut()
            .and_then(|steps| {
                steps
                    .iter_mut()
                    .find(|step| step["id"].as_str() == Some(binding.step_id.as_str()))
            })
            .and_then(Value::as_object_mut)
            .ok_or_else(|| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.sourceStepMissing"),
                )
            })?;
        let mut env = step
            .get("env")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for field in ["KIND", "USER", "SECRET"] {
            let key = format!("__SOURCE_{index}_{field}");
            let value = variables.get(&key).cloned().unwrap_or_default();
            secrets.insert(key.clone(), value);
            env.insert(
                format!("SOURCE_{field}"),
                json!(format!("${{{{ secrets.{key} }}}}")),
            );
        }
        step.remove("uses");
        step.remove("with");
        step.insert("shell".into(), json!("sh"));
        step.insert("working-directory".into(), json!("${{ github.workspace }}"));
        step.insert("env".into(), json!(env));
        step.insert("run".into(), json!(checkout_node_script(binding)));
    }
    serde_json::to_string(&workflow).map_err(|_| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.prepareSourceStep"),
        )
    })
}

fn checkout_node_script(binding: &crate::pipelines::actions::CheckoutBinding) -> String {
    let path = if binding.path.is_empty() {
        "."
    } else {
        &binding.path
    };
    let reference = if binding.git_ref.is_empty() {
        "HEAD"
    } else {
        &binding.git_ref
    };
    format!(
        r#"set -eu
workspace=$(pwd -P)
relative={path}
case "$relative" in
    /*|[A-Za-z]:/*) [ -d "$relative" ] || mkdir -p -- "$relative"; cd -- "$relative"; relative='' ;;
esac
original_ifs=$IFS
IFS=/
set -f
for component in $relative; do
    [ -n "$component" ] || continue
    mkdir -p -- "$component"
    cd -- "$component"
    target=$(pwd -P)
    case "$target/" in "$workspace/"*) ;; *) echo 'Checkout path escapes workspace' >&2; exit 1;; esac
done
set +f
IFS=$original_ifs
# Never mix a checkout into unrelated files: an existing directory must be
# empty or already a Git repository (a persistent workdir from earlier runs).
if [ ! -d .git ] && [ -n "$(ls -A)" ]; then
    echo 'Checkout directory is not empty and is not a Git repository; refusing to check out into it' >&2
    exit 1
fi
umask 077
credential_dir=$(mktemp -d "$workspace/.checkout-credentials.XXXXXX")
trap 'rm -rf -- "$credential_dir"' EXIT HUP INT TERM
chmod 700 "$credential_dir"
export GIT_TERMINAL_PROMPT=0
if [ "$SOURCE_KIND" = ssh-key ]; then
    printf '%s' "$SOURCE_SECRET" > "$credential_dir/key"
    chmod 600 "$credential_dir/key"
    export GIT_SSH_COMMAND="ssh -o BatchMode=yes -o IdentitiesOnly=yes -i '$credential_dir/key'"
elif [ -n "$SOURCE_SECRET" ]; then
    cat > "$credential_dir/helper" <<'HELPER'
#!/bin/sh
cat >/dev/null
[ "$1" = get ] || exit 0
printf 'username=%s\npassword=%s\n\n' "${{SOURCE_USER:-git}}" "$SOURCE_SECRET"
HELPER
    chmod 700 "$credential_dir/helper"
    export GIT_CONFIG_COUNT=2 GIT_CONFIG_KEY_0=credential.helper GIT_CONFIG_VALUE_0=''
    export SOURCE_CREDENTIAL_HELPER="$credential_dir/helper"
    export GIT_CONFIG_KEY_1=credential.helper GIT_CONFIG_VALUE_1={helper}
fi
git init -q
git remote remove origin 2>/dev/null || true
git remote add origin {url}
git fetch --prune --no-tags -- origin {reference}
revision=$(git rev-parse --verify --end-of-options 'FETCH_HEAD^{{commit}}')
git checkout --detach "$revision" --
if [ -n "${{GITHUB_OUTPUT:-}}" ]; then
    printf 'commit=%s\nref=%s\n' "$revision" {reference} >> "$GITHUB_OUTPUT"
fi
"#,
        path = quote(path),
        url = quote(&binding.repository_url),
        reference = quote(reference),
        helper = quote("!\"$SOURCE_CREDENTIAL_HELPER\"")
    )
}

#[cfg(test)]
mod tests {
    use super::super::tests::test_job;
    use super::*;
    #[test]
    fn worker_receives_standard_dependency_for_manual_gate() {
        let workflow = "jobs:\n  gate:\n    ferry-approval: true\n  deploy:\n    needs: gate\n    runs-on: self-hosted\n    steps:\n      - run: echo deploy\n";
        let mut gate = test_job("gate", &[]);
        gate.manual = true;
        gate.host_id = 0;
        let deploy = test_job("deploy", &["gate"]);
        let snapshot = Snapshot {
            engine: crate::pipelines::actions::ENGINE.into(),
            document: crate::pipelines::actions::Document {
                engine: crate::pipelines::actions::ENGINE.into(),
                workflow: workflow.into(),
                executor_bindings: vec![],
                checkout_bindings: vec![],
                graph_layout: Value::Null,
            },
            jobs: vec![gate, deploy.clone()],
            checkouts: vec![],
        };
        let transformed =
            checkout_workflow(&snapshot, &deploy, &BTreeMap::new(), &mut BTreeMap::new()).unwrap();
        let value: Value = serde_json::from_str(&transformed).unwrap();
        assert_eq!(value["jobs"]["gate"]["runs-on"], "self-hosted");
        assert_eq!(value["jobs"]["gate"]["steps"][0]["run"], "echo approved");
        assert!(value["jobs"]["gate"].get("ferry-approval").is_none());
        assert_eq!(value["jobs"]["deploy"]["needs"], "gate");
        assert_eq!(snapshot.document.workflow, workflow);
    }

    #[test]
    fn checkout_nodes_preserve_order_conditions_and_keep_credentials_out_of_scripts() {
        let binding = crate::pipelines::actions::CheckoutBinding {
            job_id: "build".into(),
            step_id: "source".into(),
            connection_id: "1".into(),
            repository_url: "https://example.invalid/project.git".into(),
            git_ref: "release".into(),
            path: "source".into(),
        };
        let snapshot = Snapshot {
            engine: crate::pipelines::actions::ENGINE.into(),
            document: crate::pipelines::actions::Document {
                engine: crate::pipelines::actions::ENGINE.into(),
                workflow: "jobs: {build: {runs-on: self-hosted, steps: [{id: before, run: echo before}, {id: source, if: 'success()', uses: 'actions/checkout@v4'}, {id: after, run: echo after}]}}".into(),
                executor_bindings: vec![], checkout_bindings: vec![binding.clone()], graph_layout: Value::Null,
            },
            jobs: vec![], checkouts: vec![crate::pipelines::actions::ResolvedCheckout { binding, credential_id: 7 }],
        };
        let variables = BTreeMap::from([("__SOURCE_0_SECRET".into(), "private-token".into())]);
        let mut secrets = BTreeMap::new();
        let workflow =
            checkout_workflow(&snapshot, &test_job("build", &[]), &variables, &mut secrets)
                .unwrap();
        assert!(!workflow.contains("private-token"));
        let workflow: Value = serde_json::from_str(&workflow).unwrap();
        let steps = workflow["jobs"]["build"]["steps"].as_array().unwrap();
        assert_eq!(steps[0]["id"], "before");
        assert_eq!(steps[1]["id"], "source");
        assert_eq!(steps[1]["if"], "success()");
        assert!(steps[1].get("uses").is_none());
        assert_eq!(steps[2]["id"], "after");
        assert_eq!(secrets["__SOURCE_0_SECRET"], "private-token");
    }

    #[test]
    fn checkout_credential_helper_handles_spaces_and_quotes_in_workspace() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace with spaces and ' quote");
        std::fs::create_dir(&workspace).unwrap();
        let script = checkout_node_script(&crate::pipelines::actions::CheckoutBinding::default());
        let auth = script.split_once("git init -q\n").unwrap().0;
        let script = format!(
            "{auth}\nprintf 'protocol=https\\nhost=fixture.invalid\\n\\n' | git -c core.askPass= credential fill\n"
        );
        let mut command = std::process::Command::new("sh");
        command
            .args(["-c", &script])
            .current_dir(&workspace)
            .env("SOURCE_KIND", "password")
            .env("SOURCE_USER", "fixture-user")
            .env("SOURCE_SECRET", "fixture-password");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = String::from_utf8(output.stdout).unwrap();
        assert!(output.contains("username=fixture-user"));
        assert!(output.contains("password=fixture-password"));
        assert!(std::fs::read_dir(&workspace).unwrap().next().is_none());
    }

    #[test]
    fn checkout_scripts_fetch_independent_sources_and_reject_symlink_escape() {
        use std::process::Command;
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(workspace.join("before-checkout"), "preserved").unwrap();
        for name in ["first", "second"] {
            let repo = root.path().join(name);
            std::fs::create_dir(&repo).unwrap();
            std::fs::write(repo.join("identity"), name).unwrap();
            for args in [
                vec!["init", "-q"],
                vec!["add", "identity"],
                vec![
                    "-c",
                    "user.name=fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "-qm",
                    "fixture",
                ],
            ] {
                assert!(
                    Command::new("git")
                        .current_dir(&repo)
                        .args(args)
                        .status()
                        .unwrap()
                        .success()
                );
            }
            let binding = crate::pipelines::actions::CheckoutBinding {
                repository_url: repo.to_string_lossy().into(),
                path: name.into(),
                ..Default::default()
            };
            let output = Command::new("sh")
                .arg("-c")
                .arg(checkout_node_script(&binding))
                .current_dir(&workspace)
                .env("SOURCE_KIND", "")
                .env("SOURCE_SECRET", "")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                std::fs::read_to_string(workspace.join(name).join("identity")).unwrap(),
                name
            );
        }
        assert_eq!(
            std::fs::read_to_string(workspace.join("before-checkout")).unwrap(),
            "preserved"
        );
        // An absolute (persistent) directory holding unrelated files is refused
        // and left untouched; once it is a checkout, later runs refresh it.
        let persistent = root.path().join("persistent");
        std::fs::create_dir(&persistent).unwrap();
        std::fs::write(persistent.join("keep"), "preserved").unwrap();
        let absolute = |path: &std::path::Path| crate::pipelines::actions::CheckoutBinding {
            repository_url: root.path().join("first").to_string_lossy().into(),
            path: path.to_string_lossy().replace('\\', "/"),
            ..Default::default()
        };
        let run = |binding: &crate::pipelines::actions::CheckoutBinding| {
            Command::new("sh")
                .arg("-c")
                .arg(checkout_node_script(binding))
                .current_dir(&workspace)
                .env("SOURCE_KIND", "")
                .env("SOURCE_SECRET", "")
                .output()
                .unwrap()
        };
        let refused = run(&absolute(&persistent));
        assert!(!refused.status.success());
        assert!(String::from_utf8_lossy(&refused.stderr).contains("not empty"));
        assert_eq!(
            std::fs::read_to_string(persistent.join("keep")).unwrap(),
            "preserved"
        );
        assert!(!persistent.join(".git").exists());
        let reused = root.path().join("reused");
        for _ in 0..2 {
            let output = run(&absolute(&reused));
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        assert_eq!(
            std::fs::read_to_string(reused.join("identity")).unwrap(),
            "first"
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.path(), workspace.join("outside")).unwrap();
            let binding = crate::pipelines::actions::CheckoutBinding {
                repository_url: root.path().join("first").to_string_lossy().into(),
                path: "outside".into(),
                ..Default::default()
            };
            let output = Command::new("sh")
                .arg("-c")
                .arg(checkout_node_script(&binding))
                .current_dir(&workspace)
                .env("SOURCE_KIND", "")
                .env("SOURCE_SECRET", "")
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("escapes workspace"));
        }
    }
}
