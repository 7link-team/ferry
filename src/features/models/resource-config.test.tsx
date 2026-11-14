import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ResourceForm } from "../../app/resources/ResourceForm";
import { i18n, workflowText } from "../../shared/i18n/i18n";
import { modelsForms } from "./resource-config";
import { matchPreset, providerPresets } from "./ProviderPresets";

let root: Root, host: HTMLDivElement;
let requests: { url: string; method: string | undefined; body: string | undefined }[];
let stored: Record<string, unknown>;
const permissions = new Set(["ai-models.read", "ai-models.create", "ai-models.update", "credentials.read"]);

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
    const body = stored;
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

const type = (input: HTMLInputElement, value: string) => {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
};
const cli = (extra: Record<string, unknown>) => ({
  id: "9", name: "Claude", kind: "cli", runtime: "claude", provider_id: "0", models: [], default_model: "",
  tool_version: "", state: "enabled", version: 2, cli_options: null, ...extra,
});

it("configures a CLI's own custom API with per-tool options instead of reusing an API model", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/ops/ai-models/9/edit");
  stored = cli({});
  await act(async () => root.render(<ResourceForm config={config()} workspace="ops" permissions={permissions} />));
  expect(requests.some((r) => r.url.includes("kind=api"))).toBe(false);
  expect(host.textContent).toContain("Claude Code");
  const toggle = host.querySelector<HTMLButtonElement>('[role="switch"][aria-label="使用自定义 API 连接"]')!;
  await act(async () => toggle.click());
  await act(async () => type(host.querySelector<HTMLInputElement>('input[aria-label="接口地址"]')!, "https://api.deepseek.com/anthropic"));
  await act(async () => type(host.querySelector<HTMLInputElement>('input[aria-label="API Key"]')!, "sk-new"));
  await act(async () => type(host.querySelector<HTMLInputElement>('input[aria-label="Opus 模型"]')!, "deepseek-pro"));
  await submit();
  const body = JSON.parse(requests.find((r) => r.method === "PATCH" || r.method === "PUT")!.body!);
  expect(body).toMatchObject({
    kind: "cli",
    runtime: "claude",
    provider_id: "0",
    cli_options: { format: "anthropic", opus_model: "deepseek-pro" },
    connection: { base_url: "https://api.deepseek.com/anthropic", api_key: "sk-new", protocol: "anthropic" },
  });
});

it("edits a saved CLI connection, keeping the key unless replaced", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/ops/ai-models/9/edit");
  stored = cli({
    runtime: "codex", provider_id: "7", cli_options: { format: "openai_responses", reasoning_effort: "high" },
    connection: { name: "Codex", protocol: "openai", base_url: "https://gw.example/v1", credential_id: "3", qps: 1, concurrency: 1, version: 1 },
  });
  await act(async () => root.render(<ResourceForm config={config()} workspace="ops" permissions={permissions} />));
  expect(host.querySelector<HTMLInputElement>('input[aria-label="接口地址"]')!.value).toBe("https://gw.example/v1");
  expect(host.textContent).toContain("留空保留已保存的密钥");
  await submit();
  const body = JSON.parse(requests.find((r) => r.method === "PATCH" || r.method === "PUT")!.body!);
  expect(body.cli_options).toEqual({ format: "openai_responses", reasoning_effort: "high" });
  expect(body.connection).toMatchObject({ base_url: "https://gw.example/v1", credential_id: "3" });
  expect(body.connection.api_key).toBeUndefined();
});

it("keeps login-only CLIs on their own configuration", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/ops/ai-models/9/edit");
  stored = cli({ runtime: "copilot", name: "Copilot" });
  await act(async () => root.render(<ResourceForm config={config()} workspace="ops" permissions={permissions} />));
  expect(host.textContent).toContain("GitHub Copilot CLI 只使用节点上自己的登录和配置");
  expect(host.querySelector('[role="switch"][aria-label="使用自定义 API 连接"]')).toBeNull();
});

it("shows no pricing, currency, timezone or budget fields and keeps stored metering on save", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/ops/ai-models/11/edit");
  const prices = [{ model: "m", operation: "ai.llm.chat", input_micros: 2, output_micros: 3, unit_size: 1000 }];
  stored = {
    id: "11", name: "Priced", kind: "api", runtime: "", provider_id: "8", models: ["m"], default_model: "m", tool_version: "",
    state: "enabled", version: 4,
    connection: { name: "Priced", protocol: "openai", base_url: "https://api.example.com/v1", credential_id: "3", currency: "EUR", timezone: "Asia/Shanghai", monthly_limit_micros: 50, qps: 2, concurrency: 3, prices, version: 1 },
  };
  await act(async () => root.render(<ResourceForm config={config()} workspace="ops" permissions={permissions} />));
  for (const label of (["aiCurrency", "aiTimezone", "aiMonthlyBudget", "aiInputPrice", "aiOutputPrice"] as const).map(workflowText))
    expect(host.querySelector(`[aria-label="${label}"]`)).toBeNull();
  expect(host.textContent).not.toContain(workflowText("aiAddPrice"));
  expect(workflowText("aiAddPrice")).not.toBe("aiAddPrice");
  await submit();
  const body = JSON.parse(requests.find((r) => r.method === "PATCH" || r.method === "PUT")!.body!);
  expect(body.connection).toMatchObject({ currency: "EUR", timezone: "Asia/Shanghai", monthly_limit_micros: 50, prices });
});
