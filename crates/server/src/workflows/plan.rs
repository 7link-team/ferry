//! A plan consumes one explicitly selected trigger's graph. Save-time warnings
//! are not publish errors; an ambiguous input still cannot be executed.
use crate::{
    error::{ApiError, Result},
    i18n::Message,
};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Endpoint {
    pub node: String,
    pub port: String,
    pub index: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub source: Endpoint,
    pub port: String,
    pub index: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Node {
    pub id: String,
    pub path: String,
    pub inputs: Vec<Input>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Plan {
    pub version: u8,
    pub entry_node_id: String,
    pub nodes: Vec<Node>,
}

fn invalid(path: &str, reason: &'static str) -> ApiError {
    ApiError::localized(
        StatusCode::BAD_REQUEST,
        Message::new("workflow.runNotReady")
            .with("path", path)
            .with_message("reason", Message::new(reason)),
    )
}
fn endpoint(value: &Value) -> Result<Endpoint> {
    Ok(Endpoint {
        node: value["node"]
            .as_str()
            .ok_or_else(|| invalid("/connections", "workflow.graphSchema"))?
            .into(),
        port: value["port"]
            .as_str()
            .ok_or_else(|| invalid("/connections", "workflow.graphSchema"))?
            .into(),
        index: super::graph::port_index(&value["index"])
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| invalid("/connections", "workflow.graphPort"))?,
    })
}
pub(crate) fn compile(input: &str, entry: &str) -> Result<Plan> {
    let canonical = super::graph::validate(input)?;
    let graph: Value = serde_json::from_str(&canonical).expect("validated graph");
    let lookup: BTreeMap<_, _> = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(index, node)| (node["id"].as_str().unwrap(), (index, node)))
        .collect();
    let (_, start) = lookup
        .get(entry)
        .ok_or_else(|| invalid("/entry_node_id", "workflow.runEntry"))?;
    if !start["type"].as_str().unwrap().starts_with("core.trigger.") {
        return Err(invalid("/entry_node_id", "workflow.runEntry"));
    }
    let mut edges = Vec::new();
    for edge in graph["connections"].as_array().unwrap() {
        edges.push((endpoint(&edge["from"])?, endpoint(&edge["to"])?));
    }
    let mut reachable = BTreeSet::from([entry.to_owned()]);
    let mut queue = VecDeque::from([entry.to_owned()]);
    while let Some(id) = queue.pop_front() {
        for (_, target) in edges.iter().filter(|(source, _)| source.node == id) {
            if reachable.insert(target.node.clone()) {
                queue.push_back(target.node.clone());
            }
        }
    }
    let mut planned = BTreeMap::new();
    let mut dependencies = BTreeMap::new();
    for id in &reachable {
        let (index, node) = lookup[id.as_str()];
        let path = format!("/nodes/{index}");
        if id != entry && node["type"].as_str().unwrap().starts_with("core.trigger.") {
            return Err(invalid(&path, "workflow.runEntry"));
        }
        if node["type"] == "core.loop" {
            return Err(invalid(&path, "workflow.runLoopInterface"));
        }
        let spec = super::graph::manifest(node).expect("validated manifest");
        let mut slots = BTreeSet::new();
        for port in spec["inputs"].as_array().unwrap() {
            let count = if node["type"] == "core.merge" {
                super::graph::port_index(&node["params"]["input_count"])
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| invalid(&path, "workflow.graphSchema"))?
            } else {
                1
            };
            for index in 0..count {
                slots.insert((port["port"].as_str().unwrap().to_owned(), index));
            }
        }
        let mut inputs = BTreeMap::new();
        let mut needs = BTreeSet::new();
        for (source, target) in edges.iter().filter(|(_, target)| target.node == *id) {
            if !reachable.contains(&source.node) {
                return Err(invalid(&path, "workflow.runOtherEntry"));
            }
            let slot = (target.port.clone(), target.index);
            if inputs
                .insert(
                    slot.clone(),
                    Input {
                        source: source.clone(),
                        port: target.port.clone(),
                        index: target.index,
                    },
                )
                .is_some()
            {
                return Err(invalid(&path, "workflow.runAmbiguousInput"));
            }
            if !slots.contains(&slot) {
                return Err(invalid(&path, "workflow.graphPort"));
            }
            needs.insert(source.node.clone());
        }
        if inputs.keys().cloned().collect::<BTreeSet<_>>() != slots {
            return Err(invalid(&path, "workflow.runMissingInput"));
        }
        dependencies.insert(id.clone(), needs);
        planned.insert(
            id.clone(),
            Node {
                id: id.clone(),
                path,
                inputs: inputs.into_values().collect(),
            },
        );
    }
    // Stable node IDs resolve ties. Neither canvas order nor edge order decides
    // which producer supplies an input, or changes execution semantics.
    let mut ready: BTreeSet<_> = dependencies
        .iter()
        .filter(|(_, needs)| needs.is_empty())
        .map(|(id, _)| id.clone())
        .collect();
    let mut nodes = Vec::new();
    while let Some(id) = ready.pop_first() {
        nodes.push(planned.remove(&id).expect("each planned node once"));
        for (other, needs) in &mut dependencies {
            if needs.remove(&id) && needs.is_empty() {
                ready.insert(other.clone());
            }
        }
    }
    if !planned.is_empty() {
        return Err(invalid("/connections", "workflow.graphCycle"));
    }
    Ok(Plan {
        version: 1,
        entry_node_id: entry.into(),
        nodes,
    })
}

#[cfg(test)]
mod tests;
