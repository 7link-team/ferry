//! Scheduling candidate set: one entry per workspace host, with the facts
//! placement needs (state, group, tags, free memory and disk).
use crate::{
    entities::{host_groups, host_tags, hosts, tags},
    error::Result,
    snowflake::Id,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: Id,
    pub name: String,
    pub address: String,
    pub os: String,
    pub online: bool,
    pub group: Option<String>,
    pub tags: Vec<String>,
    pub mem_free: Option<u64>,
    pub disk_free: Option<u64>,
}

pub async fn load(conn: &impl ConnectionTrait, workspace: Id) -> Result<Vec<Candidate>> {
    let rows = hosts::Entity::find()
        .filter(hosts::Column::WorkspaceId.eq(workspace))
        .order_by_asc(hosts::Column::Id)
        .all(conn)
        .await?;
    let groups: BTreeMap<i64, String> = host_groups::Entity::find()
        .filter(host_groups::Column::WorkspaceId.eq(workspace))
        .all(conn)
        .await?
        .into_iter()
        .map(|g| (g.id, g.name))
        .collect();
    let names: BTreeMap<i64, String> = tags::Entity::find()
        .filter(tags::Column::WorkspaceId.eq(workspace))
        .all(conn)
        .await?
        .into_iter()
        .map(|t| (t.id, t.name))
        .collect();
    let mut by_host: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    let ids: Vec<i64> = rows.iter().map(|h| h.id).collect();
    for link in host_tags::Entity::find()
        .filter(host_tags::Column::HostId.is_in(ids))
        .all(conn)
        .await?
    {
        // Tags of other workspaces are absent from `names`, so they drop out here.
        if let Some(name) = names.get(&link.tag_id) {
            by_host.entry(link.host_id).or_default().push(name.clone());
        }
    }
    Ok(rows
        .into_iter()
        .map(|h| {
            let mut tags = by_host.remove(&h.id).unwrap_or_default();
            tags.sort();
            let metrics: Option<Value> = serde_json::from_str(&h.metrics).ok();
            let free = |total: &str, used: &str| -> Option<u64> {
                let m = metrics.as_ref()?;
                m.get(total)?.as_u64()?.checked_sub(m.get(used)?.as_u64()?)
            };
            Candidate {
                id: h.id.into(),
                online: h.state == "online",
                group: groups.get(&h.group_id).cloned(),
                tags,
                mem_free: free("mem_total", "mem_used"),
                disk_free: free("disk_total", "disk_used"),
                name: h.name,
                address: h.address,
                os: h.os,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::tests::setup;
    use sea_orm::{ActiveModelTrait, Set};

    async fn host(
        db: &impl ConnectionTrait,
        id: i64,
        ws: i64,
        state: &str,
        group: i64,
        metrics: &str,
    ) {
        hosts::ActiveModel {
            id: Set(id),
            workspace_id: Set(ws),
            name: Set(format!("H{id}")),
            address: Set(format!("10.0.0.{id}")),
            port: Set(22),
            username: Set("t".into()),
            connection: Set("runner".into()),
            state: Set(state.into()),
            group_id: Set(group),
            os: Set("linux".into()),
            metrics: Set(metrics.into()),
            description: Set(String::new()),
            host_key: Set(String::new()),
            runner_key_hash: Set(String::new()),
            enroll_token: Set(String::new()),
            enroll_token_hash: Set(String::new()),
            seen_at: Set(0),
            created_at: Set(0),
            updated_at: Set(0),
            version: Set(1),
            arch: Set(String::new()),
            agent_version: Set(String::new()),
            credential_id: Set(0),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
    }
    async fn tag(db: &impl ConnectionTrait, id: i64, ws: i64, name: &str, host: i64) {
        tags::ActiveModel {
            id: Set(id),
            workspace_id: Set(ws),
            name: Set(name.into()),
            color: Set(String::new()),
            created_at: Set(0),
            updated_at: Set(0),
            version: Set(1),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
        host_tags::ActiveModel {
            host_id: Set(host),
            tag_id: Set(id),
        }
        .insert(db)
        .await
        .unwrap();
    }
    async fn group(db: &impl ConnectionTrait, id: i64, ws: i64, name: &str) {
        host_groups::ActiveModel {
            id: Set(id),
            workspace_id: Set(ws),
            name: Set(name.into()),
            description: Set(String::new()),
            parent_id: Set(0),
            position: Set(0),
            created_at: Set(0),
            updated_at: Set(0),
            version: Set(1),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn candidates_include_tags_group_and_free_memory() {
        let (_d, s) = setup().await;
        group(&s.db, 10, 1, "edge").await;
        host(
            &s.db,
            3,
            1,
            "online",
            10,
            r#"{"mem_used":300,"mem_total":1000,"disk_used":40,"disk_total":100}"#,
        )
        .await;
        tag(&s.db, 20, 1, "ssd", 3).await;
        tag(&s.db, 21, 1, "gpu", 3).await;
        let c = load(&s.db, 1.into()).await.unwrap();
        assert_eq!(
            c.iter().map(|c| i64::from(c.id)).collect::<Vec<_>>(),
            [1, 3]
        );
        assert!(!c[0].online);
        assert_eq!(c[0].group, None);
        let h = &c[1];
        assert!(h.online);
        assert_eq!(h.name, "H3");
        assert_eq!(h.address, "10.0.0.3");
        assert_eq!(h.os, "linux");
        assert_eq!(h.group.as_deref(), Some("edge"));
        assert_eq!(h.tags, ["gpu", "ssd"]);
        assert_eq!(h.mem_free, Some(700));
        assert_eq!(h.disk_free, Some(60));
    }

    #[tokio::test]
    async fn candidates_are_workspace_scoped() {
        let (_d, s) = setup().await;
        group(&s.db, 11, 2, "other").await;
        host(&s.db, 4, 2, "online", 11, "").await;
        tag(&s.db, 22, 2, "secret", 4).await;
        // A workspace-2 tag attached to a workspace-1 host must not leak either.
        tag(&s.db, 23, 2, "foreign", 1).await;
        let c = load(&s.db, 1.into()).await.unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(i64::from(c[0].id), 1);
        assert!(c[0].tags.is_empty());
        // A workspace-1 host pointing at a workspace-2 group gets no group name.
        host(&s.db, 5, 1, "online", 11, "").await;
        let c = load(&s.db, 1.into()).await.unwrap();
        assert_eq!(c.iter().find(|c| i64::from(c.id) == 5).unwrap().group, None);
    }

    #[tokio::test]
    async fn missing_metrics_give_none() {
        let (_d, s) = setup().await;
        host(&s.db, 3, 1, "online", 0, "not json").await;
        host(
            &s.db,
            4,
            1,
            "unknown",
            0,
            r#"{"mem_used":9,"mem_total":5,"disk_used":1,"disk_total":2}"#,
        )
        .await;
        let c = load(&s.db, 1.into()).await.unwrap();
        assert_eq!((c[0].mem_free, c[0].disk_free), (None, None));
        assert_eq!((c[1].mem_free, c[1].disk_free), (None, None));
        assert_eq!((c[2].mem_free, c[2].disk_free), (None, Some(1)));
        assert!(!c[2].online);
    }
}
