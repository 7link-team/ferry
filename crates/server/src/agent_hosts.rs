//! Which Runner hosts may run an Agent's CLI, in what order, and why each one
//! could not. Conversations and workflow Agent nodes share this, so both pick
//! hosts the same way and explain a failure the same way.
use crate::{
    entities::{ai_agents, hosts},
    error::{ApiError, Result},
    i18n::Message,
    snowflake::Id,
};
use axum::http::StatusCode;
use ferry_protocol::agent::{Binding, Probe, Runtime};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

/// Online Runners considered when a CLI model names no hosts.
const AUTOMATIC_LIMIT: u64 = 100;
/// Hosts listed by name in one error; the rest are counted.
const LISTED_REASONS: usize = 8;

/// Hosts to try, in order: an explicitly requested host, then the host still
/// pinned on a legacy Agent, then the CLI model's hosts in order, and — only
/// when the model names none — every online Runner by id. Named hosts are kept
/// even when offline so the caller can say so; deleted ones are skipped.
pub(crate) async fn candidates(
    conn: &impl ConnectionTrait,
    workspace: Id,
    requested: Option<i64>,
    agent: &ai_agents::Model,
    exclude: &[i64],
) -> Result<Vec<hosts::Model>> {
    let preferred = crate::models::cli_hosts(conn, workspace, agent).await?;
    let mut named = vec![];
    named.extend(requested.filter(|id| *id > 0));
    named.extend((agent.host_id > 0).then_some(agent.host_id));
    named.extend(preferred.iter().copied());
    let mut rows: Vec<hosts::Model> = vec![];
    for id in named {
        if exclude.contains(&id) || rows.iter().any(|row| row.id == id) {
            continue;
        }
        if let Some(row) = runners(workspace)
            .filter(hosts::Column::Id.eq(id))
            .one(conn)
            .await?
        {
            rows.push(row);
        }
    }
    if preferred.is_empty() {
        for row in runners(workspace)
            .filter(hosts::Column::State.eq("online"))
            .order_by_asc(hosts::Column::Id)
            .limit(AUTOMATIC_LIMIT)
            .all(conn)
            .await?
        {
            if !exclude.contains(&row.id) && !rows.iter().any(|known| known.id == row.id) {
                rows.push(row);
            }
        }
    }
    Ok(rows)
}

fn runners(workspace: Id) -> sea_orm::Select<hosts::Entity> {
    hosts::Entity::find()
        .filter(hosts::Column::WorkspaceId.eq(workspace))
        .filter(
            hosts::Column::Connection.is_in(crate::models::runner_connections().iter().copied()),
        )
}

/// Why one host could not run the CLI.
pub(crate) fn reason(key: &'static str, cli: &str) -> Message {
    Message::new(key).with("cli", cli.to_owned())
}

/// The reason behind an error opening or probing a host, for the host list.
pub(crate) fn reason_from(error: &ApiError) -> Message {
    match &error.2 {
        Some(message) => Message::new("models.reasonError").with_message("detail", message.clone()),
        None => Message::new("models.reasonError").with("detail", error.1.clone()),
    }
}

/// Every host tried and why it could not run the CLI.
#[derive(Default)]
pub(crate) struct Reasons(Vec<Message>);

impl Reasons {
    pub(crate) fn push(&mut self, host: &hosts::Model, reason: Message) {
        self.push_named(host.name.clone(), reason);
    }
    pub(crate) fn push_named(&mut self, host: String, reason: Message) {
        self.0.push(
            Message::new("models.hostReason")
                .with("host", host)
                .with_message("reason", reason),
        );
    }
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
    /// After the bound host failed: it is named first with its own reason, then
    /// every other candidate. A bound host that is merely offline is said so.
    pub(crate) fn into_failover_error(self, host: String, reason: Message) -> ApiError {
        let offline = matches!(
            reason.key.as_str(),
            "models.reasonOffline" | "models.reasonUnreachable"
        );
        if !offline {
            let mut all = Reasons::default();
            all.push_named(host, reason);
            all.0.extend(self.0);
            return all.into_error();
        }
        let message = if self.0.is_empty() {
            Message::new("conversations.boundHostOfflineNoOthers")
        } else {
            Message::new("conversations.boundHostOffline").with_message("others", self.chain())
        };
        ApiError::localized(
            StatusCode::CONFLICT,
            message.with("host", host).with_message("reason", reason),
        )
    }
    /// One error naming each host and its reason, or "no Runner" when none was tried.
    pub(crate) fn into_error(self) -> ApiError {
        if self.0.is_empty() {
            return ApiError::localized(StatusCode::CONFLICT, Message::new("models.noRunner"));
        }
        ApiError::localized(
            StatusCode::CONFLICT,
            Message::new("models.noHost").with_message("reasons", self.chain()),
        )
    }
    /// The listed reasons as one message, capped with a "and N more" tail.
    fn chain(self) -> Message {
        let total = self.0.len();
        let mut listed: Vec<Message> = self.0.into_iter().take(LISTED_REASONS).collect();
        if total > LISTED_REASONS {
            listed.push(
                Message::new("models.hostReasonMore")
                    .with("count", (total - LISTED_REASONS).to_string()),
            );
        }
        let mut chain = listed.pop().expect("at least one reason");
        while let Some(previous) = listed.pop() {
            chain = Message::new("models.hostReasonList")
                .with_message("first", previous)
                .with_message("rest", chain);
        }
        chain
    }
}

/// The probe answer checked for running `runtime`: a found, working CLI, the
/// directory, platform tools and a usable login. `login_usable` decides
/// whether the reported login state is enough (a custom upstream replaces it).
pub(crate) fn check(
    probe: Probe,
    cli: &str,
    login_usable: impl Fn(&str) -> bool,
) -> std::result::Result<Binding, Message> {
    let Some(directory) = probe
        .canonical_directory
        .clone()
        .filter(|_| probe.directory_exists)
    else {
        return Err(Message::new("models.reasonDirectory"));
    };
    let Some(executable) = probe.executable.clone().filter(|_| probe.error.is_none()) else {
        return Err(cli_problem(
            probe.error.as_deref().unwrap_or_default(),
            cli,
            &probe.execution_user,
        ));
    };
    if !probe.platform_tools
        || probe.execution_user.is_empty()
        || probe.execution_identity.is_empty()
    {
        return Err(Message::new("models.reasonRunnerOld"));
    }
    if !login_usable(&probe.authentication) {
        return Err(reason("models.reasonLogin", cli));
    }
    Ok(Binding {
        execution_user: probe.execution_user,
        execution_identity: probe.execution_identity,
        canonical_directory: directory,
        executable,
    })
}

/// Why a CLI cannot run, from the Runner's probe answer. Runners send
/// "missing" (not on the execution user's PATH), "failed" or
/// "failed: <first output line>" (`--version` did not succeed); older Runners
/// send only the bare words, and anything else is passed on as detail.
pub(crate) fn cli_problem(error: &str, cli: &str, user: &str) -> Message {
    let error = error.trim();
    // New Runners name where they looked: "missing: <places>".
    if let Some(places) = error.strip_prefix("missing:") {
        let places = truncate(places);
        if !places.is_empty() {
            return reason("models.reasonCliMissingSearched", cli)
                .with("user", truncate(user))
                .with("places", places);
        }
    }
    if error.is_empty() || error == "missing" || error == "missing:" {
        return if user.is_empty() {
            reason("models.reasonCliMissing", cli)
        } else {
            reason("models.reasonCliMissingUser", cli).with("user", truncate(user))
        };
    }
    if error == "failed" {
        return reason("models.reasonCliBroken", cli);
    }
    if let Some(detail) = error.strip_prefix("failed:") {
        let detail = truncate(detail);
        return if detail.is_empty() {
            reason("models.reasonCliBroken", cli)
        } else {
            reason("models.reasonCliBrokenDetail", cli).with("detail", detail)
        };
    }
    reason("models.reasonCliFailed", cli).with("detail", truncate(error))
}

/// Runner-reported detail, bounded and on one line.
fn truncate(detail: &str) -> String {
    let line = detail.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut end = line.len().min(300);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    line[..end].to_owned()
}

/// Runtime name shown in reasons.
pub(crate) fn cli_name(runtime: Runtime) -> String {
    serde_json::to_value(runtime)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Locale, render};

    fn host(id: i64, name: &str) -> hosts::Model {
        crate::hosts::fake::host_row(1, id, name, "online")
    }

    /// Stored errors are re-rendered per reader only when every part is a
    /// known message with exactly its template's parameters.
    #[test]
    fn every_reason_is_stored_as_a_translatable_message() {
        let unreachable = ApiError::localized(
            StatusCode::BAD_GATEWAY,
            Message::new("ssh.unreachable")
                .with("address", "10.0.0.1")
                .with("detail", "refused"),
        );
        let reasons = [
            Message::new("models.reasonOffline"),
            Message::new("models.reasonUnreachable")
                .with_message("detail", reason_from(&unreachable)),
            reason_from(&ApiError::bad("plain text")),
            Message::new("models.reasonDirectory"),
            Message::new("models.reasonRunnerOld"),
            Message::new("models.reasonNotRunner"),
            reason("models.reasonCliMissing", "codex"),
            cli_problem("missing", "codex", "SYSTEM"),
            cli_problem("failed", "codex", "SYSTEM"),
            cli_problem("failed: node: not found", "codex", "SYSTEM"),
            reason("models.reasonCliFailed", "codex").with("detail", "unexpected"),
            reason("models.reasonLogin", "codex"),
            Message::new("conversations.bindingChanged"),
        ];
        let mut all = Reasons::default();
        for (id, reason) in reasons.into_iter().enumerate() {
            let one = Message::new("conversations.hostUnavailable")
                .with("host", "alpha")
                .with_message("reason", reason.clone());
            let stored = crate::i18n::store_message(&one);
            assert!(
                crate::i18n::decode_stored(&stored).is_some(),
                "{}",
                reason.key
            );
            all.push(&host(id as i64, "h"), reason);
        }
        let stored = all.into_error().stored();
        assert_eq!(
            crate::i18n::decode_stored(&stored).unwrap().key,
            "models.noHost"
        );
        assert!(crate::i18n::decode_stored(&Reasons::default().into_error().stored()).is_some());
    }

    #[test]
    fn every_host_and_its_reason_is_named_in_order() {
        let mut reasons = Reasons::default();
        reasons.push(
            &host(1, "alpha"),
            reason("models.reasonCliMissing", "codex"),
        );
        reasons.push(&host(2, "beta"), reason("models.reasonLogin", "codex"));
        reasons.push(&host(3, "gamma"), Message::new("models.reasonOffline"));
        let text = render(Locale::EnUs, reasons.into_error().2.as_ref().unwrap());
        let (a, b, c) = (
            text.find("alpha").unwrap(),
            text.find("beta").unwrap(),
            text.find("gamma").unwrap(),
        );
        assert!(a < b && b < c, "{text}");
        assert!(text.contains("codex"), "{text}");
        assert!(!text.contains('{'), "{text}");
        let zh = render(Locale::ZhCn, &Reasons::default().into_error().2.unwrap());
        assert!(!zh.contains("models."), "{zh}");
    }

    #[test]
    fn many_hosts_are_capped_and_counted() {
        let mut reasons = Reasons::default();
        for id in 0..12 {
            reasons.push(
                &host(id, &format!("h{id:02}")),
                Message::new("models.reasonOffline"),
            );
        }
        let text = render(Locale::EnUs, reasons.into_error().2.as_ref().unwrap());
        assert!(text.contains("h07") && !text.contains("h08"), "{text}");
        assert!(text.contains('4'), "{text}");
        assert!(!text.contains('{'), "{text}");
    }

    #[test]
    fn cli_problems_name_the_execution_user_and_the_failure_for_new_and_old_runners() {
        // Old Runners send bare words; new ones add the first output line.
        let missing = cli_problem("missing", "codex", "SYSTEM");
        assert_eq!(missing.key, "models.reasonCliMissingUser");
        assert!(render(Locale::ZhCn, &missing).contains("SYSTEM"));
        assert_eq!(
            cli_problem("missing", "codex", "").key,
            "models.reasonCliMissing"
        );
        assert_eq!(
            cli_problem("", "codex", "u").key,
            "models.reasonCliMissingUser"
        );
        let broken = cli_problem("failed", "codex", "u");
        assert_eq!(broken.key, "models.reasonCliBroken");
        let detailed = cli_problem("failed: env: node: No such file\n", "codex", "u");
        assert_eq!(detailed.key, "models.reasonCliBrokenDetail");
        assert_eq!(detailed.params["detail"], "env: node: No such file");
        assert_eq!(
            cli_problem("failed:  ", "codex", "u").key,
            "models.reasonCliBroken"
        );
        for message in [missing, broken, detailed] {
            for locale in [Locale::EnUs, Locale::ZhCn, Locale::JaJp] {
                let rendered = render(locale, &message);
                assert!(
                    !rendered.contains('{')
                        && !rendered.contains("models.")
                        && !rendered.contains("(failed")
                        && !rendered.contains("（failed"),
                    "{rendered}"
                );
            }
        }
    }

    #[test]
    fn a_failed_probe_names_what_is_wrong() {
        let usable = |_: &str| true;
        let mut probe = crate::hosts::fake::probe_answer(Runtime::Codex, "/w");
        probe.executable = None;
        probe.error = Some("codex: not found\n in PATH".into());
        let message = check(probe, "codex", usable).unwrap_err();
        assert_eq!(message.key, "models.reasonCliFailed");
        assert_eq!(message.params["detail"], "codex: not found in PATH");
        let mut probe = crate::hosts::fake::probe_answer(Runtime::Codex, "/w");
        probe.authentication = "logged_out".into();
        assert_eq!(
            check(probe, "codex", |state| state == "logged_in")
                .unwrap_err()
                .key,
            "models.reasonLogin"
        );
        let probe = crate::hosts::fake::probe_answer(Runtime::Codex, "/w");
        assert_eq!(
            check(probe, "codex", usable).unwrap().canonical_directory,
            "/w"
        );
    }

    #[test]
    fn a_failed_failover_names_the_bound_host_first_and_says_when_it_is_only_offline() {
        use crate::i18n::{Locale, render};
        let missing = cli_problem("missing", "codex", "aspeed");
        // Offline bound host, other candidates also unusable.
        let mut others = Reasons::default();
        others.push_named("Aspeeds-Mac-mini.local".into(), missing.clone());
        let error =
            others.into_failover_error("ASPEED".into(), Message::new("models.reasonOffline"));
        let message = crate::i18n::decode_stored(&error.stored()).unwrap();
        assert_eq!(message.key, "conversations.boundHostOffline");
        let text = render(Locale::ZhCn, &message);
        assert!(text.starts_with("绑定主机 ASPEED 当前离线"), "{text}");
        assert!(
            text.contains("Aspeeds-Mac-mini.local") && text.contains("aspeed"),
            "{text}"
        );
        // Offline bound host and nothing else to try.
        let alone = Reasons::default()
            .into_failover_error("ASPEED".into(), Message::new("models.reasonOffline"));
        assert_eq!(
            crate::i18n::decode_stored(&alone.stored()).unwrap().key,
            "conversations.boundHostOfflineNoOthers"
        );
        // Any other reason: the usual list, bound host first.
        let mut rest = Reasons::default();
        rest.push_named("Mac".into(), missing);
        let listed = rest.into_failover_error(
            "ASPEED".into(),
            cli_problem("failed: node: not found", "codex", "u"),
        );
        let message = crate::i18n::decode_stored(&listed.stored()).unwrap();
        assert_eq!(message.key, "models.noHost");
        let text = render(Locale::ZhCn, &message);
        assert!(
            text.find("ASPEED").unwrap() < text.find("Mac").unwrap(),
            "{text}"
        );
    }

    #[test]
    fn a_missing_cli_says_where_the_runner_looked() {
        use crate::i18n::{Locale, render};
        let searched = cli_problem(
            "missing: /opt/homebrew/bin, ~/.nvm/versions/node/*/bin",
            "codex",
            "aspeed",
        );
        assert_eq!(searched.key, "models.reasonCliMissingSearched");
        let text = render(Locale::ZhCn, &searched);
        assert!(
            text.contains("aspeed") && text.contains("/opt/homebrew/bin"),
            "{text}"
        );
        // An empty list falls back to the plain message.
        assert_eq!(
            cli_problem("missing:", "codex", "aspeed").key,
            "models.reasonCliMissingUser"
        );
    }
}
