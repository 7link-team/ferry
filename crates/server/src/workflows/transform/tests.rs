use super::*;

fn item(value: Value) -> Item {
    serde_json::from_value(json!({"json":value,"binary":{"file":{"asset":format!("sha256:{}","a".repeat(64)),"mime":"text/plain","size":1,"name":"sample.txt"}},"paired":[{"node":"source","iteration_hash":"b".repeat(64),"index":2}]})).unwrap()
}
fn apply(kind: &str, values: &[Value], params: Value) -> Result<Vec<Item>> {
    let items: Vec<_> = values.iter().cloned().map(item).collect();
    let spec = json!({"type":kind,"params":params});
    let params = if per_item(&spec) {
        vec![params; items.len()]
    } else {
        vec![params]
    };
    execute(&spec, &items, &params)
}
fn ids(items: &[Item]) -> Vec<Value> {
    items.iter().map(|item| item.json["id"].clone()).collect()
}

#[test]
fn sorting_is_stable_null_placement_is_independent_of_direction_and_integers_are_exact() {
    let values = vec![
        json!({"id":"large","rank":9007199254740993u64}),
        json!({"id":"float","rank":9007199254740992.0}),
        json!({"id":"equal","rank":9007199254740992u64}),
        json!({"id":"null","rank":null}),
        json!({"id":"missing"}),
    ];
    let params = json!({"keys":[{"path":"rank","direction":"asc","nulls":"last"}]});
    assert_eq!(
        ids(&apply("core.sort", &values, params).unwrap()),
        vec![
            json!("float"),
            json!("equal"),
            json!("large"),
            json!("null"),
            json!("missing")
        ]
    );
    let params = json!({"keys":[{"path":"rank","direction":"desc","nulls":"first"}]});
    assert_eq!(
        ids(&apply("core.sort", &values, params).unwrap()),
        vec![
            json!("null"),
            json!("missing"),
            json!("large"),
            json!("float"),
            json!("equal")
        ]
    );
    for values in [
        vec![json!({"rank":1}), json!({"rank":"2"})],
        vec![json!({"rank":[]})],
        vec![json!({"rank":{}})],
    ] {
        assert!(
            apply(
                "core.sort",
                &values,
                json!({"keys":[{"path":"rank","direction":"asc","nulls":"last"}]})
            )
            .is_err()
        );
    }
}

#[test]
fn decimal_comparison_orders_sign_zero_fraction_and_exponents_without_float_rounding() {
    let raw = [
        "-1e100",
        "-100",
        "-1.25",
        "-1.0",
        "-0.0001",
        "-0.0",
        "0",
        "1e-30",
        "0.01",
        "1",
        "1.0",
        "1.000001",
        "18446744073709551614",
        "18446744073709551615",
        "1e100",
    ];
    let values: Vec<Number> = raw
        .iter()
        .map(|text| serde_json::from_str(text).unwrap())
        .collect();
    for (i, left) in values.iter().enumerate() {
        assert_eq!(numeric(left, left), Ordering::Equal);
        for right in &values[i + 1..] {
            assert_ne!(numeric(left, right), Ordering::Greater, "{left}/{right}");
            assert_eq!(numeric(right, left), numeric(left, right).reverse());
        }
    }
}

#[test]
fn no_op_and_limit_preserve_binary_metadata_and_paired_lineage_including_empty_slices() {
    let source = vec![item(json!({"id":1})), item(json!({"id":2}))];
    let passthrough = execute(&json!({"type":"core.no_op"}), &source, &[json!({})]).unwrap();
    assert_eq!(
        serde_json::to_value(&source).unwrap(),
        serde_json::to_value(&passthrough).unwrap()
    );
    let result = execute(
        &json!({"type":"core.limit"}),
        &source,
        &[json!({"offset":1,"count":1})],
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&result[0]).unwrap(),
        serde_json::to_value(&source[1]).unwrap()
    );
    for params in [
        json!({"offset":0,"count":0}),
        json!({"offset":100,"count":3}),
    ] {
        assert!(
            execute(&json!({"type":"core.limit"}), &source, &[params])
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn rename_reads_original_values_before_writing_and_never_invents_missing_fields() {
    let params = json!({"mappings":[{"from":"a","to":"b"},{"from":"b","to":"a"},{"from":"missing","to":"other"}],"on_collision":"error"});
    let result = apply("data.rename", &[json!({"a":1,"b":2})], params).unwrap();
    assert_eq!(
        result[0].json,
        json!({"a":2,"b":1}).as_object().unwrap().clone()
    );
    for params in [
        json!({"mappings":[{"from":"a","to":"b"}],"on_collision":"error"}),
        json!({"mappings":[{"from":"a","to":"constructor.x"}],"on_collision":"overwrite"}),
        json!({"mappings":[{"from":"a","to":"x"},{"from":"b","to":"x.y"}],"on_collision":"overwrite"}),
    ] {
        assert!(apply("data.rename", &[json!({"a":1,"b":2})], params).is_err());
    }
    let result = apply(
        "data.rename",
        &[json!({"a":1,"b":2})],
        json!({"mappings":[{"from":"a","to":"b"}],"on_collision":"overwrite"}),
    )
    .unwrap();
    assert_eq!(result[0].json, json!({"b":1}).as_object().unwrap().clone());
}

#[test]
fn split_only_expands_arrays_and_retains_lineage_for_every_expanded_item() {
    let source = item(json!({"keep":1,"extra":2,"nested":{"parts":[false,null,{"a":1}]}}));
    for include in ["all", "none", "selected"] {
        let result = execute(
            &json!({"type":"core.split"}),
            &[source.clone()],
            &[json!({"path":"nested.parts","include":include,"fields":["keep"]})],
        )
        .unwrap();
        assert_eq!(result.len(), 3);
        for (index, output) in result.iter().enumerate() {
            assert_eq!(
                output.json["nested"]["parts"],
                source.json["nested"]["parts"][index]
            );
            assert_eq!(
                serde_json::to_value(&output.paired).unwrap(),
                serde_json::to_value(&source.paired).unwrap()
            );
            assert_eq!(
                serde_json::to_value(&output.binary).unwrap(),
                serde_json::to_value(&source.binary).unwrap()
            );
            assert_eq!(output.json.contains_key("extra"), include == "all");
            assert_eq!(output.json.contains_key("keep"), include != "none");
        }
    }
    assert!(
        apply(
            "core.split",
            &[json!({"parts":"abc"})],
            json!({"path":"parts","include":"all"})
        )
        .is_err()
    );
    assert!(
        apply(
            "core.split",
            &[json!({"parts":[]})],
            json!({"path":"parts","include":"all"})
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn text_chunks_use_unicode_scalar_positions_and_reject_nonprogressing_overlap() {
    let result = apply(
        "data.text_split",
        &[json!({})],
        json!({"value":"a中😀b文","mode":"length","length":3,"overlap":1}),
    )
    .unwrap();
    assert_eq!(
        result
            .iter()
            .map(|item| item.json["text"].clone())
            .collect::<Vec<_>>(),
        vec![json!("a中😀"), json!("😀b文")]
    );
    for overlap in [3, 4] {
        assert!(
            apply(
                "data.text_split",
                &[json!({})],
                json!({"value":"abc","mode":"length","length":3,"overlap":overlap})
            )
            .is_err()
        );
    }
    assert!(
        apply(
            "data.text_split",
            &[json!({})],
            json!({"value":"abc","mode":"delimiter","delimiter":""})
        )
        .is_err()
    );
}

#[test]
fn execution_deduplication_retains_selected_original_items_and_distinguishes_null_from_missing() {
    let values = [
        json!({"id":1,"key":"same"}),
        json!({"id":2,"key":null}),
        json!({"id":3}),
        json!({"id":4,"key":"same"}),
    ];
    assert_eq!(
        ids(&apply(
            "core.deduplicate",
            &values,
            json!({"scope":"execution","keys":["key"],"keep":"first"})
        )
        .unwrap()),
        vec![json!(1), json!(2), json!(3)]
    );
    assert_eq!(
        ids(&apply(
            "core.deduplicate",
            &values,
            json!({"scope":"execution","keys":["key"],"keep":"last"})
        )
        .unwrap()),
        vec![json!(2), json!(3), json!(4)]
    );
    assert!(!supports(
        &json!({"type":"core.deduplicate","params":{"scope":"workflow"}})
    ));
    assert!(
        apply(
            "core.deduplicate",
            &values,
            json!({"scope":"workflow","keys":["key"],"keep":"first"})
        )
        .is_err()
    );
}

#[test]
fn expansion_checks_the_journal_output_budget_while_building_overlapping_chunks() {
    let params = json!({"value":"x".repeat(12000),"mode":"length","length":8000,"overlap":7999});
    assert!(apply("data.text_split", &[json!({})], params).is_err());
    let params = json!({"value":"x".repeat(20001),"mode":"length","length":1});
    assert_eq!(
        apply("data.text_split", &[json!({})], params)
            .unwrap()
            .len(),
        20001
    );
}
