import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ResourceForm } from "../../app/resources/ResourceForm";
import { i18n } from "../../shared/i18n/i18n";
import { modelsForms } from "./resource-config";
import { matchPreset, providerPresets } from "./ProviderPresets";

let root: Root, host: HTMLDivElement;
let requests: { url: string; method: string | undefined; body: string | undefined }[];
let stored: Record<string, unknown>;
const permissions = new Set(["ai-models.read", "ai-models.create", "ai-models.update", "credentials.read"]);
const apiModels = [{ id: "5", name: "DeepSeek API", kind: "api", provider_id: "7", models: ["deepseek-flash"], default_model: "deepseek-flash" }];

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
    const body = url.includes("ai-models?") ? { items: apiModels, total: 1, page: 1, page_size: 100 } : stored;
    return { ok: true, status: 200, json: async () => body };
  }));
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
});
const config = () => modelsForms({ workspace: "ops", permissions } as never)["ai-models"]!;
const submit = () => act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));

it("ships CC Switch presets with clean, unique endpoints", () => {
  expect(providerPresets.length).toBeGreaterThan(50);
  expect(new Set(providerPresets.map((p) => p.name)).size).toBe(providerPresets.length);
  for (const p of providerPresets) {
    expect(["openai", "anthropic"]).toContain(p.protocol);
    expect(p.base_url).toMatch(/^https?:\/\/[^?#]+[^/]$/);
    for (const model of p.models) expect(model).not.toMatch(/\[/);
    for (const link of [p.website, p.api_key_url]) if (link) expect(link).not.toMatch(/[?#]/);
  }
  expect(matchPreset("anthropic", "https://api.deepseek.com/anthropic/v1/")?.name).toBe("DeepSeek");
});

it("fills an API model from a provider preset", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/ops/ai-models/new");
  await act(async () => root.render(<ResourceForm config={config()} workspace="ops" permissions={permissions} />));
  const chip = [...host.querySelectorAll("button")].find((b) => b.textContent === "DeepSeek")!;
  await act(async () => chip.click());
  expect(chip.getAttribute("aria-pressed")).toBe("true");
  expect(host.querySelector<HTMLInputElement>('input[name="name"]')!.value).toBe("DeepSeek");
  expect([...host.querySelectorAll<HTMLInputElement>("input")].some((i) => i.value === "https://api.deepseek.com/anthropic/v1")).toBe(true);
  expect(host.querySelector('a[href="https://platform.deepseek.com"]')).toBeTruthy();
});

it("lets a CLI with a custom API only reference an existing API model and never resave its connection", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/ops/ai-models/9/edit");
  stored = {
    id: "9", name: "Claude via DeepSeek", kind: "cli", runtime: "claude", provider_id: "7", models: ["deepseek-flash"],
    default_model: "deepseek-flash", tool_version: "", state: "enabled", version: 2,
    connection: { name: "DeepSeek API", protocol: "anthropic", base_url: "https://api.deepseek.com/anthropic/v1", credential_id: "3", currency: "USD", timezone: "UTC", monthly_limit_micros: 0, qps: 1, concurrency: 1, prices: [], version: 1 },
  };
  await act(async () => root.render(<ResourceForm config={config()} workspace="ops" permissions={permissions} />));
  expect(requests.some((r) => r.url.includes("/workspaces/ops/ai-models?kind=api"))).toBe(true);
  expect(host.textContent).toContain("DeepSeek API");
  expect([...host.querySelectorAll<HTMLInputElement>("input")].some((i) => i.value.includes("api.deepseek.com"))).toBe(false);
  await submit();
  const saved = requests.find((r) => r.method === "PATCH" || r.method === "PUT")!;
  const body = JSON.parse(saved.body!);
  expect(body).toMatchObject({ kind: "cli", provider_id: "7", version: 2 });
  expect(body.connection ?? null).toBeNull();
});
