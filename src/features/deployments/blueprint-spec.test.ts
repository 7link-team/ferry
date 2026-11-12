import { beforeEach, expect, it } from "vitest";
import { i18n } from "../../shared/i18n/i18n";
import { initialSpec, instancesProblem, isBlueprint, readBlueprint, readSpec, stepInstances, writeSpec } from "./blueprint-spec";

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
  expect(readSpec(`${commented}failover: {}\n`).errors[0]).toContain("failover");
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
  expect(readBlueprint(json)).toEqual({ name: "mq", roles: [{ name: "nats", instances: { min: 3, max: 9, default: 3, odd: true }, identity: "stable", distinctHosts: false, colocateWith: undefined }], drivers: ["docker"] });
});
