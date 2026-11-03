//! A dedicated, editable Actions workflow. Repository scripts never receive AI credentials.
use super::*;

pub(super) fn document(c: &Config) -> Result<crate::pipelines::actions::Document> {
    let mut steps = vec![json!({"uses":"actions/checkout@v4","id":"source"})];
    if c.ai_enabled {
        // Capture immutable Git objects before executing any repository-controlled tests.
        steps.push(json!({"name":"Capture review input","shell":"bash","env":{
        "REVIEW_BASE":"${{ inputs.review_base }}","REVIEW_HEAD":"${{ inputs.review_head }}",
        "REVIEW_FULL":"${{ inputs.review_full }}","REVIEW_CONFIG":encode(&json!({"include":c.include,"exclude":c.exclude,"max_files":c.max_files,"max_bytes":c.max_bytes}))?
    },"run":format!("REVIEW_PYTHON=\nfor candidate in python3 python; do\n  if [ \"$(\"$candidate\" -c 'import sys; sys.stdout.write(str(sys.version_info[0]))' 2>/dev/null)\" = 3 ]; then\n    REVIEW_PYTHON=\"$candidate\"\n    break\n  fi\ndone\nif [ -z \"$REVIEW_PYTHON\" ]; then\n  echo 'Python 3 is required on this Runner for AI review input capture.' >&2\n  exit 1\nfi\n\"$REVIEW_PYTHON\" - <<'REVIEW_PY'\n{}\nREVIEW_PY", include_str!("capture.py"))}));
        // Persist source data before a repository test can mutate the working tree.
        steps.push(json!({"uses":"actions/upload-artifact@v4","with":{"name":"review-input","path":"review-input.json","if-no-files-found":"error"}}));
    }
    if c.tests_enabled {
        steps.push(json!({"id":"repository_tests","name":"Repository tests","run":c.test_command}));
        if !c.junit_path.is_empty() {
            steps.push(json!({"uses":"actions/upload-artifact@v4","if":"always()","with":{"name":"review-junit","path":c.junit_path,"if-no-files-found":"error"}}));
        }
    }
    let labels = if c.runner_labels.is_empty() {
        vec!["self-hosted".to_string()]
    } else {
        c.runner_labels.clone()
    };
    let workflow = json!({"name":"Code review","on":{"workflow_dispatch":{"inputs":{
        "review_base":{"type":"string","required":true},"review_head":{"type":"string","required":true},"review_full":{"type":"string","default":"false"},"review_job_id":{"type":"string","required":true}
    }}},"jobs":{"review":{"runs-on":labels,"steps":steps}}});
    let d = crate::pipelines::actions::Document {
        engine: crate::pipelines::actions::ENGINE.into(),
        workflow: serde_yaml_ng::to_string(&workflow).map_err(|_| bad("template"))?,
        executor_bindings: vec![crate::pipelines::actions::Binding {
            job_id: "review".into(),
            target_kind: c.runner_kind.clone(),
            target_ref: c.runner_ref.clone(),
            image: "host".into(),
        }],
        checkout_bindings: vec![crate::pipelines::actions::CheckoutBinding {
            job_id: "review".into(),
            step_id: "source".into(),
            connection_id: if c.repositories[0].repository.is_empty() {
                String::new()
            } else {
                c.repositories[0].connection_id.clone()
            },
            repository_url: c.repositories[0].repository.clone(),
            git_ref: String::new(),
            path: String::new(),
        }],
        graph_layout: json!({}),
    };
    crate::pipelines::actions::parse(&d)?;
    Ok(d)
}
