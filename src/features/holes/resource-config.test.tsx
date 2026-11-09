import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api } from "../../shared/api/api";
import { i18n } from "../../shared/i18n/i18n";
import { setLocale } from "../../shared/i18n/locale";
import messages from "../../shared/i18n/locales/holes.json";
import { label } from "../../shared/resources/config";
import { nav } from "../../app/nav";
import { href, parse } from "../../app/router";
import { HOLE_STATES, holesForms, holesLists, PROBE_STATES, type Hole } from "./resource-config";

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
  created_at: 1, created_by: "1", created_by_name: "Ada", updated_at: 1, updated_by: "1", version: 4, ...patch,
});

beforeEach(async () => {
  history.replaceState({}, "", "/en-US/");
  setLocale("en-US", false);
  await i18n.changeLanguage("en-US");
});
afterEach(() => { vi.clearAllMocks(); vi.restoreAllMocks(); });

it("adds a holes.read nav entry right after hosts and derives list, create, detail and edit routes", () => {
  const host = nav.find((group) => group.key === "host")!;
  const index = host.items.findIndex((item) => item.key === "hosts");
  expect(host.items[index + 1]).toMatchObject({ key: "holes", resource: "holes", permission: "holes.read" });
  expect(host.items[index + 1]!.label).toBe("Preview tunnels");
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
    "name", "host_name", "target_port", "state", "probe_state", "active_connections", "expires_at", "created_by_name",
  ]);
  expect(config.filters!.map((filter) => filter.key)).toEqual(["q", "host_id", "state"]);
  expect(config.emptyKey).toBe("holes:empty");
  expect(config.toolbar?.create).toMatchObject({ labelKey: "holes:create" });
});

it("gates row actions by permission and state", () => {
  const permissions = Object.fromEntries(listConfig().actions!.map((candidate) => [candidate.labelKey, candidate.permission]));
  expect(permissions).toEqual({
    "holes:copyLink": undefined, "holes:open": undefined, "holes:probe": "holes.update", "holes:revoke": "holes.update",
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
  expect(byKey("strip_prefix")[0]).toMatchObject({ type: "switch", helpKey: "holes:stripPrefixHelp" });
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
  expect(visible(edit)).toEqual(["name", "host_id", "target_port", "strip_prefix", "ttl_hours"]);
  expect(visible(config.blank)).toEqual(["name", "host_id", "target_port", "strip_prefix", "ttl_hours"]);
  expect(visible(row())).toEqual([
    "name", "host_id", "target_port", "strip_prefix", "url", "state", "probe_state", "probe_error",
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

it.each([
  ["en-US", "Preview tunnels", "Revoked", "Runner offline"],
  ["zh-CN", "预览穿透", "已撤销", "Runner 离线"],
  ["ms-MY", "Terowong pratonton", "Dibatalkan", "Runner luar talian"],
])("translates the page in %s", async (locale, title, revoked, offline) => {
  await i18n.changeLanguage(locale);
  expect(i18n.t("title", { ns: "holes" })).toBe(title);
  expect(label(HOLE_STATES, "revoked")).toBe(revoked);
  expect(label(PROBE_STATES, "runner_offline")).toBe(offline);
});
