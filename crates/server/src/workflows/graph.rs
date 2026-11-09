//! Save-time structural validation. Catalog schemas describe draft contracts;
//! they do not register executors or certify a graph as runnable/publishable.
use crate::{
    error::{ApiError, Result},
    i18n::Message,
};
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::OnceLock,
};

pub(crate) const MAX_DEFINITION_BYTES: usize = 2 * 1024 * 1024;
const MAX_PIN_BYTES: usize = 1024 * 1024;
const MAX_NODES: usize = 500;

struct NodeContract {
    spec: Value,
    params: jsonschema::Validator,
    resolved: jsonschema::Validator,
}
struct Contracts {
    graph: jsonschema::Validator,
    nodes: BTreeMap<String, NodeContract>,
}

fn pointer_key(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}
fn matches_field(pattern: &str, path: &str) -> bool {
    let pattern: Vec<_> = pattern.split('/').collect();
    let path: Vec<_> = path.split('/').collect();
    pattern.len() == path.len() && pattern.iter().zip(path).all(|(a, b)| *a == "*" || *a == b)
}

/// Drafts keep raw expressions only at manifest-declared parameter paths.
/// Original params_schema remains the contract for evaluated runtime values.
fn draft_schema(mut schema: Value, fields: &[String], path: &str) -> Value {
    if let Some(object) = schema.as_object_mut() {
        if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
            for (key, child) in properties {
                *child = draft_schema(
                    child.take(),
                    fields,
                    &format!("{path}/{}", pointer_key(key)),
                );
            }
        }
        for keyword in ["items", "additionalProperties"] {
            if let Some(child) = object.get_mut(keyword) {
                *child = draft_schema(child.take(), fields, &format!("{path}/*"));
            }
        }
        for keyword in ["allOf", "anyOf", "oneOf"] {
            if let Some(children) = object.get_mut(keyword).and_then(Value::as_array_mut) {
                for child in children {
                    *child = draft_schema(child.take(), fields, path);
                }
            }
        }
        for keyword in ["if", "then", "else", "not"] {
            if let Some(child) = object.get_mut(keyword) {
                *child = draft_schema(child.take(), fields, path);
            }
        }
    }
    if fields.iter().any(|field| matches_field(field, path)) {
        json!({"anyOf":[schema,{"type":"string","pattern":"^="}]})
    } else {
        schema
    }
}

fn expression_fields(spec: &Value) -> Vec<String> {
    spec["expression_fields"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect()
}

fn contracts() -> &'static Contracts {
    static CONTRACTS: OnceLock<Contracts> = OnceLock::new();
    CONTRACTS.get_or_init(|| {
        let compile = |schema: &Value| {
            jsonschema::options()
                .with_draft(jsonschema::Draft::Draft202012)
                .build(schema)
                .expect("bundled Workflow schema must compile")
        };
        let graph = compile(
            &serde_json::from_str::<Value>(include_str!(
                "../../../../src/shared/workflow-contracts/workflow-graph.schema.json"
            ))
            .expect("bundled graph schema must be JSON"),
        );
        let mut nodes = BTreeMap::new();
        for catalog in [
            include_str!("../../../../src/shared/workflow-contracts/workflow-core-nodes.json"),
            include_str!("../../../../src/shared/workflow-contracts/workflow-data-nodes.json"),
            include_str!("../../../../src/shared/workflow-contracts/workflow-file-nodes.json"),
            include_str!("../../../../src/shared/workflow-contracts/workflow-ai-nodes.json"),
            include_str!(
                "../../../../src/shared/workflow-contracts/workflow-integration-nodes.json"
            ),
        ] {
            let catalog: Value =
                serde_json::from_str(catalog).expect("bundled catalog must be JSON");
            for spec in catalog["nodes"]
                .as_array()
                .expect("bundled catalog must have nodes")
            {
                let kind = spec["type"].as_str().expect("node type").to_owned();
                let contract = NodeContract {
                    resolved: compile(&spec["params_schema"]),
                    params: compile(&draft_schema(
                        spec["params_schema"].clone(),
                        &expression_fields(spec),
                        "",
                    )),
                    spec: spec.clone(),
                };
                assert!(
                    nodes.insert(kind, contract).is_none(),
                    "duplicate bundled node type"
                );
            }
        }
        for spec in &super::extensions::catalog().nodes {
            let kind = spec["type"]
                .as_str()
                .expect("validated extension type")
                .to_owned();
            let contract = NodeContract {
                resolved: compile(&spec["params_schema"]),
                params: compile(&draft_schema(
                    spec["params_schema"].clone(),
                    &expression_fields(spec),
                    "",
                )),
                spec: spec.clone(),
            };
            assert!(
                nodes.insert(kind, contract).is_none(),
                "duplicate extension node type"
            );
        }
        Contracts { graph, nodes }
    })
}

fn invalid(path: impl Into<String>, key: &'static str) -> ApiError {
    ApiError::localized(
        StatusCode::BAD_REQUEST,
        Message::new(key).with("path", path.into()),
    )
}

pub(super) fn port_index(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|number| {
                *number >= 0.0 && *number < 18_446_744_073_709_551_616.0 && number.fract() == 0.0
            })
            .map(|number| number as u64)
    })
}

pub(crate) fn validate(input: &str) -> Result<String> {
    if input.len() > MAX_DEFINITION_BYTES {
        return Err(invalid("/", "workflow.graphLimit"));
    }
    let graph: Value =
        serde_json::from_str(input).map_err(|_| invalid("/", "workflow.graphSchema"))?;
    let pin_bytes = serde_json::to_vec(graph.get("pin_data").unwrap_or(&json!({})))
        .map_err(|_| invalid("/pin_data", "workflow.graphSchema"))?
        .len();
    if pin_bytes > MAX_PIN_BYTES {
        return Err(invalid("/pin_data", "workflow.graphLimit"));
    }
    validate_layer(&graph, "", 0, &mut 0)?;
    crate::definitions::canonical(input)
}

pub(crate) fn pin_data<'a>(graph: &'a Value, node: &Value, mode: &str) -> Option<&'a Value> {
    if !matches!(mode, "manual" | "test")
        || node
            .pointer("/settings/mode")
            .and_then(Value::as_str)
            .unwrap_or("active")
            != "active"
    {
        return None;
    }
    graph.get("pin_data")?.get(node["id"].as_str()?)
}

pub(crate) fn pin_items(value: &Value) -> Result<Vec<ferry_protocol::sandbox::Item>> {
    let mut value = value.clone();
    let items = value
        .as_array_mut()
        .ok_or_else(|| invalid("/pin_data", "workflow.graphPin"))?;
    for item in items {
        if let Some(binary) = item.get_mut("binary").and_then(Value::as_object_mut) {
            for asset in binary.values_mut() {
                asset["size"] = port_index(&asset["size"])
                    .ok_or_else(|| invalid("/pin_data", "workflow.graphPin"))?
                    .into();
            }
        }
        if let Some(paired) = item.get_mut("paired").and_then(Value::as_array_mut) {
            for pair in paired {
                pair["index"] = port_index(&pair["index"])
                    .ok_or_else(|| invalid("/pin_data", "workflow.graphPin"))?
                    .into();
            }
        }
    }
    serde_json::from_value(value).map_err(|_| invalid("/pin_data", "workflow.graphPin"))
}

pub(crate) fn manifest(node: &Value) -> Option<&'static Value> {
    contracts()
        .nodes
        .get(node["type"].as_str()?)
        .filter(|contract| contract.spec["type_version"].as_f64() == node["type_version"].as_f64())
        .map(|contract| &contract.spec)
}

fn collect_params(
    value: &Value,
    path: &str,
    base: &str,
    fields: &[String],
    out: &mut Vec<(String, String)>,
) {
    if let Some(value) = value.as_str() {
        if value.starts_with('=') && fields.iter().any(|field| matches_field(field, path)) {
            out.push((format!("{base}{path}"), value.to_owned()));
        }
    } else if let Some(values) = value.as_array() {
        for (index, value) in values.iter().enumerate() {
            collect_params(value, &format!("{path}/{index}"), base, fields, out);
        }
    } else if let Some(values) = value.as_object() {
        for (key, value) in values {
            collect_params(
                value,
                &format!("{path}/{}", pointer_key(key)),
                base,
                fields,
                out,
            );
        }
    }
}

fn collect_graph(graph: &Value, base: &str, out: &mut Vec<(String, String)>) {
    for (index, node) in graph["nodes"].as_array().unwrap().iter().enumerate() {
        let path = format!("{base}/nodes/{index}/params");
        let spec = &contracts().nodes[node["type"].as_str().unwrap()].spec;
        collect_params(&node["params"], "", &path, &expression_fields(spec), out);
        if node["type"] == "core.loop" {
            collect_graph(&node["params"]["body"], &format!("{path}/body"), out);
        }
    }
}

pub(crate) async fn checked(input: &str) -> Result<String> {
    let canonical = validate(input)?;
    let graph: Value = serde_json::from_str(&canonical).expect("validated JSON");
    let mut expressions = Vec::new();
    collect_graph(&graph, "", &mut expressions);
    super::expression::syntax(expressions).await?;
    Ok(canonical)
}

/// Legacy authentication is retained only for the same persisted node and auth
/// configuration. Clients cannot opt a newly created node into legacy behavior.
pub(super) fn repository_auth_policy(input: &str, previous: Option<&str>) -> Result<()> {
    let graph: Value = serde_json::from_str(input).map_err(|_| invalid("/", "workflow.graphSchema"))?;
    let previous = previous.and_then(|value| serde_json::from_str::<Value>(value).ok());
    fn layer(graph: &Value, previous: Option<&Value>, path: &str) -> Result<()> {
        for (index, node) in graph["nodes"].as_array().into_iter().flatten().enumerate() {
            let at = format!("{path}/nodes/{index}/params");
            let old = previous.and_then(|graph| graph["nodes"].as_array())
                .and_then(|nodes| nodes.iter().find(|old| old["id"] == node["id"] && old["type"] == node["type"]));
            if node["type"] == "core.loop" {
                layer(&node["params"]["body"], old.map(|old| &old["params"]["body"]), &format!("{at}/body"))?;
            }
            let Some(spec) = manifest(node) else { continue; };
            let api = matches!(spec["http"]["auth"]["profile"].as_str(), Some("github" | "gitlab" | "codeup"));
            let network = matches!(spec["git"]["operation"].as_str(), Some("clone" | "remote_refs" | "fetch" | "pull" | "push"));
            if !api && !network { continue; }
            let params = &node["params"];
            if params.get("connection_id").is_some() {
                if network && params["auth_mode"] == "anonymous" { return Err(invalid(at, "workflow.repositoryConnectionRequired")); }
                continue;
            }
            let legacy = old.is_some_and(|old| old["params"].get("connection_id").is_none()
                && ["credential_id", "organization_id", "auth_mode"].iter().all(|key| old["params"].get(key) == params.get(key)));
            if legacy { continue; }
            if network && params["auth_mode"] == "anonymous" {
                if params["url"].as_str().is_some_and(|url| url.starts_with('=')) { continue; }
                if anonymous_git_url(params["url"].as_str().unwrap_or_default()) { continue; }
            }
            return Err(invalid(at, "workflow.repositoryConnectionRequired"));
        }
        Ok(())
    }
    layer(&graph, previous.as_ref(), "")
}

pub(super) fn anonymous_git_url(raw: &str) -> bool {
    reqwest::Url::parse(raw).is_ok_and(|url| matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some() && url.username().is_empty() && url.password().is_none()
        && url.query().is_none() && url.fragment().is_none())
}

pub(crate) fn node_at<'a>(graph: &'a Value, path: &str) -> Option<&'a Value> {
    let suffix = path.strip_prefix("/nodes/")?;
    let (index, nested) = suffix
        .split_once('/')
        .map_or((suffix, None), |(index, nested)| (index, Some(nested)));
    if index.len() > 1 && index.starts_with('0') {
        return None;
    }
    if !index.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let node = graph["nodes"]
        .as_array()?
        .get(index.parse::<usize>().ok()?)?;
    match nested {
        None => Some(node),
        Some(nested) if node["type"] == "core.loop" => {
            node_at(&node["params"]["body"], nested.strip_prefix("params/body")?)
        }
        _ => None,
    }
}

/// Resolve a node's parameter batch before starting any node attempt or effect.
/// Callers supply the frozen authorized snapshot and explicit item selections;
/// None selects batch mode, so expressions can use $input.all() without an item.
pub(crate) async fn resolve(
    node: &Value,
    base: &str,
    data: ferry_protocol::sandbox::InputData,
    selections: &[Option<usize>],
    context: Value,
    deterministic: bool,
) -> Result<Vec<Value>> {
    resolve_mode(node, base, data, selections, context, deterministic, false).await
}

/// Runtime-only operands retain existence metadata in the encrypted snapshot.
/// Editor previews keep their original plain resolved-parameter contract.
pub(super) async fn resolve_attempt(
    node: &Value,
    base: &str,
    data: ferry_protocol::sandbox::InputData,
    selections: &[Option<usize>],
    context: Value,
    deterministic: bool,
) -> Result<Vec<Value>> {
    resolve_mode(node, base, data, selections, context, deterministic, true).await
}

async fn resolve_mode(
    node: &Value,
    base: &str,
    data: ferry_protocol::sandbox::InputData,
    selections: &[Option<usize>],
    context: Value,
    deterministic: bool,
    capture_operands: bool,
) -> Result<Vec<Value>> {
    use ferry_protocol::sandbox::{
        Batch, ErrorCode, Evaluation, MAX_EVALUATIONS, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES,
        Reply, VERSION,
    };
    let path = format!("{base}/params");
    let kind = node["type"].as_str().unwrap_or_default();
    let contract = contracts()
        .nodes
        .get(kind)
        .filter(|contract| contract.spec["type_version"].as_f64() == node["type_version"].as_f64())
        .ok_or_else(|| invalid(base, "workflow.graphUnknownNode"))?;
    if let Err(error) = contract.params.validate(&node["params"]) {
        return Err(invalid(
            format!("{path}{}", error.instance_path()),
            "workflow.graphSchema",
        ));
    }
    let mut expressions = Vec::new();
    collect_params(
        &node["params"],
        "",
        "",
        &expression_fields(&contract.spec),
        &mut expressions,
    );
    let limit = || super::expression::error(ErrorCode::InvalidRequest, &path);
    let count = selections
        .len()
        .checked_mul(expressions.len())
        .ok_or_else(limit)?;
    let source_bytes = expressions
        .iter()
        .map(|(_, source)| source.len())
        .sum::<usize>();
    if selections.len() > MAX_EVALUATIONS
        || count > MAX_EVALUATIONS
        || source_bytes
            .checked_mul(selections.len())
            .is_none_or(|bytes| bytes > MAX_REQUEST_BYTES)
        || expressions
            .iter()
            .any(|(_, source)| source.len() > 64 * 1024)
        || selections
            .iter()
            .flatten()
            .any(|index| *index >= data.items.len())
    {
        return Err(limit());
    }
    let operands = if capture_operands {
        super::condition::operand_paths(node)
    } else {
        vec![]
    };
    let evaluations = selections
        .iter()
        .flat_map(|index| {
            let operands = &operands;
            expressions.iter().map(move |(field, source)| Evaluation {
                expression: source.clone(),
                template: true,
                capture_presence: operands.contains(field),
                item_index: *index,
                context: json!({}),
            })
        })
        .collect();
    let batch = Batch {
        program: None,
        version: VERSION,
        timeout_ms: 1000,
        memory_bytes: 64 * 1024 * 1024,
        deterministic,
        syntax_only: false,
        data: Some(data),
        context: Some(context),
        evaluations,
    };
    if !batch.valid() {
        return Err(limit());
    }
    let mut values = if count == 0 {
        Vec::new().into_iter()
    } else {
        match super::expression::exchange(batch).await {
            Reply::Ok { values, .. } => values.into_iter(),
            Reply::Error { code, index, .. } => {
                let field = index
                    .and_then(|index| expressions.get(index % expressions.len()))
                    .map(|(field, _)| field.as_str())
                    .unwrap_or_default();
                return Err(super::expression::error(code, &format!("{path}{field}")));
            }
        }
    };
    let mut resolved = Vec::with_capacity(selections.len());
    let mut bytes = 2usize;
    for _ in selections {
        let mut params = node["params"].clone();
        for field in &operands {
            if !expressions
                .iter()
                .any(|(expression, _)| expression == field)
            {
                let value = params.pointer_mut(field).expect("operand path");
                *value = json!({"present":true,"value":value.clone()});
            }
        }
        for (field, _) in &expressions {
            let value = values.next().expect("sandbox reply cardinality checked");
            *params
                .pointer_mut(field)
                .expect("collected expression path") = value;
        }
        validate_resolved(node, base, &params)?;
        bytes = bytes
            .checked_add(serde_json::to_vec(&params).map_err(|_| limit())?.len() + 1)
            .filter(|bytes| *bytes <= MAX_RESPONSE_BYTES)
            .ok_or_else(limit)?;
        resolved.push(params);
    }
    Ok(resolved)
}

pub(crate) fn validate_resolved(node: &Value, base: &str, params: &Value) -> Result<()> {
    let kind = node["type"].as_str().unwrap_or_default();
    let contract = contracts()
        .nodes
        .get(kind)
        .filter(|contract| contract.spec["type_version"].as_f64() == node["type_version"].as_f64())
        .ok_or_else(|| invalid(base, "workflow.graphUnknownNode"))?;
    if let Err(error) = contract.resolved.validate(params) {
        return Err(invalid(
            format!("{base}/params{}", error.instance_path()),
            "workflow.graphSchema",
        ));
    }
    validate_semantics(kind, params, base)
}

fn validate_semantics(kind: &str, params: &Value, base: &str) -> Result<()> {
    if kind == "host.command" && params["command"].as_str().unwrap().contains("{{") {
        return Err(invalid(
            format!("{base}/params/command"),
            "workflow.graphCommand",
        ));
    }
    if kind == "core.switch" {
        let mut keys = BTreeSet::new();
        for case in params["cases"].as_array().unwrap() {
            if !keys.insert(case["key"].as_str().unwrap()) {
                return Err(invalid(
                    format!("{base}/params/cases"),
                    "workflow.graphSchema",
                ));
            }
        }
    }
    if kind == "data.text_split"
        && params["mode"] == "length"
        && params["overlap"].as_f64().unwrap_or(0.0) >= params["length"].as_f64().unwrap()
    {
        return Err(invalid(
            format!("{base}/params/overlap"),
            "workflow.graphSchema",
        ));
    }
    if kind == "human.pick" && params["min_selected"].as_f64() > params["max_selected"].as_f64() {
        return Err(invalid(
            format!("{base}/params/max_selected"),
            "workflow.graphSchema",
        ));
    }
    Ok(())
}

fn validate_layer(graph: &Value, base: &str, nesting: usize, total: &mut usize) -> Result<()> {
    let registry = contracts();
    if let Err(error) = registry.graph.validate(graph) {
        return Err(invalid(
            format!("{base}{}", error.instance_path()),
            "workflow.graphSchema",
        ));
    }
    // Shape and numeric bounds are established by the trusted graph schema.
    let nodes = graph["nodes"].as_array().unwrap();
    super::expression::timezone(&graph["settings"], &format!("{base}/settings/timezone"))?;
    *total += nodes.len();
    if *total > MAX_NODES {
        return Err(invalid(format!("{base}/nodes"), "workflow.graphLimit"));
    }
    let mut lookup = BTreeMap::new();
    for (index, node) in nodes.iter().enumerate() {
        let path = format!("{base}/nodes/{index}");
        let id = node["id"].as_str().unwrap();
        if lookup.insert(id, node).is_some() {
            return Err(invalid(path, "workflow.graphDuplicateNode"));
        }
        let kind = node["type"].as_str().unwrap();
        let contract = registry
            .nodes
            .get(kind)
            .filter(|contract| {
                contract.spec["type_version"].as_f64() == node["type_version"].as_f64()
            })
            .ok_or_else(|| invalid(&path, "workflow.graphUnknownNode"))?;
        if let Err(error) = contract.params.validate(&node["params"]) {
            return Err(invalid(
                format!("{path}/params{}", error.instance_path()),
                "workflow.graphSchema",
            ));
        }
        let settings = &node["settings"];
        let runtime = settings
            .get("runtime")
            .filter(|value| !value.is_null())
            .unwrap_or(&contract.spec["runtime"]["default"]);
        if !contract.spec["runtime"]["allowed"]
            .as_array()
            .unwrap()
            .contains(runtime)
        {
            return Err(invalid(
                format!("{path}/settings/runtime"),
                "workflow.graphRuntime",
            ));
        }
        if contract.spec["cache_allowed"] == false
            && settings.get("cache").is_some_and(|mode| mode != "off")
        {
            return Err(invalid(
                format!("{path}/settings/cache"),
                "workflow.graphCache",
            ));
        }
        if settings["purity"] == "pure"
            && kind == "code.javascript"
            && node["params"]["deterministic"] != true
        {
            return Err(invalid(
                format!("{path}/settings/purity"),
                "workflow.graphCache",
            ));
        }
        if let Some(credentials) = node.get("credentials").and_then(Value::as_object) {
            for (slot, id) in credentials {
                if contract.spec["credential_slots"].get(slot).is_none()
                    || id
                        .as_str()
                        .and_then(|id| id.parse::<i64>().ok())
                        .is_none_or(|id| id <= 0)
                {
                    return Err(invalid(
                        format!("{path}/credentials"),
                        "workflow.graphCredential",
                    ));
                }
            }
        }
        let params = &node["params"];
        validate_semantics(kind, params, &path)?;
        if kind == "core.loop" {
            if nesting >= 3 {
                return Err(invalid(&path, "workflow.graphLimit"));
            }
            let nested = json!({"engine":"ferry-graph", "schema":1,
                "nodes":params["body"]["nodes"], "connections":params["body"]["connections"]});
            validate_layer(&nested, &format!("{path}/params/body"), nesting + 1, total)?;
        }
    }
    let mut edges = BTreeSet::new();
    let mut adjacency: BTreeMap<&str, Vec<&str>> =
        lookup.keys().map(|id| (*id, Vec::new())).collect();
    let mut indegree: BTreeMap<&str, usize> = lookup.keys().map(|id| (*id, 0)).collect();
    for (index, connection) in graph["connections"].as_array().unwrap().iter().enumerate() {
        let path = format!("{base}/connections/{index}");
        let endpoint = |side: &str| {
            let endpoint = &connection[side];
            (
                endpoint["node"].as_str().unwrap(),
                endpoint["port"].as_str().unwrap(),
                port_index(&endpoint["index"]),
            )
        };
        let source = endpoint("from");
        let target = endpoint("to");
        if !edges.insert((source, target)) {
            return Err(invalid(&path, "workflow.graphDuplicateEdge"));
        }
        for (side, direction) in [("from", "outputs"), ("to", "inputs")] {
            let (id, port, number) = endpoint(side);
            let node = lookup
                .get(id)
                .ok_or_else(|| invalid(&path, "workflow.graphDangling"))?;
            let spec = &registry.nodes[node["type"].as_str().unwrap()].spec;
            if !spec[direction]
                .as_array()
                .unwrap()
                .iter()
                .any(|declared| declared["port"] == port)
            {
                return Err(invalid(&path, "workflow.graphPort"));
            }
            let valid_index =
                if direction == "outputs" && node["type"] == "core.switch" && port == "case" {
                    number.is_some_and(|number| {
                        number < node["params"]["cases"].as_array().unwrap().len() as u64
                    })
                } else if direction == "inputs" && node["type"] == "core.merge" {
                    number.is_some_and(|number| {
                        port_index(&node["params"]["input_count"]).map_or_else(
                            || (number as f64) < node["params"]["input_count"].as_f64().unwrap(),
                            |count| number < count,
                        )
                    })
                } else {
                    number == Some(0)
                };
            if !valid_index
                || (direction == "outputs"
                    && node["type"] == "core.switch"
                    && port == "fallback"
                    && node["params"]["fallback"] != true)
            {
                return Err(invalid(&path, "workflow.graphPort"));
            }
        }
        adjacency.get_mut(source.0).unwrap().push(target.0);
        *indegree.get_mut(target.0).unwrap() += 1;
    }
    let mut ready: VecDeque<_> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut visited = 0;
    while let Some(id) = ready.pop_front() {
        visited += 1;
        for target in &adjacency[id] {
            let degree = indegree.get_mut(target).unwrap();
            *degree -= 1;
            if *degree == 0 {
                ready.push_back(*target);
            }
        }
    }
    if visited != lookup.len() {
        return Err(invalid(
            format!("{base}/connections"),
            "workflow.graphCycle",
        ));
    }
    if graph
        .get("pin_data")
        .and_then(Value::as_object)
        .is_some_and(|pins| pins.keys().any(|id| !lookup.contains_key(id.as_str())))
    {
        return Err(invalid(format!("{base}/pin_data"), "workflow.graphPin"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_auth_policy_rejects_new_legacy_and_preserves_only_saved_identity() {
        let legacy = node("repo", "integration.github.repository.get");
        let previous = json!({"nodes":[legacy.clone()]}).to_string();
        resolved_error(repository_auth_policy(&previous,None).unwrap_err(),
            "workflow.repositoryConnectionRequired", "/nodes/0/params");
        assert!(repository_auth_policy(&previous,Some(&previous)).is_ok());
        let mut changed = legacy.clone();
        changed["params"]["credential_id"] = json!("123");
        assert!(repository_auth_policy(&json!({"nodes":[changed]}).to_string(),Some(&previous)).is_err());
        let mut copied=legacy.clone();copied["id"]=json!("copy");
        assert!(repository_auth_policy(&json!({"nodes":[legacy.clone(),copied]}).to_string(),Some(&previous)).is_err());
        let mut connected=legacy.clone();connected["params"]["connection_id"]=json!("123");
        assert!(repository_auth_policy(&json!({"nodes":[connected.clone()]}).to_string(),None).is_ok());
        assert!(repository_auth_policy(&previous,Some(&json!({"nodes":[connected]}).to_string())).is_err());
        let nested=json!({"nodes":[{"id":"loop","type":"core.loop","params":{"body":{"nodes":[legacy]}}}]}).to_string();
        resolved_error(repository_auth_policy(&nested,None).unwrap_err(),
            "workflow.repositoryConnectionRequired", "/nodes/0/params/body/nodes/0/params");
        assert!(repository_auth_policy(&nested,Some(&nested)).is_ok());
    }

    #[test]
    fn repository_auth_policy_requires_explicit_anonymous_http_and_no_local_connection() {
        let mut git=node("git","integration.git.repository.clone");
        assert!(repository_auth_policy(&json!({"nodes":[git.clone()]}).to_string(),None).is_err());
        git["params"]["auth_mode"]=json!("anonymous");
        for url in ["https://example.invalid/repo.git","http://example.invalid/repo.git"] {
            git["params"]["url"]=json!(url);
            assert!(repository_auth_policy(&json!({"nodes":[git.clone()]}).to_string(),None).is_ok());
        }
        for url in ["ssh://git@example.invalid/repo","https://token@example.invalid/repo","https://u:p@example.invalid/repo","file:///repo","https://example.invalid/repo?token=secret"] {
            git["params"]["url"]=json!(url);
            assert!(repository_auth_policy(&json!({"nodes":[git.clone()]}).to_string(),None).is_err());
        }
        assert!(repository_auth_policy(&json!({"nodes":[node("status","integration.git.repository.status")]}).to_string(),None).is_ok());
    }

    fn inputs(values: &[Value]) -> ferry_protocol::sandbox::InputData {
        serde_json::from_value(json!({"items":values.iter().map(|value|json!({"json":{"value":value}})).collect::<Vec<_>>(),"upstream":[]})).unwrap()
    }

    fn resolved_error(error: ApiError, key: &str, path: &str) {
        let message = error.2.as_ref().unwrap();
        assert_eq!(message.key, key, "{error:?}");
        assert_eq!(message.params["path"], path);
        for locale in crate::i18n::Locale::ALL {
            let text = crate::i18n::render(locale, message);
            assert!(
                !text.contains("workflow.") && !text.contains("PRIVATE"),
                "{text}"
            );
        }
    }

    #[tokio::test]
    async fn resolved_literals_accept_all_catalog_examples_without_starting_a_sandbox() {
        for kind in contracts().nodes.keys() {
            let mut node = node("test", kind);
            let mut expressions = Vec::new();
            collect_params(
                &node["params"],
                "",
                "",
                &expression_fields(&contracts().nodes[kind].spec),
                &mut expressions,
            );
            for (path, _) in expressions {
                *node["params"].pointer_mut(&path).unwrap() = json!("literal input");
            }
            let result = resolve(&node, "/nodes/0", inputs(&[]), &[None], json!({}), true)
                .await
                .unwrap();
            assert_eq!(result, vec![node["params"].clone()], "{kind}");
        }
        let mut node = node("test", "data.set");
        node["type_version"] = json!(1.0);
        assert!(
            resolve(&node, "/nodes/0", inputs(&[]), &[], json!({}), true)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn resolved_limits_and_literal_semantics_fail_before_effects() {
        let mut stop = node("stop", "core.stop");
        stop["params"]["message"] = json!("x".repeat(512 * 1024));
        resolved_error(
            resolve(&stop, "/nodes/0", inputs(&[]), &[None; 20], json!({}), true)
                .await
                .unwrap_err(),
            "workflow.expressionLimit",
            "/nodes/0/params",
        );
        let mut command = node("command", "host.command");
        command["params"]["command"] = json!("echo {{ $json.secret }}");
        resolved_error(
            resolve(&command, "/nodes/0", inputs(&[]), &[None], json!({}), true)
                .await
                .unwrap_err(),
            "workflow.graphCommand",
            "/nodes/0/params/command",
        );
        let mut switch = node("switch", "core.switch");
        let duplicate = switch["params"]["cases"][0].clone();
        switch["params"]["cases"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        resolved_error(
            resolve(&switch, "/nodes/0", inputs(&[]), &[None], json!({}), true)
                .await
                .unwrap_err(),
            "workflow.graphSchema",
            "/nodes/0/params/cases",
        );
        let mut split = node("split", "data.text_split");
        split["params"] = json!({"value":"abc","mode":"length","length":2,"overlap":2});
        resolved_error(
            resolve(&split, "/nodes/0", inputs(&[]), &[None], json!({}), true)
                .await
                .unwrap_err(),
            "workflow.graphSchema",
            "/nodes/0/params/overlap",
        );
        let mut wait = node("wait", "core.wait");
        wait["params"]["duration_ms"] = json!("={{$json.value}}");
        resolved_error(
            resolve(
                &wait,
                "/nodes/0",
                inputs(&[json!(1)]),
                &vec![Some(0); 20001],
                json!({}),
                true,
            )
            .await
            .unwrap_err(),
            "workflow.expressionLimit",
            "/nodes/0/params",
        );
        resolved_error(
            resolve(&wait, "/nodes/0", inputs(&[]), &[Some(0)], json!({}), true)
                .await
                .unwrap_err(),
            "workflow.expressionLimit",
            "/nodes/0/params",
        );
        resolved_error(
            resolve(
                &wait,
                "/nodes/0",
                inputs(&[json!(1)]),
                &[Some(0)],
                json!({"$env":{"secret":"PRIVATE"}}),
                true,
            )
            .await
            .unwrap_err(),
            "workflow.expressionLimit",
            "/nodes/0/params",
        );
    }

    #[tokio::test]
    #[ignore = "requires a built real ferry-sandbox via absolute FERRY_SANDBOX_PATH"]
    async fn real_sandbox_resolves_node_batches_and_rejects_invalid_resolved_parameters() {
        assert!(std::path::Path::new(&std::env::var_os("FERRY_SANDBOX_PATH").unwrap()).is_file());
        let mut wait = node("wait", "core.wait");
        wait["params"]["duration_ms"] = json!("={{$json.value * $vars.factor}}");
        let original = wait.clone();
        let result = resolve(
            &wait,
            "/nodes/0",
            inputs(&[json!(5), json!(9)]),
            &[Some(0), Some(1)],
            json!({"$vars":{"factor":2}}),
            true,
        )
        .await
        .unwrap();
        assert_eq!(
            result,
            vec![
                json!({"mode":"duration","duration_ms":10}),
                json!({"mode":"duration","duration_ms":18})
            ]
        );
        assert_eq!(wait, original);
        println!("PASS typed per-item resolution shares frozen metadata and preserves the draft");
        wait["params"]["duration_ms"] = json!("={{$json.value}}");
        for value in [json!("5"), json!(-1), json!(null), json!({"PRIVATE":1})] {
            resolved_error(
                resolve(
                    &wait,
                    "/nodes/0",
                    inputs(&[json!(5), value]),
                    &[Some(0), Some(1)],
                    json!({}),
                    true,
                )
                .await
                .unwrap_err(),
                "workflow.graphSchema",
                "/nodes/0/params/duration_ms",
            );
        }
        println!("PASS later-item type and range failures reject the whole batch without coercion");
        let mut set = node("set", "data.set");
        set["params"]["fields"][0]["path"] = json!("=literal {{ invalid + }}");
        set["params"]["fields"][0]["value"] = json!("={{$json.value}}");
        let payload = json!({"nested":[true,null],"literal":"={{ do_not_evaluate_again }}"});
        let resolved = resolve(
            &set,
            "/nodes/1",
            inputs(&[payload.clone()]),
            &[Some(0)],
            json!({}),
            true,
        )
        .await
        .unwrap();
        assert_eq!(resolved[0]["fields"][0]["value"], payload);
        assert_eq!(
            resolved[0]["fields"][0]["path"],
            set["params"]["fields"][0]["path"]
        );
        println!("PASS only whitelisted fields evaluate once and preserve JSON result types");
        set["params"]["fields"][0]["value"] = json!("={{$input.all().map(x=>x.json.value)}}");
        assert_eq!(
            resolve(
                &set,
                "/nodes/1",
                inputs(&[json!(1), json!(2)]),
                &[None],
                json!({}),
                true
            )
            .await
            .unwrap()[0]["fields"][0]["value"],
            json!([1, 2])
        );
        set["params"]["fields"][0]["value"] = json!("={{$input.item}}");
        resolved_error(
            resolve(&set, "/nodes/1", inputs(&[]), &[None], json!({}), true)
                .await
                .unwrap_err(),
            "workflow.expressionMissingItem",
            "/nodes/1/params/fields/0/value",
        );
        println!(
            "PASS batch-mode all input access works and missing current-item access stays explicit"
        );
        for (source, key) in [
            ("={{1+}}", "workflow.expressionSyntax"),
            (
                "={{(()=>{throw new Error('PRIVATE_SOURCE')})()}}",
                "workflow.expressionFailed",
            ),
            ("={{(()=>{while(true){}})()}}", "workflow.expressionLimit"),
        ] {
            set["params"]["fields"][0]["value"] = json!(source);
            resolved_error(
                resolve(
                    &set,
                    "/nodes/1",
                    inputs(&[json!(1)]),
                    &[Some(0)],
                    json!({}),
                    true,
                )
                .await
                .unwrap_err(),
                key,
                "/nodes/1/params/fields/0/value",
            );
        }
        println!(
            "PASS syntax, exceptions and timeouts retain localized field paths without source leakage"
        );
        let mut http = node("http", "net.http");
        http["params"]["headers"] = json!({"a/b~c":"={{42}}"});
        resolved_error(
            resolve(
                &http,
                "/nodes/2",
                inputs(&[json!(1)]),
                &[Some(0)],
                json!({}),
                true,
            )
            .await
            .unwrap_err(),
            "workflow.graphSchema",
            "/nodes/2/params/headers/a~1b~0c",
        );
        println!("PASS resolved map-value schemas retain escaped JSON Pointer error paths");
        set["params"]["fields"][0]["value"] = json!("={{$json.value}}");
        let data = (0..1000).map(|value| json!(value)).collect::<Vec<_>>();
        let selections = (0..1000).map(Some).collect::<Vec<_>>();
        let result = resolve(
            &set,
            "/nodes/1",
            inputs(&data),
            &selections,
            json!({"$vars":{"large":"x".repeat(20000)}}),
            true,
        )
        .await
        .unwrap();
        assert_eq!(result.len(), 1000);
        assert_eq!(result[999]["fields"][0]["value"], json!(999));
        println!("PASS 1000 per-item resolutions transmit a large workspace context only once");
    }

    fn node(id: &str, kind: &str) -> Value {
        json!({"id":id,"type":kind,"type_version":1,"name":id,
            "params":contracts().nodes[kind].spec["example_params"],"position":[0,0]})
    }
    fn graph() -> Value {
        json!({"engine":"ferry-graph","schema":1,
            "nodes":[node("start","core.trigger.manual"),node("set","data.set")],
            "connections":[edge("start",0,"set",0)]})
    }

    #[test]
    fn preview_node_paths_only_address_nodes_in_validated_loop_scopes() {
        let mut graph = graph();
        let mut outer = node("loop", "core.loop");
        outer["params"]["body"] = json!({"nodes":[node("inner", "data.set")],"connections":[]});
        graph["nodes"][1] = outer;
        assert_eq!(
            node_at(&graph, "/nodes/1/params/body/nodes/0").unwrap()["id"],
            json!("inner")
        );
        for path in [
            "/nodes/+1",
            "/nodes/01",
            "/nodes/1/params",
            "/nodes/0/params/body/nodes/0",
            "/nodes/1/params/body/nodes/0/params",
        ] {
            assert!(node_at(&graph, path).is_none(), "{path}");
        }
    }
    fn edge(source: &str, output: u64, target: &str, input: u64) -> Value {
        json!({"from":{"node":source,"port":"main","index":output},
            "to":{"node":target,"port":"main","index":input}})
    }
    fn accepts(graph: &Value) {
        validate(&graph.to_string()).unwrap();
    }
    fn rejects(graph: &Value, key: &str) {
        let error = validate(&graph.to_string()).unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        assert_eq!(error.2.as_ref().unwrap().key, key);
    }

    #[test]
    fn draft_contracts_accept_all_catalog_examples_and_reject_unknown_parameters() {
        assert_eq!(contracts().nodes.len(), 182);
        for kind in contracts().nodes.keys() {
            let mut graph = json!({"engine":"ferry-graph","schema":1,"nodes":[node("test",kind)],"connections":[]});
            accepts(&graph);
            graph["nodes"][0]["params"]["unknown_option"] = json!(true);
            rejects(&graph, "workflow.graphSchema");
        }
    }

    #[test]
    fn expression_contracts_keep_literal_types_and_collect_only_declared_paths() {
        let mut wait = node("wait", "core.wait");
        wait["params"]["duration_ms"] = json!("={{ $json.duration }}");
        let mut graph = json!({"engine":"ferry-graph","schema":1,"nodes":[wait],"connections":[]});
        accepts(&graph);
        graph["nodes"][0]["params"]["duration_ms"] = json!("wrong literal type");
        rejects(&graph, "workflow.graphSchema");
        graph["nodes"][0]["params"]["duration_ms"] = json!(1000);
        graph["nodes"][0]["params"]["mode"] = json!("={{ 'duration' }}");
        rejects(&graph, "workflow.graphSchema");

        let mut inner = node("set", "data.set");
        inner["params"]["fields"][0]["value"] = json!("={{ missing.value }}");
        inner["settings"] = json!({"notes":"={{1+}}"});
        let mut outer = node("loop", "core.loop");
        outer["params"]["body"] = json!({"nodes":[inner],"connections":[]});
        let graph = json!({"engine":"ferry-graph","schema":1,"nodes":[outer],"connections":[]});
        accepts(&graph);
        let mut expressions = Vec::new();
        collect_graph(&graph, "", &mut expressions);
        assert_eq!(
            expressions,
            vec![(
                "/nodes/0/params/body/nodes/0/params/fields/0/value".to_owned(),
                "={{ missing.value }}".to_owned(),
            )]
        );

        let mut expressions = Vec::new();
        collect_params(
            &json!({"a/b~c":{"value":"=42"},"literal":"=invalid+"}),
            "",
            "/params",
            &["/*/value".to_owned()],
            &mut expressions,
        );
        assert_eq!(
            expressions,
            vec![("/params/a~1b~0c/value".to_owned(), "=42".to_owned())]
        );
    }

    #[test]
    fn shape_identity_ports_and_cycles_reject_bad_drafts() {
        let mut bad = graph();
        bad["extra"] = json!(true);
        rejects(&bad, "workflow.graphSchema");
        let mut bad = graph();
        bad["nodes"][1]["id"] = json!("start");
        rejects(&bad, "workflow.graphDuplicateNode");
        let mut bad = graph();
        bad["nodes"][0]["type_version"] = json!(99);
        rejects(&bad, "workflow.graphUnknownNode");
        let mut bad = graph();
        bad["nodes"][0]["id"] = json!("Bad-ID");
        rejects(&bad, "workflow.graphSchema");
        let mut bad = graph();
        bad["connections"][0]["to"]["node"] = json!("missing");
        rejects(&bad, "workflow.graphDangling");
        let mut bad = graph();
        bad["connections"][0]["from"]["port"] = json!("missing");
        rejects(&bad, "workflow.graphPort");
        let mut bad = graph();
        bad["connections"][0]["to"]["index"] = json!(1);
        rejects(&bad, "workflow.graphPort");
        let mut bad = graph();
        bad["connections"][0]["to"]["index"] = json!(1e100);
        rejects(&bad, "workflow.graphPort");
        let mut bad = graph();
        let duplicate = bad["connections"][0].clone();
        bad["connections"].as_array_mut().unwrap().push(duplicate);
        rejects(&bad, "workflow.graphDuplicateEdge");
        let mut bad = graph();
        bad["nodes"][0] = node("start", "data.set");
        bad["connections"]
            .as_array_mut()
            .unwrap()
            .push(edge("set", 0, "start", 0));
        rejects(&bad, "workflow.graphCycle");
        let mut bad = graph();
        bad["pin_data"] = json!({"missing":[]});
        rejects(&bad, "workflow.graphPin");
        // JSON Schema's integer type includes integral JSON floating numbers.
        let mut valid = graph();
        valid["nodes"][0]["type_version"] = json!(1.0);
        valid["connections"][0]["to"]["index"] = json!(0.0);
        accepts(&valid);
    }

    #[test]
    fn runtime_cache_credentials_and_command_rules_apply_without_demanding_runtime_bindings() {
        let mut bad = graph();
        bad["nodes"][1]["settings"] = json!({"runtime":"host"});
        rejects(&bad, "workflow.graphRuntime");
        let mut bad = graph();
        bad["nodes"][0]["settings"] = json!({"cache":"auto"});
        rejects(&bad, "workflow.graphCache");
        let mut bad = graph();
        bad["nodes"][1]["credentials"] = json!({"unknown":"7"});
        rejects(&bad, "workflow.graphCredential");
        let mut command = json!({"engine":"ferry-graph","schema":1,"nodes":[node("command","host.command")],"connections":[]});
        accepts(&command); // Drafts may be saved before choosing a host.
        command["nodes"][0]["params"]["command"] = json!("echo {{ $json.password }}");
        rejects(&command, "workflow.graphCommand");
    }

    #[test]
    fn switch_merge_indices_and_unconnected_branches_have_distinct_rules() {
        let mut graph = json!({"engine":"ferry-graph","schema":1,
            "nodes":[node("switch","core.switch"),node("merge","core.merge")],
            "connections":[{"from":{"node":"switch","port":"case","index":0},"to":{"node":"merge","port":"input","index":1}}]});
        accepts(&graph);
        graph["connections"][0]["to"]["index"] = json!(2);
        rejects(&graph, "workflow.graphPort");
        graph["connections"][0]["to"]["index"] = json!(1);
        graph["connections"][0]["from"]["index"] = json!(
            graph["nodes"][0]["params"]["cases"]
                .as_array()
                .unwrap()
                .len()
        );
        rejects(&graph, "workflow.graphPort");
        graph["connections"] = json!([]);
        accepts(&graph); // No fabricated branch sinks.
    }

    #[test]
    fn nested_loop_scopes_count_toward_global_node_and_depth_limits() {
        let mut body = json!({"nodes":[],"connections":[]});
        for _ in 0..3 {
            let mut loop_node = node("loop", "core.loop");
            loop_node["params"]["body"] = body;
            body = json!({"nodes":[loop_node],"connections":[]});
        }
        let mut graph =
            json!({"engine":"ferry-graph","schema":1,"nodes":body["nodes"],"connections":[]});
        accepts(&graph);
        let mut extra = node("loop", "core.loop");
        extra["params"]["body"] = body;
        graph["nodes"] = json!([extra]);
        rejects(&graph, "workflow.graphLimit");
        let mut graph = json!({"engine":"ferry-graph","schema":1,"nodes":[],"connections":[]});
        for i in 0..500 {
            graph["nodes"]
                .as_array_mut()
                .unwrap()
                .push(node(&format!("n{i}"), "data.set"));
        }
        accepts(&graph);
        graph["nodes"]
            .as_array_mut()
            .unwrap()
            .push(node("overflow", "data.set"));
        rejects(&graph, "workflow.graphSchema");
        graph["nodes"].as_array_mut().unwrap().pop();
        let mut extra = node("container", "core.loop");
        extra["params"]["body"]["nodes"] = json!([node("nested", "data.set")]);
        graph["nodes"][0] = extra;
        rejects(&graph, "workflow.graphLimit");
    }

    #[test]
    fn unicode_pin_bytes_and_definition_bytes_obey_exact_boundaries() {
        let mut graph = graph();
        let mut pins = json!({"set":[{"json":{"value":""}}]});
        let overhead = serde_json::to_vec(&pins).unwrap().len();
        let length = MAX_PIN_BYTES - overhead;
        pins["set"][0]["json"]["value"] = json!("版".repeat(length / 3) + &"x".repeat(length % 3));
        assert_eq!(serde_json::to_vec(&pins).unwrap().len(), MAX_PIN_BYTES);
        graph["pin_data"] = pins;
        accepts(&graph);
        let text = graph["pin_data"]["set"][0]["json"]["value"]
            .as_str()
            .unwrap()
            .to_owned()
            + "x";
        graph["pin_data"]["set"][0]["json"]["value"] = json!(text);
        rejects(&graph, "workflow.graphLimit");
        graph.as_object_mut().unwrap().remove("pin_data");
        graph["nodes"][0]["settings"] = json!({"notes":""});
        let overhead = graph.to_string().len();
        graph["nodes"][0]["settings"]["notes"] = json!("x".repeat(MAX_DEFINITION_BYTES - overhead));
        assert_eq!(graph.to_string().len(), MAX_DEFINITION_BYTES);
        accepts(&graph);
        graph["nodes"][0]["settings"]["notes"] =
            json!("x".repeat(MAX_DEFINITION_BYTES - overhead + 1));
        rejects(&graph, "workflow.graphLimit");
    }
}
