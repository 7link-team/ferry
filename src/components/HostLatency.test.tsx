import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { HostLatency } from "./HostLatency";

let root: Root;
let element: HTMLDivElement;
let complete: (response: { ok: boolean; stderr?: string }) => void;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.useFakeTimers();
  element = document.createElement("div"); document.body.append(element);
  root = createRoot(element);
  vi.stubGlobal("fetch", vi.fn((_url: string, init: RequestInit) => new Promise((resolve, reject) => {
    complete = result => resolve({ ok: true, status: 200, json: async () => result });
    init.signal?.addEventListener("abort", () => reject(new DOMException("Aborted", "AbortError")));
  })));
});
afterEach(async () => {
  await act(async () => root.unmount());
  element.remove(); vi.unstubAllGlobals(); vi.useRealTimers();
});
const render = async (hostId = "1") => {
  await act(async () => root.render(<HostLatency workspace="ops" hostId={hostId} />));
};

it.each([[30, "text-success"], [150, "text-warning"], [450, "text-destructive"]])(
  "shows measured %i ms with the corresponding color and animated icon", async (delay, color) => {
    await render();
    expect(element.textContent).toBe("检测中");
    expect(element.querySelector("svg")?.getAttribute("class")).toContain("animate-spin");
    await act(async () => { await vi.advanceTimersByTimeAsync(delay); complete({ ok: true }); });
    expect(element.textContent).toBe(`${delay} ms`);
    expect(element.firstElementChild?.className).toContain(color);
    expect(element.querySelector("svg")?.getAttribute("class")).toContain("animate-pulse");
    expect(element.firstElementChild?.getAttribute("title")).toContain("包含连接建立和探测命令处理时间");
    expect(vi.mocked(fetch).mock.calls[0]![0]).toBe("/api/workspaces/ops/hosts/1/actions/test-connection");
  },
);

it("times out, clears stale latency, and recovers on the next probe without overlapping requests", async () => {
  await render();
  await act(async () => { await vi.advanceTimersByTimeAsync(25); complete({ ok: true }); });
  await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
  expect(element.textContent).toBe("25 ms");
  expect(fetch).toHaveBeenCalledTimes(2);
  await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
  expect(element.textContent).toBe("连接异常");
  expect(element.firstElementChild?.getAttribute("title")).toContain("超时");
  expect(fetch).toHaveBeenCalledTimes(2);
  await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
  await act(async () => { await vi.advanceTimersByTimeAsync(40); complete({ ok: true }); });
  expect(element.textContent).toBe("40 ms");
});

it("reports a failed host probe even when the HTTP request succeeds", async () => {
  await render();
  await act(async () => complete({ ok: false, stderr: "host unavailable" }));
  expect(element.textContent).toBe("连接异常");
  expect(element.firstElementChild?.getAttribute("title")).toContain("host unavailable");
});

it("aborts the old request on host change and stops polling on unmount", async () => {
  await render();
  const signal = vi.mocked(fetch).mock.calls[0]![1]!.signal!;
  await render("2");
  expect(signal.aborted).toBe(true);
  expect(fetch).toHaveBeenCalledTimes(2);
  await act(async () => root.unmount());
  await vi.advanceTimersByTimeAsync(30_000);
  expect(fetch).toHaveBeenCalledTimes(2);
  root = createRoot(element);
});

it("pauses probes in a hidden page and samples immediately when visible", async () => {
  vi.spyOn(document, "hidden", "get").mockReturnValue(true);
  await render();
  expect(fetch).not.toHaveBeenCalled();
  vi.spyOn(document, "hidden", "get").mockReturnValue(false);
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  expect(fetch).toHaveBeenCalledTimes(1);
  vi.restoreAllMocks();
});
