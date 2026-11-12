import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { parse } from "yaml";
import { commonText, i18n } from "../../shared/i18n/i18n";
import type { Navigation } from "../../shared/routing/types";
import { marketplaceLists, type MarketEntry } from "../marketplace/resource-config";
import { BlueprintWizard } from "./BlueprintWizard";

vi.mock("../../shared/editor/CodeEditor", () => ({ CodeEditor: ({ value, label, onChange }: { value: string; label: string; onChange: (value: string) => void }) => <textarea aria-label={label} value={value} onChange={event => onChange(event.target.value)} /> }));

const w = (key: string, values?: Record<string, unknown>) => commonText(`resource-domain:wizard.${key}`, values);
const blueprint = {
  apiVersion: "ferry/blueprint-v1", kind: "Blueprint", metadata: { name: "nats-cluster" },
  roles: [{ name: "nats", instances: { min: 3, max: 9, default: 3, odd: true }, placement: { distinctHosts: true }, identity: "stable", service: { image: "nats" } }],
  drivers: { docker: { supported: true }, stack: { supported: true }, native: { supported: false } },
};
const template = { id: "42", name: "NATS cluster", description: "", version: 3, content: JSON.stringify(blueprint, null, 2), parameters: JSON.stringify([{ name: "client_port", type: "integer", default: 4222, min: 1, max: 65535 }]) };
const hosts = [{ id: "1", name: "host-a", address: "10.0.0.1" }, { id: "2", name: "host-b", address: "10.0.0.2" }, { id: "3", name: "host-c", address: "10.0.0.3" }];
const placed = (roles: { hostId: string; address: string }[]) => ({ manifest: "{}", binding: { driver: "docker", roles: { nats: roles } }, warnings: [{ key: "blueprint.placementSpreadUnmet", params: {}, message: "Spread unmet for nats" }] });
let root: Root, host: HTMLDivElement, requests: { url: string; method: string; body?: Record<string, unknown> }[], navigate: Navigation["navigate"] & ReturnType<typeof vi.fn>;
let placementReply: (spec: string) => { status: number; body: unknown };

beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  history.replaceState({}, "", "/en-US/workspaces/ops/deployments/new");
  await i18n.changeLanguage("en-US");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  requests = []; navigate = vi.fn() as typeof navigate;
  placementReply = () => ({ status: 200, body: placed([{ hostId: "1", address: "10.0.0.1" }, { hostId: "2", address: "10.0.0.2" }, { hostId: "3", address: "10.0.0.3" }]) });
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    const method = init.method ?? "GET";
    const body = init.body ? JSON.parse(String(init.body)) as Record<string, unknown> : undefined;
    requests.push({ url, method, ...(body ? { body } : {}) });
    const reply = (status: number, data: unknown) => ({ ok: status < 400, status, headers: new Headers({ "content-type": "application/json" }), json: async () => data });
    if (url.includes("/app-templates/42")) return reply(200, template);
    if (url.includes("/nodes")) return reply(200, { items: hosts, total: 3, page: 1, page_size: 200 });
    if (url.includes("/tags")) return reply(200, { items: [{ id: "t1", name: "cache", color: "" }], total: 1, page: 1, page_size: 200 });
    if (url.includes("/node-groups")) return reply(200, { items: [], total: 0, page: 1, page_size: 200 });
    if (url.includes("/deployment-placements")) { const r = placementReply(String(body?.spec)); return reply(r.status, r.body); }
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
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("failover");
  expect(button(w("next")).disabled).toBe(true);
  await typeYaml("roles: [");
  expect(button(w("next")).disabled).toBe(true);
  await click(w("form"));
  // The form keeps the last valid spec and still explains why the wizard is blocked.
  expect(host.querySelector<HTMLOutputElement>(`output[aria-label="${w("instances")}"]`)!.textContent).toBe("5");
  expect(host.querySelector('[role="alert"]')?.textContent).toContain(w("yamlInvalid"));
  expect(button(w("next")).disabled).toBe(true);
  // Returning to YAML shows the user's text unchanged; fixing it unblocks the wizard.
  await click(w("yaml"));
  expect(yaml().value).toBe("roles: [");
  await typeYaml(valid);
  expect(button(w("next")).disabled).toBe(false);
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
  placementReply = () => ({ status: 200, body: placed([{ hostId: "1", address: "10.0.0.1" }, { hostId: "3", address: "10.0.0.3" }, { hostId: "2", address: "10.0.0.2" }]) });
  const select = host.querySelector<HTMLSelectElement>(`select[aria-label="${w("hostOf", { instance: "nats-1" })}"]`)!;
  await act(async () => { select.value = "3"; select.dispatchEvent(new Event("change", { bubbles: true })); });
  await flush();
  const last = placementSpecs().at(-1)!;
  expect(last.placement).toEqual({ nats: [{ hostId: "1", address: "10.0.0.1" }, { hostId: "3", address: "10.0.0.3" }, { hostId: "3", address: "10.0.0.3" }] });
  // Server errors are already localized; they replace the table and block the next step.
  placementReply = () => ({ status: 400, body: { error: "nats needs 3 hosts with tags [cache]; 2 matched: host-a, host-b" } });
  await click(w("refreshPlacement"));
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("nats needs 3 hosts");
  expect(button(w("next")).disabled).toBe(true);
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
