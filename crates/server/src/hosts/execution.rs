use crate::{
    db,
    entities::{
        host_execution_policies as policies, host_execution_slots as slots, host_tags, hosts, tags,
    },
    error::{ApiError, Result},
    snowflake::Id,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set, sea_query::Expr};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub enabled: bool,
    pub shared: bool,
    pub capacity: u32,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            shared: false,
            capacity: 1,
        }
    }
}
impl Policy {
    pub fn validate(&self) -> Result<()> {
        if !(1..=64).contains(&self.capacity) {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("hosts.executionCapacityInvalid"),
            ));
        }
        Ok(())
    }
}
pub async fn save(
    conn: &impl ConnectionTrait,
    workspace: Id,
    host: Id,
    value: &Policy,
) -> Result<()> {
    value.validate()?;
    policies::Entity::insert(policies::ActiveModel {
        host_id: Set(host.into()),
        workspace_id: Set(workspace.into()),
        enabled: Set(value.enabled),
        shared: Set(value.shared),
        capacity: Set(value.capacity.into()),
    })
    .on_conflict(
        sea_orm::sea_query::OnConflict::column(policies::Column::HostId)
            .update_columns([
                policies::Column::Enabled,
                policies::Column::Shared,
                policies::Column::Capacity,
            ])
            .to_owned(),
    )
    .exec(conn)
    .await?;
    Ok(())
}
pub async fn decorate_many(
    conn: &impl ConnectionTrait,
    rows: &[hosts::Model],
    values: &mut [Value],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let policies = policies::Entity::find()
        .filter(policies::Column::HostId.is_in(ids.clone()))
        .all(conn)
        .await?
        .into_iter()
        .map(|p| {
            (
                p.host_id,
                Policy {
                    enabled: p.enabled,
                    shared: p.shared,
                    capacity: p.capacity as u32,
                },
            )
        })
        .collect::<HashMap<_, _>>();
    let live = slots::Entity::find()
        .filter(slots::Column::HostId.is_in(ids))
        .filter(slots::Column::ExpiresAt.gt(db::now()))
        .all(conn)
        .await?;
    for (row, value) in rows.iter().zip(values) {
        let p = policies.get(&row.id).cloned().unwrap_or_default();
        let live = live
            .iter()
            .filter(|lease| lease.host_id == row.id)
            .collect::<Vec<_>>();
        let state = if !p.enabled {
            if live.is_empty() {
                "disabled"
            } else {
                "draining"
            }
        } else if live.len() >= p.capacity as usize {
            "busy"
        } else {
            "available"
        };
        value["execution"] = json!(p);
        value["execution_status"] = json!({"active":live.len(),"capacity":p.capacity,"state":state,"running_jobs":live.iter().map(|l| json!({"run_id":Id::from(l.run_id),"job_key":l.job_key})).collect::<Vec<_>>()});
    }
    Ok(())
}
#[cfg(test)]
async fn decorate(
    conn: &impl ConnectionTrait,
    row: &hosts::Model,
    value: &mut Value,
) -> Result<()> {
    decorate_many(conn, std::slice::from_ref(row), std::slice::from_mut(value)).await
}
fn normalize_os(value: &str) -> &str {
    if value == "macos" { "darwin" } else { value }
}
fn normalize_arch(value: &str) -> &str {
    match value {
        "amd64" | "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    }
}
fn matches(host: &hosts::Model, labels: &[String], tags: &[String]) -> bool {
    labels.iter().all(|label| {
        let label = label.to_ascii_lowercase();
        match label.as_str() {
            "self-hosted" => true,
            "linux" | "windows" | "darwin" | "macos" => {
                normalize_os(&label) == normalize_os(&host.os.to_ascii_lowercase())
            }
            "x64" | "amd64" | "x86_64" | "arm64" | "aarch64" => {
                normalize_arch(&label) == normalize_arch(&host.arch.to_ascii_lowercase())
            }
            _ => tags.iter().any(|tag| tag.eq_ignore_ascii_case(&label)),
        }
    })
}
pub async fn candidates(
    conn: &impl ConnectionTrait,
    workspace: i64,
    host: i64,
    group: i64,
    pool: bool,
    labels: &[String],
) -> Result<Vec<(hosts::Model, Policy, usize)>> {
    let mut query = hosts::Entity::find().filter(hosts::Column::WorkspaceId.eq(workspace));
    if !pool && group == 0 {
        query = query.filter(hosts::Column::Id.eq(host));
    }
    if group > 0 {
        query = query.filter(hosts::Column::GroupId.eq(group));
    }
    let rows = query.all(conn).await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let policies = policies::Entity::find()
        .filter(policies::Column::HostId.is_in(ids.clone()))
        .all(conn)
        .await?
        .into_iter()
        .map(|p| {
            (
                p.host_id,
                Policy {
                    enabled: p.enabled,
                    shared: p.shared,
                    capacity: p.capacity as u32,
                },
            )
        })
        .collect::<HashMap<_, _>>();
    let live = slots::Entity::find()
        .filter(slots::Column::HostId.is_in(ids.clone()))
        .filter(slots::Column::ExpiresAt.gt(db::now()))
        .all(conn)
        .await?;
    let mut loads = HashMap::<i64, usize>::new();
    for lease in live {
        *loads.entry(lease.host_id).or_default() += 1;
    }
    let mut names = HashMap::<i64, Vec<String>>::new();
    if pool || group > 0 {
        let links = host_tags::Entity::find()
            .filter(host_tags::Column::HostId.is_in(ids))
            .all(conn)
            .await?;
        if !links.is_empty() {
            let tags = tags::Entity::find()
                .filter(tags::Column::WorkspaceId.eq(workspace))
                .filter(tags::Column::Id.is_in(links.iter().map(|l| l.tag_id)))
                .all(conn)
                .await?
                .into_iter()
                .map(|tag| (tag.id, tag.name))
                .collect::<HashMap<_, _>>();
            for link in links {
                if let Some(name) = tags.get(&link.tag_id) {
                    names.entry(link.host_id).or_default().push(name.clone());
                }
            }
        }
    }
    let mut available = Vec::new();
    for row in rows {
        let p = policies.get(&row.id).cloned().unwrap_or_default();
        if !p.enabled || (pool && !p.shared) || ((pool || group > 0) && row.state != "online") {
            continue;
        }
        if (pool || group > 0)
            && !matches(&row, labels, names.get(&row.id).map_or(&[], Vec::as_slice))
        {
            continue;
        }
        let occupied = *loads.get(&row.id).unwrap_or(&0);
        available.push((row, p, occupied));
    }
    available.sort_by_key(|(h, _, load)| (*load, h.id));
    Ok(available)
}
// Renew every 10s; five minutes leaves room for remote cancellation and cleanup
// before another process can reclaim a slot after a renewal failure.
const TTL: i64 = 300_000;
pub struct Lease {
    pub host: i64,
    slot: i64,
    owner: String,
}
impl Lease {
    pub async fn renew(&self, conn: &impl ConnectionTrait) -> Result<bool> {
        Ok(slots::Entity::update_many()
            .col_expr(slots::Column::ExpiresAt, Expr::value(db::now() + TTL))
            .filter(slots::Column::HostId.eq(self.host))
            .filter(slots::Column::Slot.eq(self.slot))
            .filter(slots::Column::Owner.eq(&self.owner))
            .filter(slots::Column::ExpiresAt.gt(db::now()))
            .exec(conn)
            .await?
            .rows_affected
            == 1)
    }
    pub async fn release(&self, conn: &impl ConnectionTrait) -> Result<()> {
        slots::Entity::delete_many()
            .filter(slots::Column::HostId.eq(self.host))
            .filter(slots::Column::Slot.eq(self.slot))
            .filter(slots::Column::Owner.eq(&self.owner))
            .exec(conn)
            .await?;
        Ok(())
    }
}
pub async fn try_acquire(
    conn: &impl ConnectionTrait,
    host: i64,
    capacity: u32,
    run: i64,
    key: &str,
) -> Result<Option<Lease>> {
    // Includes slots above a newly reduced capacity: existing work drains before
    // any replacement enters the smaller budget. Call under the mutation lock.
    if slots::Entity::find()
        .filter(slots::Column::HostId.eq(host))
        .filter(slots::Column::ExpiresAt.gt(db::now()))
        .all(conn)
        .await?
        .len()
        >= capacity as usize
    {
        return Ok(None);
    }
    for slot in 0..i64::from(capacity) {
        let owner = uuid::Uuid::new_v4().to_string();
        slots::Entity::insert(slots::ActiveModel {
            host_id: Set(host),
            slot: Set(slot),
            owner: Set(owner.clone()),
            run_id: Set(run),
            job_key: Set(key.into()),
            expires_at: Set(db::now() + TTL),
        })
        .on_conflict_do_nothing()
        .exec_without_returning(conn)
        .await?;
        let updated = slots::Entity::update_many()
            .col_expr(slots::Column::Owner, Expr::value(owner.clone()))
            .col_expr(slots::Column::RunId, Expr::value(run))
            .col_expr(slots::Column::JobKey, Expr::value(key))
            .col_expr(slots::Column::ExpiresAt, Expr::value(db::now() + TTL))
            .filter(slots::Column::HostId.eq(host))
            .filter(slots::Column::Slot.eq(slot))
            .filter(slots::Column::ExpiresAt.lte(db::now()))
            .exec(conn)
            .await?
            .rows_affected;
        let own = updated == 1
            || slots::Entity::find_by_id((host, slot))
                .one(conn)
                .await?
                .is_some_and(|s| s.owner == owner);
        if own {
            return Ok(Some(Lease { host, slot, owner }));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{Database, DatabaseConnection};
    use sea_orm_migration::MigratorTrait;
    async fn fixture() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        crate::migration::Migrator::up(&db, None).await.unwrap();
        for (id, workspace, group, state, os, arch) in [
            (1, 1, 10, "online", "linux", "x86_64"),
            (2, 1, 10, "online", "darwin", "aarch64"),
            (3, 2, 10, "online", "linux", "x86_64"),
            (4, 1, 10, "offline", "linux", "x86_64"),
        ] {
            hosts::Entity::insert(hosts::ActiveModel {
                id: Set(id),
                workspace_id: Set(workspace),
                name: Set(format!("host-{id}")),
                address: Set("127.0.0.1".into()),
                port: Set(22),
                username: Set("user".into()),
                connection: Set("runner".into()),
                state: Set(state.into()),
                group_id: Set(group),
                description: Set(String::new()),
                host_key: Set(String::new()),
                runner_key_hash: Set(String::new()),
                jump_host_id: Set(0),
                seen_at: Set(0),
                created_at: Set(0),
                updated_at: Set(0),
                version: Set(1),
                os: Set(os.into()),
                arch: Set(arch.into()),
                agent_version: Set(String::new()),
                metrics: Set(String::new()),
                credential_id: Set(0),
                ..Default::default()
            })
            .exec(&db)
            .await
            .unwrap();
        }
        db
    }
    #[tokio::test]
    async fn pools_are_opt_in_workspace_scoped_and_match_platform_labels() {
        let db = fixture().await;
        let arm_mac = hosts::Entity::find_by_id(2)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(!matches(&arm_mac, &["linux".into()], &["linux".into()]));
        assert!(!matches(&arm_mac, &["x64".into()], &["x64".into()]));
        assert!(matches(&arm_mac, &["GPU".into()], &["gpu".into()]));
        assert!(
            candidates(&db, 1, 0, 0, true, &[])
                .await
                .unwrap()
                .is_empty()
        );
        for id in [1, 2, 3, 4] {
            save(
                &db,
                Id::from(if id == 3 { 2 } else { 1 }),
                Id::from(id),
                &Policy {
                    shared: true,
                    ..Policy::default()
                },
            )
            .await
            .unwrap();
        }
        let labels = ["self-hosted", "linux", "amd64"].map(str::to_owned);
        let selected = candidates(&db, 1, 0, 0, true, &labels).await.unwrap();
        assert_eq!(
            selected.iter().map(|(h, _, _)| h.id).collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(
            candidates(&db, 1, 0, 10, false, &["macos".into(), "arm64".into()])
                .await
                .unwrap()[0]
                .0
                .id,
            2
        );
        assert!(
            candidates(&db, 1, 0, 0, true, &["ubuntu-latest".into()])
                .await
                .unwrap()
                .is_empty()
        );
        // Explicit binding keeps the historic descriptive-label semantics.
        assert_eq!(
            candidates(&db, 1, 1, 0, false, &["unrelated-label".into()])
                .await
                .unwrap()
                .len(),
            1
        );
        save(
            &db,
            Id::from(1),
            Id::from(1),
            &Policy {
                enabled: false,
                ..Policy::default()
            },
        )
        .await
        .unwrap();
        assert!(
            candidates(&db, 1, 1, 0, false, &[])
                .await
                .unwrap()
                .is_empty()
        );
    }
    #[tokio::test]
    async fn slots_enforce_capacity_and_stale_owners_cannot_renew_or_release_successors() {
        let db = fixture().await;
        let first = try_acquire(&db, 1, 2, 11, "build").await.unwrap().unwrap();
        let second = try_acquire(&db, 1, 2, 12, "test").await.unwrap().unwrap();
        assert!(try_acquire(&db, 1, 2, 13, "third").await.unwrap().is_none());
        assert!(
            try_acquire(&db, 1, 1, 13, "reduced")
                .await
                .unwrap()
                .is_none()
        );
        assert!(first.renew(&db).await.unwrap());
        slots::Entity::update_many()
            .col_expr(slots::Column::ExpiresAt, Expr::value(0))
            .filter(slots::Column::HostId.eq(1))
            .filter(slots::Column::Slot.eq(first.slot))
            .exec(&db)
            .await
            .unwrap();
        let replacement = try_acquire(&db, 1, 2, 14, "replacement")
            .await
            .unwrap()
            .unwrap();
        assert!(!first.renew(&db).await.unwrap());
        first.release(&db).await.unwrap();
        assert!(replacement.renew(&db).await.unwrap());
        let host = hosts::Entity::find_by_id(1)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        save(
            &db,
            Id::from(1),
            Id::from(1),
            &Policy {
                enabled: false,
                ..Policy::default()
            },
        )
        .await
        .unwrap();
        let mut json = json!({});
        decorate(&db, &host, &mut json).await.unwrap();
        assert_eq!(json["execution_status"]["state"], "draining");
        assert_eq!(json["execution_status"]["active"], 2);
        replacement.release(&db).await.unwrap();
        second.release(&db).await.unwrap();
        decorate(&db, &host, &mut json).await.unwrap();
        assert_eq!(json["execution_status"]["state"], "disabled");
    }
    #[test]
    fn capacity_is_bounded_and_defaults_are_private() {
        assert!(!Policy::default().shared);
        for capacity in [0, 65, u32::MAX] {
            assert!(
                Policy {
                    capacity,
                    ..Policy::default()
                }
                .validate()
                .is_err()
            );
        }
        for capacity in [1, 64] {
            assert!(
                Policy {
                    capacity,
                    ..Policy::default()
                }
                .validate()
                .is_ok()
            );
        }
    }
}
