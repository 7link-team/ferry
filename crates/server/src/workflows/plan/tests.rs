use super::*;
use serde_json::json;
fn node(id: &str, kind: &str) -> Value {
    let params = match kind {
        "core.trigger.manual" => json!({"input_schema":{"type":"object"}}),
        "core.merge" => json!({"mode":"append","wait":"all_inputs","input_count":2}),
        _ => json!({"fields":[],"keep_other_fields":true}),
    };
    json!({"id":id,"name":id,"type":kind,"type_version":1,"params":params,"position":[0,0]})
}
fn edge(from: &str, to: &str, index: usize) -> Value {
    let port = if to == "merge" { "input" } else { "main" };
    json!({"from":{"node":from,"port":"main","index":0},"to":{"node":to,"port":port,"index":index}})
}
fn graph(nodes: Vec<Value>, connections: Vec<Value>) -> Value {
    json!({"engine":"ferry-graph","schema":1,"nodes":nodes,"connections":connections})
}
#[test]
fn selected_entry_has_only_its_reachable_graph_and_requires_explicit_trigger() {
    let g = graph(
        vec![
            node("a", "core.trigger.manual"),
            node("b", "core.trigger.manual"),
            node("set", "data.set"),
            node("unselected", "data.set"),
        ],
        vec![edge("a", "set", 0), edge("b", "unselected", 0)],
    );
    assert_eq!(
        compile(&g.to_string(), "a")
            .unwrap()
            .nodes
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "set"]
    );
    for entry in ["", "missing", "set"] {
        assert!(compile(&g.to_string(), entry).is_err());
    }
}
#[test]
fn missing_merge_slot_and_cross_entry_dependency_do_not_turn_into_empty_items() {
    let mut g = graph(
        vec![
            node("a", "core.trigger.manual"),
            node("b", "core.trigger.manual"),
            node("merge", "core.merge"),
        ],
        vec![edge("a", "merge", 0)],
    );
    assert!(super::super::graph::validate(&g.to_string()).is_ok());
    assert!(compile(&g.to_string(), "a").is_err());
    g["connections"]
        .as_array_mut()
        .unwrap()
        .push(edge("b", "merge", 1));
    assert!(compile(&g.to_string(), "a").is_err());
}
#[test]
fn fanin_is_editable_but_cannot_produce_an_ambiguous_run_plan() {
    let g = graph(
        vec![
            node("a", "core.trigger.manual"),
            node("first", "data.set"),
            node("second", "data.set"),
            node("last", "data.set"),
        ],
        vec![
            edge("a", "first", 0),
            edge("a", "second", 0),
            edge("first", "last", 0),
            edge("second", "last", 0),
        ],
    );
    assert!(super::super::graph::validate(&g.to_string()).is_ok());
    assert!(compile(&g.to_string(), "a").is_err());
}
#[test]
fn merge_inputs_follow_declared_slot_order_and_topological_ties_use_ids() {
    let mut g = graph(
        vec![
            node("a", "core.trigger.manual"),
            node("z", "data.set"),
            node("b", "data.set"),
            node("merge", "core.merge"),
        ],
        vec![
            edge("a", "z", 0),
            edge("a", "b", 0),
            edge("z", "merge", 1),
            edge("b", "merge", 0),
        ],
    );
    let first = compile(&g.to_string(), "a").unwrap();
    assert_eq!(
        first
            .nodes
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b", "z", "merge"]
    );
    assert_eq!(
        first.nodes[3]
            .inputs
            .iter()
            .map(|i| i.source.node.as_str())
            .collect::<Vec<_>>(),
        vec!["b", "z"]
    );
    g["connections"].as_array_mut().unwrap().reverse();
    assert_eq!(first, compile(&g.to_string(), "a").unwrap());
}
#[test]
fn unreachable_unconnected_nodes_and_unconnected_branch_outputs_are_allowed() {
    let g = graph(
        vec![node("a", "core.trigger.manual"), node("unused", "data.set")],
        vec![],
    );
    assert_eq!(compile(&g.to_string(), "a").unwrap().nodes.len(), 1);
}
#[test]
fn unknown_implementations_and_graph_corruption_are_not_accepted_as_executable_code() {
    let mut g = graph(vec![node("a", "core.trigger.manual")], vec![]);
    g["nodes"][0]["type_version"] = json!(2);
    assert!(compile(&g.to_string(), "a").is_err());
    let g = graph(
        vec![node("a", "core.trigger.manual"), node("b", "data.set")],
        vec![edge("a", "b", 0), edge("b", "b", 0)],
    );
    assert!(compile(&g.to_string(), "a").is_err());
}
