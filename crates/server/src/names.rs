//! Display names for the records a response references by ID. Responses keep
//! every reference ID and gain a name field next to it, so pages can show the
//! name and keep the ID for links. Each referenced kind costs one query per
//! response and is scoped to the workspace (users are global; only their
//! display name is exposed). A reference that no longer resolves gets "".

use crate::{
    entities::{deployments, host_groups, hosts, pipeline_runs, pipelines, repositories, users},
    error::Result,
    snowflake::Id,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Pipeline,
    User,
    Host,
    HostGroup,
    Deployment,
    Repository,
    /// A pipeline run is named by its per-pipeline number.
    RunNumber,
}

/// How the kind of a reference is known.
pub enum Of {
    Kind(Kind),
    /// Named by a sibling field holding "host" or "host-group" (`target_kind`).
    Target(&'static str),
}

/// `id`: the reference field; `name`: the field added next to it.
pub struct Ref {
    pub id: &'static str,
    pub name: &'static str,
    pub of: Of,
}

pub const fn of(id: &'static str, name: &'static str, kind: Kind) -> Ref {
    Ref {
        id,
        name,
        of: Of::Kind(kind),
    }
}

pub const fn target(id: &'static str, name: &'static str, kind_field: &'static str) -> Ref {
    Ref {
        id,
        name,
        of: Of::Target(kind_field),
    }
}

fn kind_of(row: &Value, of: &Of) -> Option<Kind> {
    match of {
        Of::Kind(kind) => Some(*kind),
        Of::Target(field) => match row[*field].as_str() {
            Some("host") => Some(Kind::Host),
            Some("host-group") => Some(Kind::HostGroup),
            _ => None,
        },
    }
}

fn id_of(row: &Value, field: &str) -> Option<i64> {
    row[field]
        .as_str()
        .and_then(|value| value.parse::<Id>().ok())
        .map(i64::from)
        .filter(|id| *id != 0)
}

pub async fn attach(
    conn: &impl ConnectionTrait,
    workspace: Id,
    values: &mut [Value],
    refs: &[Ref],
) -> Result<()> {
    let mut wanted: HashMap<Kind, HashSet<i64>> = HashMap::new();
    for row in values.iter() {
        for reference in refs {
            if let (Some(kind), Some(id)) = (kind_of(row, &reference.of), id_of(row, reference.id))
            {
                wanted.entry(kind).or_default().insert(id);
            }
        }
    }
    let mut names = HashMap::new();
    for (kind, ids) in wanted {
        for (id, name) in lookup(conn, workspace, kind, ids.into_iter().collect()).await? {
            names.insert((kind, id), name);
        }
    }
    for row in values.iter_mut() {
        for reference in refs {
            let name = kind_of(row, &reference.of)
                .zip(id_of(row, reference.id))
                .and_then(|key| names.get(&key).cloned())
                .unwrap_or_else(|| json!(""));
            row[reference.name] = name;
        }
    }
    Ok(())
}

async fn lookup(
    conn: &impl ConnectionTrait,
    workspace: Id,
    kind: Kind,
    ids: Vec<i64>,
) -> Result<Vec<(i64, Value)>> {
    let workspace = i64::from(workspace);
    macro_rules! named {
        ($table:ident) => {
            $table::Entity::find()
                .select_only()
                .columns([$table::Column::Id, $table::Column::Name])
                .filter($table::Column::WorkspaceId.eq(workspace))
                .filter($table::Column::Id.is_in(ids))
                .into_tuple::<(i64, String)>()
                .all(conn)
                .await?
                .into_iter()
                .map(|(id, name)| (id, json!(name)))
                .collect()
        };
    }
    Ok(match kind {
        Kind::Pipeline => named!(pipelines),
        Kind::Host => named!(hosts),
        Kind::HostGroup => named!(host_groups),
        Kind::Deployment => named!(deployments),
        Kind::Repository => named!(repositories),
        Kind::User => users::Entity::find()
            .select_only()
            .columns([users::Column::Id, users::Column::Name])
            .filter(users::Column::Id.is_in(ids))
            .into_tuple::<(i64, String)>()
            .all(conn)
            .await?
            .into_iter()
            .map(|(id, name)| (id, json!(name)))
            .collect(),
        Kind::RunNumber => pipeline_runs::Entity::find()
            .select_only()
            .columns([pipeline_runs::Column::Id, pipeline_runs::Column::Number])
            .filter(pipeline_runs::Column::WorkspaceId.eq(workspace))
            .filter(pipeline_runs::Column::Id.is_in(ids))
            .into_tuple::<(i64, i64)>()
            .all(conn)
            .await?
            .into_iter()
            .map(|(id, number)| (id, json!(number)))
            .collect(),
    })
}
