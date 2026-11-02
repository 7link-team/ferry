import { act } from "react";
import { createRoot,type Root } from "react-dom/client";
import { afterEach,beforeEach,expect,it,vi } from "vitest";
import { i18n } from "../../shared/i18n/i18n";
import { AnalysisSummary,RepositoryAnalysis,analysisConfig,analysisDateRange,analysisReport,analysisReports,overviewPeriods,overviewRanking } from "./RepositoryAnalysis";

let root: Root;
let host: HTMLDivElement;
let requests: { url: string; method: string; body?: Record<string, unknown> }[];
const settings = { id: "11", enabled: true, all_repositories: true, repositories: [], full_history: false, initial_days: 90, reconcile_minutes: 1440, pipeline_id: "", aliases: [], version: 7, coverage_start: 0, last_error: "", sources: [{ repository: "team/repo", state: "partial", coverage_start: 0, synced_at: 0, last_error: "差异采集未完成", job: { state: "queued", generation: 2, completed_generation: 1 } }] };
beforeEach(() => {
  void i18n.changeLanguage("zh-CN");
  localStorage.setItem("ui.locale", "zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  history.replaceState({}, "", "/workspaces/ws/code-connections/11?tab=analysis");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host); requests = [];
  vi.stubGlobal("fetch", vi.fn(async (url: string, options: RequestInit = {}) => {
    requests.push({ url, method: options.method ?? "GET", body: options.body ? JSON.parse(String(options.body)) : undefined });
    const data = url.includes("code-connection-analyses") ? settings : { items: [], total: 0, page: 1, page_size: 20 };
    return { ok: true, status: 200, json: async () => data };
  }));
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

it("saves nested optimistic configuration and normalizes repository lines and aliases", async () => {
  const config = analysisConfig("ws", "11");
  await config.viewActions![0]!.action!(settings, { ...settings, repositories: " team/repo\n\nsecond/repo ", aliases: '[{"email":"alice@example.com","nickname":"Alice"}]' });
  expect(requests).toEqual([{ url: "/api/workspaces/ws/code-connections/11/analysis", method: "PATCH", body: { version: 7, config: { enabled: true, all_repositories: true, repositories: ["team/repo", "second/repo"], full_history: false, initial_days: 90, reconcile_minutes: 1440, pipeline_id: "", aliases: [{ name: "", email: "alice@example.com", nickname: "Alice" }] } } }]);
});

it("rejects malformed or incomplete nickname mappings without writing", async () => {
  const save = analysisConfig("ws", "11").viewActions![0]!.action!;
  await expect(save(settings, { aliases: "oops" })).rejects.toThrow("JSON");
  await expect(save(settings, { aliases: '[{"nickname":"Alice"}]' })).rejects.toThrow("姓名或邮箱");
  expect(requests).toHaveLength(0);
});

it("preserves stable contributor IDs when editing alias display names", async () => {
  await analysisConfig("ws", "11").viewActions![0]!.action!(settings, {
    ...settings, aliases: '[{"email":"alice@example.com","nickname":"New name","contributor_id":"123"}]',
  });
  expect(requests[0]!.body).toMatchObject({ config: { aliases: [{
    email: "alice@example.com", nickname: "New name", contributor_id: "123",
  }] } });
});

it("queues retry through the connection's analysis-runs collection", async () => {
  await analysisConfig("ws", "11").viewActions![1]!.action!(settings);
  expect(requests[0]).toMatchObject({ method: "POST", url: "/api/workspaces/ws/code-connections/11/analysis-runs" });
});

it("exposes every report with workspace and connection scoping and all filters", () => {
  for (const [kind] of analysisReports) {
    const report = analysisReport("ws", "11", kind);
    expect(report.endpoint).toBe(`/workspaces/ws/code-contributions/${kind}`);
    expect(report.fixed).toEqual({ connection_id: "11" });
    expect(report.filters!.map(filter => filter.key)).toEqual(expect.arrayContaining(["repos", "authors", "branches", "since", "until", "isMerge", "history", "messageType", "commitHash", "geEffective", "leEffective"]));
    expect(report.toolbar).toMatchObject({ create: false, import: false });
  }
  expect(analysisReport("ws", "11", "period").filters!.find(filter => filter.key === "period")!.options).toHaveLength(3);
});

it("renders source coverage errors and switches the real shared report table", async () => {
  await act(async () => root.render(<RepositoryAnalysis workspace="ws" connectionId="11" permissions={new Set(["repositories.read", "repositories.update"])} />));
  expect(requests.some(request => request.url.includes("/code-connection-analyses/11"))).toBe(true);
  expect(host.textContent).toContain("差异采集未完成");
  expect(host.textContent).toContain("进度 1/2");
  expect(host.textContent).toContain("完整历史");
  expect(requests.some(request => request.url.includes("/pipelines"))).toBe(false);
  expect(host.querySelector('[aria-label="六个周期贡献排名"]')).not.toBeNull();
  expect(requests.filter(request => request.url.includes("/code-contributions/ranking?")).length).toBeGreaterThanOrEqual(6);
  await act(async () => [...host.querySelectorAll("button")].find(button => button.textContent === "提交明细")!.click());
  expect(requests.some(request => request.url.includes("/code-contributions/commits?") && request.url.includes("connection_id=11"))).toBe(true);
  expect(host.textContent).toContain("语言变更");
});

it("renders all filtered heatmap dates independently of a paginated table page", async () => {
  const data = { items: [{ id: "page-row" }], total: 3, page: 2, page_size: 1, coverage: { complete: false, start: 0, sources: [] }, series: [{ period: "2026-01-01", commits: 2 }, { period: "2026-01-03", commits: 5 }] };
  await act(async () => root.render(<AnalysisSummary data={data} kind="heatmap" />));
  expect(host.querySelectorAll("svg rect")).toHaveLength(6);
  expect(host.querySelector("svg")!.textContent).toContain("2026-01-02：0 次提交");
  expect(host.querySelector("svg")!.textContent).toContain("2026-01-03：5 次提交");
  expect(host.textContent).toContain("覆盖尚未完成");
});

it("keeps each selected author in a separate trend series with an accessible legend and exact values", async () => {
  const data = { items: [], total: 0, page: 1, page_size: 20, series: [{ period: "2026-01", nickname: "A", effectives: 4, commits: 2 }, { period: "2026-01", nickname: "B", effectives: 6, commits: 1 }, { period: "2026-02", nickname: "A", effectives: 5, commits: 2 }] };
  await act(async () => root.render(<AnalysisSummary data={data} kind="period" />));
  expect(host.querySelectorAll("svg circle")).toHaveLength(4);
  expect(host.querySelectorAll("svg polyline")).toHaveLength(2);
  expect(host.querySelector('[data-author="A"]')!.textContent).toContain("A · 2026-01：4 有效变更，2 次提交");
  expect(host.querySelector('[data-author="B"]')!.textContent).toContain("B · 2026-01：6 有效变更，1 次提交");
  expect(host.querySelector('[aria-label="趋势作者图例"]')!.textContent).toBe("AB");
});

it("computes UTC calendar presets across month and year boundaries with Monday weeks", () => {
  const now = new Date("2026-03-01T23:30:00Z");
  expect(analysisDateRange("today", now)).toEqual({ since: "2026-03-01", until: "2026-03-01" });
  expect(analysisDateRange("yesterday", now)).toEqual({ since: "2026-02-28", until: "2026-02-28" });
  expect(analysisDateRange("this-week", now)).toEqual({ since: "2026-02-23", until: "2026-03-01" });
  expect(analysisDateRange("last-week", now)).toEqual({ since: "2026-02-16", until: "2026-02-22" });
  expect(analysisDateRange("two-weeks-ago", now)).toEqual({ since: "2026-02-09", until: "2026-02-15" });
  expect(analysisDateRange("this-month", now)).toEqual({ since: "2026-03-01", until: "2026-03-31" });
  expect(analysisDateRange("last-month", now)).toEqual({ since: "2026-02-01", until: "2026-02-28" });
  expect(analysisDateRange("two-months-ago", now)).toEqual({ since: "2026-01-01", until: "2026-01-31" });
  expect(analysisDateRange("last-month", new Date("2024-03-02T00:00:00Z"))).toEqual({ since: "2024-02-01", until: "2024-02-29" });
  expect(analysisDateRange("last-month", new Date("2026-01-02T00:00:00Z"))).toEqual({ since: "2025-12-01", until: "2025-12-31" });
  expect(analysisDateRange("default", now)).toEqual({ since: "", until: "" });
  expect(analysisDateRange("all", now)).toEqual({ since: "", until: "" });
});

it("uses stable author identities so authors sharing a display name remain separate", async () => {
  const data = { items: [], total: 0, page: 1, page_size: 20, series: [
    { period: "2026-01", authorKey: "first", nickname: "Alice", email: "first@example.invalid", effectives: 4 },
    { period: "2026-01", authorKey: "second", nickname: "Alice", email: "second@example.invalid", effectives: 6 },
    { period: "2026-02", authorKey: "first", nickname: "Alice", email: "first@example.invalid", effectives: 5 },
  ] };
  await act(async () => root.render(<AnalysisSummary data={data} kind="period" />));
  expect(host.querySelectorAll("[data-author-key]")).toHaveLength(2);
  expect(host.querySelector('[aria-label="趋势作者图例"]')!.textContent).toContain("Alice（first@example.invalid）");
  expect(host.querySelector('[aria-label="趋势作者图例"]')!.textContent).toContain("Alice（second@example.invalid）");
});

it("gives all six overview rankings isolated query state and fixed UTC periods", async () => {
  expect(overviewPeriods).toHaveLength(6);
  for (const period of overviewPeriods) {
    const config = overviewRanking("ws", "11", period, new Date("2026-03-01T00:00:00Z"));
    expect(config.endpoint).toBe("/workspaces/ws/code-contributions/ranking");
    expect(config.queryPrefix).toBe(`overview.${period}`);
    expect(config.fixed).toEqual({ connection_id: "11", ...analysisDateRange(period, new Date("2026-03-01T00:00:00Z")) });
    expect(config.filters!.some(filter => filter.key === "since" || filter.key === "until")).toBe(false);
  }
  history.replaceState({}, "", "/workspaces/ws/code-connections/11?tab=analysis&overview.today.page=2&overview.yesterday.page=3");
  await act(async () => root.render(<RepositoryAnalysis workspace="ws" connectionId="11" permissions={new Set(["repositories.read"])} />));
  const rankingRequests = requests.filter(request => request.url.includes("/code-contributions/ranking?")).map(request => new URL(request.url, location.origin).searchParams);
  expect(rankingRequests.filter(params => params.get("page") === "2")).toHaveLength(1);
  expect(rankingRequests.filter(params => params.get("page") === "3")).toHaveLength(1);
});

it("commits heatmap shortcuts to both shared filters and the URL without losing other conditions", async () => {
  history.replaceState({}, "", "/workspaces/ws/code-connections/11?tab=analysis&analysis_report=heatmap&authors=Alice&since=2026-01-01");
  await act(async () => root.render(<RepositoryAnalysis workspace="ws" connectionId="11" permissions={new Set(["repositories.read"])} />));
  const click = async (label: string) => {
    const buttons = [...host.querySelectorAll<HTMLButtonElement>("button")];
    expect(buttons.map(button => button.textContent).join(" | ")).toContain(label);
    await act(async () => buttons.find(button => button.textContent === label)!.click());
  };
  await click("合并 merge");
  let params = new URL(requests.at(-1)!.url, location.origin).searchParams;
  expect(params.get("isMerge")).toBe("1"); expect(params.get("authors")).toBe("Alice");
  await click("功能 feat");
  params = new URL(requests.at(-1)!.url, location.origin).searchParams;
  expect(params.get("messageType")).toBe("feat"); expect(params.get("isMerge")).toBe("0");
  await click("全部提交");
  params = new URL(requests.at(-1)!.url, location.origin).searchParams;
  expect(params.get("messageType")).toBeNull(); expect(params.get("isMerge")).toBe("0,1");
  await click("全部");
  expect(new URLSearchParams(location.search).get("since")).toBeNull();
  expect(new URLSearchParams(location.search).get("authors")).toBe("Alice");
});

it("keeps pipeline configuration readable without pipeline permissions and loads options only when allowed", async () => {
  const limited = analysisConfig("ws", "11", new Set(["repositories.read", "repositories.update"]));
  const field = limited.sections[0]!.fields.find(field => field.key === "pipeline_id")!;
  expect(field.type).toBe("text"); expect(typeof field.options).not.toBe("function");
  expect(limited.viewActions![0]!.parameters!(settings).some(field => field.key === "pipeline_id")).toBe(false);
  const allowed = analysisConfig("ws", "11", new Set(["pipelines.read", "pipelines.run"]));
  const pipelineField = allowed.sections[0]!.fields.find(field => field.key === "pipeline_id")!;
  expect(typeof pipelineField.options).toBe("function");
  await (pipelineField.options as () => Promise<unknown>)();
  expect(requests[0]!.url).toContain("/pipelines?");
});

it("links source commits only to safe URLs supplied by the platform", async () => {
  const column = analysisReport("ws", "11", "commits").columns.find(column => column.key === "commitHash")!;
  await act(async () => root.render(<>{column.render!({ commitHash: "abc", commitUrl: "https://git.example/team/repo/commit/abc" })}</>));
  expect(host.querySelector("a")!.getAttribute("href")).toBe("https://git.example/team/repo/commit/abc");
  expect(host.querySelector("a")!.getAttribute("rel")).toBe("noopener noreferrer");
  await act(async () => root.render(<>{column.render!({ commitHash: "abc", commitUrl: "javascript:alert(1)" })}</>));
  expect(host.querySelector("a")).toBeNull(); expect(host.textContent).toBe("abc");
});
