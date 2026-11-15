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
const apiModels = [
  { id: "5", name: "DeepSeek Anthropic", kind: "api", provider_id: "7", models: ["deepseek-pro"], default_model: "deepseek-pro" },
  { id: "6", name: "OpenAI Chat", kind: "api", provider_id: "8", models: ["gpt-x"], default_model: "gpt-x" },
];
const providers = [{ id: "7", protocol: "anthropic" }, { id: "8", protocol: "openai" }];
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
    const body = url.includes("ai-models?") ? { items: apiModels, total: apiModels.length, page: 1, page_size: 100 }
      : url.endsWith("/ai-providers") ? { items: providers } : stored;
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
  // Existing API models come first; a separate connection is one click away.
  expect(host.querySelector('[role="tab"][aria-selected="true"]')!.textContent).toBe("使用已有 API 模型");
  const own = [...host.querySelectorAll<HTMLButtonElement>('[role="tab"]')].find((b) => b.textContent === "单独填写连接")!;
  // Radix tabs switch on pointer down / keyboard, not a synthetic click.
  await act(async () => {
    own.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    own.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, button: 0 }));
  });
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
    runtime: "codex", provider_id: "17", cli_options: { format: "openai_responses", reasoning_effort: "high" },
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

const choose = async (name: string, value: string) => {
  const select = host.querySelector<HTMLSelectElement>(`select[name="${name}"]`)!;
  await act(async () => {
    select.value = value;
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
};

it("lets a CLI reuse only API models it can call directly, without resaving their connection", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/ops/ai-models/9/edit");
  stored = cli({});
  await act(async () => root.render(<ResourceForm config={config()} workspace="ops" permissions={permissions} />));
  await act(async () => host.querySelector<HTMLButtonElement>('[role="switch"][aria-label="使用自定义 API 连接"]')!.click());
  const options = [...host.querySelectorAll<HTMLOptionElement>('select[name="cli-api-model"] option')].map((o) => o.textContent);
  expect(options).toContain("DeepSeek Anthropic");
  // Claude Code speaks Anthropic only; an OpenAI connection would need conversion.
  expect(options).not.toContain("OpenAI Chat");
  await choose("cli-api-model", "5");
  await submit();
  const body = JSON.parse(requests.find((r) => r.method === "PATCH" || r.method === "PUT")!.body!);
  expect(body).toMatchObject({ provider_id: "7", models: ["deepseek-pro"], default_model: "deepseek-pro", cli_options: { format: "anthropic" } });
  expect(body.connection ?? null).toBeNull();
});

it("keeps a saved CLI linked to its API model when edited", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/ops/ai-models/9/edit");
  stored = cli({
    runtime: "codex", provider_id: "8", cli_options: { format: "openai_responses" },
    connection: { name: "OpenAI Chat", protocol: "openai", base_url: "https://api.openai.com/v1", credential_id: "3", qps: 1, concurrency: 1, version: 1 },
  });
  await act(async () => root.render(<ResourceForm config={config()} workspace="ops" permissions={permissions} />));
  expect(host.querySelector('[role="tab"][aria-selected="true"]')!.textContent).toBe("使用已有 API 模型");
  expect(host.querySelector('input[aria-label="接口地址"]')).toBeNull();
  await submit();
  const body = JSON.parse(requests.find((r) => r.method === "PATCH" || r.method === "PUT")!.body!);
  expect(body).toMatchObject({ provider_id: "8", cli_options: { format: "openai_responses" } });
  expect(body.connection ?? null).toBeNull();
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

it("requires a default model for an API model before it can be saved", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/ops/ai-models/12/edit");
  stored = {
    id: "12", name: "sub2api#codex", kind: "api", runtime: "", provider_id: "8", models: [], default_model: "", tool_version: "",
    state: "enabled", version: 1,
    connection: { name: "sub2api#codex", protocol: "openai", base_url: "https://gw.example.com/v1", credential_id: "3", qps: 35, concurrency: 7, version: 1 },
  };
  await act(async () => root.render(<ResourceForm config={config()} workspace="ops" permissions={permissions} />));
  const field = host.querySelector<HTMLInputElement>('input[aria-label="默认模型"]')!;
  expect(field.closest("label, [data-field], div")!.parentElement!.textContent).toContain("*");
  const save = host.querySelector<HTMLButtonElement>('form button[type="submit"]')!;
  expect(save.disabled).toBe(true);
  await submit();
  expect(requests.some((r) => r.method === "PATCH" || r.method === "PUT")).toBe(false);
  await act(async () => type(field, "gpt-5-codex"));
  expect(save.disabled).toBe(false);
  await submit();
  const body = JSON.parse(requests.find((r) => r.method === "PATCH" || r.method === "PUT")!.body!);
  expect(body.default_model).toBe("gpt-5-codex");
  // An API model runs on the server: it has no time zone to set.
  expect(host.querySelector('input[aria-label="时区"]')).toBeNull();
});

it("lets a CLI model require a time zone for its executions, checked before saving", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/ops/ai-models/9/edit");
  stored = cli({ runtime: "codex" });
  await act(async () => root.render(<ResourceForm config={config()} workspace="ops" permissions={permissions} />));
  const zone = host.querySelector<HTMLInputElement>('input[aria-label="时区"]')!;
  expect(zone.value).toBe("");
  expect(zone.placeholder).toBe("跟随执行器");
  expect(host.textContent).toContain("TZ 变量");
  const save = host.querySelector<HTMLButtonElement>('form button[type="submit"]')!;
  await act(async () => type(zone, "Mars/Olympus"));
  expect(save.disabled).toBe(true);
  await act(async () => type(zone, "Asia/Shanghai"));
  expect(save.disabled).toBe(false);
  await submit();
  const body = JSON.parse(requests.find((r) => r.method === "PATCH" || r.method === "PUT")!.body!);
  expect(body.timezone).toBe("Asia/Shanghai");
});

it("orders a CLI model's execution hosts, empty meaning any ready Runner", async () => {
  const runners = [
    { id: "21", name: "build-a", state: "online" },
    { id: "22", name: "build-b", state: "offline" },
    { id: "23", name: "build-c", state: "online" },
  ];
  const original = vi.mocked(fetch).getMockImplementation()!;
  vi.mocked(fetch).mockImplementation(async (url, init) => {
    if (String(url).includes("/nodes?")) {
      requests.push({ url: String(url), method: init?.method, body: init?.body as string });
      return { ok: true, status: 200, json: async () => ({ items: runners, total: runners.length, page: 1, page_size: 200 }) } as Response;
    }
    return original(url, init);
  });
  history.replaceState({}, "", "/zh-CN/workspaces/ops/ai-models/9/edit");
  stored = cli({ runtime: "codex", hosts: [] });
  await act(async () => root.render(<ResourceForm config={config()} workspace="ops" permissions={permissions} />));
  expect(host.textContent).toContain("自动（任一在线且 CLI 就绪的 Runner）");
  // Only Runner hosts are offered.
  expect(requests.some((r) => r.url.includes("/nodes?") && r.url.includes("connection=runner"))).toBe(true);
  const add = () => host.querySelector<HTMLSelectElement>('select[aria-label="添加主机"]')!;
  const choose = (id: string) => act(async () => {
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(add(), id);
    add().dispatchEvent(new Event("change", { bubbles: true }));
  });
  await choose("23");
  await choose("22");
  const order = () => [...host.querySelectorAll('ol[aria-label="执行主机"] li')].map((li) => li.textContent);
  expect(order()[0]).toContain("build-c");
  expect(order()[1]).toContain("build-b");
  expect(order()[1]).toContain("离线");
  await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="上移 build-b"]')!.click());
  expect(order()[0]).toContain("build-b");
  await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="移除 build-c"]')!.click());
  expect(order()).toHaveLength(1);
  await submit();
  const body = JSON.parse(requests.find((r) => r.method === "PATCH" || r.method === "PUT")!.body!);
  expect(body.hosts).toEqual(["22"]);
});

it("offers execution hosts only on CLI models", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/ops/ai-models/new");
  await act(async () => root.render(<ResourceForm config={config()} workspace="ops" permissions={permissions} />));
  expect(host.textContent).not.toContain("执行主机");
});
