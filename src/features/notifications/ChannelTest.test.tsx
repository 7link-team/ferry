import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ResourceForm } from "../../app/resources/ResourceForm";
import { i18n } from "../../shared/i18n/i18n";
import { notificationForms, notificationLists } from "./resource-config";

let root: Root, host: HTMLDivElement;
let requests: { url: string; method: string | undefined }[];
let stored: Record<string, unknown>;
beforeEach(() => {
  void i18n.changeLanguage("zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  requests = [];
  stored = { id: "15", name: "WeCom", provider: "WeCom", enabled: true, language: "zh-CN", role_ids: [], recipients: [], has_webhook: true, webhook_masked: "https://x/••••", version: 3, last_test: null };
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    requests.push({ url, method: init.method });
    const body = url.endsWith("/tests") ? { id: "77", state: "succeeded" } : url.includes("/roles") ? { items: [], total: 0 } : stored;
    return { ok: true, status: 200, json: async () => body };
  }));
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

it("tests a saved channel with a real message and opens its delivery", async () => {
  const navigate = vi.fn();
  const config = notificationLists({ workspace: "ops", navigate, permissions: new Set(["notifications.update"]) } as never)["notification-platforms"]!;
  const test = config.actions!.find((action) => action.labelKey === "notifications:testChannel")!;
  expect(test.when!(stored as never)).toBe(true);
  expect(test.when!({ ...stored, provider: "Aliyun SMS" } as never)).toBe(false);
  await test.action!(stored as never);
  expect(requests).toContainEqual({ url: "/api/workspaces/ops/notification-channels/15/tests", method: "POST" });
  expect(navigate).toHaveBeenCalledWith({ group: "ops", item: "notification-deliveries", id: "77", mode: "view" });
});

it("shows whether a channel was tested and how it went", async () => {
  const permissions = new Set(["notifications.read"]);
  const config = notificationForms({ workspace: "ops", navigate: vi.fn(), permissions } as never)["notification-channels"]!;
  history.replaceState({}, "", "/zh-CN/workspaces/ops/notification-channels/15");
  await act(async () => root.render(<ResourceForm config={config} workspace="ops" permissions={permissions} />));
  expect(host.textContent).toContain("尚未测试");
  await act(async () => root.unmount());
  root = createRoot(host);
  stored.last_test = { id: "77", state: "failed", at: Date.UTC(2026, 10, 16), result: "无法连接企业微信" };
  await act(async () => root.render(<ResourceForm config={config} workspace="ops" permissions={permissions} />));
  expect(host.textContent).toContain("测试未成功：无法连接企业微信");
});
