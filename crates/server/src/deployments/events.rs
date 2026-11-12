//! Failover timeline: append-only events per deployment instance, and the
//! per-instance history the coordinator derives from them.
use super::*;
use crate::entities::deployment_events as events;
use sea_orm::ConnectionTrait;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    HostSuspected,
    HostConfirmedDown,
    PeerVeto,
    Fenced,
    ReplaceStarted,
    ReplaceSucceeded,
    ReplaceFailed,
    HookOutput,
    NeedsAttention,
    Unfenced,
    AlertOnly,
    /// The user acknowledged an instance that needed attention; resets it.
    Acknowledged,
}

impl Kind {
    pub(crate) const ALL: [Kind; 12] = [
        Kind::HostSuspected,
        Kind::HostConfirmedDown,
        Kind::PeerVeto,
        Kind::Fenced,
        Kind::ReplaceStarted,
        Kind::ReplaceSucceeded,
        Kind::ReplaceFailed,
        Kind::HookOutput,
        Kind::NeedsAttention,
        Kind::Unfenced,
        Kind::AlertOnly,
        Kind::Acknowledged,
    ];
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Kind::HostSuspected => "hostSuspected",
            Kind::HostConfirmedDown => "hostConfirmedDown",
            Kind::PeerVeto => "peerVeto",
            Kind::Fenced => "fenced",
            Kind::ReplaceStarted => "replaceStarted",
            Kind::ReplaceSucceeded => "replaceSucceeded",
            Kind::ReplaceFailed => "replaceFailed",
            Kind::HookOutput => "hookOutput",
            Kind::NeedsAttention => "needsAttention",
            Kind::Unfenced => "unfenced",
            Kind::AlertOnly => "alertOnly",
            Kind::Acknowledged => "acknowledged",
        }
    }
}

pub(crate) struct NewEvent {
    pub workspace: Id,
    pub deployment: Id,
    pub kind: Kind,
    pub instance: String,
    pub host: Id,
    pub revision: Id,
    pub message: Message,
    pub data: Value,
}

/// detail = {"message": StoredText::from_message(message).stored(), ...data}
pub(crate) async fn record(
    s: &AppState,
    conn: &impl ConnectionTrait,
    event: NewEvent,
) -> Result<Id> {
    let mut detail = match event.data {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        other => serde_json::Map::from_iter([("data".to_owned(), other)]),
    };
    detail.insert(
        "message".into(),
        Value::String(StoredText::from_message(event.message).stored()),
    );
    let id = s.next_id()?;
    events::Entity::insert(events::ActiveModel {
        id: sea_orm::Set(id.into()),
        workspace_id: sea_orm::Set(event.workspace.into()),
        deployment_id: sea_orm::Set(event.deployment.into()),
        kind: sea_orm::Set(event.kind.as_str().into()),
        instance: sea_orm::Set(event.instance),
        host_id: sea_orm::Set(event.host.into()),
        detail: sea_orm::Set(Value::Object(detail).to_string()),
        revision_id: sea_orm::Set(event.revision.into()),
        created_at: sea_orm::Set(db::now()),
    })
    .exec(conn)
    .await?;
    Ok(id)
}

/// Minimum gap between two automatic replacements of one instance.
pub(crate) const COOLDOWN_MS: i64 = 600_000;
/// Consecutive failed replacements before an instance needs attention.
pub(crate) const MAX_FAILURES: u32 = 3;

#[derive(Debug, Default, PartialEq)]
pub(crate) struct InstanceHistory {
    pub last_started: Option<i64>,
    pub failures: u32,
    pub attention: bool,
    pub open: bool,
}

/// Events of one instance in ascending order → its state. `failures` counts
/// ReplaceFailed after the latest ReplaceSucceeded/Acknowledged; `attention`
/// is a NeedsAttention newer than the latest Acknowledged; `open` is a
/// ReplaceStarted not yet followed by Succeeded/Failed.
pub(crate) fn summarize(events: &[events::Model]) -> InstanceHistory {
    let is = |event: &events::Model, kind: Kind| event.kind == kind.as_str();
    let mut history = InstanceHistory::default();
    for event in events {
        if is(event, Kind::ReplaceStarted) {
            history.last_started = Some(event.created_at);
            history.open = true;
        } else if is(event, Kind::ReplaceSucceeded) {
            history.open = false;
            history.failures = 0;
        } else if is(event, Kind::ReplaceFailed) {
            history.open = false;
            history.failures += 1;
        } else if is(event, Kind::NeedsAttention) {
            history.attention = true;
        } else if is(event, Kind::Acknowledged) {
            history.attention = false;
            history.failures = 0;
        }
    }
    history
}

pub(crate) fn in_cooldown(h: &InstanceHistory, now_ms: i64) -> bool {
    h.last_started
        .is_some_and(|started| now_ms.saturating_sub(started) < COOLDOWN_MS)
}

/// Only the kinds `summarize` reads, oldest first; ties keep insertion order.
pub(crate) async fn history(
    conn: &impl ConnectionTrait,
    workspace: Id,
    deployment: Id,
    instance: &str,
) -> Result<InstanceHistory> {
    let kinds = [
        Kind::ReplaceStarted,
        Kind::ReplaceSucceeded,
        Kind::ReplaceFailed,
        Kind::NeedsAttention,
        Kind::Acknowledged,
    ]
    .map(Kind::as_str);
    let rows = events::Entity::find()
        .filter(events::Column::WorkspaceId.eq(workspace))
        .filter(events::Column::DeploymentId.eq(deployment))
        .filter(events::Column::Instance.eq(instance))
        .filter(events::Column::Kind.is_in(kinds))
        .order_by_asc(events::Column::CreatedAt)
        .order_by_asc(events::Column::Id)
        .all(conn)
        .await?;
    Ok(summarize(&rows))
}

const EVENT_SPEC: Spec = Spec {
    search: &[],
    filters: &["deployment_id", "kind", "instance"],
    sortable: &["created_at"],
    default_sort: "-created_at",
};

/// The stored message renders in the reader's language; `detail` keeps the rest.
fn event_json(row: &events::Model) -> Value {
    let mut detail = match serde_json::from_str::<Value>(&row.detail) {
        Ok(Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    };
    let message = match detail.remove("message") {
        Some(Value::String(stored)) => {
            crate::i18n::render_text_stored(crate::i18n::current_locale(), &stored)
        }
        _ => String::new(),
    };
    json!({"id": Id::from(row.id), "kind": row.kind, "instance": row.instance,
        "host_id": Id::from(row.host_id), "revision_id": Id::from(row.revision_id),
        "created_at": row.created_at, "message": message, "detail": detail})
}

/// Failover timeline of one deployment, newest first; `message` renders in the reader's language.
pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Query(mut raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("deployments.read")?;
    let app = scoped(&s.db, &scope, &id).await?;
    raw.insert("deployment_id".into(), app.id.to_string());
    let q = ListQuery::parse(&EVENT_SPEC, &raw)?;
    let (rows, total) = resource::list(
        &s.db,
        &EVENT_SPEC,
        events::Entity::find(),
        &scope.workspace_id,
        &q,
    )
    .await?;
    let mut items: Vec<Value> = rows.iter().map(event_json).collect();
    crate::resources::names::attach(
        &s.db,
        scope.workspace_id,
        &mut items,
        &[crate::resources::names::of(
            "host_id",
            "host_name",
            crate::resources::names::Kind::Host,
        )],
    )
    .await?;
    Ok(Json(
        json!({"items":items,"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: i64, kind: Kind, at: i64) -> events::Model {
        events::Model {
            id,
            workspace_id: 1,
            deployment_id: 2,
            kind: kind.as_str().into(),
            instance: "db-0".into(),
            host_id: 3,
            detail: "{}".into(),
            revision_id: 0,
            created_at: at,
        }
    }

    #[test]
    fn summarize_cooldown_failures_and_ack() {
        use Kind::*;
        let t0 = 1_000_000;
        let minute = 60_000;
        let mut log: Vec<events::Model> = [
            ReplaceStarted,
            ReplaceFailed,
            ReplaceStarted,
            ReplaceFailed,
            ReplaceStarted,
            ReplaceFailed,
            NeedsAttention,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, kind)| event(i as i64 + 1, kind, t0 + i as i64 * minute))
        .collect();
        let t_last = t0 + 4 * minute;
        let h = summarize(&log);
        assert_eq!(
            h,
            InstanceHistory {
                last_started: Some(t_last),
                failures: MAX_FAILURES,
                attention: true,
                open: false,
            }
        );
        assert!(in_cooldown(&h, t_last + 9 * minute));
        assert!(!in_cooldown(&h, t_last + 11 * minute));

        let at = t0 + 7 * minute;
        log.push(event(8, Acknowledged, at));
        let h = summarize(&log);
        assert_eq!((h.failures, h.attention, h.open), (0, false, false));

        // Same timestamp as the acknowledgement: order, not time, decides.
        log.push(event(9, ReplaceStarted, at));
        let h = summarize(&log);
        assert!(h.open);
        assert_eq!(h.last_started, Some(at));

        log.push(event(10, ReplaceFailed, at + minute));
        assert_eq!(summarize(&log).failures, 1);
        log.push(event(11, ReplaceStarted, at + 2 * minute));
        log.push(event(12, ReplaceSucceeded, at + 3 * minute));
        let h = summarize(&log);
        assert!(!h.open);
        assert_eq!((h.failures, h.attention), (0, false));
        assert!(!in_cooldown(&InstanceHistory::default(), at));
    }

    #[test]
    fn kinds_are_camel_case_with_messages_in_every_locale() {
        assert_eq!(Kind::HostConfirmedDown.as_str(), "hostConfirmedDown");
        assert_eq!(Kind::Acknowledged.as_str(), "acknowledged");
        let keys = Kind::ALL
            .iter()
            .map(|kind| format!("deployment.event.{}", kind.as_str()))
            .chain(["deployment.event.hostConfirmedDownSingleSource".to_owned()])
            .chain(
                [
                    "noCandidate",
                    "quorumLost",
                    "hookFailed",
                    "revisionFailed",
                    "unhealthy",
                    "interrupted",
                    "actorRevoked",
                    "driverUnsupported",
                    "needsBackup",
                    "modeAlert",
                    "tooManyFailures",
                ]
                .map(|reason| format!("deployment.failoverReason.{reason}")),
            );
        for key in keys {
            for locale in crate::i18n::Locale::ALL {
                let text = crate::i18n::render(locale, &Message::new(key.clone()));
                assert_ne!(text, key, "{locale:?} lacks {key}");
            }
        }
    }
}
