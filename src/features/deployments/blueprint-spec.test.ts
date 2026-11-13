import { beforeEach, expect, it } from "vitest";
import { i18n } from "../../shared/i18n/i18n";
import { failoverClass, initialSpec, instancesProblem, isBlueprint, readBlueprint, readSpec, stepInstances, topologyKey, writeSpec } from "./blueprint-spec";

beforeEach(async () => { await i18n.changeLanguage("en-US"); });

const commented = `# deployment of the message bus
apiVersion: ferry/deploy-v1
template: {id: "42", version: 3}
driver: docker # chosen by ops
values:
  client_port: 4222 # keep me
roles:
  nats:
    instances: 3
    hosts:
      tags: [cache]
`;

it("failover_roundtrip_and_validation", () => {
  const source = `${commented}failover:\n  mode: auto # keep policy note\n  hostDownAfter: 90\n`;
  const read = readSpec(source);
  expect(read.errors).toEqual([]);
  const updated = writeSpec(read.doc!, [{ path: ["failover", "mode"], value: "alert" }]);
  expect(updated).toContain("# keep policy note");
  expect(updated).toContain("# chosen by ops");
  expect(readSpec(updated).spec?.failover).toEqual({ mode: "alert", hostDownAfter: 90 });
  for (const value of ["mode: sometimes", "mode: null", "hostDownAfter: 10", "hostDownAfter: 3601", "hostDownAfter: 30.5", 'hostDownAfter: "90"', "unknown: true"]) {
    expect(readSpec(`${commented}failover:\n  ${value}\n`).errors).toHaveLength(1);
  }
  for (const hostDownAfter of [30, 3600]) expect(readSpec(`${commented}failover: {hostDownAfter: ${hostDownAfter}}\n`).errors).toEqual([]);
});

it("topology_key_ignores_failover", () => {
  const spec = readSpec(commented).spec!;
  expect(topologyKey({ ...spec, failover: { mode: "off", hostDownAfter: 300 } })).toBe(topologyKey(spec));
});

it("failover_class_matches_server_rules", () => {
  const role = readBlueprint(JSON.stringify({ kind: "Blueprint", roles: [{ name: "r", instances: {min: 1,max: 3,default: 1}, identity: "stateless" }] }))!.roles[0]!;
  expect(failoverClass(role)).toBe("stateless");
  expect(failoverClass({ ...role, identity: "stable", failoverReplace: true })).toBe("replicated");
  expect(failoverClass({ ...role, identity: "stable", failoverReplace: false })).toBe("manual");
});

it("spec_roundtrip_preserves_comments", () => {
  const read = readSpec(commented);
  expect(read.errors).toEqual([]);
  expect(read.spec?.roles.nats?.instances).toBe(3);
  expect(read.spec?.template).toEqual({ id: "42", version: 3 });
  const text = writeSpec(read.doc!, [
    { path: ["roles", "nats", "instances"], value: 5 },
    { path: ["roles", "nats", "hosts", "names"], value: ["host-a", "host-b"] },
    { path: ["roles", "nats", "hosts", "tags"], value: [] },
  ]);
  expect(text).toContain("# deployment of the message bus");
  expect(text).toContain("# chosen by ops");
  expect(text).toContain("# keep me");
  const again = readSpec(text);
  expect(again.errors).toEqual([]);
  expect(again.spec?.roles.nats).toEqual({ instances: 5, hosts: { names: ["host-a", "host-b"] } });
  // The canonical initial document reads back as the same spec.
  const fresh = initialSpec({ template: { id: "42", version: 3 }, driver: "stack", values: { port: 1 }, roles: { nats: { instances: 3 } } });
  expect(fresh.startsWith("apiVersion: ferry/deploy-v1")).toBe(true);
  expect(readSpec(fresh).spec).toEqual({ template: { id: "42", version: 3 }, driver: "stack", values: { port: 1 }, roles: { nats: { instances: 3 } } });
});

it("rejects syntax errors, wrong apiVersion and unknown keys at every level", () => {
  expect(readSpec("roles: [").errors).toHaveLength(1);
  expect(readSpec(commented.replace("ferry/deploy-v1", "ferry/v1")).errors[0]).toContain("ferry/deploy-v1");
  expect(readSpec(`${commented}surprise: {}\n`).errors[0]).toContain("surprise");
  expect(readSpec(commented.replace("tags: [cache]", "tags: [cache]\n      zone: a")).errors[0]).toContain("zone");
  expect(readSpec(commented.replace("instances: 3", "instances: three")).errors[0]).toContain("instances");
  expect(readSpec(`${commented}placement:\n  nats:\n    - {host: "1"}\n`).errors[0]).toContain("host");
  // Types mirror the server: strings stay strings, IDs may be integers, instances are 1–100.
  expect(readSpec(commented.replace("driver: docker", "driver: 7")).errors[0]).toContain("driver");
  expect(readSpec(commented.replace("tags: [cache]", "group: 5")).errors[0]).toContain("group");
  expect(readSpec(commented.replace("tags: [cache]", "spread: {prefix: 3}")).errors[0]).toContain("spread.prefix");
  expect(readSpec(`${commented}manager: 12\n`).errors[0]).toContain("manager");
  expect(readSpec(`${commented}placement:\n  nats:\n    - {hostId: 1, address: 10}\n`).errors[0]).toContain("address");
  expect(readSpec(`${commented}placement:\n  nats:\n    - {hostId: 1, address: "10.0.0.1"}\n`).spec?.placement).toEqual({ nats: [{ hostId: "1", address: "10.0.0.1" }] });
  expect(readSpec(commented.replace("instances: 3", "instances: 0")).errors[0]).toContain("instances");
  expect(readSpec(commented.replace("instances: 3", "instances: 101")).errors[0]).toContain("instances");
  expect(readSpec(commented.replace("instances: 3", "instances: 100")).errors).toEqual([]);
});

it("instances stepper keeps odd counts inside the template bounds", () => {
  const bounds = { min: 3, max: 9, default: 3, odd: true };
  expect(stepInstances(3, 1, bounds)).toBe(5);
  expect(stepInstances(9, 1, bounds)).toBe(9);
  expect(stepInstances(3, -1, bounds)).toBe(3);
  expect(stepInstances(4, 1, bounds)).toBe(5);
  expect(stepInstances(4, -1, bounds)).toBe(3);
  expect(instancesProblem(4, bounds)).toBe("odd");
  expect(instancesProblem(11, bounds)).toBe("range");
  expect(instancesProblem(5, bounds)).toBeUndefined();
  expect(stepInstances(1, 1, { min: 1, max: 2, default: 1, odd: false })).toBe(2);
});

it("recognizes blueprint templates in JSON and YAML", () => {
  const json = JSON.stringify({ apiVersion: "ferry/blueprint-v1", kind: "Blueprint", metadata: { name: "mq" }, roles: [{ name: "nats", instances: { min: 3, max: 9, default: 3, odd: true }, identity: "stable", service: {} }], drivers: { docker: { supported: true }, native: { supported: false }, k8s: { supported: true } } });
  expect(isBlueprint(json)).toBe(true);
  expect(isBlueprint("apiVersion: ferry/v1\nkind: Application\n")).toBe(false);
  expect(isBlueprint("kind: Blueprint\nroles:\n  - published: {{ .Values.port }}\n")).toBe(true);
  expect(readBlueprint(json)).toEqual({ name: "mq", roles: [{ name: "nats", instances: { min: 3, max: 9, default: 3, odd: true }, identity: "stable", distinctHosts: false, colocateWith: undefined, failoverReplace: false }], drivers: ["docker"] });
});

it("topology_key_ignores_values_version_and_placement_addresses", () => {
  const spec = (patch: string) => readSpec(`apiVersion: ferry/deploy-v1
template: {id: "42", version: 3}
driver: stack
manager: host-a
values: {client_port: 4222}
roles: {nats: {instances: 2}}
${patch}`).spec!;
  const base = spec("placement: {nats: [{hostId: '1', address: 10.0.0.1}, {hostId: '2', address: 10.0.0.2}]}\n");
  // Values, the template version and addresses (derived from hosts by the server) may change.
  const allowed = readSpec(initialSpec({ ...base, template: { id: "42", version: 4 }, values: { client_port: 1 },
    placement: { nats: [{ hostId: "1", address: "nats-0.internal" }, { hostId: "2", address: "10.9.9.9" }] } })).spec!;
  expect(topologyKey(allowed)).toBe(topologyKey(base));
  // Moving an instance to another host, or another manager, changes the topology.
  const moved = { ...base, placement: { nats: [{ hostId: "1", address: "10.0.0.1" }, { hostId: "3", address: "10.0.0.2" }] } };
  expect(topologyKey(moved)).not.toBe(topologyKey(base));
  expect(topologyKey({ ...base, manager: "host-b" })).not.toBe(topologyKey(base));
  expect(topologyKey({ ...base, template: { id: "43", version: 3 } })).not.toBe(topologyKey(base));
});
