"""Validate design artifacts, not a workflow runtime implementation."""
from pathlib import Path
from copy import deepcopy
from collections import Counter
import json
import hashlib
import math
import struct
import re
import sys
import sqlite3
from itertools import product, permutations
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "data/maintenance/workflow-schema-python"))
try:
    from jsonschema import Draft202012Validator
    from jsonschema.exceptions import SchemaError
except ImportError as error:
    raise SystemExit("Install jsonschema==4.26.0 as described in schemas/README.md") from error

HERE = ROOT / 'src/shared/workflow-contracts'
DOCS = Path(__file__).resolve().parent


def parse_json(text):
    def unique_object(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate JSON object key: " + key)
            result[key] = value
        return result

    def finite_float(value):
        number = float(value)
        if not math.isfinite(number):
            raise ValueError("nonfinite JSON number")
        return number

    def invalid_constant(value):
        raise ValueError("invalid JSON constant: " + value)

    return json.loads(text, object_pairs_hook=unique_object, parse_float=finite_float,
                      parse_constant=invalid_constant)


def read_json(path):
    return parse_json(path.read_text(encoding="utf-8"))


def validate_embedded_schema(schema, policy, limits):
    """Bounded single-resource design compiler; it does not validate user data."""
    expected = set(policy["example_limits"])
    if set(limits) != expected or any(type(v) is not int or v < 1 for v in limits.values()):
        raise ValueError("explicit positive schema compilation budgets required")
    count, deepest = 0, 0
    stack = [(schema, 0)]
    while stack:
        value, depth = stack.pop()
        count += 1
        deepest = max(deepest, depth)
        if count > limits["max_json_values"] or depth > limits["max_json_depth"]:
            raise ValueError("schema JSON tree budget exceeded")
        if isinstance(value, dict):
            stack.extend((v, depth + 1) for v in value.values())
        elif isinstance(value, list):
            stack.extend((v, depth + 1) for v in value)
    encoded = json.dumps(schema, ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode("utf-8")
    if len(encoded) > limits["max_document_bytes"]:
        raise ValueError("schema document byte budget exceeded")
    try:
        Draft202012Validator.check_schema(schema)
    except SchemaError as error:
        raise ValueError("invalid embedded schema") from error
    maps, arrays, singles = (policy[key] for key in ("schema_maps", "schema_arrays", "schema_values"))
    supported = set(maps + arrays + singles + policy["leaf_keywords"])
    pending = [((), schema)]
    locations, anchors, refs, edges = {}, {}, [], {}
    while pending:
        path, current = pending.pop()
        locations[path] = current
        edges[path] = []
        if len(locations) > limits["max_schema_locations"]:
            raise ValueError("schema location budget exceeded")
        if isinstance(current, bool):
            continue
        if not isinstance(current, dict) or not current.keys() <= supported:
            raise ValueError("unsupported schema location or keyword")
        if "$schema" in current and (path or current["$schema"] != policy["dialect"]):
            raise ValueError("only the root 2020-12 dialect is supported")
        if "$anchor" in current:
            anchor = current["$anchor"]
            if not re.fullmatch(r"[A-Za-z_][-A-Za-z0-9._]*", anchor) or anchor in anchors:
                raise ValueError("invalid or duplicate local anchor")
            anchors[anchor] = path
        if "$ref" in current:
            refs.append((path, current["$ref"]))
            if len(refs) > limits["max_reference_edges"]:
                raise ValueError("schema reference edge budget exceeded")
        children = []
        for keyword in maps:
            mapping = current.get(keyword, {})
            if not isinstance(mapping, dict):
                raise ValueError("schema mapping must be an object")
            children.extend((path + (keyword, key), child) for key, child in mapping.items())
        for keyword in arrays:
            sequence = current.get(keyword, [])
            if not isinstance(sequence, list):
                raise ValueError("schema sequence must be an array")
            children.extend((path + (keyword, str(index)), child) for index, child in enumerate(sequence))
        for keyword in singles:
            if keyword in current:
                children.append((path + (keyword,), current[keyword]))
        edges[path].extend(child_path for child_path, _ in children)
        pending.extend(children)

    for path, ref in refs:
        if not ref.startswith("#") or re.search(r"%(?![0-9A-Fa-f]{2})", ref):
            raise ValueError("only valid local URI fragments are supported")
        fragment = unquote(ref[1:], encoding="utf-8", errors="strict")
        if not fragment:
            target = ()
        elif fragment.startswith("/"):
            tokens = fragment[1:].split("/")
            if any(re.search(r"~(?![01])", token) for token in tokens):
                raise ValueError("invalid JSON Pointer escape")
            target = tuple(token.replace("~1", "/").replace("~0", "~") for token in tokens)
        elif fragment in anchors:
            target = anchors[fragment]
        else:
            raise ValueError("missing local anchor")
        # Looking up schema sites excludes arbitrary dictionaries under default/const.
        if target not in locations:
            raise ValueError("reference target is missing or is not a schema location")
        edges[path].append(target)

    # Do not expand references. A recursive schema is not invalid just for having a cycle.
    colors, cyclic = {}, False
    for start in locations:
        if colors.get(start) == 2:
            continue
        walk = [(start, False)]
        while walk:
            site, exiting = walk.pop()
            if exiting:
                colors[site] = 2
                continue
            if colors.get(site) == 1:
                cyclic = True
                continue
            if colors.get(site) == 2:
                continue
            colors[site] = 1
            walk.append((site, True))
            walk.extend((child, False) for child in edges[site])
    return {"json_values": count, "json_depth": deepest, "document_bytes": len(encoded), "schema_locations": len(locations),
            "reference_edges": len(refs), "has_cycles": cyclic}


def archive_parents(path):
    parts = path.split("/")
    return ["/".join(parts[:index]) for index in range(1, len(parts))]


def safe_file_path(path, single=False):
    # Virtual asset paths only; this does not validate an extraction sandbox.
    parts = path.split("/")
    if (not path or any(part in ("", ".", "..") for part in parts)
            or any(c in path for c in "\\:") or any(ord(c) < 32 or ord(c) == 127 for c in path)
            or (single and len(parts) != 1)):
        raise ValueError("invalid virtual file path")
    path.encode("utf-8", errors="strict")


def merge_projection(state, count, *, final=True, policy="stop", port="main"):
    """Normalize a committed edge projection, not the raw state of an If node."""
    if type(count) is not int or count < 0 or port not in ("main", "error"):
        raise ValueError("invalid reference projection")
    if state in ("pending", "queued", "running", "waiting"):
        return "P"  # Partial output is never a committed input.
    if state in ("failed", "timed_out"):
        if not final:
            return "P"
        if policy == "stop":
            return "H"
        if policy == "error_output" and port == "main":
            return "U"  # Main projection has no frozen empty/skip rule yet.
        if (policy == "continue" and port == "main") or (policy == "error_output" and port == "error"):
            if count == 0:
                raise ValueError("continued failure must have committed error items")
            return "F"
        raise ValueError("unsupported error policy/port combination")
    if state == "cancelled":
        return "C"
    if state == "skipped":
        if count:
            raise ValueError("skipped projection cannot have items")
        return "S"
    if state in ("succeeded", "cached", "pinned", "reused", "bypassed"):
        return "D" if count else "E"
    raise ValueError("unknown source state")


def credential_projection(spec, bindings, records, *, workspace, actor, module_scope, profiles, operation=None):
    """Synthetic authorization metadata only, not a vault or live permission check."""
    if module_scope not in ("host", "build", "deploy") or not set(bindings) <= spec.keys():
        raise ValueError("unknown module or credential slot")
    projection = []
    for slot, declaration in spec.items():
        needed = declaration["required"] or operation in declaration.get("operation_profiles", {})
        if slot not in bindings:
            if needed:
                raise ValueError("missing credential slot")
            continue
        credential_id = bindings[slot]
        if not isinstance(credential_id, str) or not re.fullmatch(r"[1-9][0-9]*", credential_id):
            raise ValueError("credential ID reference required")
        row = records.get(credential_id)
        if not row or row["workspace_id"] != workspace or row["scope"] != module_scope:
            raise ValueError("credential unavailable")
        if not row["active"] or actor not in row["allowed_actors"] or not row["has_secret"]:
            raise ValueError("credential unavailable")
        profile = row["profile"]
        if profile not in declaration["accepted_profiles"] or profile not in profiles:
            raise ValueError("logical credential profile mismatch")
        if row["kind"] not in profiles[profile]["storage_kinds"]:
            raise ValueError("storage kind does not implement profile")
        required_profile = declaration.get("operation_profiles", {}).get(operation)
        if required_profile and required_profile != profile:
            raise ValueError("operation credential profile mismatch")
        if type(row["version"]) is not int or row["version"] < 1 or not row["adapter_version"]:
            raise ValueError("unversioned credential adapter")
        if not re.fullmatch(r"[0-9a-f]{64}", row["public_config_hash"]):
            raise ValueError("invalid public configuration projection")
        projection.append({"workspace_id": workspace, "slot": slot, "credential_id": credential_id,
                           "credential_version": row["version"], "profile": profile,
                           "adapter_version": row["adapter_version"], "public_config_hash": row["public_config_hash"]})
    return sorted(projection, key=lambda value: value["slot"])


def port_compatibility(source, target, same_gpu_segment=None):
    """Static design relation; compatible still requires runtime payload checks."""
    if source.startswith("gpu:") or target.startswith("gpu:"):
        if source != target or not source.startswith("gpu:"):
            return "incompatible"
        if same_gpu_segment is None:
            return "unfrozen"
        return "compatible" if same_gpu_segment else "incompatible"
    if "model" in (source, target):
        if source == target:
            return "compatible"
        return "unfrozen" if "main" in (source, target) else "incompatible"
    regular = {"main", "text", "image", "video", "audio", "file"}
    if source not in regular or target not in regular:
        return "unfrozen"
    return "compatible" if source == target or "main" in (source, target) else "incompatible"


def topology_review(graph, registry, entry=None, require_entry=False):
    """Top-layer advisory analysis AFTER structural validation; no publish decision."""
    lookup = {node["id"]: node for node in graph["nodes"]}
    outgoing, incoming = {key: [] for key in lookup}, {key: {} for key in lookup}
    for edge in graph["connections"]:
        origin, destination = edge["from"], edge["to"]
        outgoing[origin["node"]].append(destination["node"])
        incoming[destination["node"]].setdefault((destination["port"], destination["index"]), []).append(origin["node"])
    triggers = {key for key, node in lookup.items() if node["type"].startswith("core.trigger.")}
    if (require_entry and entry is None) or (entry is not None and entry not in triggers):
        raise ValueError("trigger-start requires an explicit trigger in this graph layer")

    def reachable(starts):
        reached, pending = set(), list(starts)
        while pending:
            key = pending.pop()
            if key in reached:
                continue
            reached.add(key)
            pending.extend(outgoing[key])
        return reached

    reachable_any = reachable(triggers)
    active = reachable([entry]) if entry is not None else None
    findings = []
    def warning(code, node, port="", index=0):
        findings.append({"code": code, "node": node, "port": port, "index": index, "severity": "warning"})
    for key, node in lookup.items():
        spec = registry[node["type"]]
        expected = [(port["port"], index) for port in spec["inputs"]
                    for index in range(node["params"]["input_count"] if node["type"] == "core.merge" else 1)]
        for port, index in expected:
            sources = incoming[key].get((port, index), [])
            if not sources:
                warning("merge_input_unconnected" if node["type"] == "core.merge" else "input_unconnected", key, port, index)
            elif len(sources) > 1:
                warning("input_fanin_unfrozen", key, port, index)
            if active is not None and key in active and node["type"] == "core.merge" and any(source not in active for source in sources):
                warning("merge_other_entry_dependency", key, port, index)
        if key not in reachable_any:
            warning("not_reachable_from_trigger", key)
        if node["type"] == "core.loop":
            warning("loop_body_interface_unfrozen", key)
    return {"triggers": sorted(triggers), "selected_entry": entry,
            "reachable_nodes": sorted(active) if active is not None else None,
            "findings": sorted(findings, key=lambda issue: (issue["node"], issue["port"], issue["index"], issue["code"]))}


def merge_readiness(inputs, wait, execution="running"):
    """Design decision labels; these are not new persisted node states."""
    if not inputs or not set(inputs) <= set("DESFPHCU") or wait not in ("any_items", "all_inputs"):
        raise ValueError("invalid readiness input")
    if execution in ("cancelling", "cancelled"):
        return "cancel"
    if execution in ("succeeded", "failed", "timed_out"):
        return "closed"
    if execution in ("pending", "queued", "waiting", "paused"):
        return "hold"
    if execution != "running":
        raise ValueError("unknown execution state")
    for marker, decision in (("H", "stop"), ("C", "cancelled_source"), ("U", "contract_blocked"), ("P", "wait")):
        if marker in inputs:
            return decision
    if wait == "all_inputs":
        return "skipped" if "S" in inputs else "run"
    return "run" if set(inputs) & set("DF") else "skipped"


def run():
    graph_schema = read_json(HERE / "workflow-graph.schema.json")
    catalog = read_json(HERE / "workflow-core-nodes.json")
    data_catalog = read_json(HERE / "workflow-data-nodes.json")
    file_catalog = read_json(HERE / "workflow-file-nodes.json")
    schema_policy = catalog["embedded_schema_contract"]
    credential_policy = catalog["credential_contract"]
    Draft202012Validator.check_schema(graph_schema)
    graph_validator = Draft202012Validator(graph_schema)
    assert len(catalog["nodes"]) == 46 and len(data_catalog["nodes"]) == 3
    assert len(file_catalog["nodes"]) == 4
    all_nodes = catalog["nodes"] + data_catalog["nodes"] + file_catalog["nodes"]
    registry = {node["type"]: node for node in all_nodes}
    assert len(registry) == len(all_nodes) == 53
    cases = []

    def record(name, accepted, function):
        try:
            function()
        except (ValueError, AssertionError) as error:
            if accepted:
                raise AssertionError(name) from error
        else:
            if not accepted:
                raise AssertionError(f"Unexpectedly accepted: {name}")
        cases.append({"case": name, "expected": "accept" if accepted else "reject", "result": "PASS"})

    def validate_params(spec, value):
        errors = list(Draft202012Validator(spec["params_schema"]).iter_errors(value))
        if errors:
            raise ValueError(errors[0].message)
        name = spec["type"]
        if name.startswith("file."):
            if "output_name" in value:
                safe_file_path(value["output_name"], single=True)
            if "entries" in value:
                paths = [entry["path"] for entry in value["entries"]]
                for path in paths:
                    safe_file_path(path)
                if len(set(paths)) != len(paths):
                    raise ValueError("duplicate archive path")
                if any(parent in paths for path in paths for parent in archive_parents(path)):
                    raise ValueError("archive file/directory conflict")
            if "csv" in value:
                dialect = value["csv"]
                if dialect["delimiter"] == dialect["quote"] or any(
                        char in "\r\n\x00" for char in (dialect["delimiter"], dialect["quote"])):
                    raise ValueError("invalid CSV delimiter/quote")
            if "sheet_name" in value:
                sheet = value["sheet_name"]
                if len(sheet.encode("utf-16-le")) // 2 > 31 or any(c in sheet for c in "[]:*?/\\") or (
                        sheet.startswith("'") or sheet.endswith("'") or any(ord(c) < 32 for c in sheet)):
                    raise ValueError("invalid XLSX sheet name")
        if name == "core.switch" and len({case["key"] for case in value["cases"]}) != len(value["cases"]):
            raise ValueError("duplicate switch case key")
        if name == "data.text_split" and value["mode"] == "length" and value.get("overlap", 0) >= value["length"]:
            raise ValueError("overlap must be less than length")
        if name == "human.pick" and value["min_selected"] > value["max_selected"]:
            raise ValueError("selection bounds reversed")
        for key in ("input_schema", "response_schema"):
            if key in value:
                validate_embedded_schema(value[key], schema_policy, schema_policy["example_limits"])

    for spec in all_nodes:
        Draft202012Validator.check_schema(spec["params_schema"])
        sample = spec["example_params"]
        record(spec["type"] + ": valid resolved parameters", True, lambda s=spec, v=sample: validate_params(s, v))
        unknown = {**sample, "unknown_option": 1}
        record(spec["type"] + ": unknown parameter", False, lambda s=spec, v=unknown: validate_params(s, v))
        required = spec["params_schema"]["required"]
        if required:
            missing = deepcopy(sample)
            missing.pop(required[0])
            record(spec["type"] + ": missing required parameter", False, lambda s=spec, v=missing: validate_params(s, v))
        assert spec["runtime"]["default"] in spec["runtime"]["allowed"]
        if spec["type"].startswith(("human.", "core.trigger.")) or spec["type"] in ("core.wait", "net.smtp", "net.respond"):
            assert spec["cache_allowed"] is False
        assert spec["required_evidence"] and spec["semantic_rule"]
        for declaration in spec["credential_slots"].values():
            assert set(declaration["accepted_profiles"]) <= credential_policy["profiles"].keys()
            assert set(declaration.get("operation_profiles", {}).values()) <= set(declaration["accepted_profiles"])

    source_map = catalog["source_mapping"]
    assert [m["source_id"] for m in source_map] == [f"B{i:03}" for i in range(1, 76)]
    inventory = (ROOT / "docs/ferry-工作流能力清单.md").read_text(encoding="utf-8")
    core_section = inventory.split("## B.", 1)[1].split("## C.", 1)[0]
    inventory_rows = {}
    for line in core_section.splitlines():
        if re.match(r"\| B\d{3} \|", line):
            cells = [cell.strip() for cell in line.split("|")[1:-1]]
            inventory_rows[cells[0]] = cells
    assert len(inventory_rows) == 75
    for source in source_map:
        original = inventory_rows[source["source_id"]]
        assert source["source_member"] == original[1]
        assert source["phase"] == original[4]
        assert set(source["ferry_types"]) <= registry.keys()
        if source["phase"] == "S1":
            assert source["contract_status"] in ("parameter_contract", "layout_contract")
        if source["contract_status"] == "scheduled":
            assert not source["ferry_types"]
    for spec in all_nodes:
        if "source_ids" in spec:
            assert set(spec["source_ids"]) == {row["source_id"] for row in source_map
                                                if spec["type"] in row["ferry_types"]}
    file_review = (ROOT / "docs/prd/ferry-工作流平台PRD.md").read_text(encoding="utf-8").split("## 20.", 1)[1]
    review_members = dict(re.findall(r"^\| (B\d{3}) ([^|]+?) \|", file_review, re.MULTILINE))
    assert set(review_members) == set(file_catalog["source_members"])
    for source_id, member in review_members.items():
        assert member == inventory_rows[source_id][1]

    # These checks illustrate the stated static contracts. They perform no
    # expression evaluation, permission query, scheduling or external request.
    def validate_graph(graph, nesting=0, node_total=None, stage="resolved_contract"):
        if stage not in ("structure", "resolved_contract"):
            raise ValueError("unknown static graph validation stage")
        if node_total is None:
            node_total = [0]
        errors = list(graph_validator.iter_errors(graph))
        if errors:
            raise ValueError(errors[0].message)
        if len(json.dumps(graph, ensure_ascii=False, allow_nan=False).encode("utf-8")) > 2 * 1024 * 1024:
            raise ValueError("definition limit")
        if len(json.dumps(graph.get("pin_data", {}), ensure_ascii=False, allow_nan=False,
                          separators=(",", ":")).encode("utf-8")) > 1024 * 1024:
            raise ValueError("pin data limit")
        lookup = {n["id"]: n for n in graph["nodes"]}
        node_total[0] += len(graph["nodes"])
        if node_total[0] > 500:
            raise ValueError("total node limit including loop bodies")
        if len(lookup) != len(graph["nodes"]):
            raise ValueError("duplicate node IDs")
        adjacency = {node: [] for node in lookup}
        for instance in graph["nodes"]:
            spec = registry.get(instance["type"])
            if not spec or instance["type_version"] != spec["type_version"]:
                raise ValueError("unknown node/version")
            validate_params(spec, instance["params"])
            settings = instance.get("settings", {})
            runtime = settings.get("runtime") or spec["runtime"]["default"]
            if runtime not in spec["runtime"]["allowed"]:
                raise ValueError("runtime not supported")
            if stage == "resolved_contract" and runtime == "host" and not {"target_kind", "target_ref"} <= settings.keys():
                raise ValueError("host binding required")
            if not spec["cache_allowed"] and settings.get("cache", "off") != "off":
                raise ValueError("cannot cache")
            if settings.get("purity") == "pure" and instance["type"] == "code.javascript" and not instance["params"]["deterministic"]:
                raise ValueError("JS pure mode requires deterministic sandbox")
            if instance["type"] == "host.command" and "{{" in instance["params"]["command"]:
                raise ValueError("host command interpolation")
            credentials = instance.get("credentials", {})
            if not credentials.keys() <= spec["credential_slots"].keys():
                raise ValueError("unknown credential slot")
            if stage == "resolved_contract" and any(slot not in credentials for slot, declaration in spec["credential_slots"].items() if declaration["required"]):
                raise ValueError("missing credential")
            if stage == "resolved_contract" and instance["type"] == "data.crypto" and instance["params"]["operation"] not in ("sha256", "sha512") and "key" not in credentials:
                raise ValueError("crypto operation requires key credential")
            if instance["type"] == "core.loop":
                if nesting >= 3:
                    raise ValueError("loop nesting limit")
                nested = {"engine": "ferry-graph", "schema": 1, **instance["params"]["body"]}
                validate_graph(nested, nesting + 1, node_total, stage)
        seen_connections = set()
        for connection in graph["connections"]:
            identity = tuple(connection[side][field] for side in ("from", "to") for field in ("node", "port", "index"))
            if identity in seen_connections:
                raise ValueError("duplicate connection")
            seen_connections.add(identity)
            for side, direction in (("from", "outputs"), ("to", "inputs")):
                endpoint = connection[side]
                if endpoint["node"] not in lookup:
                    raise ValueError("dangling edge")
                spec = registry[lookup[endpoint["node"]]["type"]]
                if endpoint["port"] not in {port["port"] for port in spec[direction]}:
                    raise ValueError("unknown port")
                params = lookup[endpoint["node"]]["params"]
                if direction == "outputs" and spec["type"] == "core.switch" and endpoint["port"] == "case":
                    if endpoint["index"] >= len(params["cases"]):
                        raise ValueError("switch case port index outside cases")
                elif direction == "inputs" and spec["type"] == "core.merge":
                    if endpoint["index"] >= params["input_count"]:
                        raise ValueError("merge input index outside declared inputs")
                elif endpoint["index"] != 0:
                    raise ValueError("ordinary port index must be zero")
                if direction == "outputs" and spec["type"] == "core.switch" and endpoint["port"] == "fallback" and not params["fallback"]:
                    raise ValueError("disabled switch fallback port")
            adjacency[connection["from"]["node"]].append(connection["to"]["node"])
            source_node, target_node = (lookup[connection[side]["node"]] for side in ("from", "to"))
            source_type = next(port["type"] for port in registry[source_node["type"]]["outputs"] if port["port"] == connection["from"]["port"])
            target_type = next(port["type"] for port in registry[target_node["type"]]["inputs"] if port["port"] == connection["to"]["port"])
            if port_compatibility(source_type, target_type) == "incompatible":
                raise ValueError("incompatible declared port types")
        visiting, visited = set(), set()

        def visit(node):
            if node in visiting:
                raise ValueError("cycle outside loop container")
            if node in visited:
                return
            visiting.add(node)
            for target in adjacency[node]:
                visit(target)
            visiting.remove(node)
            visited.add(node)

        for node in lookup:
            visit(node)
        if not graph.get("pin_data", {}).keys() <= lookup.keys():
            raise ValueError("unknown pin node")

    def instance(node_id, type_name):
        return {"id": node_id, "type": type_name, "type_version": 1, "name": node_id,
                "params": deepcopy(registry[type_name]["example_params"]), "position": [0, 0]}

    graph = {"engine": "ferry-graph", "schema": 1,
             "nodes": [instance("start", "core.trigger.manual"), instance("set", "data.set")],
             "connections": [{"from": {"node": "start", "port": "main", "index": 0},
                              "to": {"node": "set", "port": "main", "index": 0}}]}
    record("graph: valid manual/set graph", True, lambda: validate_graph(graph))
    mutations = [
        ("duplicate ID", lambda g: g["nodes"][1].update(id="start")),
        ("unknown type version", lambda g: g["nodes"][1].update(type_version=99)),
        ("dangling edge", lambda g: g["connections"][0]["to"].update(node="missing")),
        ("unknown output port", lambda g: g["connections"][0]["from"].update(port="missing")),
        ("ordinary output index", lambda g: g["connections"][0]["from"].update(index=1)),
        ("ordinary input index", lambda g: g["connections"][0]["to"].update(index=1)),
        ("duplicate connection", lambda g: g["connections"].append(deepcopy(g["connections"][0]))),
        ("unknown graph field", lambda g: g.update(secret="not-a-graph-field")),
        ("unknown credential slot", lambda g: g["nodes"][1].update(credentials={"token": "22"})),
        ("runtime override forbidden", lambda g: g["nodes"][1].update(settings={"runtime": "host"})),
        ("bad node ID", lambda g: g["nodes"][1].update(id="bad-node")),
        ("trigger cannot be cached", lambda g: g["nodes"][0].update(settings={"cache": "auto"})),
        ("unknown fixed-data node", lambda g: g.update(pin_data={"missing": [{"json": {}}]})),
        ("loop-free cycle", lambda g: (g["nodes"].__setitem__(0, instance("start", "core.no_op")), g["connections"].append({"from": {"node": "set", "port": "main", "index": 0}, "to": {"node": "start", "port": "main", "index": 0}}))),
    ]
    for name, mutate in mutations:
        candidate = deepcopy(graph)
        mutate(candidate)
        record("graph: " + name, False, lambda value=candidate: validate_graph(value))
    host = {"engine": "ferry-graph", "schema": 1, "nodes": [instance("command", "host.command")], "connections": []}
    record("graph: unbound host", False, lambda: validate_graph(host))
    host["nodes"][0]["settings"] = {"target_kind": "host", "target_ref": "22"}
    record("graph: bound host", True, lambda: validate_graph(host))
    host["nodes"][0]["params"]["command"] = "echo {{untrusted}}"
    record("graph: command injection template", False, lambda: validate_graph(host))
    js = {"engine": "ferry-graph", "schema": 1, "nodes": [instance("js", "code.javascript")], "connections": []}
    js["nodes"][0]["settings"] = {"purity": "pure", "cache": "auto"}
    record("graph: nondeterministic JS marked pure", False, lambda: validate_graph(js))
    js["nodes"][0]["params"]["deterministic"] = True
    record("graph: deterministic JS cache declaration", True, lambda: validate_graph(js))

    def loop_graph(depth, leaf_count=1):
        body = {"nodes": [instance(f"leaf_{i}", "core.no_op") for i in range(leaf_count)], "connections": []}
        for level in range(depth):
            node = instance(f"loop_{level}", "core.loop")
            node["params"]["body"] = body
            body = {"nodes": [node], "connections": []}
        return {"engine": "ferry-graph", "schema": 1, **body}

    record("graph: three nested loops", True, lambda: validate_graph(loop_graph(3)))
    record("graph: fourth nested loop rejected", False, lambda: validate_graph(loop_graph(4)))
    record("graph: total 500 nodes including body", True, lambda: validate_graph(loop_graph(1, 499)))
    record("graph: total 501 nodes including body rejected", False, lambda: validate_graph(loop_graph(1, 500)))

    for name, source, accepted in (
        ("duplicate key", '{"a":1,"a":2}', False),
        ("nested duplicate key", '{"a":{"b":1,"b":2}}', False),
        ("NaN", '{"a":NaN}', False),
        ("Infinity", '{"a":Infinity}', False),
        ("exponent overflow", '{"a":1e400}', False),
        ("finite exponent", '{"a":1e3}', True),
        ("repeated key in distinct objects", '[{"a":1},{"a":2}]', True),
    ):
        record("JSON: " + name, accepted, lambda value=source: parse_json(value))

    parameter_cases = [
        ("core.merge", {"mode": "fields"}, False),
        ("core.merge", {"mode": "fields", "keys": []}, False),
        ("core.merge", {"mode": "fields", "keys": [{"from": "id", "to": "id"}]}, True),
        ("core.merge", {"mode": "join", "keys": [{"from": "id", "to": "id"}]}, False),
        ("core.merge", {"mode": "join", "keys": [{"from": "id", "to": "id"}], "join": "left"}, True),
        ("core.loop", {"kind": "while"}, False),
        ("core.loop", {"kind": "while", "condition": "true", "parallelism": 2}, False),
        ("core.loop", {"kind": "while", "condition": "true"}, True),
        ("core.loop", {"carry": {}, "parallelism": 2}, False),
        ("core.split", {"include": "selected"}, False),
        ("core.split", {"include": "selected", "fields": ["id"]}, True),
        ("core.aggregate", {"mode": "summary"}, False),
        ("core.aggregate", {"mode": "summary", "operations": [{"field": "v", "operation": "sum", "as": "total"}]}, True),
        ("core.wait", {"mode": "until"}, False),
        ("core.wait", {"mode": "until", "until": "2026-11-03T00:00:00Z"}, True),
        ("core.wait", {"mode": "callback"}, False),
        ("core.wait", {"mode": "callback", "response_schema": {"type": "object"}}, True),
        ("core.wait", {"mode": "callback", "response_schema": {"$ref": "https://example.invalid/schema"}}, False),
        ("core.wait", {"mode": "callback", "response_schema": {"properties": {"x": {"$dynamicRef": "https://example.invalid/schema"}}}}, False),
        ("core.wait", {"mode": "callback", "response_schema": {"type": "wrong"}}, False),
        ("core.wait", {"mode": "callback", "response_schema": {"$defs": {"item": {"type": "string"}}, "properties": {"item": {"$ref": "#/$defs/item"}}}}, True),
        ("data.date_time", {"operation": "format"}, False),
        ("data.date_time", {"operation": "add", "amount": 1}, False),
        ("data.date_time", {"operation": "add", "amount": 1, "unit": "days"}, True),
        ("data.date_time", {"operation": "difference", "other": "2026-11-02T00:00:00Z", "unit": "hours"}, True),
        ("data.crypto", {"operation": "encrypt_aes_gcm"}, False),
        ("data.crypto", {"operation": "totp", "at": "2026-11-03T00:00:00Z", "period_seconds": 30, "digits": 6}, True),
        ("data.text_split", {"delimiter": ""}, False),
        ("data.text_split", {"mode": "length"}, False),
        ("data.text_split", {"mode": "length", "length": 3, "overlap": 3}, False),
        ("data.text_split", {"mode": "length", "length": 3, "overlap": 2}, True),
        ("host.files", {"operation": "write"}, False),
        ("host.files", {"operation": "write", "asset": "asset-reference"}, True),
        ("net.http", {"pagination": {"mode": "cursor", "max_pages": 10}}, False),
        ("net.http", {"pagination": {"mode": "cursor", "max_pages": 10, "next_path": "next", "parameter": "after"}}, True),
        ("core.if", {"rules": [{"left": 1, "operator": "eq"}]}, False),
        ("core.if", {"rules": [{"left": None, "operator": "exists"}]}, True),
        ("core.switch", {"cases": [deepcopy(registry["core.switch"]["example_params"]["cases"][0])] * 2}, False),
        ("human.pick", {"min_selected": 3, "max_selected": 2}, False),
    ]
    for index, (type_name, patch, accepted) in enumerate(parameter_cases):
        value = {**deepcopy(registry[type_name]["example_params"]), **patch}
        record(f"condition {index}: {type_name}", accepted,
               lambda s=registry[type_name], v=value: validate_params(s, v))

    switch = {"engine": "ferry-graph", "schema": 1,
              "nodes": [instance("switch", "core.switch"), instance("sink", "core.no_op")],
              "connections": [{"from": {"node": "switch", "port": "case", "index": 0},
                               "to": {"node": "sink", "port": "main", "index": 0}}]}
    record("graph: switch first case", True, lambda: validate_graph(switch))
    switch["connections"][0]["from"]["index"] = 1
    record("graph: switch case out of range", False, lambda: validate_graph(switch))
    switch["connections"][0]["from"].update(port="fallback", index=0)
    switch["nodes"][0]["params"]["fallback"] = False
    record("graph: disabled switch fallback", False, lambda: validate_graph(switch))
    crypto = {"engine": "ferry-graph", "schema": 1, "nodes": [instance("crypto", "data.crypto")], "connections": []}
    crypto["nodes"][0]["params"]["operation"] = "hmac_sha256"
    record("graph: HMAC missing credential", False, lambda: validate_graph(crypto))
    crypto["nodes"][0]["credentials"] = {"key": "22"}
    record("graph: HMAC credential reference", True, lambda: validate_graph(crypto))
    merge_graph = deepcopy(graph)
    merge_graph["nodes"][1] = instance("set", "core.merge")
    merge_graph["connections"][0]["to"].update(port="input", index=1)
    record("graph: Merge last declared input", True, lambda: validate_graph(merge_graph))
    merge_graph["connections"][0]["to"]["index"] = 2
    record("graph: Merge input outside declared count", False, lambda: validate_graph(merge_graph))
    merge_graph["nodes"][1]["params"].pop("input_count")
    record("graph: Merge input count must be explicit", False, lambda: validate_graph(merge_graph))

    for index, (patch, accepted) in enumerate((
        ({"operation": "select", "limit": 20}, True),
        ({"operation": "select"}, False),
        ({"operation": "insert"}, False),
        ({"operation": "insert", "values": {}}, True),
        ({"operation": "update", "values": {}, "row_id": "1"}, False),
        ({"operation": "update", "values": {"title": "new"}, "row_id": "1", "expected_version": 2}, True),
        ({"operation": "delete", "row_id": "1", "expected_version": 2}, True),
        ({"operation": "delete", "row_id": "1", "expected_version": 2, "values": {}}, False),
        ({"operation": "select", "limit": 10, "filters": [{"column": "v", "operator": "eq"}]}, False),
        ({"operation": "select", "limit": 10, "filters": [{"column": "v", "operator": "eq", "value": None}]}, True),
        ({"operation": "select", "limit": 10, "order_by": {"column": "v", "direction": "asc", "nulls": "last"}, "after_row_id": "1"}, False),
        ({"operation": "select", "limit": 10, "order_by": {"column": "v", "direction": "asc"}}, False),
        ({"operation": "select", "limit": 10, "order_by": {"column": "v", "direction": "asc", "nulls": "last"}}, True),
    )):
        value = {"table_id": "31", **patch}
        record(f"data table operation {index}", accepted, lambda v=value: validate_params(registry["data.table"], v))

    def vector_context(embedding, dimension, metric):
        if len(embedding) != dimension or any(isinstance(value, bool) or not isinstance(value, (int, float)) for value in embedding):
            raise ValueError("vector dimension/type")
        try:
            values = [struct.unpack('<f', struct.pack('<f', value))[0] for value in embedding]
        except (OverflowError, struct.error) as error:
            raise ValueError("float32 overflow") from error
        if not all(math.isfinite(value) for value in values):
            raise ValueError("nonfinite float32")
        if metric == "cosine" and not any(value != 0 for value in values):
            raise ValueError("zero cosine vector after float32 conversion")

    for name, embedding, dimension, metric, accepted in (
        ("valid", [0.5, 0.5], 2, "cosine", True),
        ("wrong dimension", [0.5], 2, "cosine", False),
        ("overflow", [1e40, 0], 2, "dot", False),
        ("nonfinite", [float('nan'), 0], 2, "dot", False),
        ("cosine zero", [0, 0], 2, "cosine", False),
        ("cosine underflow to zero", [1e-50, 0], 2, "cosine", False),
        ("euclidean zero allowed", [0, 0], 2, "euclidean", True),
        ("boolean rejected", [True, 0], 2, "dot", False),
    ):
        record("vector context fixture: " + name, accepted,
               lambda e=embedding, d=dimension, m=metric: vector_context(e, d, m))

    file_cases = [
        ("file.encode", {"format": "csv", "columns": ["value"], "csv": {"delimiter": ",", "quote": '"', "header": True, "line_ending": "lf", "formula_policy": "escape"}}, True),
        ("file.encode", {"format": "csv", "columns": ["value"]}, False),
        ("file.encode", {"format": "xlsx", "columns": ["value"], "sheet_name": "Sheet1"}, True),
        ("file.encode", {"format": "xlsx", "columns": ["value"], "sheet_name": "bad/name"}, False),
        ("file.encode", {"format": "text", "value_path": "/value"}, True),
        ("file.encode", {"format": "base64", "value_path": "/value"}, True),
        ("file.encode", {"format": "text"}, False),
        ("file.encode", {"columns": ["value"]}, False),
        ("file.encode", {"output_name": "../rows.json"}, False),
        ("file.encode", {"limits": {}}, False),
        ("file.decode", {"format": "xlsx", "sheet_name": "Data", "formula_policy": "cached_value", "date_policy": "serial"}, True),
        ("file.decode", {"format": "xlsx", "sheet_name": "Data", "formula_policy": "reject", "date_policy": "iso8601", "source_timezone": "Asia/Kuala_Lumpur"}, True),
        ("file.decode", {"format": "xlsx", "sheet_name": "Data", "formula_policy": "reject", "date_policy": "iso8601"}, False),
        ("file.decode", {"format": "csv"}, False),
        ("file.decode", {"format": "csv", "csv": {"delimiter": ",", "quote": '"', "header": False, "line_ending": "crlf", "formula_policy": "preserve"}}, True),
        ("file.decode", {"format": "xlsx", "sheet_name": "Data", "date_policy": "serial"}, False),
        ("file.decode", {"format": "json"}, True),
        ("file.decode", {"format": "docx"}, True),
        ("file.decode", {"format": "text"}, True),
        ("file.decode", {"format": "base64"}, True),
        ("file.decode", {"pages": [1, 2]}, True),
        ("file.decode", {"pages": [1, 1]}, False),
        ("file.decode", {"format": "docx", "pages": [1]}, False),
        ("file.decode", {"format": "text", "sheet_name": "Data"}, False),
        ("file.decode", {"format": "xlsx", "sheet_name": "Data", "formula_policy": "reject", "date_policy": "serial", "source_timezone": "UTC"}, False),
        ("file.archive_pack", {"entries": [{"path": "../escape", "asset": "sha256:" + "a" * 64}]}, False),
        ("file.archive_pack", {"format": "tar_gzip"}, True),
        ("file.archive_pack", {"limits": {**registry["file.archive_pack"]["example_params"]["limits"], "max_ratio": 100}}, False),
        ("file.archive_unpack", {"limits": {**registry["file.archive_unpack"]["example_params"]["limits"], "max_ratio": 0}}, False),
        ("file.archive_unpack", {"format": "tar"}, True),
        ("file.archive_unpack", {"format": "rar"}, False),
        ("file.archive_unpack", {"limits": {"max_files": 1}}, False),
    ]
    for index, (name, patch, accepted) in enumerate(file_cases):
        value = {**deepcopy(registry[name]["example_params"]), **patch}
        record(f"file condition {index}: {name}", accepted,
               lambda s=registry[name], v=value: validate_params(s, v))
    for path, accepted in (("folder/数据.txt", True), ("/root", False), ("a//b", False),
                           ("a/./b", False), ("a/../b", False), ("C:/file", False),
                           ("a\\b", False), ("file\x00.txt", False), ("a/", False)):
        record("virtual archive path: " + repr(path), accepted, lambda p=path: safe_file_path(p))
    pack_sample = registry["file.archive_pack"]["example_params"]
    for paths in (("a", "a"), ("a", "a/b")):
        value = deepcopy(pack_sample)
        value["entries"] = [{"path": p, "asset": "sha256:" + "a" * 64} for p in paths]
        record("archive conflict: " + repr(paths), False,
               lambda v=value: validate_params(registry["file.archive_pack"], v))
    for delimiter, quote in ((",", ","), ("\n", '"'), (",", "\x00")):
        value = {**deepcopy(registry["file.encode"]["example_params"]), **file_cases[0][1]}
        value["csv"] = {**value["csv"], "delimiter": delimiter, "quote": quote}
        record("CSV dialect: " + repr((delimiter, quote)), False,
               lambda v=value: validate_params(registry["file.encode"], v))

    readiness = catalog["merge_readiness"]
    rows = readiness["terminal_matrix"]
    assert len(rows) == 16
    assert {tuple(row["inputs"]) for row in rows} == set(product("DESF", repeat=2))
    for row in rows:
        for wait in ("any_items", "all_inputs"):
            def check_row(r=row, w=wait):
                assert merge_readiness(r["inputs"], w) == r[w]
            record(f"merge truth {row['inputs']} / {wait}", True, check_row)

    def check_permutations():
        # N-ary formula and arrival order; this is not a scheduler race test.
        for size in (1, 2, 3):
            for markers in product("DESF", repeat=size):
                for wait in ("any_items", "all_inputs"):
                    expected = ("skipped" if "S" in markers else "run") if wait == "all_inputs" else (
                        "run" if any(m in "DF" for m in markers) else "skipped")
                    assert merge_readiness(markers, wait) == expected
                    for ordered in permutations(markers):
                        assert merge_readiness(ordered, wait) == expected
        assert merge_readiness(["P", "D"], "any_items") == "wait"
        assert merge_readiness(["D", "P"], "all_inputs") == "wait"
    record("merge: n-ary and arrival permutation invariant", True, check_permutations)

    for label, markers, execution, expected in (
        ("running sibling", ["D", "P"], "running", "wait"),
        ("waiting sibling", ["E", "P"], "running", "wait"),
        ("stop before sibling settles", ["H", "P"], "running", "stop"),
        ("cancel request before launch", ["D", "D"], "cancelling", "cancel"),
        ("cancelled execution", ["D", "D"], "cancelled", "cancel"),
        ("source cancellation is not skipped", ["D", "C"], "running", "cancelled_source"),
        ("unfrozen error projection", ["D", "U"], "running", "contract_blocked"),
        ("paused execution", ["D", "D"], "paused", "hold"),
        ("waiting execution", ["D", "D"], "waiting", "hold"),
        ("closed execution", ["D", "D"], "failed", "closed"),
    ):
        def check_gate(m=markers, e=execution, want=expected):
            assert merge_readiness(m, "any_items", e) == want
        record("merge gate: " + label, True, check_gate)

    projection_cases = [
        ("data", "succeeded", 1, {}, "D"), ("empty", "succeeded", 0, {}, "E"),
        ("skipped", "skipped", 0, {}, "S"),
        ("continued error", "failed", 1, {"policy": "continue"}, "F"),
        ("error port", "failed", 1, {"policy": "error_output", "port": "error"}, "F"),
        ("unselected main", "failed", 0, {"policy": "error_output"}, "U"),
        ("retry remains open", "failed", 0, {"final": False}, "P"),
        ("retry timeout remains open", "timed_out", 0, {"final": False}, "P"),
        ("terminal timeout", "timed_out", 0, {}, "H"),
        ("uncommitted partial output", "running", 5, {}, "P"),
        ("human wait", "waiting", 0, {}, "P"),
        ("cache hit", "cached", 1, {}, "D"),
        ("pinned empty", "pinned", 0, {}, "E"),
        ("reused data", "reused", 1, {}, "D"),
        ("bypassed data", "bypassed", 1, {}, "D"),
        ("cancelled source", "cancelled", 0, {}, "C"),
    ]
    for label, state, count, options, expected in projection_cases:
        def check_projection(s=state, n=count, o=options, want=expected):
            assert merge_projection(s, n, **o) == want
        record("merge projection: " + label, True, check_projection)
    for label, state, count, options in (
        ("skip with data", "skipped", 1, {}),
        ("continue without error items", "failed", 0, {"policy": "continue"}),
        ("unknown state", "unknown", 0, {}),
        ("negative count", "succeeded", -1, {}),
        ("boolean count", "succeeded", True, {}),
    ):
        record("merge invalid projection: " + label, False,
               lambda s=state, n=count, o=options: merge_projection(s, n, **o))

    # Minimal target-column projection, not the production scheduler or migration.
    with sqlite3.connect(":memory:") as db:
        db.executescript("""
            CREATE TABLE workflow_executions (
              id INTEGER PRIMARY KEY, lease_owner TEXT, lease_epoch INTEGER,
              state TEXT, context TEXT, version INTEGER);
            CREATE TABLE workflow_node_runs (
              execution_id INTEGER, node_id TEXT, iteration_hash TEXT,
              attempt INTEGER, state TEXT, output TEXT, version INTEGER,
              UNIQUE(execution_id,node_id,iteration_hash,attempt));
            INSERT INTO workflow_executions VALUES (1,'owner-a',7,'running','{"merge_inputs":{"digest":"frozen"}}',1);
            INSERT INTO workflow_node_runs VALUES (1,'merge','iteration-a',1,'running','',1);
        """)

        def commit_projection(owner, epoch, version, fail=False, attempt=1):
            with db:
                updated = db.execute("""UPDATE workflow_executions SET version=version+1
                  WHERE id=1 AND lease_owner=? AND lease_epoch=? AND version=? AND state='running'""",
                  (owner, epoch, version))
                if updated.rowcount != 1:
                    raise ValueError("stale lease, version or execution state")
                updated = db.execute("""UPDATE workflow_node_runs SET state='succeeded', output='[]', version=version+1
                  WHERE execution_id=1 AND node_id='merge' AND iteration_hash='iteration-a'
                    AND attempt=? AND state='running' AND version=1""", (attempt,))
                if updated.rowcount != 1:
                    raise ValueError("already committed or wrong attempt")
                if fail:
                    raise ValueError("simulated abort before transaction commit")

        def assert_projection(state, version, attempt=1):
            assert db.execute("SELECT state FROM workflow_node_runs WHERE attempt=?", (attempt,)).fetchone()[0] == state
            row = db.execute("SELECT version,context FROM workflow_executions").fetchone()
            assert row[0] == version and parse_json(row[1])["merge_inputs"]["digest"] == "frozen"

        record("merge SQL reference: transaction abort", False,
               lambda: commit_projection("owner-a", 7, 1, fail=True))
        record("merge SQL reference: abort retains running and frozen context", True,
               lambda: assert_projection("running", 1))
        with db:
            db.execute("UPDATE workflow_executions SET lease_owner='owner-b',lease_epoch=8,version=2 WHERE id=1")
        record("merge SQL reference: old owner rejected", False,
               lambda: commit_projection("owner-a", 7, 2))
        record("merge SQL reference: stale row version rejected", False,
               lambda: commit_projection("owner-b", 8, 1))
        record("merge SQL reference: takeover commits same attempt", True,
               lambda: commit_projection("owner-b", 8, 2))
        record("merge SQL reference: duplicate finish rejected", False,
               lambda: commit_projection("owner-b", 8, 3))
        record("merge SQL reference: duplicate finish has no extra version write", True,
               lambda: assert_projection("succeeded", 3))
        with db:
            db.execute("UPDATE workflow_executions SET state='cancelling',version=4 WHERE id=1")
            db.execute("INSERT INTO workflow_node_runs VALUES (1,'merge','iteration-a',2,'running','',1)")
        record("merge SQL reference: cancel blocks commit", False,
               lambda: commit_projection("owner-b", 8, 4, attempt=2))
        record("merge SQL reference: cancellation preserves uncommitted node", True,
               lambda: (assert_projection("running", 4, attempt=2), assert_projection("succeeded", 4)))

    embedded_cases = [
        ("root empty", {}, True), ("boolean true", True, True), ("boolean false", False, True),
        ("missing pointer", {"$ref": "#/$defs/missing"}, False),
        ("forward pointer", {"$ref": "#/$defs/x", "$defs": {"x": {"type": "string"}}}, True),
        ("boolean target", {"$ref": "#/$defs/x", "$defs": {"x": False}}, True),
        ("pointer escaped slash and tilde", {"$ref": "#/$defs/a~1b~0c", "$defs": {"a/b~c": {}}}, True),
        ("tilde decoded once", {"$ref": "#/$defs/~01", "$defs": {"~1": {}}}, True),
        ("bad tilde escape", {"$ref": "#/$defs/a~2b", "$defs": {"a~2b": {}}}, False),
        ("percent UTF8", {"$ref": "#/$defs/%E4%B8%AD", "$defs": {"中": {}}}, True),
        ("percent slash", {"$ref": "#%2F$defs%2Fx", "$defs": {"x": {}}}, True),
        ("percent decoded once", {"$ref": "#/$defs/%252F", "$defs": {"%2F": {}}}, True),
        ("invalid percent", {"$ref": "#/$defs/%ZZ"}, False),
        ("invalid percent UTF8", {"$ref": "#/$defs/%FF"}, False),
        ("empty property token", {"$ref": "#/properties/", "properties": {"": {}}}, True),
        ("array schema site", {"$ref": "#/allOf/0", "allOf": [{}]}, True),
        ("array leading zero", {"$ref": "#/allOf/00", "allOf": [{}]}, False),
        ("array append token", {"$ref": "#/allOf/-", "allOf": [{}]}, False),
        ("ordinary object not schema", {"$ref": "#/default", "default": {}}, False),
        ("scalar target", {"$ref": "#/type", "type": "string"}, False),
        ("anchor forward", {"$ref": "#record", "$defs": {"x": {"$anchor": "record"}}}, True),
        ("missing anchor", {"$ref": "#missing"}, False),
        ("duplicate anchor", {"$defs": {"x": {"$anchor": "same"}, "y": {"$anchor": "same"}}}, False),
        ("remote reference", {"$ref": "https://example.invalid/schema"}, False),
        ("file reference", {"$ref": "file:///private/schema"}, False),
        ("relative resource", {"$ref": "other.json#x"}, False),
        ("id resource boundary", {"$defs": {"x": {"$id": "child"}}}, False),
        ("dynamic vocabulary", {"$dynamicRef": "#node"}, False),
        ("unknown extension", {"custom_validator": True}, False),
        ("wrong dialect", {"$schema": "http://json-schema.org/draft-07/schema#"}, False),
        ("selected dialect", {"$schema": schema_policy["dialect"]}, True),
        ("literal ref in annotations", {"default": {"$ref": "https://example.invalid"}, "examples": [{"$id": "child"}]}, True),
        ("unreferenced bad definition", {"$defs": {"x": {"$ref": "#absent"}}}, False),
        ("legacy definitions mapping type", {"definitions": []}, False),
        ("legacy definitions child type", {"definitions": {"x": 1}}, False),
        ("root self recursion", {"$ref": "#"}, True),
        ("mutual recursion", {"$defs": {"x": {"$ref": "#/$defs/y"}, "y": {"$ref": "#/$defs/x"}}}, True),
    ]
    for label, schema, accepted in embedded_cases:
        record("embedded schema: " + label, accepted,
               lambda s=schema: validate_embedded_schema(s, schema_policy, schema_policy["example_limits"]))

    def assert_cycle(schema, expected):
        assert validate_embedded_schema(schema, schema_policy, schema_policy["example_limits"])["has_cycles"] is expected
    record("embedded schema: cycle detected without expansion", True,
           lambda: assert_cycle({"$ref": "#"}, True))
    record("embedded schema: shared DAG is not recursion", True,
           lambda: assert_cycle({"$defs": {"x": {}}, "allOf": [{"$ref": "#/$defs/x"}, {"$ref": "#/$defs/x"}]}, False))
    bounded = {"$defs": {"x": {"type": "string"}},
               "properties": {"a": {"$ref": "#/$defs/x"}, "b": {"$ref": "#/$defs/x"}}}
    metrics = validate_embedded_schema(bounded, schema_policy, schema_policy["example_limits"])
    for key in schema_policy["example_limits"]:
        metric = key.removeprefix("max_")
        cutoff = metrics[metric]
        assert cutoff > 1
        for limit, accepted in ((cutoff, True), (cutoff - 1, False)):
            budgets = {**schema_policy["example_limits"], key: limit}
            record(f"embedded schema budget: {key}={limit}", accepted,
                   lambda b=budgets: validate_embedded_schema(bounded, schema_policy, b))
    record("embedded schema budget: missing explicit configuration", False,
           lambda: validate_embedded_schema({}, schema_policy, {}))

    for group in ("schema_maps", "schema_arrays", "schema_values"):
        def assert_all_sites(g=group):
            for keyword in schema_policy[g]:
                good = {"$ref": "#"}
                valid_container = {"x": good} if g == "schema_maps" else [good] if g == "schema_arrays" else good
                validate_embedded_schema({keyword: valid_container}, schema_policy, schema_policy["example_limits"])
                bad = {"$ref": "#missing"}
                container = {"x": bad} if g == "schema_maps" else [bad] if g == "schema_arrays" else bad
                try:
                    validate_embedded_schema({keyword: container}, schema_policy, schema_policy["example_limits"])
                except ValueError:
                    continue
                raise AssertionError("unresolved reference missed at " + keyword)
        record("embedded schema: traverse every " + group, True, assert_all_sites)
    deep_annotation = None
    for _ in range(schema_policy["example_limits"]["max_json_depth"] + 1):
        deep_annotation = [deep_annotation]
    record("embedded schema budget: annotation depth is counted", False,
           lambda: validate_embedded_schema({"default": deep_annotation}, schema_policy, schema_policy["example_limits"]))

    http_slots = registry["net.http"]["credential_slots"]
    fixture_credential = {"workspace_id": "1", "scope": "build", "active": True,
                          "allowed_actors": ["21"], "has_secret": True, "kind": "token",
                          "profile": "http.bearer", "version": 1, "adapter_version": "fixture-1",
                          "public_config_hash": "a" * 64, "secret": "FIXTURE-SECRET-NEVER-OUTPUT"}
    def check_credential(slots=http_slots, bindings=None, row=None, actor="21", workspace="1", scope="build", operation=None):
        return credential_projection(slots, {"auth": "22"} if bindings is None else bindings,
                                     {"22": deepcopy(fixture_credential) if row is None else row},
                                     workspace=workspace, actor=actor, module_scope=scope,
                                     profiles=credential_policy["profiles"], operation=operation)

    for label, patch, accepted in (
        ("valid bearer", {}, True), ("cross workspace", {"workspace_id": "2"}, False),
        ("wrong module", {"scope": "host"}, False), ("revoked metadata", {"active": False}, False),
        ("no effective authority", {"allowed_actors": []}, False), ("missing secret", {"has_secret": False}, False),
        ("same kind wrong profile", {"profile": "crypto.totp"}, False),
        ("wrong kind", {"kind": "ssh-key"}, False),
        ("unknown profile", {"profile": "vendor.unfrozen"}, False),
        ("missing adapter version", {"adapter_version": ""}, False),
        ("invalid row version", {"version": 0}, False),
        ("invalid public projection hash", {"public_config_hash": "raw-config"}, False),
        ("typed basic", {"profile": "http.basic", "kind": "password"}, True),
        ("typed header", {"profile": "http.header"}, True),
        ("typed OAuth metadata only", {"profile": "http.oauth2"}, True),
    ):
        value = {**deepcopy(fixture_credential), **patch}
        record("credential metadata: " + label, accepted, lambda r=value: check_credential(row=r))
    for label, options, accepted in (
        ("optional absent", {"bindings": {}}, True),
        ("unknown slot", {"bindings": {"other": "22"}}, False),
        ("inline secret reference", {"bindings": {"auth": "raw-secret"}}, False),
        ("integer ID", {"bindings": {"auth": 22}}, False),
        ("explicit null is not omitted", {"bindings": {"auth": None}}, False),
        ("missing row", {"bindings": {"auth": "23"}}, False),
        ("runtime identity differs from publisher", {"actor": "20"}, False),
        ("invented workflow module", {"scope": "workflow"}, False),
        ("SMTP required", {"slots": registry["net.smtp"]["credential_slots"], "bindings": {}}, False),
        ("hash needs no key", {"slots": registry["data.crypto"]["credential_slots"], "bindings": {}, "operation": "sha256"}, True),
        ("HMAC requires key", {"slots": registry["data.crypto"]["credential_slots"], "bindings": {}, "operation": "hmac_sha256"}, False),
    ):
        record("credential binding: " + label, accepted, lambda o=options: check_credential(**o))
    for operation, profile in registry["data.crypto"]["credential_slots"]["key"]["operation_profiles"].items():
        for actual_profile, accepted in ((profile, True), ("http.bearer", False)):
            row = {**deepcopy(fixture_credential), "profile": actual_profile}
            record(f"credential crypto: {operation}/{actual_profile}", accepted,
                   lambda r=row, op=operation: check_credential(slots=registry["data.crypto"]["credential_slots"],
                                                              bindings={"key": "22"}, row=r, operation=op))
    smtp_row = {**deepcopy(fixture_credential), "profile": "smtp.account", "kind": "password"}
    record("credential SMTP: typed account metadata", True,
           lambda: check_credential(slots=registry["net.smtp"]["credential_slots"], bindings={"smtp": "22"}, row=smtp_row))
    for stage in ("publish", "attempt", "cache_hit", "before_secret_use"):
        record("credential revocation reference at " + stage, False,
               lambda: check_credential(row={**fixture_credential, "allowed_actors": []}))

    def check_partition(patch, changed):
        before = check_credential()
        after = check_credential(row={**fixture_credential, **patch})
        assert (before != after) is changed
        assert set(after[0]) == set(credential_policy["cache_identity_fields"])
        assert "FIXTURE-SECRET" not in json.dumps(after)
    for label, patch, changed in (
        ("row version changes on rotation", {"version": 2}, True),
        ("nonsecret config changes", {"public_config_hash": "b" * 64}, True),
        ("adapter version changes", {"adapter_version": "fixture-2"}, True),
        ("secret never directly hashed", {"secret": "OTHER-FIXTURE-SECRET"}, False),
    ):
        record("credential cache projection: " + label, True, lambda p=patch, c=changed: check_partition(p, c))

    for source, target, segment, expected in (
        ("main", "main", None, "compatible"), ("main", "image", None, "compatible"),
        ("text", "main", None, "compatible"), ("image", "text", None, "incompatible"),
        ("text", "text", None, "compatible"), ("model", "model", None, "compatible"),
        ("main", "model", None, "unfrozen"), ("model", "main", None, "unfrozen"),
        ("model", "text", None, "incompatible"), ("gpu:latent", "main", True, "incompatible"),
        ("main", "gpu:latent", True, "incompatible"),
        ("gpu:latent", "gpu:latent", True, "compatible"),
        ("gpu:latent", "gpu:latent", False, "incompatible"),
        ("gpu:latent", "gpu:latent", None, "unfrozen"),
        ("gpu:latent", "gpu:conditioning", True, "incompatible"),
        ("json<schema>", "text", None, "unfrozen"),
    ):
        def check_relation(s=source, t=target, same=segment, want=expected):
            assert port_compatibility(s, t, same) == want
        record(f"port relation: {source}/{target}/{segment}", True, check_relation)

    smtp_graph = {"engine": "ferry-graph", "schema": 1, "nodes": [instance("mail", "net.smtp")], "connections": []}
    record("graph stage: missing SMTP slot allowed in structure fixture", True,
           lambda: validate_graph(smtp_graph, stage="structure"))
    record("graph stage: missing SMTP slot rejected in resolved contract", False,
           lambda: validate_graph(smtp_graph))
    host_graph = {"engine": "ferry-graph", "schema": 1, "nodes": [instance("host", "host.command")], "connections": []}
    record("graph stage: missing host binding allowed in structure fixture", True,
           lambda: validate_graph(host_graph, stage="structure"))
    record("graph stage: missing host binding rejected in resolved contract", False,
           lambda: validate_graph(host_graph))
    host_graph["nodes"][0]["params"]["command"] = "echo {{ value }}"
    record("graph stage: command interpolation still rejected in structure fixture", False,
           lambda: validate_graph(host_graph, stage="structure"))

    def edge(origin, target, port="main", index=0):
        return {"from": {"node": origin, "port": "main", "index": 0},
                "to": {"node": target, "port": port, "index": index}}
    topo = {"engine": "ferry-graph", "schema": 1,
            "nodes": [instance("a", "core.trigger.manual"), instance("b", "core.trigger.manual"),
                      instance("join", "core.merge"), instance("island", "core.no_op")],
            "connections": [edge("a", "join", "input", 0)]}
    def check_findings(g, expected, entry=None):
        validate_graph(g)
        report = topology_review(g, registry, entry)
        assert {issue["code"] for issue in report["findings"]} == set(expected)
        assert all(issue["severity"] == "warning" for issue in report["findings"])
        return report
    base_findings = {"merge_input_unconnected", "input_unconnected", "not_reachable_from_trigger"}
    record("topology review: missing inputs and island are warnings", True,
           lambda: check_findings(topo, base_findings))
    connected = deepcopy(topo)
    connected["nodes"] = [node for node in connected["nodes"] if node["id"] != "island"]
    connected["connections"].append(edge("b", "join", "input", 1))
    record("topology review: complete merge across two entry candidates", True,
           lambda: check_findings(connected, set()))
    record("topology review: other entry cannot supply selected merge run", True,
           lambda: check_findings(connected, {"merge_other_entry_dependency"}, "a"))
    fanin = deepcopy(connected)
    fanin["connections"][1]["to"]["index"] = 0
    record("topology review: multi-producer slot does not fill missing slot", True,
           lambda: check_findings(fanin, {"input_fanin_unfrozen", "merge_input_unconnected"}))
    for entry in (None, "missing", "join"):
        record("trigger start: explicit entry required " + repr(entry), False,
               lambda e=entry: topology_review(connected, registry, e, require_entry=True))
    def check_selection():
        report = topology_review(connected, registry, "a", require_entry=True)
        assert report["reachable_nodes"] == ["a", "join"] and report["triggers"] == ["a", "b"]
        assert topology_review(connected, registry)["reachable_nodes"] is None
        reordered = deepcopy(connected)
        reordered["nodes"].reverse()
        reordered["connections"].reverse()
        assert report == topology_review(reordered, registry, "a", require_entry=True)
    record("trigger start: node order does not choose/expand entry", True, check_selection)
    branch = deepcopy(graph)
    branch["nodes"][1] = instance("choose", "core.if")
    branch["connections"][0]["to"]["node"] = "choose"
    record("topology review: unused branch outputs do not require dummy sinks", True,
           lambda: check_findings(branch, set()))
    unicode_pin = {"set": [{"json": {"text": ""}}]}
    overhead = len(json.dumps(unicode_pin, ensure_ascii=False, separators=(",", ":")).encode("utf-8"))
    characters, padding = divmod(1024 * 1024 - overhead, 3)
    unicode_pin["set"][0]["json"]["text"] = "中" * characters + "x" * padding
    pin_graph = {**deepcopy(graph), "pin_data": unicode_pin}
    record("graph UTF8 pin budget: exact 1MiB with Unicode", True, lambda: validate_graph(pin_graph))
    unicode_pin["set"][0]["json"]["text"] += "x"
    record("graph UTF8 pin budget: 1MiB plus one byte", False, lambda: validate_graph(pin_graph))

    result = {"result": "PASS", "scope": "design schemas and static fixture checks only",
              "manifests": len(registry), "source_members": len(source_map),
              "core_manifests": len(catalog["nodes"]), "data_manifests": len(data_catalog["nodes"]),
              "file_manifests": len(file_catalog["nodes"]),
              "mapping_status": dict(Counter(row["contract_status"] for row in source_map)),
              "cases": cases,
              "merge_truth_rows": len(rows),
              "embedded_schema_profile": schema_policy["version"],
              "embedded_schema_limit_status": schema_policy["limit_status"],
              "credential_profile": credential_policy["version"],
              "graph_publication_profile": catalog["graph_publication_contract"]["version"],
              "not_tested": ["publisher, trigger binding/index atomicity and runtime planner; incomplete draft API and typed/GPU payload validation", "expression runtime", "OS sandbox", "permissions/DB concurrency", "provider/host execution", "screens/translations", "actual archive/parser limits, isolation and format roundtrip", "Merge scheduler, crash recovery and atomic commits; position/field/join data algebra", "runtime Schema instance validation budgets, regex interruption and cross-language dialect parity", "credential vault/live permissions/rotation paths, typed adapters, OAuth refresh and credential-aware runtime cache"]}
    result["artifact_sha256"] = {
        path.relative_to(ROOT).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in (HERE / "workflow-graph.schema.json", HERE / "workflow-core-nodes.json", HERE / "workflow-data-nodes.json", HERE / "workflow-file-nodes.json", DOCS / "check_contracts.py", DOCS / "README.md", ROOT / "docs/ferry-工作流能力清单.md", ROOT / "docs/prd/ferry-工作流平台PRD.md", ROOT / "docs/prd/ferry-工作流平台数据模型.md")
    }
    destination = ROOT / "data/maintenance/workflow-contract-validation.json"
    destination.parent.mkdir(parents=True, exist_ok=True)
    serialized = (json.dumps(result, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
    # Keep evidence for each artifact revision while retaining the latest alias.
    evidence = destination.parent / "workflow-review-evidence"
    evidence.mkdir(exist_ok=True)
    for payload in ([destination.read_bytes()] if destination.exists() else []) + [serialized]:
        digest = hashlib.sha256(payload).hexdigest()
        snapshot = evidence / f"contracts-{digest}.json"
        if not snapshot.exists():
            snapshot.write_bytes(payload)
    destination.write_bytes(serialized)
    print(f"PASS: {len(registry)} manifests, {len(source_map)} source mappings, {len(cases)} cases; {destination}")


if __name__ == "__main__":
    run()
