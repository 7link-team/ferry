import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { HostMonitor } from "./HostMonitor";

let root: Root;
let host: HTMLDivElement;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

it("shows individual disks and network interfaces from the host monitor endpoint", async () => {
  const fetch = vi.fn(async () => ({ ok: true, status: 200, json: async () => ({
    cpu_percent: 24.3, load1: 0.4, memory_used: 1024, memory_total: 4096,
    disks: [{ name: "/dev/sda1", mount: "/", used: 1024, total: 4096 }, { name: "/dev/sdb1", mount: "/data", used: 2048, total: 8192 }],
    networks: [{ name: "eth0", received: 1234, transmitted: 5678 }, { name: "eth1", received: 55, transmitted: 66 }],
  }) }));
  vi.stubGlobal("fetch", fetch);
  await act(async () => root.render(<HostMonitor workspace="ops" hostId="h1" />));
  expect(fetch).toHaveBeenCalledWith("/api/workspaces/ops/nodes/h1/metrics", expect.anything());
  expect(host.textContent).toContain("24.3%");
  expect(host.textContent).toContain("/data");
  expect(host.textContent).toContain("eth0");
  expect(host.textContent).toContain("eth1");
});

it("keeps the host page mounted when an older backend returns an empty monitor response", async () => {
  vi.stubGlobal("fetch", vi.fn(async () => ({ ok: true, status: 200, json: async () => ({}) })));
  await act(async () => root.render(<HostMonitor workspace="ops" hostId="h1" />));
  expect(host.querySelector('[aria-label="主机监控"]')).not.toBeNull();
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("更新后端服务");
});
