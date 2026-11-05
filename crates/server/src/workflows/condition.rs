//! Conditional item routing over frozen operands. Missing is internal metadata,
//! not a user JSON value; regular expressions execute only in the sandbox.
use super::journal::{Output, Port};
use crate::{
    error::{ApiError, Result},
    i18n::Message,
};
use axum::http::StatusCode;
use ferry_protocol::sandbox::{Batch, Evaluation, Item, Reply, VERSION};
use serde_json::{Value, json};
use std::cmp::Ordering;

fn invalid() -> ApiError {
    ApiError::localized(
        StatusCode::BAD_REQUEST,
        Message::new("workflow.graphSchema").with("path", "/params/rules"),
    )
}

pub(super) fn supports(node: &Value) -> bool {
    matches!(
        node["type"].as_str(),
        Some("core.filter" | "core.if" | "core.switch")
    )
}

pub(super) fn operand_paths(node: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    let mut collect = |condition: &Value, base: &str| {
        if let Some(rules) = condition["rules"].as_array() {
            for (index, rule) in rules.iter().enumerate() {
                for side in ["left", "right"] {
                    if rule.get(side).is_some() {
                        paths.push(format!("{base}/rules/{index}/{side}"));
                    }
                }
            }
        }
    };
    if node["type"] == "core.switch" {
        if let Some(cases) = node["params"]["cases"].as_array() {
            for (index, case) in cases.iter().enumerate() {
                collect(&case["condition"], &format!("/cases/{index}/condition"));
            }
        }
    } else if supports(node) {
        collect(&node["params"], "");
    }
    paths
}

fn operand(value: &Value) -> Result<Option<&Value>> {
    let object = value.as_object().ok_or_else(invalid)?;
    match object.get("present").and_then(Value::as_bool) {
        Some(false) if object.len() == 1 => Ok(None),
        Some(true) if object.len() == 2 && object.contains_key("value") => Ok(object.get("value")),
        _ => Err(invalid()),
    }
}

fn compare(left: Option<&Value>, right: Option<&Value>, operator: &str) -> Result<bool> {
    if operator == "exists" {
        return Ok(left.is_some());
    }
    if matches!(operator, "eq" | "ne") {
        let equal = match (left, right) {
            (None, None) => true,
            (None, Some(_)) | (Some(_), None) => false,
            (Some(Value::Null), Some(Value::Null)) => true,
            (Some(Value::Null), _) | (_, Some(Value::Null)) => false,
            (Some(left), Some(right)) => super::transform::scalar(left, right)?.is_eq(),
        };
        return Ok(if operator == "eq" { equal } else { !equal });
    }
    let (left, right) = (left.ok_or_else(invalid)?, right.ok_or_else(invalid)?);
    if operator == "contains" {
        return Ok(left
            .as_str()
            .ok_or_else(invalid)?
            .contains(right.as_str().ok_or_else(invalid)?));
    }
    let order = super::transform::scalar(left, right)?;
    match operator {
        "gt" => Ok(order == Ordering::Greater),
        "gte" => Ok(order != Ordering::Less),
        "lt" => Ok(order == Ordering::Less),
        "lte" => Ok(order != Ordering::Greater),
        _ => Err(invalid()),
    }
}

fn conditions(params: &Value, switch: bool) -> Result<Vec<&Value>> {
    if switch {
        Ok(params["cases"]
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .map(|case| &case["condition"])
            .collect())
    } else {
        Ok(vec![params])
    }
}

/// Declared order also defines the indices used by paired provenance lookup.
pub(super) fn ports(node: &Value) -> Result<Vec<Port>> {
    let declared = super::graph::manifest(node).ok_or_else(invalid)?;
    let mut result = Vec::new();
    for entry in declared["outputs"].as_array().ok_or_else(invalid)? {
        let name = entry["port"].as_str().ok_or_else(invalid)?;
        let count = if node["type"] == "core.switch" && name == "case" {
            node["params"]["cases"]
                .as_array()
                .ok_or_else(invalid)?
                .len()
        } else {
            1
        };
        for index in 0..count {
            result.push(Port {
                name: name.into(),
                index,
                skipped: true,
                items: vec![],
            });
        }
    }
    Ok(result)
}

pub(super) async fn execute(node: &Value, items: &[Item], params: &[Value]) -> Result<Output> {
    if items.len() != params.len() {
        return Err(invalid());
    }
    let switch = node["type"] == "core.switch";
    // One bounded exchange for every regex in this attempt, never one process
    // per item/rule. Tenant text and patterns are JSON constants, not JS source.
    let mut evaluations = Vec::new();
    for params in params {
        for condition in conditions(params, switch)? {
            for rule in condition["rules"].as_array().ok_or_else(invalid)? {
                if rule["operator"] == "matches" {
                    let left = operand(&rule["left"])?
                        .and_then(Value::as_str)
                        .ok_or_else(invalid)?;
                    let right = operand(&rule["right"])?
                        .and_then(Value::as_str)
                        .ok_or_else(invalid)?;
                    evaluations.push(Evaluation {
                        expression: format!("new RegExp({}).test({})", json!(right), json!(left)),
                        template: false,
                        capture_presence: false,
                        item_index: None,
                        context: json!({}),
                    });
                }
            }
        }
    }
    let mut matches = if evaluations.is_empty() {
        Vec::new().into_iter()
    } else {
        match super::expression::exchange(Batch {
            program: None,
            version: VERSION,
            timeout_ms: 1000,
            memory_bytes: 64 * 1024 * 1024,
            deterministic: true,
            syntax_only: false,
            data: None,
            context: None,
            evaluations,
        })
        .await
        {
            Reply::Ok { values, .. } => values.into_iter(),
            Reply::Error { code, .. } => {
                return Err(super::expression::error(code, "/params/rules"));
            }
        }
    };
    let mut output = Output {
        version: 1,
        ports: ports(node)?,
    };
    if node["type"] == "core.filter" {
        output.ports[0].skipped = false;
    }
    let mut bytes = 0;
    for (item, params) in items.iter().zip(params) {
        let mut results = Vec::new();
        for condition in conditions(params, switch)? {
            let and = match condition["combinator"].as_str() {
                Some("and") => true,
                Some("or") => false,
                _ => return Err(invalid()),
            };
            let mut accepted = and;
            for rule in condition["rules"].as_array().ok_or_else(invalid)? {
                let operator = rule["operator"].as_str().ok_or_else(invalid)?;
                let value = if operator == "matches" {
                    matches
                        .next()
                        .and_then(|value| value.as_bool())
                        .ok_or_else(invalid)?
                } else {
                    let left = operand(&rule["left"])?;
                    let right = if operator == "exists" {
                        None
                    } else {
                        operand(&rule["right"])?
                    };
                    compare(left, right, operator)?
                };
                if and {
                    accepted &= value;
                } else {
                    accepted |= value;
                }
            }
            results.push(accepted);
        }
        let targets: Vec<usize> = if switch {
            let mut selected: Vec<_> = results
                .iter()
                .enumerate()
                .filter_map(|(index, value)| value.then_some(index))
                .collect();
            if params["match"] == "first" {
                selected.truncate(1);
            }
            if selected.is_empty() && params["fallback"] == true {
                selected.push(results.len());
            }
            selected
        } else if node["type"] == "core.if" {
            vec![if results[0] { 0 } else { 1 }]
        } else if results[0] {
            vec![0]
        } else {
            vec![]
        };
        for target in targets {
            let port = &mut output.ports[target];
            port.skipped = false;
            super::transform::append(&mut port.items, &mut bytes, item.clone())?;
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests;
