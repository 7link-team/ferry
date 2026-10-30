import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { HostTelemetry, TerminalMetrics } from "./HostTelemetry";

let root: Root;
let element: HTMLDivElement;
const monitor = { cpu_percent: 25, load1: 1, memory_used: 4 * 1024 ** 3, memory_total: 8 * 1024 ** 3,
  disks: [{ name: "disk", mount: "/", used: 30, total: 100 }], networks: [] };
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.useFakeTimers();
  element = document.createElement("div"); document.body.append(element); root = createRoot(element);
  vi.stubGlobal("fetch", vi.fn(async () => ({ ok: true, status: 200, json: async () => monitor })));
});
afterEach(async () => { await act(async () => root.unmount()); element.remove(); vi.unstubAllGlobals(); vi.useRealTimers(); });
const render = async (enabled = true) => {
  await act(async () => root.render(<HostTelemetry workspace="ops" hostId="1" enabled={enabled}><TerminalMetrics /><TerminalMetrics /></HostTelemetry>));
};
it("shares one 3-second sample across terminal splits and shows CPU, memory and disk details", async () => {
  await render();
  expect(fetch).toHaveBeenCalledTimes(1);
  expect(element.querySelectorAll('[aria-label="实时主机监控"]')).toHaveLength(2);
  expect(element.textContent).toContain("CPU 25%内存 50%磁盘 30%");
  expect(element.querySelector('[title="内存：4.0 / 8.0 GiB"]')).not.toBeNull();
  await act(async () => { await vi.advanceTimersByTimeAsync(3_000); });
  expect(fetch).toHaveBeenCalledTimes(2);
});
it("rejects incomplete monitoring data without crashing and recovers on a later sample", async () => {
  vi.mocked(fetch).mockResolvedValueOnce({ ok: true, status: 200, json: async () => ({}) } as Response);
  await render();
  expect(element.textContent).toContain("监控暂不可用");
  expect(element.textContent).not.toContain("NaN");
  await act(async () => { await vi.advanceTimersByTimeAsync(3_000); });
  expect(element.textContent).toContain("CPU 25%");
});
it("does not poll for file-only tabs and stops sampling when the terminal tab becomes inactive", async () => {
  await render(false); expect(fetch).not.toHaveBeenCalled();
  await render(true); expect(fetch).toHaveBeenCalledTimes(1);
  await render(false);
  await act(async () => { await vi.advanceTimersByTimeAsync(30_000); });
  expect(fetch).toHaveBeenCalledTimes(1);
});
