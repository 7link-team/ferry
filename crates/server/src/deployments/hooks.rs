//! Bounded lifecycle commands, rendered with the instance's existing blueprint context.
use super::{
    events::{self, Kind, NewEvent},
    *,
};
use crate::blueprint::template::{self, Context, Phase};

pub(crate) const HOOK_TIMEOUT_SECS: u64 = 120;
pub(crate) const HEALTHY_ATTEMPT_SECS: u64 = 15;

#[derive(Clone, Copy)]
pub(crate) enum Hook {
    Join,
    Leave,
    Healthy,
}
impl Hook {
    pub fn name(self) -> &'static str {
        match self {
            Self::Join => "join",
            Self::Leave => "leave",
            Self::Healthy => "healthy",
        }
    }
}

pub(crate) fn hook_command(
    app: &apps::Model,
    instance: &str,
    argv: &[String],
    ctx: &Context,
    timeout_secs: u64,
) -> Result<String> {
    let own = ctx
        .instance
        .as_ref()
        .filter(|own| own.name == instance)
        .ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("blueprint.bindingInvalid"),
            )
        })?;
    if argv.is_empty() || timeout_secs == 0 {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("blueprint.hookInvalid")
                .with("role", &own.role)
                .with("hook", "command"),
        ));
    }
    let args = argv
        .iter()
        .map(|arg| template::render_text(arg, ctx, Phase::Instance).map(|arg| quote(&arg)))
        .collect::<Result<Vec<_>>>()?
        .join(" ");
    match app.mode.as_str() {
        "docker" => Ok(format!(
            "timeout {timeout_secs} docker exec {} {args}",
            quote(&format!("{}-{instance}", project(app)))
        )),
        "native" => Ok(format!(
            "cd {} && FERRY_INSTANCE={} FERRY_ROLE={} timeout {timeout_secs} {args}",
            quote(&directory(app)),
            quote(instance),
            quote(&own.role)
        )),
        _ => Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("deployment.replaceUnsupportedDriver"),
        )),
    }
}

pub(crate) struct HookRun {
    pub code: i32,
}

pub(crate) async fn run(
    s: &AppState,
    app: &apps::Model,
    host: Id,
    instance: &str,
    hook: Hook,
    argv: &[String],
    timeout_secs: u64,
    operation: Id,
) -> Result<HookRun> {
    let plans = groups::compiled(app)?;
    let ctx = plans
        .iter()
        .find(|plan| plan.host_id == host)
        .and_then(|plan| plan.contexts.get(instance))
        .ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("blueprint.bindingInvalid"),
            )
        })?;
    let command = hook_command(app, instance, argv, ctx, timeout_secs)?;
    let current = revisions::Entity::find_by_id(app.current_revision)
        .filter(revisions::Column::WorkspaceId.eq(app.workspace_id))
        .filter(revisions::Column::DeploymentId.eq(app.id))
        .one(&s.db)
        .await?;
    // Missing or malformed snapshots cannot make output less protected.
    let protected = match &current {
        Some(rev) => {
            rev.variable_snapshot
                .as_ref()
                .is_some_and(|v| !v.is_empty())
                || artifact_bindings(&rev.artifact).map_or(true, |v| v != json!([]))
        }
        None => true,
    };
    let result = async {
        let channel = remote::open(s, &app.workspace_id.into(), &host).await?;
        channel
            .exec(&command, Duration::from_secs(timeout_secs + 5))
            .await
    }
    .await;
    // A transport failure can contain credentials in an upstream error; retain no raw error.
    let out = result.unwrap_or(crate::hosts::channel::Output {
        code: -1,
        stdout: String::new(),
        stderr: String::new(),
    });
    let mut output = StoredText::from_stored(&execution_output(
        &out,
        protected
            .then(|| Message::new("deployment.output.exitCode").with("code", out.code.to_string())),
    ));
    output.truncate_raw(8 * 1024);
    events::record(s, &s.db, NewEvent {
        workspace: app.workspace_id.into(), deployment: app.id.into(), kind: Kind::HookOutput,
        instance: instance.into(), host, revision: app.current_revision.into(),
        message: Message::new("deployment.event.hookOutput").with("hook", hook.name()).with("instance", instance).with("code", out.code.to_string()),
        data: json!({"hook": hook.name(), "code": out.code, "output": output.stored(), "operation_id": operation}),
    }).await?;
    Ok(HookRun { code: out.code })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blueprint::template::Instance;
    #[tokio::test]
    async fn hook_command_docker_and_native() {
        let (_dir, s) = super::super::blueprint_tests::fixture(&[3001, 3002]).await;
        let id = super::super::blueprint_tests::create_app(
            &s,
            super::super::blueprint_tests::body(&[3001, 3002]),
        )
        .await
        .unwrap();
        let mut app = super::super::blueprint_tests::row(&s, &id).await;
        app.id = 9;
        app.workdir = "/srv/with space".into();
        let ctx = Context {
            instance: Some(Instance {
                name: "nats-1".into(),
                role: "nats".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let args = vec![
            "echo".into(),
            "{{ .Instance.Name }}".into(),
            "quote ' and ; $(bad)".into(),
        ];
        let cmd = hook_command(&app, "nats-1", &args, &ctx, 120).unwrap();
        assert_eq!(
            cmd,
            r"timeout 120 docker exec 'deployment-1-9-nats-1' 'echo' 'nats-1' 'quote '\'' and ; $(bad)'"
        );
        app.mode = "native".into();
        assert!(hook_command(&app, "nats-1", &args, &ctx, 15).unwrap().starts_with("cd '/srv/with space/1/9' && FERRY_INSTANCE='nats-1' FERRY_ROLE='nats' timeout 15 "));
        assert!(hook_command(&app, "nats-2", &args, &ctx, 15).is_err());
        assert!(hook_command(&app, "nats-1", &[], &ctx, 15).is_err());
    }
}
