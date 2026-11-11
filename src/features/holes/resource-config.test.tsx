import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { api } from "../../shared/api/api";
import { i18n } from "../../shared/i18n/i18n";
import { setLocale } from "../../shared/i18n/locale";
import messages from "../../shared/i18n/locales/holes.json";
import { label } from "../../shared/resources/config";
import { nav } from "../../app/nav";
import { href, parse } from "../../app/router";
import { HOLE_STATES, holesForms, holesLists, PROBE_STATES, REVIEW_STATES, type Hole } from "./resource-config";

vi.mock("../../shared/api/api", async (original) => ({
  ...(await original<typeof import("../../shared/api/api")>()),
  api: vi.fn(async () => ({})),
}));

const context = { workspace: "ops", navigate: () => {} };
const listConfig = () => holesLists(context).holes!;
const formConfig = () => holesForms(context).holes!;
const fields = () => formConfig().sections.flatMap((section) => section.fields);
const action = (labelKey: string) => listConfig().actions!.find((candidate) => candidate.labelKey === labelKey)!;
const row = (patch: Partial<Hole> = {}): Hole => ({
  id: "7", host_id: "3", host_name: "build-01", host_online: true, name: "Web", target_port: 5173,
  strip_prefix: true, url: "/holes/abc/", state: "active", expires_at: 1, revoked_at: 0,
  probe_state: "unreachable", probe_error: "connection refused", probed_at: 0, active_connections: 2,
  created_at: 1, created_by: "1", created_by_name: "Ada", updated_at: 1, updated_by: "1", version: 4,
  source_kind: "manual", source_id: "0", source_ref: "", review_state: "none", reviewed_at: 0, reviewed_by: "0", ...patch,
});

beforeEach(async () => {
  history.replaceState({}, "", "/en-US/");
  setLocale("en-US", false);
  await i18n.changeLanguage("en-US");
});
let mounted: Root | undefined;
let container: HTMLDivElement;
afterEach(async () => { if (mounted) await act(async () => mounted?.unmount()); mounted = undefined; container?.remove(); vi.clearAllMocks(); vi.restoreAllMocks(); });
async function mount(node: ReactNode) { container = document.createElement("div"); document.body.append(container); mounted = createRoot(container); await act(async () => mounted!.render(node)); }

it("keeps holes out of the menu but routable under hosts for list, create, detail and edit", () => {
  const host = nav.find((group) => group.key === "host")!;
  expect(host.items.filter((item) => !item.hidden).map((item) => item.key)).not.toContain("holes");
  expect(host.items.find((item) => item.key === "holes")).toMatchObject({ resource: "holes", permission: "holes.read", hidden: true, parent: "hosts" });
  for (const suffix of ["", "/new", "/9", "/9/edit"]) {
    const path = `/workspaces/ops/holes${suffix}`;
    expect(parse(path, "")).toMatchObject({ group: "host", item: "holes" });
    expect(href(parse(path, ""), {})).toBe(`/en-US${path}`);
  }
});

it("lists the required columns with a 5-second default refresh", () => {
  const config = listConfig();
  expect(config.resource).toBe("holes");
  expect(config.defaultRefresh).toBe(5);
  expect(config.columns.map((column) => column.key)).toEqual([
    "name", "ingress_mode", "source_kind", "host_name", "target_port", "state", "probe_state", "active_connections", "expires_at", "review_state", "created_by_name",
  ]);
  expect(config.filters!.map((filter) => filter.key)).toEqual(["q", "host_id", "state"]);
  expect(config.emptyKey).toBe("holes:empty");
  expect(config.toolbar?.create).toMatchObject({ labelKey: "holes:create" });
});

it("gates row actions by permission and state", () => {
  const permissions = Object.fromEntries(listConfig().actions!.map((candidate) => [candidate.labelKey, candidate.permission]));
  expect(permissions).toEqual({
    "holes:copyLink": undefined, "holes:open": undefined, "holes:probe": "holes.update", "holes:revoke": "holes.update",
    "holes:approve": "workflows.run", "holes:reject": "workflows.run",
    "common-domain:common.view": "holes.read", "common-domain:common.edit": "holes.update", "common-domain:common.delete": "holes.delete",
  });
  const visible = (labelKey: string, state: Hole["state"]) => { const candidate = action(labelKey); return !candidate.when || candidate.when(row({ state })); };
  expect(["active", "expired", "revoked"].map((state) => visible("holes:probe", state as Hole["state"]))).toEqual([true, false, false]);
  expect(["active", "expired", "revoked"].map((state) => visible("holes:revoke", state as Hole["state"]))).toEqual([true, true, false]);
  expect(["active", "expired", "revoked"].map((state) => visible("common-domain:common.edit", state as Hole["state"]))).toEqual([true, true, false]);
  expect(["active", "expired", "revoked"].map((state) => visible("holes:copyLink", state as Hole["state"]))).toEqual([true, true, true]);
  expect(action("holes:revoke").destructive).toBe(true);
  expect(action("common-domain:common.delete").destructive).toBe(true);
  expect(action("holes:copyLink").destructive).toBeFalsy();
});

it("offers approve and reject only on waiting reviews, gated by the source's run permission", () => {
  const reviews = listConfig().actions!.filter((candidate) => ["holes:approve", "holes:reject"].includes(candidate.labelKey!));
  expect(reviews.map((candidate) => [candidate.labelKey, candidate.permission])).toEqual([
    ["holes:approve", "pipelines.run"], ["holes:reject", "pipelines.run"], ["holes:approve", "workflows.run"], ["holes:reject", "workflows.run"],
  ]);
  const shown = (patch: Partial<Hole>, permissions: string[]) => reviews
    .filter((candidate) => permissions.includes(candidate.permission!) && candidate.when!(row(patch)))
    .map((candidate) => candidate.labelKey);
  expect(shown({ source_kind: "pipeline", review_state: "waiting" }, ["pipelines.run"])).toEqual(["holes:approve", "holes:reject"]);
  expect(shown({ source_kind: "pipeline", review_state: "waiting" }, ["workflows.run"])).toEqual([]);
  expect(shown({ source_kind: "workflow", review_state: "waiting" }, ["workflows.run"])).toEqual(["holes:approve", "holes:reject"]);
  expect(shown({ source_kind: "workflow", review_state: "waiting" }, ["pipelines.run"])).toEqual([]);
  for (const review_state of ["none", "approved", "rejected", "timed_out"] as const)
    expect(shown({ source_kind: "pipeline", review_state }, ["pipelines.run", "workflows.run"]), review_state).toEqual([]);
  expect(shown({ source_kind: "manual", review_state: "waiting" }, ["pipelines.run", "workflows.run"])).toEqual([]);
  expect(reviews.map((candidate) => !!candidate.destructive)).toEqual([false, true, false, true]);
});

it("posts the review decision to the reviews subresource", async () => {
  const [approve, reject] = listConfig().actions!.filter((candidate) => ["holes:approve", "holes:reject"].includes(candidate.labelKey!));
  await approve!.action!(row({ source_kind: "pipeline", review_state: "waiting" }));
  await reject!.action!(row({ source_kind: "pipeline", review_state: "waiting" }));
  expect(api).toHaveBeenNthCalledWith(1, "/workspaces/ops/holes/7/reviews", { method: "POST", body: JSON.stringify({ decision: "approved" }) });
  expect(api).toHaveBeenNthCalledWith(2, "/workspaces/ops/holes/7/reviews", { method: "POST", body: JSON.stringify({ decision: "rejected" }) });
});

it("hides editing and renewal for previews owned by a run or execution", () => {
  const edit = action("common-domain:common.edit");
  expect(edit.when!(row({ source_kind: "manual" }))).toBe(true);
  expect(edit.when!(row({ source_kind: "pipeline" }))).toBe(false);
  expect(edit.when!(row({ source_kind: "workflow" }))).toBe(false);
  expect(formConfig().readOnly!(row({ source_kind: "pipeline" }))).toBe(true);
  expect(formConfig().readOnly!(row({ source_kind: "workflow" }))).toBe(true);
  expect(formConfig().readOnly!(row({ source_kind: "manual" }))).toBe(false);
});

it("lets the host tab preset and lock the host through the create query", () => {
  expect(formConfig().queryDefaults).toEqual(["host_id"]);
});

it("copies and opens the absolute link built from the root-relative url", async () => {
  const writeText = vi.fn(async () => {});
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
  await action("holes:copyLink").action!(row());
  expect(writeText).toHaveBeenCalledWith(`${location.origin}/holes/abc/`);
  const open = vi.spyOn(window, "open").mockReturnValue(null);
  await action("holes:open").action!(row());
  expect(open).toHaveBeenCalledWith(`${location.origin}/holes/abc/`, "_blank", "noopener,noreferrer");
});

it("posts revocations and probes as item subresources", async () => {
  await action("holes:revoke").action!(row());
  await action("holes:probe").action!(row());
  expect(api).toHaveBeenNthCalledWith(1, "/workspaces/ops/holes/7/revocations", { method: "POST" });
  expect(api).toHaveBeenNthCalledWith(2, "/workspaces/ops/holes/7/probes", { method: "POST" });
});

it("starts creation with a 24-hour validity and prefix stripping, and locks host and port after creation", () => {
  const config = formConfig();
  expect(config.blank).toEqual({ name: "", host_id: "", strip_prefix: true, ttl_hours: 24 });
  const byKey = (key: string) => fields().filter((field) => field.key === key);
  expect(byKey("host_id")[0]).toMatchObject({ type: "select", required: true, immutable: true });
  expect(byKey("target_port")[0]).toMatchObject({ type: "number", required: true, immutable: true, min: 1, max: 65535 });
  expect(byKey("name")[0]).toMatchObject({ required: true });
  expect(byKey("name")[0]!.immutable).toBeUndefined();
  expect(byKey("strip_prefix")).toEqual([]);
  const [createTtl, editTtl] = byKey("ttl_hours");
  expect(createTtl!.required).toBe(true);
  expect(createTtl!.when!(config.blank)).toBe(true);
  expect(editTtl!.when!(config.blank)).toBe(false);
  expect(editTtl!.required).toBeUndefined();
});

it("edits only name, path mode and validity, and treats revoked records as read-only", () => {
  const config = formConfig();
  const edit = { ...row(), ...config.editValues!(row()) };
  expect(edit).toMatchObject({ name: "Web", strip_prefix: true, version: 4 });
  expect(edit.ttl_hours).toBeUndefined();
  const visible = (draft: Partial<Hole>) => fields().filter((field) => !field.when || field.when(draft)).map((field) => field.key);
  expect(visible(edit)).toEqual(["ingress_mode", "name", "host_id", "target_port", "ttl_hours"]);
  expect(visible(config.blank)).toEqual(["ingress_mode", "name", "host_id", "target_port", "ttl_hours"]);
  expect(visible(row())).toEqual([
    "ingress_mode", "name", "host_id", "target_port", "url", "state", "probe_state", "probe_error",
    "active_connections", "expires_at", "created_by_name", "created_at", "updated_at", "usage",
  ]);
  expect(visible(row({ state: "revoked", revoked_at: 5, probe_error: "" }))).toContain("revoked_at");
  expect(config.readOnly!(row({ state: "revoked" }))).toBe(true);
  expect(config.readOnly!(row({ state: "expired" }))).toBe(false);
});

it("ships every holes message in six non-empty locales", () => {
  for (const [key, values] of Object.entries(messages)) {
    expect(values, key).toHaveLength(6);
    for (const value of values) expect(value.trim(), key).not.toBe("");
  }
});

function EntryHarness({ editing = false }: {editing?: boolean}) {
  const [draft, setDraft] = useState<Partial<Hole>>(editing ? {...row(), ...formConfig().editValues!(row({ingress_mode:"ip_port"}))} : formConfig().blank);
  const [blocked, setBlocked] = useState(true);
  const entry = fields().find(field => field.key === "ingress_mode")!;
  return <>{entry.render!({workspace:"ops",draft,readOnly:false,onPatch:patch=>setDraft(value=>({...value,...patch})),onValidityChange:setBlocked})}
    <button disabled={blocked}>test-submit</button><output data-testid="entry-draft">{JSON.stringify(draft)}</output></>;
}

it("applies the configured default and blocks public creation until explicitly acknowledged", async () => {
  vi.mocked(api).mockResolvedValueOnce({items:[{id:"domain",available:true,public_access:false},{id:"ip_port",available:true,public_access:true}],default_mode:"ip_port"});
  await mount(<EntryHarness />);
  expect(container.textContent).toContain("HTTP is unencrypted");
  expect(api).toHaveBeenCalledWith("/workspaces/ops/preview-ingresses");
  const submit = Array.from(container.querySelectorAll("button")).find(button=>button.textContent==="test-submit")!;
  expect(submit.disabled).toBe(true);
  await act(async()=>{container.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click();});
  expect(submit.disabled).toBe(false);
  expect(JSON.parse(container.querySelector("output")!.textContent!)).toMatchObject({ingress_mode:"ip_port",public_access_acknowledged:true});
});

it("preserves an existing entry when platform defaults change and supports retry after options fail", async () => {
  vi.mocked(api).mockRejectedValueOnce(new Error("offline")).mockResolvedValueOnce({items:[{id:"domain",available:true,public_access:false}],default_mode:"domain"});
  await mount(<EntryHarness editing />);
  expect(container.querySelector('[role="alert"]')).toBeTruthy();
  const submit = Array.from(container.querySelectorAll("button")).find(button=>button.textContent==="test-submit")!;
  expect(submit.disabled).toBe(true);
  const retry = Array.from(container.querySelectorAll("button")).find(button=>/retry/i.test(button.textContent??""))!;
  await act(async()=>{retry.click();});
  expect(container.textContent).toContain("Keep current:");
  expect(submit.disabled).toBe(false);
  const draft = JSON.parse(container.querySelector("output")!.textContent!);
  expect(draft.current_ingress_mode).toBe("ip_port");
  expect(draft.ingress_mode).toBeUndefined();
  expect(draft.reconnect_ingress).toBe(false);
});

it("shows public exposure on details and hides copy/open actions for an offline ingress", async () => {
  const draft=row({ingress_mode:"cloudflare",public_access:true,ingress_available:false});
  const field=fields().find(field=>field.key==="ingress_mode")!;
  await mount(<>{field.render!({workspace:"ops",draft,readOnly:true,onPatch:vi.fn(),onValidityChange:vi.fn()})}</>);
  expect(container.textContent).toContain("Public Internet access");
  expect(container.querySelector('input[type="checkbox"]')).toBeNull();
  expect(api).not.toHaveBeenCalled();
  expect(action("holes:open").when!(draft)).toBe(false);
  expect(action("holes:copyLink").when!(draft)).toBe(false);
});

it.each([
  ["en-US", "Preview tunnels", "Revoked", "Runner offline", "Waiting", "Pipeline run · build"],
  ["zh-CN", "预览穿透", "已撤销", "Runner 离线", "待验收", "流水线运行 · build"],
  ["ms-MY", "Terowong pratonton", "Dibatalkan", "Runner luar talian", "Menunggu", "Larian saluran paip · build"],
])("translates the page in %s", async (locale, title, revoked, offline, waiting, pipeline) => {
  await i18n.changeLanguage(locale);
  expect(i18n.t("title", { ns: "holes" })).toBe(title);
  expect(label(HOLE_STATES, "revoked")).toBe(revoked);
  expect(label(PROBE_STATES, "runner_offline")).toBe(offline);
  expect(label(REVIEW_STATES, "waiting")).toBe(waiting);
  expect(i18n.t("source.pipeline", { ns: "holes", ref: "build" })).toBe(pipeline);
});
