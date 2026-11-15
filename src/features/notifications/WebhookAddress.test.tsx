import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ResourceForm } from "../../app/resources/ResourceForm";
import { i18n } from "../../shared/i18n/i18n";
import { notificationForms } from "./resource-config";

let root: Root, host: HTMLDivElement;
let requests: { url: string; method: string | undefined; body: string | undefined }[];
const address = "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=private-1234";
const stored = {
  id: "15", name: "WeCom", description: "", provider: "WeCom", enabled: true, language: "zh-CN", role_ids: [], recipients: [],
  has_webhook: true, webhook_masked: "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=••••1234", has_secret: false, version: 3,
};

beforeEach(() => {
  void i18n.changeLanguage("zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  requests = [];
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    requests.push({ url, method: init.method, body: init.body as string });
    const body = url.endsWith("/webhook-reveals") ? { webhook: address } : url.includes("/roles") ? { items: [], total: 0 } : stored;
    return { ok: true, status: 200, json: async () => body };
  }));
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
});
const render = async (permissions: string[]) => {
  const set = new Set(permissions);
  const config = notificationForms({ workspace: "ops", permissions: set } as never)["notification-channels"]!;
  await act(async () => root.render(<ResourceForm config={config} workspace="ops" permissions={set} />));
};
const button = (text: string) => [...host.querySelectorAll("button")].find((b) => b.textContent === text);

it("echoes the saved address masked and keeps it when the input is left empty", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/ops/notification-channels/15/edit");
  await render(["notifications.read", "notifications.update"]);
  expect(host.querySelector('[aria-label="当前地址"]')!.textContent).toBe(stored.webhook_masked);
  expect(host.textContent).not.toContain("private-1234");
  const input = host.querySelector<HTMLInputElement>('input[aria-label="Webhook 地址"]')!;
  expect(input.value).toBe("");
  expect(input.placeholder).toContain("留空则保持当前地址");
  await act(async () => button("查看明文（记录审计）")!.click());
  expect(requests.some((r) => r.url.endsWith("/notification-channels/15/webhook-reveals") && r.method === "POST")).toBe(true);
  expect(host.querySelector('[aria-label="当前地址"]')!.textContent).toBe(address);
});

it("offers no reveal to someone who can only read the channel", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/ops/notification-channels/15");
  await render(["notifications.read"]);
  expect(host.querySelector('[aria-label="当前地址"]')!.textContent).toBe(stored.webhook_masked);
  expect([...host.querySelectorAll("button")].some((b) => /明文|plaintext/i.test(b.textContent ?? ""))).toBe(false);
});
