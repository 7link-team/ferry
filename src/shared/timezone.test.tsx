import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { i18n, commonText } from "./i18n/i18n";
import { WorkspaceDate, WorkspaceTimezoneContext, dayKey, formatDate, localDateTime, nextDayAt, parseAbsoluteDateTime, parseLocalDateTime } from "./timezone";
import { WorkspaceDateTimeInput } from "./forms/WorkspaceDateTimeInput";
import { TimezoneField } from "./forms/TimezoneField";
import { analysisDateRange } from "../features/code-insights/analysis-config";
import { workspacesForms } from "../features/workspaces/resource-config";
import { Entry } from "../app/Entry";
import { ResourceForm } from "../app/resources/ResourceForm";
import { Dashboard } from "../features/dashboard/Dashboard";

let host: HTMLDivElement, root: Root;
it.each(["zh-CN", "en-US", "ms-MY"])("interpolates dashboard update time as workspace text in %s", async locale => {
  const previous = location.pathname + location.search;
  history.replaceState({}, "", `/${locale}/workspaces/1/dashboard`);
  await i18n.changeLanguage(locale);
  const at = Date.parse("2026-01-01T00:30:00Z");
  vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify({ counts:{},states:{},failures_24h:{},runners:[],recent:[],as_of:at }), {headers:{"Content-Type":"application/json"}})));
  try {
    for (const zone of ["Asia/Kuala_Lumpur", "America/Los_Angeles"]) {
      await act(async () => root.render(<WorkspaceTimezoneContext.Provider value={zone}><Dashboard workspace="1" /></WorkspaceTimezoneContext.Provider>));
      expect(host.textContent).toContain(commonText("dashboard.updated", {time:formatDate(at,zone,"datetime",locale)}));
      expect(host.textContent).not.toContain("[object Object]");
    }
  } finally { history.replaceState({}, "", previous); }
});
beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  localStorage.setItem("ui.locale", "zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
const input = async (element: HTMLInputElement, value: string) => act(async () => {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(element, value);
  element.dispatchEvent(new Event("input", { bubbles: true }));
});
it("formats instants in the selected workspace without leaking into another context or legacy", async () => {
  const at = Date.parse("2026-01-01T00:30:00Z");
  await act(async () => root.render(<><WorkspaceTimezoneContext.Provider value="America/Los_Angeles"><span id="west"><WorkspaceDate value={at} /></span></WorkspaceTimezoneContext.Provider><WorkspaceTimezoneContext.Provider value="Asia/Kuala_Lumpur"><span id="east"><WorkspaceDate value={at} /></span></WorkspaceTimezoneContext.Provider><span id="legacy"><WorkspaceDate value={at} /></span></>));
  expect(host.querySelector("#west")!.textContent).toBe(formatDate(at, "America/Los_Angeles"));
  expect(host.querySelector("#east")!.textContent).toBe(formatDate(at, "Asia/Kuala_Lumpur"));
  expect(host.querySelector("#legacy")!.textContent).toBe(new Date(at).toLocaleString("zh-CN"));
  expect(dayKey(at, "America/Los_Angeles")).toBe("2025-12-31");
  expect(dayKey(at, "Asia/Kuala_Lumpur")).toBe("2026-01-01");
  expect(dayKey(at, null)).toBe("2026-01-01");
});
it("uses business calendar days for presets and retains the historical UTC calendar", () => {
  const at = new Date("2026-03-01T00:30:00Z");
  expect(analysisDateRange("today", at, "America/Los_Angeles")).toEqual({ since: "2026-02-28", until: "2026-02-28" });
  expect(analysisDateRange("this-month", at, "America/Los_Angeles")).toEqual({ since: "2026-02-01", until: "2026-02-28" });
  expect(analysisDateRange("today", at)).toEqual({ since: "2026-03-01", until: "2026-03-01" });
  expect(analysisDateRange("this-week", new Date("2026-03-09T00:30:00Z"), "America/New_York")).toEqual({ since: "2026-03-02", until: "2026-03-08" });
});
it("finds the next business midnight across short and long DST days", () => {
  expect(nextDayAt(Date.parse("2026-03-08T05:00:00Z"), "America/New_York")).toBe(Date.parse("2026-03-09T04:00:00Z"));
  expect(nextDayAt(Date.parse("2026-11-01T04:00:00Z"), "America/New_York")).toBe(Date.parse("2026-11-02T05:00:00Z"));
  expect(parseAbsoluteDateTime("2026-02-30T01:30:00Z")).toBeNull();
  expect(parseAbsoluteDateTime("2026-11-01T01:30:00-04:00")).toBe("2026-11-01T05:30:00.000Z");
});
it("round-trips a wall time and rejects DST gaps, repeated clocks, invalid calendar dates and fractional-hour transitions", () => {
  expect(parseLocalDateTime("2026-05-01T08:45", "Asia/Kathmandu")).toBe("2026-05-01T03:00:00.000Z");
  expect(localDateTime("2026-05-01T03:00:00Z", "Asia/Kathmandu")).toBe("2026-05-01T08:45");
  expect(parseLocalDateTime("2026-03-08T02:30", "America/New_York")).toBeNull();
  expect(parseLocalDateTime("2026-11-01T01:30", "America/New_York")).toBeNull();
  expect(parseLocalDateTime("2026-02-30T10:00", "UTC")).toBeNull();
  expect(parseLocalDateTime("2026-10-04T02:15", "Australia/Lord_Howe")).toBeNull();
  expect(parseLocalDateTime("2026-04-05T01:45", "Australia/Lord_Howe")).toBeNull();
});
it("keeps invalid wall input visible, blocks saving, and accepts an explicit ISO offset to disambiguate", async () => {
  let stored = "2026-11-01T06:00:00Z"; let blocked = false;
  function Form() { const [value, setValue] = useState(stored); return <WorkspaceTimezoneContext.Provider value="America/New_York"><WorkspaceDateTimeInput value={value} readOnly={false} onChange={next => { stored = next; setValue(next); }} onValidityChange={next => { blocked = next; }} /></WorkspaceTimezoneContext.Provider>; }
  await act(async () => root.render(<Form />));
  await input(host.querySelector('input[type="datetime-local"]')!, "2026-11-01T01:30");
  expect(blocked).toBe(true); expect(stored).toBe("2026-11-01T06:00:00Z");
  expect(host.querySelector('[role="alert"]')).not.toBeNull();
  await input(host.querySelectorAll<HTMLInputElement>("input")[1]!, "2026-11-01T01:30:00-04:00");
  expect(stored).toBe("2026-11-01T05:30:00.000Z"); expect(blocked).toBe(false);
});
it("requires a nonempty valid business timezone without changing optional legacy fields", async () => {
  let blocked = false;
  await act(async () => root.render(<TimezoneField label="zone" value="" required emptyLabel="choose" readOnly={false} onChange={() => {}} onValidityChange={value => { blocked = value; }} />));
  expect(blocked).toBe(true);
  await act(async () => root.render(<TimezoneField label="zone" value="" emptyLabel="legacy" readOnly={false} onChange={() => {}} onValidityChange={value => { blocked = value; }} />));
  expect(blocked).toBe(false);
});
it("does not confirm a legacy workspace on load; explicit settings selection saves the scoped versioned timezone", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/ws/workspace-settings/1/edit");
  const writes: { url: string; body: Record<string, unknown> }[] = [];
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    if (init.method === "PATCH") writes.push({ url, body: JSON.parse(String(init.body)) });
    return { ok: true, status: 200, json: async () => ({ id: "1", name: "Test", timezone: null, version: 7 }) };
  }));
  const config = workspacesForms({ workspace: "ws", navigate: () => {} })["workspace-settings"]!;
  await act(async () => root.render(<ResourceForm config={config} workspace="ws" permissions={new Set(["settings.update"])} />));
  expect(writes).toEqual([]);
  const picker = host.querySelector<HTMLInputElement>('input[aria-label="'+commonText("resource-domain:workspace.timezone")+'"]')!;
  expect(picker.value).toBe("");
  await input(picker, "Asia/Kuala_Lumpur");
  await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(writes).toEqual([{ url: "/api/workspaces/ws/workspace-settings/1", body: { id: "1", name: "Test", timezone: "Asia/Kuala_Lumpur", version: 7 } }]);
});

it("creates a workspace with the editable detected timezone and only declared payload fields", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces");
  const writes: Record<string, unknown>[] = [];
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    if (url === "/api/workspaces" && init.method === "POST") writes.push(JSON.parse(String(init.body)));
    return { ok: true, status: 200, json: async () => url === "/api/setup" ? { required: false, has_users: true, registration_enabled: true } : { id: "ws" } };
  }));
  await act(async () => root.render(<Entry signedIn onReady={() => {}} error="" />));
  const picker = host.querySelector<HTMLInputElement>('[role="combobox"]')!;
  expect(picker.value).toBe(Intl.DateTimeFormat().resolvedOptions().timeZone);
  await input(picker, "Asia/Kathmandu");
  await input(host.querySelector('input[name="name"]')!, "Workspace");
  await input(host.querySelector('input[name="slug"]')!, "workspace");
  await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(writes).toEqual([{ name: "Workspace", slug: "workspace", timezone: "Asia/Kathmandu" }]);
});
