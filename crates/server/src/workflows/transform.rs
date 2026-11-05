//! Pure item transformations. Inputs and resolved parameters come from the
//! durable attempt snapshot; these operations never fetch data or credentials.
use crate::{
    error::{ApiError, Result},
    i18n::Message,
};
use axum::http::StatusCode;
use ferry_protocol::sandbox::Item;
use serde_json::{Map, Number, Value, json};
use std::{cmp::Ordering, collections::BTreeSet};

fn invalid() -> ApiError {
    ApiError::localized(
        StatusCode::BAD_REQUEST,
        Message::new("workflow.graphSchema").with("path", "/params"),
    )
}

pub(super) fn supports(spec: &Value) -> bool {
    match spec["type"].as_str() {
        Some(
            "core.no_op" | "core.limit" | "core.sort" | "core.split" | "data.rename"
            | "data.text_split",
        ) => true,
        Some("core.deduplicate") => spec["params"]["scope"] == "execution",
        _ => false,
    }
}

pub(super) fn per_item(spec: &Value) -> bool {
    matches!(
        spec["type"].as_str(),
        Some("core.split" | "data.rename" | "data.text_split")
    )
}

pub(super) fn path(value: &str) -> Result<Vec<&str>> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.iter().any(|part| {
        part.is_empty()
            || matches!(*part, "__proto__" | "constructor" | "prototype")
            || part.contains(['[', ']', '\\'])
    }) {
        return Err(invalid());
    }
    Ok(parts)
}

fn read<'a>(object: &'a Map<String, Value>, parts: &[&str]) -> Option<&'a Value> {
    let mut value = object.get(parts[0])?;
    for part in &parts[1..] {
        value = value.as_object()?.get(*part)?;
    }
    Some(value)
}

pub(super) fn write(object: &mut Map<String, Value>, parts: &[&str], value: Value) -> Result<()> {
    let mut parent = object;
    for part in &parts[..parts.len() - 1] {
        parent = parent
            .entry((*part).to_owned())
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(invalid)?;
    }
    parent.insert(parts[parts.len() - 1].to_owned(), value);
    Ok(())
}

fn remove(object: &mut Map<String, Value>, parts: &[&str]) {
    let mut parent = object;
    for part in &parts[..parts.len() - 1] {
        let Some(next) = parent.get_mut(*part).and_then(Value::as_object_mut) else {
            return;
        };
        parent = next;
    }
    parent.remove(parts[parts.len() - 1]);
}

// Compare the JSON decimal values without rounding large integers to f64.
fn numeric(left: &Number, right: &Number) -> Ordering {
    fn decimal(value: &Number) -> (bool, i32, Vec<u8>) {
        let text = value.to_string();
        let negative = text.starts_with('-');
        let raw = text.trim_start_matches('-');
        let (mantissa, exponent) = raw.split_once(['e', 'E']).map_or((raw, 0), |(m, e)| {
            (m, e.parse::<i32>().expect("JSON exponent"))
        });
        let point = mantissa.find('.').unwrap_or(mantissa.len());
        let mut digits: Vec<_> = mantissa.bytes().filter(u8::is_ascii_digit).collect();
        let leading = digits.iter().take_while(|digit| **digit == b'0').count();
        let magnitude = point as i32 + exponent - leading as i32;
        digits.drain(..leading);
        while digits.last() == Some(&b'0') {
            digits.pop();
        }
        if digits.is_empty() {
            (false, 0, vec![])
        } else {
            (negative, magnitude, digits)
        }
    }
    let (ln, lm, ld) = decimal(left);
    let (rn, rm, rd) = decimal(right);
    let order = match (ld.is_empty(), rd.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => {
            if rn {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        (false, true) => {
            if ln {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
        (false, false) if ln != rn => {
            if ln {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
        _ => {
            let magnitude = lm.cmp(&rm).then_with(|| {
                (0..ld.len().max(rd.len()))
                    .map(|i| ld.get(i).unwrap_or(&b'0').cmp(rd.get(i).unwrap_or(&b'0')))
                    .find(|order| !order.is_eq())
                    .unwrap_or(Ordering::Equal)
            });
            if ln { magnitude.reverse() } else { magnitude }
        }
    };
    order
}

pub(super) fn scalar(left: &Value, right: &Value) -> Result<Ordering> {
    match (left, right) {
        (Value::Number(l), Value::Number(r)) => Ok(numeric(l, r)),
        (Value::String(l), Value::String(r)) => Ok(l.cmp(r)),
        (Value::Bool(l), Value::Bool(r)) => Ok(l.cmp(r)),
        _ => Err(invalid()),
    }
}

fn sort(items: &[Item], params: &Value) -> Result<Vec<Item>> {
    let mut result = items.to_vec();
    let keys = params["keys"].as_array().ok_or_else(invalid)?;
    let mut prepared = Vec::new();
    for key in keys {
        let parts = path(key["path"].as_str().ok_or_else(invalid)?)?;
        let mut first = None;
        for item in items {
            if let Some(value) = read(&item.json, &parts).filter(|v| !v.is_null()) {
                scalar(value, first.unwrap_or(value))?;
                first = Some(value);
            }
        }
        prepared.push((parts, key["direction"] == "desc", key["nulls"] == "first"));
    }
    result.sort_by(|left, right| {
        for (parts, desc, nulls_first) in &prepared {
            let l = read(&left.json, parts).filter(|v| !v.is_null());
            let r = read(&right.json, parts).filter(|v| !v.is_null());
            let order = match (l, r) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => {
                    if *nulls_first {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    }
                }
                (Some(_), None) => {
                    if *nulls_first {
                        Ordering::Greater
                    } else {
                        Ordering::Less
                    }
                }
                (Some(l), Some(r)) => {
                    let cmp = scalar(l, r).expect("validated comparable values");
                    if *desc { cmp.reverse() } else { cmp }
                }
            };
            if !order.is_eq() {
                return order;
            }
        }
        Ordering::Equal
    });
    Ok(result)
}

fn deduplicate(items: &[Item], params: &Value) -> Result<Vec<Item>> {
    let paths = params["keys"]
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .map(|key| path(key.as_str().ok_or_else(invalid)?))
        .collect::<Result<Vec<_>>>()?;
    let mut seen = BTreeSet::new();
    let mut indices = Vec::new();
    let mut scan = |index: usize| -> Result<()> {
        let item = &items[index];
        let value = if paths.is_empty() {
            json!(item.json)
        } else {
            // The presence marker distinguishes a missing field from JSON null.
            Value::Array(
                paths
                    .iter()
                    .map(|parts| match read(&item.json, parts) {
                        Some(value) => json!([true, value]),
                        None => json!([false]),
                    })
                    .collect(),
            )
        };
        let key =
            crate::definitions::canonical(&serde_json::to_string(&value).map_err(|_| invalid())?)?;
        if seen.insert(key) {
            indices.push(index);
        }
        Ok(())
    };
    if params["keep"] == "last" {
        for index in (0..items.len()).rev() {
            scan(index)?;
        }
    } else {
        for index in 0..items.len() {
            scan(index)?;
        }
    }
    indices.sort_unstable();
    Ok(indices
        .into_iter()
        .map(|index| items[index].clone())
        .collect())
}

fn rename(item: &Item, params: &Value) -> Result<Item> {
    let mappings = params["mappings"].as_array().ok_or_else(invalid)?;
    let mut result = item.clone();
    let mut changes = Vec::new();
    let mut targets: Vec<Vec<&str>> = Vec::new();
    for mapping in mappings {
        let from = path(mapping["from"].as_str().ok_or_else(invalid)?)?;
        let to = path(mapping["to"].as_str().ok_or_else(invalid)?)?;
        if let Some(value) = read(&item.json, &from) {
            if targets
                .iter()
                .any(|prior| prior.starts_with(&to) || to.starts_with(prior))
            {
                return Err(invalid());
            }
            targets.push(to.clone());
            changes.push((from, to, value.clone()));
        }
    }
    for (from, _, _) in &changes {
        remove(&mut result.json, from);
    }
    for (_, to, value) in changes {
        if params["on_collision"] == "error" && read(&result.json, &to).is_some() {
            return Err(invalid());
        }
        write(&mut result.json, &to, value)?;
    }
    Ok(result)
}

pub(super) fn append(result: &mut Vec<Item>, bytes: &mut usize, item: Item) -> Result<()> {
    let size = serde_json::to_vec(&item).map_err(|_| invalid())?.len();
    *bytes = bytes.checked_add(size).ok_or_else(invalid)?;
    if size > super::journal::MAX_ITEM_BYTES || *bytes > super::journal::MAX_OUTPUT_BYTES {
        return Err(invalid());
    }
    result.push(item);
    Ok(())
}

fn split(item: &Item, params: &Value, result: &mut Vec<Item>, bytes: &mut usize) -> Result<()> {
    let parts = path(params["path"].as_str().ok_or_else(invalid)?)?;
    let array = read(&item.json, &parts)
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let mut base = item.clone();
    if params["include"] == "none" {
        base.json.clear();
    }
    if params["include"] == "selected" {
        base.json.clear();
        for field in params["fields"].as_array().ok_or_else(invalid)? {
            let parts = path(field.as_str().ok_or_else(invalid)?)?;
            if let Some(value) = read(&item.json, &parts) {
                write(&mut base.json, &parts, value.clone())?;
            }
        }
    }
    for value in array {
        let mut output = base.clone();
        write(&mut output.json, &parts, value.clone())?;
        append(result, bytes, output)?;
    }
    Ok(())
}

fn text_split(
    item: &Item,
    params: &Value,
    result: &mut Vec<Item>,
    bytes: &mut usize,
) -> Result<()> {
    let text = params["value"].as_str().ok_or_else(invalid)?;
    let mut emit = |text: String| -> Result<()> {
        let mut output = item.clone();
        output.json = json!({"text":text}).as_object().unwrap().clone();
        append(result, bytes, output)
    };
    if params["mode"] == "delimiter" {
        let delimiter = params["delimiter"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(invalid)?;
        for part in text.split(delimiter) {
            emit(part.to_owned())?;
        }
    } else {
        let length = super::graph::port_index(&params["length"])
            .and_then(|v| usize::try_from(v).ok())
            .filter(|v| *v > 0)
            .ok_or_else(invalid)?;
        let overlap = params
            .get("overlap")
            .map(super::graph::port_index)
            .unwrap_or(Some(0))
            .and_then(|v| usize::try_from(v).ok())
            .filter(|v| *v < length)
            .ok_or_else(invalid)?;
        let chars: Vec<_> = text.chars().collect();
        let mut start = 0;
        while start < chars.len() {
            let end = start.saturating_add(length).min(chars.len());
            emit(chars[start..end].iter().collect())?;
            if end == chars.len() {
                break;
            }
            start = end - overlap;
        }
    }
    Ok(())
}

pub(super) fn execute(spec: &Value, items: &[Item], params: &[Value]) -> Result<Vec<Item>> {
    let kind = spec["type"].as_str().ok_or_else(invalid)?;
    if per_item(spec) {
        if items.len() != params.len() {
            return Err(invalid());
        }
        let mut result = Vec::new();
        let mut bytes = 0;
        for (item, params) in items.iter().zip(params) {
            match kind {
                "data.rename" => append(&mut result, &mut bytes, rename(item, params)?)?,
                "core.split" => split(item, params, &mut result, &mut bytes)?,
                "data.text_split" => text_split(item, params, &mut result, &mut bytes)?,
                _ => return Err(invalid()),
            }
        }
        return Ok(result);
    }
    let params = params.first().ok_or_else(invalid)?;
    match kind {
        "core.no_op" => Ok(items.to_vec()),
        "core.sort" => sort(items, params),
        "core.limit" => {
            let count = super::graph::port_index(&params["count"])
                .and_then(|v| usize::try_from(v).ok())
                .ok_or_else(invalid)?;
            let offset = super::graph::port_index(&params["offset"])
                .and_then(|v| usize::try_from(v).ok())
                .ok_or_else(invalid)?;
            Ok(items.iter().skip(offset).take(count).cloned().collect())
        }
        "core.deduplicate" if params["scope"] == "execution" => deduplicate(items, params),
        _ => Err(invalid()),
    }
}

#[cfg(test)]
mod tests;
