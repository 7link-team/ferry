import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { PipelineRunDetail } from "./PipelineRunDetail";

let root: Root;
let host: HTMLDivElement;
let state: string;
let logs: { seq: number; step_index: number; stream: string; chunk: string }[];
let requests: string[];
let signals: AbortSignal[];
let fetchMock: ReturnType<typeof vi.fn>;
const detail = () => ({ id: "1", number: 1, state, pipeline_id: "2", host_id: "3", triggered_by: "4",
  started_at: 1, finished_at: state === "running" ? 0 : 5000, commit: "abc",
  steps: [{ index: 0, name: "build", state: state === "failed" ? "timed_out" : state, exit_code: null, started_at: 1, finished_at: 5000 }] });
const chunk = (seq: number) => ({ seq, step_index: 0, stream: "stdout", chunk: `log-${seq}\n` });
const flush = async () => { await act(async () => { await Promise.resolve(); }); };
const tick = async (ms = 1000) => { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); };
const visibility = async (hidden: boolean) => {
  Object.defineProperty(document, "hidden", { configurable: true, value: hidden });
  await act(async () => { document.dispatchEvent(new Event("visibilitychange")); });
};
const mount = async (permissions = new Set<string>()) => {
  await act(async () => { root.render(<PipelineRunDetail workspace="ops" runId="1" permissions={permissions} />); });
};
beforeEach(() => {
  vi.useFakeTimers();
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  Object.defineProperty(document, "hidden", { configurable: true, value: false });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  state = "running"; logs = [chunk(1)]; requests = []; signals = [];
  fetchMock = vi.fn(async (url: string, init: RequestInit) => {
    requests.push(url); signals.push(init.signal as AbortSignal);
    const after = Number(new URL(url, "http://local").searchParams.get("after"));
    const items = logs.filter((l) => l.seq > after).slice(0, 200);
    const value = url.includes("/logs?") ? { items, next_seq: items.at(-1)?.seq ?? after } : detail();
    return { ok: true, status: 200, json: async () => value };
  });
  vi.stubGlobal("fetch", fetchMock);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks();
});
it.each(["succeeded", "failed", "cancelled"])("终态 %s 排空220片后停止轮询（变异门槛）", async (terminal) => {
  await mount();
  const firstNode = host.querySelector('[data-seq="1"]');
  logs = Array.from({ length: 220 }, (_, i) => chunk(i + 1)); state = terminal;
  await tick();
  expect(host.querySelectorAll("[data-seq]")).toHaveLength(220);
  expect(host.querySelector('[data-seq="1"]')).toBe(firstNode);
  expect(requests.filter((s) => s.includes("logs?"))).toEqual([
    "/api/workspaces/ops/pipeline-runs/1/logs?after=0",
    "/api/workspaces/ops/pipeline-runs/1/logs?after=1",
    "/api/workspaces/ops/pipeline-runs/1/logs?after=201",
  ]);
  const count = requests.length;
  await tick(5000); await visibility(true); await visibility(false); await tick(2000);
  expect(requests).toHaveLength(count);
  expect(host.textContent).toContain("轮询已停止");
  if (terminal === "failed") expect(host.textContent).toContain("超时");
});
it("隐藏暂停、恢复立即拉取，不重复追加", async () => {
  await mount(); const count = requests.length;
  await visibility(true); await tick(5000); expect(requests).toHaveLength(count);
  logs.push(chunk(2)); await visibility(false);
  expect(requests.length).toBe(count + 2);
  await tick(); expect(host.querySelectorAll("[data-seq]")).toHaveLength(2);
});
it("初始隐藏不请求；在途请求隐藏或卸载会 abort", async () => {
  await visibility(true); await mount(); expect(requests).toHaveLength(0);
  fetchMock.mockImplementation((_url: string, init: RequestInit) => {
    signals.push(init.signal as AbortSignal); return new Promise(() => {});
  });
  await visibility(false); expect(signals[0]?.aborted).toBe(false);
  await visibility(true); expect(signals[0]?.aborted).toBe(true);
  await visibility(false);
  await act(async () => root.unmount()); expect(signals[1]?.aborted).toBe(true);
  root = createRoot(host);
});
it("卸载后忽略不遵守 abort 的晚到响应", async () => {
  let resolve!: (value: unknown) => void;
  fetchMock.mockImplementation(() => new Promise((done) => { resolve = done; }));
  await mount(); await act(async () => root.unmount());
  resolve({ ok: true, status: 200, json: async () => detail() }); await flush();
  expect(fetchMock).toHaveBeenCalledTimes(1); root = createRoot(host);
});
it("只读无取消按钮；授权按钮与确认文案如实", async () => {
  await mount(); expect(host.querySelector("button")).toBeNull();
  await mount(new Set(["pipelines.run"]));
  const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
  await act(async () => host.querySelector("button")!.click());
  expect(confirm).toHaveBeenCalledWith(expect.stringContaining("目标主机上的命令可能继续执行完毕"));
  expect(requests.some((s) => s.endsWith("/actions/cancel"))).toBe(true);
});
it("向上阅读不跳底；底部新日志跟随", async () => {
  await mount(); const el = host.querySelector('[aria-label="执行日志"]') as HTMLDivElement;
  Object.defineProperties(el, { scrollHeight: { configurable: true, value: 1000 }, clientHeight: { value: 200 } });
  el.scrollTop = 100; el.dispatchEvent(new Event("scroll", { bubbles: true }));
  logs.push(chunk(2)); await tick(); expect(el.scrollTop).toBe(100);
  el.scrollTop = 800; el.dispatchEvent(new Event("scroll", { bubbles: true }));
  Object.defineProperty(el, "scrollHeight", { value: 1200 });
  logs.push(chunk(3)); await tick(); expect(el.scrollTop).toBe(1200);
});
it("服务端错误完整显示且可恢复", async () => {
  fetchMock.mockResolvedValueOnce({ ok: false, status: 400, json: async () => ({ error: "具体错误句子。" }) });
  await mount(); expect(host.querySelector('[role="alert"]')?.textContent).toBe("具体错误句子。");
  await tick(); expect(host.querySelector('[role="alert"]')).toBeNull();
});
it("Actions cancellation describes remote cleanup and displays each job's executor", async () => {
  fetchMock.mockImplementation(async (url: string) => ({ok:true,status:200,json:async()=>url.includes('/logs?')?{items:[],next_seq:0}:{...detail(),definition_format:'github-actions',jobs:[{key:'compile',host_id:'81',state:'running',started_at:1,finished_at:0}]}}));
  await mount(new Set(['pipelines.run']));
  expect(host.textContent).toContain('取消执行');
  expect(host.textContent).toContain('compile');
  expect(host.textContent).toContain('81');
  expect(host.textContent).not.toContain('命令可能继续执行');
});
