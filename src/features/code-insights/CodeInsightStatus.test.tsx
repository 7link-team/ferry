import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { i18n } from "../../shared/i18n/i18n";
import { CodeInsightStatus, sourceTaskState } from "./CodeInsightStatus";
import { time, type Row } from "./config";

const endpoint = "/api/workspaces/ws/code-insights/31";
const checkedAt = Date.parse("2026-11-03T09:00:00Z");
const successAt = checkedAt - 3_600_000;
const nextAt = checkedAt + 14_400_000;
let host: HTMLDivElement;
let root: Root;
let detail: Row;
let requests: { url: string; method: string; body?: string }[];
let readFailure: string;
let retryFailure: string;
let onConfigure: ReturnType<typeof vi.fn<() => void>>;
let onPublished: ReturnType<typeof vi.fn<() => void>>;

beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  localStorage.setItem("ui.locale", "zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.useFakeTimers();
  vi.setSystemTime(checkedAt);
  vi.spyOn(document, "hidden", "get").mockReturnValue(false);
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  requests = [];
  readFailure = "";
  retryFailure = "";
  onConfigure = vi.fn();
  onPublished = vi.fn();
  detail = {
    id: "31", name: "Team insight", enabled: true, update_mode: "scheduled",
    next_run_at: nextAt, sources: [],
  };
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    const method = init.method ?? "GET";
    requests.push({ url, method, ...(init.body ? { body: String(init.body) } : {}) });
    const failure = method === "POST" ? retryFailure : readFailure;
    return {
      ok: !failure, status: failure ? 503 : 200,
      json: async () => failure ? { error: failure } : method === "POST" ? { scheduled: true } : detail,
    };
  }));
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

async function render(permissions: string[] = ["code-insights.read"]) {
  await act(async () => root.render(<CodeInsightStatus workspace="ws" insightId="31" permissions={new Set(permissions)} onConfigure={onConfigure} onPublished={onPublished} />));
}

const status = () => host.querySelector('[role="status"]')!.textContent!;
const button = (name: string) => [...host.querySelectorAll("button")].find(item => item.textContent?.includes(name));
const metric = (label: string) => [...host.querySelectorAll("dt")].find(item => item.textContent === label)?.nextElementSibling?.textContent;

it("keeps source diagnostics collapsed by default while showing the aggregate failure state", async () => {
  detail.sources = [{ repository: "team/new", state: "unavailable", last_error: "Source discovery has not completed", synced_at: 0 }];
  await render();
  expect(status()).toContain("分析尚未完成");
  expect(host.textContent).toContain("等待发现或恢复访问");
  expect(host.textContent).toContain("Source discovery has not completed");
  expect(host.querySelector("details")!.open).toBe(false);
  expect(host.textContent).not.toContain("unavailable");
});

it("shows a countdown, pauses polling, preserves expanded diagnostics and updates the repository total", async () => {
  detail.repository_count = 8;
  detail.sources = [{ repository: "team/project", state: "complete", synced_at: successAt }];
  await render();
  expect(status()).toContain("8 仓库");
  expect(host.textContent).toContain("15 秒后刷新");
  const diagnostics = host.querySelector("details")!;
  diagnostics.open = true;
  await act(async () => vi.advanceTimersByTimeAsync(1000));
  expect(host.textContent).toContain("14 秒后刷新");
  await act(async () => button("定时刷新")!.click());
  const paused = requests.length;
  await act(async () => vi.advanceTimersByTimeAsync(45000));
  expect(requests).toHaveLength(paused);
  detail = { ...detail, repository_count: 12, sources: [{ repository: "team/project", state: "complete", synced_at: checkedAt + 45000 }] };
  await act(async () => button("定时刷新")!.click());
  expect(status()).toContain("12 仓库");
  expect(onPublished).toHaveBeenCalledOnce();
  expect(host.querySelector("details")).toBe(diagnostics);
  expect(diagnostics.open).toBe(true);
  expect(requests.every(request => request.method === "GET")).toBe(true);
});

it("shows legacy blocked diagnostics without requiring a business pipeline or inventing progress", async () => {
  detail.sources = [{ repository: "team/project", state: "queued", pipeline_required: true, synced_at: 0, job: { state: "queued", effective_state: "blocked", generation: 10, completed_generation: 0 } }];
  await render();
  expect(status()).toContain("分析尚未完成");
  expect(host.textContent).toContain("采集暂不可用");
  expect(host.textContent).not.toContain("流水线");
  expect(host.textContent).not.toContain("0/10");
  expect(host.textContent).not.toContain("已排队");
  expect(host.querySelector(".animate-spin")).toBeNull();
  expect(metric("最近成功分析")).toBe("尚未成功");
  expect(sourceTaskState({ pipeline_required: true, state: "queued" })).toBe("queued");
  expect(sourceTaskState({ pipeline_required: true, job: { effective_state: "running" } })).toBe("running");
  await act(async () => button("配置与同步")!.click());
  expect(onConfigure).toHaveBeenCalledOnce();
});

it.each([
  ["api", "平台接口采集"],
  ["git_fetch", "获取仓库变更"],
  ["git_analyze", "分析提交信息"],
])("shows the actual %s collector phase only while running", async (phase, label) => {
  detail.sources = [{ repository: "team/project", pipeline_required: true, job: { state: "queued", effective_state: "running", collector_phase: phase } }];
  await render();
  expect(host.querySelector("li")!.textContent).toContain(label);
  expect(status()).toContain("同步中");
  detail.sources = [{ repository: "team/project", job: { effective_state: "retry_wait", collector_phase: phase } }];
  await act(async () => vi.advanceTimersByTimeAsync(15_000));
  expect(host.querySelector("li")!.textContent).not.toContain(label);
  expect(status()).toContain("等待重试");
});

it.each([
  ["zh-CN", ["平台接口采集", "获取仓库变更", "分析提交信息"]],
  ["en-US", ["Collecting from provider API", "Fetching repository changes", "Analyzing commits"]],
  ["zh-TW", ["平台介面採集", "取得倉庫變更", "分析提交資訊"]],
  ["ja-JP", ["プラットフォーム API から収集中", "リポジトリの変更を取得中", "コミットを分析中"]],
  ["ko-KR", ["플랫폼 API에서 수집 중", "저장소 변경 사항 가져오는 중", "커밋 분석 중"]],
  ["ms-MY", ["Mengumpul daripada API platform", "Mendapatkan perubahan repositori", "Menganalisis komit"]],
])("localizes collection phases in %s", async (locale, labels) => {
  await i18n.changeLanguage(locale);
  detail.sources = ["api", "git_fetch", "git_analyze"].map(phase => ({ repository: phase, job: { effective_state: "running", collector_phase: phase } }));
  await render();
  for (const label of labels) expect(host.textContent).toContain(label);
});

it.each([
  ["queued", "已排队"],
  ["running", "同步中"],
  ["retry_wait", "等待重试"],
])("shows truthful %s collection state and the retry time where relevant", async (state, label) => {
  detail.sources = [{ repository: "team/project", synced_at: successAt, job: { state: "queued", effective_state: state, updated_at: checkedAt - 1000, retry_at: checkedAt + 60_000, generation: 10, completed_generation: 0 } }];
  await render();
  expect(status()).toContain(label);
  expect(host.querySelector("li")!.textContent).toContain(label);
  expect(host.textContent).not.toContain("0/10");
  expect(!!host.querySelector(".animate-spin")).toBe(state === "running");
  expect(host.querySelector("li")!.textContent?.includes("下次重试")).toBe(state === "retry_wait");
  if (state === "retry_wait") expect(host.querySelector("li")!.textContent).toContain(time(checkedAt + 60_000));
});

it("separates the latest successful collection, next trigger, and actual status check time", async () => {
  detail.sources = [{ repository: "team/older", state: "complete", synced_at: successAt - 60_000 }, { repository: "team/newer", state: "complete", synced_at: successAt }];
  await render();
  expect(status()).toContain("同步完成");
  expect(metric("最近成功分析")).toBe(time(successAt));
  expect(metric("下次计划更新")).toBe(time(nextAt));
  expect(metric("状态检查时间")).toBe(time(checkedAt));
  expect(button("重新核对")).toBeUndefined();
  expect(onPublished).not.toHaveBeenCalled();
  expect(requests).toEqual([{ url: endpoint, method: "GET" }]);
});

it("explicit retry requires permission and polling refreshes published reports without scheduling more work", async () => {
  detail.sources = [{ repository: "team/project", state: "queued", synced_at: successAt }];
  await render(["code-insights.read", "code-insights.sync"]);
  await act(async () => button("重新核对")!.click());
  expect(requests.filter(item => item.method === "POST")).toEqual([{ url: `${endpoint}/sync`, method: "POST", body: "{}" }]);
  expect(onPublished).not.toHaveBeenCalled();
  detail = { ...detail, sources: [{ repository: "team/project", state: "running", synced_at: successAt }] };
  await act(async () => vi.advanceTimersByTimeAsync(15_000));
  expect(host.querySelector("li")!.textContent).toContain("同步中");
  expect(onPublished).not.toHaveBeenCalled();
  detail = { ...detail, sources: [{ repository: "team/project", state: "complete", synced_at: checkedAt + 25_000 }] };
  await act(async () => vi.advanceTimersByTimeAsync(15_000));
  expect(status()).toContain("同步完成");
  expect(onPublished).toHaveBeenCalledOnce();
  expect(metric("状态检查时间")).toBe(time(checkedAt + 30_000));
  await act(async () => vi.advanceTimersByTimeAsync(15_000));
  expect(onPublished).toHaveBeenCalledOnce();
  expect(requests.filter(item => item.method === "POST")).toHaveLength(1);
  const beforeUnmount = requests.length;
  await act(async () => root.render(null));
  await act(async () => vi.advanceTimersByTimeAsync(60_000));
  expect(requests).toHaveLength(beforeUnmount);
});

it("shows API and collection failures explicitly while keeping the last successful timestamp", async () => {
  detail.last_error = "Collection cannot access the repository";
  detail.sources = [{ repository: "team/project", state: "failed", synced_at: successAt, job: { effective_state: "failed", last_error: "Git authentication failed" } }];
  await render(["code-insights.read", "code-insights.sync"]);
  expect(status()).toContain("分析尚未完成");
  expect(host.querySelector('[role="alert"]')!.textContent).toBe(detail.last_error);
  expect(host.querySelector("li")!.textContent).toContain("Git authentication failed");
  expect(metric("最近成功分析")).toBe(time(successAt));
  retryFailure = "Retry temporarily unavailable";
  await act(async () => button("重新核对")!.click());
  expect([...host.querySelectorAll('[role="alert"]')].map(item => item.textContent)).toContain(retryFailure);
  expect(button("重新核对")!.disabled).toBe(false);
  readFailure = "Status temporarily unavailable";
  await act(async () => vi.advanceTimersByTimeAsync(15_000));
  expect([...host.querySelectorAll('[role="alert"]')].map(item => item.textContent)).toContain(readFailure);
  expect(onPublished).not.toHaveBeenCalled();
});

it("shows paused and event-triggered scheduling without presenting a next run as a completion estimate", async () => {
  detail.enabled = false;
  detail.sources = [{ repository: "team/project", state: "complete", synced_at: successAt }];
  await render();
  expect(status()).toContain("分析已暂停");
  expect(metric("下次计划更新")).toBe("分析已暂停");
  expect(metric("最近成功分析")).toBe(time(successAt));
  detail = { ...detail, enabled: true, update_mode: "event", next_run_at: 0 };
  await act(async () => vi.advanceTimersByTimeAsync(15_000));
  expect(metric("下次计划更新")).toBe("仓库事件触发");
  expect(requests.every(item => item.method === "GET")).toBe(true);
});
