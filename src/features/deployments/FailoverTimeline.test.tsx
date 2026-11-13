import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { commonText, i18n } from "../../shared/i18n/i18n";
import { FailoverAttention, FailoverTimeline } from "./FailoverTimeline";
import type { Deployment } from "./deployment";

const t = (key: string) => commonText(`resource-domain:failover.${key}`);
const app = { id: "7", version: 4, target_kind: "blueprint", failover: { mode: "auto", host_down_after: 90, fences: [], attention: [{ instance: "role-with-dashes-1", since: 1000 }] } } as unknown as Deployment;
const event = { id: "1", kind: "replaceSucceeded", instance: "nats-1", host_id: "5", host_name: "node-five", revision_id: "11", created_at: 1000, message: "Healthy on replacement host", detail: { operation_id: "91" } };
let root: Root, host: HTMLDivElement, calls: { url: string; init: RequestInit }[], reply: () => Promise<unknown>, status: number;
const flush = async () => { for (let i = 0; i < 6; i++) await act(async () => { await Promise.resolve(); }); };
const mount = async () => { await act(async () => root.render(<FailoverTimeline workspace="ops" deployment={app} />)); await flush(); };
const click = async (name: string) => { const b = [...host.querySelectorAll("button")].find(b => b.textContent === name)!; expect(b).toBeDefined(); await act(async () => b.click()); await flush(); };
beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  await i18n.changeLanguage("en-US");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host); calls = []; status = 200;
  reply = async () => ({ items: [event], total: 45, page: 1, page_size: 20 });
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => { calls.push({ url, init }); const data = await reply(); return { ok: status < 400, status, json: async () => data }; }));
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

it("timeline_renders_events_with_host_names_and_revision_links", async () => {
  await mount(); expect(host.textContent).toContain("node-five"); expect(host.textContent).toContain(event.message);
  expect(host.querySelector("a")?.getAttribute("href")).toBe("/en-US/workspaces/ops/deployment-revisions/11");
  expect(calls[0]!.url).toContain("/deployments/7/events?page=1&page_size=20");
  reply = async () => ({ items: [{ ...event, revision_id: "0" }], total: 1 });
  await act(async () => root.render(<FailoverTimeline workspace="ops" deployment={{ ...app, version: 5 }} />)); await flush();
  expect(host.querySelector("a")).toBeNull();
});
it("timeline_states_loading_empty_error_retry", async () => {
  let release!: (value: unknown) => void; reply = () => new Promise(resolve => { release = resolve; });
  await mount(); expect(host.querySelector('[role="status"]')).not.toBeNull();
  await act(async () => release({ items: [], total: 0 })); await flush(); expect(host.textContent).toContain(t("empty"));
  status = 500; reply = async () => ({ error: "store offline" });
  await act(async () => root.render(<FailoverTimeline workspace="ops" deployment={{ ...app, version: 5 }} />)); await flush();
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("store offline");
  status = 200; reply = async () => ({ items: [event], total: 1 }); await click(commonText("common.retry"));
  expect(host.querySelector('[role="alert"]')).toBeNull(); expect(host.textContent).toContain(event.message);
});
it("timeline_paginates_and_clears_old_page_while_loading", async () => {
  await mount(); expect([...host.querySelectorAll("button")].find(b => b.textContent === t("previous"))?.disabled).toBe(true);
  await click(t("next")); expect(calls.at(-1)!.url).toContain("page=2");
  await click(t("next")); expect(calls.at(-1)!.url).toContain("page=3");
  expect([...host.querySelectorAll("button")].find(b => b.textContent === t("next"))?.disabled).toBe(true);
  await click(t("previous")); expect(calls.at(-1)!.url).toContain("page=2");
});
it("attention_banner_acknowledges_with_version_and_preserves_error", async () => {
  const done = vi.fn(); await act(async () => root.render(<FailoverAttention workspace="ops" deployment={app} permissions={new Set(["deployments.apply"])} onDone={done} />));
  status = 409; reply = async () => ({ error: "version conflict" }); await click(t("acknowledge"));
  expect(done).not.toHaveBeenCalled(); expect(host.textContent).toContain("version conflict");
  status = 200; reply = async () => ({}); await click(t("acknowledge")); expect(done).toHaveBeenCalledTimes(1);
  const request = calls.at(-1)!; expect(request.url).toBe("/api/workspaces/ops/deployments/7/instances/role-with-dashes/1/acknowledgements");
  expect(request.init.method).toBe("POST"); expect(JSON.parse(String(request.init.body))).toEqual({ version: 4 });
  await act(async () => root.render(<FailoverAttention workspace="ops" deployment={app} permissions={new Set()} onDone={done} />));
  expect(host.querySelector("button")).toBeNull(); expect(host.textContent).toContain("role-with-dashes-1");
});
