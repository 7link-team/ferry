import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { HostMonitor, isMonitor, isVirtualNetwork, networkRates, type MonitorSample } from "./HostMonitor";

let root: Root;
let host: HTMLDivElement;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); vi.useRealTimers(); });

it("shows individual disks and network interfaces from the host monitor endpoint", async () => {
  const fetch = vi.fn(async () => ({ ok: true, status: 200, json: async () => ({
    cpu_percent: 24.3, load1: 0.4, memory_used: 1024, memory_total: 4096,
    disks: [{ name: "/dev/sda1", mount: "/", used: 1024, total: 4096 }, { name: "/dev/sdb1", mount: "/data", used: 2048, total: 8192 }],
    networks: [{ name: "eth0", received: 1234, transmitted: 5678 }, { name: "eth1", received: 55, transmitted: 66 }],
  }) }));
  vi.stubGlobal("fetch", fetch);
  await act(async () => root.render(<HostMonitor workspace="ops" hostId="h1" />));
  expect(fetch).toHaveBeenCalledWith("/api/workspaces/ops/nodes/h1/metrics?window_seconds=3600&step_seconds=30", expect.anything());
  expect(host.textContent).toContain("24.3%");
  expect(host.textContent).toContain("/data");
  expect(host.textContent).toContain("eth0");
  expect(host.textContent).toContain("eth1");
});

it("classifies loopback, container, hypervisor, VPN and OS filter adapters as virtual", () => {
  for (const name of ["lo", "lo0", "docker0", "veth12ab", "br-3f2a", "virbr0", "cni0", "flannel.1", "cali123", "tun0", "tap0", "wg0", "tailscale0", "zt5u4y", "utun3", "awdl0", "bridge0",
    "Loopback Pseudo-Interface 1", "vEthernet (WSL)", "vEthernet (Default Switch)", "isatap.{6B2F}", "Teredo Tunneling Pseudo-Interface", "Npcap Loopback Adapter", "VMware Network Adapter VMnet8",
    "VirtualBox Host-Only Network", "本地连接* 10", "Local Area Connection* 2", "WLAN-WFP Native MAC Layer LightWeight Filter-0000", "以太网-QoS Packet Scheduler-0000", "ZeroTier One [abc]", "OpenVPN Data Channel Offload"])
    expect(isVirtualNetwork(name), name).toBe(true);
  for (const name of ["eth0", "ens33", "enp0s31f6", "eno1", "wlan0", "wlp2s0", "en0", "bond0", "以太网", "以太网 2", "WLAN", "Ethernet", "Wi-Fi", "Local Area Connection"])
    expect(isVirtualNetwork(name), name).toBe(false);
});

it("shows physical adapters by default, reveals virtual adapters on request, and never hides every adapter", async () => {
  const body = (networks: { name: string }[]) => ({ cpu_percent: 1, load1: 0, memory_used: 1, memory_total: 2, disks: [], networks: networks.map(n => ({ ...n, received: 1, transmitted: 1 })) });
  vi.stubGlobal("fetch", vi.fn(async () => response(body([{ name: "以太网" }, { name: "vEthernet (WSL)" }, { name: "docker0" }]))));
  await act(async () => root.render(<HostMonitor workspace="ops" hostId="h1" />));
  expect(host.textContent).toContain("以太网");
  expect(host.textContent).not.toContain("vEthernet (WSL)");
  expect(host.textContent).toContain("网卡 1 / 3");
  const toggle = host.querySelector<HTMLButtonElement>('button[role="checkbox"]')!;
  expect(document.getElementById(toggle.getAttribute("aria-labelledby")!)?.textContent).toContain("显示虚拟网卡（2）");
  await act(async () => toggle.click());
  expect(host.textContent).toContain("vEthernet (WSL)");
  expect(host.textContent).toContain("docker0");
  await act(async () => root.unmount()); root = createRoot(host);
  vi.stubGlobal("fetch", vi.fn(async () => response(body([{ name: "vEthernet (External)" }]))));
  await act(async () => root.render(<HostMonitor workspace="ops" hostId="h2" />));
  expect(host.textContent).toContain("vEthernet (External)");
});

const metrics = (cpu_percent = 20) => ({ cpu_percent, load1: 0.4, memory_used: 1024, memory_total: 4096,
  disks: [{ name: "system", mount: "/", used: 1024, total: 4096 }], networks: [{ name: "eth0", received: 100, transmitted: 100 }] });
const response = (body: object) => ({ ok: true, status: 200, json: async () => body });
const sample = (sampled_at: number, cpu = 20): MonitorSample => ({ sampled_at, monitor: metrics(cpu) });

it("rejects nonfinite metrics instead of rendering invalid percentages", () => {
  expect(isMonitor({ ...metrics(), memory_total: NaN })).toBe(false);
  expect(isMonitor({ ...metrics(), disks: [{ name: "bad", mount: "/", used: Infinity, total: 10 }] })).toBe(false);
  expect(isMonitor({ ...metrics(), networks: [{ name: "bad", received: -1, transmitted: 0 }] })).toBe(false);
});

it("breaks disk history when a mount disappears and clears old values after collector restart", async () => {
  vi.useFakeTimers();
  const now = Date.now();
  const samples = [sample(now - 60_000), sample(now - 30_000), sample(now)];
  samples[1]!.monitor.disks = [];
  const fetch = vi.fn().mockResolvedValueOnce(response({ ...metrics(), sampled_at: now, server_time: now, samples }))
    .mockResolvedValue(response({ status: "collecting", server_time: now + 30_000, samples: [] }));
  vi.stubGlobal("fetch", fetch);
  await act(async () => root.render(<HostMonitor workspace="ops" hostId="h1" />));
  const disk = host.querySelector('[data-monitor-chart="/ 磁盘使用率趋势"]')!;
  expect(disk.querySelectorAll('[data-sample-line]')).toHaveLength(0);
  expect(disk.querySelectorAll('[data-sample-point]')).toHaveLength(2);
  await act(async () => vi.advanceTimersByTimeAsync(30_000));
  expect(host.querySelector('svg[role="img"]')).toBeNull();
  expect(host.querySelector('[role="status"]')?.textContent).toContain("暂无历史数据");
});

it("does not let a late response from the previous host overwrite the new host", async () => {
  let finish!: (value: ReturnType<typeof response>) => void;
  const oldRequest = new Promise<ReturnType<typeof response>>(resolve => { finish = resolve; });
  vi.stubGlobal("fetch", vi.fn().mockReturnValueOnce(oldRequest).mockResolvedValue(response(metrics(77))));
  await act(async () => root.render(<HostMonitor workspace="ops" hostId="old" />));
  await act(async () => root.render(<HostMonitor workspace="ops" hostId="new" />));
  expect(host.textContent).toContain("77.0%");
  await act(async () => finish(response(metrics(12))));
  expect(host.textContent).toContain("77.0%");
  expect(host.textContent).not.toContain("12.0%");
});

it("shows a single real point without inventing a full-width flat history, then advances on refresh", async () => {
  vi.useFakeTimers();
  const now = new Date("2026-10-31T04:00:00Z").getTime(); vi.setSystemTime(now);
  const fetch = vi.fn().mockResolvedValueOnce(response({ ...metrics(), sampled_at: now, server_time: now, samples: [sample(now)] }))
    .mockResolvedValue(response({ ...metrics(60), sampled_at: now + 30_000, server_time: now + 30_000, samples: [sample(now), sample(now + 30_000, 60)] }));
  vi.stubGlobal("fetch", fetch);
  await act(async () => root.render(<HostMonitor workspace="ops" hostId="h1" />));
  const chart = () => host.querySelector('[data-monitor-chart="CPU 使用率趋势"]')!;
  expect(chart().querySelector('[data-sample-line]')).toBeNull();
  expect(chart().querySelectorAll('[data-sample-point]')).toHaveLength(1);
  expect(host.textContent).toContain("当前 1 个真实采样点");
  await act(async () => vi.advanceTimersByTimeAsync(30_000));
  const coordinates = chart().querySelector('[data-sample-line]')!.getAttribute("points")!.split(" ").map(point => point.split(",").map(Number));
  expect(coordinates[1]![0]! - coordinates[0]![0]!).toBeCloseTo(5); // 30 seconds within a real one-hour axis.
  expect(coordinates[0]![1]).not.toBe(coordinates[1]![1]);
  expect(host.textContent).toContain("60.0%");
  expect(host.textContent).toContain("当前 2 个真实采样点");
  await act(async () => chart().querySelector("svg")!.dispatchEvent(new FocusEvent("focusin", { bubbles: true })));
  expect(chart().querySelector('[role="tooltip"]')?.textContent).toContain("60%");
});

it("requests the selected time range and refresh step, and keeps real history after a query failure", async () => {
  const now = Date.now();
  const fetch = vi.fn().mockResolvedValue(response({ ...metrics(), server_time: now, sampled_at: now, samples: [sample(now)] }));
  vi.stubGlobal("fetch", fetch);
  await act(async () => root.render(<HostMonitor workspace="ops" hostId="h1" />));
  const change = async (label: string, value: string) => act(async () => {
    const select = host.querySelector<HTMLSelectElement>(`[aria-label="${label}"]`)!;
    select.value = value; select.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await change("监控时段", "300");
  await change("自动刷新间隔", "5");
  expect(fetch).toHaveBeenLastCalledWith("/api/workspaces/ops/nodes/h1/metrics?window_seconds=300&step_seconds=5", expect.anything());
  fetch.mockResolvedValueOnce({ ok: false, status: 503, json: async () => ({ error: "query failed" }) });
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="刷新监控"]')!.click());
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("query failed");
  expect(host.textContent).toContain("20.0%");
  expect(host.querySelectorAll('[data-sample-point]').length).toBeGreaterThan(0);
});

it("does not render invented zeros while the collector has no history", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(response({ status: "collecting", samples: [], server_time: Date.now() })));
  await act(async () => root.render(<HostMonitor workspace="ops" hostId="h1" />));
  expect(host.querySelector('[role="status"]')?.textContent).toContain("暂无历史数据");
  expect(host.querySelector("svg[role=img]")).toBeNull();
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("leaves first, reset, missing-interface and disconnected network rates unknown instead of zero", () => {
  const samples = [sample(1000), sample(6000), sample(11000), sample(16000), sample(60000)];
  samples[1]!.monitor.networks[0]!.received = 300;
  samples[2]!.monitor.networks[0]!.received = 10; // Interface counter reset.
  samples[3]!.monitor.networks = [];
  expect(networkRates(samples, "eth0", "received", 5000).map(point => point.value)).toEqual([null, 40, null, null, null]);
});

it("keeps the host page mounted when an older backend returns an empty monitor response", async () => {
  vi.stubGlobal("fetch", vi.fn(async () => ({ ok: true, status: 200, json: async () => ({}) })));
  await act(async () => root.render(<HostMonitor workspace="ops" hostId="h1" />));
  expect(host.querySelector('[aria-label="主机监控"]')).not.toBeNull();
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("更新后端服务");
});
