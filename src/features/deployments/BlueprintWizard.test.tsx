import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { parse } from "yaml";
import { commonText, i18n } from "../../shared/i18n/i18n";
import type { Navigation } from "../../shared/routing/types";
import { marketplaceLists, type MarketEntry } from "../marketplace/resource-config";
import { templatesLists } from "../templates/resource-config";
import { BlueprintWizard } from "./BlueprintWizard";

vi.mock("../../shared/editor/CodeEditor", () => ({ CodeEditor: ({ value, label, onChange }: { value: string; label: string; onChange: (value: string) => void }) => <textarea aria-label={label} value={value} onChange={event => onChange(event.target.value)} /> }));

const w = (key: string, values?: Record<string, unknown>) => commonText(`resource-domain:wizard.${key}`, values);
const blueprint = {
  apiVersion: "ferry/blueprint-v1", kind: "Blueprint", metadata: { name: "nats-cluster" },
  roles: [{ name: "nats", instances: { min: 3, max: 9, default: 3, odd: true }, placement: { distinctHosts: true }, identity: "stable", service: { image: "nats" } }],
  drivers: { docker: { supported: true }, stack: { supported: true }, native: { supported: false } },
};
let distinct = true;
const template = () => ({ id: "42", name: "NATS cluster", description: "", version: 3, content: JSON.stringify({ ...blueprint, roles: blueprint.roles.map(role => ({ ...role, placement: { distinctHosts: distinct } })) }, null, 2), parameters: JSON.stringify([{ name: "client_port", type: "integer", default: 4222, min: 1, max: 65535 }]) });
const hosts = [{ id: "1", name: "host-a", address: "10.0.0.1" }, { id: "2", name: "host-b", address: "10.0.0.2" }, { id: "3", name: "host-c", address: "10.0.0.3" }];
const placed = (roles: { hostId: string; address: string }[]) => ({ manifest: "{}", binding: { driver: "docker", roles: { nats: roles } }, warnings: [{ key: "blueprint.placementSpreadUnmet", params: {}, message: "Spread unmet for nats" }] });
let root: Root, host: HTMLDivElement, requests: { url: string; method: string; body?: Record<string, unknown> }[], navigate: Navigation["navigate"] & ReturnType<typeof vi.fn>;
let placementReply: (spec: string) => { status: number; body: unknown } | Promise<{ status: number; body: unknown }>;

beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  history.replaceState({}, "", "/en-US/workspaces/ops/deployments/new");
  await i18n.changeLanguage("en-US");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  distinct = true;
  requests = []; navigate = vi.fn() as typeof navigate;
  placementReply = () => ({ status: 200, body: placed([{ hostId: "1", address: "10.0.0.1" }, { hostId: "2", address: "10.0.0.2" }, { hostId: "3", address: "10.0.0.3" }]) });
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    const method = init.method ?? "GET";
    const body = init.body ? JSON.parse(String(init.body)) as Record<string, unknown> : undefined;
    requests.push({ url, method, ...(body ? { body } : {}) });
    const reply = (status: number, data: unknown) => ({ ok: status < 400, status, headers: new Headers({ "content-type": "application/json" }), json: async () => data });
    if (url.includes("/app-templates/42")) return reply(200, template());
    if (url.includes("/nodes")) return reply(200, { items: hosts, total: 3, page: 1, page_size: 200 });
    if (url.includes("/tags")) return reply(200, { items: [{ id: "t1", name: "cache", color: "" }], total: 1, page: 1, page_size: 200 });
    if (url.includes("/node-groups")) return reply(200, { items: [], total: 0, page: 1, page_size: 200 });
    if (url.includes("/deployment-placements")) { const r = await placementReply(String(body?.spec)); return reply(r.status, r.body); }
    if (url.endsWith("/deployments") && method === "POST") return reply(201, { id: "99" });
    if (url.includes("/marketplace/m1/installations")) return reply(201, { id: "42", version: 1 });
    return reply(404, { error: "missing" });
  }));
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

const flush = async () => { for (let i = 0; i < 5; i++) await act(async () => { await Promise.resolve(); }); };
const mount = async () => {
  await act(async () => root.render(<BlueprintWizard workspace="ops" templateId="42" version={3} permissions={new Set(["deployments.create", "app-templates.read", "hosts.read"])} navigation={{ navigate, href: () => "#" }} />));
  await flush();
};
const button = (name: string) => [...host.querySelectorAll("button")].find(b => b.textContent === name || b.getAttribute("aria-label") === name)!;
const click = async (name: string) => { const target = button(name); expect(target, name).toBeDefined(); await act(async () => target.click()); await flush(); };
const yaml = () => host.querySelector<HTMLTextAreaElement>(`textarea[aria-label="${w("yamlLabel")}"]`)!;
const typeYaml = async (text: string) => {
  const area = yaml();
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(area, text);
    area.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await flush();
};
const specOf = async () => { await click(w("yaml")); const text = yaml().value; await click(w("form")); return text; };
const alertWith = (text: string) => [...host.querySelectorAll('[role="alert"]')].find(alert => alert.textContent?.includes(text));
const chooseHost = async (instance: string, hostId: string) => {
  const select = host.querySelector<HTMLSelectElement>(`select[aria-label="${w("hostOf", { instance })}"]`)!;
  await act(async () => { select.value = hostId; select.dispatchEvent(new Event("change", { bubbles: true })); });
  await flush();
};
const restore = () => [...host.querySelectorAll("button")].find(b => b.textContent?.startsWith(w("autoPlacement")));
const placementSpecs = () => requests.filter(r => r.url.includes("deployment-placements")).map(r => parse(String(r.body?.spec)) as Record<string, unknown>);

it("instances_stepper_respects_odd_and_bounds", async () => {
  await mount();
  expect(host.textContent).toContain("NATS cluster");
  await click(w("next")); await click(w("next"));
  const count = () => host.querySelector<HTMLOutputElement>(`output[aria-label="${w("instances")}"]`)!.textContent;
  expect(count()).toBe("3");
  expect(host.textContent).toContain(w("instancesHintOdd", { min: 3, max: 9 }));
  expect(button(w("decrease", { role: "nats" })).disabled).toBe(true);
  await click(w("increase", { role: "nats" }));
  expect(count()).toBe("5");
  for (let i = 0; i < 5; i++) await click(w("increase", { role: "nats" }));
  expect(count()).toBe("9");
  expect(button(w("increase", { role: "nats" })).disabled).toBe(true);
  expect((parse(await specOf()) as { roles: { nats: { instances: number } } }).roles.nats.instances).toBe(9);
});

it("yaml_errors_block_submit_and_keep_form", async () => {
  await mount();
  await click(w("next")); await click(w("next"));
  await click(w("increase", { role: "nats" }));
  await click(w("yaml"));
  const valid = yaml().value;
  await typeYaml(`${valid}failover: {}\n`);
  expect(alertWith("failover")).toBeDefined();
  expect(button(w("next")).disabled).toBe(true);
  await typeYaml("roles: [");
  expect(button(w("next")).disabled).toBe(true);
  await click(w("form"));
  // The form shows the last valid spec, locked, and explains why the wizard is blocked.
  expect(host.querySelector<HTMLOutputElement>(`output[aria-label="${w("instances")}"]`)!.textContent).toBe("5");
  expect(alertWith(w("yamlInvalid"))).toBeDefined();
  expect(button(w("next")).disabled).toBe(true);
  expect(button(w("increase", { role: "nats" })).disabled).toBe(true);
  expect([...host.querySelectorAll<HTMLSelectElement>("select")].every(select => select.disabled)).toBe(true);
  await act(async () => button(w("increase", { role: "nats" })).click());
  // Returning to YAML shows the user's text unchanged: the locked form never overwrote it.
  await click(w("yaml"));
  expect(yaml().value).toBe("roles: [");
  // Fixing the YAML unblocks the wizard.
  await typeYaml(valid);
  expect(button(w("next")).disabled).toBe(false);
  // Discarding restores the last valid spec and unlocks the form.
  await typeYaml(`${valid}failover: {}\n`);
  await click(w("form"));
  await click(w("discardYaml"));
  expect(alertWith(w("yamlInvalid"))).toBeUndefined();
  expect(button(w("increase", { role: "nats" })).disabled).toBe(false);
  expect(button(w("next")).disabled).toBe(false);
  await click(w("yaml"));
  expect(yaml().value).toBe(valid);
  expect(requests.some(r => r.url.includes("deployment-placements") || (r.url.endsWith("/deployments") && r.method === "POST"))).toBe(false);
});

it("placement_preview_renders_rows_and_warnings", async () => {
  await mount();
  for (let i = 0; i < 3; i++) await click(w("next"));
  expect(placementSpecs()).toHaveLength(1);
  const table = host.querySelector(`[role="region"][aria-label="${w("placementTitle")}"]`)!;
  const rows = [...table.querySelectorAll("tbody tr")].map(row => row.textContent);
  expect(rows).toHaveLength(3);
  expect(rows[0]).toContain("nats-0"); expect(rows[0]).toContain("host-a"); expect(rows[0]).toContain("10.0.0.1");
  expect(rows[2]).toContain("host-c"); expect(rows[2]).toContain("10.0.0.3");
  expect(host.textContent).toContain("Spread unmet for nats");
  // Choosing another host writes the role's whole list into spec.placement and recalculates.
  // nats has distinctHosts, so the instance already on host-c takes over host-b: a swap.
  placementReply = () => ({ status: 200, body: placed([{ hostId: "1", address: "10.0.0.1" }, { hostId: "3", address: "10.0.0.3" }, { hostId: "2", address: "10.0.0.2" }]) });
  await chooseHost("nats-1", "3");
  expect(placementSpecs().at(-1)!.placement).toEqual({ nats: [{ hostId: "1", address: "10.0.0.1" }, { hostId: "3", address: "10.0.0.3" }, { hostId: "2", address: "10.0.0.2" }] });
  // Server errors are already localized; they replace the table and block the next step.
  placementReply = () => ({ status: 400, body: { error: "nats needs 3 hosts with tags [cache]; 2 matched: host-a, host-b" } });
  await click(w("refreshPlacement"));
  expect(alertWith("nats needs 3 hosts")).toBeDefined();
  expect(button(w("next")).disabled).toBe(true);
  // A manual placement can be dropped even while the preview fails.
  placementReply = () => ({ status: 200, body: placed([{ hostId: "1", address: "10.0.0.1" }, { hostId: "2", address: "10.0.0.2" }, { hostId: "3", address: "10.0.0.3" }]) });
  await act(async () => restore()!.click()); await flush();
  expect(placementSpecs().at(-1)!).not.toHaveProperty("placement");
  expect(restore()).toBeUndefined();
  expect(button(w("next")).disabled).toBe(false);
});

it("phone_placement_renders_cards_with_select_and_readonly_address", async () => {
  vi.stubGlobal("matchMedia", (query: string) => ({ matches: query === "(max-width: 767px)", media: query, addEventListener() {}, removeEventListener() {} }));
  await mount();
  for (let i = 0; i < 3; i++) await click(w("next"));
  const region = host.querySelector(`[role="region"][aria-label="${w("placementTitle")}"]`)!;
  expect(region.querySelector("table")).toBeNull();
  const cards = [...region.querySelectorAll("[data-placement-instance]")];
  expect(cards.map(card => card.getAttribute("data-placement-instance"))).toEqual(["nats-0", "nats-1", "nats-2"]);
  expect(cards[1]!.querySelector<HTMLSelectElement>(`select[aria-label="${w("hostOf", { instance: "nats-1" })}"]`)!.value).toBe("2");
  expect(cards[1]!.textContent).toContain("10.0.0.2");
  expect(cards[1]!.querySelector("input, textarea")).toBeNull();
  placementReply = () => ({ status: 200, body: placed([{ hostId: "1", address: "10.0.0.1" }, { hostId: "3", address: "10.0.0.3" }, { hostId: "2", address: "10.0.0.2" }]) });
  await chooseHost("nats-1", "3");
  expect(placementSpecs().at(-1)!.placement).toEqual({ nats: [{ hostId: "1", address: "10.0.0.1" }, { hostId: "3", address: "10.0.0.3" }, { hostId: "2", address: "10.0.0.2" }] });
  const updated = [...host.querySelectorAll("[data-placement-instance]")];
  expect(updated[0]!.textContent).toContain(w("manual"));
  expect(updated[1]!.textContent).toContain("10.0.0.3");
});

it("roles without distinctHosts may share a host", async () => {
  distinct = false;
  await mount();
  for (let i = 0; i < 3; i++) await click(w("next"));
  await chooseHost("nats-1", "3");
  expect(placementSpecs().at(-1)!.placement).toEqual({ nats: [{ hostId: "1", address: "10.0.0.1" }, { hostId: "3", address: "10.0.0.3" }, { hostId: "3", address: "10.0.0.3" }] });
});

it("changing instances or selectors drops the role's stale manual placement", async () => {
  await mount();
  for (let i = 0; i < 3; i++) await click(w("next"));
  await chooseHost("nats-1", "3");
  expect(placementSpecs().at(-1)!.placement).toBeDefined();
  await click(w("back"));
  await click(w("increase", { role: "nats" }));
  let spec = parse(await specOf()) as Record<string, unknown>;
  expect(spec).not.toHaveProperty("placement");
  expect(spec.roles).toEqual({ nats: { instances: 5 } });
  await click(w("next"));
  expect(placementSpecs().at(-1)!).not.toHaveProperty("placement");
  expect((placementSpecs().at(-1)!.roles as { nats: { instances: number } }).nats.instances).toBe(5);
  // The same holds for the role's host selector.
  await chooseHost("nats-1", "3");
  await click(w("back"));
  const tags = host.querySelector<HTMLSelectElement>('select[name="nats-tags"]')!;
  await act(async () => { tags.options[0]!.selected = true; tags.dispatchEvent(new Event("change", { bubbles: true })); });
  await flush();
  spec = parse(await specOf()) as Record<string, unknown>;
  expect(spec).not.toHaveProperty("placement");
  expect(spec.roles).toEqual({ nats: { instances: 5, hosts: { tags: ["cache"] } } });
});

it("shows the latest placement when an earlier reply arrives last", async () => {
  let release!: (value: { status: number; body: unknown }) => void;
  placementReply = () => new Promise(resolve => { release = resolve; });
  await mount();
  for (let i = 0; i < 3; i++) await click(w("next"));
  const first = release;
  // While the first request is pending, a YAML edit changes the spec and the form recalculates.
  placementReply = () => ({ status: 200, body: placed([{ hostId: "1", address: "fresh-0" }, { hostId: "2", address: "fresh-1" }, { hostId: "3", address: "fresh-2" }]) });
  await click(w("yaml"));
  await typeYaml(yaml().value.replace("client_port: 4222", "client_port: 5222"));
  await click(w("form"));
  expect(placementSpecs()).toHaveLength(2);
  await act(async () => first({ status: 200, body: placed([{ hostId: "1", address: "stale-0" }, { hostId: "2", address: "stale-1" }, { hostId: "3", address: "stale-2" }]) }));
  await flush();
  expect(host.textContent).toContain("fresh-0");
  expect(host.textContent).not.toContain("stale-0");
  expect(button(w("next")).disabled).toBe(false);
});

it("offers the Swarm manager only for stack and drops it when the driver changes", async () => {
  await mount();
  const driver = () => host.querySelector<HTMLSelectElement>("section select")!;
  expect([...driver().options].map(option => option.textContent)).toEqual([w("driverDocker"), w("driverStack")]);
  const pick = async (select: HTMLSelectElement, value: string) => { await act(async () => { select.value = value; select.dispatchEvent(new Event("change", { bubbles: true })); }); await flush(); };
  await click(w("next")); await click(w("next"));
  expect(host.textContent).not.toContain(w("manager"));
  await click(w("back")); await click(w("back"));
  await pick(driver(), "stack");
  await click(w("next")); await click(w("next"));
  expect(host.textContent).toContain(w("manager"));
  expect(button(w("next")).disabled).toBe(true);
  await pick(host.querySelector<HTMLSelectElement>("section select")!, "host-a");
  expect(button(w("next")).disabled).toBe(false);
  expect(parse(await specOf())).toMatchObject({ driver: "stack", manager: "host-a" });
  await click(w("back")); await click(w("back"));
  await pick(driver(), "docker");
  const spec = parse(await specOf()) as Record<string, unknown>;
  expect(spec.driver).toBe("docker");
  expect(spec).not.toHaveProperty("manager");
});

it("submits_deploy_spec", async () => {
  await mount();
  for (let i = 0; i < 4; i++) await click(w("next"));
  const name = host.querySelector<HTMLInputElement>('input[name="name"]')!;
  expect(name.value).toBe("nats-cluster");
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(name, "bus");
    name.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const text = await specOf();
  await click(w("create"));
  const created = requests.find(r => r.url.endsWith("/workspaces/ops/deployments") && r.method === "POST")!;
  expect(created.body).toEqual({ name: "bus", category: "", description: "", workdir: "/opt/deployments", deploy_spec: text });
  expect(parse(text)).toEqual({ apiVersion: "ferry/deploy-v1", template: { id: "42", version: 3 }, driver: "docker", values: { client_port: 4222 }, roles: { nats: { instances: 3 } } });
  expect(navigate).toHaveBeenCalledWith({ group: "deploy", item: "apps", id: "99", mode: "view" }, {});
});

it("app-template list sends blueprint templates to the wizard", () => {
  const use = templatesLists({ workspace: "ops", navigate: vi.fn() })["app-templates"]!.actions!.find(a => a.labelKey === "resource-domain:template.use")!;
  const cluster = template();
  const app = { ...cluster, id: "7", version: 2, content: "apiVersion: ferry/v1\nkind: Application\n" };
  expect(use.to!(cluster)).toEqual({ group: "deploy", item: "apps", mode: "new" });
  expect(use.query!(cluster)).toEqual({ wizard: "blueprint", template: "42", version: "3" });
  expect(use.to!(app)).toEqual({ group: "deploy", item: "apps", mode: "new" });
  expect(use.query!(app)).toEqual({ template: "7" });
});

it("market_blueprint_entry_opens_wizard", async () => {
  const marketNavigate = vi.fn();
  const actions = marketplaceLists({ workspace: "ops", navigate: marketNavigate }).marketplace!.actions!;
  const deploy = actions.find(a => a.labelKey === "resource-domain:wizard.deploy")!;
  expect(deploy).toBeDefined();
  const entry = { id: "m1", version: 7, compatible: true, content: JSON.stringify(blueprint) } as unknown as MarketEntry;
  const app = { ...entry, content: JSON.stringify({ apiVersion: "ferry/v1", kind: "Application" }) } as MarketEntry;
  expect(deploy.when!(entry)).toBe(true);
  expect(deploy.when!(app)).toBe(false);
  // List rows have no content; the package's declared topology decides.
  expect(deploy.when!({ id: "m2", meta: { topology: "cluster" } } as unknown as MarketEntry)).toBe(true);
  expect(deploy.when!({ id: "m3", meta: { topology: "single" } } as unknown as MarketEntry)).toBe(false);
  // The existing import ("use template") action stays available to every entry.
  expect(actions.find(a => a.labelKey === "resource-domain:template.importApp")?.when?.(app) ?? true).toBe(true);
  await deploy.action!(entry);
  expect(requests.find(r => r.url.includes("/marketplace/m1/installations"))?.body).toEqual({ version: 7 });
  expect(marketNavigate).toHaveBeenCalledWith({ group: "deploy", item: "apps", mode: "new" }, { wizard: "blueprint", template: "42", version: "1" });
});
