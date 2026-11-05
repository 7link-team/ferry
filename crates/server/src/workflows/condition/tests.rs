use super::*;

fn item(value: Value) -> Item {
    Item {
        json: value.as_object().unwrap().clone(),
        binary: Default::default(),
        paired: vec![],
    }
}
fn frozen(mut params: Value) -> Value {
    let node = json!({"type":"core.if","params":params});
    for path in operand_paths(&node) {
        let value = params.pointer_mut(&path).unwrap();
        *value = json!({"present":true,"value":value.clone()});
    }
    params
}
fn node(kind: &str, params: Value) -> Value {
    json!({"id":"condition","name":"condition","type":kind,"type_version":1,"params":params})
}

#[test]
fn existence_and_null_are_distinct_and_values_cannot_spoof_presence() {
    assert!(compare(None, Some(&Value::Null), "ne").unwrap());
    assert!(!compare(None, None, "exists").unwrap());
    assert!(compare(Some(&Value::Null), None, "exists").unwrap());
    assert!(compare(Some(&Value::Null), Some(&Value::Null), "eq").unwrap());
    assert_eq!(operand(&json!({"present":false})).unwrap(), None);
    assert!(operand(&json!({"present":false,"value":7})).is_err());
    assert!(operand(&json!({"present":true})).is_err());
    let params = frozen(
        json!({"combinator":"and","rules":[{"left":{"present":false},"operator":"exists"}]}),
    );
    assert_eq!(
        operand(&params["rules"][0]["left"]).unwrap(),
        Some(&json!({"present":false}))
    );
}

#[test]
fn comparisons_are_typed_and_preserve_large_integer_order() {
    assert!(
        compare(
            Some(&json!(9007199254740993u64)),
            Some(&json!(9007199254740992u64)),
            "gt"
        )
        .unwrap()
    );
    assert!(compare(Some(&json!(1)), Some(&json!(1.0)), "eq").unwrap());
    assert!(compare(Some(&json!("ab😀中")), Some(&json!("😀")), "contains").unwrap());
    for operator in ["eq", "ne", "gt", "gte", "lt", "lte", "contains"] {
        assert!(
            compare(Some(&json!(1)), Some(&json!("1")), operator).is_err(),
            "{operator}"
        );
        assert!(
            compare(Some(&json!([])), Some(&json!([])), operator).is_err(),
            "{operator}"
        );
    }
    assert!(compare(None, Some(&json!(1)), "lt").is_err());
}

#[tokio::test]
async fn runtime_literal_presence_cannot_be_spoofed_and_preview_stays_plain() {
    let raw = json!({"combinator":"and","rules":[{"left":{"present":false},"operator":"exists"}]});
    let spec = node("core.filter", raw.clone());
    let data = || ferry_protocol::sandbox::InputData {
        items: vec![item(json!({}))],
        upstream: vec![],
    };
    let preview =
        super::super::graph::resolve(&spec, "/nodes/0", data(), &[Some(0)], json!({}), true)
            .await
            .unwrap();
    assert_eq!(preview, [raw]);
    let params = super::super::graph::resolve_attempt(
        &spec,
        "/nodes/0",
        data(),
        &[Some(0)],
        json!({}),
        true,
    )
    .await
    .unwrap();
    assert_eq!(
        params[0]["rules"][0]["left"],
        json!({"present":true,"value":{"present":false}})
    );
    let out = execute(&spec, &data().items, &params).await.unwrap();
    assert_eq!(out.ports[0].items.len(), 1);
}

#[tokio::test]
async fn if_routes_each_item_once_preserves_order_and_explicitly_skips_unused_ports() {
    let input = vec![
        item(json!({"id":0})),
        item(json!({"id":1})),
        item(json!({"id":2})),
    ];
    let p = |left| {
        frozen(json!({"combinator":"and","rules":[{"left":left,"operator":"eq","right":true}]}))
    };
    let spec = node(
        "core.if",
        json!({"combinator":"and","rules":[{"left":true,"operator":"eq","right":true}]}),
    );
    let out = execute(&spec, &input, &[p(true), p(false), p(true)])
        .await
        .unwrap();
    assert_eq!(
        out.ports
            .iter()
            .map(|port| (port.name.as_str(), port.skipped, port.items.len()))
            .collect::<Vec<_>>(),
        [("true", false, 2), ("false", false, 1)]
    );
    assert_eq!(out.ports[0].items[1].json["id"], 2);
    let out = execute(&spec, &input[..1], &[p(true)]).await.unwrap();
    assert!(out.ports[1].skipped && out.ports[1].items.is_empty());
    assert!(!out.ports[0].skipped);
}

#[tokio::test]
async fn filter_zero_results_are_executed_empty_and_all_rules_are_validated() {
    let params =
        frozen(json!({"combinator":"or","rules":[{"left":false,"operator":"eq","right":true}]}));
    let spec = node("core.filter", params.clone());
    let out = execute(&spec, &[item(json!({}))], &[params]).await.unwrap();
    assert!(!out.ports[0].skipped && out.ports[0].items.is_empty());
    let out = execute(&spec, &[], &[]).await.unwrap();
    assert!(!out.ports[0].skipped);
    let invalid = frozen(
        json!({"combinator":"or","rules":[{"left":1,"operator":"eq","right":1},{"left":{},"operator":"gt","right":1}]}),
    );
    assert!(
        execute(&spec, &[item(json!({}))], &[invalid])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn switch_first_all_fallback_and_disabled_fallback_have_stable_ports() {
    let c = json!({"combinator":"and","rules":[{"left":1,"operator":"eq","right":1}]});
    let mut params = json!({"cases":[{"key":"a","condition":c},{"key":"b","condition":c}],"match":"all","fallback":true});
    let spec = node("core.switch", params.clone());
    for case in params["cases"].as_array_mut().unwrap() {
        case["condition"] = frozen(case["condition"].clone());
    }
    let input = [item(json!({"id":1}))];
    let out = execute(&spec, &input, &[params.clone()]).await.unwrap();
    assert_eq!(
        out.ports
            .iter()
            .map(|p| (p.name.as_str(), p.index, p.items.len(), p.skipped))
            .collect::<Vec<_>>(),
        [
            ("case", 0, 1, false),
            ("case", 1, 1, false),
            ("fallback", 0, 0, true)
        ]
    );
    params["match"] = json!("first");
    let out = execute(&spec, &input, &[params.clone()]).await.unwrap();
    assert!(out.ports[1].skipped);
    for case in params["cases"].as_array_mut().unwrap() {
        case["condition"]["rules"][0]["right"]["value"] = json!(2);
    }
    let out = execute(&spec, &input, &[params.clone()]).await.unwrap();
    assert!(out.ports[0].skipped && out.ports[1].skipped && !out.ports[2].skipped);
    params["fallback"] = json!(false);
    let out = execute(&spec, &input, &[params]).await.unwrap();
    assert!(out.ports.iter().all(|p| p.skipped));
}
