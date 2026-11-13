import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { parse, stringify } from "yaml";
import { commonText, i18n } from "../../shared/i18n/i18n";
import { BlueprintSpecEdit } from "./BlueprintSpecEdit";
import { BlueprintTarget, BlueprintTopology } from "./BlueprintTopology";
import type { Deployment } from "./deployment";

vi.mock("../../shared/editor/CodeEditor", () => ({ CodeEditor: ({ value, label, onChange }: { value: string; label: string; onChange: (value: string) => void }) => <textarea aria-label={label} value={value} onChange={event => onChange(event.target.value)} /> }));

const t = (key: string, values?: Record<string, unknown>) => commonText(`resource-domain:topology.${key}`, values);
const w = (key: string, values?: Record<string, unknown>) => commonText(`resource-domain:wizard.${key}`, values);
const blueprint = (driver: string) => ({
  apiVersion: "ferry/blueprint-v1", kind: "Blueprint", metadata: { name: "nats-cluster" },
  roles: [
    { name: "nats", instances: { min: 3, max: 9, default: 3, odd: true }, placement: { distinctHosts: true }, identity: "stable", service: { image: "nats" } },
    { name: "web", instances: { min: 1, max: 100, default: 1 }, identity: "stateless", service: { image: "web" } },
  ],
  drivers: { [driver]: { supported: true } },
});
const hosts = [{ id: "1", name: "host-a", address: "10.0.0.1" }, { id: "2", name: "host-b", address: "10.0.0.2" }, { id: "3", name: "host-c", address: "10.0.0.3" }];
const bind = (id: string) => ({ hostId: id, address: `10.0.0.${id}` });
const spec = { apiVersion: "ferry/deploy-v1", template: { id: "42", version: 3 }, driver: "docker", values: { client_port: 4222 }, roles: { nats: { instances: 3 }, web: { instances: 1 } } };
const deployment = (patch: Partial<Deployment> = {}): Deployment => ({
  id: "7", name: "nats-prod", manifest: JSON.stringify(blueprint("docker")), mode: "docker", target_kind: "blueprint", target_ref: "0",
  workdir: "/opt/deployments", description: "", state: "idle", version: 4, current_revision: "0",
  blueprint_binding: { driver: "docker", roles: { web: [bind("3")], nats: [bind("1"), bind("2"), bind("3")] } },
  deploy_spec: stringify(spec),
  ...patch,
});

let root: Root, host: HTMLDivElement, requests: { url: string; method: string; body?: Record<string, unknown> }[];
let health: () => { status: number; body: unknown } | Promise<{ status: number; body: unknown }>;
let patchReply: () => { status: number; body: unknown };
let templateReply: () => { status: number; body: unknown } | Promise<{ status: number; body: unknown }>;

beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  await i18n.changeLanguage("en-US");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  requests = [];
  health = () => ({ status: 200, body: { id: "7", placement: [] } });
  patchReply = () => ({ status: 200, body: {} });
  templateReply = () => ({ status: 200, body: { id: "42", name: "NATS cluster", version: 3, content: JSON.stringify(blueprint("docker")), parameters: JSON.stringify([{ name: "client_port", type: "integer", default: 4222, min: 1, max: 65535 }]) } });
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    const method = init.method ?? "GET";
    const body = init.body ? JSON.parse(String(init.body)) as Record<string, unknown> : undefined;
    requests.push({ url, method, ...(body ? { body } : {}) });
    const reply = (status: number, data: unknown) => ({ ok: status < 400, status, headers: new Headers({ "content-type": "application/json" }), json: async () => data });
    if (url.includes("/application-health/7")) { const r = await health(); return reply(r.status, r.body); }
    if (url.includes("/nodes")) return reply(200, { items: hosts, total: 3, page: 1, page_size: 200 });
    if (url.includes("/app-templates/42")) { const r = await templateReply(); return reply(r.status, r.body); }
    if (url.includes("/tags")) return reply(200, { items: [], total: 0, page: 1, page_size: 200 });
    if (url.includes("/node-groups")) return reply(200, { items: [], total: 0, page: 1, page_size: 200 });
    if (url.includes("/deployment-placements")) return reply(200, { manifest: "{}", binding: { driver: "docker", roles: { nats: [bind("1"), bind("2"), bind("3")], web: [bind("1")] } }, warnings: [] });
    if (url.endsWith("/deployments/7") && method === "PATCH") { const r = patchReply(); return reply(r.status, r.body); }
    return reply(404, { error: "missing" });
  }));
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

const flush = async () => { for (let i = 0; i < 6; i++) await act(async () => { await Promise.resolve(); }); };
const all = new Set(["deployments.read", "deployments.update", "deployments.create", "alerts.read", "hosts.read", "app-templates.read"]);
const mountTopology = async (app: Deployment, permissions = all) => {
  await act(async () => root.render(<BlueprintTopology workspace="ops" deployment={app} permissions={permissions} />));
  await flush();
};
const roleGroups = () => [...host.querySelectorAll<HTMLElement>("[data-topology-role]")];
const rowsOf = (role: string) => [...host.querySelectorAll<HTMLElement>(`[data-topology-role="${role}"] [data-topology-instance]`)];
const stateOf = (instance: string) => host.querySelector(`[data-topology-instance="${instance}"] [data-health]`)?.getAttribute("data-health");
const button = (name: string) => [...host.querySelectorAll("button")].find(b => b.textContent === name || b.getAttribute("aria-label") === name)!;
const click = async (name: string) => { const target = button(name); expect(target, name).toBeDefined(); await act(async () => target.click()); await flush(); };
const alertWith = (text: string) => [...host.querySelectorAll('[role="alert"]')].find(alert => alert.textContent?.includes(text));

it("fenced_and_attention_badges_match_host_and_instance", async () => {
  await mountTopology(deployment({ failover: { mode: "auto", host_down_after: 90, fences: [{ host_id: "1", instance: "nats-0", created_at: 1 }, { host_id: "99", instance: "nats-1", created_at: 1 }], attention: [{ instance: "nats-1", since: 1 }] } }));
  expect(rowsOf("nats")[0]!.textContent).toContain(t("fenced"));
  expect(rowsOf("nats")[1]!.textContent).not.toContain(t("fenced"));
  expect(rowsOf("nats")[1]!.textContent).toContain(t("needsAttention"));
});

it("replace_action_only_for_supported_driver_class_and_permission", async () => {
  const permissions = new Set([...all, "deployments.apply"]);
  await mountTopology(deployment(), permissions);
  expect(rowsOf("web")[0]!.querySelector("button")).not.toBeNull();
  expect(rowsOf("nats")[0]!.querySelector("button")).toBeNull();
  for (const mode of ["stack", "compose"]) { await mountTopology(deployment({ mode }), permissions); expect(host.querySelector('[data-replace-instance]')).toBeNull(); }
  for (const state of ["queued", "running", "needs-review"]) { await mountTopology(deployment({ state }), permissions); expect(host.querySelector('[data-replace-instance]')).toBeNull(); }
  await mountTopology(deployment(), all); expect(host.querySelector('[data-replace-instance]')).toBeNull();
});

it("replace_confirms_then_posts_version_and_accepts_operation_id", async () => {
  await mountTopology(deployment(), new Set([...all, "deployments.apply"]));
  const pressReplace = async () => { const b = rowsOf("web")[0]!.querySelector<HTMLButtonElement>("button")!; expect(b).not.toBeNull(); await act(async () => b.click()); await flush(); };
  await pressReplace(); expect(document.querySelector('[role="dialog"]')).not.toBeNull();
  const dialogButton = (name: string) => [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find(b => b.textContent === name)!;
  await act(async () => dialogButton(commonText("common.cancel")).click());
  expect(requests.filter(r => r.method === "POST")).toHaveLength(0);
  await pressReplace();
  const prior = vi.mocked(fetch).getMockImplementation()!;
  vi.mocked(fetch).mockImplementation(async (...args) => {
    if (String(args[0]).endsWith("/instances/web/0/replace")) { requests.push({ url: String(args[0]), method: "POST", body: JSON.parse(String(args[1]?.body)) }); return { ok: true, status: 202, json: async () => ({ operation_id: "81" }) } as Response; }
    return prior(...args);
  });
  await act(async () => dialogButton(commonText("delete.confirm", { action: commonText("resource-domain:failover.replace") })).click()); await flush();
  expect(requests.find(r => r.url.endsWith("/instances/web/0/replace"))).toMatchObject({ method: "POST", body: { version: 4 } });
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  expect(host.textContent).toContain(commonText("resource-domain:failover.accepted", { id: "81" }));
});

it("topology_groups_instances_by_role_with_host_names", async () => {
  await mountTopology(deployment());
  // Roles follow the blueprint's order, not the binding's key order.
  expect(roleGroups().map(group => group.getAttribute("data-topology-role"))).toEqual(["nats", "web"]);
  expect(rowsOf("nats").map(row => row.getAttribute("data-topology-instance"))).toEqual(["nats-0", "nats-1", "nats-2"]);
  expect(rowsOf("nats")[0]!.textContent).toContain("host-a");
  expect(rowsOf("nats")[2]!.textContent).toContain("host-c");
  expect(rowsOf("web")[0]!.textContent).toContain("web-0");
  expect(rowsOf("web")[0]!.textContent).toContain("host-c");
  expect(roleGroups()[0]!.textContent).toContain(t("instanceCount", { count: 3 }));
  // Phone stacks each instance as a card: rows are list items, never a wide table.
  expect(host.querySelector("table")).toBeNull();
});

it("health_badges_from_application_health", async () => {
  health = () => ({ status: 200, body: { id: "7", placement: [
    { name: "nats-0", role: "nats", host_id: "1", host_name: "host-a", state: "up", expected: 1, healthy: 1, unknown: 0 },
    { name: "nats-1", role: "nats", host_id: "2", host_name: "host-b", state: "down", expected: 1, healthy: 0, unknown: 0 },
  ] } });
  await mountTopology(deployment({ current_revision: "11" }));
  expect(requests.some(r => r.url.endsWith("/workspaces/ops/application-health/7"))).toBe(true);
  expect(stateOf("nats-0")).toBe("up");
  expect(stateOf("nats-1")).toBe("down");
  // An instance the health report does not mention is unknown, never healthy.
  expect(stateOf("nats-2")).toBe("unknown");
  expect(stateOf("web-0")).toBe("unknown");
  expect(rowsOf("nats")[0]!.textContent).toContain(t("state.up"));
  expect(rowsOf("nats")[1]!.textContent).toContain(t("state.down"));
  expect(roleGroups()[0]!.textContent).toContain(t("summary", { up: 1, total: 3 }));
});

it("stateless swarm role is one replicated row on the manager", async () => {
  health = () => ({ status: 200, body: { id: "7", placement: [{ name: "web", role: "web", host_id: "1", host_name: "host-a", state: "down", expected: 2, healthy: 1, unknown: 0 }] } });
  await mountTopology(deployment({
    mode: "stack", manifest: JSON.stringify(blueprint("stack")), current_revision: "11",
    blueprint_binding: { driver: "stack", manager: "1", roles: { nats: [bind("1"), bind("2"), bind("3")], web: [bind("2"), bind("3")] } },
  }));
  expect(rowsOf("web")).toHaveLength(1);
  expect(rowsOf("web")[0]!.getAttribute("data-topology-instance")).toBe("web");
  expect(rowsOf("web")[0]!.textContent).toContain(t("replicated", { count: 2 }));
  expect(rowsOf("web")[0]!.textContent).toContain("host-a");
  expect(stateOf("web")).toBe("down");
  // The summary counts Swarm replicas from the health entry, not rows.
  expect(roleGroups()[1]!.querySelector("header")!.textContent).toContain(t("summary", { up: 1, total: 2 }));
  // Stable roles keep one row per pinned instance.
  expect(rowsOf("nats")).toHaveLength(3);
});

it("loading, error, empty and no-binding states", async () => {
  let release!: () => void;
  health = () => new Promise(resolve => { release = () => resolve({ status: 500, body: { error: "health store offline" } }); });
  await mountTopology(deployment({ current_revision: "11" }));
  expect(host.querySelector('[role="status"]')?.textContent).toContain(t("healthLoading"));
  expect(stateOf("nats-0")).toBe("loading");
  await act(async () => release()); await flush();
  expect(alertWith("health store offline")).toBeDefined();
  expect(stateOf("nats-0")).toBe("unknown");
  health = () => ({ status: 200, body: { id: "7", placement: [{ name: "nats-0", role: "nats", host_id: "1", state: "up" }] } });
  await click(commonText("common-domain:common.retry"));
  expect(alertWith("health store offline")).toBeUndefined();
  expect(stateOf("nats-0")).toBe("up");

  await act(async () => root.unmount()); root = createRoot(host);
  await mountTopology(deployment({ blueprint_binding: null }));
  expect(host.textContent).toContain(t("noBinding"));
  expect(roleGroups()).toHaveLength(0);

  await act(async () => root.unmount()); root = createRoot(host);
  await mountTopology(deployment({ blueprint_binding: { driver: "docker", roles: {} } }));
  expect(host.textContent).toContain(t("empty"));
});

it("health without alerts permission is not requested and reads unknown", async () => {
  await mountTopology(deployment({ current_revision: "11" }), new Set(["deployments.read", "hosts.read"]));
  expect(requests.some(r => r.url.includes("application-health"))).toBe(false);
  expect(stateOf("nats-0")).toBe("unknown");
  expect(host.textContent).toContain(t("healthUnavailable"));
});

it("many instances stay in a bounded scroll region", async () => {
  const many = Array.from({ length: 100 }, (_, index) => bind(String((index % 3) + 1)));
  await mountTopology(deployment({ blueprint_binding: { driver: "docker", roles: { nats: many } } }));
  expect(rowsOf("nats")).toHaveLength(100);
  const region = host.querySelector<HTMLElement>(`[role="region"][aria-label="${t("instancesOf", { role: "nats" })}"]`)!;
  expect(region).not.toBeNull();
  expect(region.className).toContain("overflow-y-auto");
  expect(rowsOf("nats")[99]!.getAttribute("data-topology-instance")).toBe("nats-99");
});

const yaml = () => host.querySelector<HTMLTextAreaElement>(`textarea[aria-label="${w("yamlLabel")}"]`)!;
const typeYaml = async (text: string) => {
  const area = yaml();
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(area, text);
    area.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await flush();
};
const mountEdit = async (app: Deployment, onSaved = vi.fn()) => {
  await act(async () => root.render(<BlueprintSpecEdit workspace="ops" deployment={app} permissions={all} navigation={{ navigate: vi.fn(), href: () => "#" }} onSaved={onSaved} />));
  await flush();
  return onSaved;
};
const steps = () => [...host.querySelectorAll(`[aria-label="${w("steps")}"] li`)];

it("failover_editable_after_deploy_without_unlocking_topology", async () => {
  const onSaved = await mountEdit(deployment({ current_revision: "11" }));
  await click(w("editSpec"));
  const select = host.querySelector<HTMLSelectElement>(`select[aria-label="${w("failoverMode")}"]`)!;
  expect(select.disabled).toBe(false);
  await act(async () => { select.value = "alert"; select.dispatchEvent(new Event("change", { bubbles: true })); }); await flush();
  expect(alertWith(w("topologyLocked"))).toBeUndefined();
  await click(w("save"));
  const saved = requests.find(r => r.method === "PATCH")!;
  expect(parse(String(saved.body!.deploy_spec)).failover).toEqual({ mode: "alert" });
  expect(onSaved).toHaveBeenCalledTimes(1);
});

it("spec_edit_limited_to_values_after_deploy", async () => {
  const onSaved = await mountEdit(deployment({ current_revision: "11" }));
  await click(w("editSpec"));
  // Only the parameters step: roles and placement are frozen after the first deployment.
  expect(steps()).toHaveLength(1);
  expect(steps()[0]!.textContent).toContain(w("stepValues"));
  expect(host.textContent).toContain(w("frozenHelp"));
  await click(w("yaml"));
  const stored = yaml().value;
  // A topology change is caught before submit and blocks it.
  await typeYaml(stored.replace("instances: 3", "instances: 5"));
  expect(alertWith(w("topologyLocked"))).toBeDefined();
  expect(button(w("save")).disabled).toBe(true);
  await typeYaml(stored.replace("driver: docker", "driver: compose"));
  expect(alertWith(w("topologyLocked"))).toBeDefined();
  expect(button(w("save")).disabled).toBe(true);
  // Values and the template version may change.
  await typeYaml(stored.replace("client_port: 4222", "client_port: 4333").replace("version: 3", "version: 4"));
  expect(alertWith(w("topologyLocked"))).toBeUndefined();
  expect(button(w("save")).disabled).toBe(false);
  // The server's localized rejection is shown and the editor stays open.
  // The server rejects a topology change with 400 `blueprint.topologyChangeRequiresPlan`.
  patchReply = () => ({ status: 400, body: { error: "首次部署后不能再修改蓝图绑定" } });
  await click(w("save"));
  expect(alertWith("首次部署后不能再修改蓝图绑定")).toBeDefined();
  expect(onSaved).not.toHaveBeenCalled();
  patchReply = () => ({ status: 200, body: {} });
  await click(w("save"));
  const sent = requests.filter(r => r.method === "PATCH");
  expect(sent).toHaveLength(2);
  expect(sent[1]!.url).toBe("/api/workspaces/ops/deployments/7");
  expect(sent[1]!.body).toMatchObject({ name: "nats-prod", workdir: "/opt/deployments", version: 4 });
  expect((parse(String(sent[1]!.body!.deploy_spec)) as typeof spec).values.client_port).toBe(4333);
  expect(onSaved).toHaveBeenCalledTimes(1);
  expect(requests.some(r => r.url.includes("deployment-placements"))).toBe(false);
});

it("before the first deploy the whole wizard edits the stored spec", async () => {
  const onSaved = await mountEdit(deployment({ deploy_spec: stringify({ ...spec, roles: { nats: { instances: 5 }, web: { instances: 1 } } }) }));
  await click(w("editSpec"));
  expect(steps()).toHaveLength(5);
  await click(w("next")); await click(w("next"));
  // Prefilled from the stored spec, not the template defaults.
  expect(host.querySelector<HTMLOutputElement>(`output[aria-label="${w("instances")}"]`)!.textContent).toBe("5");
  await click(w("next")); await click(w("next"));
  // The working directory cannot change on update.
  expect(host.querySelector<HTMLInputElement>('input[name="workdir"]')!.disabled).toBe(true);
  expect(host.querySelector<HTMLInputElement>('input[name="name"]')!.value).toBe("nats-prod");
  await click(w("save"));
  const sent = requests.filter(r => r.method === "PATCH");
  expect(sent).toHaveLength(1);
  expect((parse(String(sent[0]!.body!.deploy_spec)) as typeof spec).roles.nats.instances).toBe(5);
  expect(onSaved).toHaveBeenCalledTimes(1);
});

it("cancel closes the spec editor without saving", async () => {
  await mountEdit(deployment());
  await click(w("editSpec"));
  expect(steps()).toHaveLength(5);
  await click(w("cancel"));
  expect(steps()).toHaveLength(0);
  expect(requests.some(r => r.method === "PATCH")).toBe(false);
});

it("the embedded editor offers cancel while loading and after a load error", async () => {
  let release!: () => void;
  templateReply = () => new Promise(resolve => { release = () => resolve({ status: 500, body: { error: "template store offline" } }); });
  await mountEdit(deployment());
  await click(w("editSpec"));
  expect(button(w("cancel"))).toBeDefined();
  await act(async () => release()); await flush();
  expect(alertWith("template store offline")).toBeDefined();
  await click(w("cancel"));
  expect(button(w("editSpec"))).toBeDefined();
  // A template that is no longer a blueprint is a dead end too, with a way back.
  templateReply = () => ({ status: 200, body: { id: "42", name: "plain", version: 3, content: "services: {}", parameters: "[]" } });
  await click(w("editSpec"));
  expect(alertWith(w("notBlueprint"))).toBeDefined();
  await click(w("cancel"));
  expect(button(w("editSpec"))).toBeDefined();
});

it("labels use the locale's separator", async () => {
  await i18n.changeLanguage("zh-CN");
  await mountTopology(deployment());
  expect(rowsOf("nats")[0]!.textContent).toContain(`${w("host")}：`);
  expect(roleGroups()[0]!.getAttribute("aria-label")).toBe(`${w("role")}：nats`);
});

it("a refresh skipped while hidden runs when the page becomes visible", async () => {
  vi.useFakeTimers();
  try {
    await mountTopology(deployment({ current_revision: "11" }));
    const count = () => requests.filter(r => r.url.includes("application-health")).length;
    expect(count()).toBe(1);
    Object.defineProperty(document, "hidden", { configurable: true, value: true });
    await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
    expect(count()).toBe(1);
    Object.defineProperty(document, "hidden", { configurable: true, value: false });
    await act(async () => { document.dispatchEvent(new Event("visibilitychange")); });
    await flush();
    expect(count()).toBe(2);
  } finally { vi.useRealTimers(); Object.defineProperty(document, "hidden", { configurable: true, value: false }); }
});

it("blueprint_target_names_driver_distinct_hosts_and_stack_manager", async () => {
  // Three distinct hosts across both roles (host 3 runs nats-2 and web-0).
  await act(async () => root.render(<BlueprintTarget workspace="ops" deployment={deployment()} permissions={all} />));
  await flush();
  expect(host.textContent).toBe(`${w("driverDocker")} · ${t("hostCount", { count: 3 })}`);
  expect(requests.some(r => r.url.includes("/nodes"))).toBe(false);
  const stack = deployment({ mode: "stack", blueprint_binding: { driver: "stack", manager: "2", roles: { nats: [bind("1"), bind("2")], web: [bind("1")] } } });
  await act(async () => root.render(<BlueprintTarget workspace="ops" deployment={stack} permissions={all} />));
  await flush();
  expect(host.textContent).toBe(`${w("driverStack")} · ${t("hostCount", { count: 2 })} · ${t("manager", { host: "host-b" })}`);
  await act(async () => root.render(<BlueprintTarget workspace="ops" deployment={deployment({ blueprint_binding: null })} permissions={all} />));
  expect(host.textContent).toBe("—");
});
