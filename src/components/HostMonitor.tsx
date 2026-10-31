import { AutoField } from "./auto-form";
import { useEffect, useState } from "react";
import { Activity, HardDrive, MemoryStick, Network, RotateCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { api, errorText } from "../api";
import { MonitorTimeChart, type MonitorPoint } from "./MonitorTimeChart";

export interface Monitor {
  cpu_percent: number;
  load1: number;
  memory_used: number;
  memory_total: number;
  disks: { name: string; mount: string; used: number; total: number }[];
  networks: { name: string; received: number; transmitted: number }[];
}

const nonnegative = (value: unknown): value is number => typeof value === "number" && Number.isFinite(value) && value >= 0;

export function isMonitor<T>(value: T): value is T & Monitor {
  if (!value || typeof value !== "object") return false;
  const row = value as Partial<Monitor>;
  return typeof row.cpu_percent === "number" && Number.isFinite(row.cpu_percent)
    && typeof row.load1 === "number" && Number.isFinite(row.load1)
    && nonnegative(row.memory_used) && nonnegative(row.memory_total)
    && Array.isArray(row.disks) && row.disks.every(disk => disk && typeof disk.name === "string" && typeof disk.mount === "string" && nonnegative(disk.used) && nonnegative(disk.total))
    && Array.isArray(row.networks) && row.networks.every(network => network && typeof network.name === "string" && nonnegative(network.received) && nonnegative(network.transmitted));
}

const bytes = (value: number) => {
  if (!Number.isFinite(value)) return "—";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let index = 0;
  while (value >= 1024 && index < units.length - 1) { value /= 1024; index++; }
  return `${value.toFixed(index ? 1 : 0)} ${units[index]}`;
};
const percent = (used: number, total: number) => total > 0 ? `${(used / total * 100).toFixed(1)}%` : "—";
const ratio = (used: number, total: number) => total > 0 ? Math.max(0, Math.min(100, used / total * 100)) : 0;
export type MonitorSample = { sampled_at: number; monitor: Monitor };
type MonitorResponse = Partial<Monitor> & {
  sampled_at?: number; server_time?: number; samples?: MonitorSample[];
  status?: "online" | "offline" | "collecting"; error?: string;
  collection_interval_seconds?: number;
};
/**
 * Loopback, container, hypervisor, VPN and OS filter interfaces, by name, as
 * monitoring dashboards conventionally exclude them. Names are a heuristic (a
 * Hyper-V external vEthernet carries real traffic), so all stay one click away.
 */
const VIRTUAL_NETWORK = new RegExp([
  "^lo\\d*$", "^(docker|veth|br-|virbr|vnet|cni|flannel|cali|weave|kube|vxlan|genev|dummy|tun|tap|wg|tailscale|zt|vmnet|vboxnet|utun|awdl|llw|anpi|bridge|gif|stf|ap\\d)",
  "loopback", "^vethernet", "isatap", "teredo", "npcap", "pseudo-interface", "virtualbox", "vmware", "hyper-v", "wan miniport", "kernel debug", "wi-fi direct", "^(本地连接|local area connection)\\*", "filter-\\d{4}$", "-(wfp|qos)", "zerotier", "openvpn", "wireguard", "tap-windows",
].join("|"), "i");
export const isVirtualNetwork = (name: string) => VIRTUAL_NETWORK.test(name.trim());

export function networkRates(samples: MonitorSample[], name: string, key: "received" | "transmitted", step: number): MonitorPoint[] {
  return samples.map((sample, index) => {
    const current = sample.monitor.networks.find(network => network.name === name)?.[key];
    const previousSample = samples[index - 1];
    const previous = previousSample?.monitor.networks.find(network => network.name === name)?.[key];
    const elapsed = previousSample ? sample.sampled_at - previousSample.sampled_at : 0;
    return { at: sample.sampled_at, value: current === undefined || previous === undefined || elapsed <= 0 || elapsed > step * 1.5 || current < previous
      ? null : (current - previous) / (elapsed / 1000) };
  });
}

export function HostMonitor({ workspace, hostId }: { workspace: string; hostId: string }) {
  const [monitor, setMonitor] = useState<Monitor>();
  const [samples, setSamples] = useState<MonitorSample[]>([]);
  const [error, setError] = useState("");
  const [interval, setIntervalSeconds] = useState(30);
  const [revision, setRevision] = useState(0);
  const [updatedAt, setUpdatedAt] = useState(0);
  const [rangeSeconds, setRangeSeconds] = useState(3600);
  const [windowEnd, setWindowEnd] = useState(Date.now());
  const [collectionSeconds, setCollectionSeconds] = useState(5);
  const [collecting, setCollecting] = useState(false);
  const [allNetworks, setAllNetworks] = useState(false);
  const step = (interval || 30) * 1000;
  const chart = { start: windowEnd - rangeSeconds * 1000, end: windowEnd, step };
  const points = (value: (monitor: Monitor) => number | null) => samples.map(sample => ({ at: sample.sampled_at, value: value(sample.monitor) }));
  const percentage = (value: number) => `${value.toFixed(0)}%`;
  useEffect(() => { setMonitor(undefined); setSamples([]); setUpdatedAt(0); setError(""); }, [workspace, hostId]);
  useEffect(() => {
    let live = true;
    let request: AbortController | undefined;
    const refresh = async () => {
      if (request) return;
      const controller = new AbortController(); request = controller;
      const timeout = setTimeout(() => controller.abort(), 8_000);
      try {
        const value = await api<MonitorResponse>(`/workspaces/${encodeURIComponent(workspace)}/nodes/${encodeURIComponent(hostId)}/metrics?window_seconds=${rangeSeconds}&step_seconds=${interval || 30}`, { signal: controller.signal });
        if (!live) return;
        const at = value.sampled_at ?? Date.now();
        const end = value.server_time ?? Date.now();
        if (!Number.isFinite(at) || !Number.isFinite(end)) throw new Error("监控采样时间无效");
        if (value.samples !== undefined && (!Array.isArray(value.samples) || !value.samples.every(sample => Number.isFinite(sample?.sampled_at) && isMonitor(sample.monitor)))) throw new Error("监控历史数据格式无效");
        if (!isMonitor(value) && value.status !== "collecting" && value.status !== "offline") throw new Error("监控接口尚未提供有效数据，请更新后端服务。");
        setWindowEnd(end); setCollecting(value.status === "collecting");
        if (value.collection_interval_seconds) setCollectionSeconds(value.collection_interval_seconds);
        if (isMonitor(value)) { setMonitor(value); setUpdatedAt(at); }
        else { setMonitor(undefined); setUpdatedAt(0); }
        setSamples(current => {
          const next = value.samples ?? (isMonitor(value) ? [...current.filter(sample => sample.sampled_at !== at), { sampled_at: at, monitor: value }] : current);
          return [...new Map(next.filter(sample => sample.sampled_at >= end - rangeSeconds * 1000 && sample.sampled_at <= end).map(sample => [sample.sampled_at, sample])).values()].sort((a, b) => a.sampled_at - b.sampled_at).slice(-721);
        });
        setError(value.status === "offline" ? (value.error || "主机监控采集已中断，空白时段没有有效数据。") : "");
      } catch (cause) {
        if (live) setError(controller.signal.aborted ? "监控查询超时，已保留最后一次有效数据。" : errorText(cause));
      } finally { clearTimeout(timeout); request = undefined; }
    };
    void refresh();
    const timer = interval ? setInterval(() => { void refresh(); }, interval * 1000) : undefined;
    return () => { live = false; request?.abort(); if (timer) clearInterval(timer); };
  }, [workspace, hostId, interval, revision, rangeSeconds]);
  const physical = monitor?.networks.filter(network => !isVirtualNetwork(network.name)) ?? [];
  const virtualCount = (monitor?.networks.length ?? 0) - physical.length;
  // Never hide everything: a host whose only adapters look virtual shows them all.
  const networks = allNetworks || !physical.length ? monitor?.networks ?? [] : physical;
  return <section className="w-full min-w-0 space-y-6 pb-8" aria-label="主机监控">
    <div className="flex flex-wrap items-center gap-3 border-b pb-4">
      <div className="flex items-center gap-2"><Activity className="size-4 text-primary" /><h2 className="text-sm font-semibold">实时监控</h2></div>
      <AutoField label="时段" className="flex items-center gap-2 text-xs text-muted-foreground"><select aria-label="监控时段" className="h-8 rounded-md border border-input bg-background px-2 text-sm text-foreground" value={rangeSeconds} onChange={event => setRangeSeconds(Number(event.target.value))}><option value={300}>近 5 分钟</option><option value={900}>近 15 分钟</option><option value={3600}>近 1 小时</option></select></AutoField>
      <span className="flex-1" />
      {updatedAt > 0 && <span className="text-xs tabular-nums text-muted-foreground">更新于 {new Date(updatedAt).toLocaleTimeString()}</span>}
      <AutoField label="自动刷新" className="flex items-center gap-2 text-xs text-muted-foreground"><select aria-label="自动刷新间隔" className="h-8 rounded-md border border-input bg-background px-2 text-sm text-foreground" value={interval} onChange={event => setIntervalSeconds(Number(event.target.value))}><option value={0}>关闭</option><option value={5}>5 秒</option><option value={15}>15 秒</option><option value={30}>30 秒</option><option value={60}>1 分钟</option></select></AutoField>
      <Button type="button" variant="ghost" size="icon" aria-label="刷新监控" title="刷新监控" onClick={() => setRevision(value => value + 1)}><RotateCw className="size-4" /></Button>
    </div>
    <p className="text-xs leading-5 tabular-nums text-muted-foreground">{new Date(chart.start).toLocaleTimeString([], { hour12: false })} — {new Date(chart.end).toLocaleTimeString([], { hour12: false })} · 后台每 {collectionSeconds} 秒采集 · 图表刻度 {interval || 30} 秒 · {interval ? `每 ${interval} 秒刷新` : "自动刷新已暂停"} · 当前 {samples.length} 个真实采样点；空白处无数据</p>
    {error && <p role="alert" className="border-l-2 border-destructive bg-destructive/5 px-3 py-2 text-sm text-destructive">监控暂不可用：{error}</p>}
    {!monitor && !error && <p role="status" className="py-8 text-sm text-muted-foreground">{collecting ? "等待后台首次采样，暂无历史数据。" : "正在读取监控数据…"}</p>}
    {monitor && <>
      <div className="grid gap-8 border-b pb-7 md:grid-cols-2">
        <div className="space-y-3"><div className="flex items-center gap-2 text-sm text-muted-foreground"><Activity className="size-4" />CPU 使用率</div><div className="text-3xl font-semibold tabular-nums">{monitor.cpu_percent.toFixed(1)}<span className="ml-1 text-base font-normal text-muted-foreground">%</span></div><MonitorTimeChart {...chart} label="CPU 使用率趋势" series={[{ name: "CPU", points: points(value => value.cpu_percent) }]} ceiling={100} format={percentage} /><p className="text-xs tabular-nums text-muted-foreground">一分钟负载 {monitor.load1.toFixed(2)}</p></div>
        <div className="space-y-3"><div className="flex items-center gap-2 text-sm text-muted-foreground"><MemoryStick className="size-4" />内存使用率</div><div className="text-3xl font-semibold tabular-nums">{percent(monitor.memory_used, monitor.memory_total)}</div><MonitorTimeChart {...chart} label="内存使用率趋势" series={[{ name: "内存", points: points(value => value.memory_total > 0 ? ratio(value.memory_used, value.memory_total) : null) }]} ceiling={100} format={percentage} /><p className="text-xs tabular-nums text-muted-foreground">{bytes(monitor.memory_used)} 已使用 / {bytes(monitor.memory_total)} 总量</p></div>
      </div>
      <div className="grid gap-8 lg:grid-cols-2">
        <section className="min-w-0"><h3 className="mb-3 flex items-center gap-2 text-sm font-semibold"><HardDrive className="size-4 text-muted-foreground" />磁盘 <span className="text-xs font-normal tabular-nums text-muted-foreground">{monitor.disks.length}</span></h3>{monitor.disks.length ? monitor.disks.map(disk => <div key={`${disk.name}/${disk.mount}`} className="space-y-2 border-t py-3 text-sm"><div className="flex min-w-0 flex-wrap justify-between gap-x-3 gap-y-1"><span className="min-w-0 truncate font-medium" title={`${disk.name} · ${disk.mount}`}>{disk.mount} <span className="font-normal text-muted-foreground">{disk.name}</span></span><strong className="tabular-nums">{percent(disk.used, disk.total)}</strong></div><MonitorTimeChart {...chart} label={`${disk.mount} 磁盘使用率趋势`} series={[{ name: "磁盘", points: points(value => { const item = value.disks.find(item => item.mount === disk.mount && item.name === disk.name); return item && item.total > 0 ? ratio(item.used, item.total) : null; }) }]} ceiling={100} format={percentage} /><p className="text-xs tabular-nums text-muted-foreground">{bytes(disk.used)} / {bytes(disk.total)}</p></div>) : <p className="text-sm text-muted-foreground">暂无磁盘数据</p>}</section>
        <section className="min-w-0"><div className="mb-3 flex flex-wrap items-center justify-between gap-2"><h3 className="flex items-center gap-2 text-sm font-semibold"><Network className="size-4 text-muted-foreground" />网卡 <span className="text-xs font-normal tabular-nums text-muted-foreground">{networks.length}{networks.length !== monitor.networks.length && ` / ${monitor.networks.length}`}</span></h3>{virtualCount > 0 && <AutoField label={`显示虚拟网卡（${virtualCount}）`} className="flex-row-reverse items-center justify-end gap-2 text-xs text-muted-foreground"><Checkbox checked={allNetworks} onCheckedChange={checked => setAllNetworks(checked === true)} /></AutoField>}</div>{networks.length ? networks.map(network => { const received = networkRates(samples, network.name, "received", step); const transmitted = networkRates(samples, network.name, "transmitted", step); return <div key={network.name} className="space-y-2 border-t py-3 text-sm"><div className="flex min-w-0 flex-wrap items-center justify-between gap-2"><span className="truncate font-medium" title={network.name}>{network.name}</span><span className="text-xs tabular-nums text-muted-foreground">累计接收 {bytes(network.received)} · 发送 {bytes(network.transmitted)}</span></div><MonitorTimeChart {...chart} label={`${network.name} 接收与发送速率趋势`} series={[{ name: "接收", points: received }, { name: "发送", points: transmitted, secondary: true }]} format={value => `${bytes(value)}/s`} /><p className="text-xs tabular-nums text-muted-foreground"><span className="text-primary">接收 {received.at(-1)?.value == null ? "—" : `${bytes(received.at(-1)!.value!)}/s`}</span><span className="mx-3 text-success">发送 {transmitted.at(-1)?.value == null ? "—" : `${bytes(transmitted.at(-1)!.value!)}/s`}</span></p></div>; }) : <p className="text-sm text-muted-foreground">暂无网卡数据</p>}</section>
      </div>
    </>}
  </section>;
}
