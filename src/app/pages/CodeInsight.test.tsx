import { act,useState } from "react";
import { createRoot,type Root } from "react-dom/client";
import { afterEach,beforeEach,expect,it,vi } from "vitest";
import { i18n,insightText } from "../../shared/i18n/i18n";
import insightMessages from "../../shared/i18n/locales/insight-domain.json";
import insightZh from "../../shared/i18n/locales/insight-zh.json";
import { ResourceForm } from "../resources/ResourceForm";
import { CodeInsightDetail,ConnectionInsights,codeInsightForm,codeInsightList } from "./CodeInsight";
import { CodeInsightAliases,CodeInsightRepositories,type InsightRepository } from "./CodeInsightFields";
import { analysisReport,analysisReports,overviewPeriods,overviewRanking } from "./RepositoryAnalysis";

let host: HTMLDivElement, root: Root;
it("renders every insight message and named parameter in all six catalogs", async () => {
  const params = (text: string) => [...text.matchAll(/{{(\w+)}}/g)].map(match => match[1]).sort();
  for (const [key, texts] of Object.entries(insightMessages)) {
    expect(texts, key).toHaveLength(5);
    const chinese = insightZh[key as keyof typeof insightZh] ?? key;
    for (const text of texts) {
      expect(text.trim(), key).not.toBe("");
      expect(params(text), key).toEqual(params(chinese));
    }
  }
  for (const locale of ["en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "ms-MY"]) {
    await i18n.changeLanguage(locale);
    expect(insightText("jobProgress", { state: "custom", completed: 0, generation: 2 })).toContain("0/2");
    expect(insightText("viewCommit", { sha: "raw-sha" })).toContain("raw-sha");
  }
});
let requests: { url: string; method: string; body?: unknown }[];
const settings = { id: "31", name: "Team insight", description: "", enabled: true, update_mode: "scheduled", schedule: "0 */4 * * *", timezone: "", next_run_at: 1800000000000, full_history: false, initial_days: 90, reconcile_minutes: 1440, aliases: [{ email: "alice@example.invalid", nickname: "Alice", contributor_id: "123" }], repositories: [{ connection_id: "11", connection_name: "Platform source", repository: "team/project", pipeline_id: "" }], version: 5, sources: [] };
beforeEach(() => {
  void i18n.changeLanguage("zh-CN");
  localStorage.setItem("ui.locale", "zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  history.replaceState({}, "", "/workspaces/ws/code-insights/31");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host); requests = [];
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    requests.push({ url, method: init.method ?? "GET", ...(init.body ? { body: JSON.parse(String(init.body)) } : {}) });
    let data: unknown = { items: [], total: 0, page: 1, page_size: 100 };
    if (url === "/api/workspaces/ws/code-insights/31") data = settings;
    else if (url.startsWith("/api/workspaces/ws/code-connections?")) {
      const second = new URL(url, location.origin).searchParams.get("page") === "2";
      data = { items: second ? [{ id: "12", name: "Git source", provider: "git" }] : [{ id: "11", name: "Platform source", provider: "github" }], total: 201, page_size: 200 };
    } else if (url.includes("/code-connections/11/repositories?")) {
      const second = new URL(url, location.origin).searchParams.get("page") === "2";
      data = { items: [{ name: second ? "second" : "project", full_name: second ? "team/second" : "team/project", url: `https://git.example.invalid/team/${second ? "second" : "project"}.git`, default_branch: "main" }], has_more: !second };
    } else if (url.includes("/pipelines?")) data = { items: [{ id: "21", name: "Analysis workflow" }], total: 1 };
    return { ok: true, status: 200, json: async () => data };
  }));
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

const setInput = async (input: HTMLInputElement | HTMLTextAreaElement, value: string) => act(async () => {
  Object.getOwnPropertyDescriptor(input instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype, "value")!.set!.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
});
const choose = async (select: HTMLSelectElement, value: string) => act(async () => { select.value = value; select.dispatchEvent(new Event("change", { bubbles: true })); });

it("scopes all reports and six period rankings to an insight without the legacy connection query", async () => {
  for (const [kind] of analysisReports) {
    const config = analysisReport("ws", "31", kind, "insight");
    expect(config.endpoint).toBe(`/workspaces/ws/code-insights/31/reports/${kind}`);
    expect(config.fixed).toEqual({}); expect(config.permissionResource).toBe("code-insights");
  }
  for (const period of overviewPeriods) expect(overviewRanking("ws", "31", period, new Date(), "insight").endpoint).toBe("/workspaces/ws/code-insights/31/reports/ranking");
  await act(async () => root.render(<CodeInsightDetail workspace="ws" insightId="31" permissions={new Set(["code-insights.read"])} />));
  expect(host.querySelector('[aria-label="六个周期贡献排名"]')).not.toBeNull();
  expect(requests.filter(request => request.url.includes("/reports/ranking?")).length).toBeGreaterThanOrEqual(6);
  expect(requests.some(request => request.url.includes("connection_id=") || request.url.includes("code-connection-analyses"))).toBe(false);
});

it("keeps related insights in the shared list with an explicit connection filter", async () => {
  expect(codeInsightList("ws", "11").fixed).toEqual({ connection_id: "11" });
  await act(async () => root.render(<ConnectionInsights workspace="ws" connectionId="11" permissions={new Set(["repositories.read"])} />));
  expect(requests).toHaveLength(0);
  expect(host.textContent).toContain("需要代码洞察读取权限");
  await act(async () => root.render(<ConnectionInsights workspace="ws" connectionId="11" permissions={new Set(["code-insights.read"])} />));
  expect(requests.some(request => request.url.includes("/code-insights?") && request.url.includes("connection_id=11"))).toBe(true);
});

it("adds repositories from paginated named connections, remote pages and explicit Git URLs", async () => {
  let selected: InsightRepository[] = [];
  function Editor() {
    const [value, setValue] = useState<InsightRepository[]>([]); selected = value;
    return <CodeInsightRepositories workspace="ws" value={value} readOnly={false} permissions={new Set(["repositories.read", "pipelines.read", "pipelines.run"])} onChange={next => setValue(next as InsightRepository[])} onValidityChange={() => {}} />;
  }
  await act(async () => root.render(<Editor />));
  const connectionSelect = host.querySelector<HTMLSelectElement>('select[aria-label="仓库连接"]')!;
  expect(connectionSelect.textContent).toContain("Git source");
  await choose(connectionSelect, "11");
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "加载更多仓库")!.click());
  await choose(host.querySelector<HTMLSelectElement>('select[aria-label="添加洞察仓库"]')!, "https://git.example.invalid/team/second.git");
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "加入洞察")!.click());
  expect(selected).toEqual([{ connection_id: "11", repository: "team/second", pipeline_id: "" }]);
  await choose(host.querySelector<HTMLSelectElement>('select[aria-label="分析流水线 team/second"]')!, "21");
  await choose(connectionSelect, "12");
  await setInput(host.querySelector<HTMLInputElement>('input[aria-label="Git 仓库地址"]')!, "git@git.example.invalid:other/repo.git");
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "加入洞察")!.click());
  expect(selected).toEqual([{ connection_id: "11", repository: "team/second", pipeline_id: "21" }, { connection_id: "12", repository: "git@git.example.invalid:other/repo.git", pipeline_id: "" }]);
  expect(requests.filter(request => request.url.includes("code-connections?")).length).toBe(2);
});

it("edits author mappings without JSON and preserves contributor IDs across locales", async () => {
  let value: unknown = settings.aliases, blocked = false;
  function Editor() {
    const [aliases, setAliases] = useState<unknown>(settings.aliases);
    return <CodeInsightAliases value={aliases} readOnly={false} onChange={next => { value = next; setAliases(next); }} onValidityChange={next => { blocked = next; }} />;
  }
  await act(async () => root.render(<Editor />));
  expect(host.querySelector('textarea')).toBeNull();
  const input = host.querySelector<HTMLInputElement>('input[aria-label="统一昵称 1"]')!;
  await setInput(input, ""); expect(blocked).toBe(true); expect(host.querySelector('[role="alert"]')).not.toBeNull();
  for (const locale of ["en-US", "ja-JP", "zh-CN"]) {
    await act(async () => { await i18n.changeLanguage(locale); });
    expect(input.value).toBe("");
    expect(host.querySelector('[role="alert"]')?.textContent).toBe(insightText("每条作者映射须填写姓名或邮箱，以及显示昵称。"));
    expect(blocked).toBe(true);
  }
  await setInput(input, 'New name');
  expect(blocked).toBe(false); expect(value).toEqual([{ email: "alice@example.invalid", nickname: "New name", contributor_id: "123" }]);
});

it("requires a pipeline for enabled Git memberships and permits saving disabled ones", async () => {
  let blocked = false;
  function Editor({ enabled }: { enabled: boolean }) {
    const [value, setValue] = useState<InsightRepository[]>([{ connection_id: "12", repository: "git@git.example.invalid:team/repo.git", pipeline_id: "", provider: "git" }]);
    return <CodeInsightRepositories workspace="ws" value={value} enabled={enabled} readOnly={false} permissions={new Set(["repositories.read", "pipelines.read", "pipelines.run"])} onChange={next => setValue(next as InsightRepository[])} onValidityChange={next => { blocked = next; }} />;
  }
  await act(async () => root.render(<Editor enabled />));
  expect(blocked).toBe(true);
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("请为此 Git 仓库选择分析流水线");
  const pipeline = host.querySelector<HTMLSelectElement>('select[aria-label="分析流水线 git@git.example.invalid:team/repo.git"]')!;
  expect(pipeline.required).toBe(true);
  await choose(pipeline, "21"); expect(blocked).toBe(false);
  await choose(pipeline, ""); expect(blocked).toBe(true);
  await act(async () => root.render(<Editor enabled={false} />));
  expect(blocked).toBe(false); expect(pipeline.required).toBe(false);
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("saves an optimistic flat insight through the shared edit form without pipeline permission fetches", async () => {
  history.replaceState({}, "", "/workspaces/ws/code-insights/31/edit");
  await act(async () => root.render(<ResourceForm config={codeInsightForm("ws")} workspace="ws" permissions={new Set(["code-insights.read", "code-insights.update"])} />));
  expect(host.textContent).toContain("Platform source");
  const name = host.querySelector<HTMLInputElement>('input[type="text"]')!;
  await setInput(name, "Renamed insight");
  await act(async () => host.querySelector<HTMLFormElement>("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  const saved = requests.find(request => request.method === "PATCH");
  expect(saved).toMatchObject({ url: "/api/workspaces/ws/code-insights/31", body: { name: "Renamed insight", version: 5, update_mode: "scheduled", schedule: "0 */4 * * *", timezone: "", aliases: settings.aliases, repositories: settings.repositories } });
  expect(requests.some(request => request.url.includes("pipelines?") || request.url.includes("code-connections?"))).toBe(false);
});

it("defaults new insights to four-hour schedules and switches event reconciliation independently", async () => {
  const config = codeInsightForm("ws");
  expect(config.blank).toMatchObject({ update_mode: "scheduled", schedule: "0 */4 * * *", timezone: "", reconcile_minutes: 1440 });
  history.replaceState({}, "", "/workspaces/ws/code-insights/31/edit");
  await act(async () => root.render(<ResourceForm config={config} workspace="ws" permissions={new Set(["code-insights.read", "code-insights.update"])} />));
  const mode = host.querySelector<HTMLSelectElement>('select[name="update_mode"]')!;
  expect(mode.value).toBe("scheduled");
  expect(host.querySelector('input[aria-label="Cron 表达式"]')?.getAttribute("value")).toBe("0 */4 * * *");
  expect(host.querySelector('input[name="timezone"]')).not.toBeNull();
  expect(host.querySelector('input[name="reconcile_minutes"]')).toBeNull();
  await choose(mode, "event");
  expect(host.querySelector('input[aria-label="Cron 表达式"]')).toBeNull();
  expect(host.querySelector('input[name="timezone"]')).toBeNull();
  expect(host.querySelector('input[name="reconcile_minutes"]')).not.toBeNull();
  await choose(mode, "scheduled");
  expect(host.querySelector('input[aria-label="Cron 表达式"]')).not.toBeNull();
  expect(host.querySelector('input[name="reconcile_minutes"]')).toBeNull();
  await choose(mode, "event");
  await act(async () => host.querySelector<HTMLFormElement>("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(requests.find(request => request.method === "PATCH")?.body).toMatchObject({ update_mode: "event", schedule: "0 */4 * * *", timezone: "", reconcile_minutes: 1440, version: 5, aliases: settings.aliases, repositories: settings.repositories });
});

it("allows API-backed repository URLs without requiring a Git analysis pipeline", async () => {
  let blocked = true;
  await act(async () => root.render(<CodeInsightRepositories workspace="ws"
    value={[{ connection_id: "11", provider: "github", repository: "https://git.example.invalid/team/project.git", pipeline_id: "" }]}
    readOnly={false} permissions={new Set(["repositories.read"])} onChange={() => {}}
    onValidityChange={value => { blocked = value; }} />));
  expect(blocked).toBe(false);
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("keeps author mapping out of the insight creation form", () => {
  const config = codeInsightForm("ws");
  expect(config.sections.flatMap(section => section.fields).some(field => field.key === "aliases")).toBe(false);
  expect(config.blank.aliases).toEqual([]);
});

it("does not leave Git selection disabled when a previous platform request is still pending", async () => {
  const originalFetch = fetch;
  vi.stubGlobal("fetch", vi.fn((url: string, init?: RequestInit) => url.includes('/code-connections/11/repositories?') ? new Promise(() => {}) : originalFetch(url, init)));
  await act(async () => root.render(<CodeInsightRepositories workspace="ws" value={[]} readOnly={false} permissions={new Set(["repositories.read"])} onChange={() => {}} onValidityChange={() => {}} />));
  const connection = host.querySelector<HTMLSelectElement>('select[aria-label="仓库连接"]')!;
  await choose(connection, '11');
  expect(host.querySelector<HTMLSelectElement>('select[aria-label="添加洞察仓库"]')?.disabled).toBe(true);
  await choose(connection, '12');
  await setInput(host.querySelector<HTMLInputElement>('input[aria-label="Git 仓库地址"]')!, 'https://git.example.invalid/team/repo.git');
  expect([...host.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent?.includes('加入洞察'))!.disabled).toBe(false);
});
